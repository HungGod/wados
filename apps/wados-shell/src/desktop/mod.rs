//! The desktop (`shell/desktop/Desktop.svelte`): icons and nothing else, on a
//! grid from the top left: this machine's wadspaces with no tag (glass
//! tiles), a folder per tag, the desktop's files (`~/Desktop`), then
//! Machines and the Trash.
//!
//! - **Where they sit** ([`grid`], kept in `~/.config/wados/desk.json` by
//!   [`arrange`]): an icon dragged to a spot stays there (onto another, the
//!   two swap; a selection moves together); the rest fill the free spots. A
//!   wadspace or folder new on the desktop goes after everything there.
//! - **Selecting** ([`select`]): a box across the background (Ctrl or Shift
//!   adds), Ctrl/Shift-clicks, Ctrl+A, Esc. Delete puts the selection in the
//!   Trash; Enter opens one; F2 renames one.
//! - **Tag folders** (`TagFolder.svelte`): double-click (or Enter) opens one
//!   beside its icon; Esc or a click elsewhere closes it. Drop a wadspace on
//!   a folder (or it open) to tag it; drag one out onto the desktop to untag
//!   it, where it's let go. Right-click to rename or delete the tag.
//! - **Files** ([`files`]): double-click opens one with the system's app for
//!   it; files dropped from other apps are copied into `~/Desktop`; the
//!   Trash is the freedesktop.org one.
//! - **Menus** ([`menu`]): a wadspace's, a folder's, a file's, the Trash's,
//!   Machines', the background's.
//! - **The Builder's drags** ([`crate::dnd`], `shell/spaces/targets.ts`): a
//!   piece of a stack (or a key's group) let go on a wadspace's icon (or one
//!   in the open folder) is copied into it; a whole stack on a folder is
//!   tagged that; anywhere else, it's open desktop (`drops::offer_fresh`: a
//!   new wadspace of just that, or a copy). The hint's under the pointer. A
//!   wadspace carried from here (one, from the desktop or out of a folder)
//!   is held there too, so the Builder's board takes it.
//!
//! The interface machine.rs uses: `new`, `widget` (sized by machine.rs to
//! the screen below the bar), `key` (the desktop surface's keys, before
//! machine.rs's own), `shown` (each time the page is entered). Tests reach it
//! with `wadsctl desk …` (see [`Desktop::ctl`]).

mod arrange;
mod files;
mod grid;
mod icons;
mod keep;
mod menu;
mod select;

use std::cell::{Cell as Flag, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use gtk::{gdk, gio, glib, graphene, prelude::*};
use indexmap::IndexMap;
use wad_model::edit::Op;
use wad_model::tags::{GENERAL_ID, tag_of};
use wad_model::{Category, SessionDisplay};
use wad_proto::Level;

use crate::builder::drops;
use crate::dnd::{self, Held, SpaceFrom, Verdict};
use crate::machine::BAR;
use crate::shell::Shell;
use crate::store::Change;
use grid::{Spot, Spots};
use icons::{Cell, Item};
use menu::{Entry, Item as MenuItem};
use select::Mods;

const CSS: &str = include_str!("desktop.css");
/// What a drag from the desktop carries, besides files: its icons' keys.
const DESK_MIME: &str = "application/x-wados-desk";
/// The most icons drawn under the pointer while a selection's carried.
const GHOSTS: usize = 12;

/// A drag from the desktop, while it's on.
#[derive(Clone, Debug)]
struct Drag {
    /// What's carried, the one pressed first.
    keys: Vec<String>,
    /// From an open folder (a wadspace, to untag or tag again).
    from_folder: bool,
    /// One wadspace, as the other surfaces know it (held there too).
    held: Option<Held>,
}

/// The box being dragged across the background.
struct Band {
    base: Vec<String>,
    x0: f64,
    y0: f64,
}

/// What's under a point.
#[derive(Clone, Debug, PartialEq)]
enum Hit {
    Icon(String),
    /// A wadspace in the open folder.
    Inside(String),
    /// The open folder, between its icons.
    Panel,
    Background,
}

/// Where a drop goes.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    /// Into a tag (a folder's icon, or it open).
    Tag {
        tag: String,
        panel: bool,
    },
    Trash,
    /// Into a folder of files on the desktop.
    Into {
        key: String,
        dir: PathBuf,
    },
    /// Kept in a wadspace (the Builder's, phase 3).
    Keep(String),
    /// Something held (a piece of a Builder stack) on a wadspace's icon:
    /// `ws:<id>`, or `in:<id>` in the open folder.
    Put(String),
    /// A spot of the grid.
    Spot(Spot),
}

/// Where something held (`dnd::Held`: a piece of a Builder stack, a whole
/// stack) goes when it's let go.
#[derive(Clone, Debug, PartialEq)]
enum Lands {
    /// Onto a wadspace (`key`: its icon's), copied in.
    On { ws: String, key: String },
    /// Into a tag (a folder's icon, or it open): a whole stack is tagged that.
    Tag { tag: String, panel: bool },
    /// Open desktop: a new wadspace of just that, or a copy.
    Fresh,
}

impl Lands {
    /// What's lit while it's over it.
    fn lit(&self) -> Option<Target> {
        match self {
            Lands::On { key, .. } => Some(Target::Put(key.clone())),
            Lands::Tag { tag, panel } => Some(Target::Tag { tag: tag.clone(), panel: *panel }),
            Lands::Fresh => None,
        }
    }
}

#[derive(Default)]
struct State {
    /// Every icon's key, in the desktop's order, and its spot.
    order: Vec<String>,
    spots: Spots,
    /// Where they were put (desk.json).
    saved: Spots,
    /// The wadspaces and folders on the desktop last time (none until the
    /// library's here), to tell new ones.
    known: Option<HashSet<String>>,
    sel: Vec<String>,
    open_tag: Option<String>,
    /// Named as soon as its icon's there (a new wadspace, a new folder).
    pending_rename: Option<String>,
    /// The icon whose menu is open.
    menu_for: Option<String>,
    drag: Option<Drag>,
    hot: Option<Target>,
    /// What something held from elsewhere would light, where it was last offered.
    held_hot: Option<Target>,
    band: Option<Band>,
    /// The icon pressed (for a click without Ctrl or Shift).
    pressed: Option<String>,
    files: Vec<files::Entry>,
    trash_files: usize,
    /// Files cut (Ctrl+X): moved, not copied, when they're pasted.
    cut: Option<Vec<PathBuf>>,
}

/// The open folder's panel, and what it shows.
struct Panel {
    tag: String,
    root: gtk::Box,
    shows: Vec<(wad_model::Wadspace, wad_gtk::TileState)>,
    name: String,
}

pub struct Desktop {
    shell: Rc<Shell>,
    me: Weak<Desktop>,
    board: gtk::Fixed,
    band: gtk::Box,
    cells: RefCell<IndexMap<String, Cell>>,
    s: RefCell<State>,
    panel: RefCell<Option<Panel>>,
    rename: RefCell<Option<(String, gtk::Entry)>>,
    menu: RefCell<Option<menu::Open>>,
    /// Where the last menu opened (Open with's opens there too).
    menu_at: Flag<(f64, f64)>,
    /// A wadspace's picture, made as its drag's prepared, for its drag's icon (with the hint).
    held_ghost: RefCell<Option<(gdk::Paintable, i32, i32)>>,
    desk_dir: PathBuf,
    desk_json: PathBuf,
    seen_library: Flag<bool>,
    refill_queued: Flag<bool>,
    reload_queued: Flag<bool>,
    _monitors: RefCell<Vec<gio::FileMonitor>>,
}

impl Desktop {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        if let Some(d) = gdk::Display::default() {
            wad_gtk::theme::add_css(&d, CSS);
        }
        let board = gtk::Fixed::builder().css_classes(["desk"]).overflow(gtk::Overflow::Hidden).build();
        let band = gtk::Box::builder().css_classes(["desk-band"]).can_target(false).visible(false).build();
        board.put(&band, 0.0, 0.0);
        let desk_json = arrange::path();
        let me = Rc::new_cyclic(|me| Self {
            shell: shell.clone(),
            me: me.clone(),
            board,
            band,
            cells: RefCell::default(),
            s: RefCell::new(State { saved: arrange::load(&desk_json), ..Default::default() }),
            panel: RefCell::default(),
            rename: RefCell::default(),
            menu: RefCell::default(),
            menu_at: Flag::new((0.0, 0.0)),
            held_ghost: RefCell::default(),
            desk_dir: files::desktop_dir(),
            desk_json,
            seen_library: Flag::new(false),
            refill_queued: Flag::new(false),
            reload_queued: Flag::new(false),
            _monitors: RefCell::default(),
        });

        let w = Rc::downgrade(&me);
        shell.store.watch(move |c| {
            let Some(m) = w.upgrade() else { return };
            match c {
                Change::Notice { .. } | Change::Screen => return,
                Change::Connected(false) => {
                    m.seen_library.set(false);
                    m.s.borrow_mut().known = None;
                }
                Change::Library => m.seen_library.set(true),
                _ => {}
            }
            m.queue_refill();
        });
        let w = Rc::downgrade(&me);
        shell.on_ctl("desk", move |args| w.upgrade().map(|m| m.ctl(args)).unwrap_or_else(|| "error gone".into()));

