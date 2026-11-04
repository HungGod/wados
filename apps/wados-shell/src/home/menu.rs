//! The launcher's menus (the spike's MenuList): a heading, rows of an icon,
//! a name and what it does, lines between groups; one that isn't here yet
//! dimmed, its hint saying when. The machine's list and a card's
//! right-click menu (`shell/common/menus.ts`'s wadspaceMenu, what works
//! today).

use std::rc::Rc;

use gtk::prelude::*;

/// One line of a menu.
pub enum Item {
    Heading(String),
    Row(Row),
    Line,
}

/// A row: its icon, its name, a hint under it, ticked, in red, or not here yet.
pub struct Row {
    icon: &'static str,
    label: String,
    hint: Option<String>,
    checked: bool,
    danger: bool,
    enabled: bool,
    run: Rc<dyn Fn()>,
}

impl Row {
    pub fn new(icon: &'static str, label: impl Into<String>, run: impl Fn() + 'static) -> Self {
        Row { icon, label: label.into(), hint: None, checked: false, danger: false, enabled: true, run: Rc::new(run) }
    }

    pub fn hint(self, hint: impl Into<String>) -> Self {
        Row { hint: Some(hint.into()), ..self }
    }

    pub fn checked(self) -> Self {
        Row { checked: true, ..self }
    }

    pub fn danger(self) -> Self {
        Row { danger: true, ..self }
    }

    /// Not here yet: dimmed, `when` under it.
    pub fn later(self, when: impl Into<String>) -> Self {
        Row { enabled: false, hint: Some(when.into()), ..self }
    }
}

/// A menu of `items`, under `parent`; a row picked runs it and closes the menu.
pub fn popover(parent: &impl IsA<gtk::Widget>, items: Vec<Item>) -> gtk::Popover {
    let menu = gtk::Popover::builder()
        .has_arrow(false)
        .position(gtk::PositionType::Bottom)
        .halign(gtk::Align::Start)
        .css_classes(["home-menu"])
        .build();
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    for item in items {
        match item {
            Item::Heading(text) => col.append(
                &gtk::Label::builder()
                    .label(text.to_uppercase())
                    .xalign(0.0)
                    .css_classes(["home-menu-heading"])
                    .build(),
            ),
            Item::Line => col.append(&gtk::Separator::builder().css_classes(["home-menu-line"]).build()),
            Item::Row(Row { icon, label, hint, checked, danger, enabled, run }) => {
                let row = gtk::Box::builder().spacing(10).css_classes(["home-menu-item"]).build();
                if danger {
                    row.add_css_class("danger");
                }
                if !enabled {
                    row.add_css_class("off");
                }
                row.append(&wad_gtk::Icon::new(icon, 16));
                let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
                text.append(&gtk::Label::builder().label(&label).xalign(0.0).build());
                if let Some(h) = &hint {
                    text.append(
                        &gtk::Label::builder()
                            .label(h)
                            .xalign(0.0)
                            .wrap(true)
                            .max_width_chars(28)
                            .css_classes(["home-menu-hint"])
                            .build(),
                    );
                }
                row.append(&text);
                if checked {
                    row.append(&wad_gtk::Icon::new("check", 16));
                }
                if enabled {
                    let click = gtk::GestureClick::new();
                    let m = menu.downgrade();
                    click.connect_released(move |g, _, _, _| {
                        g.set_state(gtk::EventSequenceState::Claimed);
                        if let Some(m) = m.upgrade() {
                            m.popdown();
                        }
                        run();
                    });
                    row.add_controller(click);
                }
                col.append(&row);
            }
        }
    }
    menu.set_child(Some(&col));
    menu.set_parent(parent);
    menu
}
