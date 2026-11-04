//! The browser's dialogs, in the spike's look (`ui/Dialog.svelte`, `Button`,
//! `TextField`): a sheet over the dimmed window with a tinted badge, its
//! title and ×, a body, and its buttons at the end. Each is a function of
//! the data it shows and what it does, presenting an `adw::Dialog` over
//! `parent` and returning it.
//!
//! - [`create_app`]: "Create app from this page", then "<name> is an app".
//! - [`new_profile`]: a name and a colour.
//! - [`signins`]: a new sign-in for this site, or every saved one's sites.
//! - [`permission`]: a site asking for the camera, a location, …

pub mod create_app;
pub mod new_profile;
pub mod permission;
pub mod signins;

use adw::prelude::*;

/// A dialog's badge colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Accent,
    Magenta,
    Warn,
    Danger,
}

impl Tone {
    fn class(self) -> &'static str {
        match self {
            Tone::Accent => "accent",
            Tone::Magenta => "magenta",
            Tone::Warn => "warn",
            Tone::Danger => "danger",
        }
    }
}

/// A button's look (`ui/Button.svelte`'s variants).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Primary,
    Secondary,
    Ghost,
    Danger,
    Quiet,
}

/// A button: `label`, with `icon` before it.
pub fn button(label: &str, variant: Variant, icon: Option<&'static str>) -> gtk::Button {
    let class = match variant {
        Variant::Primary => "primary",
        Variant::Secondary => "secondary",
        Variant::Ghost => "ghost",
        Variant::Danger => "danger",
        Variant::Quiet => "quiet",
    };
    let b = gtk::Button::builder().css_classes(["wb-btn", class]).build();
    let inner = gtk::Box::builder().spacing(8).halign(gtk::Align::Center).build();
    if let Some(i) = icon {
        inner.append(&wad_gtk::Icon::new(i, 16));
    }
    inner.append(&gtk::Label::new(Some(label)));
    b.set_child(Some(&inner));
    b
}

/// A dialog's frame: the sheet, its header (badge, title, ×), its body and
/// its footer of buttons.
pub(crate) struct Frame {
    pub dialog: adw::Dialog,
    pub body: gtk::Box,
    pub footer: gtk::Box,
    title: gtk::Label,
    badge_icon: wad_gtk::Icon,
}

impl Frame {
    pub fn new(title: &str, icon: &'static str, tone: Tone, width: i32) -> Frame {
        let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["wb-dialog-col"]).build();
        let header = gtk::Box::builder().spacing(12).css_classes(["wb-dialog-header"]).build();
        let badge_icon = wad_gtk::Icon::new(icon, 18);
        badge_icon.set_halign(gtk::Align::Center);
        badge_icon.set_hexpand(true);
        let badge = gtk::Box::builder()
            .css_classes(["wb-dialog-badge", tone.class()])
            .valign(gtk::Align::Center)
            .hexpand(false)
            .build();
        badge.set_size_request(36, 36);
        badge.append(&badge_icon);
        let title_label = gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .css_classes(["wb-dialog-title"])
            .build();
        let x = gtk::Button::builder()
            .child(&wad_gtk::Icon::new("x", 16))
            .css_classes(["wb-dialog-x"])
            .valign(gtk::Align::Center)
            .tooltip_text("Close")
            .build();
        header.append(&badge);
        header.append(&title_label);
        header.append(&x);
        col.append(&header);
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(14)
            .css_classes(["wb-dialog-body"])
            .build();
        col.append(&body);
        let footer = gtk::Box::builder().spacing(8).halign(gtk::Align::End).css_classes(["wb-dialog-footer"]).build();
        col.append(&footer);

        // As wide as asked, as tall as what's in it (a second step may be shorter).
        col.set_size_request(width, -1);
        let dialog = adw::Dialog::builder()
            .title(title)
            .follows_content_size(true)
            .presentation_mode(adw::DialogPresentationMode::Floating)
            .css_classes(["wb-dialog"])
            .child(&col)
            .build();
        let d = dialog.downgrade();
        x.connect_clicked(move |_| {
            if let Some(d) = d.upgrade() {
                d.close();
            }
        });
        Frame { dialog, body, footer, title: title_label, badge_icon }
    }

    /// A new title and badge (a dialog's second step).
    pub fn retitle(&self, title: &str, icon: &'static str) {
        self.title.set_label(title);
        self.dialog.set_title(title);
        self.badge_icon.set(icon);
    }

    /// A paragraph of the body.
    pub fn say(&self, text: &str) -> gtk::Label {
        let l = gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["wb-dialog-text"]).build();
        l.set_max_width_chars(1);
        self.body.append(&l);
        l
    }

    /// Clears the body and the footer (for the next step).
    pub fn clear(&self) {
        for b in [&self.body, &self.footer] {
            while let Some(c) = b.first_child() {
                b.remove(&c);
            }
        }
    }
}

/// A labelled text field (`ui/TextField.svelte`), with room for an error under it.
pub(crate) struct Field {
    pub root: gtk::Box,
    pub entry: gtk::Entry,
    error: gtk::Label,
}

impl Field {
    pub fn new(label: &str, placeholder: &str, text: &str, mono: bool) -> Field {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
        root.append(&gtk::Label::builder().label(label).xalign(0.0).css_classes(["wb-field-label"]).build());
        let entry = gtk::Entry::builder().placeholder_text(placeholder).text(text).css_classes(["wb-field"]).build();
        if mono {
            entry.add_css_class("mono");
        }
        entry.update_property(&[gtk::accessible::Property::Label(label)]);
        root.append(&entry);
        let error = gtk::Label::builder().xalign(0.0).wrap(true).visible(false).css_classes(["wb-field-error"]).build();
        root.append(&error);
        let (r, e) = (root.downgrade(), error.downgrade());
        entry.connect_changed(move |_| {
            if let (Some(r), Some(e)) = (r.upgrade(), e.upgrade()) {
                r.remove_css_class("has-error");
                e.set_visible(false);
            }
        });
        Field { root, entry, error }
    }

    /// Focused when its dialog opens, the caret at the end (GTK would select it all).
    pub fn focus_in(&self, dialog: &adw::Dialog) {
        dialog.set_focus(Some(&self.entry));
        let first = std::rc::Rc::new(std::cell::Cell::new(true));
        let focus = gtk::EventControllerFocus::new();
        let e = self.entry.downgrade();
        focus.connect_enter(move |_| {
            if !first.replace(false) {
                return;
            }
            let e = e.clone();
            gtk::glib::idle_add_local_once(move || {
                if let Some(e) = e.upgrade() {
                    e.set_position(-1);
                }
            });
        });
        self.entry.add_controller(focus);
    }

    pub fn text(&self) -> String {
        self.entry.text().to_string()
    }

    /// Says what's wrong with it, under it.
    pub fn error(&self, why: &str) {
        self.error.set_label(why);
        self.error.set_visible(true);
        self.root.add_css_class("has-error");
    }
}
