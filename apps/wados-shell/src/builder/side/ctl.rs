//! `wadsctl side …`: the sidebar's Apps, Add-ons and Secrets tabs, for
//! tests and scripts. Each answer is one line.
//!
//! | Command | What it does |
//! |---|---|
//! | `side` | the tab, the wadspace, how many rows, picks, and the menu or dialog open |
//! | `side rows [prefix]` | every row: `id=flags@x,y`, flags `lit`, `picked`, `off`, `hidden` (scrolled out of sight) or `-`; x, y its middle on the screen (logical pixels) |
//! | `side row <id>` | one row: its flags, its box on the screen, its label |
//! | `side show <id>` | scrolls the tab so it's in sight |
//! | `side click <id>` | clicks it (a pill in Secrets: picks it); a menu or dialog it opens is said |
//! | `side menu <id>` | its right-click menu (a pill in Secrets, a disc in Add-ons) |
//! | `side open` | the menu open: what it's for and its items (`+off`, `+checked`, `@x,y`) |
//! | `side choose <item>` | picks an item of the menu open (`key:work-web`, `key:new`, `own-key:new`) |
//! | `side ask` | the dialog open: its title, its fields and its OK (`@x,y`) |
//! | `side fill <field> <text…>` | types into the dialog's field, or a row that's an entry (`command-input`) |
//! | `side ok` | the dialog's OK |
//! | `side enter <id>` | Enter in a row that's an entry |
//! | `side close` | closes the menu and the dialog |
//!
//! Row ids are the spike's test hooks: the sidebar's tabs, `tab:<tab>`
//! (where to click for one); `app:<app>`; `suggest:<item>`,
//! `addon:<key>`, `focus`, `site-add`, `command:<id>`,
//! `command-remove:<id>`, `command-kind`, `command-input`, `command-add`,
//! `disc-upload`; `pill:<variant>`, `group:<home|key>`,
//! `key-select:<home|key|variant>`, `own-key`, `unpick`. A disc's pill
//! (`addon:dolphin:<id>`) has one too: `disc-remove:<id>` takes it off the
//! machine.
//!
//! Screen points: the Builder's window is labwc's `builder` region, down
//! the right of the screen to its bottom, `GAP` in (rc.xml's `<gap>`):
//! its origin is worked out from its size.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::prelude::*;

use super::kit;
use crate::builder::{Builder, Tab};

/// labwc's gap around a snapped window (host/etc/xdg/labwc/rc.xml).
const GAP: f64 = 10.0;

/// One thing in a tab, for wadsctl.
pub struct Row {
    pub id: String,
    pub widget: gtk::Widget,
    pub lit: bool,
    pub picked: bool,
    pub label: String,
    /// Its right-click menu, if it has one.
    pub menu: Option<Rc<dyn Fn()>>,
}

impl Row {
    pub fn new(id: impl Into<String>, widget: &impl IsA<gtk::Widget>, label: impl Into<String>) -> Self {
        Row {
            id: id.into(),
            widget: widget.clone().upcast(),
            lit: false,
            picked: false,
            label: label.into(),
            menu: None,
        }
    }

    pub fn lit(mut self, on: bool) -> Self {
        self.lit = on;
        self
    }

    pub fn picked(mut self, on: bool) -> Self {
        self.picked = on;
        self
    }

    pub fn menu(mut self, f: Rc<dyn Fn()>) -> Self {
        self.menu = Some(f);
        self
    }
}

/// A tab's panel, as wadsctl reads it.
pub trait Rows {
    fn rows(&self) -> Vec<Row>;
}

thread_local! {
    static PANELS: RefCell<Vec<(Tab, Weak<dyn Rows>)>> = const { RefCell::new(Vec::new()) };
    static WIRED: Cell<bool> = const { Cell::new(false) };
}

/// `p` is `tab`'s panel now (the window's newest); `wadsctl side` is wired up once.
pub fn register(b: &Rc<Builder>, tab: Tab, p: Weak<dyn Rows>) {
    PANELS.with(|ps| {
        let mut ps = ps.borrow_mut();
        ps.retain(|(t, _)| *t != tab);
        ps.push((tab, p));
    });
    if WIRED.with(|w| w.replace(true)) {
        return;
    }
    // Not now: the window may be being made for a wadsctl command, which has the shell's handlers.
    let w = Rc::downgrade(b);
    gtk::glib::idle_add_local_once(move || {
        let Some(b) = w.upgrade() else { return };
        let w = Rc::downgrade(&b);
        b.shell.on_ctl("side", move |args| match w.upgrade() {
            Some(b) => kit::by_ctl(|| answer(&b, args)),
            None => "error gone".into(),
        });
    });
}

fn panel(tab: Tab) -> Option<Rc<dyn Rows>> {
    PANELS.with(|ps| ps.borrow().iter().find(|(t, _)| *t == tab).and_then(|(_, w)| w.upgrade()))
}

const USAGE: &str = "error side [rows [prefix]|row <id>|show <id>|click <id>|menu <id>|open|choose <item>|ask|fill <field> <text>|ok|enter <id>|close]";

