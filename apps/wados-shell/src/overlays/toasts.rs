//! Toasts (`ui/Toasts.svelte`): wadd's notices and the shell's, top right
//! over everything (a full-screen wadspace too), the newest four; each goes
//! after six seconds (nine for an error), or on its ×. One that took
//! something out has Undo. One may have a body under its title: the shell
//! says it ([`toast_with`]), or expects it for a notice of wadd's
//! ([`expect_body`]: Copy here's "{name} is on this machine"). One of the
//! shell's may have a button of its own ([`toast_doing`]: a stream's Show
//! link, Retry).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::{glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use wad_gtk::Icon;
use wad_proto::Level;

use crate::bar::in_front;
use crate::machine::{BAR, Page};
use crate::shell::Shell;
use crate::surface::{self, Anchor, Spec};

/// The most shown at once (older ones go first).
pub const MAX: usize = 4;

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub level: Level,
    pub title: String,
    pub body: Option<String>,
    pub undo: Option<String>,
    /// A button of its own: its label (what it does is kept beside it, [`Toasts`]).
    pub action: Option<String>,
    /// A Lucide icon's name; none: its level's.
    pub icon: Option<String>,
}

/// What a toast's own button does.
pub type Act = std::rc::Rc<dyn Fn()>;

/// How long a body waits for its notice.
const EXPECTED: Duration = Duration::from_secs(600);

thread_local! {
    /// Bodies for toasts to come, by their title.
    static BODIES: RefCell<Vec<(String, String, Instant)>> = RefCell::default();
    /// Buttons for toasts to come, by their title: the label and what it does.
    static ACTIONS: RefCell<Vec<(String, String, Act, Instant)>> = RefCell::default();
}

/// The next toast titled `title` has a button `label` that does `act`.
pub fn expect_action(title: &str, label: &str, act: Act) {
    ACTIONS.with(|a| {
        let mut a = a.borrow_mut();
        a.retain(|(t, _, _, at)| t != title && at.elapsed() < EXPECTED);
        a.push((title.to_string(), label.to_string(), act, Instant::now()));
    });
}

/// The button expected for a toast titled `title` (taken).
pub fn take_action(title: &str) -> Option<(String, Act)> {
    ACTIONS.with(|a| {
        let mut a = a.borrow_mut();
        let i = a.iter().position(|(t, _, _, at)| t == title && at.elapsed() < EXPECTED)?;
        let (_, label, act, _) = a.remove(i);
        Some((label, act))
    })
}

/// A toast of the shell's with a body (if any) and a button of its own (`label`, doing `act`).
pub fn toast_doing(shell: &Shell, level: Level, title: &str, body: Option<&str>, label: &str, act: Act) {
    if let Some(b) = body {
        expect_body(title, b);
    }
    expect_action(title, label, act);
    shell.toast(level, title, None);
}

/// The next toast titled `title` has `body` under it.
pub fn expect_body(title: &str, body: &str) {
    BODIES.with(|b| {
        let mut b = b.borrow_mut();
        b.retain(|(t, _, at)| t != title && at.elapsed() < EXPECTED);
        b.push((title.to_string(), body.to_string(), Instant::now()));
    });
}

/// The body expected for a toast titled `title` (taken).
pub fn take_body(title: &str) -> Option<String> {
    BODIES.with(|b| {
        let mut b = b.borrow_mut();
        let i = b.iter().position(|(t, _, at)| t == title && at.elapsed() < EXPECTED)?;
        Some(b.remove(i).1)
    })
}

/// A toast of the shell's with a body under its title (its notice, in the bell's list, is the title).
pub fn toast_with(shell: &Shell, level: Level, title: &str, body: &str) {
    expect_body(title, body);
    shell.toast(level, title, None);
}

/// How long one stays.
pub fn lasts(level: Level) -> Duration {
    Duration::from_millis(if level == Level::Error { 9000 } else { 6000 })
}

/// The toasts showing, oldest first. Pure: `Toasts` draws it.
#[derive(Debug, Default)]
pub struct Queue {
    shown: Vec<Toast>,
    next: u64,
}

impl Queue {
    pub fn push(
        &mut self,
        level: Level,
        title: &str,
        body: Option<String>,
        undo: Option<String>,
        action: Option<String>,
    ) -> u64 {
        self.next += 1;
        // The same again (a retry, a double click) moves up rather than piling up.
        self.shown.retain(|t| !(t.title == title && t.undo == undo));
        self.shown.push(Toast { id: self.next, level, title: title.into(), body, undo, action, icon: None });
        if self.shown.len() > MAX {
            let extra = self.shown.len() - MAX;
            self.shown.drain(..extra);
        }
        self.next
    }

    pub fn dismiss(&mut self, id: u64) -> bool {
        let before = self.shown.len();
        self.shown.retain(|t| t.id != id);
        self.shown.len() != before
    }

