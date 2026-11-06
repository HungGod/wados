//! Which display is the primary, and following the displays as they
//! change, without a restart: one plugged in or out, turned off or on,
//! moved, scaled or resized in Settings, or another made the primary.
//!
//! The primary is the one kept with the displays' layout (Settings ▸
//! Displays sets it: desk.rs reads it), else `WADOS_PRIMARY`'s connector,
//! else the first. When anything changes (GDK's monitors, their geometry or
//! scale, labwc's heads, the kept layouts), the shell's displays are worked
//! out again a moment later (a burst of changes is one) and, if they're
//! not as they were, `Shell::set_monitors` moves the primary's surfaces
//! and tells everyone who builds something per display (machine.rs's
//! wallpapers and layout, the other displays' bars, the reveal bars).

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::{gdk, glib, prelude::*};
use wad_desk::layout;

use crate::desk::Desk;
use crate::shell::Shell;

/// The primary and the rest, among `all` (GDK's, in labwc's order).
pub fn pick(desk: &Desk, all: &[gdk::Monitor]) -> Option<(gdk::Monitor, Vec<gdk::Monitor>)> {
    let connectors: Vec<String> = all.iter().map(connector).collect();
    // As Settings works it out (wad-desk's primary_of), from labwc's heads;
    // GDK's first when there's no output management.
    let p = desk.primary();
    let env = std::env::var("WADOS_PRIMARY").ok();
    let i = layout::pick_primary(&connectors, p.as_deref(), env.as_deref());
    let primary = all.get(i)?.clone();
    let others = all.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, m)| m.clone()).collect();
    Some((primary, others))
}

/// A display as the shell's menus say it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Display {
    /// 1 is the primary.
    pub n: usize,
    pub name: String,
    pub connector: String,
}

pub fn numbered(primary: &gdk::Monitor, others: &[gdk::Monitor]) -> Vec<Display> {
    std::iter::once(primary)
        .chain(others)
        .enumerate()
        .map(|(i, m)| {
            let c = connector(m);
            let name = display_name(m.model().as_deref(), &c);
            Display { n: i + 1, name, connector: c }
        })
        .collect()
}

/// Its model, unless it hasn't one: wlroots says "Unknown" for those.
fn display_name(model: Option<&str>, connector: &str) -> String {
    model.filter(|m| !m.is_empty() && *m != "Unknown").unwrap_or(connector).to_string()
}

/// What left and what came, between two lists of displays.
pub fn changes(was: &[Display], now: &[Display]) -> (Vec<Display>, Vec<Display>) {
    let has = |l: &[Display], d: &Display| l.iter().any(|x| x.connector == d.connector);
    (was.iter().filter(|d| !has(now, d)).cloned().collect(), now.iter().filter(|d| !has(was, d)).cloned().collect())
}

/// The toast for a display gone: `spaces` are what went to the background.
pub fn unplugged_body(spaces: &[String]) -> String {
    match spaces {
        [] => "Its windows moved here.".into(),
        [one] => format!("Its windows moved here; {one} is still running, in the background."),
        many => format!("Its windows moved here; {} are still running, in the background.", many.join(", ")),
    }
}

pub fn connector(m: &gdk::Monitor) -> String {
    m.connector().map(|c| c.to_string()).unwrap_or_default()
}

/// The displays as the shell lays them out: a change in this is a change.
fn signature(primary: &gdk::Monitor, others: &[gdk::Monitor]) -> String {
    std::iter::once(primary)
        .chain(others)
        .map(|m| {
            let g = m.geometry();
            format!("{}@{},{}:{}x{}*{}", connector(m), g.x(), g.y(), g.width(), g.height(), m.scale())
        })
        .collect::<Vec<_>>()
        .join(";")
}

pub fn monitors() -> Vec<gdk::Monitor> {
    gdk::Display::default()
        .map(|d| d.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).filter(|m| m.is_valid()).collect())
        .unwrap_or_default()
}

