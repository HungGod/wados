//! Files' window (the spike's `apps/files/Files.svelte`): a sidebar
//! (`Sidebar.svelte`: Places, this machine's Wadspaces, Machines) beside
//! what's open, under a bar of Back, Forward, Up, where it is (crumbs, the
//! middle folded deep down), a filter, grid or list, and New folder; a note
//! under it (Moved to the Trash, with Undo), the copies coming here
//! (`Transfers.svelte`), and how many things it shows.
//!
//! What's open: a folder here or the Trash (places.rs), one of this
//! machine's wadspaces (read-only), or Machines (machines.rs).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use gtk::{gdk, gio, glib, prelude::*};
use wad_gtk::Icon;
use wad_gtk::form::{Opt, Segmented};
use wad_gtk::kit::{self, Variant};
use wad_model::{JobKind, JobStatus};

use crate::drive::{self, Entry};
use crate::link::Link;
use crate::local;
use crate::state::{self, Loc, Spaces, State};

pub const CSS: &str = r#"
window.files-window { background-color: var(--surface); }
.files-side { background-color: var(--bg-2); border-right: 1px solid var(--line); padding: 10px 8px 16px; }
.files-h5 { margin: 14px 10px 6px; font-size: 11px; font-weight: 700; letter-spacing: 0.07em; color: var(--faint); }
.files-h5.first { margin-top: 4px; }
.files-entry { min-height: 32px; padding: 0 10px; border-radius: 8px; color: var(--muted); font-size: 13px; }
.files-entry:hover { background-color: var(--surface-2); color: var(--fg); }
.files-entry.on { background-color: var(--surface-3); color: var(--fg); box-shadow: inset 2px 0 0 var(--accent); }
.files-entry.sub { padding-left: 22px; }
.files-count { min-width: 18px; padding: 0 5px; border-radius: 99px; background-color: var(--warn); color: var(--accent-fg); font-size: 11px; font-weight: 700; }
.files-dot { min-width: 7px; min-height: 7px; border-radius: 99px; background-color: var(--surface-3); box-shadow: inset 0 0 0 1px var(--line-strong); }
.files-dot.on { background-color: var(--accent); box-shadow: 0 0 6px var(--accent); }
.files-pip { min-width: 7px; min-height: 7px; border-radius: 99px; background-color: var(--accent-2); }
.files-bar { min-height: 48px; padding: 0 10px 0 8px; border-bottom: 1px solid var(--line); }
button.files-crumb { min-height: 28px; padding: 0 6px; border-radius: 7px; border: none; background: none; box-shadow: none; color: var(--muted); font-size: 13px; }
button.files-crumb:hover { background-color: var(--surface-2); color: var(--fg); }
button.files-crumb.last { color: var(--fg); font-weight: 600; }
.files-chev { color: var(--faint); }
.files-filter { min-height: 32px; padding: 0 10px; border-radius: 9px; border: 1px solid var(--line); background-color: var(--bg-2); color: var(--faint); }
.files-filter text { color: var(--fg); font-size: 13px; }
.files-body { padding: 10px 12px 24px; }
.files-status { min-height: 28px; padding: 0 14px; border-top: 1px solid var(--line); background-color: var(--bg-2); font-size: 12px; color: var(--muted); }
.files-status .faint { color: var(--faint); }
.files-note { padding: 8px 10px 8px 16px; border-bottom: 1px solid var(--line); background-color: var(--surface-2); font-size: 13px; color: var(--muted); }

