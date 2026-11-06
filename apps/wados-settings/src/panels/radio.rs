//! What Wi-Fi, Network and Bluetooth share: a page drawn again from what
//! wadd says (a burst of events is one draw), its calls (a button waits
//! meanwhile; a refusal shows under the page's title), rows that are
//! clicked, and the look the spike's panels add to the kit's.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};

use gtk::{glib, prelude::*};
use wad_gtk::kit::{self, Row};
use wad_wadd_client::{Failure, Wadd};

use crate::link::Link;

/// The spike's panels' own bits (WifiPanel's gear, details and hint;
/// BluetoothPanel's lead, battery and "Searching…").
const CSS: &str = r#"
.set-click:hover { background-color: rgb(255 255 255 / 0.03); }
.set-click.active .kit-row-title { color: var(--accent); font-weight: 600; }
button.set-gear { min-width: 30px; min-height: 30px; padding: 0; border-radius: 8px; border: 1px solid transparent; background: var(--surface-3); background-image: none; box-shadow: none; color: var(--muted); }
button.set-gear:hover, button.set-gear.open { color: var(--fg); border-color: var(--line-strong); }
.set-dt { font-size: 13px; color: var(--muted); }
.set-dd { font-size: 13px; }
.set-hint { font-size: 12.5px; color: var(--faint); margin: 0 4px; }
.set-lead { font-size: 13px; color: var(--muted); margin: 0 4px; }
.set-battery { font-size: 12.5px; color: var(--muted); margin-right: 4px; }
.set-searching { font-size: 12.5px; color: var(--muted); }
.set-note { font-size: 13px; color: var(--danger); margin: 0 4px; }
.set-code { font-family: monospace; font-size: 28px; font-weight: 600; letter-spacing: 6px; }
.set-muted-ic { color: var(--muted); }
"#;

thread_local! {
    static CSS_ADDED: Cell<bool> = const { Cell::new(false) };
}

fn install_css() {
    if !CSS_ADDED.with(|c| c.replace(true))
        && let Some(d) = gtk::gdk::Display::default()
    {
        wad_gtk::theme::add_css(&d, CSS);
    }
}

/// A page and its calls.
pub struct Core {
    pub page: kit::Page,
    /// What wadd refused, under the title.
    pub note: gtk::Label,
    pub link: Link,
    pending: RefCell<HashSet<String>>,
    queued: Cell<bool>,
    draw: RefCell<Option<Rc<dyn Fn()>>>,
}

impl Core {
    pub fn new(title: &str, link: Link) -> Rc<Self> {
        install_css();
        let page = kit::Page::new(title, false);
        let note = kit::label("", &["set-note"]);
        note.set_wrap(true);
        note.set_visible(false);
        Rc::new(Self {
            page,
            note,
            link,
            pending: RefCell::default(),
            queued: Cell::new(false),
            draw: RefCell::default(),
        })
    }

    /// What drawing it is (the panel's).
    pub fn on_draw(&self, f: impl Fn() + 'static) {
        *self.draw.borrow_mut() = Some(Rc::new(f));
    }

    /// Drawn again on the next idle.
    pub fn queue_draw(self: &Rc<Self>) {
        if self.queued.replace(true) {
            return;
        }
        let w = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(m) = w.upgrade() {
                m.queued.set(false);
                m.draw_now();
            }
        });
    }

    pub fn draw_now(&self) {
        let f = self.draw.borrow().clone();
        if let Some(f) = f {
            f();
        }
    }

    /// The column emptied (the note stays at its top).
    pub fn clear(&self) {
        let col = &self.page.column;
        while let Some(c) = col.first_child() {
            col.remove(&c);
        }
        col.append(&self.note);
    }

    pub fn busy(&self, what: &str) -> bool {
        self.pending.borrow().contains(what)
    }

    pub fn say(&self, what: Option<&str>) {
        match what {
            Some(t) => {
                self.note.set_label(t);
                self.note.set_visible(true);
            }
            None => self.note.set_visible(false),
        }
    }

    /// A call to wadd for `what` (busy meanwhile), then `done` with its
    /// answer; a refusal `done` leaves (it says false) shows under the title.
    pub fn call<T: Send + 'static, F>(
        self: &Rc<Self>,
        what: &str,
        call: impl FnOnce(Wadd) -> F,
        done: impl FnOnce(&Result<T, Failure>) -> bool + 'static,
    ) where
        F: std::future::Future<Output = Result<T, Failure>> + Send + 'static,
    {
        let what = what.to_string();
        self.pending.borrow_mut().insert(what.clone());
        self.say(None);
        self.queue_draw();
        let w: Weak<Core> = Rc::downgrade(self);
        self.link.call(call, move |r| {
            let Some(m) = w.upgrade() else { return };
            m.pending.borrow_mut().remove(&what);
            if !done(&r)
                && let Err(e) = &r
            {
                m.say(Some(&e.message));
            }
            m.queue_draw();
        });
    }
}

