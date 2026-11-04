//! The logo's menu (`shell/bar/Launcher.svelte`): search, your wadspaces
//! (what's running, then the latest launched: two rows; the rest on the
//! launcher), the machine's own apps (and the Builder), and power. It's the
//! logo's half grown out: over it on the dock, under it on the desktop.

use std::cell::Cell;
use std::rc::Rc;

use gtk::{glib, prelude::*};
use wad_gtk::Icon;
use wad_model::{SessionDisplay, Wadspace};

use super::tasks::MachineApp;
use super::ui::{self, label};
use crate::machine::Page;
use crate::shell::Shell;

/// Two rows of four.
const MAX: usize = 8;
/// A cell's width: a quarter of the menu.
const CELL: i32 = 97;

/// The machine's own apps in the menu, with what else finds one ("ai" finds Agents).
const APPS: [(&str, &str, &[&str]); 7] = [
    ("files", "Files", &[]),
    ("browser", "WadBrowser", &["browser", "web"]),
    ("terminal", "Terminal", &["shell", "console"]),
    ("settings", "Settings", &[]),
    ("agents", "Agents", &["ai agents", "claude code", "claude"]),
    ("updates", "Updates", &[]),
    ("builder", "Builder", &[]),
];

pub struct Start {
    shell: Rc<Shell>,
    button: gtk::Button,
    pop: gtk::Popover,
    query: gtk::Text,
    body: gtk::Box,
    closed_at: Cell<Option<std::time::Instant>>,
    below: Cell<bool>,
}

