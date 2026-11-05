//! The bell's notices (phase 8, the spike's `SysEvent`): [`crate::Event::Notice`]
//! has, beside its level, title and Undo, an optional body, icon and action,
//! and whether it's a toast too or only in Notifications. Each is optional,
//! so a notice from before (level, title, undo) reads the same: a toast.

use serde::{Deserialize, Serialize};

use crate::Level;

/// A button on a notice: the shell does `verb` with `args` when it's pressed.
///
/// The verbs the shell knows (others: the button isn't shown):
/// - `power.restart`: restarts the machine (an update staged);
/// - `settings.open` `[panel]`: opens Settings at a panel (`bluetooth`, `network`, `storage`…);
/// - `app.open` `[app]`: opens one of the machine's apps (`updates`, `files`…);
/// - `wadspace.open` `[id]`: opens a wadspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoticeAction {
    /// What the button says ("Restart").
    pub label: String,
    pub verb: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

impl NoticeAction {
    pub fn new(label: &str, verb: &str, args: &[&str]) -> Self {
        Self { label: label.into(), verb: verb.into(), args: args.iter().map(|a| a.to_string()).collect() }
    }
}

impl crate::Event {
    /// A notice as before phase 8's: a toast with a title, and an Undo when
    /// it took something out. A richer one is `Event::Notice { .. }` itself.
    pub fn notice(level: Level, title: impl Into<String>, undo: Option<String>) -> Self {
        crate::Event::Notice { level, title: title.into(), undo, body: None, icon: None, action: None, toast: true }
    }
}

pub(crate) fn yes() -> bool {
    true
}

pub(crate) fn is_yes(b: &bool) -> bool {
    *b
}