    pub fn shown(&self) -> &[Toast] {
        &self.shown
    }
}

pub struct Toasts {
    shell: Rc<Shell>,
    win: gtk::ApplicationWindow,
    col: gtk::Box,
    queue: RefCell<Queue>,
    /// What the toasts' own buttons do, by their toast.
    acts: RefCell<Vec<(u64, Act)>>,
}

impl Toasts {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let win = surface::make(
            &shell.app,
            &shell.primary,
            &Spec {
                namespace: "wados-toasts",
                layer: Layer::Overlay,
                anchor: Anchor::Top,
                exclusive: -1,
                keyboard: KeyboardMode::None,
                height: 0,
            },
        );
        win.set_anchor(Edge::Left, false);
        win.set_margin(Edge::Right, 12);
        let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
        col.set_size_request(360, -1);
        win.set_child(Some(&col));
        Rc::new(Self { shell: shell.clone(), win, col, queue: RefCell::default(), acts: RefCell::default() })
    }

    /// A notice as a toast: its body (or one the shell expects for it), its
    /// icon, and its button (a verb of the shell's, else one it expects).
    pub fn push_notice(
        self: &Rc<Self>,
        level: Level,
        title: &str,
        undo: Option<String>,
        body: Option<String>,
        icon: Option<String>,
        verb: Option<wad_proto::NoticeAction>,
    ) {
        let body = body.or_else(|| take_body(title));
        let action = match verb.filter(|a| crate::verbs::known(&a.verb)) {
            Some(a) => {
                let s = self.shell.clone();
                let act: Act = Rc::new(move || crate::verbs::run(&s, &a.verb, &a.args));
                Some((a.label.clone(), act))
            }
            None => take_action(title),
        };
        let id = self.queue.borrow_mut().push(level, title, body, undo, action.as_ref().map(|(l, _)| l.clone()));
        if let Some(t) = self.queue.borrow_mut().shown.iter_mut().find(|t| t.id == id) {
            t.icon = icon;
        }
        {
            let mut acts = self.acts.borrow_mut();
            let shown: Vec<u64> = self.queue.borrow().shown().iter().map(|t| t.id).collect();
            acts.retain(|(i, _)| shown.contains(i));
            if let Some((_, act)) = action {
                acts.push((id, act));
            }
        }
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(lasts(level), move || {
            if let Some(m) = w.upgrade() {
                m.dismiss(id);
            }
        });
        self.draw();
    }

    pub fn dismiss(self: &Rc<Self>, id: u64) {
        if self.queue.borrow_mut().dismiss(id) {
            self.draw();
        }
    }

    /// The newest toast's own button with this label pressed (`wadsctl toasts press <label>`).
    pub fn press(self: &Rc<Self>, label: &str) -> bool {
        let t =
            self.shown().into_iter().rev().find(|t| t.action.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(label)));
        let Some(t) = t else { return false };
        let act = self.acts.borrow().iter().find(|(i, _)| *i == t.id).map(|(_, a)| a.clone());
        self.dismiss(t.id);
        if let Some(a) = act {
            a();
        }
        true
    }

    /// Its surface's size, while it's up (for `wadsctl toasts`).
    pub fn size(&self) -> Option<(i32, i32)> {
        self.win.is_visible().then(|| (self.win.width(), self.win.height()))
    }

    pub fn shown(&self) -> Vec<Toast> {
        self.queue.borrow().shown().to_vec()
    }

    /// Where they go: under the bar on the desktop, by the top over a
    /// full-screen wadspace (its bar is gone: nothing is there to avoid), else
    /// near the top.
    fn top(&self) -> i32 {
        let primary = self.shell.primary.connector().map(|c| c.to_string()).unwrap_or_default();
        let full = self
            .shell
            .placer
            .windows()
            .iter()
            .any(|t| t.wadspace().is_some() && t.fullscreen && in_front(t) && t.outputs.contains(&primary));
        if full {
            44
        } else if self.shell.page() == Page::Desktop {
            BAR + 8
        } else {
            12
        }
    }

    fn draw(self: &Rc<Self>) {
        while let Some(c) = self.col.first_child() {
            self.col.remove(&c);
        }
        let shown = self.shown();
        for t in &shown {
            self.col.append(&self.card(t));
        }
        if shown.is_empty() {
            self.win.set_visible(false);
            return;
        }
        self.win.set_margin(Edge::Top, self.top());
        // As tall as what's in it (a layer surface keeps its size otherwise).
        self.win.set_default_size(-1, 1);
        self.win.present();
    }

    fn press_id(self: &Rc<Self>, id: u64) {
        let act = self.acts.borrow().iter().find(|(i, _)| *i == id).map(|(_, a)| a.clone());
        self.dismiss(id);
        if let Some(a) = act {
            a();
        }
    }

    fn card(self: &Rc<Self>, t: &Toast) -> gtk::Box {
        let card = gtk::Box::builder().spacing(10).css_classes(["ov-toast", crate::bar::level_class(t.level)]).build();
        let ic = gtk::Box::builder().css_classes(["ov-tic"]).homogeneous(true).valign(gtk::Align::Start).build();
        let name = t.icon.as_deref().and_then(wad_gtk::icon::named).unwrap_or(crate::bar::level_icon(t.level));
        ic.append(&Icon::new(name, 16));
        card.append(&ic);
        let title = gtk::Label::builder()
            .label(&t.title)
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .css_classes(["ov-toast-title"])
            .build();
        match &t.body {
            Some(b) => {
                let text = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(2)
                    .hexpand(true)
                    .valign(gtk::Align::Center)
                    .build();
                text.append(&title);
                title.set_max_width_chars(40);
                let body = gtk::Label::builder()
                    .label(b)
                    .xalign(0.0)
                    .max_width_chars(40)
                    .wrap(true)
                    .wrap_mode(gtk::pango::WrapMode::WordChar)
                    .css_classes(["ov-toast-body"])
                    .build();
                text.append(&body);
                card.append(&text);
            }
            None => card.append(&title),
        }
        if let Some(label) = t.action.clone() {
            let b = gtk::Button::builder().label(&label).css_classes(["ov-act"]).valign(gtk::Align::Center).build();
            let (w, id) = (Rc::downgrade(self), t.id);
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.press_id(id);
                }
            });
            card.append(&b);
        }
        if let Some(undo) = t.undo.clone() {
            let b = gtk::Button::builder().label("Undo").css_classes(["ov-act"]).valign(gtk::Align::Center).build();
            let (w, id) = (Rc::downgrade(self), t.id);
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.shell.undo(&undo);
                    m.dismiss(id);
                }
            });
            card.append(&b);
        }
        let x = gtk::Button::builder()
            .child(&Icon::new("x", 14))
            .css_classes(["ov-x"])
            .tooltip_text("Dismiss")
            .valign(gtk::Align::Start)
            .build();
        let (w, id) = (Rc::downgrade(self), t.id);
        x.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.dismiss(id);
            }
        });
        card.append(&x);
        card
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_four_oldest_first() {
        let mut q = Queue::default();
        for i in 1..=6 {
            q.push(Level::Info, &format!("t{i}"), None, None, None);
        }
        let titles: Vec<&str> = q.shown().iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["t3", "t4", "t5", "t6"]);
    }

    #[test]
    fn dismissing_and_the_same_again() {
        let mut q = Queue::default();
        let a = q.push(Level::Ok, "Built", None, None, None);
        let b = q.push(Level::Info, "Moved to the Trash", None, Some("u1".into()), None);
        assert!(q.dismiss(a));
        assert!(!q.dismiss(a));
        assert_eq!(q.shown().len(), 1);
        // The same notice again replaces itself, and comes last (newest).
        q.push(Level::Warn, "Other", None, None, None);
        let c = q.push(Level::Info, "Moved to the Trash", None, Some("u1".into()), None);
        assert_ne!(b, c);
        let titles: Vec<&str> = q.shown().iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Other", "Moved to the Trash"]);
        // But a different Undo is a different toast.
        q.push(Level::Info, "Moved to the Trash", None, Some("u2".into()), None);
        assert_eq!(q.shown().len(), 3);
    }

    #[test]
    fn a_body_waits_for_its_toast_once() {
        expect_body("Pixel Art is on this machine", "Copied from Desk.");
        assert_eq!(take_body("Other"), None);
        assert_eq!(take_body("Pixel Art is on this machine").as_deref(), Some("Copied from Desk."));
        assert_eq!(take_body("Pixel Art is on this machine"), None);
    }

    #[test]
    fn a_button_waits_for_its_toast_once() {
        let hit = Rc::new(std::cell::Cell::new(0));
        let h = hit.clone();
        expect_action("Streaming WadOS", "Show link", Rc::new(move || h.set(h.get() + 1)));
        assert!(take_action("Other").is_none());
        let (label, act) = take_action("Streaming WadOS").unwrap();
        assert_eq!(label, "Show link");
        act();
        assert_eq!(hit.get(), 1);
        assert!(take_action("Streaming WadOS").is_none());
        let mut q = Queue::default();
        q.push(Level::Ok, "Streaming WadOS", Some("Public · h".into()), None, Some(label));
        assert_eq!(q.shown()[0].action.as_deref(), Some("Show link"));
    }

    #[test]
    fn errors_stay_longer() {
        assert_eq!(lasts(Level::Info), Duration::from_secs(6));
        assert_eq!(lasts(Level::Ok), Duration::from_secs(6));
        assert_eq!(lasts(Level::Error), Duration::from_secs(9));
    }
}
