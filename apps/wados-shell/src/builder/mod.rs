//! The Builder (`apps/builder/Builder.svelte`, `sim/actions/builderWindow.ts`):
//! a window in the middle of the screen (labwc's window rule,
//! `host/etc/xdg/labwc/rc.xml`), its own app id
//! (`io.wados.Builder`, so the bar groups it). The wadspaces opened in it
//! are **stacks**, left to right ([`board`]); the **sidebar** edits the
//! selected one, in four tabs ([`side`]). Its bar: New, then the selected
//! one's name, Build, Delete, In a window and Open (Show, when it runs).
//!
//! Its state is the stacks open, the one selected and the sidebar's tab.
//! A wadspace opened to edit (its desktop menu's Edit in the Builder) brings
//! its group (the desktop's
//! loose ones, or its tag folder's), it selected; one made in it (New, or
//! from a piece of another) gets a stack of its own, beside where it came
//! from. Closing the window forgets them; the wadspaces stay.
//!
//! Everything it changes is an edit (`wad_model::edit::Op`) sent to wadd,
//! whose events come back through the store: the board and the sidebar
//! are redrawn from the library as it is then (once per burst).

pub mod board;
pub mod drops;
pub mod side;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::{glib, prelude::*};
use wad_model::edit::{Op, Payload};
use wad_model::{Built, SessionDisplay, Wadspace};
use wad_proto::Level;

use crate::shell::Shell;
use crate::store::Change;

/// Its app id: labwc's window rule places it, and the bar groups it.
pub const APP_ID: &str = "io.wados.Builder";

const CSS: &str = include_str!("builder.css");

/// The sidebar's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Apps,
    Addons,
    Secrets,
    Customize,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Apps, Tab::Addons, Tab::Secrets, Tab::Customize];

    pub fn id(self) -> &'static str {
        match self {
            Tab::Apps => "apps",
            Tab::Addons => "addons",
            Tab::Secrets => "secrets",
            Tab::Customize => "customize",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == s || (s == "customizations" && *t == Tab::Customize))
    }
}

#[derive(Default)]
struct State {
    stacks: Vec<String>,
    selected: Option<String>,
    tab: Tab,
    /// Stacks of wadspaces just made, which the store hasn't heard of yet
    /// (wadd answers an edit before its event comes): kept till they come.
    pending: Vec<String>,
}

/// The window and what's in it, while it's open.
struct Win {
    window: gtk::ApplicationWindow,
    tools: gtk::Box,
    which: gtk::Label,
    build: gtk::Button,
    trash: gtk::Button,
    in_window: gtk::Button,
    open: gtk::Button,
    open_label: gtk::Label,
    new: gtk::Button,
    board: Rc<board::Board>,
    side: Rc<side::Sidebar>,
}

pub struct Builder {
    pub shell: Rc<Shell>,
    me: Weak<Builder>,
    st: RefCell<State>,
    win: RefCell<Option<Win>>,
    /// The title's to be ready to type when Customize shows (a new one).
    title_focus: Cell<bool>,
    refresh_queued: Cell<bool>,
}

