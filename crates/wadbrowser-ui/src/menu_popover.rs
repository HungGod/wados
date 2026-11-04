//! A menu, drawn as the spike draws one (`ui/MenuList.svelte`, its shot 66):
//! a popover of rows, each an icon (a check when it's on), a label, a hint or
//! why it's off under it, and a shortcut; headings and separators; a submenu
//! beside its row, opened by hovering it (150 ms), clicking it, or →.
//!
//! Copied from the shell's Builder menu (apps/wados-shell/src/builder/board/
//! menu.rs), which is tied to the Builder's pieces: the two (and the
//! desktop's) belong in wad-gtk once they share one `Entry`.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::{gdk, glib, prelude::*};

use crate::menus::{Entry, Item, find};

/// Where a menu opens.
#[derive(Debug, Clone, Copy)]
pub enum Place {
    /// Its top left at this point of its parent (a right-click).
    Point(f64, f64),
    /// Under its parent (a header bar button), their right edges lined up.
    Below,
}

/// A submenu open beside its row.
struct Sub {
    id: String,
    popover: gtk::Popover,
    row: glib::WeakRef<gtk::Button>,
}

pub struct MenuPopover {
    me: Weak<MenuPopover>,
    popover: gtk::Popover,
    entries: Vec<Entry>,
    autohide: bool,
    sub: RefCell<Option<Sub>>,
    /// Hover intent: the row last entered (its submenu opens, or the open one closes, after a moment there).
    hover: Cell<u64>,
}

impl MenuPopover {
    /// `entries` in a popover on `parent`, at `place`. `autohide`: a click
    /// elsewhere closes it (one opened without a click, as the gallery's,
    /// has nothing to grab with, so it stays till it's picked from).
    pub fn show(parent: &impl IsA<gtk::Widget>, place: Place, entries: Vec<Entry>, autohide: bool) -> Rc<MenuPopover> {
        let popover = popover(autohide);
        popover.set_parent(parent);
        match place {
            Place::Point(x, y) => {
                popover.set_pointing_to(Some(&gdk::Rectangle::new(x.round() as i32, y.round() as i32, 1, 1)));
            }
            Place::Below => {
                let p = parent.as_ref();
                popover.set_pointing_to(Some(&gdk::Rectangle::new(0, 0, p.width().max(1), p.height().max(1))));
                popover.set_halign(gtk::Align::End);
                popover.set_offset(0, 6);
            }
        }
        let me = Rc::new_cyclic(|me| MenuPopover {
            me: me.clone(),
            popover: popover.clone(),
            entries,
            autohide,
            sub: RefCell::default(),
            hover: Cell::new(0),
        });
        popover.set_child(Some(&me.list(&me.entries, true)));
        let w = me.me.clone();
        popover.connect_closed(move |p| {
            if let Some(m) = w.upgrade() {
                m.close_sub();
            }
            let p = p.clone();
            // Unparented once it's done closing.
            glib::idle_add_local_once(move || p.unparent());
        });
        // It lives as long as it's open (the caller needn't keep it).
        let keep = RefCell::new(Some(me.clone()));
        popover.connect_closed(move |_| {
            keep.borrow_mut().take();
        });
        popover.popup();
        me
    }

    pub fn popover(&self) -> &gtk::Popover {
        &self.popover
    }

    /// Closes it (and its submenu).
    pub fn close(&self) {
        self.close_sub();
        self.popover.popdown();
    }

    /// Opens item `id`'s submenu, or runs it (the gallery, tests). False if
    /// there's no such item, or it's off.
    pub fn open(&self, id: &str) -> bool {
        let Some(it) = find(&self.entries, id).cloned() else { return false };
        if !it.enabled() {
            return false;
        }
        if it.submenu.is_empty() {
            self.run(&it);
            return true;
        }
        match find_row(&self.popover, id) {
            Some(row) => {
                self.open_sub(&it, &row);
                true
            }
            None => false,
        }
    }

    // ------------------------------------------------------------ drawing