impl Start {
    pub fn new(shell: &Rc<Shell>, button: &gtk::Button, below: bool) -> Rc<Self> {
        let pop = ui::popover(button, &["wb-start-pop"], below);
        let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
        col.set_size_request(396, -1);
        let search = gtk::Box::builder().spacing(9).css_classes(["wb-search"]).build();
        search.append(&Icon::new("search", 15));
        let query = gtk::Text::builder().placeholder_text("Search wadspaces and apps").hexpand(true).build();
        search.append(&query);
        col.append(&search);
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        col.append(&body);
        pop.set_child(Some(&col));
        let me = Rc::new(Self {
            shell: shell.clone(),
            button: button.clone(),
            pop,
            query,
            body,
            closed_at: Cell::new(None),
            below: Cell::new(below),
        });
        let w = Rc::downgrade(&me);
        me.query.connect_changed(move |_| {
            if let Some(m) = w.upgrade() {
                m.fill();
            }
        });
        let w = Rc::downgrade(&me);
        me.query.connect_activate(move |_| {
            // Enter: the first it found.
            if let Some(m) = w.upgrade() {
                if let Some(ws) = m.found().first() {
                    m.open_ws(ws);
                } else if let Some((key, _, _)) = m.found_apps().first() {
                    m.open_app(key);
                }
            }
        });
        let w = Rc::downgrade(&me);
        me.pop.connect_closed(move |_| {
            if let Some(m) = w.upgrade() {
                m.closed_at.set(Some(std::time::Instant::now()));
                m.button.remove_css_class("wb-on");
            }
        });
        let w = Rc::downgrade(&me);
        button.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                if m.closed_at.get().is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(250)) {
                    return;
                }
                m.open();
            }
        });
        me
    }

    pub fn set_below(&self, below: bool) {
        self.below.set(below);
        self.close();
        self.pop.set_position(if below { gtk::PositionType::Bottom } else { gtk::PositionType::Top });
        self.pop.set_offset(0, if below { 8 } else { -8 });
    }

    pub fn open(self: &Rc<Self>) {
        self.query.set_text("");
        self.fill();
        self.button.add_css_class("wb-on");
        // The logo's half grown out: from its left edge, over it or under it.
        if let Some(h) = self.button.parent().and_then(|h| h.compute_bounds(&self.button)) {
            let below = self.below.get();
            self.pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                h.x() as i32,
                h.y() as i32,
                422,
                h.height() as i32,
            )));
            self.pop.set_offset(0, if below { 8 } else { -8 });
        }
        self.pop.popup();
        self.query.grab_focus();
    }

    pub fn close(&self) {
        self.pop.popdown();
    }

    fn q(&self) -> String {
        self.query.text().trim().to_lowercase()
    }

    /// The machine's wadspaces that match, what's running first, then the latest launched.
    fn found(&self) -> Vec<Wadspace> {
        let q = self.q();
        let store = &self.shell.store;
        let mut all: Vec<Wadspace> =
            store.live().into_iter().filter(|w| q.is_empty() || w.name.to_lowercase().contains(&q)).collect();
        all.sort_by_key(|w| (!store.is_running(&w.id), std::cmp::Reverse(w.last_launched.unwrap_or(0))));
        all
    }

    fn found_apps(&self) -> Vec<(&'static str, &'static str, &'static [&'static str])> {
        let q = self.q();
        APPS.into_iter()
            .filter(|(_, name, also)| {
                q.is_empty() || name.to_lowercase().contains(&q) || also.iter().any(|a| a.contains(q.as_str()))
            })
            .collect()
    }

    fn fill(self: &Rc<Self>) {
        while let Some(c) = self.body.first_child() {
            self.body.remove(&c);
        }
        let all = self.found();
        let total = self.shell.store.live().len();
        let head = gtk::Box::builder().build();
        let h = label("WADSPACES", &["wb-h4"]);
        h.set_hexpand(true);
        head.append(&h);
        if all.len() > MAX {
            let b = gtk::Button::builder()
                .css_classes(["wb-quiet"])
                .tooltip_text(format!("All {total} of your wadspaces, on the launcher"))
                .build();
            let r = gtk::Box::builder().spacing(2).build();
            r.append(&gtk::Label::new(Some(&format!("All {total}"))));
            r.append(&Icon::new("chevron-right", 13));
            b.set_child(Some(&r));
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.close();
                    m.shell.go(Page::Launcher);
                }
            });
            head.append(&b);
        }
        self.body.append(&head);
        let grid = gtk::Grid::builder().column_spacing(2).row_spacing(2).halign(gtk::Align::Start).build();
        for (i, ws) in all.iter().take(MAX).enumerate() {
            let tile = wad_gtk::WadspaceTile::new(1.0, true);
            tile.set(ws, &self.shell.store.tile_state(&ws.id));
            let b = gtk::Button::builder()
                .child(&tile.root)
                .css_classes(["wb-res"])
                .tooltip_text(format!("Open {}", ws.name))
                .build();
            b.set_size_request(CELL, -1);
            let (w, ws) = (Rc::downgrade(self), ws.clone());
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.open_ws(&ws);
                }
            });
            grid.attach(&b, (i % 4) as i32, (i / 4) as i32, 1, 1);
        }
        if all.is_empty() {
            let q = self.query.text();
            let msg = if q.is_empty() {
                "None yet: make one on the desktop.".to_string()
            } else {
                format!("No wadspaces called \u{201c}{q}\u{201d}.")
            };
            self.body.append(&label(&msg, &["wb-p"]));
        } else {
            self.body.append(&grid);
        }

        let apps = self.found_apps();
        if !apps.is_empty() {
            self.body.append(&label("THIS MACHINE", &["wb-h4"]));
        }
        let grid = gtk::Grid::builder().column_spacing(2).row_spacing(2).halign(gtk::Align::Start).build();
        for (i, (key, name, _)) in apps.into_iter().enumerate() {
            let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build();
            let ai = gtk::Box::builder().css_classes(["wb-ai"]).homogeneous(true).halign(gtk::Align::Center).build();
            ai.append(&Icon::new(icon_of(key), 20));
            col.append(&ai);
            col.append(&gtk::Label::builder().label(name).css_classes(["wb-res-name"]).build());
            let b =
                gtk::Button::builder().child(&col).css_classes(["wb-res"]).tooltip_text(format!("Open {name}")).build();
            b.set_size_request(CELL, -1);
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.open_app(key);
                }
            });
            grid.attach(&b, (i % 4) as i32, (i / 4) as i32, 1, 1);
        }
        self.body.append(&grid);

        // Who, and power.
        let foot = gtk::Box::builder().spacing(4).css_classes(["wb-foot"]).margin_top(8).build();
        let me = gtk::Box::builder().spacing(8).hexpand(true).build();
        let name = glib::real_name().to_string_lossy().to_string();
        let name =
            if name.is_empty() || name == "Unknown" { glib::user_name().to_string_lossy().to_string() } else { name };
        let avatar = gtk::Label::builder()
            .label(name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default())
            .css_classes(["wb-avatar"])
            .build();
        me.append(&avatar);
        me.append(&label(&name, &["wb-me"]));
        foot.append(&me);
        let (lock, _) = ui::tb("lock", 15, "Lock");
        lock.add_css_class("wb-pw");
        let w = Rc::downgrade(self);
        lock.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.close();
                // The lock is main.rs's: asked for the way labwc's Ctrl+Alt+L does.
                if !ui::spawn(&["wadsctl", "lock"]) {
                    m.shell.toast(wad_proto::Level::Error, "Couldn't lock: wadsctl isn't on the machine", None);
                }
            }
        });
        foot.append(&lock);
        for (icon, tip) in [
            ("server", "Server mode (the screen goes off and it only streams) comes in phase 10"),
            ("rotate-ccw", "Restart comes with the host image (phase 4)"),
            ("power", "Power off comes with the host image (phase 4)"),
        ] {
            let (b, _) = ui::tb(icon, 15, tip);
            b.add_css_class("wb-pw");
            b.set_sensitive(false);
            // Its tooltip still says why.
            let holder = gtk::Box::builder().tooltip_text(tip).build();
            holder.append(&b);
            foot.append(&holder);
        }
        self.body.append(&foot);
    }

    fn open_ws(&self, ws: &Wadspace) {
        self.close();
        if self.shell.store.is_running(&ws.id) {
            self.shell.show(&ws.id);
        } else {
            self.shell.open(&ws.id, SessionDisplay::Screen);
        }
    }

    fn open_app(&self, key: &str) {
        self.close();
        let s = &self.shell;
        match key {
            "files" => super::apps::launch(s, MachineApp::Files),
            "browser" => super::apps::launch(s, MachineApp::Browser),
            "settings" => super::apps::launch(s, MachineApp::Settings),
            "terminal" => {
                if !ui::spawn(&["foot", "--app-id", "io.wados.Terminal"]) {
                    s.toast(wad_proto::Level::Warn, "There's no terminal (foot) on this machine", None);
                }
            }
            "builder" => {
                if let Some(b) = s.builder() {
                    b.present();
                }
            }
            "agents" => s.toast(wad_proto::Level::Info, "Agents come in phase 11", None),
            "updates" => s.toast(wad_proto::Level::Info, "Updates come in phase 8", None),
            _ => {}
        }
    }
}

fn icon_of(key: &str) -> &'static str {
    match key {
        "files" => "folder",
        "browser" => "globe",
        "terminal" => "square-terminal",
        "settings" => "settings",
        "agents" => "bot",
        "updates" => "download",
        _ => "blocks",
    }
}
