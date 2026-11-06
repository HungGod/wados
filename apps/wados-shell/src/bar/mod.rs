//! The bar (`shell/bar/*`): on the launcher a dock at the bottom; on the
//! desktop two halves, the logo's at the top left and the tray's at the top
//! right, with what's running and the machine's apps between them.
//!
//! The interface machine.rs uses:
//! - `Halves::new(shell, on_desktop)`: the two halves (built twice: in the
//!   launcher's surface, where they fly between their two places, and in
//!   the bar's, where they stay); `left`, `right`, `set_on_desktop`, and
//!   `on_resize` (a half changed size: lay them out again).
//! - `Middle::new(shell)`: what's between the halves on the desktop (sized
//!   by machine.rs to the room between them); `widget`.
//!
//! And overlays.rs: `Displays::new(shell)`, every other display's own bar.
//!
//! - **The logo's half** (`Panel.svelte`): the logo (its menu: `start.rs`),
//!   the way between the pages, search (Alt+Space's), and on the dock a
//!   button per window.
//! - **The tray's half** (`tray.rs`).
//! - **Between them** (`running.rs`, `apps.rs`): the running wadspaces, then
//!   Settings, Files and WadBrowser, then the machine's other windows.
//!
//! Both instances of the halves are fed the same, so they're the same size
//! at the same place and the hand-off between the surfaces is seamless.

mod apps;
mod display;
mod levels;
mod notices;
mod running;
pub mod shared;
mod start;
mod style;
mod system;
mod tasks;
mod tray;
mod ui;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::{glib, prelude::*};
use wad_gtk::{Icon, Svg};

pub use display::Displays;
pub use notices::{level_class, level_icon};
pub use system::Volume;
pub use tasks::in_front;

use crate::machine::Page;
use crate::shell::Shell;
use crate::wm::{Cmd, Top, Which};
use shared::Topic;

static LOGO: &[u8] = include_bytes!("../../../../assets/brand/wadspaces-logo-dark.svg");

pub struct Halves {
    shell: Rc<Shell>,
    left: gtk::Box,
    right: gtk::Box,
    start: Rc<start::Start>,
    page_button: gtk::Button,
    page_icon: Icon,
    sep: gtk::Box,
    tasks: gtk::Box,
    tasks_sig: RefCell<Vec<String>>,
    tray: Rc<tray::Tray>,
    on_desktop: Cell<bool>,
    resize: RefCell<Vec<Rc<dyn Fn()>>>,
    resize_queued: Rc<Cell<bool>>,
}

impl Halves {
    pub fn new(shell: &Rc<Shell>, on_desktop: bool) -> Rc<Self> {
        style::install();
        let shared = shared::get(shell);
        let left = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(2)
            .css_classes(["wb-half", "wb-left"])
            .build();
        let logo = Svg::trimmed(LOGO, 109, 42);
        let start_button = gtk::Button::builder()
            .child(&logo)
            .css_classes(["wb-start"])
            .tooltip_text("WadSpaces: your wadspaces, this machine's apps, and power")
            .valign(gtk::Align::Center)
            .build();
        left.append(&start_button);
        let (page_button, page_icon) = ui::tb("arrow-down", 17, "The desktop (scroll down)");
        page_button.set_valign(gtk::Align::Center);
        let s = shell.clone();
        page_button
            .connect_clicked(move |_| s.go(if s.page() == Page::Launcher { Page::Desktop } else { Page::Launcher }));
        left.append(&page_button);
        let (search, _) = ui::tb("search", 17, "Search apps and their variants (Alt+Space)");
        search.set_valign(gtk::Align::Center);
        let s = shell.clone();
        search.connect_clicked(move |_| {
            let _ = s.ctl(&["search"]);
        });
        left.append(&search);
        let sep = gtk::Box::builder().css_classes(["wb-sep"]).valign(gtk::Align::Center).visible(false).build();
        left.append(&sep);
        let tasks = gtk::Box::builder().spacing(2).valign(gtk::Align::Center).visible(false).build();
        left.append(&tasks);

        let right =
            gtk::Box::builder().orientation(gtk::Orientation::Horizontal).css_classes(["wb-half", "wb-right"]).build();
        let tray = tray::Tray::new(shell, &shared, on_desktop);
        right.append(&tray.root);

        let start = start::Start::new(shell, &start_button, on_desktop);
        let me = Rc::new(Self {
            shell: shell.clone(),
            left,
            right,
            start,
            page_button,
            page_icon,
            sep,
            tasks,
            tasks_sig: RefCell::default(),
            tray,
            on_desktop: Cell::new(!on_desktop),
            resize: RefCell::default(),
            resize_queued: Rc::default(),
        });
        shared.add_halves(on_desktop, &me);
        let w = Rc::downgrade(&me);
        me.tray.on_resize(Rc::new(move || {
            if let Some(m) = w.upgrade() {
                m.resized();
            }
        }));
        let w = Rc::downgrade(&me);
        shared.on(Topic::Windows, move || {
            if let Some(m) = w.upgrade() {
                m.fill_tasks();
            }
        });
        let w = Rc::downgrade(&me);
        shell.store.watch(move |_| {
            if let Some(m) = w.upgrade() {
                m.fill_tasks();
            }
        });
        me.set_on_desktop(on_desktop);
        // Measured before their styles were there: again once they're on the screen.
        for h in [&me.left, &me.right] {
            let w = Rc::downgrade(&me);
            h.connect_map(move |_| {
                if let Some(m) = w.upgrade() {
                    m.resized();
                }
            });
        }
        me
    }