fn answer(b: &Rc<Builder>, args: &[&str]) -> String {
    match args {
        ["open"] => return said_menu(b, true).unwrap_or_else(|| "error no menu is open".into()),
        ["choose", item] => {
            return match kit::choose(item) {
                Ok(()) => "ok".into(),
                Err(e) => format!("error {e}"),
            };
        }
        ["ask"] => return said_ask(b, true).unwrap_or_else(|| "error no dialog is open".into()),
        ["ok"] => {
            return match kit::ask_ok() {
                Ok(()) => match kit::open_ask() {
                    // Still open: it said why.
                    Some(_) => "error it's still open (it says why)".into(),
                    None => "ok".into(),
                },
                Err(e) => format!("error {e}"),
            };
        }
        ["close"] => {
            kit::close_menu();
            kit::close_ask();
            return "ok".into();
        }
        _ => {}
    }
    if !b.is_open() {
        return "error the Builder isn't open".into();
    }
    let tab = b.tab();
    let Some(p) = panel(tab) else { return format!("error the {} tab isn't this one's", tab.id()) };
    let mut rows = p.rows();
    if let Some(first) = rows.first() {
        rows.splice(0..0, tabs(&first.widget));
    }
    let find = |id: &str| rows.iter().find(|r| r.id == id);
    match args {
        [] => format!(
            "ok tab={} ws={} rows={} picked={} menu={} ask={}",
            tab.id(),
            b.selected().unwrap_or_else(|| "-".into()),
            rows.len(),
            rows.iter().filter(|r| r.picked).count(),
            kit::open_menu().map(|(of, _)| of).unwrap_or_else(|| "-".into()),
            kit::open_ask().map(|a| a.title.replace(' ', "_")).unwrap_or_else(|| "-".into()),
        ),
        ["rows"] | ["rows", _] => {
            let prefix = args.get(1).copied().unwrap_or("");
            let mut out = vec!["ok".to_string()];
            for r in rows.iter().filter(|r| r.id.starts_with(prefix)) {
                let (x, y, w, h) = screen_box(b, &r.widget).unwrap_or_default();
                out.push(format!("{}={}@{},{}", r.id, flags(r), (x + w / 2.0).round(), (y + h / 2.0).round()));
            }
            out.join(" ")
        }
        ["row", id] => match find(id) {
            Some(r) => {
                let (x, y, w, h) = screen_box(b, &r.widget).unwrap_or_default();
                format!(
                    "ok id={} lit={} picked={} off={} shown={} x={} y={} w={} h={} label={}",
                    r.id,
                    r.lit,
                    r.picked,
                    !r.widget.is_sensitive(),
                    shown(&r.widget),
                    x.round(),
                    y.round(),
                    w.round(),
                    h.round(),
                    r.label
                )
            }
            None => format!("error no row {id}"),
        },
        ["show", id] => match find(id) {
            Some(r) => {
                scroll_to(&r.widget);
                "ok".into()
            }
            None => format!("error no row {id}"),
        },
        ["click", id] => match find(id) {
            Some(r) if !r.widget.is_sensitive() => {
                let why = r.widget.tooltip_text().map(|t| format!(": {t}")).unwrap_or_default();
                format!("error {id} is off{why}")
            }
            Some(r) => match r.widget.downcast_ref::<gtk::Button>() {
                Some(button) => {
                    button.emit_clicked();
                    after(b)
                }
                None => format!("error {id} isn't a button"),
            },
            None => format!("error no row {id}"),
        },
        ["menu", id] => match find(id).and_then(|r| r.menu.clone()) {
            Some(f) => {
                f();
                after(b)
            }
            None => format!("error {id} has no menu"),
        },
        ["fill", field, text @ ..] => {
            let text = text.join(" ");
            if let Some(e) = find(field).and_then(|r| r.widget.downcast_ref::<gtk::Entry>().cloned()) {
                e.set_text(&text);
                return "ok".into();
            }
            match kit::fill(field, &text) {
                Ok(()) => "ok".into(),
                Err(e) => format!("error {e}"),
            }
        }
        ["enter", id] => match find(id).and_then(|r| r.widget.downcast_ref::<gtk::Entry>().cloned()) {
            Some(e) => {
                e.emit_activate();
                "ok".into()
            }
            None => format!("error {id} isn't an entry"),
        },
        _ => USAGE.into(),
    }
}

/// The sidebar's tabs (`tab:apps`…), found from a widget in it: where to click for one.
fn tabs(w: &gtk::Widget) -> Vec<Row> {
    let mut side = w.parent();
    while let Some(s) = &side {
        if s.has_css_class("builder-side") {
            break;
        }
        side = s.parent();
    }
    let mut out = vec![];
    let mut stack: Vec<gtk::Widget> = side.into_iter().collect();
    while let Some(n) = stack.pop() {
        if n.has_css_class("builder-tab") {
            if let Some(t) = Tab::ALL.into_iter().find(|t| n.has_css_class(t.id())) {
                out.push(Row::new(format!("tab:{}", t.id()), &n, t.id()).lit(n.has_css_class("on")));
            }
            continue;
        }
        let mut c = n.last_child();
        while let Some(k) = c {
            c = k.prev_sibling();
            stack.push(k);
        }
    }
    out
}

