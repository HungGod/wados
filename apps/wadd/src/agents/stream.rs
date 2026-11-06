//! Claude Code's `--output-format stream-json`, a JSON object a line, as
//! what the Agents app shows. Anything else in it is ignored, for the stream
//! grows new kinds of line.

use serde_json::Value;
use wad_model::agents::AgentNote;

#[derive(Debug, PartialEq, Eq)]
pub enum Seen {
    Session(String),
    Note(AgentNote),
    /// The turn's end: its text, and whether it failed.
    Result {
        text: String,
        failed: bool,
    },
}

/// The MCP server's tools are noted by wadd as they're called.
const MCP: &str = "mcp__";

fn tool(name: &str, input: &Value) -> Option<AgentNote> {
    let text = |k: &str| input[k].as_str().unwrap_or_default().to_string();
    match name {
        "Read" => Some(AgentNote::Read { path: text("file_path") }),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => Some(AgentNote::Edit { path: text("file_path") }),
        "Bash" => Some(AgentNote::Run { command: text("command") }),
        n if n.starts_with(MCP) => None,
        _ => None,
    }
}

pub fn read(line: &str) -> Vec<Seen> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return vec![] };
    match v["type"].as_str() {
        Some("system") if v["subtype"] == "init" => {
            v["session_id"].as_str().map(|s| Seen::Session(s.to_string())).into_iter().collect()
        }
        Some("assistant") if v["parent_tool_use_id"].is_null() => {
            let blocks = v["message"]["content"].as_array().cloned().unwrap_or_default();
            blocks
                .iter()
                .filter_map(|b| match b["type"].as_str()? {
                    "text" => {
                        let text = b["text"].as_str()?.trim().to_string();
                        (!text.is_empty()).then_some(Seen::Note(AgentNote::Say { text }))
                    }
                    "tool_use" => tool(b["name"].as_str()?, &b["input"]).map(Seen::Note),
                    _ => None,
                })
                .collect()
        }
        Some("result") => {
            let failed = v["is_error"].as_bool().unwrap_or(false) || v["subtype"] != "success";
            let text = v["result"].as_str().unwrap_or_default().to_string();
            vec![Seen::Result { text, failed }]
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_starts_with_its_id() {
        let l = r#"{"type":"system","subtype":"init","session_id":"s1","mcp_servers":[]}"#;
        assert_eq!(read(l), [Seen::Session("s1".into())]);
    }

    #[test]
    fn what_it_says_and_does_becomes_its_work() {
        let l = r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[
            {"type":"text","text":"Reading it."},
            {"type":"tool_use","name":"Read","input":{"file_path":"a.rs"}},
            {"type":"tool_use","name":"Edit","input":{"file_path":"b.rs"}},
            {"type":"tool_use","name":"Bash","input":{"command":"cargo test"}},
            {"type":"tool_use","name":"mcp__wadspaces__notify","input":{"title":"x"}}]}}"#
            .replace('\n', "");
        assert_eq!(
            read(&l),
            [
                Seen::Note(AgentNote::Say { text: "Reading it.".into() }),
                Seen::Note(AgentNote::Read { path: "a.rs".into() }),
                Seen::Note(AgentNote::Edit { path: "b.rs".into() }),
                Seen::Note(AgentNote::Run { command: "cargo test".into() }),
            ]
        );
    }

    #[test]
    fn a_subagents_work_and_unknown_lines_are_left_out() {
        let l = r#"{"type":"assistant","parent_tool_use_id":"t1","message":{"content":[{"type":"text","text":"x"}]}}"#;
        assert!(read(l).is_empty());
        assert!(read(r#"{"type":"something_new"}"#).is_empty());
        assert!(read("not json").is_empty());
    }

    #[test]
    fn a_failure_can_call_itself_a_success() {
        // Claude Code 2.1.318, as it really says it, not logged in.
        let l = r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}"#;
        assert_eq!(read(l), [Seen::Result { text: "Not logged in · Please run /login".into(), failed: true }]);
    }

    #[test]
    fn the_result_says_how_it_ended() {
        let ok = r#"{"type":"result","subtype":"success","is_error":false,"result":"Done."}"#;
        assert_eq!(read(ok), [Seen::Result { text: "Done.".into(), failed: false }]);
        let bad = r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#;
        assert_eq!(read(bad), [Seen::Result { text: String::new(), failed: true }]);
    }
}
