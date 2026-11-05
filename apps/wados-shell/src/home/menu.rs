//! The launcher's menus (the spike's MenuList): a heading, rows of an icon,
//! a name and what it does, lines between groups; one that isn't here yet
//! dimmed, its hint saying when (or why not); a submenu beside its row,
//! opened by a click or a moment's hover (Stream ▸). The machine's list and
//! a card's right-click menu (`shell/common/menus.ts`'s wadspaceMenu).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::{glib, prelude::*};

/// One line of a menu.
#[derive(Clone)]
pub enum Item {
    Heading(String),
    Row(Row),
    Line,
}

/// A row: its icon, its name, a hint under it, ticked, in red, or not here yet; a submenu of its own.
#[derive(Clone)]
pub struct Row {
    icon: &'static str,
    label: String,
    hint: Option<String>,
    checked: bool,
    danger: bool,
    enabled: bool,
    run: Rc<dyn Fn()>,
    submenu: Vec<Item>,
}

/// A submenu's row: the menu it's in, its name, and what opens it.
type Sub = (glib::WeakRef<gtk::Popover>, String, Rc<dyn Fn()>);

thread_local! {
    /// The submenus of the menus open, by their row's name: what opens each (`wadsctl home menu sub <name>`).
    static SUBS: RefCell<Vec<Sub>> = RefCell::default();
}

/// Opens the submenu of the open menu's row named `label`.
pub fn open_sub(label: &str) -> bool {
    let f = SUBS.with(|s| {
        let mut s = s.borrow_mut();
        s.retain(|(p, _, _)| p.upgrade().is_some_and(|p| p.is_visible()));
        s.iter().find(|(_, l, _)| l.eq_ignore_ascii_case(label)).map(|(_, _, f)| f.clone())
    });
    match f {
        Some(f) => {
            f();
            true
        }
        None => false,
    }
}

impl Row {
    pub fn new(icon: &'static str, label: impl Into<String>, run: impl Fn() + 'static) -> Self {
        Row {
            icon,
            label: label.into(),
            hint: None,
            checked: false,
            danger: false,
            enabled: true,
            run: Rc::new(run),
            submenu: vec![],
        }
    }

    /// Its own menu, beside it.
    pub fn submenu(self, items: Vec<Item>) -> Self {
        Row { submenu: items, ..self }
    }

    /// Ticked, if `on`.
    pub fn checked_if(self, on: bool) -> Self {
        Row { checked: on, ..self }
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
    let sub: Rc<RefCell<Option<gtk::Popover>>> = Rc::default();
    let s = sub.clone();
    menu.connect_closed(move |_| {
        if let Some(p) = s.borrow_mut().take() {
            p.popdown();
        }
    });
    menu.set_child(Some(&list(&menu, items, &sub)));
    menu.set_parent(parent);
    menu
}

/// The rows of `items`; a row picked closes `menu` (the one it's in, and the one that's in).
fn list(menu: &gtk::Popover, items: Vec<Item>, sub: &Rc<RefCell<Option<gtk::Popover>>>) -> gtk::Box {
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
            Item::Row(Row { icon, label, hint, checked, danger, enabled, run, submenu }) => {
                let row = gtk::Box::builder().spacing(10).css_classes(["home-menu-item"]).build();
                row.set_widget_name(&label);
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
                // Hovering another row of this menu closes its submenu; one with its own opens it.
                let motion = gtk::EventControllerMotion::new();
                let hover: Rc<std::cell::Cell<u64>> = Rc::default();
                if !submenu.is_empty() && enabled {
                    row.append(&wad_gtk::Icon::new("chevron-right", 14));
                    let (m, s, r, items) = (menu.downgrade(), sub.clone(), row.downgrade(), submenu.clone());
                    let open: Rc<dyn Fn()> = Rc::new(move || {
                        let (Some(m), Some(r)) = (m.upgrade(), r.upgrade()) else { return };
                        open_submenu(&m, &r, items.clone(), &s);
                    });
                    SUBS.with(|x| x.borrow_mut().push((menu.downgrade(), label.clone(), open.clone())));
                    let o = open.clone();
                    let click = gtk::GestureClick::new();
                    click.connect_released(move |g, _, _, _| {
                        g.set_state(gtk::EventSequenceState::Claimed);
                        o();
                    });
                    row.add_controller(click);
                    let h = hover.clone();
                    motion.connect_enter(move |_, _, _| {
                        let n = h.get() + 1;
                        h.set(n);
                        let (h, o) = (h.clone(), open.clone());
                        glib::timeout_add_local_once(Duration::from_millis(150), move || {
                            if h.get() == n {
                                o();
                            }
                        });
                    });
                } else {
                    let s = sub.clone();
                    motion.connect_enter(move |_, _, _| {
                        if let Some(p) = s.borrow_mut().take() {
                            p.popdown();
                        }
                    });
                    if enabled {
                        let click = gtk::GestureClick::new();
                        let m = menu.downgrade();
                        click.connect_released(move |g, _, _, _| {
                            g.set_state(gtk::EventSequenceState::Claimed);
                            if let Some(m) = m.upgrade() {
                                // Picked: a submenu closes the menu it's beside too.
                                if let Some(c) = m.child() {
                                    c.add_css_class("picked");
                                }
                                m.popdown();
                            }
                            // After the menu's gone (a run may open another, or a sheet).
                            let run = run.clone();
                            glib::idle_add_local_once(move || run());
                        });
                        row.add_controller(click);
                    }
                }
                let h = hover.clone();
                motion.connect_leave(move |_| h.set(h.get() + 1));
                row.add_controller(motion);
                col.append(&row);
            }
        }
    }
    col
}

/// `items` in a menu of their own beside `row` (of `menu`); picking one closes both.
fn open_submenu(menu: &gtk::Popover, row: &gtk::Box, items: Vec<Item>, sub: &Rc<RefCell<Option<gtk::Popover>>>) {
    if sub.borrow().as_ref().is_some_and(|p| p.parent().as_ref() == Some(row.upcast_ref()) && p.is_visible()) {
        return;
    }
    if let Some(p) = sub.borrow_mut().take() {
        p.popdown();
    }
    let p =
        gtk::Popover::builder().has_arrow(false).position(gtk::PositionType::Right).css_classes(["home-menu"]).build();
    // Its rows close the menu it's beside too.
    let inner: Rc<RefCell<Option<gtk::Popover>>> = Rc::default();
    let col = list(&p, items, &inner);
    p.set_child(Some(&col));
    let m = menu.downgrade();
    p.connect_closed(move |p| {
        // A row picked in it (not a click elsewhere in the menu) closes the menu too.
        if p.child().is_some_and(|c| c.has_css_class("picked"))
            && let Some(m) = m.upgrade()
        {
            m.popdown();
        }
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    p.set_parent(row);
    row.add_css_class("open");
    let r = row.downgrade();
    p.connect_hide(move |_| {
        if let Some(r) = r.upgrade() {
            r.remove_css_class("open");
        }
    });
    p.popup();
    *sub.borrow_mut() = Some(p);
}