/* A machine (MachinesView's .device). */
.device { padding: 18px 18px 16px; border-radius: 16px; border: 1px solid var(--line); background-image: linear-gradient(160deg, var(--surface-2), var(--bg-2)); }
.device:hover { border-color: var(--line-strong); }
.device.selected { border-color: rgb(198 255 31 / 0.5); background-image: linear-gradient(160deg, rgb(198 255 31 / 0.1), var(--bg-2)); }
.device-glyph { min-width: 64px; min-height: 64px; border-radius: 18px; background-image: linear-gradient(160deg, #123247, #0c1b2b); color: var(--c-app); box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.1); margin-bottom: 8px; }
.device.offline .device-glyph { background-image: none; background-color: var(--surface-3); color: var(--faint); }
.device-dot { min-width: 14px; min-height: 14px; border-radius: 99px; background-color: var(--surface-3); box-shadow: 0 0 0 3px var(--surface-2); margin: 0 -3px -3px 0; }
.device-dot.on { background-color: var(--accent); box-shadow: 0 0 0 3px var(--surface-2), 0 0 10px var(--accent); }
.device-name { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 17px; }
.device-host { font-family: monospace; font-size: 11.5px; color: var(--muted); }
.device-status { font-size: 12.5px; color: var(--faint); }
.device-status.on { color: var(--accent); }
.device-fact { font-size: 12px; color: var(--muted); }

/* Someone asking (MachinesView's .ask). */
.files-ask { padding: 12px 12px 12px 14px; margin-bottom: 14px; border-radius: 12px; border: 1px solid rgb(232 195 106 / 0.35); background-color: var(--warn-soft); }
.files-ask-badge { min-width: 36px; min-height: 36px; border-radius: 10px; background-color: rgb(232 195 106 / 0.18); color: var(--warn); }
.files-ask-sub { font-size: 12.5px; color: var(--muted); }

/* A folder's things (Item.svelte), a machine's wadspaces. */
.item { padding: 10px 6px 8px; border-radius: 10px; }
.item:hover { background-color: rgb(255 255 255 / 0.04); }
.item.selected { background-color: rgb(198 255 31 / 0.12); box-shadow: inset 0 0 0 1px rgb(198 255 31 / 0.35); }
.item.cut { opacity: 0.45; }
.item-glyph { color: var(--c-app); }
.item-glyph.file { color: var(--muted); }
.item-name { font-size: 12.5px; }
.item-sub { font-size: 11.5px; color: var(--muted); }
.item-rename { font-size: 12.5px; min-height: 24px; padding: 0 5px; border-radius: 6px; border: 1px solid var(--accent); background-color: var(--bg); }
.row { min-height: 34px; padding: 0 12px; border-radius: 8px; font-size: 13px; }
.row:hover { background-color: rgb(255 255 255 / 0.04); }
.row.selected { background-color: rgb(198 255 31 / 0.12); box-shadow: inset 0 0 0 1px rgb(198 255 31 / 0.35); }
.row .muted { color: var(--muted); }
.head { min-height: 32px; padding: 0 12px; border-bottom: 1px solid var(--line); margin-bottom: 4px; }
.head label { color: var(--faint); font-size: 11.5px; font-weight: 600; }
.files-empty { color: var(--faint); font-size: 13px; }
.files-strip { padding: 8px 12px 8px 16px; border-bottom: 1px solid var(--line); background-color: var(--bg-2); color: var(--muted); font-size: 12.5px; }
.files-section { margin: 4px 4px 10px; font-size: 11px; font-weight: 700; letter-spacing: 0.08em; color: var(--faint); }
.space-row { padding: 10px 12px; border-radius: 14px; border: 1px solid var(--line); background-color: var(--surface-2); }
.space-name { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 15px; }
.files-away { color: var(--faint); font-size: 13px; }
.files-away-title { color: var(--muted); font-size: 14px; font-weight: 600; }
.ws-h2 { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 22px; }
.ws-fact { font-size: 12.5px; color: var(--muted); }
.ws-fact.on { color: var(--accent); }
.ws-app { padding: 4px 10px 4px 6px; border-radius: 99px; background-color: var(--surface-2); font-size: 12.5px; }
.xfer { padding: 4px 12px; color: var(--c-app); font-size: 12.5px; }
.xfers { border-top: 1px solid var(--line); background-color: var(--bg-2); padding: 4px 0; }
"#;

/// Grid or list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Grid,
    List,
}

/// A folder's listing on another machine: its address, and what came (none yet: it's coming).
#[derive(Default)]
pub struct Listing {
    pub uri: String,
    pub got: Option<Result<Vec<Entry>, String>>,
}

pub struct Files {
    pub window: gtk::ApplicationWindow,
    pub(crate) app: gtk::Application,
    pub(crate) link: Link,
    pub(crate) me: Weak<Files>,
    pub state: RefCell<State>,
    pub(crate) home: PathBuf,
    pub(crate) loc: RefCell<Loc>,
    back: RefCell<Vec<Loc>>,
    forward: RefCell<Vec<Loc>>,
    side: gtk::Box,
    crumbs: gtk::Box,
    back_b: gtk::Button,
    fwd_b: gtk::Button,
    up_b: gtk::Button,
    pub(crate) filter: gtk::Text,
    seg: Segmented<View>,
    new_b: gtk::Button,
    pub(crate) body: gtk::Box,
    status: gtk::Box,
    status_l: gtk::Label,
    status_r: gtk::Label,
    xfers: gtk::Box,
    /// What's picked in the view (a machine's id, a thing's name; the Trash's `file:`/`ws:` keys).
    pub(crate) selected: RefCell<Vec<String>>,
    /// The last one clicked (Shift picks from it).
    pub(crate) anchor: RefCell<Option<String>>,
    pub(crate) view: Cell<View>,
    pub(crate) listing: RefCell<Listing>,
    /// The folder here that's open: what's in it.
    pub(crate) here: RefCell<Option<Result<Vec<local::Item>, String>>>,
    /// The Trash's files (its wadspaces are the state's).
    pub(crate) trash: RefCell<Vec<local::Trashed>>,
    /// The thing being renamed (its name).
    pub(crate) renaming: RefCell<Option<String>>,
    /// What was cut (Ctrl+X): moved when it's pasted.
    pub(crate) cut: RefCell<Option<Vec<PathBuf>>>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    queued: Cell<bool>,
    /// Peers asked for their wadspaces when they came online.
    asked: RefCell<HashMap<String, bool>>,
    /// A note under the bar (Moved to the Trash, a refusal), with its Undo.
    note: gtk::Box,
    note_l: gtk::Label,
    note_undo: gtk::Button,
    /// The Trash's names of what the note's Undo restores.
    pub(crate) undo: RefCell<Vec<String>>,
}

impl Files {
    pub fn new(app: &gtk::Application) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Files")
            .default_width(1000)
            .default_height(680)
            .css_classes(["files-window"])
            .build();
        kit::title_bar(&window, "folder", "Files");
        let side = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(false)
            .css_classes(["files-side"])
            .build();
        side.set_size_request(196, -1);
        let side_scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&side).build();
        let main = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        let bar = gtk::Box::builder().spacing(8).css_classes(["files-bar"]).build();
        let nav = gtk::Box::builder().spacing(2).valign(gtk::Align::Center).build();
        let back_b = kit::button("", Variant::Ghost, Some("arrow-left"), true);
        back_b.set_tooltip_text(Some("Back (Alt+←)"));
        let fwd_b = kit::button("", Variant::Ghost, Some("arrow-right"), true);
        fwd_b.set_tooltip_text(Some("Forward (Alt+→)"));
        let up_b = kit::button("", Variant::Ghost, Some("arrow-up"), true);
        up_b.set_tooltip_text(Some("Up (Alt+↑)"));
        nav.append(&back_b);
        nav.append(&fwd_b);
        nav.append(&up_b);
        bar.append(&nav);
        let crumbs = gtk::Box::builder().spacing(2).hexpand(true).valign(gtk::Align::Center).build();
        bar.append(&crumbs);
        let fbox = gtk::Box::builder()
            .spacing(6)
            .css_classes(["files-filter"])
            .valign(gtk::Align::Center)
            .hexpand(false)
            .build();
        fbox.set_size_request(150, -1);
        let fi = Icon::new("search", 14);
        fi.set_valign(gtk::Align::Center);
        fbox.append(&fi);
        let filter = gtk::Text::builder().placeholder_text("Filter").valign(gtk::Align::Center).build();
        fbox.append(&filter);
        bar.append(&fbox);
        let seg_cb: Rc<RefCell<Option<Weak<Files>>>> = Rc::default();
        let cb = seg_cb.clone();
        let seg = Segmented::new(
            [Opt::from(("", View::Grid)).icon("layout-grid"), Opt::from(("", View::List)).icon("list")],
            View::Grid,
            true,
            move |v| {
                if let Some(m) = cb.borrow().as_ref().and_then(Weak::upgrade) {
                    m.view.set(*v);
                    m.queue_draw();
                }
            },
        )
        .named("view");
        bar.append(&seg.root);
        let new_b = kit::button("New folder", Variant::Secondary, Some("folder-plus"), true);
        new_b.set_widget_name("new-folder");
        new_b.set_valign(gtk::Align::Center);
        bar.append(&new_b);
        main.append(&bar);
        let note = gtk::Box::builder().spacing(10).css_classes(["files-note"]).visible(false).build();
        note.append(&Icon::new("info", 15));
        let note_l = gtk::Label::builder().xalign(0.0).hexpand(true).wrap(true).build();
        note.append(&note_l);
        let note_undo = kit::button("Undo", Variant::Secondary, Some("undo-2"), true);
        note.append(&note_undo);
        let note_x = kit::button("", Variant::Ghost, Some("x"), true);
        note_x.set_tooltip_text(Some("Dismiss"));
        note.append(&note_x);
        main.append(&note);
        let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["files-body"]).build();
        scroll.set_child(Some(&body));
        main.append(&scroll);
        let xfers =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["xfers"]).visible(false).build();
        main.append(&xfers);
        let status = gtk::Box::builder().spacing(12).css_classes(["files-status"]).build();
        let status_l = gtk::Label::builder().xalign(0.0).hexpand(true).build();
        let status_r = gtk::Label::builder().xalign(1.0).css_classes(["faint"]).build();
        status.append(&status_l);
        status.append(&status_r);
        main.append(&status);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.append(&side_scroll);
        row.append(&main);
        window.set_child(Some(&row));

        let home = glib::home_dir();
        let me = Rc::new_cyclic(|me: &Weak<Files>| {
            let w = me.clone();
            let link = Link::start(move |i| {
                if let Some(m) = w.upgrade() {
                    m.incoming(i);
                }
            });
            Self {
                window,
                app: app.clone(),
                link,
                me: me.clone(),
                state: RefCell::default(),
                loc: RefCell::new(Loc::Local(home.clone())),
                home,
                back: RefCell::default(),
                forward: RefCell::default(),
                side,
                crumbs,
                back_b,
                fwd_b,
                up_b,
                filter,
                seg,
                new_b,
                body,
                status,
                status_l,
                status_r,
                xfers,
                selected: RefCell::default(),
                anchor: RefCell::default(),
                view: Cell::new(View::Grid),
                listing: RefCell::default(),
                here: RefCell::default(),
                trash: RefCell::default(),
                renaming: RefCell::default(),
                cut: RefCell::default(),
                monitor: RefCell::default(),
                queued: Cell::new(false),
                asked: RefCell::default(),
                note,
                note_l,
                note_undo,
                undo: RefCell::default(),
            }
        });
        *seg_cb.borrow_mut() = Some(me.me.clone());
        let w = me.me.clone();
        note_x.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.note.set_visible(false);
            }
        });
        me.wire();
        me.entered();
        me
    }

    fn wire(self: &Rc<Self>) {
        let w = self.me.clone();
        self.back_b.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.go_back();
            }
        });
        let w = self.me.clone();
        self.fwd_b.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.go_forward();
            }
        });
        let w = self.me.clone();
        self.up_b.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.go_up();
            }
        });
        let w = self.me.clone();
        self.new_b.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.new_folder();
            }
        });
        let w = self.me.clone();
        self.note_undo.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.undo_trash();
            }
        });
        let w = self.me.clone();
        self.filter.connect_changed(move |_| {
            if let Some(m) = w.upgrade() {
                m.queue_draw();
            }
        });
        let fk = gtk::EventControllerKey::new();
        let w = self.me.clone();
        fk.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape
                && let Some(m) = w.upgrade()
            {
                m.filter.set_text("");
                m.body.grab_focus();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.filter.add_controller(fk);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = self.me.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(m) = w.upgrade() else { return glib::Propagation::Proceed };
            if m.key(key, mods) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
        });
        self.window.add_controller(keys);
    }

    /// A key in the window: true if Files did something with it.
    fn key(&self, key: gdk::Key, mods: gdk::ModifierType) -> bool {
        if mods.contains(gdk::ModifierType::ALT_MASK) {
            match key {
                gdk::Key::Left => self.go_back(),
                gdk::Key::Right => self.go_forward(),
                gdk::Key::Up => self.go_up(),
                _ => return false,
            }
            return true;
        }
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        if ctrl && matches!(key, gdk::Key::f | gdk::Key::F) {
            self.filter.grab_focus();
            return true;
        }
        // Typing (the filter, a name): its own keys.
        if self.filter.has_focus() || self.renaming.borrow().is_some() {
            return false;
        }
        self.view_key(key, ctrl, mods.contains(gdk::ModifierType::SHIFT_MASK))
    }

    fn incoming(&self, i: wad_wadd_client::Incoming) {
        use wad_wadd_client::Incoming;
        let changed = match i {
            Incoming::Event(e) => {
                let mut st = self.state.borrow_mut();
                let was = st.connected;
                st.connected = true;
                st.apply(e) || !was
            }
            Incoming::Disconnected => std::mem::replace(&mut self.state.borrow_mut().connected, false),
            Incoming::Other(..) => false,
        };
        if changed {
            self.ask_online();
            // A folder asked for before wadd said where things are: now.
            let waiting = matches!(&*self.loc.borrow(), Loc::Folder { .. }) && {
                let l = self.listing.borrow();
                l.uri.is_empty() && l.got.is_none()
            };
            if waiting && self.state.borrow().ready() {
                self.list();
            }
            self.queue_draw();
        }
    }

    /// Each machine that came online is asked for its wadspaces (once each time).
    fn ask_online(&self) {
        let online: Vec<(String, bool)> =
            self.state.borrow().machines().iter().map(|p| (p.peer.id.clone(), p.peer.online)).collect();
        for (id, on) in online {
            let was = self.asked.borrow_mut().insert(id.clone(), on);
            if on && was != Some(true) {
                self.fetch(&id);
            }
        }
    }

    /// Asks `peer` for its wadspaces.
    fn fetch(&self, peer: &str) {
        {
            let mut st = self.state.borrow_mut();
            let s = st.spaces.entry(peer.to_string()).or_default();
            if s.loading {
                return;
            }
            s.loading = true;
        }
        let (w, id, p) = (self.me.clone(), peer.to_string(), peer.to_string());
        self.link.call(
            move |wadd| async move { wadd.peer_wadspaces(&p).await },
            move |r| {
                let Some(m) = w.upgrade() else { return };
                {
                    let mut st = m.state.borrow_mut();
                    let s = st.spaces.entry(id).or_default();
                    s.loading = false;
                    match r {
                        Ok(mut list) => {
                            list.retain(|x| !x.is_trashed());
                            list.sort_by_key(|x| x.order);
                            *s = Spaces { list, loading: false, error: None, at: now_ms() };
                        }
                        Err(e) => s.error = Some(e.message),
                    }
                }
                m.queue_draw();
            },
        );
    }

    // ------------------------------------------------------------ where it is

    /// Goes to `loc` (Back comes here again).
    pub fn go(&self, loc: Loc) {
        let was = self.loc.replace(loc.clone());
        if was != loc {
            self.back.borrow_mut().push(was);
            self.forward.borrow_mut().clear();
        }
        self.entered();
    }

    fn go_back(&self) {
        let Some(l) = self.back.borrow_mut().pop() else { return };
        let was = self.loc.replace(l);
        self.forward.borrow_mut().push(was);
        self.entered();
    }

    fn go_forward(&self) {
        let Some(l) = self.forward.borrow_mut().pop() else { return };
        let was = self.loc.replace(l);
        self.back.borrow_mut().push(was);
        self.entered();
    }

    pub(crate) fn go_up(&self) {
        let up = self.loc.borrow().up();
        if let Some(u) = up {
            self.go(u);
        }
    }

    /// Somewhere new: its listing asked for, the filter and the pick cleared.
    fn entered(&self) {
        self.selected.borrow_mut().clear();
        *self.renaming.borrow_mut() = None;
        self.filter.set_text("");
        self.note.set_visible(false);
        *self.monitor.borrow_mut() = None;
        let loc = self.loc.borrow().clone();
        match &loc {
            Loc::Peer(p) => self.fetch(p),
            Loc::Folder { .. } => self.list(),
            Loc::Local(p) => {
                self.reload();
                self.watch(p);
            }
            Loc::Trash => {
                self.reload();
                self.watch(&local::trash_dir().join("files"));
            }
            Loc::Machines | Loc::Space(_) => {}
        }
        self.queue_draw();
    }

    /// What's here read again (the folder open, or the Trash).
    pub(crate) fn reload(&self) {
        match &*self.loc.borrow() {
            Loc::Local(p) => *self.here.borrow_mut() = Some(local::list(p)),
            Loc::Trash => *self.trash.borrow_mut() = local::trash_list(),
            _ => {}
        }
    }

    /// A change in `dir` reads it again.
    fn watch(&self, dir: &Path) {
        let _ = std::fs::create_dir_all(dir);
        let Ok(mon) =
            gio::File::for_path(dir).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        else {
            return;
        };
        mon.set_rate_limit(300);
        let w = self.me.clone();
        mon.connect_changed(move |_, _, _, _| {
            if let Some(m) = w.upgrade() {
                // Not while a name's typed (it'd go).
                if m.renaming.borrow().is_none() {
                    m.reload();
                    m.queue_draw();
                }
            }
        });
        *self.monitor.borrow_mut() = Some(mon);
    }

    /// The home's address of `peer`, over Taildrive.
    fn home_of(&self, peer: &str) -> Option<String> {
        let st = self.state.borrow();
        let t = st.tailnet.as_ref()?;
        let p = st.peer(peer)?;
        Some(drive::home_uri(&drive::base(), &t.tailnet, &p.dns_name, &p.peer.hostname))
    }

    /// Lists the folder open on another machine.
    fn list(&self) {
        let Loc::Folder { peer, path } = self.loc.borrow().clone() else { return };
        if !self.state.borrow().ready() {
            // Listed once wadd says where it is.
            *self.listing.borrow_mut() = Listing::default();
            return;
        }
        let Some(home) = self.home_of(&peer) else {
            *self.listing.borrow_mut() =
                Listing { uri: String::new(), got: Some(Err("That machine isn't on your tailnet any more".into())) };
            return;
        };
        let uri = drive::folder_uri(&home, &path);
        *self.listing.borrow_mut() = Listing { uri: uri.clone(), got: None };
        let w = self.me.clone();
        glib::spawn_future_local(async move {
            let got = drive::list(&uri).await.map_err(|e| {
                tracing::info!("listing {uri}: {e}");
                drive::why(&e)
            });
            let Some(m) = w.upgrade() else { return };
            if m.listing.borrow().uri == uri {
                m.listing.borrow_mut().got = Some(got);
                m.queue_draw();
            }
        });
    }

    pub(crate) fn open_entry(&self, e: &Entry) {
        let Loc::Folder { peer, mut path } = self.loc.borrow().clone() else { return };
        if e.folder {
            path.push(e.name.clone());
            self.go(Loc::Folder { peer, path });
            return;
        }
        let uri = format!("{}/{}", self.listing.borrow().uri, glib::Uri::escape_string(&e.name, None, true));
        let (w, name) = (self.me.clone(), e.display.clone());
        drive::open(&uri, move |r| {
            if let (Err(err), Some(m)) = (r, w.upgrade()) {
                m.say(&format!("Nothing here opens {name}: {}", err.message()));
            }
        });
    }

    /// A note under the bar.
    pub(crate) fn say(&self, what: &str) {
        self.undo.borrow_mut().clear();
        self.note_undo.set_visible(false);
        self.note_l.set_label(what);
        self.note.set_visible(true);
    }

    pub(crate) fn hide_note(&self) {
        self.note.set_visible(false);
    }

    /// A note with Undo, which restores `names` from the Trash.
    pub(crate) fn say_undo(&self, what: &str, names: Vec<String>) {
        self.note_l.set_label(what);
        self.note_undo.set_visible(!names.is_empty());
        *self.undo.borrow_mut() = names;
        self.note.set_visible(true);
    }

    pub(crate) fn entries(&self) -> Vec<Entry> {
        match &self.listing.borrow().got {
            Some(Ok(list)) => list.clone(),
            _ => vec![],
        }
    }

    // ------------------------------------------------------------ the pick

    pub(crate) fn sel_first(&self) -> Option<String> {
        self.selected.borrow().first().cloned()
    }

    pub(crate) fn is_sel(&self, k: &str) -> bool {
        self.selected.borrow().iter().any(|s| s == k)
    }

    pub(crate) fn pick_only(&self, k: &str) {
        *self.selected.borrow_mut() = vec![k.to_string()];
        *self.anchor.borrow_mut() = Some(k.to_string());
        self.queue_draw();
    }

    /// A click on `k`: it alone, Ctrl adds or takes it away, Shift picks from the last one (in `order`).
    pub(crate) fn click_pick(&self, k: &str, mods: gdk::ModifierType, order: &[String]) {
        if mods.contains(gdk::ModifierType::SHIFT_MASK)
            && let Some(a) = self.anchor.borrow().clone()
            && let (Some(i), Some(j)) = (order.iter().position(|x| *x == a), order.iter().position(|x| x == k))
        {
            let (lo, hi) = (i.min(j), i.max(j));
            *self.selected.borrow_mut() = order[lo..=hi].to_vec();
            self.queue_draw();
            return;
        }
        if mods.contains(gdk::ModifierType::CONTROL_MASK) {
            let mut s = self.selected.borrow_mut();
            match s.iter().position(|x| x == k) {
                Some(i) => {
                    s.remove(i);
                }
                None => s.push(k.to_string()),
            }
            drop(s);
            *self.anchor.borrow_mut() = Some(k.to_string());
            self.queue_draw();
            return;
        }
        self.pick_only(k);
    }

    // ------------------------------------------------------------ drawing

    pub fn queue_draw(&self) {
        if self.queued.replace(true) {
            return;
        }
        let w = self.me.clone();
        glib::idle_add_local_once(move || {
            if let Some(m) = w.upgrade() {
                m.queued.set(false);
                m.draw();
            }
        });
    }

    pub(crate) fn q(&self) -> String {
        self.filter.text().trim().to_lowercase()
    }

    pub fn draw(&self) {
        let loc = self.loc.borrow().clone();
        self.draw_side(&loc);
        self.draw_crumbs(&loc);
        self.back_b.set_sensitive(!self.back.borrow().is_empty());
        self.fwd_b.set_sensitive(!self.forward.borrow().is_empty());
        self.up_b.set_sensitive(loc.up().is_some());
        let local = matches!(loc, Loc::Local(_));
        self.new_b.set_sensitive(local && matches!(&*self.here.borrow(), Some(Ok(_))));
        self.seg.set(self.view.get());
        while let Some(c) = self.body.first_child() {
            self.body.remove(&c);
        }
        let connected = self.state.borrow().connected;
        let (left, right) = match &loc {
            Loc::Local(p) => {
                let n = self.local_view(p);
                (n.map(|n| self.count(n)), n.map(|_| free_space(p)).unwrap_or_default())
            }
            Loc::Trash => (Some(self.count(self.trash_view())), String::new()),
            Loc::Space(id) => {
                self.space_view(id);
                (None, String::new())
            }
            _ if !connected => {
                self.body.append(&kit::hero(
                    "circle-alert",
                    "wadd isn't answering",
                    "Files shows your other machines once wadd, which knows them, answers.",
                ));
                (None, String::new())
            }
            Loc::Machines => (Some(self.machines_view()).filter(|s| !s.is_empty()), String::new()),
            Loc::Peer(p) => (Some(self.peer_view(p)).filter(|s| !s.is_empty()), String::new()),
            Loc::Folder { peer, path } => (Some(self.folder_view(peer, path)).filter(|s| !s.is_empty()), String::new()),
        };
        self.status_l.set_label(left.as_deref().unwrap_or(""));
        self.status_r.set_label(&right);
        self.status.set_visible(left.is_some());
        self.draw_transfers();
        self.window.set_title(Some(&self.title(&loc)));
    }

    /// "4 items · 1 selected".
    fn count(&self, n: usize) -> String {
        let sel = self.selected.borrow().len();
        let mut s = format!("{n} item{}", if n == 1 { "" } else { "s" });
        if sel > 0 {
            s.push_str(&format!(" · {sel} selected"));
        }
        s
    }

    fn title(&self, loc: &Loc) -> String {
        let st = self.state.borrow();
        let name = |p: &str| st.peer(p).map(|x| x.peer.name.clone()).unwrap_or_else(|| "Machines".into());
        match loc {
            Loc::Local(p) if *p == self.home => "Home".into(),
            Loc::Local(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Files".into()),
            Loc::Trash => "Trash".into(),
            Loc::Space(id) => st.wadspace(id).map(|x| x.name.clone()).unwrap_or_else(|| "Wadspace".into()),
            Loc::Machines => "Machines".into(),
            Loc::Peer(p) => name(p),
            Loc::Folder { peer, path } => {
                format!("{} on {}", path.last().map(String::as_str).unwrap_or("Home"), name(peer))
            }
        }
    }

    /// The places: Home and its folders.
    pub(crate) fn places(&self) -> [(&'static str, &'static str, PathBuf); 5] {
        let h = &self.home;
        [
            ("Home", "house", h.clone()),
            ("Desktop", "monitor", h.join("Desktop")),
            ("Documents", "file-text", h.join("Documents")),
            ("Downloads", "download", h.join("Downloads")),
            ("Pictures", "image", h.join("Pictures")),
        ]
    }

    fn draw_side(&self, loc: &Loc) {
        while let Some(c) = self.side.first_child() {
            self.side.remove(&c);
        }
        self.side.append(&kit::label("PLACES", &["files-h5", "first"]));
        for (label, icon, path) in self.places() {
            let e = entry(icon, label);
            e.set_widget_name(&format!("place-{label}"));
            if *loc == Loc::Local(path.clone()) {
                e.add_css_class("on");
            }
            self.click(&e, Loc::Local(path));
            self.side.append(&e);
        }
        let st = self.state.borrow();
        let t = entry("trash", "Trash");
        t.set_widget_name("place-Trash");
        if !local::trash_list().is_empty() || !st.trashed().is_empty() {
            let pip = gtk::Box::builder().css_classes(["files-pip"]).valign(gtk::Align::Center).build();
            pip.set_tooltip_text(Some("It has things in it"));
            t.append(&pip);
        }
        if *loc == Loc::Trash {
            t.add_css_class("on");
        }
        self.click(&t, Loc::Trash);
        self.side.append(&t);
        let mine = st.live();
        if !mine.is_empty() {
            self.side.append(&kit::label("WADSPACES", &["files-h5"]));
            for x in &mine {
                let e = entry("monitor", &x.name);
                e.set_widget_name(&format!("ws-{}", x.id));
                if st.running.contains(&x.id) {
                    let dot = gtk::Box::builder().css_classes(["files-dot", "on"]).valign(gtk::Align::Center).build();
                    dot.set_tooltip_text(Some("Running"));
                    e.append(&dot);
                }
                if *loc == Loc::Space(x.id.clone()) {
                    e.add_css_class("on");
                }
                self.click(&e, Loc::Space(x.id.clone()));
                self.side.append(&e);
            }
        }
        self.side.append(&kit::label("MACHINES", &["files-h5"]));
        let all = entry("server", "Machines");
        let asks = crate::machines::asks(&st).len();
        if asks > 0 {
            let c = gtk::Label::builder()
                .label(asks.to_string())
                .css_classes(["files-count"])
                .valign(gtk::Align::Center)
                .build();
            c.set_tooltip_text(Some("Asking to be trusted"));
            all.append(&c);
        }
        if *loc == Loc::Machines {
            all.add_css_class("on");
        }
        self.click(&all, Loc::Machines);
        self.side.append(&all);
        if st.running() {
            for p in st.machines() {
                let e = entry(crate::machines::kind_icon(p.peer.kind), &p.peer.name);
                e.add_css_class("sub");
                let dot = gtk::Box::builder().css_classes(["files-dot"]).valign(gtk::Align::Center).build();
                if p.peer.online {
                    dot.add_css_class("on");
                }
                e.append(&dot);
                if loc.peer() == Some(p.peer.id.as_str()) {
                    e.add_css_class("on");
                }
                self.click(&e, Loc::Peer(p.peer.id.clone()));
                self.side.append(&e);
            }
        }
    }

    /// A click on `w` goes to `to` (a place that's gone is made: ~/Desktop and the rest).
    fn click(&self, w: &impl IsA<gtk::Widget>, to: Loc) {
        let g = gtk::GestureClick::new();
        let me = self.me.clone();
        g.connect_released(move |g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(m) = me.upgrade() {
                if let Loc::Local(p) = &to {
                    let _ = std::fs::create_dir_all(p);
                }
                m.go(to.clone());
            }
        });
        w.add_controller(g);
    }

    fn draw_crumbs(&self, loc: &Loc) {
        while let Some(c) = self.crumbs.first_child() {
            self.crumbs.remove(&c);
        }
        let list = {
            let st = self.state.borrow();
            state::crumbs(loc, &self.home, |p| {
                st.peer(p)
                    .map(|x| x.peer.name.clone())
                    .or_else(|| st.wadspace(p).map(|x| x.name.clone()))
                    .unwrap_or_else(|| p.to_string())
            })
        };
        let (shown, folded) = state::shown_crumbs(&list);
        let n = shown.len();
        let icon_only = !folded.is_empty();
        for (i, c) in shown.into_iter().enumerate() {
            if i > 0 {
                let ch = Icon::new("chevron-right", 13);
                ch.add_css_class("files-chev");
                ch.set_valign(gtk::Align::Center);
                self.crumbs.append(&ch);
            }
            let Some(c) = c else {
                let b = gtk::Button::builder().child(&Icon::new("ellipsis", 14)).css_classes(["files-crumb"]).build();
                b.set_tooltip_text(Some("Further up"));
                let (w, folded) = (self.me.clone(), folded.clone());
                b.connect_clicked(move |b| {
                    let Some(m) = w.upgrade() else { return };
                    let rows = folded
                        .iter()
                        .map(|c| {
                            let (w, at) = (m.me.clone(), c.at.clone());
                            crate::menu::item("crumb", &c.label, c.icon.unwrap_or("folder"), move || {
                                if let Some(m) = w.upgrade() {
                                    m.go(at.clone());
                                }
                            })
                            .row()
                        })
                        .collect();
                    crate::menu::show(b, 0.0, b.height() as f64, rows);
                });
                self.crumbs.append(&b);
                continue;
            };
            let inner = gtk::Box::builder().spacing(6).build();
            if let Some(ic) = c.icon {
                inner.append(&Icon::new(ic, 14));
            }
            if !(icon_only && i == 0 && c.icon.is_some()) {
                inner.append(&gtk::Label::builder().label(&c.label).ellipsize(gtk::pango::EllipsizeMode::End).build());
            }
            let b = gtk::Button::builder().child(&inner).css_classes(["files-crumb"]).tooltip_text(&c.label).build();
            if i + 1 == n {
                b.add_css_class("last");
            }
            let (w, at) = (self.me.clone(), c.at.clone());
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.go(at.clone());
                }
            });
            self.crumbs.append(&b);
        }
    }

    /// Copies coming here (CopyWadspace jobs): how far along each is.
    fn draw_transfers(&self) {
        while let Some(c) = self.xfers.first_child() {
            self.xfers.remove(&c);
        }
        let st = self.state.borrow();
        let jobs: Vec<_> = st
            .copies
            .values()
            .filter(|j| {
                j.job.kind == JobKind::CopyWadspace && matches!(j.job.status, JobStatus::Queued | JobStatus::Running)
            })
            .collect();
        for j in &jobs {
            let r = gtk::Box::builder().spacing(10).css_classes(["xfer"]).build();
            r.append(&Icon::new("download", 15));
            let what = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).hexpand(true).build();
            what.append(&kit::label(&j.job.title, &[]));
            let p = gtk::ProgressBar::builder().fraction(j.job.progress.clamp(0.0, 1.0)).build();
            if j.job.total_bytes == 0 {
                p.pulse();
            }
            what.append(&p);
            r.append(&what);
            r.append(&kit::label(
                &format!(
                    "{} of {}",
                    drive::bytes(j.job.done_bytes),
                    drive::bytes(j.job.total_bytes.max(j.job.done_bytes))
                ),
                &["item-sub"],
            ));
            self.xfers.append(&r);
        }
        self.xfers.set_visible(!jobs.is_empty());
    }

    /// Whether what's open has all it'll show (for `--dump`).
    pub fn settled(&self) -> bool {
        let st = self.state.borrow();
        match &*self.loc.borrow() {
            Loc::Local(_) => self.here.borrow().is_some(),
            Loc::Trash | Loc::Space(_) => st.connected && st.library.is_some(),
            _ if !st.connected || st.tailnet.is_none() => false,
            Loc::Machines => {
                st.machines().iter().all(|p| !p.peer.online || st.spaces.get(&p.peer.id).is_some_and(|s| !s.loading))
            }
            Loc::Peer(p) => st.spaces.get(p).is_some_and(|s| !s.loading),
            Loc::Folder { .. } => self.listing.borrow().got.is_some(),
        }
    }

    /// What it shows, in words (`--dump`, for tests).
    pub fn dump(&self) -> String {
        fn walk(w: &gtk::Widget, out: &mut Vec<String>) {
            if !w.is_visible() {
                return;
            }
            if let Some(l) = w.downcast_ref::<gtk::Label>() {
                let t = l.label();
                if !t.is_empty() {
                    out.push(t.to_string());
                }
            }
            if let Some(e) = w.downcast_ref::<gtk::Text>() {
                let t = e.text();
                if !t.is_empty() {
                    out.push(format!("[{t}]"));
                }
            }
            let mut c = w.first_child();
            while let Some(x) = c {
                walk(&x, out);
                c = x.next_sibling();
            }
        }
        let mut out = vec![];
        if let Some(c) = self.window.child() {
            walk(&c, &mut out);
        }
        out.join(" | ")
    }
}

