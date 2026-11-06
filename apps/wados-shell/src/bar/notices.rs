//! This session's notices, for the bell (`shell/bar/Notifications.svelte`):
//! what wadd and the shell said, newest first, each with its icon, a line
//! under its title, its button (a verb the shell knows, `crate::verbs`) and
//! its Undo when it took something out. Looking at them reads them. Pure:
//! the bell draws it.

use wad_proto::{Level, NoticeAction};

/// The most the bell keeps; older ones go.
const KEEP: usize = 100;

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub id: u64,
    pub level: Level,
    pub title: String,
    /// The edit to undo, if it took something out.
    pub undo: Option<String>,
    /// When, in seconds since the epoch.
    pub at: i64,
    pub read: bool,
    pub body: Option<String>,
    /// A Lucide icon's name; none: its level's.
    pub icon: Option<String>,
    pub action: Option<NoticeAction>,
}

/// What a phase 8 notice has beside its level, title and Undo.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct More {
    pub body: Option<String>,
    pub icon: Option<String>,
    pub action: Option<NoticeAction>,
}

impl Notice {
    /// Its icon: its own (if it's one of ours), else its level's.
    pub fn icon(&self) -> &'static str {
        self.icon.as_deref().and_then(wad_gtk::icon::named).unwrap_or(level_icon(self.level))
    }
}

#[derive(Debug, Default)]
pub struct Notices {
    list: Vec<Notice>,
    next: u64,
}

impl Notices {
    #[cfg(test)]
    pub fn push(&mut self, level: Level, title: &str, undo: Option<String>, at: i64) -> u64 {
        self.push_more(level, title, undo, More::default(), at)
    }

    pub fn push_more(&mut self, level: Level, title: &str, undo: Option<String>, more: More, at: i64) -> u64 {
        self.next += 1;
        self.list.push(Notice {
            id: self.next,
            level,
            title: title.to_string(),
            undo,
            at,
            read: false,
            body: more.body,
            icon: more.icon,
            action: more.action,
        });
        if self.list.len() > KEEP {
            let extra = self.list.len() - KEEP;
            self.list.drain(..extra);
        }
        self.next
    }

    pub fn unread(&self) -> usize {
        self.list.iter().filter(|n| !n.read).count()
    }

    pub fn read_all(&mut self) {
        for n in &mut self.list {
            n.read = true;
        }
    }

    pub fn clear(&mut self) {
        self.list.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The newest `n`, newest first.
    pub fn newest(&self, n: usize) -> Vec<Notice> {
        self.list.iter().rev().take(n).cloned().collect()
    }

    /// An Undo was used: it can't be used again.
    pub fn undone(&mut self, undo: &str) {
        for n in &mut self.list {
            if n.undo.as_deref() == Some(undo) {
                n.undo = None;
            }
        }
    }
}

/// "just now", "5 min ago", "3 h ago", "2 days ago" (the spike's `ago`).
pub fn ago(now: i64, at: i64) -> String {
    let s = (now - at).max(0) as f64;
    if s < 45.0 {
        return "just now".into();
    }
    if s < 3600.0 {
        return format!("{} min ago", (s / 60.0).round() as i64);
    }
    if s < 86400.0 {
        return format!("{} h ago", (s / 3600.0).round() as i64);
    }
    let d = (s / 86400.0).round() as i64;
    format!("{d} day{} ago", if d == 1 { "" } else { "s" })
}

/// A level's icon.
pub fn level_icon(level: Level) -> &'static str {
    match level {
        Level::Info => "info",
        Level::Ok => "check",
        Level::Warn => "triangle-alert",
        Level::Error => "circle-alert",
    }
}

/// A level's CSS class.
pub fn level_class(level: Level) -> &'static str {
    match level {
        Level::Info => "wb-info",
        Level::Ok => "wb-ok",
        Level::Warn => "wb-warn",
        Level::Error => "wb-error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_and_reading_them() {
        let mut n = Notices::default();
        n.push(Level::Info, "one", None, 10);
        n.push(Level::Ok, "two", Some("u1".into()), 20);
        n.push(Level::Error, "three", None, 30);
        assert_eq!(n.unread(), 3);
        let titles: Vec<String> = n.newest(2).into_iter().map(|x| x.title).collect();
        assert_eq!(titles, ["three", "two"]);
        n.read_all();
        assert_eq!(n.unread(), 0);
        n.push(Level::Warn, "four", None, 40);
        assert_eq!(n.unread(), 1);
        n.undone("u1");
        assert!(n.newest(10).iter().all(|x| x.undo.is_none()));
        n.clear();
        assert!(n.is_empty());
    }

    #[test]
    fn a_notice_has_its_body_icon_and_button() {
        let mut n = Notices::default();
        let more = More {
            body: Some("Restart to use it.".into()),
            icon: Some("rotate-ccw".into()),
            action: Some(NoticeAction::new("Restart", "power.restart", &[])),
        };
        n.push_more(Level::Ok, "WadOS 44.2 is ready", None, more, 5);
        n.push(Level::Warn, "plain", None, 6);
        let l = n.newest(2);
        assert_eq!((l[0].icon(), l[0].body.as_deref()), ("triangle-alert", None));
        assert_eq!(l[1].icon(), "rotate-ccw");
        assert_eq!(l[1].body.as_deref(), Some("Restart to use it."));
        assert_eq!(l[1].action.as_ref().map(|a| a.label.as_str()), Some("Restart"));
    }

    #[test]
    fn it_keeps_the_last_hundred() {
        let mut n = Notices::default();
        for i in 0..130 {
            n.push(Level::Info, &format!("n{i}"), None, i);
        }
        let all = n.newest(1000);
        assert_eq!(all.len(), 100);
        assert_eq!(all[0].title, "n129");
        assert_eq!(all[99].title, "n30");
    }

    #[test]
    fn how_long_ago() {
        assert_eq!(ago(100, 90), "just now");
        assert_eq!(ago(1000, 700), "5 min ago");
        assert_eq!(ago(20_000, 9_200), "3 h ago");
        assert_eq!(ago(200_000, 100_000), "1 day ago");
        assert_eq!(ago(400_000, 100_000), "3 days ago");
        // A clock that went back is "just now", not the future.
        assert_eq!(ago(10, 50), "just now");
    }
}
