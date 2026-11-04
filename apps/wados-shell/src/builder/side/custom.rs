//! The sidebar's Customize tab (the spike's `SideCustom.svelte`). Phase 3 builds it.

use std::rc::{Rc, Weak};

use gtk::prelude::*;
use wad_model::Wadspace;

use crate::builder::Builder;

pub struct Panel {
    _builder: Weak<Builder>,
    root: gtk::Box,
    note: gtk::Label,
}

impl Panel {
    pub fn new(b: &Rc<Builder>) -> Rc<Self> {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
        let note = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["dim"]).build();
        root.append(&note);
        Rc::new(Self { _builder: Rc::downgrade(b), root, note })
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// The selected wadspace, as it is now.
    pub fn show(&self, x: &Wadspace, _editable: bool) {
        self.note.set_label(&format!("Customize for {}", x.name));
    }
}