/// `row` clicked anywhere (but on its own buttons) does `f`.
pub fn clickable(row: &Row, f: impl Fn() + 'static) {
    row.root.add_css_class("set-click");
    row.root.set_cursor_from_name(Some("pointer"));
    let g = gtk::GestureClick::new();
    g.connect_released(move |g, _, x, y| {
        // A button in it answers for itself.
        if let Some(w) = g.widget()
            && let Some(hit) = w.pick(x, y, gtk::PickFlags::DEFAULT)
            && (hit.is::<gtk::Button>() || hit.ancestor(gtk::Button::static_type()).is_some_and(|b| b != w))
        {
            return;
        }
        f();
    });
    row.root.add_controller(g);
}

/// A row with a chevron at its end that opens `f` (`nav="chevron"`).
pub fn nav_row(row: &Row, f: impl Fn() + 'static) {
    let c = wad_gtk::Icon::new("chevron-right", 16);
    c.add_css_class("set-muted-ic");
    row.ctl.append(&c);
    clickable(row, f);
}

/// An inset under a row (a form, details).
pub fn inset() -> gtk::Box {
    gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).css_classes(["kit-inset"]).build()
}

/// Labels in two columns (`<dl>`): what, and its value.
pub fn details(rows: &[(&str, String, bool)]) -> gtk::Grid {
    let g = gtk::Grid::builder().column_spacing(12).row_spacing(6).build();
    for (i, (k, v, mono)) in rows.iter().enumerate() {
        let dt = kit::label(k, &["set-dt"]);
        dt.set_size_request(120, -1);
        let dd = kit::label(v, &["set-dd"]);
        dd.set_selectable(true);
        if *mono {
            dd.add_css_class("mono");
        }
        g.attach(&dt, 0, i as i32, 1, 1);
        g.attach(&dd, 1, i as i32, 1, 1);
    }
    g
}

/// An icon-only button (`Button icon=… title=…`).
pub fn icon_button(icon: &'static str, tip: &str, class: &str) -> gtk::Button {
    let b = gtk::Button::builder().child(&wad_gtk::Icon::new(icon, 15)).css_classes([class]).build();
    b.set_valign(gtk::Align::Center);
    b.set_tooltip_text(Some(tip));
    b
}

/// A widget moved out of where it was (to be put somewhere else).
pub fn take(w: &impl IsA<gtk::Widget>) {
    if let Some(p) = w.parent().and_then(|p| p.downcast::<gtk::Box>().ok()) {
        p.remove(w);
    }
}

/// What it shows, in words (`--dump`): the visible labels, each switch as
/// `[on]` or `[off]`, a text field as `[field]` (never what's in it).
pub fn dump(w: &gtk::Widget) -> String {
    fn walk(w: &gtk::Widget, out: &mut Vec<String>) {
        if !w.is_visible() {
            return;
        }
        if let Some(l) = w.downcast_ref::<gtk::Label>() {
            let t = l.label();
            if !t.is_empty() {
                out.push(t.to_string());
            }
        } else if let Some(s) = w.downcast_ref::<gtk::Switch>() {
            out.push(if s.is_active() { "[on]".into() } else { "[off]".into() });
        } else if w.is::<gtk::Text>() {
            out.push("[field]".into());
            return;
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
