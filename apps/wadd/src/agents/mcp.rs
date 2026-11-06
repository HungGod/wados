//! The `wadspaces` MCP server's protocol: newline-delimited JSON-RPC, as
//! `wadspaces-mcp` relays it from Claude Code's stdio. Who's calling is the
//! socket, never anything in a message. What comes in is untrusted.

use serde_json::{Value, json};

pub const VERSION: &str = "2025-06-18";

pub fn tools() -> Value {
    let s = |d: &str| json!({ "type": "string", "description": d });
    json!([
        { "name": "notify", "description": "Tell the person something on their desktop. Only what wants them.",
          "inputSchema": { "type": "object", "required": ["title"],
            "properties": { "title": s("One line"), "body": s("A little more") } } },
        { "name": "progress", "description": "Say where the job is up to.",
          "inputSchema": { "type": "object", "required": ["step", "state", "now"], "properties": {
            "step": { "enum": ["plan", "build", "check", "try", "ship"] },
            "state": { "enum": ["active", "done", "failed"] },
            "now": s("What it's doing, in a few words") } } },
        { "name": "request_sandbox", "description": "Ask the person for a sandbox to try the work in. They answer later: end your turn after calling this, and you'll be told.",
          "inputSchema": { "type": "object", "required": ["title", "why"],
            "properties": { "title": s("What's to be tried"), "why": s("Why it's worth trying by hand") } } },
        { "name": "set_checklist", "description": "What the person should try in the sandbox. `done` is 0 (not started) to 4 (done).",
          "inputSchema": { "type": "object", "required": ["items"], "properties": { "items": { "type": "array", "items": {
            "type": "object", "required": ["id", "label", "how", "done"], "properties": {
              "id": s("Stays the same between builds"), "label": s("What should be true"),
              "how": s("How to try it"), "done": { "type": "integer", "minimum": 0, "maximum": 4 } } } } } } },
        { "name": "update_sandbox", "description": "Put a new build in the sandbox: `path`, a folder under this wadspace's home, is copied across, and `run`, a shell command, starts it there.",
          "inputSchema": { "type": "object", "required": ["build"], "properties": {
            "build": { "type": "integer" }, "path": s("Relative to the home"), "run": s("Starts it") } } },
    ])
}

/// A line in: what to do with it. A notification gets no answer.
#[derive(Debug, PartialEq)]
pub enum Call {
    Answer(Value),
    Tool { id: Value, name: String, args: Value },
    Nothing,
}

pub fn read(line: &str) -> Call {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return Call::Answer(error(Value::Null, -32700, "not JSON"));
    };
    let id = v["id"].clone();
    match v["method"].as_str() {
        Some("initialize") => Call::Answer(result(
            id,
            json!({ "protocolVersion": VERSION, "capabilities": { "tools": {} },
                    "serverInfo": { "name": "wadspaces", "version": "1" } }),
        )),
        Some("tools/list") => Call::Answer(result(id, json!({ "tools": tools() }))),
        Some("ping") => Call::Answer(result(id, json!({}))),
        Some("tools/call") => Call::Tool {
            id,
            name: v["params"]["name"].as_str().unwrap_or_default().to_string(),
            args: v["params"]["arguments"].clone(),
        },
        _ if id.is_null() => Call::Nothing,
        _ => Call::Answer(error(id, -32601, "no such method")),
    }
}

pub fn result(id: Value, r: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": r })
}

pub fn error(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A tool's answer, or its refusal, which the agent reads.
pub fn answer(id: Value, r: Result<String, String>) -> Value {
    let failed = r.is_err();
    let text = r.unwrap_or_else(|e| e);
    result(id, json!({ "content": [{ "type": "text", "text": text }], "isError": failed }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_introduces_itself_and_lists_the_five_tools() {
        let Call::Answer(a) = read(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#) else { panic!() };
        assert_eq!(a["result"]["serverInfo"]["name"], "wadspaces");
        let Call::Answer(a) = read(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#) else { panic!() };
        let names: Vec<&str> =
            a["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["notify", "progress", "request_sandbox", "set_checklist", "update_sandbox"]);
    }

    #[test]
    fn notifications_and_nonsense_are_handled() {
        assert_eq!(read(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#), Call::Nothing);
        let Call::Answer(a) = read("nonsense") else { panic!() };
        assert_eq!(a["error"]["code"], -32700);
        let Call::Answer(a) = read(r#"{"id":3,"method":"nope"}"#) else { panic!() };
        assert_eq!(a["error"]["code"], -32601);
    }

    #[test]
    fn a_refusal_is_the_tools_not_the_protocols() {
        let a = answer(json!(4), Err("no".into()));
        assert_eq!(a["result"]["isError"], true);
        assert_eq!(a["result"]["content"][0]["text"], "no");
    }
}
