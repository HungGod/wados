//! The bar's little pieces: a bar's item (`BarItem.svelte`: its icon, its
//! name under it), tray buttons, glyphs, flyouts and menus (popovers of our
//! own glass), and starting a machine app.

use std::rc::Rc;

use gtk::{gdk, glib, prelude::*};
use wad_gtk::Icon;

/// A bar button with an icon (the page arrow, search, the tray's).
pub fn tb(icon: &'static str, size: i32, tip: &str) -> (gtk::Button, Icon) {
    let i = Icon::new(icon, size);
    let b = gtk::Button::builder().child(&i).tooltip_text(tip).css_classes(["wb-tb"]).build();
    (b, i)
}

/// A glyph in a rounded square (an app with no picture of its own).
pub fn glyph(icon: &'static str, small: bool) -> gtk::Box {
    let b = gtk::Box::builder()
        .css_classes(["wb-glyph"])
        .homogeneous(true)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    if small {
        b.add_css_class("wb-small");
    }
    b.append(&Icon::new(icon, if small { 15 } else { 17 }));
    b
}

/// A label, its CSS classes, left-aligned.
pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).build();
    for c in classes {
        l.add_css_class(c);
    }
    l
}

/// What isn't here yet: said plainly, without a promise it can't keep.
pub fn later(text: &str) -> gtk::Label {
    let l = label(text, &["wb-later"]);
    l.set_wrap(true);
    l.set_max_width_chars(40);
    l
}

pub fn set_class(w: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        w.add_css_class(class);
    } else {
        w.remove_css_class(class);
    }
}

/// `w`'s middle in its surface (for `wadsctl bar where`).
pub fn middle_of(w: &impl IsA<gtk::Widget>) -> Option<(f64, f64)> {
    let root = w.as_ref().root()?;
    let b = w.as_ref().compute_bounds(&root)?;
    Some(((b.x() + b.width() / 2.0) as f64, (b.y() + b.height() / 2.0) as f64))
}

/// Right-clicks on `w` call `f` (with where, in `w`).
pub fn on_right_click(w: &impl IsA<gtk::Widget>, f: impl Fn(f64, f64) + 'static) {
    let g = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
    g.connect_pressed(move |g, _, x, y| {
        g.set_state(gtk::EventSequenceState::Claimed);
        f(x, y);
    });
    w.add_controller(g);
}

// ------------------------------------------------------------ a bar's item

/// One thing in a bar (a wadspace, a window, a machine app): its icon, its
/// name under it cut short; `compact`, its icon alone (the name in its
/// tooltip).
pub struct Item {
    pub button: gtk::Button,
    /// Over the icon: the open line, a display's number.
    pub over: gtk::Overlay,
    name: gtk::Label,
    compact: std::cell::Cell<bool>,
    compact_line: gtk::Box,
}

