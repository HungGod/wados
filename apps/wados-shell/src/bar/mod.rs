//! The bar (`shell/bar/*`): on the launcher a dock at the bottom; on the
//! desktop two halves, the logo's at the top left and the tray's at the top
//! right, with what's running and the machine's apps between them.
//!
//! The interface machine.rs uses:
//! - `Halves::new(shell, on_desktop)`: the two halves (built twice: in the
//!   launcher's surface, where they fly between their two places, and in
//!   the bar's, where they stay); `left`, `right`, `set_on_desktop`.
//! - `Middle::new(shell)`: what's between the halves on the desktop (sized
//!   by machine.rs to the room between them); `widget`.
//!
//! Phase 2 stub: the halves as phase 0 had them, and the running wadspaces'
//! names in the middle. The real bar replaces it.

use std::rc::Rc;

use gtk::{glib, prelude::*};
use wad_gtk::{Icon, Svg};

use crate::machine::Page;
use crate::shell::Shell;

static MARK: &[u8] = include_bytes!("../../../../assets/brand/wadspaces-icon-dark-transparent.svg");

pub struct Halves {
    left: gtk::Box,
    right: gtk::Box,
    page_icon: Icon,
}

fn button(icon: &'static str, tip: &str) -> (gtk::Button, Icon) {
    let i = Icon::new(icon, 18);
    let b = gtk::Button::builder().child(&i).tooltip_text(tip).css_classes(["bar-button"]).build();
    (b, i)
}

impl Halves {
    pub fn new(shell: &Rc<Shell>, on_desktop: bool) -> Rc<Self> {
        let left =
            gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(4).css_classes(["bar-half"]).build();
        left.append(&Svg::new(MARK, 34, 34));
        let word = gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).build();
        word.append(&gtk::Label::builder().label("WAD").css_classes(["brand-wad"]).xalign(0.0).build());
        word.append(&gtk::Label::builder().label("SPACES").css_classes(["brand-spaces"]).xalign(0.0).build());
        left.append(&word);
        let (page, page_icon) = button("arrow-down", "The desktop");
        let s = shell.clone();
        page.connect_clicked(move |_| s.go(if s.page() == Page::Launcher { Page::Desktop } else { Page::Launcher }));
        left.append(&page);
        let (search, _) = button("search", "Search (Alt+Space)");
        let s = shell.clone();
        search.connect_clicked(move |_| {
            let _ = s.ctl(&["search"]);
        });
        left.append(&search);

        let right =
            gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(2).css_classes(["bar-half"]).build();
        for (icon, tip) in [
            ("radio", "Streams"),
            ("clipboard", "Clipboard"),
            ("bluetooth", "Bluetooth"),
            ("sun", "Brightness"),
            ("volume-2", "Sound"),
            ("wifi", "Wi-Fi"),
            ("bell", "Notifications"),
        ] {
            right.append(&button(icon, tip).0);
        }
        let clock = gtk::Label::builder().css_classes(["bar-clock"]).build();
        right.append(&clock);
        let c = clock.downgrade();
        let tick = move || {
            let Some(c) = c.upgrade() else { return glib::ControlFlow::Break };
            if let Ok(t) = glib::DateTime::now_local().and_then(|n| n.format("%-I:%M %P")) {
                c.set_label(&t);
            }
            glib::ControlFlow::Continue
        };
        tick();
        glib::timeout_add_seconds_local(10, tick);
        let me = Rc::new(Self { left, right, page_icon });
        me.set_on_desktop(on_desktop);
        me
    }

    pub fn left(&self) -> gtk::Widget {
        self.left.clone().upcast()
    }

    pub fn right(&self) -> gtk::Widget {
        self.right.clone().upcast()
    }

    /// Which way the page button points: down on the launcher, up on the desktop.
    pub fn set_on_desktop(&self, on: bool) {
        self.page_icon.set(if on { "arrow-up" } else { "arrow-down" });
    }
}

pub struct Middle {
    root: gtk::Box,
}

impl Middle {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let root =
            gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).halign(gtk::Align::Start).build();
        let me = Rc::new(Self { root });
        let (w, s) = (Rc::downgrade(&me), shell.clone());
        shell.store.watch(move |_| {
            let Some(m) = w.upgrade() else { return };
            while let Some(c) = m.root.first_child() {
                m.root.remove(&c);
            }
            for ws in s.store.running() {
                m.root.append(&gtk::Label::builder().label(&ws.name).css_classes(["bar-half"]).build());
            }
        });
        me
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
}
