//! The desktop's top bar after its left half (`RunningSpaces.svelte`): the
//! running wadspaces, and only them, however each shows (full screen, in a
//! window, in the background). Click one to bring it up; right-click for
//! full screen, a window, hiding it, or stopping it. Nothing running, it
//! says so. Sideways when there's more than fits.
//!
//! Each is a drop target for what's held (`builder::drops`, the spike's
//! `spaces/targets.ts`): another's piece is copied in. It's lit while it
//! would take it, and the hint's under the pointer.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use wad_model::{Phase, SessionDisplay};

use super::shared::{Shared, Topic};
use super::ui::{self, Entry, Item, entry};
use crate::builder::drops;
use crate::dnd;
use crate::shell::Shell;

pub struct Running {
    pub root: gtk::Box,
    shell: Rc<Shell>,
    scroll: gtk::ScrolledWindow,
    list: gtk::Box,
    none: gtk::Label,
    items: RefCell<Vec<(String, Item)>>,
    sig: RefCell<Vec<String>>,
}

/// A running wadspace, as its item shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub name: String,
    /// Its window is in front.
    pub active: bool,
    /// On its way up.
    pub opening: bool,
    /// The display it's on, if it isn't the primary (its number).
    pub on: Option<usize>,
}

impl Running {
    pub fn new(shell: &Rc<Shell>, shared: &Rc<Shared>) -> Rc<Self> {
        let root = gtk::Box::builder().css_classes(["wb-strip"]).valign(gtk::Align::Center).build();
        let list = gtk::Box::builder().spacing(2).valign(gtk::Align::Center).build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .css_classes(["wb-scroll"])
            .propagate_natural_width(true)
            .child(&list)
            .valign(gtk::Align::Center)
            .build();
        root.append(&scroll);
        let none = gtk::Label::builder().label("No wadspaces running").css_classes(["wb-none"]).build();
        root.append(&none);
        let me = Rc::new(Self {
            root,
            shell: shell.clone(),
            scroll,
            list,
            none,
            items: RefCell::default(),
            sig: RefCell::default(),
        });
        let w = Rc::downgrade(&me);
        shell.store.watch(move |_| {
            if let Some(m) = w.upgrade() {
                m.fill();
            }
        });
        let w = Rc::downgrade(&me);
        shared.on(Topic::Windows, move || {
            if let Some(m) = w.upgrade() {
                m.fill();
            }
        });
        me.fill();
        me
    }

    pub fn set_max(&self, max: i32) {
        self.scroll.set_max_content_width(max.max(120));
    }

    fn rows(&self) -> Vec<Row> {
        let s = &self.shell;
        let windows = s.placer.windows();
        let outputs = display_connectors(s);
        s.store
            .running()
            .into_iter()
            .map(|ws| {
                let mine: Vec<_> = windows.iter().filter(|t| t.wadspace() == Some(ws.id.as_str())).collect();
                let opening =
                    s.store.run(&ws.id).is_some_and(|r| matches!(r.state.phase, Phase::Starting | Phase::Waiting));
                let on = mine
                    .iter()
                    .filter_map(|t| t.outputs.first())
                    .filter_map(|o| outputs.iter().position(|c| c == o))
                    .find(|i| *i > 0)
                    .map(|i| i + 1);
                Row { active: mine.iter().any(|t| super::in_front(t)), opening, on, name: ws.name, id: ws.id }
            })
            .collect()
    }

