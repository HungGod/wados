//! The bar's two halves. On the launcher they're one dock at the bottom; on
//! the desktop the logo's half sits top left and the tray's half top right,
//! with what's running between them (`shell/bar/Panel.svelte`). The same
//! halves are built twice: once in the launcher's surface, where they fly
//! between the two places, and once in the bar's, where they stay.

use gtk::{glib, prelude::*};
use wad_gtk::{Icon, Svg};

static MARK: &[u8] = include_bytes!("../../../assets/brand/wadspaces-icon-dark-transparent.svg");

/// What the halves ask the shell for.
#[derive(Clone, Copy, Debug)]
pub enum Ask {
    /// The ↓ (to the desktop) or ↑ (to the launcher) button.
    Page,
    Search,
}

pub struct Halves {
    pub left: gtk::Box,
    pub right: gtk::Box,
    page_icon: Icon,
    clock: gtk::Label,
}

fn button(icon: &'static str, tip: &str) -> (gtk::Button, Icon) {
    let i = Icon::new(icon, 18);
    let b = gtk::Button::builder().child(&i).tooltip_text(tip).css_classes(["bar-button"]).build();
    (b, i)
}

impl Halves {
    pub fn new(ask: impl Fn(Ask) + Clone + 'static) -> Self {
        let left =
            gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(4).css_classes(["bar-half"]).build();
        let mark = Svg::new(MARK, 34, 34);
        left.append(&mark);
        let word = gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).build();
        let wad = gtk::Label::builder().label("WAD").css_classes(["brand-wad"]).xalign(0.0).build();
        let spaces = gtk::Label::builder().label("SPACES").css_classes(["brand-spaces"]).xalign(0.0).build();
        word.append(&wad);
        word.append(&spaces);
        left.append(&word);
        let (page, page_icon) = button("arrow-down", "The desktop");
        let a = ask.clone();
        page.connect_clicked(move |_| a(Ask::Page));
        left.append(&page);
        let (search, _) = button("search", "Search (Alt+Space)");
        search.connect_clicked(move |_| ask(Ask::Search));
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
            let (b, _) = button(icon, tip);
            right.append(&b);
        }
        let clock = gtk::Label::builder().css_classes(["bar-clock"]).build();
        right.append(&clock);

        let me = Self { left, right, page_icon, clock };
        me.tick_clock();
        me
    }

    /// Which way the page button points: down on the launcher, up on the desktop.
    pub fn set_on_desktop(&self, on: bool) {
        self.page_icon.set(if on { "arrow-up" } else { "arrow-down" });
    }

    fn tick_clock(&self) {
        let clock = self.clock.downgrade();
        let set = move || {
            let Some(c) = clock.upgrade() else { return glib::ControlFlow::Break };
            if let Ok(now) = glib::DateTime::now_local()
                && let Ok(t) = now.format("%-I:%M %P")
            {
                c.set_label(&t);
            }
            glib::ControlFlow::Continue
        };
        set();
        glib::timeout_add_seconds_local(10, set);
    }
}