struct Follow {
    shell: Weak<Shell>,
    last: RefCell<String>,
    pending: RefCell<Option<glib::SourceId>>,
    /// The monitors whose geometry and scale are watched.
    watched: RefCell<HashSet<String>>,
}

/// Follows the displays for the rest of the session.
pub fn follow(shell: &Rc<Shell>) {
    let f = Rc::new(Follow {
        shell: Rc::downgrade(shell),
        last: RefCell::new(signature(&shell.primary, &shell.others)),
        pending: RefCell::default(),
        watched: RefCell::default(),
    });
    if let Some(d) = gdk::Display::default() {
        let w = Rc::downgrade(&f);
        d.monitors().connect_items_changed(move |_, _, _, _| {
            if let Some(f) = w.upgrade() {
                f.soon();
            }
        });
    }
    let w = Rc::downgrade(&f);
    shell.desk.on_layout(move || {
        if let Some(f) = w.upgrade() {
            f.soon();
        }
    });
    f.watch_each();
    // Kept alive by its handlers' closures? They hold it weakly: keep it here.
    shell.on_ctl("monitors", {
        let f = f.clone();
        move |_| format!("ok {}", f.last.borrow())
    });
}

impl Follow {
    /// Worked out again in a moment.
    fn soon(self: &Rc<Self>) {
        if let Some(id) = self.pending.borrow_mut().take() {
            id.remove();
        }
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(250), move || {
            if let Some(f) = w.upgrade() {
                f.pending.borrow_mut().take();
                f.now();
            }
        });
        *self.pending.borrow_mut() = Some(id);
    }

    fn watch_each(self: &Rc<Self>) {
        for m in monitors() {
            let key = format!("{:p}", m.as_ptr());
            if !self.watched.borrow_mut().insert(key) {
                continue;
            }
            for prop in ["geometry", "scale"] {
                let w = Rc::downgrade(self);
                m.connect_notify_local(Some(prop), move |_, _| {
                    if let Some(f) = w.upgrade() {
                        f.soon();
                    }
                });
            }
        }
    }

    fn now(self: &Rc<Self>) {
        let Some(shell) = self.shell.upgrade() else { return };
        self.watch_each();
        let all = monitors();
        let Some((primary, others)) = pick(&shell.desk, &all) else {
            // No display at all (the only one off): keep what there was until one's back.
            tracing::warn!("displays: none left; waiting for one");
            return;
        };
        let sig = signature(&primary, &others);
        if *self.last.borrow() == sig {
            return;
        }
        tracing::info!("displays: {} (was {})", sig, self.last.borrow());
        *self.last.borrow_mut() = sig;
        shell.set_monitors(primary, others);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(n: usize, c: &str) -> Display {
        Display { n, name: c.to_uppercase(), connector: c.into() }
    }

    #[test]
    fn swapping_the_primary_is_neither_a_plug_nor_an_unplug() {
        let (gone, new) = changes(&[d(1, "a"), d(2, "b")], &[d(1, "b"), d(2, "a")]);
        assert!(gone.is_empty() && new.is_empty());
    }

    #[test]
    fn a_display_leaving_and_one_coming_are_told_apart() {
        let (gone, new) = changes(&[d(1, "a"), d(2, "b")], &[d(1, "a"), d(2, "c")]);
        assert_eq!(gone, [d(2, "b")]);
        assert_eq!(new, [d(2, "c")]);
    }

    #[test]
    fn a_display_with_no_model_goes_by_its_connector() {
        assert_eq!(display_name(Some("DELL U2720Q"), "DP-1"), "DELL U2720Q");
        assert_eq!(display_name(Some("Unknown"), "HDMI-A-1"), "HDMI-A-1");
        assert_eq!(display_name(None, "DP-2"), "DP-2");
    }

    #[test]
    fn the_unplug_toast_says_whats_still_running() {
        assert_eq!(unplugged_body(&[]), "Its windows moved here.");
        assert!(unplugged_body(&["Krita".into()]).contains("Krita is still running"));
        assert!(unplugged_body(&["A".into(), "B".into()]).contains("A, B are still running"));
    }
}