    fn list(&self, entries: &[Entry], top: bool) -> gtk::Box {
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["wb-menu-list"]).build();
        for e in entries {
            match e {
                Entry::Heading(h) => list.append(
                    &gtk::Label::builder()
                        .label(h.to_uppercase())
                        .xalign(0.0)
                        .wrap(true)
                        .max_width_chars(30)
                        .css_classes(["wb-menu-heading"])
                        .build(),
                ),
                Entry::Sep => list.append(&gtk::Box::builder().css_classes(["wb-menu-sep"]).build()),
                Entry::Item(it) => list.append(&self.row(it, top)),
            }
        }
        list
    }

    fn row(&self, it: &Item, top: bool) -> gtk::Button {
        let b = gtk::Button::builder().css_classes(["wb-menu-item"]).halign(gtk::Align::Fill).build();
        if it.danger {
            b.add_css_class("danger");
        }
        if it.checked {
            b.add_css_class("checked");
        }
        b.set_widget_name(&it.id);
        let h = gtk::Box::builder().spacing(10).hexpand(true).halign(gtk::Align::Fill).build();
        let ic = gtk::Box::builder().css_classes(["wb-menu-ic"]).valign(gtk::Align::Center).hexpand(false).build();
        ic.set_size_request(16, 16);
        let name: &'static str = if it.checked { "check" } else { it.icon };
        if !name.is_empty() {
            let i = wad_gtk::Icon::new(name, 15);
            i.set_halign(gtk::Align::Center);
            i.set_hexpand(true);
            ic.append(&i);
        }
        h.append(&ic);
        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .hexpand(true)
            .halign(gtk::Align::Fill)
            .valign(gtk::Align::Center)
            .build();
        text.append(
            &gtk::Label::builder()
                .label(&it.label)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .max_width_chars(40)
                .css_classes(["wb-menu-label"])
                .build(),
        );
        let under = it.disabled.as_deref().filter(|w| !w.is_empty()).or(it.hint.as_deref());
        if let Some(note) = under {
            text.append(
                &gtk::Label::builder()
                    .label(note)
                    .xalign(0.0)
                    .wrap(true)
                    .max_width_chars(30)
                    .css_classes(["wb-menu-hint"])
                    .build(),
            );
        }
        h.append(&text);
        if let Some(k) = &it.shortcut {
            h.append(&gtk::Label::builder().label(k).css_classes(["wb-menu-kbd"]).valign(gtk::Align::Center).build());
        }
        if !it.submenu.is_empty() {
            let chev = wad_gtk::Icon::new("chevron-right", 14);
            chev.set_valign(gtk::Align::Center);
            chev.add_css_class("wb-menu-chev");
            h.append(&chev);
        }
        b.set_child(Some(&h));
        if let Some(why) = it.disabled.as_deref() {
            b.set_sensitive(false);
            if !why.is_empty() {
                b.set_tooltip_text(Some(why));
            }
        }

        let item = it.clone();
        let w = self.me.clone();
        b.connect_clicked(move |b| {
            let Some(m) = w.upgrade() else { return };
            if item.submenu.is_empty() {
                m.run(&item);
            } else {
                m.open_sub(&item, b);
            }
        });
        if !it.submenu.is_empty() {
            // → opens it.
            let keys = gtk::EventControllerKey::new();
            let (item, w, row) = (it.clone(), self.me.clone(), b.downgrade());
            keys.connect_key_pressed(move |_, key, _, _| {
                if key != gdk::Key::Right {
                    return glib::Propagation::Proceed;
                }
                if let (Some(m), Some(row)) = (w.upgrade(), row.upgrade()) {
                    m.open_sub(&item, &row);
                    if let Some(s) = m.sub.borrow().as_ref() {
                        s.popover.child_focus(gtk::DirectionType::TabForward);
                    }
                }
                glib::Propagation::Stop
            });
            b.add_controller(keys);
        }
        // Hover intent: a submenu opens after a moment on its row; another row of this level closes it.
        if top {
            let motion = gtk::EventControllerMotion::new();
            let (item, w, row) = (it.clone(), self.me.clone(), b.downgrade());
            motion.connect_enter(move |_, _, _| {
                let Some(m) = w.upgrade() else { return };
                let n = m.hover.get() + 1;
                m.hover.set(n);
                let (item, w, row) = (item.clone(), w.clone(), row.clone());
                glib::timeout_add_local_once(Duration::from_millis(150), move || {
                    let (Some(m), Some(row)) = (w.upgrade(), row.upgrade()) else { return };
                    if m.hover.get() != n {
                        return;
                    }
                    if item.submenu.is_empty() || !item.enabled() {
                        m.close_sub();
                    } else {
                        m.open_sub(&item, &row);
                    }
                });
            });
            b.add_controller(motion);
        }
        b
    }

    fn open_sub(&self, it: &Item, row: &gtk::Button) {
        if self.sub.borrow().as_ref().is_some_and(|s| s.id == it.id) {
            return;
        }
        self.close_sub();
        let p = popover(self.autohide);
        p.set_position(gtk::PositionType::Right);
        p.set_valign(gtk::Align::Start);
        // Its first row level with this one, overlapping this menu by 4 px (MenuList's subAt).
        p.set_offset(3, -7);
        p.set_parent(row);
        p.set_child(Some(&self.list(&it.submenu, false)));
        // ← and Escape go back to this menu.
        let keys = gtk::EventControllerKey::new();
        let w = self.me.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key != gdk::Key::Left && key != gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            if let Some(m) = w.upgrade() {
                let row = m.sub.borrow().as_ref().and_then(|s| s.row.upgrade());
                m.close_sub();
                if let Some(r) = row {
                    r.grab_focus();
                }
            }
            glib::Propagation::Stop
        });
        p.add_controller(keys);
        p.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        p.popup();
        row.add_css_class("open");
        *self.sub.borrow_mut() = Some(Sub { id: it.id.clone(), popover: p, row: row.downgrade() });
    }

    fn close_sub(&self) {
        let sub = self.sub.borrow_mut().take();
        if let Some(s) = sub {
            if let Some(row) = s.row.upgrade() {
                row.remove_css_class("open");
            }
            s.popover.popdown();
        }
    }

    fn run(&self, it: &Item) {
        let Some(run) = it.run.clone() else { return };
        self.close();
        // After the menu's gone (a run may open a dialog, or another menu).
        glib::idle_add_local_once(move || run());
    }
}

/// A popover's row for item `id`.
fn find_row(p: &gtk::Popover, id: &str) -> Option<gtk::Button> {
    let list = p.child()?;
    let mut c = list.first_child();
    while let Some(w) = c {
        if w.widget_name() == id
            && let Ok(b) = w.clone().downcast::<gtk::Button>()
        {
            return Some(b);
        }
        c = w.next_sibling();
    }
    None
}

fn popover(autohide: bool) -> gtk::Popover {
    gtk::Popover::builder()
        .has_arrow(false)
        .autohide(autohide)
        .position(gtk::PositionType::Bottom)
        .halign(gtk::Align::Start)
        .css_classes(["wb-menu"])
        .build()
}