    fn fill(self: &Rc<Self>) {
        let rows = self.rows();
        let sig: Vec<String> = rows
            .iter()
            .map(|r| {
                let tile = self.shell.store.tile_state(&r.id);
                format!("{r:?}{tile:?}{:?}", self.shell.store.wadspace(&r.id).map(|w| w.updated_at))
            })
            .collect();
        if *self.sig.borrow() == sig {
            return;
        }
        *self.sig.borrow_mut() = sig;
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let mut items = vec![];
        for r in &rows {
            let Some(ws) = self.shell.store.wadspace(&r.id) else { continue };
            let tile = wad_gtk::WadspaceTile::new(0.6, false);
            tile.set(&ws, &self.shell.store.tile_state(&r.id));
            let item = Item::new(&r.name, &tile.root);
            item.set_active(r.active);
            item.set_dim(r.opening);
            if r.opening {
                item.button.set_tooltip_text(Some(&format!("{}: opening", r.name)));
            }
            if let Some(n) = r.on {
                item.over.add_overlay(&ui::display_number(n));
            }
            let (s, id) = (self.shell.clone(), r.id.clone());
            item.button.connect_clicked(move |_| s.show(&id));
            let (w, id) = (Rc::downgrade(self), r.id.clone());
            ui::on_right_click(&item.button, move |_, _| {
                if let Some(m) = w.upgrade() {
                    m.menu(&id);
                }
            });
            self.take_drops(&r.id, &item.button);
            self.list.append(&item.button);
            items.push((r.id.clone(), item));
        }
        self.none.set_visible(rows.is_empty());
        self.scroll.set_visible(!rows.is_empty());
        *self.items.borrow_mut() = items;
    }

    /// `button` (`id`'s) takes what's held: another's piece, copied in.
    fn take_drops(&self, id: &str, button: &gtk::Button) {
        let (s, ws) = (self.shell.clone(), id.to_string());
        let offer = move |held: &dnd::Held, _: f64, _: f64| drops::offer_on(&s, &ws, held);
        let (s, ws) = (self.shell.clone(), id.to_string());
        let land = move |held: &dnd::Held, _: f64, _: f64| drops::land_on(&s, &ws, held);
        let b = button.downgrade();
        let hover = move |on: bool| {
            if let Some(b) = b.upgrade() {
                ui::set_class(&b, "wb-hot", on);
            }
        };
        dnd::target(button, offer, land, hover);
    }

    /// Where its item is, in the bar's surface (it's at the screen's top left).
    pub fn where_(&self, id: &str) -> Option<(f64, f64)> {
        let items = self.items.borrow();
        let (_, item) = items.iter().find(|(x, _)| x == id)?;
        ui::middle_of(&item.button)
    }

    /// Its menu: full screen, a window, hide it, stop it.
    pub fn menu(&self, id: &str) -> bool {
        let items = self.items.borrow();
        let Some((_, item)) = items.iter().find(|(x, _)| x == id) else { return false };
        let Some(ws) = self.shell.store.wadspace(id) else { return false };
        let shown = self.shell.store.display(id);
        let s = &self.shell;
        let how = |label: &str, icon: &'static str, d: SessionDisplay| -> Entry {
            let (s, id) = (s.clone(), id.to_string());
            entry(label, icon, move || s.open(&id, d)).checked(shown == Some(d))
        };
        let stop = {
            let (s, id) = (s.clone(), id.to_string());
            entry(&format!("Stop {}", ws.name), "square", move || s.stop(&id)).danger()
        };
        let mut rows = vec![how("Full screen", "monitor-play", SessionDisplay::Screen)];
        for d in s.displays().into_iter().skip(1) {
            let (sh, id) = (s.clone(), id.to_string());
            let label = format!("Full screen on {}: {}", d.n, d.name);
            rows.push(entry(&label, "monitor-up", move || sh.open_on(&id, &d.connector)));
        }
        rows.extend([
            how("In a window", "app-window", SessionDisplay::Window),
            how("Hide it (it keeps running)", "eye-off", SessionDisplay::Background),
            Entry::Sep,
            stop,
        ]);
        ui::menu(&self.root, &item.button, rows, true);
        true
    }

    pub fn click(&self, id: &str) -> bool {
        let items = self.items.borrow();
        match items.iter().find(|(x, _)| x == id) {
            Some((_, item)) => {
                item.button.emit_clicked();
                true
            }
            None => false,
        }
    }

    pub fn describe(&self) -> String {
        let rows = self.rows();
        let shown: Vec<String> = rows
            .iter()
            .map(|r| format!("{}{}{}", r.id, if r.active { "*" } else { "" }, if r.opening { "~" } else { "" }))
            .collect();
        format!("spaces=[{}]", shown.join(","))
    }
}

/// The displays' connectors, the primary first (display 1, 2…).
pub fn display_connectors(shell: &Shell) -> Vec<String> {
    std::iter::once(&*shell.primary)
        .chain(shell.others.iter())
        .map(|m| m.connector().map(|c| c.to_string()).unwrap_or_default())
        .collect()
}