    pub fn left(&self) -> gtk::Widget {
        self.left.clone().upcast()
    }

    pub fn right(&self) -> gtk::Widget {
        self.right.clone().upcast()
    }

    /// `f` when a half changes size (machine.rs lays them out again).
    pub fn on_resize(&self, f: impl Fn() + 'static) {
        self.resize.borrow_mut().push(Rc::new(f));
    }

    /// Tells machine.rs, once a turn of the main loop.
    fn resized(&self) {
        if self.resize_queued.replace(true) {
            return;
        }
        let fs: Vec<Rc<dyn Fn()>> = self.resize.borrow().clone();
        let queued = self.resize_queued.clone();
        glib::idle_add_local_once(move || {
            queued.set(false);
            for f in fs {
                f();
            }
        });
    }

    /// The dock on the launcher (false) or the desktop's corners (true): the
    /// page arrow's way, the flyouts' side, the dock's tasks.
    pub fn set_on_desktop(&self, on: bool) {
        if self.on_desktop.replace(on) == on {
            return;
        }
        self.page_icon.set(if on { "arrow-up" } else { "arrow-down" });
        self.page_button.set_tooltip_text(Some(if on {
            "The launcher (scroll up)"
        } else {
            "The desktop (scroll down)"
        }));
        for h in [&self.left, &self.right] {
            ui::set_class(h, "wb-dock", !on);
        }
        self.tray.set_below(on);
        self.start.set_below(on);
        self.fill_tasks();
        // Its new look is another size (the dock's halves meet without a border).
        self.resized();
    }

    pub fn open(&self, name: &str) -> bool {
        if name == "start" {
            self.tray.close_all();
            self.start.open();
            return true;
        }
        self.start.close();
        self.tray.open(name)
    }

    pub fn close_all(&self) {
        self.tray.close_all();
        self.start.close();
    }

    /// On the dock: a button per window (the desktop's bar has them between the halves).
    fn fill_tasks(&self) {
        let dock = !self.on_desktop.get();
        let windows = if dock { self.shell.placer.windows() } else { vec![] };
        let sig: Vec<String> = windows
            .iter()
            .map(|t| format!("{}:{}:{}:{}:{}", t.id, t.app_id, t.title, in_front(t), t.minimized))
            .collect();
        if *self.tasks_sig.borrow() == sig {
            return;
        }
        let was = (self.tasks.is_visible(), self.tasks_sig.borrow().len());
        *self.tasks_sig.borrow_mut() = sig;
        while let Some(c) = self.tasks.first_child() {
            self.tasks.remove(&c);
        }
        for t in &windows {
            self.tasks.append(&self.task(t));
        }
        let show = dock && !windows.is_empty();
        self.sep.set_visible(show);
        self.tasks.set_visible(show);
        if was != (show, windows.len()) {
            self.resized();
        }
    }

