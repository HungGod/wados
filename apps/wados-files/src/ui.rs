//! Files' window (the spike's `apps/files/Files.svelte`): a sidebar of
//! places beside what's open, under a bar of Back, Forward, Up, where it is
//! (crumbs) and a filter. Machines only now (`MachinesView.svelte`,
//! `PeerView.svelte`):
//!
//! - **Machines:** my other machines on the tailnet as device tiles (Online,
//!   or when it was last seen; what it runs; how many wadspaces), any device
//!   asking to be trusted on top. Off the tailnet: a way to join it.
//! - **A machine:** its Home (over Taildrive) and its wadspaces, each with
//!   Copy here; offline, it says so.
//! - **A folder in its home:** what's in it, over Taildrive; a folder opens
//!   in Files, a file with its default app.
//!
//! The rest (this machine's places and Trash) comes in phase 8.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gtk::{gdk, glib, prelude::*};
use wad_gtk::Icon;
use wad_gtk::kit::{self, Variant};
use wad_model::PeerKind;
use wad_proto::{TailnetAsk, TailnetState};

use crate::drive::{self, Entry};
use crate::link::Link;
use crate::state::{self, Loc, Spaces, State};

pub const CSS: &str = r#"
window.files-window { background-color: var(--surface); }
.files-side { background-color: var(--bg-2); border-right: 1px solid var(--line); min-width: 220px; padding: 10px 10px 14px; }
.files-h5 { margin: 12px 10px 6px; font-size: 11px; font-weight: 700; letter-spacing: 0.08em; color: var(--faint); }
.files-entry { min-height: 32px; padding: 0 10px; border-radius: 8px; color: var(--muted); font-size: 13px; }
.files-entry:hover { background-color: var(--surface-2); color: var(--fg); }
.files-entry.on { background-color: var(--surface-3); color: var(--fg); box-shadow: inset 2px 0 0 var(--accent); }
.files-entry.sub { padding-left: 22px; }
.files-entry.off { opacity: 0.45; }
.files-count { min-width: 18px; padding: 0 5px; border-radius: 99px; background-color: var(--warn); color: var(--accent-fg); font-size: 11px; font-weight: 700; }
.files-dot { min-width: 7px; min-height: 7px; border-radius: 99px; background-color: var(--surface-3); box-shadow: inset 0 0 0 1px var(--line-strong); }
.files-dot.on { background-color: var(--accent); box-shadow: 0 0 6px var(--accent); }
.files-bar { min-height: 48px; padding: 0 10px 0 8px; border-bottom: 1px solid var(--line); }
button.files-crumb { min-height: 28px; padding: 0 6px; border-radius: 7px; border: none; background: none; box-shadow: none; color: var(--muted); font-size: 13px; }
button.files-crumb:hover { background-color: var(--surface-2); color: var(--fg); }
button.files-crumb.last { color: var(--fg); font-weight: 600; }
.files-chev { color: var(--faint); }
.files-filter { min-height: 32px; min-width: 150px; padding: 0 10px; border-radius: 9px; border: 1px solid var(--line); background-color: var(--bg-2); color: var(--faint); }
.files-filter text { color: var(--fg); font-size: 13px; }
.files-body { padding: 18px 20px 28px; }
.files-status { min-height: 30px; padding: 0 14px; border-top: 1px solid var(--line); font-size: 12px; color: var(--muted); }

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

/* A folder's things, a machine's wadspaces. */
.item { padding: 10px 8px 8px; border-radius: 12px; }
.item:hover { background-color: var(--surface-2); }
.item.selected { background-color: rgb(198 255 31 / 0.12); box-shadow: inset 0 0 0 1px rgb(198 255 31 / 0.4); }
.item-glyph { color: var(--c-app); }
.item-glyph.file { color: var(--muted); }
.item-name { font-size: 13px; }
.item-sub { font-size: 11.5px; color: var(--muted); }
.files-section { margin: 4px 4px 10px; font-size: 11px; font-weight: 700; letter-spacing: 0.08em; color: var(--faint); }
.space-row { padding: 10px 12px; border-radius: 14px; border: 1px solid var(--line); background-color: var(--surface-2); }
.space-name { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 15px; }
.files-away { color: var(--faint); font-size: 13px; }
.files-away-title { color: var(--muted); font-size: 14px; font-weight: 600; }
"#;

