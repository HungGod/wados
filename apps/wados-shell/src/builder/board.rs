//! The Builder's board: a stack per wadspace open in it, left to right (two
//! side by side, more scrolling), and the slot while something's held (the
//! spike's `Stack.svelte`, `Pill.svelte`, `parts.ts`). Phase 3 builds it.

use std::rc::{Rc, Weak};

use gtk::prelude::*;

use super::Builder;

pub struct Board {
    builder: Weak<Builder>,
    scroll: gtk::ScrolledWindow,
    row: gtk::Box,
}

impl Board {
    pub fn new(b: &Rc<Builder>) -> Rc<Self> {
        let row = gtk::Box::builder().spacing(12).css_classes(["builder-board"]).build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .child(&row)
            .build();
        Rc::new(Self { builder: Rc::downgrade(b), scroll, row })
    }

    pub fn widget(&self) -> gtk::Widget {
        self.scroll.clone().upcast()
    }

    /// The stacks as they are now.
    pub fn refresh(&self) {
        let Some(b) = self.builder.upgrade() else { return };
        while let Some(c) = self.row.first_child() {
            self.row.remove(&c);
        }
        let selected = b.selected();
        for id in b.stacks() {
            let Some(x) = b.wadspace(&id) else { continue };
            let l = gtk::Label::new(Some(&x.name));
            if selected.as_deref() == Some(id.as_str()) {
                l.add_css_class("accent");
            }
            self.row.append(&l);
        }
    }

    /// Scrolled so `ws`'s stack shows.
    pub fn scroll_to(&self, _ws: &str) {}
}