    fn task(&self, t: &Top) -> gtk::Button {
        let over = gtk::Overlay::new();
        over.set_size_request(42, 42);
        let icon: gtk::Widget = match t.wadspace().and_then(|ws| self.shell.store.wadspace(ws)) {
            Some(ws) => {
                let tile = wad_gtk::WadspaceTile::new(0.42, false);
                tile.set(&ws, &self.shell.store.tile_state(&ws.id));
                tile.root.upcast()
            }
            None => Icon::new(tasks::glyph(&t.app_id), 20).upcast(),
        };
        icon.set_halign(gtk::Align::Center);
        icon.set_valign(gtk::Align::Center);
        over.set_child(Some(&icon));
        let line = gtk::Box::builder()
            .css_classes(["wb-task-line"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::End)
            .build();
        over.add_overlay(&line);
        let name = match t.wadspace().and_then(|ws| self.shell.store.wadspace(ws)) {
            Some(ws) => ws.name,
            None => tasks::title(t),
        };
        let b = gtk::Button::builder().child(&over).css_classes(["wb-task"]).tooltip_text(&name).build();
        ui::set_class(&b, "wb-active", in_front(t));
        ui::set_class(&b, "wb-dim", t.minimized);
        let (s, t) = (self.shell.clone(), t.clone());
        b.connect_clicked(move |_| {
            s.go(Page::Desktop);
            match t.wadspace() {
                Some(ws) => s.show(ws),
                None => wm(&s, Cmd::Activate(Which(format!("#{}", t.id)))),
            }
        });
        b
    }
}

/// The volume as the keys left it (session.rs): the tray shows it.
pub fn set_volume(shell: &Rc<Shell>, v: Option<Volume>) {
    shared::get(shell).update_system(&|st| st.volume = v);
}

/// The brightness (0 to 1) as the keys left it.
pub fn set_brightness(shell: &Rc<Shell>, b: f64) {
    shared::get(shell).update_system(&|st| st.brightness = Some(b));
}

/// Tells labwc (through foreign-toplevel).
pub fn wm(shell: &Shell, cmd: Cmd) {
    if let Some(wm) = shell.placer.wm() {
        wm.send(cmd);
    }
}

/// What's between the halves on the desktop: the running wadspaces, then
/// the machine's apps and its other windows.
pub struct Middle {
    root: gtk::Box,
    running: Rc<running::Running>,
    apps: Rc<apps::Apps>,
}

impl Middle {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        style::install();
        let shared = shared::get(shell);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Center)
            .build();
        let running = running::Running::new(shell, &shared);
        let apps = apps::Apps::new(shell, &shared);
        root.append(&running.root);
        root.append(&apps.root);
        let me = Rc::new(Self { root, running, apps });
        shared.set_middle(&me);
        // machine.rs gives it the room between the halves: the running
        // wadspaces take what they need (but leave the apps some), the apps the rest.
        let w = Rc::downgrade(&me);
        me.root.connect_notify_local(Some("width-request"), move |r, _| {
            if let Some(m) = w.upgrade() {
                let room = r.width_request();
                m.running.set_max(room - 270);
                m.apps.fit();
            }
        });
        me
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// What it shows, for `wadsctl bar`.
    fn describe(&self) -> String {
        format!("{} {}", self.running.describe(), self.apps.describe())
    }

    fn menu(&self, id: &str) -> bool {
        self.running.menu(id) || self.apps.menu(id)
    }

    fn peek(&self, app: &str) -> bool {
        self.apps.peek(app)
    }

    fn click(&self, what: &str) -> bool {
        self.running.click(what) || self.apps.click(what)
    }

    /// Where a wadspace's, an app's or a window's item is, on the screen.
    fn where_(&self, what: &str) -> Option<(f64, f64)> {
        self.running.where_(what).or_else(|| self.apps.where_(what))
    }
}
