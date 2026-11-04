//! The Builder's board (`Builder.svelte`'s board and slot): a stack per
//! wadspace open in it, left to right, each as wide as half the board (two
//! side by side, more scrolling).
//!
//! | Here | The spike's |
//! |---|---|
//! | this file: the stacks, the slot, drags and drops | `Builder.svelte` (`.board`, `.slot`) |
//! | [`stack`] | `Stack.svelte` |
//! | [`stack`]'s pills, [`preview::thing_icon`] | `Pill.svelte`, `ThingIcon.svelte` |
//! | [`preview::desk`] | `DeskPreview.svelte` (compact) |
//! | [`pieces`] (pure) | `keys.ts` `pieceOf`, `chipMenu.ts`, Stack.svelte's state and `boxTarget` |
//! | [`menu`] | `ui/MenuList.svelte` |
//! | [`ctl`] | `wadsctl stack …` |
//!
//! It's one drag source and one drop target, which look under the pointer
//! for the part there: GTK's targets nest badly (the innermost takes every
//! drop, interested or not), and the spike's deepest *interested* target
//! decides. A drop on a key's box, else on a stack (`drops::offer_on`), else
//! the board: a wadspace from the desktop opens here (`opens`). While
//! something `drops::offer_fresh` takes is held, the **slot** shows where
//! its new stack would go (just right of the stack it came from, else after
//! the last); only there does a new one come of it (`drops::land_fresh`).

pub mod cells;
pub mod ctl;
pub mod menu;
pub mod pieces;
pub mod preview;
pub mod stack;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::{gdk, glib, prelude::*};
use wad_model::edit::Op;
use wad_model::variants::find_variant;
use wad_model::{AppId, SessionDisplay};

use super::{Builder, drops};
use crate::dnd::{self, Held, Verdict};
use menu::Menu;
use pieces::Act;
use stack::Stack;

const CSS: &str = include_str!("board.css");

thread_local! {
    static CSS_ADDED: Cell<bool> = const { Cell::new(false) };
    /// The board shown now (the window's), for `wadsctl stack`.
    static CURRENT: RefCell<Weak<Board>> = const { RefCell::new(Weak::new()) };
}

/// Where letting go would land.
#[derive(Debug, Clone, PartialEq)]
enum Zone {
    Slot,
    /// A key's box: a thing of its own signs in with its key.
    Key {
        ws: String,
        app: AppId,
        key: String,
        variant: String,
    },
    Stack(String),
    /// The rest of the board: a wadspace from the desktop opens here.
    Board(String),
}

/// The slot, while something it takes is held.
#[derive(Debug, Clone, PartialEq)]
struct SlotAt {
    /// Just right of this stack (none: first).
    after: Option<String>,
    hint: String,
}

/// A thing's menu, open.
struct Open {
    ws: String,
    part: String,
    menu: Rc<Menu>,
}

pub struct Board {
    builder: Weak<Builder>,
    me: Weak<Board>,
    root: gtk::Overlay,
    scroll: gtk::ScrolledWindow,
    row: gtk::Box,
    empty: gtk::Box,
    slot: gtk::Box,
    slot_hint: gtk::Label,
    slot_sub: gtk::Label,
    stacks: RefCell<Vec<Rc<Stack>>>,
    /// A stack's width.
    col: Cell<i32>,
    slot_at: RefCell<Option<SlotAt>>,
    /// What's lit under the pointer.
    hot: RefCell<Option<Zone>>,
    /// Where the last offer was and what it was over: the drop lands there.
    last: RefCell<Option<(f64, f64, Zone)>>,
    menu: RefCell<Option<Open>>,
    /// The pointer, as the board last saw it (its pixels): wadsctl's aim.
    pointer: Cell<Option<(f64, f64)>>,
    /// The window's top left on the screen, when it's been told.
    origin: Cell<Option<(f64, f64)>>,
    /// Scrolling by itself while a drag's held at an edge: which way.
    edge: Cell<i32>,
    /// The stack last selected: one newly selected comes into view.
    was_selected: RefCell<Option<String>>,
    /// What's to be scrolled into view once it's laid out.
    pending: RefCell<Option<gtk::Widget>>,
}

