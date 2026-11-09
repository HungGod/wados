//! What a panel is to Settings, and what it's given.
//!
//! A panel is one module under `panels/` with a `make` the registry
//! (`panels::REGISTRY`) names: it's made when Settings starts (so it
//! hears wadd's events from the first), and its page is built the first
//! time it's opened. Its id, label, icon and search words are `nav.rs`'s.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::{glib, prelude::*};
use wad_gtk::form::{self, Empty};
use wad_gtk::kit;
use wad_wadd_client::Incoming;

use crate::link::Link;
use crate::nav::{self, PanelInfo};

/// A Settings panel.
pub trait Panel {
    /// Its page, built the first time it's opened (once).
    fn build(&self, ctx: &Ctx) -> gtk::Widget;

    /// Each of wadd's events, from when Settings starts (before its page
    /// is built too); `Disconnected` when the line drops (it comes back by itself).
    fn on_event(&self, _ctx: &Ctx, _event: &Incoming) {}

    /// Opened (after `build`), at one of its pages (System's `sub`:
    /// "about", "datetime", "users", "updates") or wherever it was (None).
    fn show(&self, _sub: Option<&str>) {}

    /// It has what it needs to say what it shows: `--dump` waits for it (5 s at most).
    fn ready(&self) -> bool {
        true
    }

    /// What it shows, in words (`--dump`, for tests); by default the visible labels of its page.
    fn dump(&self, page: &gtk::Widget) -> String {
        dump_labels(page)
    }
}

/// How a panel's made (its module's `make`).
pub type Make = fn(&Ctx) -> Rc<dyn Panel>;

/// What a panel's given: the line to wadd, the window (for dialogs), and a
/// way to open another panel.
// The window and `open` are for the panels still to come (dialogs, links to each other).
#[allow(dead_code)]
#[derive(Clone)]
pub struct Ctx {
    pub link: Link,
    pub window: gtk::ApplicationWindow,
    open: Rc<dyn Fn(&str)>,
}

#[allow(dead_code)]
impl Ctx {
    pub fn new(link: Link, window: gtk::ApplicationWindow, open: impl Fn(&str) + 'static) -> Self {
        Self { link, window, open: Rc::new(open) }
    }

    /// Opens a panel by its id or another name ("tailscale", "updates": System's Updates), as the sidebar does.
    pub fn open(&self, page: &str) {
        let (open, page) = (self.open.clone(), page.to_string());
        // Not from inside the panel's own handler: it may be building.
        glib::idle_add_local_once(move || open(&page));
    }

    /// Shows `dialog` over the window; `answer` gets it and the id it's answered with.
    pub fn dialog(&self, dialog: form::Dialog, answer: impl Fn(&form::Open, &str) + 'static) -> form::Open {
        dialog.open(&self.window, answer)
    }
}

/// The visible labels under `w`, in order, joined with " | ".
pub fn dump_labels(w: &gtk::Widget) -> String {
    fn walk(w: &gtk::Widget, out: &mut Vec<String>) {
        if !w.is_visible() {
            return;
        }
        if let Some(l) = w.downcast_ref::<gtk::Label>() {
            let t = l.text();
            if !t.is_empty() {
                out.push(t.to_string());
            }
        }
        let mut c = w.first_child();
        while let Some(x) = c {
            walk(&x, out);
            c = x.next_sibling();
        }
    }
    let mut out = vec![];
    walk(w, &mut out);
    out.join(" | ")
}

/// What a panel that isn't here yet says.
// Every panel's built: kept for one that's added later.
#[allow(dead_code)]
pub fn being_built(label: &str) -> String {
    format!("{label} isn't here yet")
}

/// A panel that isn't here yet: its page says so (and which of its pages was asked for).
#[allow(dead_code)]
pub struct Placeholder {
    info: &'static PanelInfo,
    sub: RefCell<Option<gtk::Label>>,
}

#[allow(dead_code)]
impl Placeholder {
    /// The placeholder for panel `id` (one of `nav.rs`'s).
    pub fn make(id: &str) -> Rc<dyn Panel> {
        let info = nav::info(id).unwrap_or_else(|| panic!("no panel {id:?} in nav.rs"));
        Rc::new(Self { info, sub: RefCell::default() })
    }
}

impl Panel for Placeholder {
    fn build(&self, _: &Ctx) -> gtk::Widget {
        let page = kit::Page::new(self.info.label, false);
        let e = Empty::new(self.info.icon, &being_built(self.info.label), "It comes with a later version of Settings.");
        e.root.set_vexpand(true);
        e.root.set_valign(gtk::Align::Center);
        let sub = kit::label("", &["kit-group-desc"]);
        sub.set_xalign(0.5);
        sub.set_visible(false);
        e.actions.append(&sub);
        page.column.set_vexpand(true);
        page.add(&e.root);
        *self.sub.borrow_mut() = Some(sub);
        page.root.upcast()
    }

    fn show(&self, sub: Option<&str>) {
        if let Some(l) = &*self.sub.borrow() {
            match sub.map(nav::sub_label).filter(|s| !s.is_empty()) {
                Some(s) => {
                    l.set_label(&format!("You asked for its {s} page."));
                    l.set_visible(true);
                }
                None => l.set_visible(false),
            }
        }
    }
}
