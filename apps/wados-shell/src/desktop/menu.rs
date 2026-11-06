//! The desktop's right-click menus (the spike's `ui/MenuList.svelte` drawing
//! `shell/common/menus.ts`' items): rows with an icon, a label, a hint or
//! why it's off under it, and a shortcut; headings and separators. Menus are
//! data ([`Entry`]) so `wadsctl desk menu` can list and pick from them.

use std::rc::Rc;

use gtk::{gdk, prelude::*};

pub type Run = Rc<dyn Fn()>;

thread_local! {
    static IDS: std::cell::RefCell<std::collections::HashSet<&'static str>> = std::cell::RefCell::default();
}

/// An item's id made at run time (Open with's `open-with:<app>:<ws>`), kept
/// for the session: each one once.
pub fn static_id(id: &str) -> &'static str {
    IDS.with(|ids| {
        let mut ids = ids.borrow_mut();
        if let Some(&s) = ids.get(id) {
            return s;
        }
        let s: &'static str = Box::leak(id.to_string().into_boxed_str());
        ids.insert(s);
        s
    })
}

pub enum Entry {
    Heading(String),
    Sep,
    Item(Item),
}

pub struct Item {
    /// A test hook (`wadsctl desk pick <id>`).
    pub id: &'static str,
    pub label: String,
    pub icon: &'static str,
    pub shortcut: Option<&'static str>,
    pub hint: Option<String>,
    /// Off, and why.
    pub disabled: Option<String>,
    pub danger: bool,
    pub checked: bool,
    pub run: Option<Run>,
}

impl Item {
    pub fn new(id: &'static str, label: impl Into<String>, icon: &'static str, run: impl Fn() + 'static) -> Self {
        Self {
            id,
            label: label.into(),
            icon,
            shortcut: None,
            hint: None,
            disabled: None,
            danger: false,
            checked: false,
            run: Some(Rc::new(run)),
        }
    }

    pub fn key(mut self, k: &'static str) -> Self {
        self.shortcut = Some(k);
        self
    }

    pub fn hint(mut self, h: impl Into<String>) -> Self {
        self.hint = Some(h.into());
        self
    }

    /// Off (with why), if `why` is Some.
    pub fn off(mut self, why: Option<impl Into<String>>) -> Self {
        self.disabled = why.map(Into::into);
        self
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }

    pub fn entry(self) -> Entry {
        Entry::Item(self)
    }
}

/// A menu open: its popover and what's on it.
pub struct Open {
    pub popover: gtk::Popover,
    pub items: Vec<(&'static str, String, bool, Option<Run>)>,
}

/// `entries` in a popover on `parent` at (x, y) (its pixels). `picked` is
/// told before an item runs; `closed` when it goes (either way). `autohide`:
/// a click elsewhere closes it (a menu opened without a click, by wadsctl,
/// has nothing to grab with, so it stays until it's picked from or closed).
pub fn show(
    parent: &impl IsA<gtk::Widget>,
    x: f64,
    y: f64,
    entries: Vec<Entry>,
    autohide: bool,
    closed: impl Fn() + 'static,
) -> Open {
    let popover = gtk::Popover::builder()
        .has_arrow(false)
        .autohide(autohide)
        .position(gtk::PositionType::Bottom)
        .halign(gtk::Align::Start)
        .css_classes(["desk-menu"])
        .build();
    popover.set_parent(parent);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x.round() as i32, y.round() as i32, 1, 1)));
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["desk-menu-list"]).build();
    let mut items = Vec::new();
    for e in entries {
        match e {
            Entry::Heading(h) => {
                let l = gtk::Label::builder()
                    .label(h.to_uppercase())
                    .xalign(0.0)
                    .css_classes(["desk-menu-heading"])
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .max_width_chars(30)
                    .build();
                list.append(&l);
            }
            Entry::Sep => list.append(&gtk::Box::builder().css_classes(["desk-menu-sep"]).build()),
            Entry::Item(it) => {
                let b = row(&it);
                let enabled = it.disabled.is_none();
                b.set_sensitive(enabled);
                let (run, pop) = (it.run.clone(), popover.downgrade());
                b.connect_clicked(move |_| {
                    if let Some(p) = pop.upgrade() {
                        p.popdown();
                    }
                    if let Some(r) = &run {
                        // After the popover's gone (a run may open another).
                        let r = r.clone();
                        gtk::glib::idle_add_local_once(move || r());
                    }
                });
                list.append(&b);
                items.push((it.id, it.label, enabled, it.run));
            }
        }
    }
    popover.set_child(Some(&list));
    popover.connect_closed(move |p| {
        closed();
        let p = p.clone();
        // Unparented once it's done closing.
        gtk::glib::idle_add_local_once(move || p.unparent());
    });
    popover.popup();
    Open { popover, items }
}

fn row(it: &Item) -> gtk::Button {
    let b = gtk::Button::builder().css_classes(["desk-menu-item"]).halign(gtk::Align::Fill).build();
    if it.danger {
        b.add_css_class("danger");
    }
    b.set_widget_name(it.id);
    let h = gtk::Box::builder().spacing(10).hexpand(true).halign(gtk::Align::Fill).build();
    let ic = gtk::Box::builder().css_classes(["desk-menu-ic"]).valign(gtk::Align::Center).build();
    ic.set_size_request(16, 16);
    let name: &'static str = if it.checked { "check" } else { it.icon };
    if !name.is_empty() {
        let i = wad_gtk::Icon::new(name, 15);
        i.set_halign(gtk::Align::Center);
        ic.append(&i);
    }
    h.append(&ic);
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Center)
        .build();
    text.append(&gtk::Label::builder().label(&it.label).xalign(0.0).css_classes(["desk-menu-label"]).build());
    if let Some(why) = it.disabled.as_ref().or(it.hint.as_ref()) {
        text.append(
            &gtk::Label::builder()
                .label(why)
                .xalign(0.0)
                .wrap(true)
                .max_width_chars(28)
                .css_classes(["desk-menu-hint"])
                .build(),
        );
    }
    h.append(&text);
    if let Some(k) = it.shortcut {
        h.append(&gtk::Label::builder().label(k).css_classes(["desk-menu-kbd"]).valign(gtk::Align::Center).build());
    }
    b.set_child(Some(&h));
    b
}