impl Board {
    pub fn new(b: &Rc<Builder>) -> Rc<Self> {
        if let Some(d) = gdk::Display::default()
            && !CSS_ADDED.replace(true)
        {
            wad_gtk::theme::add_css(&d, CSS);
        }
        let row = gtk::Box::builder().spacing(pieces::GAP).css_classes(["stack-board"]).build();
        let viewport = gtk::Viewport::builder().child(&row).build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .child(&viewport)
            .hexpand(true)
            .vexpand(true)
            .build();
        let root = gtk::Overlay::builder().css_classes(["stack-board-root"]).child(&scroll).build();
        let empty = gtk::Box::builder()
            .spacing(8)
            .css_classes(["stack-empty"])
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .can_target(false)
            .build();
        empty.append(&wad_gtk::Icon::new("mouse-pointer-click", 18));
        empty.append(&gtk::Label::new(Some("Double-click a wadspace on the desktop to open it here.")));
        root.add_overlay(&empty);

        let slot = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .css_classes(["stack-slot"])
            .valign(gtk::Align::Fill)
            .build();
        slot.set_widget_name("slot");
        let mid = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .vexpand(true)
            .hexpand(false)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();
        let plus =
            gtk::Box::builder().css_classes(["stack-slot-plus"]).halign(gtk::Align::Center).hexpand(false).build();
        plus.set_size_request(44, 44);
        let pi = wad_gtk::Icon::new("plus", 22);
        pi.set_hexpand(true);
        pi.set_halign(gtk::Align::Center);
        pi.set_valign(gtk::Align::Center);
        plus.append(&pi);
        mid.append(&plus);
        let slot_hint = gtk::Label::builder()
            .css_classes(["stack-slot-hint", "display"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build();
        mid.append(&slot_hint);
        let slot_sub = gtk::Label::builder().label("Drop it here").css_classes(["stack-slot-sub"]).build();
        mid.append(&slot_sub);
        slot.append(&mid);

        let me = Rc::new_cyclic(|me| Self {
            builder: Rc::downgrade(b),
            me: me.clone(),
            root,
            scroll,
            row,
            empty,
            slot,
            slot_hint,
            slot_sub,
            stacks: RefCell::default(),
            // Two-thirds of the window's first width, till it's laid out.
            col: Cell::new(pieces::column(733)),
            slot_at: RefCell::default(),
            hot: RefCell::default(),
            last: RefCell::default(),
            menu: RefCell::default(),
            pointer: Cell::new(None),
            origin: Cell::new(None),
            edge: Cell::new(0),
            was_selected: RefCell::default(),
            pending: RefCell::default(),
        });
        me.wire();
        CURRENT.with(|c| *c.borrow_mut() = Rc::downgrade(&me));
        ctl::register(&b.shell);
        me
    }

    /// The board shown now.
    pub fn current() -> Option<Rc<Board>> {
        CURRENT.with(|c| c.borrow().upgrade())
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    pub fn builder(&self) -> Option<Rc<Builder>> {
        self.builder.upgrade()
    }

    fn rc(&self) -> Rc<Self> {
        self.me.upgrade().expect("the board is alive while it's used")
    }

    /// One edit (wadd's notice says what it did, with Undo when it took something out).
    pub fn op(&self, op: Op) {
        if let Some(b) = self.builder() {
            b.shell.op(op, |_| {});
        }
    }

    fn wire(self: &Rc<Self>) {
        // Each stack half the board's viewport wide.
        let w = self.me.clone();
        self.scroll.hadjustment().connect_page_size_notify(move |_| {
            let w = w.clone();
            // After this layout (sizes asked for mid-layout wait a frame anyway), as wide as it is by then.
            glib::idle_add_local_once(move || {
                let Some(b) = w.upgrade() else { return };
                let page = b.scroll.hadjustment().page_size() as i32;
                let col = pieces::column(page);
                if page > 0 && b.col.replace(col) != col {
                    b.refresh();
                }
            });
        });

        // A scroll asked for before what it's to show was laid out: when it is.
        let w = self.me.clone();
        self.scroll.hadjustment().connect_changed(move |_| {
            if let Some(b) = w.upgrade() {
                glib::idle_add_local_once(move || b.scroll_pending());
            }
        });

        // Dragging: what's under the pointer (its header: all of it; a section; a key's box; one thing).
        let w = self.me.clone();
        dnd::source(&self.root, move |x, y| w.upgrade().and_then(|b| b.drag_at(x, y)));

        // Dropping: the part under the pointer that's interested.
        let (w1, w2, w3) = (self.me.clone(), self.me.clone(), self.me.clone());
        dnd::target(
            &self.root,
            move |h, x, y| w1.upgrade().and_then(|b| b.offer(h, x, y)),
            move |h, x, y| {
                if let Some(b) = w2.upgrade() {
                    b.land(h, x, y);
                }
            },
            move |on| {
                if let Some(b) = w3.upgrade()
                    && !on
                {
                    b.light(None);
                    b.edge.set(0);
                }
            },
        );
        let w = self.me.clone();
        dnd::on_held(move |h| {
            if let Some(b) = w.upgrade() {
                b.held(h);
            }
        });

        // Where the pointer is (wadsctl checks its aim with it).
        let motion = gtk::EventControllerMotion::new();
        let w = self.me.clone();
        motion.connect_motion(move |_, x, y| {
            if let Some(b) = w.upgrade() {
                b.pointer.set(Some((x, y)));
            }
        });
        self.root.add_controller(motion);
    }

    // ------------------------------------------------------------ showing

    /// The stacks as they are now.
    pub fn refresh(&self) {
        let Some(b) = self.builder() else { return };
        let me = self.rc();
        let store = &b.shell.store;
        let selected = b.selected();
        // As wide as the viewport is now (once it's laid out).
        let page = self.scroll.hadjustment().page_size() as i32;
        if page > 0 {
            self.col.set(pieces::column(page));
        }
        let col = self.col.get();
        let mut old: HashMap<String, Rc<Stack>> =
            self.stacks.borrow_mut().drain(..).map(|s| (s.id.clone(), s)).collect();
        let mut stacks = vec![];
        for id in b.stacks() {
            let Some(x) = b.wadspace(&id) else { continue };
            let s = old.remove(&id).unwrap_or_else(|| Stack::new(&me, &id));
            let editable = b.editable(&x);
            let facts = pieces::Facts {
                away: (!editable)
                    .then(|| if x.machine.is_empty() { "Another machine".into() } else { x.machine.clone() }),
                building: store.build(&id).is_some_and(|b| b.job.is_some()),
                phase: store.run(&id).map(|r| r.state.phase),
                built: store.build(&id).map(|b| b.state.built),
            };
            let how = stack::How {
                selected: selected.as_deref() == Some(id.as_str()),
                status: pieces::status(&facts),
                tile: store.tile_state(&id),
                editable,
                col,
            };
            s.show(&me, &x, &how);
            stacks.push(s);
        }
        for s in old.into_values() {
            if s.column.parent().is_some() {
                self.row.remove(&s.column);
            }
        }
        *self.stacks.borrow_mut() = stacks;
        self.place();
        // Selected (clicked, opened in it, made from a piece of another): into view.
        if *self.was_selected.borrow() != selected {
            self.was_selected.replace(selected.clone());
            if let Some(id) = &selected {
                self.scroll_to(id);
            }
        }
        // A thing whose menu's open stays lit; gone, its menu goes.
        let open = self.menu.borrow().as_ref().map(|m| (m.ws.clone(), m.part.clone()));
        if let Some((ws, part)) = open {
            match self.stack(&ws).and_then(|s| s.part(&part)) {
                Some(w) => w.add_css_class("open"),
                None => self.close_menu(),
            }
        }
    }

    /// The stacks in order, and the slot (if it shows) just right of its stack.
    fn place(&self) {
        let slot_at = self.slot_at.borrow().clone();
        let col = self.col.get();
        self.slot.set_size_request(col, 220);
        let mut order: Vec<gtk::Widget> = vec![];
        let stacks = self.stacks.borrow();
        if let Some(at) = &slot_at {
            if at.after.is_none() || !stacks.iter().any(|s| Some(&s.id) == at.after.as_ref()) {
                order.push(self.slot.clone().upcast());
            }
            self.slot_hint.set_label(&at.hint);
        }
        for s in stacks.iter() {
            order.push(s.column.clone().upcast());
            if slot_at.as_ref().is_some_and(|a| a.after.as_ref() == Some(&s.id)) {
                order.push(self.slot.clone().upcast());
            }
        }
        if slot_at.is_none() && self.slot.parent().is_some() {
            self.row.remove(&self.slot);
        }
        let mut prev: Option<gtk::Widget> = None;
        for w in &order {
            if w.parent().is_none() {
                self.row.insert_child_after(w, prev.as_ref());
            } else {
                self.row.reorder_child_after(w, prev.as_ref());
            }
            prev = Some(w.clone());
        }
        self.empty.set_visible(order.is_empty());
    }

    /// Scrolled so `ws`'s stack shows.
    pub fn scroll_to(&self, ws: &str) {
        if let Some(s) = self.stack(ws) {
            self.show_in_view(&s.column);
        }
    }

    /// `w` (a stack, the slot) scrolled into view across, once it's laid out where it'll stay.
    fn show_in_view(&self, w: &impl IsA<gtk::Widget>) {
        *self.pending.borrow_mut() = Some(w.as_ref().clone());
        self.scroll_pending();
    }

    /// The scroll asked for, if `w`'s laid out at its width (it's done then).
    fn scroll_pending(&self) {
        let Some(w) = self.pending.borrow().clone() else { return };
        if w.parent().as_ref() != Some(self.row.upcast_ref()) {
            self.pending.replace(None);
            return;
        }
        let Some(r) = w.compute_bounds(&self.row) else { return };
        let a = self.scroll.hadjustment();
        let (x, width, page) = (r.x() as f64, r.width() as f64, a.page_size());
        if width < self.col.get() as f64 - 1.0 || page <= 0.0 || a.upper() < x + width {
            return;
        }
        const PAD: f64 = 18.0;
        let v = a.value();
        let to = if x < v {
            x - PAD
        } else if x + width > v + page {
            x + width - page + PAD
        } else {
            v
        };
        a.set_value(to.clamp(a.lower(), (a.upper() - page).max(a.lower())));
        self.pending.replace(None);
    }

    fn stack(&self, ws: &str) -> Option<Rc<Stack>> {
        self.stacks.borrow().iter().find(|s| s.id == ws).cloned()
    }

    /// The stack `w` is in.
    fn stack_of(&self, w: &gtk::Widget) -> Option<Rc<Stack>> {
        self.stacks.borrow().iter().find(|s| w == s.root.upcast_ref::<gtk::Widget>() || w.is_ancestor(&s.root)).cloned()
    }

    // ------------------------------------------------------------ drags

    fn drag_at(&self, x: f64, y: f64) -> Option<(Held, gtk::Widget)> {
        let w = self.root.pick(x, y, gtk::PickFlags::DEFAULT)?;
        self.stack_of(&w)?.drag_at(&w)
    }

    /// Something's picked up (the slot shows, if it takes it) or let go.
    fn held(&self, h: Option<&Held>) {
        let at = h.and_then(|h| {
            let b = self.builder()?;
            let v = self.slot_offer(&b, h)?;
            Some(SlotAt { after: pieces::slot_after(&b.stacks(), h), hint: v.hint })
        });
        if h.is_some() {
            *self.last.borrow_mut() = None;
        } else {
            self.light(None);
            self.edge.set(0);
        }
        let shows = at.is_some();
        *self.slot_at.borrow_mut() = at;
        self.place();
        if shows {
            self.slot_sub.set_label("Drop it here");
            self.show_in_view(&self.slot);
        }
    }

    /// What letting `h` go on the slot does: as open desktop (a piece is a
    /// new wadspace beside where it came from; a stack, a copy), or a
    /// wadspace from the desktop opens here (one open here already: no slot, it's only selected).
    fn slot_offer(&self, b: &Builder, h: &Held) -> Option<Verdict> {
        match h {
            Held::Space { ws, from: Some(_) } => {
                if b.stacks().iter().any(|s| s == ws) || !b.shell.store.connected() {
                    return None;
                }
                Some(Verdict { ok: true, hint: format!("Open {} here", name_of(b, ws)) })
            }
            _ => drops::offer_fresh(&b.shell, h).filter(|v| v.ok),
        }
    }

    /// The part under (x, y) that's interested in `h`, and what letting go there does.
    fn zone_at(&self, h: &Held, x: f64, y: f64) -> Option<(Zone, Verdict)> {
        let b = self.builder()?;
        let picked = self.root.pick(x, y, gtk::PickFlags::DEFAULT);
        if let Some(w) = &picked {
            let on_slot = self.slot_at.borrow().is_some()
                && (w == self.slot.upcast_ref::<gtk::Widget>() || w.is_ancestor(&self.slot));
            if on_slot && let Some(v) = self.slot_offer(&b, h) {
                return Some((Zone::Slot, v));
            }
            if let Some(s) = self.stack_of(w) {
                if let Some((app, key, _)) = s.key_at(w)
                    && let Some(x) = b.wadspace(&s.id).filter(|x| b.editable(x))
                    && let Some((variant, m)) = pieces::onto_key(&x, h, app, &key)
                {
                    return Some((
                        Zone::Key { ws: s.id.clone(), app, key, variant },
                        Verdict { ok: true, hint: m.hint },
                    ));
                }
                if let Some(v) = drops::offer_on(&b.shell, &s.id, h) {
                    return Some((Zone::Stack(s.id.clone()), v));
                }
            }
        }
        match h {
            Held::Space { ws, from: Some(_) } if b.shell.store.connected() => {
                Some((Zone::Board(ws.clone()), Verdict { ok: true, hint: format!("Open {} here", name_of(&b, ws)) }))
            }
            _ => None,
        }
    }

    fn offer(&self, h: &Held, x: f64, y: f64) -> Option<Verdict> {
        self.edge_scroll(x);
        let found = self.zone_at(h, x, y);
        self.light(found.as_ref().filter(|(_, v)| v.ok).map(|(z, _)| z.clone()));
        *self.last.borrow_mut() = found.as_ref().filter(|(_, v)| v.ok).map(|(z, _)| (x, y, z.clone()));
        found.map(|(_, v)| v)
    }

    fn land(&self, h: &Held, x: f64, y: f64) {
        // Where the drop was offered (the slot may have gone since).
        let zone = match self.last.borrow_mut().take() {
            Some((lx, ly, z)) if (lx - x).abs() < 1.0 && (ly - y).abs() < 1.0 => Some(z),
            _ => None,
        }
        .or_else(|| self.zone_at(h, x, y).filter(|(_, v)| v.ok).map(|(z, _)| z));
        self.light(None);
        let (Some(zone), Some(b)) = (zone, self.builder()) else { return };
        match zone {
            Zone::Slot => match h {
                Held::Space { ws, from: Some(_) } => b.open(ws, None),
                _ => drops::land_fresh(&b.shell, h),
            },
            Zone::Key { ws, key, variant, .. } => {
                b.shell.op(Op::SignIn { ws, variant, key: Some(key) }, |_| {});
            }
            Zone::Stack(ws) => drops::land_on(&b.shell, &ws, h),
            Zone::Board(ws) => b.open(&ws, None),
        }
    }

    /// Lights what letting go would land on: the slot, a key's box, the board's edge.
    fn light(&self, z: Option<Zone>) {
        if *self.hot.borrow() == z {
            return;
        }
        if let Some(was) = self.hot.replace(z.clone()) {
            if let Some(w) = self.zone_widget(&was) {
                w.remove_css_class("hot");
            }
            if was == Zone::Slot {
                self.slot_sub.set_label("Drop it here");
            }
        }
        if let Some(z) = &z {
            if let Some(w) = self.zone_widget(z) {
                w.add_css_class("hot");
            }
            if *z == Zone::Slot {
                self.slot_sub.set_label("Let go");
                self.show_in_view(&self.slot);
            }
        }
    }

    fn zone_widget(&self, z: &Zone) -> Option<gtk::Widget> {
        match z {
            Zone::Slot => Some(self.slot.clone().upcast()),
            Zone::Key { ws, app, key, .. } => self.stack(ws).and_then(|s| s.key_box(*app, key)),
            Zone::Stack(_) => None,
            Zone::Board(_) => Some(self.root.clone().upcast()),
        }
    }

    /// Held near the board's left or right edge: it scrolls that way, till it isn't.
    fn edge_scroll(&self, x: f64) {
        const EDGE: f64 = 40.0;
        let w = self.root.width() as f64;
        let dir = if x < EDGE {
            -1
        } else if x > w - EDGE {
            1
        } else {
            0
        };
        let was = self.edge.replace(dir);
        if dir == 0 || was != 0 {
            return;
        }
        let me = self.me.clone();
        glib::timeout_add_local(Duration::from_millis(40), move || {
            let Some(b) = me.upgrade() else { return glib::ControlFlow::Break };
            let d = b.edge.get();
            if d == 0 || dnd::held().is_none() {
                b.edge.set(0);
                return glib::ControlFlow::Break;
            }
            let a = b.scroll.hadjustment();
            a.set_value((a.value() + 18.0 * d as f64).clamp(a.lower(), a.upper() - a.page_size()));
            glib::ControlFlow::Continue
        });
    }

    // ------------------------------------------------------------ menus

    /// A thing's menu (`chipMenu.ts`), under it: its part `part` of `ws`'s stack, the variant `variant`.
    pub fn open_menu(&self, ws: &str, part: &str, variant: &str, autohide: bool) -> Option<Rc<Menu>> {
        self.close_menu();
        let b = self.builder()?;
        let x = b.wadspace(ws).filter(|x| b.editable(x))?;
        let v = find_variant(&x, variant)?;
        let anchor = self.stack(ws)?.part(part)?;
        let r = anchor.compute_bounds(&self.root)?;
        let at = gdk::Rectangle::new(
            r.x() as i32 + 6,
            r.y() as i32 + 6,
            (r.width() as i32 - 12).max(1),
            (r.height() as i32 - 6).max(1),
        );
        let (w, wsid, vid) = (self.me.clone(), ws.to_string(), variant.to_string());
        let w2 = self.me.clone();
        let menu = Menu::show(
            &self.root,
            at,
            pieces::chip_menu(&x, &v),
            autohide,
            move |act| {
                if let Some(b) = w.upgrade() {
                    b.act(&wsid, &vid, act);
                }
            },
            move || {
                if let Some(b) = w2.upgrade() {
                    b.menu_closed();
                }
            },
        );
        anchor.add_css_class("open");
        *self.menu.borrow_mut() = Some(Open { ws: ws.to_string(), part: part.to_string(), menu: menu.clone() });
        Some(menu)
    }

    pub fn close_menu(&self) {
        let m = self.menu.borrow().as_ref().map(|m| m.menu.clone());
        if let Some(m) = m {
            m.close();
        }
    }

    fn menu_closed(&self) {
        if let Some(o) = self.menu.borrow_mut().take()
            && let Some(w) = self.stack(&o.ws).and_then(|s| s.part(&o.part))
        {
            w.remove_css_class("open");
        }
    }

    /// The menu open now.
    pub fn open(&self) -> Option<Rc<Menu>> {
        self.menu.borrow().as_ref().map(|m| m.menu.clone())
    }

    /// What a thing's menu item does.
    fn act(&self, ws: &str, variant: &str, act: &Act) {
        let Some(b) = self.builder() else { return };
        let shell = &b.shell;
        match act {
            Act::Open => {
                if shell.store.is_running(ws) {
                    shell.show(ws);
                } else {
                    shell.open(ws, SessionDisplay::Screen);
                }
            }
            Act::Launch(l) => self.op(Op::SetLaunch { ws: ws.into(), launch: l.clone() }),
            Act::Wallpaper(wp) => {
                self.op(Op::SetAppWallpaper { ws: ws.into(), variant: variant.into(), wallpaper: wp.clone() })
            }
            Act::Remove => self.op(Op::Remove { ws: ws.into(), chip: variant.into() }),
            Act::Rename => {
                let Some(v) = b.wadspace(ws).and_then(|x| find_variant(&x, variant)) else { return };
                let Some(m) = self.open() else { return };
                let (w, ws, vid) = (self.me.clone(), ws.to_string(), variant.to_string());
                m.ask(&format!("Rename {}", v.name), &v.name, "Rename", move |name| {
                    if let Some(b) = w.upgrade() {
                        b.op(Op::RenameVariant { ws: ws.clone(), variant: vid.clone(), name });
                    }
                });
            }
        }
    }
}

fn name_of(b: &Builder, ws: &str) -> String {
    b.shell.store.wadspace(ws).map(|x| x.name).unwrap_or_else(|| "it".into())
}