impl Item {
    pub fn new(name: &str, icon: &impl IsA<gtk::Widget>) -> Self {
        let button = gtk::Button::builder().css_classes(["wb-item"]).build();
        let col = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        let over = gtk::Overlay::builder().halign(gtk::Align::Center).build();
        over.set_size_request(34, 38);
        let icon = icon.as_ref();
        icon.set_valign(gtk::Align::Start);
        icon.set_halign(gtk::Align::Center);
        icon.set_margin_top(1);
        over.set_child(Some(icon));
        col.append(&over);
        let name = gtk::Label::builder()
            .label(name)
            .css_classes(["wb-item-name"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(10)
            .width_chars(1)
            .build();
        col.append(&name);
        let compact_line = gtk::Box::builder()
            .css_classes(["wb-compact-line"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::End)
            .visible(false)
            .build();
        over.add_overlay(&compact_line);
        button.set_child(Some(&col));
        let me = Self { button, over, name, compact: Default::default(), compact_line };
        me.set_name(&me.name.label());
        me
    }

    pub fn set_name(&self, name: &str) {
        self.name.set_label(name);
        self.button.set_tooltip_text(Some(name));
    }

    pub fn set_active(&self, on: bool) {
        set_class(&self.button, "wb-active", on);
        self.compact_line.set_visible(on && self.compact.get());
    }

    pub fn set_dim(&self, on: bool) {
        set_class(&self.button, "wb-dim", on);
    }

    /// Its icon alone (short of room).
    pub fn set_compact(&self, on: bool) {
        self.compact.set(on);
        set_class(&self.button, "wb-compact", on);
        self.name.set_visible(!on);
        self.over.set_size_request(if on { 28 } else { 34 }, if on { 34 } else { 38 });
        self.compact_line.set_visible(on && self.button.has_css_class("wb-active"));
    }
}

/// A display's number on an item (it's on another display).
pub fn display_number(n: usize) -> gtk::Label {
    gtk::Label::builder()
        .label(n.to_string())
        .css_classes(["wb-on-n"])
        .halign(gtk::Align::End)
        .valign(gtk::Align::End)
        .build()
}

// ------------------------------------------------------------ flyouts and menus

thread_local! {
    /// Told when a flyout or menu closes (`on_closed`).
    static CLOSED: std::cell::RefCell<Option<Rc<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

/// `f` each time a flyout or menu that took the keys closes (the bar has
/// none of its own, so they go back where they belong).
pub fn on_closed(f: impl Fn() + 'static) {
    CLOSED.with(|c| *c.borrow_mut() = Some(Rc::new(f)));
}

/// A popover of our glass (`class`: wb-pop, wb-menu, wb-peek…), parented to
/// `parent` (it must live as long as it does).
pub fn popover(parent: &impl IsA<gtk::Widget>, classes: &[&str], below: bool) -> gtk::Popover {
    let p = gtk::Popover::builder()
        .has_arrow(false)
        .autohide(true)
        .position(if below { gtk::PositionType::Bottom } else { gtk::PositionType::Top })
        .build();
    p.add_css_class("wb-pop");
    for c in classes {
        p.add_css_class(c);
    }
    p.set_offset(0, if below { 8 } else { -8 });
    // What's in it may expand: that mustn't make what it's by expand too.
    p.set_hexpand(false);
    p.set_vexpand(false);
    p.set_parent(parent);
    p.connect_closed(|p| {
        // A peek (no autohide) never had the keys.
        if p.is_autohide()
            && let Some(f) = CLOSED.with(|c| c.borrow().clone())
        {
            f();
        }
    });
    p
}

/// Points `p` at `at` (a widget inside its parent).
pub fn point_at(p: &gtk::Popover, at: &impl IsA<gtk::Widget>) {
    let Some(parent) = p.parent() else { return };
    if let Some(b) = at.compute_bounds(&parent) {
        p.set_pointing_to(Some(&gdk::Rectangle::new(
            b.x() as i32,
            b.y() as i32,
            b.width().max(1.0) as i32,
            b.height().max(1.0) as i32,
        )));
    }
}

/// Points `p` (parented to `button`) at `button`'s width along `half`'s
/// edge: a flyout opens `gap` px from the half, not the button (`Flyout.svelte`).
pub fn aim(p: &gtk::Popover, button: &impl IsA<gtk::Widget>, half: Option<gtk::Widget>, below: bool, gap: i32) {
    let button = button.as_ref();
    let Some(h) = half.and_then(|h| h.compute_bounds(button)) else { return };
    p.set_pointing_to(Some(&gdk::Rectangle::new(0, h.y() as i32, button.width().max(1), h.height() as i32)));
    p.set_offset(0, if below { gap } else { -gap });
}

/// One line of a menu.
pub enum Entry {
    Item { label: String, icon: &'static str, checked: bool, danger: bool, enabled: bool, run: Rc<dyn Fn()> },
    Sep,
}

pub fn entry(label: &str, icon: &'static str, run: impl Fn() + 'static) -> Entry {
    Entry::Item { label: label.into(), icon, checked: false, danger: false, enabled: true, run: Rc::new(run) }
}

impl Entry {
    pub fn checked(mut self, on: bool) -> Self {
        if let Entry::Item { checked, .. } = &mut self {
            *checked = on;
        }
        self
    }

    pub fn danger(mut self) -> Self {
        if let Entry::Item { danger, .. } = &mut self {
            *danger = true;
        }
        self
    }
}

/// A menu by `at` (inside `parent`, which outlives it): gone when it closes.
pub fn menu(
    parent: &impl IsA<gtk::Widget>,
    at: &impl IsA<gtk::Widget>,
    entries: Vec<Entry>,
    below: bool,
) -> gtk::Popover {
    let p = popover(parent, &["wb-menu"], below);
    let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    for e in entries {
        match e {
            Entry::Sep => col.append(&gtk::Box::builder().css_classes(["wb-menu-sep"]).build()),
            Entry::Item { label, icon, checked, danger, enabled, run } => {
                let row = gtk::Box::builder().spacing(10).build();
                let i = Icon::new(icon, 16);
                row.append(&i);
                let l = gtk::Label::builder().label(&label).xalign(0.0).hexpand(true).build();
                row.append(&l);
                if checked {
                    row.append(&Icon::new("check", 14));
                }
                let b = gtk::Button::builder().child(&row).css_classes(["wb-menu-item"]).sensitive(enabled).build();
                if danger {
                    b.add_css_class("wb-danger");
                }
                let pw = p.downgrade();
                b.connect_clicked(move |_| {
                    if let Some(p) = pw.upgrade() {
                        p.popdown();
                    }
                    run();
                });
                col.append(&b);
            }
        }
    }
    p.set_child(Some(&col));
    point_at(&p, at);
    p.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    p.popup();
    p
}

// ------------------------------------------------------------ starting things

/// Starts `argv` if its program is on the machine (true), reaped by GLib.
pub fn spawn(argv: &[&str]) -> bool {
    let Some(prog) = argv.first() else { return false };
    if glib::find_program_in_path(prog).is_none() {
        return false;
    }
    let args: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    match gtk::gio::Subprocess::newv(&args, gtk::gio::SubprocessFlags::NONE) {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!("{prog}: {e}");
            false
        }
    }
}

/// The tailnet's mark (Tailscale's nine dots, the T lit), in the accent.
pub const TAILSCALE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#c6ff1f"><circle cx="4" cy="4" r="2.9" opacity="0.3"/><circle cx="12" cy="4" r="2.9" opacity="0.3"/><circle cx="20" cy="4" r="2.9" opacity="0.3"/><circle cx="4" cy="12" r="2.9"/><circle cx="12" cy="12" r="2.9"/><circle cx="20" cy="12" r="2.9"/><circle cx="4" cy="20" r="2.9" opacity="0.3"/><circle cx="12" cy="20" r="2.9"/><circle cx="20" cy="20" r="2.9" opacity="0.3"/></svg>"##;