pub(crate) fn entry(icon: &'static str, label: &str) -> gtk::Box {
    let e = gtk::Box::builder().spacing(10).css_classes(["files-entry"]).build();
    let ic = Icon::new(icon, 16);
    ic.set_valign(gtk::Align::Center);
    e.append(&ic);
    let l = gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .valign(gtk::Align::Center)
        .build();
    e.append(&l);
    e
}

/// A thing in a folder: its glyph, its name, what's under it.
pub(crate) fn item(icon: &'static str, name: &str, sub: &str, selected: bool) -> gtk::Box {
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).css_classes(["item"]).build();
    b.set_size_request(112, 112);
    if selected {
        b.add_css_class("selected");
    }
    let ic = Icon::new(icon, 48);
    ic.set_size_request(60, 60);
    ic.add_css_class("item-glyph");
    if icon != "folder" {
        ic.add_css_class("file");
    }
    ic.set_halign(gtk::Align::Center);
    b.append(&ic);
    b.append(&name_label(name, selected));
    if !sub.is_empty() {
        b.append(&gtk::Label::builder().label(sub).css_classes(["item-sub"]).build());
    }
    b
}

pub(crate) fn name_label(name: &str, selected: bool) -> gtk::Label {
    gtk::Label::builder()
        .label(name)
        .justify(gtk::Justification::Center)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .lines(if selected { 4 } else { 2 })
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(14)
        .css_classes(["item-name"])
        .build()
}

/// "120 GB free" on the disk `p` is on.
fn free_space(p: &Path) -> String {
    gio::File::for_path(p)
        .query_filesystem_info("filesystem::free", gio::Cancellable::NONE)
        .ok()
        .map(|i| format!("{:.0} GB free", i.attribute_uint64("filesystem::free") as f64 / 1e9))
        .unwrap_or_default()
}

pub fn now_ms() -> i64 {
    glib::real_time() / 1000
}
