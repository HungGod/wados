//! Agents and their sandboxes (the spike's `Agent`, `Sandbox`, `CheckItem`):
//! what wadd keeps, and everything that can be said about them with no I/O.

use serde::{Deserialize, Serialize};

pub const MAX_NOTES: usize = 400;
pub const MAX_ATTACHMENTS: usize = 10;
pub const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;
/// The most a tool's text may be: what comes from a wadspace isn't trusted.
pub const MAX_TEXT: usize = 2000;
pub const MAX_ITEMS: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentStatus {
    Working,
    NeedsYou,
    Done,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepId {
    Plan,
    Build,
    Check,
    Try,
    Ship,
}

pub const STEPS: [(StepId, &str); 5] = [
    (StepId::Plan, "Plan"),
    (StepId::Build, "Build"),
    (StepId::Check, "Check"),
    (StepId::Try, "Try it"),
    (StepId::Ship, "Ship"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepState {
    Todo,
    Active,
    Waiting,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStep {
    pub id: StepId,
    pub state: StepState,
}

/// A line of its work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum AgentNote {
    Say { text: String },
    You { text: String },
    Read { path: String },
    Edit { path: String },
    Run { command: String },
    Mcp { tool: String, text: String },
    Final { text: String },
}

/// What it's waiting for the person about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Ask {
    /// May it have a sandbox?
    Sandbox { title: String, why: String },
    /// Open a pull request, or try it in a sandbox after all?
    Ship,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    pub ws: String,
    pub task: String,
    pub status: AgentStatus,
    pub now: String,
    pub steps: Vec<AgentStep>,
    pub notes: Vec<AgentNote>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<Ask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
    /// The person said it may push and open a pull request.
    #[serde(default)]
    pub may_ship: bool,
    pub started_at: i64,
}

impl Agent {
    pub fn new(id: String, ws: String, task: String, now_ms: i64) -> Self {
        let steps = STEPS.iter().map(|(id, _)| AgentStep { id: *id, state: StepState::Todo }).collect();
        Self {
            id,
            ws,
            task,
            status: AgentStatus::Working,
            now: "Starting it…".into(),
            steps,
            notes: vec![],
            ask: None,
            session: None,
            sandbox: None,
            may_ship: false,
            started_at: now_ms,
        }
    }

    pub fn note(&mut self, n: AgentNote) {
        self.notes.push(n);
        let over = self.notes.len().saturating_sub(MAX_NOTES);
        self.notes.drain(..over);
    }

    pub fn set_step(&mut self, id: StepId, state: StepState) {
        if let Some(s) = self.steps.iter_mut().find(|s| s.id == id) {
            s.state = state;
        }
    }

    /// Does it want the person?
    pub fn needs_you(&self) -> bool {
        self.status == AgentStatus::NeedsYou
    }

    pub fn live(&self) -> bool {
        matches!(self.status, AgentStatus::Working | AgentStatus::NeedsYou)
    }
}

/// The agent's own word on how far an item got (the spike's `Impl`).
pub const IMPL_WORDS: [&str; 5] = ["Not started", "Started", "Halfway", "Nearly done", "Done"];
/// An item below this can't be tried yet.
pub const READY_AT: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Works,
    Issue,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckItem {
    pub id: String,
    pub label: String,
    pub how: String,
    /// 0 to 4.
    pub done: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    #[serde(default)]
    pub note: String,
    /// Changed in the build that's in now.
    #[serde(default)]
    pub fresh: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SandboxState {
    Starting,
    Ready,
    Updating,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sandbox {
    pub id: String,
    pub agent: String,
    pub title: String,
    /// Its throwaway wadspace.
    pub ws: String,
    pub build: u32,
    pub state: SandboxState,
    pub items: Vec<CheckItem>,
}

impl Sandbox {
    /// A new checklist from the agent: what's unchanged keeps its verdict,
    /// and what changed is asked about again.
    pub fn set_items(&mut self, items: Vec<CheckItem>, build: u32) {
        let old = std::mem::take(&mut self.items);
        self.items = items
            .into_iter()
            .take(MAX_ITEMS)
            .map(|mut new| {
                new.done = new.done.min(4);
                match old.iter().find(|o| o.id == new.id) {
                    Some(o) if o.label == new.label && o.how == new.how && o.done == new.done => {
                        CheckItem { fresh: false, ..o.clone() }
                    }
                    Some(_) => CheckItem { fresh: self.build > 0 || build > 1, verdict: None, ..new },
                    None => CheckItem { fresh: build > 1, verdict: None, ..new },
                }
            })
            .collect();
    }

    /// "K of N checked · M with issues".
    pub fn checked_words(&self) -> String {
        let done = self.items.iter().filter(|i| i.verdict.is_some()).count();
        let issues = self.items.iter().filter(|i| i.verdict == Some(Verdict::Issue)).count();
        format!("{done} of {} checked · {issues} with issues", self.items.len())
    }

    /// How done the agent says it is, overall, 0 to 100.
    pub fn percent(&self) -> u32 {
        if self.items.is_empty() {
            return 0;
        }
        let sum: u32 = self.items.iter().map(|i| u32::from(i.done)).sum();
        sum * 100 / (self.items.len() as u32 * 4)
    }

    /// Every item finished, and none with an issue: it can ship from here.
    pub fn looks_good(&self) -> bool {
        !self.items.is_empty()
            && self.items.iter().all(|i| i.done == 4 && matches!(i.verdict, Some(Verdict::Works | Verdict::Skip)))
    }

    /// What goes back to the agent: the spike's words.
    pub fn report(&self) -> String {
        let count = |v| self.items.iter().filter(|i| i.verdict == Some(v)).count();
        let mut out = format!(
            "{} works, {} with issues, {} skipped.",
            count(Verdict::Works),
            count(Verdict::Issue),
            count(Verdict::Skip)
        );
        for i in self.items.iter().filter(|i| i.verdict == Some(Verdict::Issue)) {
            out.push_str(&format!("\n• {}: {}", i.label, i.note));
        }
        out
    }
}

/// What the tray's flyout and the rail say an agent wants.
pub fn wants(a: &Agent, s: Option<&Sandbox>) -> Option<String> {
    if a.status != AgentStatus::NeedsYou {
        return None;
    }
    Some(match (&a.ask, s.map(|s| (s.state, s.build))) {
        (Some(Ask::Sandbox { .. }), _) => "Wants a sandbox to try it with you".into(),
        (Some(Ask::Ship), _) => "Open a pull request, or a sandbox after all?".into(),
        (None, Some((SandboxState::Ready, b))) => format!("Build {b} is in its sandbox: try it"),
        (None, Some(_)) => "Its sandbox is getting ready".into(),
        (None, None) => a.now.clone(),
    })
}

/// Text from a wadspace, cut to size.
pub fn capped(text: &str) -> String {
    text.chars().take(MAX_TEXT).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, done: u8) -> CheckItem {
        CheckItem {
            id: id.into(),
            label: id.into(),
            how: "h".into(),
            done,
            verdict: None,
            note: String::new(),
            fresh: false,
        }
    }

    fn sandbox() -> Sandbox {
        Sandbox {
            id: "sandbox-1".into(),
            agent: "a".into(),
            title: "t".into(),
            ws: "w".into(),
            build: 1,
            state: SandboxState::Ready,
            items: vec![],
        }
    }

    #[test]
    fn a_new_build_keeps_what_didnt_change_and_asks_again_about_what_did() {
        let mut s = sandbox();
        s.set_items(vec![item("a", 4), item("b", 2)], 1);
        assert!(s.items.iter().all(|i| !i.fresh));
        s.items[0].verdict = Some(Verdict::Works);
        s.items[1].verdict = Some(Verdict::Issue);
        s.items[1].note = "pinned twice".into();
        s.set_items(vec![item("a", 4), item("b", 4), item("c", 4)], 2);
        assert_eq!((s.items[0].verdict, s.items[0].fresh), (Some(Verdict::Works), false));
        assert_eq!((s.items[1].verdict, s.items[1].fresh), (None, true));
        assert_eq!((s.items[2].verdict, s.items[2].fresh), (None, true));
    }

    #[test]
    fn it_reads_as_the_spikes() {
        let mut s = sandbox();
        s.set_items(vec![item("a", 4), item("b", 2)], 1);
        s.items[1].verdict = Some(Verdict::Issue);
        s.items[1].note = "pinned twice".into();
        assert_eq!(s.checked_words(), "1 of 2 checked · 1 with issues");
        assert_eq!(s.percent(), 75);
        assert_eq!(s.report(), "0 works, 1 with issues, 0 skipped.\n• b: pinned twice");
        assert!(!s.looks_good());
        s.items[1].done = 4;
        s.items[1].verdict = Some(Verdict::Skip);
        s.items[0].verdict = Some(Verdict::Works);
        assert!(s.looks_good());
    }

    #[test]
    fn what_an_agent_says_is_bounded() {
        let mut s = sandbox();
        s.set_items((0..100).map(|i| item(&i.to_string(), 9)).collect(), 1);
        assert_eq!(s.items.len(), MAX_ITEMS);
        assert_eq!(s.items[0].done, 4);
        assert_eq!(capped(&"é".repeat(MAX_TEXT + 5)).chars().count(), MAX_TEXT);
        let mut a = Agent::new("a".into(), "w".into(), "t".into(), 0);
        for _ in 0..MAX_NOTES + 5 {
            a.note(AgentNote::Say { text: "x".into() });
        }
        assert_eq!(a.notes.len(), MAX_NOTES);
    }

    #[test]
    fn what_it_wants_is_said_as_the_spike_says_it() {
        let mut a = Agent::new("a".into(), "w".into(), "t".into(), 0);
        assert_eq!(wants(&a, None), None);
        a.status = AgentStatus::NeedsYou;
        a.ask = Some(Ask::Sandbox { title: "t".into(), why: "w".into() });
        assert_eq!(wants(&a, None).as_deref(), Some("Wants a sandbox to try it with you"));
        a.ask = None;
        assert_eq!(wants(&a, Some(&sandbox())).as_deref(), Some("Build 1 is in its sandbox: try it"));
    }
}
