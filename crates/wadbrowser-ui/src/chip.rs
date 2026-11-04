//! The profile chip in the header bar (the spike's `.chip`): a dot in the
//! profile's colour and its name, on a tint of that colour; or, in a private
//! window, "Private" with an eye-off, in magenta. Clicking it opens the
//! profile menu ([`crate::menus::profile_menu`]).

use std::cell::RefCell;

use gtk::prelude::*;

use crate::data::safe_color;

/// What the chip shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChipInfo {
    /// The profile's name.
    pub name: String,
    /// Its colour, `#rrggbb`.
    pub color: String,
    /// A private window: "Private", whatever the profile.
    pub private: bool,
}

pub struct ProfileChip {
    button: gtk::Button,
    dot: gtk::Box,
    eye: wad_gtk::Icon,
    label: gtk::Label,
    tint: RefCell<String>,
}

impl ProfileChip {
    /// The chip; `clicked` is given it, to open its menu below.
    pub fn new(info: &ChipInfo, clicked: impl Fn(&gtk::Button) + 'static) -> ProfileChip {
        let dot = gtk::Box::builder().css_classes(["wb-chip-dot"]).valign(gtk::Align::Center).build();
        dot.set_size_request(9, 9);
        let eye = wad_gtk::Icon::new("eye-off", 13);
        eye.add_css_class("wb-chip-eye");
        let label = gtk::Label::new(None);
        let inner = gtk::Box::builder().spacing(7).build();
        inner.append(&dot);
        inner.append(&eye);
        inner.append(&label);
        let button = gtk::Button::builder()
            .child(&inner)
            .css_classes(["wb-chip"])
            .tooltip_text("Profile")
            .valign(gtk::Align::Center)
            .build();
        button.connect_clicked(move |b| clicked(b));
        let chip = ProfileChip { button, dot, eye, label, tint: RefCell::default() };
        chip.update(info);
        chip
    }

    pub fn button(&self) -> &gtk::Button {
        &self.button
    }

    pub fn update(&self, info: &ChipInfo) {
        let color = if info.private { "#ff3d81" } else { safe_color(&info.color) };
        let class = tint_class(color);
        let old = self.tint.replace(class.clone());
        if !old.is_empty() {
            self.button.remove_css_class(&old);
        }
        self.button.add_css_class(&class);
        self.dot.set_visible(!info.private);
        self.eye.set_visible(info.private);
        self.label.set_label(if info.private { "Private" } else { &info.name });
    }
}

thread_local! {
    /// The chips' colours so far, and the one stylesheet that has them all.
    static TINTS: RefCell<(Vec<String>, Option<gtk::CssProvider>)> = RefCell::default();
}

/// The CSS class for chips of `color` (`wb-pc-7fd3ff`). GTK has no
/// per-widget styles: one stylesheet holds a rule set per colour, made again
/// with the new one when a colour is first seen.
fn tint_class(color: &str) -> String {
    let hex = color.trim_start_matches('#').to_ascii_lowercase();
    let class = format!("wb-pc-{hex}");
    TINTS.with_borrow_mut(|(seen, provider)| {
        if seen.contains(&hex) {
            return;
        }
        seen.push(hex.clone());
        let Some(display) = gtk::gdk::Display::default() else { return };
        let p = provider.get_or_insert_with(|| {
            let p = gtk::CssProvider::new();
            gtk::style_context_add_provider_for_display(&display, &p, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
            p
        });
        p.load_from_string(&seen.iter().map(|h| rules(h)).collect::<String>());
    });
    class
}

/// A colour's rules.
fn rules(hex: &str) -> String {
    let class = format!("wb-pc-{hex}");
    format!(
        "button.wb-chip.{class} {{ border-color: alpha(#{hex}, 0.45); background-color: alpha(#{hex}, 0.14); }}
button.wb-chip.{class}:hover {{ background-color: alpha(#{hex}, 0.22); }}
button.wb-chip.{class} .wb-chip-dot {{ background-color: #{hex}; }}
button.wb-chip.{class} .wb-chip-eye {{ color: #{hex}; }}
"
    )
}