        me.watch_files();
        me.pointer();
        me.dnd();
        me.reload_files();
        me
    }

    pub fn widget(&self) -> gtk::Widget {
        self.board.clone().upcast()
    }

    pub fn shown(&self) {
        self.reload_files();
    }

    fn weak(&self) -> Weak<Self> {
        self.me.clone()
    }

    fn rows(&self) -> u32 {
        let (_, h) = self.shell.size();
        grid::rows_for((h - BAR) as f64)
    }

    fn desk_size(&self) -> (f64, f64) {
        let (w, h) = self.shell.size();
        (w as f64, (h - BAR) as f64)
    }

    /// The library's here (and the machine): wadspaces new from now on are new.
    fn ready(&self) -> bool {
        self.seen_library.get() && self.shell.store.machine().is_some()
    }

    // ------------------------------------------------------------ what's on it

    /// Every icon, in the desktop's order: the wadspaces with no tag (work
    /// before gaming), a folder per tag, the files, Machines, the Trash.
    fn items(&self) -> Vec<(String, Item)> {
        let store = &self.shell.store;
        let mut spaces = store.live();
        spaces.sort_by_key(|x| x.category_or_default() == Category::Gaming);
        let (tags, trashed) = {
            let lib = store.library();
            let me = store.machine_id();
            (lib.tags.clone(), lib.wadspaces.iter().any(|x| x.is_trashed() && x.machine == me))
        };
        let mut out = Vec::new();
        for x in spaces.iter().filter(|x| tag_of(&tags, x) == GENERAL_ID) {
            let state = store.tile_state(&x.id);
            out.push((format!("ws:{}", x.id), Item::Space { ws: Box::new(x.clone()), state }));
        }
        for t in &tags {
            let mine = spaces.iter().filter(|x| tag_of(&tags, x) == t.id).cloned().collect();
            out.push((format!("folder:{}", t.id), Item::Folder { tag: t.clone(), spaces: mine }));
        }
        let s = self.s.borrow();
        for f in &s.files {
            out.push((format!("file:{}", f.name), Item::File(f.clone())));
        }
        out.push(("machines".into(), Item::Machines));
        out.push(("trash".into(), Item::Trash { full: s.trash_files > 0 || trashed }));
        out
    }

    fn queue_refill(&self) {
        if self.refill_queued.replace(true) {
            return;
        }
        let w = self.weak();
        glib::idle_add_local_once(move || {
            if let Some(m) = w.upgrade() {
                m.refill_queued.set(false);
                m.refill();
            }
        });
    }

    /// Everything shown as it is now.
    fn refill(&self) {
        let items = self.items();
        let keys: Vec<String> = items.iter().map(|(k, _)| k.clone()).collect();
        let rows = self.rows();
        let ready = self.ready();
        let (spots, save) = {
            let mut s = self.s.borrow_mut();
            let fresh: Vec<String> = match (&s.known, ready) {
                (Some(before), true) => keys
                    .iter()
                    .filter(|k| {
                        (k.starts_with("ws:") || k.starts_with("folder:"))
                            && !before.contains(*k)
                            && !s.saved.contains_key(*k)
                    })
                    .cloned()
                    .collect(),
                _ => Vec::new(),
            };
            if ready {
                s.known =
                    Some(keys.iter().filter(|k| k.starts_with("ws:") || k.starts_with("folder:")).cloned().collect());
            }
            if fresh.is_empty() {
                (grid::place(&keys, &s.saved, rows), false)
            } else {
                let p = grid::place_fresh(&keys, &s.saved, rows, &fresh);
                s.saved = p.clone();
                (p, true)
            }
        };
        if save {
            self.save();
        }
        {
            let mut s = self.s.borrow_mut();
            s.sel = select::prune(&s.sel, &keys);
            if let Some(t) = s.open_tag.clone()
                && !keys.contains(&format!("folder:{t}"))
            {
                s.open_tag = None;
            }
            s.spots = spots.clone();
            s.order = keys;
        }
        {
            let mut cells = self.cells.borrow_mut();
            let gone: Vec<String> = cells.keys().filter(|k| !spots.contains_key(*k)).cloned().collect();
            for k in gone {
                if let Some(c) = cells.shift_remove(&k) {
                    self.board.remove(&c.root);
                }
            }
            for (k, item) in &items {
                let (x, y) = spots[k].origin();
                match cells.get(k) {
                    Some(c) => {
                        c.set(item);
                        self.board.move_(&c.root, x, y);
                    }
                    None => {
                        let c = Cell::new(k, item);
                        self.board.put(&c.root, x, y);
                        cells.insert(k.clone(), c);
                    }
                }
            }
        }
        self.lay_panel();
        self.restyle();
        self.raise_overlays();
        let pending = self.s.borrow().pending_rename.clone();
        if let Some(k) = pending
            && self.cells.borrow().contains_key(&k)
        {
            {
                let mut s = self.s.borrow_mut();
                s.pending_rename = None;
                s.sel = vec![k.clone()];
            }
            self.restyle();
            self.start_rename(&k);
        }
    }

    /// The selection, what's carried, what's under a drag, whose menu is open.
    fn restyle(&self) {
        let s = self.s.borrow();
        let lifted: &[String] = s.drag.as_ref().filter(|d| !d.from_folder).map(|d| d.keys.as_slice()).unwrap_or(&[]);
        let hot = match &s.hot {
            Some(Target::Tag { tag, panel: false }) => Some(format!("folder:{tag}")),
            Some(Target::Trash) => Some("trash".into()),
            Some(Target::Into { key, .. }) => Some(key.clone()),
            Some(Target::Keep(k) | Target::Put(k)) => Some(k.clone()),
            _ => None,
        };
        let open = s.open_tag.as_ref().map(|t| format!("folder:{t}"));
        for (k, c) in self.cells.borrow().iter() {
            c.set_flag("on", s.sel.contains(k));
            c.set_flag("lifted", lifted.contains(k));
            c.set_flag("hot", hot.as_ref() == Some(k));
            c.set_flag("open", open.as_ref() == Some(k));
            c.set_flag("menu-for", s.menu_for.as_ref() == Some(k));
        }
        if let Some(p) = self.panel.borrow().as_ref() {
            if matches!(s.hot, Some(Target::Tag { panel: true, .. })) {
                p.root.add_css_class("hot");
            } else {
                p.root.remove_css_class("hot");
            }
            // A wadspace in it with something held over it.
            each_inside(p.root.upcast_ref(), &mut |w| {
                if hot.as_deref() == Some(w.widget_name().as_str()) {
                    w.add_css_class("hot");
                } else {
                    w.remove_css_class("hot");
                }
            });
        }
    }

    /// The panel, the name being typed and the box over the icons.
    fn raise_overlays(&self) {
        if let Some(p) = self.panel.borrow().as_ref() {
            p.root.insert_before(&self.board, None::<&gtk::Widget>);
        }
        if let Some((_, e)) = self.rename.borrow().as_ref() {
            e.insert_before(&self.board, None::<&gtk::Widget>);
        }
        self.band.insert_before(&self.board, None::<&gtk::Widget>);
    }

    fn save(&self) {
        let saved = self.s.borrow().saved.clone();
        if let Err(e) = arrange::save(&self.desk_json, &saved) {
            tracing::warn!("{}: {e}", self.desk_json.display());
        }
    }

    /// Icons put where `next` says (and the rest where they fall), kept.
    fn arrange(&self, next: &Spots) {
        let (mut order, rows) = (self.s.borrow().order.clone(), self.rows());
        // The ones not on the desktop yet (out of a folder) too: their spots are taken.
        for k in next.keys() {
            if !order.contains(k) {
                order.push(k.clone());
            }
        }
        self.s.borrow_mut().saved = grid::place(&order, next, rows);
        self.save();
        self.refill();
    }

    // ------------------------------------------------------------ files

    fn watch_files(&self) {
        let mut mons = Vec::new();
        let trash = files::trash_dir();
        for sub in ["files", "info"] {
            let _ = std::fs::create_dir_all(trash.join(sub));
        }
        for dir in [self.desk_dir.clone(), trash.join("files")] {
            match gio::File::for_path(&dir)
                .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
            {
                Ok(m) => {
                    let w = self.weak();
                    m.connect_changed(move |_, _, _, _| {
                        if let Some(d) = w.upgrade() {
                            d.reload_files_soon();
                        }
                    });
                    mons.push(m);
                }
                Err(e) => tracing::warn!("watching {}: {e}", dir.display()),
            }
        }
        *self._monitors.borrow_mut() = mons;
    }

    fn reload_files_soon(&self) {
        if self.reload_queued.replace(true) {
            return;
        }
        let w = self.weak();
        glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
            if let Some(m) = w.upgrade() {
                m.reload_queued.set(false);
                m.reload_files();
            }
        });
    }

    fn reload_files(&self) {
        let list = files::list(&self.desk_dir);
        let count = files::trash_count();
        {
            let mut s = self.s.borrow_mut();
            s.files = list;
            s.trash_files = count;
        }
        self.refill();
    }

    fn entry(&self, key: &str) -> Option<files::Entry> {
        let name = key.strip_prefix("file:")?;
        self.s.borrow().files.iter().find(|f| f.name == name).cloned()
    }

    fn file_paths(&self, keys: &[String]) -> Vec<PathBuf> {
        keys.iter().filter_map(|k| self.entry(k)).map(|e| e.path).collect()
    }

    /// Runs `work` off the main thread, then `done` with what it said.
    fn off_thread<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(&Rc<Self>, T) + 'static,
    ) {
        let w = self.weak();
        glib::spawn_future_local(async move {
            let Ok(out) = gio::spawn_blocking(work).await else { return };
            if let Some(m) = w.upgrade() {
                done(&m, out);
                m.reload_files();
            }
        });
    }

    fn toast(&self, level: Level, title: &str) {
        let (shell, title) = (self.shell.clone(), title.to_string());
        // Never from inside a store watcher: next turn.
        glib::idle_add_local_once(move || shell.toast(level, &title, None));
    }

    // ------------------------------------------------------------ the pointer

    /// What's under (x, y), in the desktop's pixels.
    fn hit(&self, x: f64, y: f64) -> Hit {
        let Some(mut w) = self.board.pick(x, y, gtk::PickFlags::DEFAULT) else { return Hit::Background };
        loop {
            let name = w.widget_name();
            if let Some(k) = name.strip_prefix("desk:") {
                return Hit::Icon(k.to_string());
            }
            if let Some(id) = name.strip_prefix("in:") {
                return Hit::Inside(id.to_string());
            }
            if name == "desk-panel" {
                return Hit::Panel;
            }
            if w == self.board.clone().upcast::<gtk::Widget>() {
                return Hit::Background;
            }
            match w.parent() {
                Some(p) => w = p,
                None => return Hit::Background,
            }
        }
    }

    fn pointer(&self) {
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_PRIMARY).build();
        let w = self.weak();
        click.connect_pressed(move |g, n, x, y| {
            if let Some(m) = w.upgrade() {
                m.pressed(n, x, y, mods(g.current_event_state()));
            }
        });
        let w = self.weak();
        click.connect_released(move |g, n, x, y| {
            if let Some(m) = w.upgrade() {
                m.released(n, x, y, mods(g.current_event_state()));
            }
        });
        self.board.add_controller(click);

        let right = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let w = self.weak();
        right.connect_pressed(move |g, _, x, y| {
            if let Some(m) = w.upgrade() {
                g.set_state(gtk::EventSequenceState::Claimed);
                m.context_menu(x, y, true);
            }
        });
        self.board.add_controller(right);

        // A box across the background.
        let band = gtk::GestureDrag::builder().button(gdk::BUTTON_PRIMARY).build();
        let w = self.weak();
        band.connect_drag_begin(move |g, x, y| {
            let Some(m) = w.upgrade() else { return };
            if m.hit(x, y) != Hit::Background || m.rename.borrow().is_some() {
                return;
            }
            let mut s = m.s.borrow_mut();
            let base = select::band_base(&s.sel, mods(g.current_event_state()));
            s.sel = base.clone();
            s.band = Some(Band { base, x0: x, y0: y });
        });
        let w = self.weak();
        band.connect_drag_update(move |g, dx, dy| {
            let Some(m) = w.upgrade() else { return };
            let Some((x0, y0)) = g.start_point() else { return };
            m.band_to(x0, y0, x0 + dx, y0 + dy);
        });
        let w = self.weak();
        band.connect_drag_end(move |_, _, _| {
            if let Some(m) = w.upgrade() {
                m.s.borrow_mut().band = None;
                m.band.set_visible(false);
            }
        });
        self.board.add_controller(band);
    }

    fn band_to(&self, x0: f64, y0: f64, x1: f64, y1: f64) {
        {
            let mut s = self.s.borrow_mut();
            let Some(b) = s.band.as_ref() else { return };
            let (bx, by) = (b.x0, b.y0);
            debug_assert!((bx - x0).abs() < 0.5 && (by - y0).abs() < 0.5);
            let touched = grid::in_box(&s.spots, (bx, by, x1, y1));
            s.sel = select::band(&b.base, &touched);
        }
        let (l, t) = (x0.min(x1), y0.min(y1));
        self.band.set_size_request((x1 - x0).abs().round() as i32, (y1 - y0).abs().round() as i32);
        self.board.move_(&self.band, l.round(), t.round());
        self.band.set_visible(true);
        self.restyle();
    }

    fn pressed(&self, n: i32, x: f64, y: f64, m: Mods) {
        if self.on_rename(x, y) {
            return;
        }
        if self.rename.borrow().is_some() {
            self.end_rename(true);
        }
        let hit = self.hit(x, y);
        // A click elsewhere closes the open folder.
        let keep_open = match &hit {
            Hit::Panel | Hit::Inside(_) => true,
            Hit::Icon(k) => k.starts_with("folder:"),
            Hit::Background => false,
        };
        if !keep_open && self.s.borrow().open_tag.is_some() {
            self.s.borrow_mut().open_tag = None;
            self.lay_panel();
        }
        match hit {
            Hit::Icon(k) if n >= 2 && !m.any() => {
                self.s.borrow_mut().sel = vec![k.clone()];
                self.restyle();
                self.open_key(&k);
            }
            Hit::Icon(k) => {
                let mut s = self.s.borrow_mut();
                s.sel = select::press(&s.sel, &k, m);
                s.pressed = Some(k);
            }
            Hit::Inside(id) if n >= 2 => {
                self.s.borrow_mut().open_tag = None;
                self.lay_panel();
                self.shell.edit(&id);
            }
            Hit::Inside(_) | Hit::Panel => {}
            Hit::Background => {
                let mut s = self.s.borrow_mut();
                s.sel = select::band_base(&s.sel, m);
            }
        }
        self.restyle();
    }

    /// (x, y) is on the name being typed.
    fn on_rename(&self, x: f64, y: f64) -> bool {
        let Some(e) = self.rename.borrow().as_ref().map(|(_, e)| e.clone().upcast::<gtk::Widget>()) else {
            return false;
        };
        self.board.pick(x, y, gtk::PickFlags::DEFAULT).is_some_and(|w| w == e || w.is_ancestor(&e))
    }

    fn released(&self, n: i32, x: f64, y: f64, m: Mods) {
        let pressed = self.s.borrow_mut().pressed.take();
        if n != 1 {
            return;
        }
        if let (Hit::Icon(k), Some(p)) = (self.hit(x, y), pressed)
            && k == p
        {
            let mut s = self.s.borrow_mut();
            s.sel = select::click(&s.sel, &k, m, false);
        }
        self.restyle();
    }

    // ------------------------------------------------------------ drag and drop

    fn dnd(&self) {
        let src = gtk::DragSource::builder().actions(gdk::DragAction::COPY | gdk::DragAction::MOVE).build();
        let w = self.weak();
        src.connect_prepare(move |src, x, y| {
            let m = w.upgrade()?;
            if m.rename.borrow().is_some() {
                return None;
            }
            let (keys, from_folder) = match m.hit(x, y) {
                Hit::Icon(k) => (select::carried(&m.s.borrow().sel, &k), false),
                Hit::Inside(id) => (vec![format!("ws:{id}")], true),
                _ => return None,
            };
            let held = held_of(&keys, from_folder);
            let content = m.content(&keys, held.as_ref());
            let ghost = m.ghost(&keys, from_folder, x, y);
            *m.held_ghost.borrow_mut() = None;
            match ghost {
                // Its drag's icon is made as it begins, with the hint under it.
                Some(g) if held.is_some() => *m.held_ghost.borrow_mut() = Some(g),
                Some((icon, hx, hy)) => src.set_icon(Some(&icon), hx, hy),
                None => {}
            }
            m.s.borrow_mut().drag = Some(Drag { keys, from_folder, held });
            Some(content)
        });
        let w = self.weak();
        src.connect_drag_begin(move |_, drag| {
            if let Some(m) = w.upgrade() {
                m.s.borrow_mut().pressed = None;
                let held = m.s.borrow().drag.as_ref().and_then(|d| d.held.clone());
                if let Some(h) = held {
                    if let Some((picture, hx, hy)) = m.held_ghost.borrow_mut().take() {
                        gtk::DragIcon::for_drag(drag).set_child(Some(&held_ghost(&picture)));
                        drag.set_hotspot(hx, hy);
                    }
                    dnd::begin(h);
                }
                m.restyle();
            }
        });
        let w = self.weak();
        src.connect_drag_end(move |_, _, _| {
            if let Some(m) = w.upgrade() {
                let mut s = m.s.borrow_mut();
                let held = s.drag.take().is_some_and(|d| d.held.is_some());
                s.hot = None;
                drop(s);
                if held {
                    dnd::end();
                }
                m.restyle();
            }
        });
        self.board.add_controller(src);

        let formats = gdk::ContentFormatsBuilder::new()
            .add_mime_type(DESK_MIME)
            .add_type(gdk::FileList::static_type())
            .build()
            .union_deserialize_mime_types();
        let target = gtk::DropTargetAsync::new(Some(formats), gdk::DragAction::COPY | gdk::DragAction::MOVE);
        let w = self.weak();
        target.connect_drag_enter(move |_, drop, x, y| {
            w.upgrade().map(|m| m.hover(drop, x, y)).unwrap_or(gdk::DragAction::empty())
        });
        let w = self.weak();
        target.connect_drag_motion(move |_, drop, x, y| {
            w.upgrade().map(|m| m.hover(drop, x, y)).unwrap_or(gdk::DragAction::empty())
        });
        let w = self.weak();
        target.connect_drag_leave(move |_, _| {
            if let Some(m) = w.upgrade() {
                m.s.borrow_mut().hot = None;
                m.restyle();
            }
        });
        let w = self.weak();
        target.connect_drop(move |_, drop, x, y| w.upgrade().is_some_and(|m| m.dropped(drop, x, y)));
        self.board.add_controller(target);

        self.held_target();
    }

    /// What's held from elsewhere (a piece of a Builder stack, a whole
    /// stack, a key's group), let go here: onto a wadspace, into a tag, or
    /// open desktop. The desktop's own drags are its own target's (above).
    fn held_target(&self) {
        let w = self.weak();
        let offer = move |held: &Held, x: f64, y: f64| -> Option<Verdict> {
            let m = w.upgrade()?;
            let at = m.held_at(held, x, y);
            m.s.borrow_mut().held_hot = at.as_ref().filter(|(_, v)| v.ok).and_then(|(l, _)| l.lit());
            at.map(|(_, v)| v)
        };
        let w = self.weak();
        let land = move |held: &Held, x: f64, y: f64| {
            if let Some(m) = w.upgrade() {
                m.land_held(held, x, y);
            }
        };
        let w = self.weak();
        let hover = move |on: bool| {
            let Some(m) = w.upgrade() else { return };
            let hot = if on { m.s.borrow().held_hot.clone() } else { None };
            if m.s.borrow().hot != hot {
                m.s.borrow_mut().hot = hot;
                m.restyle();
            }
        };
        let t = dnd::target(&self.board, offer, land, hover);
        // Not while one of the desktop's own is carried (it's held too, for the other surfaces).
        let w = self.weak();
        t.connect_accept(move |_, drop| {
            let ours = w.upgrade().is_some_and(|m| m.s.borrow().drag.is_some());
            !ours && drop.formats().contain_mime_type(dnd::MIME)
        });
    }

    /// Where `held` let go at (x, y) goes, and what it'd do (as the spike's
    /// targets: the deepest that has something to say about it).
    fn held_at(&self, held: &Held, x: f64, y: f64) -> Option<(Lands, Verdict)> {
        if matches!(held, Held::Space { from: Some(SpaceFrom::Desktop | SpaceFrom::Folder), .. }) {
            return None;
        }
        let hit = self.hit(x, y);
        // A wadspace (its icon, or it in the open folder) takes another's piece.
        let on = match &hit {
            Hit::Icon(k) => k.strip_prefix("ws:").map(|id| (id.to_string(), k.clone())),
            Hit::Inside(id) => Some((id.clone(), format!("in:{id}"))),
            _ => None,
        };
        if let Some((ws, key)) = on
            && let Some(v) = drops::offer_on(&self.shell, &ws, held)
        {
            return Some((Lands::On { ws, key }, v));
        }
        // A tag's folder (its icon, or it open) takes a whole stack.
        let tag = match &hit {
            Hit::Icon(k) => k.strip_prefix("folder:").map(|t| (t.to_string(), false)),
            Hit::Inside(_) | Hit::Panel => self.s.borrow().open_tag.clone().map(|t| (t, true)),
            Hit::Background => None,
        };
        if let (Some((tag, panel)), Held::Space { ws, .. }) = (tag, held)
            && let Some(v) = self.offer_tag(ws, &tag)
        {
            return Some((Lands::Tag { tag, panel }, v));
        }
        // Anywhere else, it's open desktop.
        drops::offer_fresh(&self.shell, held).map(|v| (Lands::Fresh, v))
    }

    /// A whole stack let go on `tag`'s folder: what tagging it would do.
    fn offer_tag(&self, ws: &str, tag: &str) -> Option<Verdict> {
        let store = &self.shell.store;
        let x = store.wadspace(ws).filter(|x| !x.is_trashed())?;
        let tags = store.library().tags.clone();
        let name = tags.iter().find(|t| t.id == tag)?.name.clone();
        if x.machine != store.machine_id() {
            return Verdict::no("It lives on another machine");
        }
        if tag_of(&tags, &x) == tag {
            Verdict::yes(format!("In {name}"))
        } else {
            Verdict::yes(format!("{} into {name}", x.name))
        }
    }

    fn land_held(&self, held: &Held, x: f64, y: f64) {
        let Some((to, v)) = self.held_at(held, x, y) else { return };
        if !v.ok {
            return;
        }
        match to {
            Lands::On { ws, .. } => drops::land_on(&self.shell, &ws, held),
            Lands::Tag { tag, .. } => {
                if let Held::Space { ws, .. } = held
                    && self.shell.store.wadspace(ws).is_some_and(|x| x.tag.as_deref() != Some(tag.as_str()))
                {
                    self.shell.op(Op::SetTag { ws: ws.clone(), tag: Some(tag) }, |_| {});
                }
            }
            Lands::Fresh => drops::land_fresh(&self.shell, held),
        }
    }

    /// What a drag carries: its files (for other apps too), its keys, and
    /// (one wadspace) it as it's held, for the other surfaces.
    fn content(&self, keys: &[String], held: Option<&Held>) -> gdk::ContentProvider {
        let bytes = glib::Bytes::from_owned(keys.join("\n").into_bytes());
        let mut providers = vec![gdk::ContentProvider::for_bytes(DESK_MIME, &bytes)];
        let paths = self.file_paths(keys);
        if !paths.is_empty() {
            let list: Vec<gio::File> = paths.iter().map(gio::File::for_path).collect();
            providers.push(gdk::ContentProvider::for_value(&gdk::FileList::from_array(&list).to_value()));
        }
        if let Some(h) = held {
            providers.push(dnd::content(h));
        }
        gdk::ContentProvider::new_union(&providers)
    }

    /// The carried icons as they're laid out, for under the pointer.
    fn ghost(&self, keys: &[String], from_folder: bool, x: f64, y: f64) -> Option<(gdk::Paintable, i32, i32)> {
        let widgets: Vec<gtk::Widget> = if from_folder {
            let id = keys.first()?.strip_prefix("ws:")?;
            let panel = self.panel.borrow();
            let mut found = None;
            let mut c = panel.as_ref()?.root.first_child();
            while let Some(w) = c {
                if let Some(hit) = find_named(&w, &format!("in:{id}")) {
                    found = Some(hit);
                    break;
                }
                c = w.next_sibling();
            }
            vec![found?]
        } else {
            let cells = self.cells.borrow();
            keys.iter().take(GHOSTS).filter_map(|k| cells.get(k).map(|c| c.root.clone().upcast())).collect()
        };
        let bounds: Vec<graphene::Rect> = widgets.iter().filter_map(|w| w.compute_bounds(&self.board)).collect();
        let first = *bounds.first()?;
        let all = bounds.iter().fold(first, |a, b| a.union(b));
        let snap = gtk::Snapshot::new();
        for (w, b) in widgets.iter().zip(&bounds) {
            snap.save();
            snap.translate(&graphene::Point::new(b.x() - all.x(), b.y() - all.y()));
            gtk::WidgetPaintable::new(Some(w)).snapshot(&snap, b.width() as f64, b.height() as f64);
            snap.restore();
        }
        let p = snap.to_paintable(Some(&graphene::Size::new(all.width(), all.height())))?;
        Some((p, (x - all.x() as f64).round() as i32, (y - all.y() as f64).round() as i32))
    }

    /// Ours (from this desktop), and what it carries.
    fn ours(&self, drop: &gdk::Drop) -> Option<Drag> {
        drop.drag()?;
        self.s.borrow().drag.clone()
    }

    /// Where a drop at (x, y) would go.
    fn target(&self, drag: Option<&Drag>, x: f64, y: f64) -> Target {
        let spot = Target::Spot(grid::spot_at(x, y, self.rows()));
        let carried = drag.map(|d| d.keys.as_slice()).unwrap_or(&[]);
        let spaces = carried.iter().any(|k| k.starts_with("ws:"));
        let files_held = drag.is_none() || carried.iter().any(|k| k.starts_with("file:"));
        let open = self.s.borrow().open_tag.clone();
        match self.hit(x, y) {
            Hit::Panel | Hit::Inside(_) => match open {
                Some(tag) if spaces && !drag.is_some_and(|d| d.from_folder) => Target::Tag { tag, panel: true },
                _ => spot,
            },
            Hit::Icon(k) if carried.contains(&k) => spot,
            Hit::Icon(k) => {
                if let Some(tag) = k.strip_prefix("folder:") {
                    if spaces { Target::Tag { tag: tag.to_string(), panel: false } } else { spot }
                } else if k == "trash" {
                    if drag.is_some() && carried.iter().any(|c| c.starts_with("ws:") || c.starts_with("file:")) {
                        Target::Trash
                    } else {
                        spot
                    }
                } else if k.starts_with("file:") {
                    match self.entry(&k) {
                        Some(e) if e.kind == files::Kind::Folder && files_held && !spaces => {
                            Target::Into { key: k, dir: e.path }
                        }
                        _ => spot,
                    }
                } else if k.starts_with("ws:") && files_held && !spaces {
                    Target::Keep(k)
                } else {
                    spot
                }
            }
            Hit::Background => spot,
        }
    }

    fn hover(&self, drop: &gdk::Drop, x: f64, y: f64) -> gdk::DragAction {
        let ours = self.ours(drop);
        let t = self.target(ours.as_ref(), x, y);
        let hot = (!matches!(t, Target::Spot(_))).then_some(t);
        let changed = self.s.borrow().hot != hot;
        if changed {
            self.s.borrow_mut().hot = hot;
            self.restyle();
        }
        if ours.is_some() { gdk::DragAction::MOVE } else { gdk::DragAction::COPY }
    }

    fn dropped(&self, drop: &gdk::Drop, x: f64, y: f64) -> bool {
        let ours = self.ours(drop);
        let target = self.target(ours.as_ref(), x, y);
        self.s.borrow_mut().hot = None;
        self.restyle();
        match ours {
            Some(d) => {
                self.dropped_ours(&d, target);
                drop.finish(gdk::DragAction::MOVE);
            }
            None => self.dropped_files(drop, target),
        }
        true
    }

    fn dropped_ours(&self, d: &Drag, target: Target) {
        let spaces: Vec<String> = d.keys.iter().filter_map(|k| k.strip_prefix("ws:")).map(String::from).collect();
        match target {
            Target::Tag { tag, .. } => {
                let store = &self.shell.store;
                for id in &spaces {
                    if store.wadspace(id).is_some_and(|x| x.tag.as_deref() != Some(tag.as_str())) {
                        self.shell.op(Op::SetTag { ws: id.clone(), tag: Some(tag.clone()) }, |_| {});
                    }
                }
                let mut s = self.s.borrow_mut();
                s.sel.retain(|k| !k.starts_with("ws:"));
            }
            Target::Trash => self.trash(&d.keys),
            Target::Into { dir, .. } => {
                let paths = self.file_paths(&d.keys);
                self.off_thread(
                    move || files::move_into(&paths, &dir),
                    |m, r| {
                        if let Err(e) = r {
                            m.toast(Level::Warn, &e);
                        }
                    },
                );
            }
            // Kept in it: a folder mounted, a file copied in (a picture asks first).
            Target::Keep(k) => self.keep_in(&k, self.file_paths(&d.keys)),
            // Only what's held from elsewhere (held_target's).
            Target::Put(_) => {}
            Target::Spot(to) if d.from_folder => {
                let Some(id) = spaces.first() else { return };
                let key = format!("ws:{id}");
                let next = grid::place_at(&self.s.borrow().spots, &key, to);
                // Where it lands, before it's back on the desktop (it isn't new there).
                self.arrange(&next);
                self.shell.op(Op::SetTag { ws: id.clone(), tag: None }, |_| {});
            }
            Target::Spot(to) => {
                let Some(anchor) = d.keys.first() else { return };
                let next = grid::move_to(&self.s.borrow().spots, anchor, to, &d.keys, self.rows());
                if let Some(next) = next {
                    self.arrange(&next);
                }
            }
        }
    }

    /// Files from another app: copied onto the desktop (or into a folder on
    /// it), the first where it was let go.
    fn dropped_files(&self, drop: &gdk::Drop, target: Target) {
        let (dir, spot) = match &target {
            Target::Into { dir, .. } => (dir.clone(), None),
            Target::Spot(s) => (self.desk_dir.clone(), Some(*s)),
            _ => (self.desk_dir.clone(), None),
        };
        // On a wadspace: kept in it, where they are (not copied onto the desktop).
        let keep = if let Target::Keep(k) = &target { Some(k.clone()) } else { None };
        let (w, drop) = (self.weak(), drop.clone());
        glib::spawn_future_local(async move {
            let got = drop.read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT).await;
            let paths = got.ok().and_then(|v| v.get::<gdk::FileList>().ok()).map(|l| files::local(&l.files()));
            let Some(paths) = paths.filter(|p| !p.is_empty()) else {
                drop.finish(gdk::DragAction::empty());
                return;
            };
            drop.finish(gdk::DragAction::COPY);
            let Some(m) = w.upgrade() else { return };
            if let Some(k) = keep {
                m.keep_in(&k, paths);
                return;
            }
            m.off_thread(
                move || files::copy_into(&paths, &dir),
                move |m, r| match r {
                    Ok(names) => {
                        if let (Some(to), Some(first)) = (spot, names.first()) {
                            let next = grid::place_at(&m.s.borrow().spots, &format!("file:{first}"), to);
                            m.s.borrow_mut().saved = next;
                            m.save();
                        }
                    }
                    Err(e) => m.toast(Level::Warn, &e),
                },
            );
        });
    }

    // ------------------------------------------------------------ the open folder

    fn open_folder(&self, tag: &str) {
        self.s.borrow_mut().open_tag = Some(tag.to_string());
        self.lay_panel();
        self.restyle();
        self.raise_overlays();
    }

    fn close_folder(&self) {
        self.s.borrow_mut().open_tag = None;
        self.lay_panel();
        self.restyle();
    }

    /// The open folder's panel beside its icon (made again when what's in it changed).
    fn lay_panel(&self) {
        let (open, spot) = {
            let s = self.s.borrow();
            let open = s.open_tag.clone();
            let spot = open.as_ref().and_then(|t| s.spots.get(&format!("folder:{t}")).copied());
            (open, spot)
        };
        let item = open.as_ref().and_then(|t| self.cells.borrow().get(&format!("folder:{t}")).map(|c| c.item()));
        let (Some(tag_id), Some(spot), Some(Item::Folder { tag, spaces })) = (open, spot, item) else {
            if let Some(p) = self.panel.borrow_mut().take() {
                self.board.remove(&p.root);
            }
            return;
        };
        let shows: Vec<_> = spaces.iter().map(|x| (x.clone(), self.shell.store.tile_state(&x.id))).collect();
        let same =
            self.panel.borrow().as_ref().is_some_and(|p| p.tag == tag_id && p.shows == shows && p.name == tag.name);
        if !same {
            if let Some(p) = self.panel.borrow_mut().take() {
                self.board.remove(&p.root);
            }
            let root = self.make_panel(&tag, &shows);
            self.board.put(&root, 0.0, 0.0);
            *self.panel.borrow_mut() = Some(Panel { tag: tag_id, root, shows: shows.clone(), name: tag.name.clone() });
        }
        let panel = self.panel.borrow();
        let Some(p) = panel.as_ref() else { return };
        // As wide as the spike's (four to a row at most), or as it needs.
        let cols = shows.len().clamp(2, 4) as f64;
        p.root.set_size_request(-1, -1);
        let (min_w, _, _, _) = p.root.measure(gtk::Orientation::Horizontal, -1);
        let width = (cols * 104.0 + 28.0).max(min_w as f64);
        p.root.set_size_request(width as i32, -1);
        let (_, h, _, _) = p.root.measure(gtk::Orientation::Vertical, width as i32);
        // Beside its icon (to its left, if there's no room on its right).
        let (desk_w, desk_h) = self.desk_size();
        let (fx, fy) = spot.origin();
        let mut left = grid::PAD_X + (spot.c as f64 + 1.0) * grid::CW + 6.0;
        if left + width > desk_w - 8.0 && fx - width - 6.0 >= 8.0 {
            left = fx - width - 6.0;
        }
        let top = (fy - 10.0).min(desk_h - h as f64 - 12.0).max(8.0);
        self.board.move_(&p.root, left.round(), top.round());
    }

    fn make_panel(&self, tag: &wad_model::Tag, shows: &[(wad_model::Wadspace, wad_gtk::TileState)]) -> gtk::Box {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .css_classes(["desk-panel", &icons::tag_class(&tag.color)])
            .build();
        root.set_widget_name("desk-panel");
        let head = gtk::Box::builder().spacing(8).margin_start(4).margin_end(2).margin_top(2).build();
        let dot = gtk::Box::builder().css_classes(["desk-panel-dot"]).valign(gtk::Align::Center).build();
        head.append(&dot);
        head.append(&gtk::Label::builder().label(&tag.name).css_classes(["desk-panel-name"]).build());
        head.append(
            &gtk::Label::builder()
                .label(shows.len().to_string())
                .css_classes(["desk-panel-count"])
                .valign(gtk::Align::Center)
                .build(),
        );
        let close = gtk::Button::builder()
            .css_classes(["desk-panel-close"])
            .halign(gtk::Align::End)
            .hexpand(true)
            .child(&wad_gtk::Icon::new("x", 14))
            .build();
        let w = self.weak();
        close.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.close_folder();
            }
        });
        head.append(&close);
        root.append(&head);
        if shows.is_empty() {
            root.append(
                &gtk::Label::builder()
                    .label(format!("Drag wadspaces here to tag them {}.", tag.name))
                    .css_classes(["desk-panel-empty"])
                    .xalign(0.0)
                    .wrap(true)
                    .build(),
            );
        } else {
            let cols = shows.len().clamp(2, 4);
            let g = gtk::Grid::builder().column_spacing(4).row_spacing(4).build();
            for (i, (x, st)) in shows.iter().enumerate() {
                let c = icons::inside(x, st);
                g.attach(&c, (i % cols) as i32, (i / cols) as i32, 1, 1);
            }
            root.append(&g);
        }
        root
    }

    // ------------------------------------------------------------ naming one

    fn can_rename(&self, key: &str) -> bool {
        key.starts_with("ws:") || key.starts_with("folder:") || key.starts_with("file:")
    }

    /// Its name, ready to type over under its icon.
    fn start_rename(&self, key: &str) {
        self.end_rename(true);
        if !self.can_rename(key) {
            return;
        }
        let Some(spot) = self.s.borrow().spots.get(key).copied() else { return };
        let Some(item) = self.cells.borrow().get(key).map(|c| c.item()) else { return };
        let (name, below) = match &item {
            Item::File(e) => (e.display.clone(), 61.0),
            other => (other.name(), 82.0),
        };
        let e = gtk::Entry::builder().text(&name).css_classes(["desk-rename"]).xalign(0.5).build();
        e.set_size_request(116, 26);
        let (x, y) = spot.origin();
        self.board.put(&e, x - 8.0, y + below);
        if let Some(c) = self.cells.borrow().get(key) {
            c.set_naming(true);
        }
        let w = self.weak();
        e.connect_activate(move |_| {
            if let Some(m) = w.upgrade() {
                m.end_rename(true);
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = self.weak();
        keys.connect_key_pressed(move |_, k, _, _| {
            if k == gdk::Key::Escape
                && let Some(m) = w.upgrade()
            {
                m.end_rename(false);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        e.add_controller(keys);
        let focus = gtk::EventControllerFocus::new();
        let (w, mine) = (self.weak(), e.downgrade());
        focus.connect_leave(move |_| {
            let (w, mine) = (w.clone(), mine.clone());
            // After the click that took the focus is done with; only this one
            // (another may have started since).
            glib::idle_add_local_once(move || {
                let (Some(m), Some(e)) = (w.upgrade(), mine.upgrade()) else { return };
                let still = m.rename.borrow().as_ref().is_some_and(|(_, x)| *x == e);
                if still && !e.has_focus() {
                    m.end_rename(true);
                }
            });
        });
        e.add_controller(focus);
        *self.rename.borrow_mut() = Some((key.to_string(), e.clone()));
        e.grab_focus();
        // A file's name without its extension, to type over.
        let stem = match (&item, name.rfind('.')) {
            (Item::File(f), Some(i)) if f.kind != files::Kind::Folder && i > 0 => name[..i].chars().count() as i32,
            _ => -1,
        };
        e.select_region(0, stem);
    }

    /// Done typing: kept (`commit`) or not.
    fn end_rename(&self, commit: bool) {
        let Some((key, e)) = self.rename.borrow_mut().take() else { return };
        let text = e.text().to_string();
        self.board.remove(&e);
        if let Some(c) = self.cells.borrow().get(&key) {
            c.set_naming(false);
        }
        if commit {
            self.rename_to(&key, &text);
        }
    }

    fn rename_to(&self, key: &str, text: &str) {
        let new = text.trim();
        if new.is_empty() {
            return;
        }
        let Some(item) = self.cells.borrow().get(key).map(|c| c.item()) else { return };
        if new == item.name() {
            return;
        }
        if let Some(id) = key.strip_prefix("ws:") {
            self.shell.op(Op::Rename { ws: id.to_string(), name: new.to_string() }, |_| {});
        } else if let Some(tag) = key.strip_prefix("folder:") {
            self.shell.op(Op::RenameTag { tag: tag.to_string(), name: new.to_string() }, |_| {});
        } else if let Item::File(f) = item {
            let Some(name) = files::valid_name(new) else {
                self.toast(Level::Warn, "A name can't have a / in it");
                return;
            };
            if f.path.with_file_name(&name).symlink_metadata().is_ok() {
                self.toast(Level::Warn, &format!("There's already a {name} on the desktop"));
                return;
            }
            match f.file().set_display_name(&name, gio::Cancellable::NONE) {
                Ok(done) => {
                    let now = done.basename().map(|b| b.to_string_lossy().into_owned()).unwrap_or(name);
                    // It keeps its spot under its new name.
                    let mut s = self.s.borrow_mut();
                    if let Some(p) = s.spots.get(key).copied() {
                        s.saved.shift_remove(key);
                        s.saved.insert(format!("file:{now}"), p);
                    }
                    drop(s);
                    self.save();
                    self.s.borrow_mut().sel = vec![format!("file:{now}")];
                    self.reload_files();
                }
                Err(e) => self.toast(Level::Warn, &format!("Couldn't rename it: {e}")),
            }
        }
    }

    // ------------------------------------------------------------ doing things

    /// Opened (double-click, Enter): a wadspace, a folder shown, a folder of
    /// files in Files, a file with the app that opened its kind last (in a
    /// space that has it), else in Files' viewer (the spike's `shell.open`).
    fn open_key(&self, key: &str) {
        if let Some(id) = key.strip_prefix("ws:") {
            self.shell.edit(id);
        } else if let Some(tag) = key.strip_prefix("folder:") {
            self.open_folder(tag);
        } else if let Some(e) = self.entry(key) {
            let path = e.path.to_string_lossy().into_owned();
            if e.kind == files::Kind::Folder {
                crate::peers::open_app(&self.shell, &["wados-files", &path]);
                return;
            }
            let f = open_file(&e);
            match crate::open_with::default_for(&self.shell, &f) {
                Some((app, ws)) => crate::open_with::open(&self.shell, &[f], app, Some(ws)),
                None => crate::peers::open_app(&self.shell, &["wados-files", "--view", &path]),
            }
        } else if key == "trash" {
            crate::peers::open_app(&self.shell, &["wados-files", "--trash"]);
        } else if key == "machines" {
            // Files, at Machines: my other machines on the tailnet, their homes and wadspaces.
            crate::peers::open_app(&self.shell, &["wados-files", "--machines"]);
        }
    }

    /// Open with ▸ for these files: its rows, as a menu of their own where the
    /// desktop's was (the desktop's menus have no submenus).
    fn open_with_menu(&self, keys: &[String]) {
        let files: Vec<crate::open_with::File> =
            keys.iter().filter_map(|k| self.entry(k)).map(|e| open_file(&e)).collect();
        if files.is_empty() {
            return;
        }
        let (x, y) = self.menu_at.get();
        let mut entries = vec![Entry::Heading("Open with".into())];
        for r in crate::open_with::rows(&self.shell, &files) {
            match r {
                wad_model::open_with::Row::Sep => entries.push(Entry::Sep),
                wad_model::open_with::Row::Nothing => {
                    entries.push(MenuItem::new("open-with:none", "No app opens it", "", || {}).off(Some("")).entry())
                }
                wad_model::open_with::Row::App { app, ws, label, hint, checked } => {
                    let id = menu::static_id(
                        &wad_model::open_with::Row::App {
                            app,
                            ws: ws.clone(),
                            label: String::new(),
                            hint: None,
                            checked: false,
                        }
                        .id(),
                    );
                    let (s, f) = (self.shell.clone(), files.clone());
                    let icon = if ws.is_some() { "monitor-play" } else { "plus" };
                    let mut it = MenuItem::new(id, label, icon, move || {
                        crate::open_with::open(&s, &f, app, ws.clone());
                    })
                    .checked(checked);
                    if let Some(h) = hint {
                        it = it.hint(h);
                    }
                    entries.push(it.entry());
                }
            }
        }
        self.show_menu(x, y, entries, keys.first().cloned(), true);
    }

    /// Up on the screen as `display` (shown, if it's running): its menu's
    /// Open. A double-click opens it in the Builder instead (`Shell::edit`).
    fn open_space_as(&self, id: &str, display: SessionDisplay) {
        if self.shell.store.is_running(id) {
            self.shell.show(id);
        } else {
            self.shell.open(id, display);
        }
    }

    /// Into the Trash: wadspaces (wadd's, with an Undo) and files.
    fn trash(&self, keys: &[String]) {
        for id in keys.iter().filter_map(|k| k.strip_prefix("ws:")) {
            self.shell.op(Op::Trash { ws: id.to_string() }, |_| {});
        }
        let paths = self.file_paths(keys);
        if paths.is_empty() {
            return;
        }
        match files::trash(&paths) {
            Ok(()) => {
                let title = match paths.as_slice() {
                    [one] => format!("{} is in the Trash", one.file_name().and_then(|n| n.to_str()).unwrap_or("It")),
                    many => format!("{} files are in the Trash", many.len()),
                };
                self.toast(Level::Info, &title);
            }
            Err(e) => self.toast(Level::Warn, &format!("Couldn't put it in the Trash: {e}")),
        }
        self.reload_files();
    }

    /// Asks first (the spike's "Empty the Trash?"), beside the Trash.
    fn ask_empty_trash(&self) {
        let (x, y) = match self.s.borrow().spots.get("trash").copied() {
            Some(spot) => {
                let (x, y) = spot.origin();
                (x + grid::ICON_W / 2.0, y + 40.0)
            }
            None => (0.0, 0.0),
        };
        let entries = vec![
            Entry::Heading("Empty the Trash?".into()),
            MenuItem::new("empty-trash-yes", "Empty Trash", "trash", self.on(|m| m.empty_trash()))
                .hint("Everything in it goes for good")
                .danger()
                .entry(),
            MenuItem::new("cancel", "Cancel", "x", || {}).entry(),
        ];
        self.show_menu(x, y, entries, Some("trash".into()), true);
    }

    /// Its files gone for good, and its wadspaces (wadd deletes each, with everything in it).
    fn empty_trash(&self) {
        let trashed = self.trashed_spaces();
        for id in trashed {
            let s = self.shell.clone();
            self.shell.link.call(
                move |w| async move { w.purge_wadspace(&id).await },
                move |r| {
                    if let Err(e) = r {
                        s.toast(Level::Warn, &format!("A wadspace in the Trash didn't go: {}", e.message), None);
                    }
                },
            );
        }
        self.off_thread(files::empty_trash, |m, r| match r {
            Ok(()) => m.toast(Level::Info, "The Trash is empty"),
            Err(e) => m.toast(Level::Warn, &format!("Couldn't empty the Trash: {e}")),
        });
    }

    /// This machine's wadspaces in the Trash.
    fn trashed_spaces(&self) -> Vec<String> {
        let me = self.shell.store.machine_id();
        self.shell
            .store
            .library()
            .wadspaces
            .iter()
            .filter(|x| x.is_trashed() && x.machine == me)
            .map(|x| x.id.clone())
            .collect()
    }

    /// A new wadspace: empty glass after everything; it opens in the
    /// Builder, its title ready to type there.
    fn new_wadspace(&self) {
        self.shell.new_wadspace();
    }

    fn new_folder(&self) {
        let name = files::free_name(&self.desk_dir, "New folder");
        match std::fs::create_dir(self.desk_dir.join(&name)) {
            Ok(()) => {
                self.s.borrow_mut().pending_rename = Some(format!("file:{name}"));
                self.reload_files();
            }
            Err(e) => self.toast(Level::Warn, &format!("Couldn't make a folder: {e}")),
        }
    }

    /// The selection's files on the clipboard (to be moved when pasted, if `cut`).
    fn copy(&self, keys: &[String], cut: bool) {
        let paths = self.file_paths(keys);
        if paths.is_empty() {
            return;
        }
        let list: Vec<gio::File> = paths.iter().map(gio::File::for_path).collect();
        let gnome = format!(
            "{}\n{}",
            if cut { "cut" } else { "copy" },
            list.iter().map(|f| f.uri().to_string()).collect::<Vec<_>>().join("\n")
        );
        let p = gdk::ContentProvider::new_union(&[
            gdk::ContentProvider::for_value(&gdk::FileList::from_array(&list).to_value()),
            gdk::ContentProvider::for_bytes(
                "x-special/gnome-copied-files",
                &glib::Bytes::from_owned(gnome.into_bytes()),
            ),
        ]);
        if let Err(e) = self.board.clipboard().set_content(Some(&p)) {
            tracing::warn!("the clipboard: {e}");
        }
        self.s.borrow_mut().cut = cut.then_some(paths);
    }

    fn can_paste(&self) -> bool {
        self.board.clipboard().formats().contains_type(gdk::FileList::static_type())
    }

    /// The clipboard's files into `dir` (moved, if they were cut here).
    fn paste(&self, dir: PathBuf) {
        let (w, clip) = (self.weak(), self.board.clipboard());
        glib::spawn_future_local(async move {
            let got = clip.read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT).await;
            let Some(m) = w.upgrade() else { return };
            let paths = got.ok().and_then(|v| v.get::<gdk::FileList>().ok()).map(|l| files::local(&l.files()));
            let Some(paths) = paths.filter(|p| !p.is_empty()) else {
                m.toast(Level::Info, "There are no files on the clipboard");
                return;
            };
            let cut = m.s.borrow().cut.as_ref() == Some(&paths);
            if cut {
                m.s.borrow_mut().cut = None;
            }
            m.off_thread(
                move || if cut { files::move_into(&paths, &dir) } else { files::copy_into(&paths, &dir) },
                |m, r| {
                    if let Err(e) = r {
                        m.toast(Level::Warn, &e);
                    }
                },
            );
        });
    }

    fn open_terminal(&self) {
        let dir = self.desk_dir.to_string_lossy().into_owned();
        let tries: [(&str, Vec<String>); 6] = [
            ("xdg-terminal-exec", vec![]),
            ("foot", vec!["--working-directory".into(), dir.clone()]),
            ("ptyxis", vec!["--new-window".into(), "--working-directory".into(), dir.clone()]),
            ("gnome-terminal", vec![format!("--working-directory={dir}")]),
            ("konsole", vec!["--workdir".into(), dir.clone()]),
            ("xterm", vec![]),
        ];
        for (prog, args) in tries {
            let Some(path) = glib::find_program_in_path(prog) else { continue };
            let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::NONE);
            launcher.set_cwd(&self.desk_dir);
            let mut argv: Vec<&std::ffi::OsStr> = vec![path.as_os_str()];
            argv.extend(args.iter().map(std::ffi::OsStr::new));
            match launcher.spawn(&argv) {
                Ok(_) => return,
                Err(e) => tracing::warn!("{prog}: {e}"),
            }
        }
        self.toast(Level::Warn, "There's no terminal on this machine");
    }

    fn select_all(&self) {
        let mut s = self.s.borrow_mut();
        s.sel = s.order.clone();
        drop(s);
        self.restyle();
    }

    // ------------------------------------------------------------ keys

    /// The desktop surface's keys: Esc, Enter, F2, Delete, Ctrl+A, Ctrl+N,
    /// Ctrl+C/X/V. False: not one of ours (machine.rs's PageUp, say).
    pub fn key(&self, key: gdk::Key, mods: gdk::ModifierType) -> bool {
        if self.menu.borrow().is_some() {
            return false;
        }
        if self.rename.borrow().is_some() {
            if key == gdk::Key::Escape {
                self.end_rename(false);
                return true;
            }
            return false;
        }
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let sel = self.s.borrow().sel.clone();
        let lower = key.to_lower();
        match key {
            gdk::Key::Escape => {
                if self.s.borrow().open_tag.is_some() {
                    self.close_folder();
                } else if !sel.is_empty() {
                    self.s.borrow_mut().sel.clear();
                    self.restyle();
                } else {
                    return false;
                }
            }
            gdk::Key::Return | gdk::Key::KP_Enter if sel.len() == 1 => self.open_key(&sel[0]),
            gdk::Key::F2 if sel.len() == 1 && self.can_rename(&sel[0]) => self.start_rename(&sel[0]),
            gdk::Key::Delete | gdk::Key::KP_Delete
                if sel.iter().any(|k| k.starts_with("ws:") || k.starts_with("file:")) =>
            {
                self.trash(&sel)
            }
            _ if ctrl && lower == gdk::Key::a => self.select_all(),
            _ if ctrl && lower == gdk::Key::n => self.new_wadspace(),
            _ if ctrl && lower == gdk::Key::c && !self.file_paths(&sel).is_empty() => self.copy(&sel, false),
            _ if ctrl && lower == gdk::Key::x && !self.file_paths(&sel).is_empty() => self.copy(&sel, true),
            _ if ctrl && lower == gdk::Key::v => self.paste_here(),
            _ => return false,
        }
        true
    }

    // ------------------------------------------------------------ menus

    /// The menu for what's at (x, y): an icon's (it selected, if it wasn't),
    /// or the background's.
    fn context_menu(&self, x: f64, y: f64, autohide: bool) {
        if self.rename.borrow().is_some() {
            self.end_rename(true);
        }
        let hit = self.hit(x, y);
        let (entries, menu_for) = match hit {
            Hit::Icon(k) => {
                {
                    let mut s = self.s.borrow_mut();
                    s.sel = select::pick_for(&s.sel, &k);
                }
                if !k.starts_with("folder:") && self.s.borrow().open_tag.is_some() {
                    self.close_folder();
                }
                (self.menu_for(&k), Some(k))
            }
            Hit::Inside(id) => (self.space_menu(&id, &[format!("ws:{id}")], false), None),
            Hit::Panel => return,
            Hit::Background => {
                self.s.borrow_mut().sel.clear();
                if self.s.borrow().open_tag.is_some() {
                    self.close_folder();
                }
                (self.board_menu(), None)
            }
        };
        if entries.is_empty() {
            return;
        }
        self.show_menu(x, y, entries, menu_for, autohide);
    }

    fn show_menu(&self, x: f64, y: f64, entries: Vec<Entry>, menu_for: Option<String>, autohide: bool) {
        self.menu_at.set((x, y));
        let old = self.menu.borrow_mut().take();
        if let Some(old) = old {
            old.popover.popdown();
        }
        self.s.borrow_mut().menu_for = menu_for;
        self.restyle();
        let w = self.weak();
        let open = menu::show(&self.board, x, y, entries, autohide, move || {
            if let Some(m) = w.upgrade() {
                m.menu.borrow_mut().take();
                m.s.borrow_mut().menu_for = None;
                m.restyle();
            }
        });
        *self.menu.borrow_mut() = Some(open);
    }

    fn close_menu(&self) {
        let open = self.menu.borrow_mut().take();
        if let Some(o) = open {
            o.popover.popdown();
        }
    }

    fn menu_for(&self, key: &str) -> Vec<Entry> {
        let sel = self.s.borrow().sel.clone();
        if let Some(id) = key.strip_prefix("ws:") {
            self.space_menu(id, &sel, true)
        } else if let Some(tag) = key.strip_prefix("folder:") {
            self.folder_menu(tag)
        } else if key == "trash" {
            self.trash_menu()
        } else if key == "machines" {
            self.machines_menu()
        } else {
            self.file_menu(key, &sel)
        }
    }

    /// A closure on this desktop.
    fn on(&self, f: impl Fn(&Rc<Self>) + 'static) -> impl Fn() + 'static {
        let w = self.weak();
        move || {
            if let Some(m) = w.upgrade() {
                f(&m);
            }
        }
    }

    fn space_menu(&self, id: &str, sel: &[String], on_desk: bool) -> Vec<Entry> {
        let Some(x) = self.shell.store.wadspace(id) else { return Vec::new() };
        let running = self.shell.store.is_running(id);
        let key = format!("ws:{id}");
        let ids: Vec<String> = if sel.contains(&key) { sel.to_vec() } else { vec![key.clone()] };
        let mut out = vec![Entry::Heading(x.name.clone())];
        let i = id.to_string();
        out.push(
            MenuItem::new("open", if running { "Show" } else { "Open" }, "monitor-play", {
                let i = i.clone();
                self.on(move |m| m.open_space_as(&i, SessionDisplay::Screen))
            })
            .entry(),
        );
        out.push(
            MenuItem::new("open-window", "Open in window", "app-window", {
                let i = i.clone();
                self.on(move |m| m.open_space_as(&i, SessionDisplay::Window))
            })
            .entry(),
        );
        // Stream: the stream sheet (Private | Public), or its link while it streams.
        let streaming = self.shell.store.stream(id);
        out.push(
            MenuItem::new("stream", "Stream", "radio", {
                let (i, on) = (i.clone(), streaming.is_some());
                move || {
                    if on {
                        crate::streams::share(&i);
                    } else {
                        crate::streams::sheet(&i, wad_model::StreamMode::Private);
                    }
                }
            })
            .hint(match &streaming {
                Some(s) if s.live => "Streaming now: show its link",
                Some(_) => "Starting…",
                None => "Privately on your tailnet, or publicly",
            })
            .checked(streaming.is_some())
            .entry(),
        );
        out.push(
            MenuItem::new("edit", "Edit in the Builder", "blocks", {
                let i = i.clone();
                self.on(move |m| m.shell.edit(&i))
            })
            .hint("What's in it, to change (or double-click it)")
            .key("Enter")
            .entry(),
        );
        out.push(
            MenuItem::new("rename", "Rename", "pencil", {
                let (k, on_desk) = (key.clone(), on_desk);
                self.on(move |m| {
                    if on_desk {
                        m.start_rename(&k);
                    }
                })
            })
            .key("F2")
            .off((!on_desk).then_some("Drag it out of its folder first"))
            .entry(),
        );
        out.push(
            MenuItem::new("duplicate", "Duplicate", "copy-plus", {
                let i = i.clone();
                self.on(move |m| m.shell.op(Op::Duplicate { ws: i.clone(), machine: None }, |_| {}))
            })
            .hint("A copy beside it, to change")
            .entry(),
        );
        out.push(
            MenuItem::new("agents", "Agents", "bot", {
                let (i, on) = (i.clone(), !x.agents);
                self.on(move |m| m.shell.op(Op::SetAgents { ws: i.clone(), on }, |_| {}))
            })
            .hint("They may message this desktop and ask for sandboxes")
            .checked(x.agents)
            .entry(),
        );
        out.push(self.keep_paste_item(id));
        out.push(Entry::Sep);
        out.push(
            MenuItem::new("trash", "Move to Trash", "trash", self.on(move |m| m.trash(&ids)))
                .key("Delete")
                .danger()
                .entry(),
        );
        out
    }

    fn folder_menu(&self, tag: &str) -> Vec<Entry> {
        let Some(t) = self.shell.store.library().tags.iter().find(|t| t.id == tag).cloned() else { return Vec::new() };
        let (open_tag, ren, del) = (tag.to_string(), format!("folder:{tag}"), tag.to_string());
        vec![
            Entry::Heading(t.name.clone()),
            MenuItem::new("folder-open", "Open", "folder-open", self.on(move |m| m.open_folder(&open_tag)))
                .key("Enter")
                .entry(),
            MenuItem::new("tag-rename", "Rename…", "pencil", self.on(move |m| m.start_rename(&ren))).key("F2").entry(),
            Entry::Sep,
            MenuItem::new(
                "tag-remove",
                "Delete the folder",
                "trash",
                self.on(move |m| m.shell.op(Op::RemoveTag { tag: del.clone() }, |_| {})),
            )
            .hint("Its wadspaces go back on the desktop, untagged")
            .danger()
            .entry(),
        ]
    }

    fn file_menu(&self, key: &str, sel: &[String]) -> Vec<Entry> {
        let Some(e) = self.entry(key) else { return Vec::new() };
        let ids: Vec<String> = if sel.iter().any(|k| k == key) { sel.to_vec() } else { vec![key.to_string()] };
        let folder = e.kind == files::Kind::Folder;
        let (k_open, k_ren) = (key.to_string(), key.to_string());
        let (c1, c2, t, ow) = (ids.clone(), ids.clone(), ids.clone(), ids.clone());
        let dir = e.path.clone();
        let paste_off = if !folder {
            Some("Paste into a folder")
        } else if !self.can_paste() {
            Some("The clipboard is empty")
        } else {
            None
        };
        vec![
            MenuItem::new(
                "open",
                "Open",
                if folder { "folder-open" } else { "file" },
                self.on(move |m| m.open_key(&k_open)),
            )
            .key("Enter")
            .entry(),
            MenuItem::new("open-with", "Open with", "app-window", self.on(move |m| m.open_with_menu(&ow))).entry(),
            Entry::Sep,
            MenuItem::new("cut", "Cut", "scissors", self.on(move |m| m.copy(&c1, true))).key("Ctrl+X").entry(),
            MenuItem::new("copy", "Copy", "copy", self.on(move |m| m.copy(&c2, false))).key("Ctrl+C").entry(),
            MenuItem::new("paste", "Paste", "clipboard-paste", self.on(move |m| m.paste(dir.clone())))
                .key("Ctrl+V")
                .off(paste_off)
                .entry(),
            MenuItem::new("rename", "Rename", "pencil", self.on(move |m| m.start_rename(&k_ren)))
                .key("F2")
                .off((ids.len() > 1).then_some("One at a time"))
                .entry(),
            Entry::Sep,
            MenuItem::new("trash", "Move to Trash", "trash", self.on(move |m| m.trash(&t)))
                .key("Delete")
                .danger()
                .entry(),
        ]
    }

    fn trash_menu(&self) -> Vec<Entry> {
        let full = self.s.borrow().trash_files > 0 || !self.trashed_spaces().is_empty();
        vec![
            MenuItem::new("open", "Open", "folder-open", self.on(|m| m.open_key("trash"))).entry(),
            MenuItem::new("empty-trash", "Empty Trash", "trash", self.on(|m| m.ask_empty_trash()))
                .off((!full).then_some("It's empty"))
                .danger()
                .entry(),
        ]
    }

    fn machines_menu(&self) -> Vec<Entry> {
        vec![
            MenuItem::new("open", "Open", "folder-open", self.on(|m| m.open_key("machines"))).entry(),
            MenuItem::new("tailscale", "Tailscale settings", "network", {
                let s = self.shell.clone();
                move || crate::peers::open_app(&s, &["wados-settings", "--page", "accounts"])
            })
            .entry(),
        ]
    }

    fn board_menu(&self) -> Vec<Entry> {
        let dir = self.desk_dir.clone();
        let settings = |what: &'static str| self.on(move |m| m.toast(Level::Info, what));
        vec![
            MenuItem::new("new-wadspace", "New wadspace", "plus", self.on(|m| m.new_wadspace())).key("Ctrl+N").entry(),
            MenuItem::new("new-folder", "New folder", "folder-plus", self.on(|m| m.new_folder())).entry(),
            MenuItem::new("paste", "Paste", "clipboard-paste", self.on(move |m| m.paste(dir.clone())))
                .key("Ctrl+V")
                .off((!self.can_paste()).then_some("The clipboard is empty"))
                .entry(),
            MenuItem::new("terminal", "Open terminal here", "square-terminal", self.on(|m| m.open_terminal())).entry(),
            Entry::Sep,
            MenuItem::new(
                "background",
                "Change background",
                "image",
                settings("Changing the background comes with Settings"),
            )
            .entry(),
            MenuItem::new(
                "displays",
                "Display settings",
                "monitor-cog",
                settings("Display settings come with Settings"),
            )
            .entry(),
        ]
    }

    // ------------------------------------------------------------ wadsctl

    /// `wadsctl desk …`, for tests:
    ///
    /// - `desk`: how many icons, what's selected, the open folder
    /// - `desk icons`: every icon's key and spot (`key@c,r ; …`)
    /// - `desk sel`; `desk select all|none|<key> [; <key>…]`
    /// - `desk where <key>`: its middle on the screen (logical pixels)
    /// - `desk at <x> <y>`: what's at a point of the desktop
    /// - `desk open <key>`; `desk folder [<tag>|close]`; `desk rename <key>`
    /// - `desk menu [<key>|background]`: opens its menu (or lists the open
    ///   one's items, `id:label[:off]`); `desk pick <id>`; `desk unmenu`
    /// - `desk key <name> [ctrl]`: a key as the desktop has it (Delete, F2…)
    /// - `desk trash`; `desk reload`; `desk saved` (desk.json)
    pub fn ctl(&self, args: &[&str]) -> String {
        let rest = args.get(1..).unwrap_or(&[]).join(" ");
        let keys_of = |rest: &str| -> Vec<String> {
            rest.split(';').map(str::trim).filter(|k| !k.is_empty()).map(String::from).collect()
        };
        let list = |v: &[String]| v.join(" ; ");
        match args {
            [] => {
                let s = self.s.borrow();
                let focus = self.board.root().and_downcast::<gtk::Window>().is_some_and(|w| w.is_active());
                format!(
                    "ok icons={} sel={} open={} rows={} renaming={} menu={} focus={focus}",
                    s.order.len(),
                    s.sel.len(),
                    s.open_tag.as_deref().unwrap_or("-"),
                    self.rows(),
                    self.rename.borrow().as_ref().map(|(k, _)| k.as_str()).unwrap_or("-"),
                    self.menu.borrow().is_some()
                )
            }
            ["icons"] => {
                let s = self.s.borrow();
                let v: Vec<String> = s.spots.iter().map(|(k, p)| format!("{k}@{},{}", p.c, p.r)).collect();
                format!("ok {}", list(&v))
            }
            ["saved"] => {
                let s = self.s.borrow();
                let v: Vec<String> = s.saved.iter().map(|(k, p)| format!("{k}@{},{}", p.c, p.r)).collect();
                format!("ok {}", list(&v))
            }
            ["sel"] => format!("ok {}", list(&self.s.borrow().sel)),
            ["select", "all"] => {
                self.select_all();
                "ok".into()
            }
            ["select", "none"] => {
                self.s.borrow_mut().sel.clear();
                self.restyle();
                "ok".into()
            }
            ["select", ..] => {
                let want = keys_of(&rest);
                let order = self.s.borrow().order.clone();
                if let Some(bad) = want.iter().find(|k| !order.contains(k)) {
                    return format!("error no icon {bad}");
                }
                self.s.borrow_mut().sel = want;
                self.restyle();
                "ok".into()
            }
            ["where", ..] if rest.starts_with("in:") => match self.inside_point(&rest[3..]) {
                Some((x, y)) => {
                    let (sx, sy) = self.to_screen(x, y);
                    format!("ok {sx:.0} {sy:.0}")
                }
                None => format!("error {rest} isn't in the open folder"),
            },
            ["where", ..] => {
                let Some(spot) = self.s.borrow().spots.get(&rest).copied() else {
                    return format!("error no icon {rest}");
                };
                let (x, y) = spot.origin();
                let (sx, sy) = self.to_screen(x + grid::ICON_W / 2.0, y + 40.0);
                format!("ok {sx:.0} {sy:.0}")
            }
            ["at", x, y] => {
                let (Ok(x), Ok(y)) = (x.parse::<f64>(), y.parse::<f64>()) else { return "error at <x> <y>".into() };
                format!("ok {:?}", self.hit(x, y))
            }
            ["open", ..] => {
                if !self.s.borrow().order.contains(&rest) {
                    return format!("error no icon {rest}");
                }
                self.open_key(&rest);
                "ok".into()
            }
            ["folder"] => format!("ok {}", self.s.borrow().open_tag.as_deref().unwrap_or("-")),
            ["folder", "close"] => {
                self.close_folder();
                "ok".into()
            }
            ["folder", tag] => {
                self.open_folder(tag);
                format!("ok {}", self.s.borrow().open_tag.as_deref().unwrap_or("-"))
            }
            ["rename", ..] => {
                self.start_rename(&rest);
                format!("ok {}", self.rename.borrow().is_some())
            }
            ["menu"] => match self.menu.borrow().as_ref() {
                Some(o) => {
                    let v: Vec<String> = o
                        .items
                        .iter()
                        .map(
                            |(id, label, on, _)| {
                                if *on { format!("{id}:{label}") } else { format!("{id}:{label}:off") }
                            },
                        )
                        .collect();
                    format!("ok {}", list(&v))
                }
                None => "ok none".into(),
            },
            ["menu", ..] => {
                let (x, y) = if rest == "background" {
                    let (w, h) = self.desk_size();
                    (w - 200.0, h - 200.0)
                } else if let Some(p) = self.s.borrow().spots.get(&rest) {
                    let (x, y) = p.origin();
                    (x + 60.0, y + 50.0)
                } else if let Some(id) = rest.strip_prefix("in:") {
                    match self.inside_point(id) {
                        Some(p) => p,
                        None => return format!("error {rest} isn't in the open folder"),
                    }
                } else {
                    return format!("error no icon {rest}");
                };
                self.context_menu(x, y, false);
                self.ctl(&["menu"])
            }
            ["unmenu"] => {
                self.close_menu();
                "ok".into()
            }
            ["pick", id] => {
                let run =
                    self.menu.borrow().as_ref().and_then(|o| {
                        o.items.iter().find(|(i, _, _, _)| i == id).map(|(_, _, on, r)| (*on, r.clone()))
                    });
                match run {
                    Some((true, Some(r))) => {
                        self.close_menu();
                        r();
                        "ok".into()
                    }
                    Some(_) => format!("error {id} is off"),
                    None => format!("error no {id} on the menu"),
                }
            }
            ["key", name, more @ ..] => {
                let Some(k) = gdk::Key::from_name(*name) else { return format!("error no key {name}") };
                let m =
                    if more.contains(&"ctrl") { gdk::ModifierType::CONTROL_MASK } else { gdk::ModifierType::empty() };
                format!("ok {}", self.key(k, m))
            }
            ["trash"] => {
                let s = self.s.borrow();
                format!(
                    "ok files={} full={}",
                    s.trash_files,
                    matches!(self.items().last(), Some((_, Item::Trash { full: true })))
                )
            }
            ["reload"] => {
                self.s.borrow_mut().saved = arrange::load(&self.desk_json);
                self.reload_files();
                "ok".into()
            }
            _ => format!("error unknown: desk {}", args.join(" ")),
        }
    }

    /// A wadspace's middle in the open folder, in the desktop's pixels.
    fn inside_point(&self, id: &str) -> Option<(f64, f64)> {
        let panel = self.panel.borrow();
        let root = panel.as_ref()?.root.clone();
        let w = find_named(&root.upcast(), &format!("in:{id}"))?;
        let b = w.compute_bounds(&self.board)?;
        Some(((b.x() + b.width() / 2.0) as f64, (b.y() + 36.0) as f64))
    }

    /// A point of the desktop on the screen (the surface sits under the bar,
    /// to the screen's bottom).
    fn to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        let Some(root) = self.board.root() else { return (x, y + BAR as f64) };
        let p = self
            .board
            .compute_point(&root, &graphene::Point::new(x as f32, y as f32))
            .unwrap_or(graphene::Point::new(x as f32, y as f32));
        let (_, sh) = self.shell.size();
        let top = sh as f64 - root.height() as f64;
        (p.x() as f64, p.y() as f64 + top)
    }
}

/// One wadspace carried (from the desktop, or out of a folder), as it's held.
/// A desktop file, as Open with has it.
fn open_file(e: &files::Entry) -> crate::open_with::File {
    crate::open_with::File {
        path: e.path.to_string_lossy().into_owned(),
        name: e.name.clone(),
        mime: e.content_type.clone(),
        folder: e.kind == files::Kind::Folder,
    }
}

fn held_of(keys: &[String], from_folder: bool) -> Option<Held> {
    let [k] = keys else { return None };
    let id = k.strip_prefix("ws:")?;
    Some(Held::Space {
        ws: id.to_string(),
        from: Some(if from_folder { SpaceFrom::Folder } else { SpaceFrom::Desktop }),
    })
}

/// A wadspace's drag's icon: its picture, and the hint under it (what a target elsewhere would do).
fn held_ghost(picture: &gdk::Paintable) -> gtk::Widget {
    let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
    let pic = gtk::Picture::for_paintable(picture);
    pic.set_can_shrink(false);
    pic.set_halign(gtk::Align::Start);
    col.append(&pic);
    col.append(&dnd::hint_label());
    col.upcast()
}

fn mods(state: gdk::ModifierType) -> Mods {
    Mods {
        ctrl: state
            .intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::META_MASK | gdk::ModifierType::SUPER_MASK),
        shift: state.contains(gdk::ModifierType::SHIFT_MASK),
    }
}

/// Each wadspace in the open folder (`in:<id>`) under `w`.
fn each_inside(w: &gtk::Widget, f: &mut impl FnMut(&gtk::Widget)) {
    let mut c = w.first_child();
    while let Some(ch) = c {
        if ch.widget_name().starts_with("in:") {
            f(&ch);
        } else {
            each_inside(&ch, f);
        }
        c = ch.next_sibling();
    }
}

/// `w` or the first widget under it named `name`.
fn find_named(w: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if w.widget_name() == name {
        return Some(w.clone());
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        if let Some(f) = find_named(&ch, name) {
            return Some(f);
        }
        c = ch.next_sibling();
    }
    None
}
