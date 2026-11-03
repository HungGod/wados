//! The desktop (`shell/desktop/Desktop.svelte`): icons and nothing else, on a
//! grid from the top left: this machine's wadspaces with no tag (glass
//! tiles), a folder per tag, the desktop's files, then Machines and the Trash.
//!
//! The interface machine.rs uses: `new`, `widget` (sized by machine.rs to
//! the screen below the bar), `key` (the desktop surface's keys, before
//! machine.rs's own), `shown` (each time the page is entered).
//!
//! Phase 2 stub: the untagged wadspaces' tiles, and a drop target. The real
//! desktop replaces it.

use std::rc::Rc;

use gtk::{gdk, glib, prelude::*};
use wad_model::SessionDisplay;

use crate::shell::Shell;

pub struct Desktop {
    shell: Rc<Shell>,
    grid: gtk::FlowBox,
}

impl Desktop {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let grid = gtk::FlowBox::builder()
            .orientation(gtk::Orientation::Vertical)
            .max_children_per_line(6)
            .selection_mode(gtk::SelectionMode::None)
            .valign(gtk::Align::Start)
            .halign(gtk::Align::Start)
            .margin_top(16)
            .margin_start(16)
            .build();
        let me = Rc::new(Self { shell: shell.clone(), grid });
        let w = Rc::downgrade(&me);
        shell.store.watch(move |_| {
            if let Some(m) = w.upgrade() {
                m.refill();
            }
        });
        let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY | gdk::DragAction::MOVE);
        drop.connect_drop(move |_, value, x, y| {
            tracing::info!("dropped {:?} on the desktop at {x:.0},{y:.0}", value.get::<String>().ok());
            true
        });
        me.grid.add_controller(drop);
        me
    }

    pub fn widget(&self) -> gtk::Widget {
        self.grid.clone().upcast()
    }

    pub fn key(&self, _key: gdk::Key, _mods: gdk::ModifierType) -> bool {
        false
    }

    pub fn shown(&self) {}

    fn refill(&self) {
        self.grid.remove_all();
        for ws in self.shell.store.live().into_iter().filter(|w| w.tag.is_none()) {
            let tile = wad_gtk::WadspaceTile::new(1.3, true);
            tile.set(&ws, &self.shell.store.tile_state(&ws.id));
            let click = gtk::GestureClick::new();
            let (shell, id) = (self.shell.clone(), ws.id.clone());
            click.connect_released(move |g, n, _, _| {
                if n == 2 {
                    g.set_state(gtk::EventSequenceState::Claimed);
                    shell.open(&id, SessionDisplay::Screen);
                }
            });
            tile.root.add_controller(click);
            self.grid.insert(&tile.root, -1);
        }
    }
}
