//! The launcher (`shell/home/Home.svelte`): the bar along its top (the
//! machine, Running, Recent, the tags), a console-style carousel of square
//! cards of that machine's wadspaces, and under the card in focus its
//! buttons. All of it by the arrow keys.
//!
//! The interface machine.rs uses: `new`, `widget` (sized by machine.rs to the
//! screen above the dock, and slid/faded by it), `key` (the chrome's keys,
//! before machine.rs's own), `shown` (each time the page is entered).
//!
//! Phase 2 stub: a row of the live wadspaces' tiles. The real launcher
//! replaces it.

use std::rc::Rc;

use gtk::{gdk, prelude::*};
use wad_model::SessionDisplay;

use crate::shell::Shell;

pub struct Launcher {
    shell: Rc<Shell>,
    root: gtk::Box,
    row: gtk::Box,
}

impl Launcher {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).build();
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(24)
            .halign(gtk::Align::Center)
            .build();
        root.append(&row);
        let me = Rc::new(Self { shell: shell.clone(), root, row });
        let w = Rc::downgrade(&me);
        shell.store.watch(move |_| {
            if let Some(m) = w.upgrade() {
                m.refill();
            }
        });
        me
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    pub fn key(&self, _key: gdk::Key, _mods: gdk::ModifierType) -> bool {
        false
    }

    pub fn shown(&self) {
        self.refill();
    }

    fn refill(&self) {
        while let Some(c) = self.row.first_child() {
            self.row.remove(&c);
        }
        for ws in self.shell.store.live() {
            let tile = wad_gtk::WadspaceTile::new(3.0, true);
            tile.set(&ws, &self.shell.store.tile_state(&ws.id));
            let b = gtk::Button::builder().child(&tile.root).css_classes(["flat"]).build();
            let (shell, id) = (self.shell.clone(), ws.id.clone());
            b.connect_clicked(move |_| shell.open(&id, SessionDisplay::Screen));
            self.row.append(&b);
        }
    }
}