/// A menu's row: its icon, name, hint, and what it does.
type MenuRow<'a> = (&'static str, &'a str, Option<String>, Box<dyn Fn()>);

/// A folder's listing: its address, and what came (none yet: it's coming).
#[derive(Default)]
struct Listing {
    uri: String,
    got: Option<Result<Vec<Entry>, String>>,
}

pub struct Files {
    pub window: gtk::ApplicationWindow,
    link: Link,
    me: Weak<Files>,
    pub state: RefCell<State>,
    loc: RefCell<Loc>,
    back: RefCell<Vec<Loc>>,
    forward: RefCell<Vec<Loc>>,
    side: gtk::Box,
    crumbs: gtk::Box,
    back_b: gtk::Button,
    fwd_b: gtk::Button,
    up_b: gtk::Button,
    filter: gtk::Text,
    body: gtk::Box,
    status: gtk::Label,
    /// What's picked in the view (a machine's id, a thing's name).
    selected: RefCell<Option<String>>,
    listing: RefCell<Listing>,
    queued: Cell<bool>,
    /// Peers asked for their wadspaces when they came online.
    asked: RefCell<HashMap<String, bool>>,
    /// A note under the bar (a refusal, a file that didn't open).
    note: gtk::Label,
}

impl Files {
    pub fn new(app: &gtk::Application) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Files")
            .default_width(1000)
            .default_height(700)
            .css_classes(["files-window"])
            .build();
        kit::title_bar(&window, "folder", "Files");
        let side = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(false)
            .css_classes(["files-side"])
            .build();
        side.set_size_request(230, -1);
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
        fbox.set_size_request(170, -1);
        let fi = Icon::new("search", 14);
        fi.set_valign(gtk::Align::Center);
        fbox.append(&fi);
        let filter = gtk::Text::builder().placeholder_text("Filter").valign(gtk::Align::Center).build();
        fbox.append(&filter);
        bar.append(&fbox);
        main.append(&bar);
        let note = kit::label("", &["files-note"]);
        note.set_margin_start(20);
        note.set_margin_top(10);
        note.set_visible(false);
        main.append(&note);
        let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["files-body"]).build();
        scroll.set_child(Some(&body));
        main.append(&scroll);
        let status = gtk::Label::builder().xalign(0.0).css_classes(["files-status"]).build();
        main.append(&status);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.append(&side);
        row.append(&main);
        window.set_child(Some(&row));

        let me = Rc::new_cyclic(|me: &Weak<Files>| {
            let w = me.clone();
            let link = Link::start(move |i| {
                if let Some(m) = w.upgrade() {
                    m.incoming(i);
                }
            });
            Self {
                window,
                link,
                me: me.clone(),
                state: RefCell::default(),
                loc: RefCell::new(Loc::Machines),
                back: RefCell::default(),
                forward: RefCell::default(),
                side,
                crumbs,
                back_b,
                fwd_b,
                up_b,
                filter,
                body,
                status,
                selected: RefCell::default(),
                listing: RefCell::default(),
                queued: Cell::new(false),
                asked: RefCell::default(),
                note,
            }
        });
        me.wire();
        me.draw();
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
                let up = m.loc.borrow().up();
                if let Some(u) = up {
                    m.go(u);
                }
            }
        });
        let w = self.me.clone();
        self.filter.connect_changed(move |_| {
            if let Some(m) = w.upgrade() {
                m.queue_draw();
            }
        });
        let keys = gtk::EventControllerKey::new();
        let w = self.me.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(m) = w.upgrade() else { return glib::Propagation::Proceed };
            if mods.contains(gdk::ModifierType::ALT_MASK) {
                match key {
                    gdk::Key::Left => m.go_back(),
                    gdk::Key::Right => m.go_forward(),
                    gdk::Key::Up => {
                        let up = m.loc.borrow().up();
                        if let Some(u) = up {
                            m.go(u);
                        }
                    }
                    _ => return glib::Propagation::Proceed,
                }
                return glib::Propagation::Stop;
            }
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) && !m.filter.has_focus() {
                m.open_selected();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.window.add_controller(keys);
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

    /// Somewhere new: its listing asked for, the filter and the pick cleared.
    fn entered(&self) {
        *self.selected.borrow_mut() = None;
        self.filter.set_text("");
        self.note.set_visible(false);
        let loc = self.loc.borrow().clone();
        match &loc {
            Loc::Peer(p) => self.fetch(p),
            Loc::Folder { .. } => self.list(),
            Loc::Machines => {}
        }
        self.queue_draw();
    }

    /// The home's address of `peer`, over Taildrive.
    fn home_of(&self, peer: &str) -> Option<String> {
        let st = self.state.borrow();
        let t = st.tailnet.as_ref()?;
        let p = st.peer(peer)?;
        Some(drive::home_uri(&drive::base(), &t.tailnet, &p.dns_name, &p.peer.hostname))
    }

    /// Lists the folder open.
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

    fn open_selected(&self) {
        let Some(sel) = self.selected.borrow().clone() else { return };
        let loc = self.loc.borrow().clone();
        match loc {
            Loc::Machines => self.go(Loc::Peer(sel)),
            Loc::Peer(p) if sel == "home" => self.go(Loc::Folder { peer: p, path: vec![] }),
            Loc::Folder { .. } => {
                let e = self.entries().into_iter().find(|e| e.name == sel);
                if let Some(e) = e {
                    self.open_entry(&e);
                }
            }
            _ => {}
        }
    }

    fn open_entry(&self, e: &Entry) {
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

    fn say(&self, what: &str) {
        self.note.set_label(what);
        self.note.set_visible(true);
    }

    fn entries(&self) -> Vec<Entry> {
        match &self.listing.borrow().got {
            Some(Ok(list)) => list.clone(),
            _ => vec![],
        }
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

    fn q(&self) -> String {
        self.filter.text().trim().to_lowercase()
    }

    pub fn draw(&self) {
        let loc = self.loc.borrow().clone();
        self.draw_side(&loc);
        self.draw_crumbs(&loc);
        self.back_b.set_sensitive(!self.back.borrow().is_empty());
        self.fwd_b.set_sensitive(!self.forward.borrow().is_empty());
        self.up_b.set_sensitive(loc.up().is_some());
        while let Some(c) = self.body.first_child() {
            self.body.remove(&c);
        }
        let connected = self.state.borrow().connected;
        let status = if !connected {
            self.body.append(&kit::hero(
                "circle-alert",
                "wadd isn't answering",
                "Files shows your other machines once wadd, which knows them, answers.",
            ));
            String::new()
        } else {
            match &loc {
                Loc::Machines => self.machines_view(),
                Loc::Peer(p) => self.peer_view(p),
                Loc::Folder { peer, path } => self.folder_view(peer, path),
            }
        };
        self.status.set_label(&status);
        self.status.set_visible(!status.is_empty());
        self.window.set_title(Some(&self.title(&loc)));
    }

    fn title(&self, loc: &Loc) -> String {
        let st = self.state.borrow();
        let name = |p: &str| st.peer(p).map(|x| x.peer.name.clone()).unwrap_or_else(|| "Machines".into());
        match loc {
            Loc::Machines => "Machines".into(),
            Loc::Peer(p) => name(p),
            Loc::Folder { peer, path } => {
                format!("{} on {}", path.last().map(String::as_str).unwrap_or("Home"), name(peer))
            }
        }
    }

    fn draw_side(&self, loc: &Loc) {
        while let Some(c) = self.side.first_child() {
            self.side.remove(&c);
        }
        self.side.append(&kit::label("PLACES", &["files-h5"]));
        for (label, icon) in [
            ("Home", "house"),
            ("Desktop", "monitor"),
            ("Documents", "file-text"),
            ("Downloads", "download"),
            ("Pictures", "image"),
            ("Trash", "trash"),
        ] {
            let e = entry(icon, label);
            e.add_css_class("off");
            e.set_tooltip_text(Some("This machine's own files come to Files in phase 8"));
            self.side.append(&e);
        }
        self.side.append(&kit::label("MACHINES", &["files-h5"]));
        let st = self.state.borrow();
        let all = entry("server", "Machines");
        let asks = asks(&st).len();
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
                let e = entry(kind_icon(p.peer.kind), &p.peer.name);
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

    /// A click on `w` goes to `to`.
    fn click(&self, w: &impl IsA<gtk::Widget>, to: Loc) {
        let g = gtk::GestureClick::new();
        let me = self.me.clone();
        g.connect_released(move |g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(m) = me.upgrade() {
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
            state::crumbs(loc, |p| st.peer(p).map(|x| x.peer.name.clone()).unwrap_or_else(|| p.to_string()))
        };
        let n = list.len();
        for (i, c) in list.into_iter().enumerate() {
            if i > 0 {
                let ch = Icon::new("chevron-right", 13);
                ch.add_css_class("files-chev");
                ch.set_valign(gtk::Align::Center);
                self.crumbs.append(&ch);
            }
            let inner = gtk::Box::builder().spacing(6).build();
            if let Some(ic) = c.icon {
                inner.append(&Icon::new(ic, 14));
            }
            inner.append(&gtk::Label::new(Some(&c.label)));
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

    // ------------------------------------------------------------ Machines

    fn machines_view(&self) -> String {
        let st = self.state.borrow();
        let t = st.tailnet.clone().unwrap_or_default();
        if t.state != TailnetState::Running {
            let body = match t.state {
                TailnetState::NeedsLogin | TailnetState::Off => {
                    "Sign in to Tailscale and your other WadOS machines show up here, with their wadspaces and folders."
                }
                TailnetState::Stopped => {
                    "Tailscale is stopped on this machine. Turn it on to reach your other machines."
                }
                _ => "Tailscale is still connecting…",
            };
            let hero = kit::hero("network", "Join your tailnet to see your other machines", body);
            let b = kit::button("Tailscale settings", Variant::Primary, Some("network"), false);
            b.set_halign(gtk::Align::Center);
            b.connect_clicked(|_| open_accounts());
            hero.append(&b);
            self.body.append(&hero);
            return String::new();
        }
        for a in asks(&st).iter().rev() {
            self.body.append(&self.ask_row(a));
        }
        let q = self.q();
        let machines: Vec<_> = st
            .machines()
            .into_iter()
            .filter(|p| q.is_empty() || p.peer.name.to_lowercase().contains(&q) || p.dns_name.contains(&q))
            .cloned()
            .collect();
        if machines.is_empty() {
            let (title, body) = if q.is_empty() {
                (
                    "No other machines yet".to_string(),
                    "Your other WadOS machines on this tailnet, signed in to Tailscale as you, show up here."
                        .to_string(),
                )
            } else {
                ("No machines match".into(), format!("Nothing called “{}”.", self.filter.text()))
            };
            self.body.append(&kit::hero("server", &title, &body));
            return String::new();
        }
        let grid = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .min_children_per_line(1)
            .max_children_per_line(4)
            .column_spacing(14)
            .row_spacing(14)
            .homogeneous(true)
            .valign(gtk::Align::Start)
            .build();
        let now = now_ms();
        let selected = self.selected.borrow().clone();
        for p in &machines {
            let n = st.spaces.get(&p.peer.id).filter(|s| s.at > 0).map(|s| s.list.len());
            grid.append(&self.device(p, now, n, selected.as_deref() == Some(p.peer.id.as_str())));
        }
        self.body.append(&grid);
        format!("{} machine{}", machines.len(), if machines.len() == 1 { "" } else { "s" })
    }

    fn device(&self, p: &wad_proto::PeerInfo, now: i64, spaces: Option<usize>, selected: bool) -> gtk::Box {
        let d = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).css_classes(["device"]).build();
        d.set_size_request(230, -1);
        d.set_widget_name(&format!("peer-{}", p.peer.id));
        if !p.peer.online {
            d.add_css_class("offline");
        }
        if selected {
            d.add_css_class("selected");
        }
        let glyph = gtk::Overlay::builder().halign(gtk::Align::Start).build();
        let g = gtk::Box::builder().css_classes(["device-glyph"]).hexpand(false).build();
        let ic = Icon::new(kind_icon(p.peer.kind), 40);
        ic.set_hexpand(true);
        ic.set_halign(gtk::Align::Center);
        ic.set_valign(gtk::Align::Center);
        g.append(&ic);
        glyph.set_child(Some(&g));
        let dot =
            gtk::Box::builder().css_classes(["device-dot"]).halign(gtk::Align::End).valign(gtk::Align::End).build();
        dot.set_margin_bottom(8);
        if p.peer.online {
            dot.add_css_class("on");
        }
        glyph.add_overlay(&dot);
        d.append(&glyph);
        d.append(&kit::label(&p.peer.name, &["device-name"]));
        d.append(&kit::label(&p.dns_name, &["device-host"]));
        let s = kit::label(&state::status(p, now), &["device-status"]);
        if p.peer.online {
            s.add_css_class("on");
        }
        d.append(&s);
        let facts = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(3).margin_top(8).build();
        facts.append(&fact("hard-drive", &p.peer.os));
        if let Some(gpu) = &p.peer.gpu {
            facts.append(&fact("cpu", gpu));
        }
        if let Some(n) = spaces {
            facts.append(&fact("monitor", &state::count(n)));
        }
        d.append(&facts);
        // A click picks it, a double-click (or Enter) opens it, a right-click is its menu.
        let click = gtk::GestureClick::new();
        let (w, id) = (self.me.clone(), p.peer.id.clone());
        click.connect_released(move |g, n, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            let Some(m) = w.upgrade() else { return };
            if n >= 2 {
                m.go(Loc::Peer(id.clone()));
            } else {
                *m.selected.borrow_mut() = Some(id.clone());
                m.queue_draw();
            }
        });
        d.add_controller(click);
        let menu = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let (w, id, addr, name) = (self.me.clone(), p.peer.id.clone(), p.dns_name.clone(), p.peer.name.clone());
        let dw = d.downgrade();
        menu.connect_pressed(move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            let (Some(m), Some(d)) = (w.upgrade(), dw.upgrade()) else { return };
            *m.selected.borrow_mut() = Some(id.clone());
            m.machine_menu(&d, &id, &name, &addr, x, y);
        });
        d.add_controller(menu);
        d
    }

    /// A machine's menu (MachinesView's): Open, Copy address, Tailscale settings.
    fn machine_menu(&self, on: &gtk::Box, id: &str, name: &str, addr: &str, x: f64, y: f64) {
        let pop = gtk::Popover::builder().has_arrow(false).css_classes(["files-menu"]).build();
        let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
        col.append(&kit::label(&name.to_uppercase(), &["files-h5"]));
        let items: Vec<MenuRow> = vec![
            ("folder-open", "Open", None, {
                let (w, id) = (self.me.clone(), id.to_string());
                Box::new(move || {
                    if let Some(m) = w.upgrade() {
                        m.go(Loc::Peer(id.clone()));
                    }
                })
            }),
            ("copy", "Copy address", Some(addr.to_string()), {
                let addr = addr.to_string();
                Box::new(move || {
                    if let Some(d) = gdk::Display::default() {
                        d.clipboard().set_text(&addr);
                    }
                })
            }),
            ("network", "Tailscale settings", None, Box::new(open_accounts)),
        ];
        for (icon, label, hint, run) in items {
            let r = gtk::Box::builder().spacing(10).build();
            r.append(&Icon::new(icon, 16));
            let t = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
            t.append(&kit::label(label, &[]));
            if let Some(h) = hint {
                t.append(&kit::label(&h, &["item-sub"]));
            }
            r.append(&t);
            let b = gtk::Button::builder().child(&r).css_classes(["kit-btn", "ghost", "sm"]).build();
            let p = pop.downgrade();
            b.connect_clicked(move |_| {
                if let Some(p) = p.upgrade() {
                    p.popdown();
                }
                run();
            });
            col.append(&b);
        }
        pop.set_child(Some(&col));
        pop.set_parent(on);
        pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        pop.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        pop.popup();
    }

    fn ask_row(&self, a: &TailnetAsk) -> gtk::Box {
        let r = gtk::Box::builder().spacing(12).css_classes(["files-ask"]).build();
        r.set_widget_name(&format!("ask-{}", a.id));
        let badge =
            gtk::Box::builder().css_classes(["files-ask-badge"]).valign(gtk::Align::Center).hexpand(false).build();
        let ic = Icon::new(device_icon(a.kind), 18);
        ic.set_hexpand(true);
        ic.set_halign(gtk::Align::Center);
        ic.set_valign(gtk::Align::Center);
        badge.append(&ic);
        r.append(&badge);
        let what = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
        let t = gtk::Label::builder().xalign(0.0).wrap(true).build();
        t.set_markup(&format!("<b>{} wants to be trusted</b>", glib::markup_escape_text(&a.device)));
        what.append(&t);
        what.append(&kit::label(
            &format!(
                "{} · {} · asked {}. Trusted, it can open, stream and build your wadspaces.",
                kit::os_name(&a.os),
                a.ip,
                kit::ago(now_ms(), a.at)
            ),
            &["files-ask-sub"],
        ));
        r.append(&what);
        let deny = kit::button("Deny", Variant::Ghost, None, true);
        let trust = kit::button("Trust", Variant::Primary, Some("shield-check"), true);
        for (b, yes) in [(&deny, false), (&trust, true)] {
            b.set_valign(gtk::Align::Center);
            let (w, id) = (self.me.clone(), a.id.clone());
            b.connect_clicked(move |b| {
                b.set_sensitive(false);
                let Some(m) = w.upgrade() else { return };
                let (w, id) = (m.me.clone(), id.clone());
                m.link.call(
                    move |wadd| async move { if yes { wadd.trust_device(&id).await } else { wadd.deny_ask(&id).await } },
                    move |r| {
                        if let (Err(e), Some(m)) = (r, w.upgrade()) {
                            m.say(&e.message);
                            m.queue_draw();
                        }
                    },
                );
            });
            r.append(b);
        }
        r
    }

    // ------------------------------------------------------------ a machine

    fn peer_view(&self, peer: &str) -> String {
        let st = self.state.borrow();
        let Some(p) = st.peer(peer).cloned() else {
            self.body.append(&gone("That machine isn't on your tailnet any more."));
            return String::new();
        };
        if !st.running() {
            self.body.append(&gone("Join your tailnet to see your other machines."));
            return String::new();
        }
        if !p.peer.online {
            let away = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(8)
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .vexpand(true)
                .css_classes(["files-away"])
                .build();
            let ic = Icon::new("cloud-off", 30);
            ic.set_halign(gtk::Align::Center);
            away.append(&ic);
            let t = kit::label(&format!("{} is offline", p.peer.name), &["files-away-title"]);
            t.set_xalign(0.5);
            away.append(&t);
            let s = kit::label(&format!("Last seen {}", kit::ago(now_ms(), p.peer.last_seen)), &[]);
            s.set_xalign(0.5);
            away.append(&s);
            self.body.append(&away);
            return String::new();
        }
        let q = self.q();
        let selected = self.selected.borrow().clone();
        let mut items = 0;
        // Its Home, over Taildrive.
        if q.is_empty() || "home".contains(&q) {
            self.body.append(&kit::label("HOME", &["files-section"]));
            let home = item("folder", "Home", "Over Taildrive", selected.as_deref() == Some("home"));
            home.set_halign(gtk::Align::Start);
            home.set_widget_name("home");
            let (w, id) = (self.me.clone(), peer.to_string());
            let click = gtk::GestureClick::new();
            click.connect_released(move |g, n, _, _| {
                g.set_state(gtk::EventSequenceState::Claimed);
                let Some(m) = w.upgrade() else { return };
                if n >= 2 {
                    m.go(Loc::Folder { peer: id.clone(), path: vec![] });
                } else {
                    *m.selected.borrow_mut() = Some("home".into());
                    m.queue_draw();
                }
            });
            home.add_controller(click);
            self.body.append(&home);
            items += 1;
        }
        // Its wadspaces: Copy here.
        self.body.append(&kit::label("WADSPACES", &["files-section"]));
        let spaces = st.spaces.get(peer).cloned().unwrap_or_default();
        if spaces.at == 0 {
            let why = match &spaces.error {
                Some(e) => format!("{} didn’t answer: {e}", p.peer.name),
                None => format!("Asking {} for its wadspaces…", p.peer.name),
            };
            self.body.append(&kit::label(&why, &["files-away"]));
        } else {
            let list: Vec<_> =
                spaces.list.iter().filter(|x| q.is_empty() || x.name.to_lowercase().contains(&q)).collect();
            if list.is_empty() {
                self.body.append(&kit::label(&format!("No wadspaces on {}.", p.peer.name), &["files-away"]));
            }
            let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
            for x in list {
                col.append(&self.space_row(peer, x, st.copying(peer, &x.id)));
                items += 1;
            }
            self.body.append(&col);
        }
        format!("{items} item{}", if items == 1 { "" } else { "s" })
    }

    fn space_row(&self, peer: &str, x: &wad_model::Wadspace, copying: bool) -> gtk::Box {
        let r = gtk::Box::builder().spacing(12).css_classes(["space-row"]).build();
        r.set_widget_name(&format!("ws-{}", x.id));
        let tile = wad_gtk::WadspaceTile::new(0.8, false);
        tile.set(x, &wad_gtk::TileState::default());
        tile.root.set_valign(gtk::Align::Center);
        r.append(&tile.root);
        let t = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        t.append(&kit::label(&x.name, &["space-name"]));
        let apps: Vec<&str> = x.apps.iter().map(|a| wad_model::catalog::app_name(a.app)).collect();
        if !apps.is_empty() {
            t.append(&kit::label(&apps.join(", "), &["item-sub"]));
        }
        r.append(&t);
        let b = kit::button(if copying { "Copying…" } else { "Copy here" }, Variant::Secondary, Some("download"), true);
        b.set_tooltip_text(Some("Copy it to this machine"));
        b.set_sensitive(!copying);
        b.set_valign(gtk::Align::Center);
        let (w, peer, ws) = (self.me.clone(), peer.to_string(), x.id.clone());
        b.connect_clicked(move |b| {
            b.set_sensitive(false);
            let Some(m) = w.upgrade() else { return };
            let (w, p, s) = (m.me.clone(), peer.clone(), ws.clone());
            let (p2, s2) = (peer.clone(), ws.clone());
            m.link.call(
                move |wadd| async move { wadd.copy_from_peer(&p, &s).await },
                move |r| {
                    let Some(m) = w.upgrade() else { return };
                    match r {
                        Ok(job) => m.state.borrow_mut().started(job, &p2, &s2),
                        Err(e) => m.say(&e.message),
                    }
                    m.queue_draw();
                },
            );
        });
        r.append(&b);
        r
    }

    // ------------------------------------------------------------ a folder

    fn folder_view(&self, peer: &str, path: &[String]) -> String {
        let name = self.state.borrow().peer(peer).map(|p| p.peer.name.clone()).unwrap_or_else(|| "that machine".into());
        let got = self.listing.borrow().got.clone();
        match got {
            None => {
                let b = gtk::Box::builder()
                    .spacing(10)
                    .halign(gtk::Align::Center)
                    .valign(gtk::Align::Center)
                    .vexpand(true)
                    .build();
                let s = gtk::Spinner::builder().spinning(true).build();
                b.append(&s);
                b.append(&kit::label(&format!("Over Taildrive from {name}…"), &["files-away"]));
                self.body.append(&b);
                String::new()
            }
            Some(Err(why)) => {
                let here = path.last().map(String::as_str).unwrap_or("Home");
                self.body.append(&kit::hero("folder-open", &format!("{here} on {name} didn’t open"), &why));
                String::new()
            }
            Some(Ok(list)) => {
                let q = self.q();
                let list: Vec<Entry> =
                    list.into_iter().filter(|e| q.is_empty() || e.display.to_lowercase().contains(&q)).collect();
                if list.is_empty() {
                    let t = if q.is_empty() {
                        "This folder is empty.".to_string()
                    } else {
                        format!("Nothing here matches “{}”.", self.filter.text())
                    };
                    self.body.append(&kit::label(&t, &["files-away"]));
                    return String::new();
                }
                let grid = gtk::FlowBox::builder()
                    .selection_mode(gtk::SelectionMode::None)
                    .min_children_per_line(2)
                    .max_children_per_line(12)
                    .column_spacing(6)
                    .row_spacing(6)
                    .homogeneous(true)
                    .valign(gtk::Align::Start)
                    .build();
                let selected = self.selected.borrow().clone();
                for e in &list {
                    let sub = if e.folder { String::new() } else { drive::bytes(e.size) };
                    let it = item(e.icon(), &e.display, &sub, selected.as_deref() == Some(e.name.as_str()));
                    it.set_widget_name(&format!("entry-{}", e.name));
                    let click = gtk::GestureClick::new();
                    let (w, en) = (self.me.clone(), e.clone());
                    click.connect_released(move |g, n, _, _| {
                        g.set_state(gtk::EventSequenceState::Claimed);
                        let Some(m) = w.upgrade() else { return };
                        if n >= 2 {
                            m.open_entry(&en);
                        } else {
                            *m.selected.borrow_mut() = Some(en.name.clone());
                            m.queue_draw();
                        }
                    });
                    it.add_controller(click);
                    grid.append(&it);
                }
                self.body.append(&grid);
                format!("{} item{}", list.len(), if list.len() == 1 { "" } else { "s" })
            }
        }
    }

    /// Whether what's open has all it'll show (for `--dump`).
    pub fn settled(&self) -> bool {
        let st = self.state.borrow();
        if !st.connected || st.tailnet.is_none() {
            return false;
        }
        match &*self.loc.borrow() {
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

fn asks(st: &State) -> Vec<TailnetAsk> {
    st.tailnet.as_ref().filter(|t| t.state == TailnetState::Running).map(|t| t.asks.clone()).unwrap_or_default()
}

fn entry(icon: &'static str, label: &str) -> gtk::Box {
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

fn fact(icon: &'static str, text: &str) -> gtk::Box {
    let b = gtk::Box::builder().spacing(7).css_classes(["device-fact"]).build();
    let ic = Icon::new(icon, 13);
    ic.set_valign(gtk::Align::Center);
    b.append(&ic);
    b.append(&kit::label(text, &[]));
    b
}

/// A thing in a folder: its glyph, its name, what's under it.
fn item(icon: &'static str, name: &str, sub: &str, selected: bool) -> gtk::Box {
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).css_classes(["item"]).build();
    b.set_size_request(112, -1);
    if selected {
        b.add_css_class("selected");
    }
    let ic = Icon::new(icon, 44);
    ic.add_css_class("item-glyph");
    if icon != "folder" {
        ic.add_css_class("file");
    }
    ic.set_halign(gtk::Align::Center);
    b.append(&ic);
    let n = gtk::Label::builder()
        .label(name)
        .justify(gtk::Justification::Center)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .lines(2)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(14)
        .css_classes(["item-name"])
        .build();
    b.append(&n);
    if !sub.is_empty() {
        b.append(&gtk::Label::builder().label(sub).css_classes(["item-sub"]).build());
    }
    b
}

fn gone(what: &str) -> gtk::Box {
    kit::hero("network", what, "")
}

fn kind_icon(k: PeerKind) -> &'static str {
    match k {
        PeerKind::Laptop => "laptop",
        PeerKind::Server => "server",
        PeerKind::Desktop => "monitor",
    }
}

fn device_icon(k: wad_proto::DeviceKind) -> &'static str {
    use wad_proto::DeviceKind;
    match k {
        DeviceKind::Phone | DeviceKind::Other => "smartphone",
        DeviceKind::Tablet => "tablet",
        DeviceKind::Laptop => "laptop",
        DeviceKind::Desktop => "monitor",
        DeviceKind::Server => "server",
    }
}

/// Settings ▸ Accounts (its window, if it's open).
fn open_accounts() {
    let args = ["wados-settings", "--page", "accounts"].map(std::ffi::OsStr::new);
    if let Err(e) = gtk::gio::Subprocess::newv(&args, gtk::gio::SubprocessFlags::NONE) {
        tracing::warn!("wados-settings: {e}");
    }
}

pub fn now_ms() -> i64 {
    glib::real_time() / 1000
}