impl Builder {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        if let Some(d) = gtk::gdk::Display::default() {
            wad_gtk::theme::add_css(&d, CSS);
            wad_gtk::theme::add_css(&d, crate::dnd::CSS);
        }
        let me = Rc::new_cyclic(|me| Self {
            shell: shell.clone(),
            me: me.clone(),
            st: RefCell::default(),
            win: RefCell::default(),
            title_focus: Cell::new(false),
            refresh_queued: Cell::new(false),
        });
        let w = me.me.clone();
        shell.store.watch(move |c| {
            if let Some(b) = w.upgrade()
                && matches!(
                    c,
                    Change::Library
                        | Change::Wadspace(_)
                        | Change::Build(_)
                        | Change::Run(_)
                        | Change::Job(_)
                        | Change::Machine
                        | Change::Peers
                        | Change::PeerSpaces(_)
                )
            {
                b.queue_refresh();
            }
        });
        me.ctl();
        me
    }

    fn rc(&self) -> Rc<Self> {
        self.me.upgrade().expect("the Builder is alive while it's used")
    }

    // ------------------------------------------------------------ its state

    /// The wadspaces open in it, left to right.
    pub fn stacks(&self) -> Vec<String> {
        self.st.borrow().stacks.clone()
    }

    /// The one the sidebar edits.
    pub fn selected(&self) -> Option<String> {
        self.st.borrow().selected.clone()
    }

    pub fn tab(&self) -> Tab {
        self.st.borrow().tab
    }

    pub fn is_open(&self) -> bool {
        self.win.borrow().is_some()
    }

    /// A wadspace as it is now (none: gone, or in the Trash): this
    /// machine's by its id, or one of my other machines' as it last sent
    /// them, by its key (`<peer>:<id>`: never taken for one of this machine's).
    pub fn wadspace(&self, id: &str) -> Option<Wadspace> {
        self.shell.store.any_wadspace(id).filter(|x| !x.is_trashed())
    }

    /// The selected one, as it is now.
    pub fn selected_ws(&self) -> Option<Wadspace> {
        self.selected().and_then(|id| self.wadspace(&id))
    }

    /// It can be changed here: it lives on this machine (another's shows,
    /// with Copy here).
    pub fn editable(&self, x: &Wadspace) -> bool {
        x.machine == self.shell.store.machine_id()
    }

    /// The machine it lives on, by name ("Another machine" when it isn't known).
    pub fn machine_name(&self, x: &Wadspace) -> String {
        self.shell.store.machine_name(&x.machine).unwrap_or_else(|| "Another machine".into())
    }

    /// Its machine's glyph.
    pub fn machine_icon(&self, x: &Wadspace) -> &'static str {
        self.shell.store.peer(&x.machine).map(|p| crate::peers::kind_icon(p.peer.kind)).unwrap_or("monitor")
    }

    /// Copy here: another machine's wadspace (by its key, `<peer>:<id>`),
    /// copied to this one from there.
    pub fn copy_here(&self, ws: &str) {
        match self.shell.store.peer_wadspace(ws) {
            Some(_) => crate::peers::copy_here(&self.shell, ws),
            None => {
                let name = self.wadspace(ws).map(|x| self.machine_name(&x)).unwrap_or_else(|| "It".into());
                self.shell.toast(Level::Info, &format!("{name} isn't online: a copy comes from there"), None);
            }
        }
    }

    /// The title's to be ready to type (once): the Customize tab asks when it shows.
    pub fn take_title_focus(&self) -> bool {
        self.title_focus.replace(false)
    }

    // ------------------------------------------------------------ verbs

    /// `ws` in it: its stack (at the end, or just right of `after`'s),
    /// selected; the window opened or brought up.
    pub fn open(&self, ws: &str, after: Option<&str>) {
        {
            let mut s = self.st.borrow_mut();
            if self.shell.store.any_wadspace(ws).is_none() && !s.pending.iter().any(|x| x == ws) {
                s.pending.push(ws.to_string());
            }
            if !s.stacks.iter().any(|x| x == ws) {
                match after.and_then(|a| s.stacks.iter().position(|x| x == a)) {
                    Some(i) => s.stacks.insert(i + 1, ws.to_string()),
                    None => s.stacks.push(ws.to_string()),
                }
            }
            s.selected = Some(ws.to_string());
        }
        self.present();
        self.refill();
        self.scroll_to(ws);
    }

    /// A group in it (the desktop's loose wadspaces, or a tag folder's):
    /// their stacks, just them, in order, `selected` selected.
    pub fn open_group(&self, ids: &[String], selected: &str) {
        {
            let mut s = self.st.borrow_mut();
            let mut stacks: Vec<String> = ids.iter().filter(|id| self.wadspace(id).is_some()).cloned().collect();
            if !stacks.iter().any(|x| x == selected) {
                stacks.push(selected.to_string());
            }
            s.stacks = stacks;
            s.selected = Some(selected.to_string());
        }
        self.present();
        self.refill();
        self.scroll_to(selected);
    }

    /// The stack the sidebar edits.
    pub fn select(&self, ws: &str) {
        let changed = {
            let mut s = self.st.borrow_mut();
            if !s.stacks.iter().any(|x| x == ws) || s.selected.as_deref() == Some(ws) {
                false
            } else {
                s.selected = Some(ws.to_string());
                true
            }
        };
        if changed {
            self.refill();
        }
    }

    /// A stack put away (the wadspace stays); the one beside it is selected.
    pub fn close_stack(&self, ws: &str) {
        {
            let mut s = self.st.borrow_mut();
            let Some(i) = s.stacks.iter().position(|x| x == ws) else { return };
            s.stacks.remove(i);
            if s.selected.as_deref() == Some(ws) {
                s.selected = s.stacks.get(i.min(s.stacks.len().saturating_sub(1))).cloned();
            }
        }
        self.refill();
    }

    /// The sidebar's tab.
    pub fn set_tab(&self, tab: Tab) {
        let was = std::mem::replace(&mut self.st.borrow_mut().tab, tab);
        if was != tab {
            self.refill();
        }
    }

    /// New: a wadspace (empty glass on the desktop), its stack here,
    /// selected, on Customize with its title ready to type.
    pub fn new_wadspace(&self) {
        let me = self.rc();
        self.shell.op(Op::Create { name: None, category: None, machine: None }, move |r| {
            let Some(id) = r.created else { return };
            me.title_focus.set(true);
            me.st.borrow_mut().tab = Tab::Customize;
            me.open(&id, None);
        });
    }

    /// A piece of a stack let go on open desktop or the slot: a new
    /// wadspace of just that, Untitled, its stack beside where it came from.
    pub fn fresh(&self, payload: Payload) {
        self.fresh_then(payload, |_| {});
    }

    /// The same, then `then` with the new one's id (more put into it).
    pub fn fresh_then(&self, payload: Payload, then: impl FnOnce(&str) + 'static) {
        let me = self.rc();
        let from = match &payload {
            Payload::Section { ws, .. } | Payload::Folder { ws, .. } => Some(ws.clone()),
            Payload::Piece { from, .. } => from.clone(),
        };
        self.shell.op(Op::Fresh { payload }, move |r| {
            if let Some(id) = r.created {
                then(&id);
                me.open(&id, from.as_deref());
            }
        });
    }

    /// A whole stack let go on open desktop or the slot: a copy, its stack beside it.
    pub fn copy(&self, ws: &str) {
        let me = self.rc();
        let from = ws.to_string();
        self.shell.op(Op::Duplicate { ws: ws.to_string(), machine: None }, move |r| {
            if let Some(id) = r.created {
                me.open(&id, Some(&from));
            }
        });
    }

    /// The board scrolled so `ws`'s stack shows.
    pub fn scroll_to(&self, ws: &str) {
        if let Some(w) = self.win.borrow().as_ref() {
            w.board.scroll_to(ws);
        }
    }

    // ------------------------------------------------------------ the window

    /// Opened, or brought up.
    pub fn present(&self) {
        if let Some(w) = self.win.borrow().as_ref() {
            w.window.present();
            return;
        }
        let win = self.make();
        win.window.present();
        *self.win.borrow_mut() = Some(win);
    }

    /// Closed: the stacks are forgotten (the wadspaces stay).
    pub fn close(&self) {
        let win = self.win.borrow_mut().take();
        *self.st.borrow_mut() = State::default();
        if let Some(w) = win {
            w.window.destroy();
        }
    }

    fn make(&self) -> Win {
        let window = gtk::ApplicationWindow::builder()
            .application(&self.shell.app)
            .title("Builder")
            .default_width(1100)
            .default_height(780)
            .css_classes(["builder"])
            .build();
        // Its own app id (one process, many windows). GtkApplication sets the
        // application's on each window as it's realized and mapped; this goes
        // after, before its first frame, so labwc's rule sees it on map.
        let own_id = |w: &gtk::ApplicationWindow| match w.surface().and_downcast::<gdk4_wayland::WaylandToplevel>() {
            Some(t) => t.set_application_id(APP_ID),
            None => tracing::warn!("the Builder's window isn't a Wayland toplevel: it keeps the shell's app id"),
        };
        window.connect_realize(own_id);
        window.connect_map(own_id);
        let w = self.me.clone();
        window.connect_close_request(move |_| {
            if let Some(b) = w.upgrade() {
                // After this handler: close() destroys the window.
                glib::idle_add_local_once(move || b.close());
            }
            glib::Propagation::Stop
        });

        // Its title bar: the Builder's mark and name, and the window's buttons.
        let head = gtk::HeaderBar::builder().css_classes(["builder-head"]).build();
        head.set_decoration_layout(Some(":minimize,maximize,close"));
        let title = gtk::Box::builder().spacing(8).build();
        title.append(&wad_gtk::Icon::new("blocks", 16));
        title.append(&gtk::Label::builder().label("Builder").css_classes(["builder-title"]).build());
        head.pack_start(&title);
        head.set_title_widget(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
        window.set_titlebar(Some(&head));

        // Its bar.
        let bar = gtk::Box::builder().spacing(6).css_classes(["builder-bar"]).build();
        let new = tool("plus", Some("New"), "A new wadspace");
        bar.append(&new);
        bar.append(&gtk::Box::builder().hexpand(true).build());
        let tools = gtk::Box::builder().spacing(6).build();
        let which = gtk::Label::builder()
            .css_classes(["builder-which"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(22)
            .build();
        tools.append(&which);
        let build = tool("hammer", None, "Build it now (or it builds when it opens)");
        let trash = tool("trash", None, "Move it to the Trash");
        trash.add_css_class("danger");
        tools.append(&build);
        tools.append(&trash);
        tools.append(
            &gtk::Separator::builder().orientation(gtk::Orientation::Vertical).css_classes(["builder-rule"]).build(),
        );
        let in_window = tool("app-window", Some("In a window"), "Open it in a window");
        tools.append(&in_window);
        let (open, open_label) = {
            let b = gtk::Button::builder().css_classes(["builder-tool", "go"]).build();
            let row = gtk::Box::builder().spacing(6).build();
            row.append(&wad_gtk::Icon::new("play", 15));
            let l = gtk::Label::new(Some("Open"));
            row.append(&l);
            b.set_child(Some(&row));
            (b, l)
        };
        tools.append(&open);
        bar.append(&tools);

        let me = self.rc();
        let board = board::Board::new(&me);
        let side = side::Sidebar::new(&me);
        // Three columns as wide as each other: the board two (two stacks side
        // by side, more scrolling), the sidebar one.
        let body = gtk::Grid::builder().column_homogeneous(true).css_classes(["builder-body"]).vexpand(true).build();
        let board_w = board.widget();
        board_w.set_hexpand(true);
        board_w.set_vexpand(true);
        body.attach(&board_w, 0, 0, 2, 1);
        body.attach(&side.widget(), 2, 0, 1, 1);

        let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        col.append(&bar);
        col.append(&body);
        window.set_child(Some(&col));

        self.wire(&new, |b| b.new_wadspace());
        self.wire(&build, |b| {
            if let Some(id) = b.selected() {
                b.shell.build(&id);
            }
        });
        self.wire(&trash, |b| {
            if let Some(id) = b.selected() {
                // wadd's notice says so, with its Undo.
                b.shell.op(Op::Trash { ws: id }, |_| {});
            }
        });
        self.wire(&in_window, |b| {
            if let Some(id) = b.selected() {
                b.shell.open(&id, SessionDisplay::Window);
            }
        });
        self.wire(&open, |b| {
            if let Some(id) = b.selected() {
                if b.shell.store.is_running(&id) {
                    b.shell.show(&id);
                } else {
                    b.shell.open(&id, SessionDisplay::Screen);
                }
            }
        });
        Win { window, tools, which, build, trash, in_window, open, open_label, new, board, side }
    }

    fn wire(&self, b: &gtk::Button, f: impl Fn(&Rc<Self>) + 'static) {
        let w = self.me.clone();
        b.connect_clicked(move |_| {
            if let Some(b) = w.upgrade() {
                f(&b);
            }
        });
    }

    // ------------------------------------------------------------ redrawing

    fn queue_refresh(&self) {
        if !self.is_open() || self.refresh_queued.replace(true) {
            return;
        }
        let w = self.me.clone();
        glib::idle_add_local_once(move || {
            if let Some(b) = w.upgrade() {
                b.refresh_queued.set(false);
                b.refill();
            }
        });
    }

    /// Everything shown as it is now: gone or trashed wadspaces' stacks put away.
    fn refill(&self) {
        {
            let mut s = self.st.borrow_mut();
            let store = &self.shell.store;
            s.pending.retain(|id| store.any_wadspace(id).is_none());
            let pending = s.pending.clone();
            let live: Vec<String> =
                s.stacks.iter().filter(|id| self.wadspace(id).is_some() || pending.contains(id)).cloned().collect();
            if live.len() != s.stacks.len() {
                let gone_selected = s.selected.as_ref().is_some_and(|x| !live.contains(x));
                let at = s.selected.as_ref().and_then(|x| s.stacks.iter().position(|y| y == x)).unwrap_or(0);
                s.stacks = live;
                if gone_selected {
                    s.selected = s.stacks.get(at.min(s.stacks.len().saturating_sub(1))).cloned();
                }
            }
            if s.selected.is_none() {
                s.selected = s.stacks.last().cloned();
            }
        }
        let win = self.win.borrow();
        let Some(w) = win.as_ref() else { return };
        let x = self.selected_ws();
        w.tools.set_visible(x.is_some());
        if let Some(x) = &x {
            let editable = self.editable(x);
            let store = &self.shell.store;
            w.which.set_label(&x.name);
            w.which.set_tooltip_text(Some(&x.name));
            let built = store.build(&x.id).is_some_and(|b| b.state.built == Built::Built && b.job.is_none());
            w.build.set_sensitive(editable && !built);
            w.build.set_tooltip_text(Some(if built {
                "It's built and up to date"
            } else {
                "Build it now (or it builds when it opens)"
            }));
            w.trash.set_sensitive(editable);
            w.open_label.set_label(if store.is_running(&x.id) { "Show" } else { "Open" });
            // Another machine's would stream from there (phase 7): copy it here to open it.
            let away = (!editable).then(|| format!("It lives on {}: copy it here to open it", self.machine_name(x)));
            for (b, tip) in [(&w.open, None), (&w.in_window, Some("Open it in a window"))] {
                b.set_sensitive(editable);
                b.set_tooltip_text(away.as_deref().or(tip));
            }
        }
        w.new.set_sensitive(self.shell.store.connected());
        w.board.refresh();
        w.side.refresh();
    }

    // ------------------------------------------------------------ wadsctl

    /// `wadsctl builder …` (the stacks' and the sidebar's own are theirs).
    fn ctl(&self) {
        let w = self.me.clone();
        self.shell.on_ctl("builder", move |args| {
            let Some(b) = w.upgrade() else { return "error gone".into() };
            match args {
                [] => {
                    let s = b.st.borrow();
                    format!(
                        "ok open={} stacks={} selected={} tab={}",
                        b.is_open(),
                        s.stacks.join(","),
                        s.selected.as_deref().unwrap_or("-"),
                        s.tab.id()
                    )
                }
                ["show"] => {
                    b.present();
                    b.refill();
                    "ok".into()
                }
                ["close"] => {
                    b.close();
                    "ok".into()
                }
                ["open", id] | ["open", id, "after", _] if b.wadspace(id).is_none() => format!("error no wadspace {id}"),
                ["open", id] => {
                    b.open(id, None);
                    "ok".into()
                }
                ["open", id, "after", after] => {
                    b.open(id, Some(after));
                    "ok".into()
                }
                ["group", sel, ids @ ..] => {
                    let ids: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
                    b.open_group(&ids, sel);
                    "ok".into()
                }
                ["edit", id] => {
                    b.shell.edit(id);
                    "ok".into()
                }
                ["new"] => {
                    b.new_wadspace();
                    "ok".into()
                }
                ["select", id] => {
                    b.select(id);
                    "ok".into()
                }
                ["unstack", id] => {
                    b.close_stack(id);
                    "ok".into()
                }
                ["tab", t] => match Tab::parse(t) {
                    Some(t) => {
                        b.set_tab(t);
                        "ok".into()
                    }
                    None => "error tab apps|addons|secrets|customize".into(),
                },
                ["click", what] => {
                    let win = b.win.borrow();
                    let Some(w) = win.as_ref() else { return "error it isn't open".into() };
                    let button = match *what {
                        "new" => &w.new,
                        "build" => &w.build,
                        "trash" => &w.trash,
                        "window" => &w.in_window,
                        "open" => &w.open,
                        _ => return "error click new|build|trash|window|open".into(),
                    };
                    if !button.is_sensitive() {
                        return format!("error {what} is off");
                    }
                    let button = button.clone();
                    drop(win);
                    button.emit_clicked();
                    "ok".into()
                }
                _ => "error builder [show|close|open <id> [after <id>]|group <selected> <ids…>|edit <id>|new|select <id>|unstack <id>|tab <tab>|click <button>]".into(),
            }
        });
    }

    /// Says something (a toast).
    pub fn say(&self, level: Level, text: &str) {
        self.shell.toast(level, text, None);
    }
}

/// A button on its bar: an icon, maybe a label, and what it does in its tooltip.
fn tool(icon: &'static str, label: Option<&str>, tip: &str) -> gtk::Button {
    let b = gtk::Button::builder().css_classes(["builder-tool"]).tooltip_text(tip).build();
    let row = gtk::Box::builder().spacing(6).halign(gtk::Align::Center).build();
    row.append(&wad_gtk::Icon::new(icon, 16));
    if let Some(l) = label {
        row.append(&gtk::Label::new(Some(l)));
    } else {
        b.add_css_class("icon");
    }
    b.set_child(Some(&row));
    b
}
