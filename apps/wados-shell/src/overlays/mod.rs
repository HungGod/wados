//! What shows over everything, full-screen wadspaces included (OVERLAY
//! layer): toasts, Super+Tab's switcher, Alt+Space's search, the reveal bar
//! over a full-screen wadspace, focus's covers.
//!
//! The interface main.rs uses: `Overlays::new(shell)`; it registers its own
//! `wadsctl` commands (`switcher next|prev`, `search`, `log`) with the shell.
//!
//! Phase 2 stub: one card in the middle of the top for a moment (a toast,
//! or a stand-in for the switcher and the search). The real overlays
//! replace it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::{glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::shell::Shell;
use crate::store::Change;
use crate::surface::{self, Anchor, Spec};

pub struct Overlays {
    card: gtk::ApplicationWindow,
    label: gtk::Label,
    showing: Rc<Cell<u64>>,
    /// What it was asked to show (tests read it back: `wadsctl log`).
    log: RefCell<Vec<String>>,
}

impl Overlays {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let card = surface::make(
            &shell.app,
            &shell.primary,
            &Spec {
                namespace: "wados-overlay",
                layer: Layer::Overlay,
                anchor: Anchor::Top,
                exclusive: -1,
                keyboard: KeyboardMode::None,
                height: 0,
            },
        );
        card.set_anchor(Edge::Left, false);
        card.set_anchor(Edge::Right, false);
        card.set_margin(Edge::Top, 160);
        let label = gtk::Label::builder().css_classes(["overlay-card", "display"]).build();
        card.set_child(Some(&label));
        surface::click_through(&card);
        let me = Rc::new(Self { card, label, showing: Rc::default(), log: RefCell::default() });

        let w = Rc::downgrade(&me);
        shell.store.watch(move |c| {
            if let (Some(m), Change::Notice { title, .. }) = (w.upgrade(), c) {
                m.flash(title);
            }
        });
        let w = Rc::downgrade(&me);
        shell.on_ctl("switcher", move |args| {
            let Some(m) = w.upgrade() else { return "error gone".into() };
            m.flash(&format!("Super+Tab: switcher {}", args.first().copied().unwrap_or("next")));
            "ok".into()
        });
        let w = Rc::downgrade(&me);
        shell.on_ctl("search", move |_| {
            let Some(m) = w.upgrade() else { return "error gone".into() };
            m.flash("Alt+Space: search");
            "ok".into()
        });
        let w = Rc::downgrade(&me);
        shell.on_ctl("log", move |_| match w.upgrade() {
            Some(m) => format!("ok {}", m.log.borrow().join(" | ")),
            None => "error gone".into(),
        });
        me
    }

    /// A card in the middle of the top for a moment.
    pub fn flash(&self, text: &str) {
        self.log.borrow_mut().push(text.to_string());
        self.label.set_label(text);
        self.card.present();
        let generation = self.showing.get() + 1;
        self.showing.set(generation);
        let (card, current) = (self.card.downgrade(), self.showing.clone());
        glib::timeout_add_local_once(std::time::Duration::from_millis(1600), move || {
            if current.get() == generation
                && let Some(c) = card.upgrade()
            {
                c.set_visible(false);
            }
        });
    }
}