/// What a click opened, if anything (where its items are comes once it's
/// placed: `side open`, `side ask`).
fn after(b: &Builder) -> String {
    if let Some(m) = said_menu(b, false) {
        return m;
    }
    if let Some(a) = said_ask(b, false) {
        return a;
    }
    "ok".into()
}

/// A widget's middle on the screen, as `@x,y` (none if it isn't laid out yet).
fn at(b: &Builder, w: &gtk::Widget) -> String {
    match screen_box(b, w) {
        Some((x, y, w, h)) if w > 0.0 => format!("@{},{}", (x + w / 2.0).round(), (y + h / 2.0).round()),
        _ => String::new(),
    }
}

/// The menu open: `ok menu=<what it's for> items=<id>[+off][+checked][@x,y] …`.
fn said_menu(b: &Builder, points: bool) -> Option<String> {
    let (of, items) = kit::open_menu()?;
    let list: Vec<String> = items
        .into_iter()
        .map(|i| {
            let mut s = i.id;
            if !i.on {
                s.push_str("+off");
            }
            if i.checked {
                s.push_str("+checked");
            }
            if points {
                s.push_str(&at(b, &i.row));
            }
            s
        })
        .collect();
    Some(format!("ok menu={of} items={}", list.join(" ")))
}

/// The dialog open: `ok ask=<title> fields=<key>=<text>[@x,y] … [ok@x,y]`.
fn said_ask(b: &Builder, points: bool) -> Option<String> {
    let a = kit::open_ask()?;
    let here = |w: &gtk::Widget| if points { at(b, w) } else { String::new() };
    let fields: Vec<String> =
        a.fields.into_iter().map(|(k, v, w)| format!("{k}={}{}", v.replace(' ', "_"), here(&w))).collect();
    let ok = if points { format!(" ok{}", at(b, &a.ok)) } else { String::new() };
    Some(format!("ok ask={} fields={}{ok}", a.title.replace(' ', "_"), fields.join(" ")))
}

fn flags(r: &Row) -> String {
    let mut f = vec![];
    if r.lit {
        f.push("lit");
    }
    if r.picked {
        f.push("picked");
    }
    if !r.widget.is_sensitive() {
        f.push("off");
    }
    if !shown(&r.widget) {
        f.push("hidden");
    }
    if f.is_empty() { "-".into() } else { f.join("+") }
}

/// Its box on the screen (logical pixels): in the Builder's window, or in
/// a popover of it (a menu, a dialog: a surface of its own, placed from the
/// window's).
pub fn screen_box(b: &Builder, w: &gtk::Widget) -> Option<(f64, f64, f64, f64)> {
    let root = w.root()?;
    let win: &gtk::Widget = root.upcast_ref();
    let (sw, sh) = b.shell.size();
    let ox = sw as f64 - GAP - win.width() as f64;
    let oy = sh as f64 - GAP - win.height() as f64;
    let native = w.native()?;
    let nw: &gtk::Widget = native.upcast_ref();
    if nw == win {
        let r = w.compute_bounds(win)?;
        return Some((ox + r.x() as f64, oy + r.y() as f64, r.width() as f64, r.height() as f64));
    }
    // A popup: placed from the window's surface (its shadow in), its widgets from its own.
    let popup = native.surface().and_downcast::<gtk::gdk::Popup>()?;
    let (wx, wy) = win.native()?.surface_transform();
    let (px, py) = native.surface_transform();
    let r = w.compute_bounds(nw)?;
    let x = ox - wx + popup.position_x() as f64 + px + r.x() as f64;
    let y = oy - wy + popup.position_y() as f64 + py + r.y() as f64;
    Some((x, y, r.width() as f64, r.height() as f64))
}

/// In sight: drawn, and inside its tab's scrolled view.
fn shown(w: &gtk::Widget) -> bool {
    if !w.is_drawable() {
        return false;
    }
    let Some(sc) = w.ancestor(gtk::ScrolledWindow::static_type()) else { return true };
    let Some(r) = w.compute_bounds(&sc) else { return false };
    let mid = r.y() + r.height() / 2.0;
    mid >= 0.0 && mid <= sc.height() as f32
}

/// Its tab scrolled so it's in the middle (as near as it goes).
fn scroll_to(w: &gtk::Widget) {
    let Some(sc) = w.ancestor(gtk::ScrolledWindow::static_type()).and_downcast::<gtk::ScrolledWindow>() else {
        return;
    };
    let Some(r) = w.compute_bounds(&sc) else { return };
    let adj = sc.vadjustment();
    let page = adj.page_size();
    let want = adj.value() + r.y() as f64 + r.height() as f64 / 2.0 - page / 2.0;
    adj.set_value(want.clamp(adj.lower(), (adj.upper() - page).max(adj.lower())));
}
