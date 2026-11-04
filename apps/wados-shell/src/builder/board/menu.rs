//! A piece's menu, drawn (the spike's `ui/MenuList.svelte`): rows with an
//! icon (a check when it's on), a label and a hint under it; headings and
//! separators; a submenu beside its row, opened by hovering it (150 ms) or
//! clicking it. Rename asks in the menu itself: its name, typed. The items
//! are [`pieces::Entry`] data, so `wadsctl stack menu` can list and pick.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::{gdk, glib, prelude::*};

use super::pieces::{Act, Entry, Item, find_item};

/// A box: x, y, width, height.
type Rect = (f64, f64, f64, f64);

type OnAct = Rc<dyn Fn(&Act)>;
type Done = Rc<dyn Fn(String)>;

/// A submenu open beside its row.
struct Sub {
    id: String,
    popover: gtk::Popover,
}

pub struct Menu {
    me: Weak<Menu>,
    pub popover: gtk::Popover,
    entries: Vec<Entry>,
    autohide: bool,
    on_act: OnAct,
    sub: RefCell<Option<Sub>>,
    /// Hover intent: the row last entered (a submenu opens, or closes, after a moment there).
    hover: Cell<u64>,
    /// Asking (Rename): its entry and what's done with the answer.
    ask: RefCell<Option<(gtk::Entry, Done)>>,
}

impl Menu {
    /// `entries` in a popover on `parent`, pointing at `at` (its pixels).
    /// `on_act` is told what's picked (after the menu's gone, but Rename:
    /// it asks in the menu, by [`Menu::ask`]); `closed` when it goes.
    /// `autohide`: a click elsewhere closes it (one opened by wadsctl has
    /// nothing to grab with, so it stays till it's picked from or closed).
    pub fn show(
        parent: &impl IsA<gtk::Widget>,
        at: gdk::Rectangle,
        entries: Vec<Entry>,
        autohide: bool,
        on_act: impl Fn(&Act) + 'static,
        closed: impl Fn() + 'static,
    ) -> Rc<Menu> {
        let popover = popover(autohide);
        popover.set_parent(parent);
        popover.set_pointing_to(Some(&at));
        let me = Rc::new_cyclic(|me| Menu {
            me: me.clone(),
            popover: popover.clone(),
            entries,
            autohide,
            on_act: Rc::new(on_act),
            sub: RefCell::default(),
            hover: Cell::new(0),
            ask: RefCell::default(),
        });
        popover.set_child(Some(&me.list(&me.entries, true)));
        let w = me.me.clone();
        popover.connect_closed(move |p| {
            if let Some(m) = w.upgrade() {
                m.close_sub();
            }
            closed();
            let p = p.clone();
            // Unparented once it's done closing.
            glib::idle_add_local_once(move || p.unparent());
        });
        popover.popup();
        me
    }

    /// Closes it (and its submenu).
    pub fn close(&self) {
        self.close_sub();
        self.popover.popdown();
    }

    /// What's on it, for wadsctl: `id "label"`, a check when it's on, `›` for a submenu; the submenu's too, if one's open.
    pub fn describe(&self) -> String {
        let mut out = describe(&self.entries);
        if let Some(s) = self.sub.borrow().as_ref()
            && let Some(it) = find_item(&self.entries, &s.id)
        {
            out.push_str(&format!(" | open {}: {}", s.id, describe(&it.submenu)));
        }
        if self.ask.borrow().is_some() {
            out.push_str(" | asking");
        }
        out
    }

    /// Where item `id` is (it, or one in the open submenu): its box in its
    /// popup's surface, and that popup's place on its parent's surface
    /// (the submenu's parent being this popup).
    pub fn item_at(&self, id: &str) -> Option<(Rect, (f64, f64))> {
        let in_popup = |p: &gtk::Popover, row: &gtk::Widget| -> Option<Rect> {
            let r = row.compute_bounds(p)?;
            let (tx, ty) = p.surface_transform();
            Some((tx + r.x() as f64, ty + r.y() as f64, r.width() as f64, r.height() as f64))
        };
        let popup_at = |p: &gtk::Popover| -> Option<(f64, f64)> {
            let s = p.surface()?.downcast::<gdk::Popup>().ok()?;
            Some((s.position_x() as f64, s.position_y() as f64))
        };
        if let Some(row) = self.row_of(id) {
            return Some((in_popup(&self.popover, row.upcast_ref())?, popup_at(&self.popover)?));
        }
        let sub = self.sub.borrow();
        let s = sub.as_ref()?;
        let row = find_row(&s.popover, id)?;
        let (sx, sy) = popup_at(&s.popover)?;
        let (mx, my) = popup_at(&self.popover)?;
        let r = in_popup(&s.popover, row.upcast_ref())?;
        Some((r, (mx + sx, my + sy)))
    }

    /// Picks `id` (wadsctl): runs it, or opens its submenu.
    pub fn pick(&self, id: &str) -> Result<(), String> {
        let it = find_item(&self.entries, id).ok_or_else(|| format!("no item {id}"))?.clone();
        if !it.submenu.is_empty() {
            let row = self.row_of(id).ok_or("its row isn't shown")?;
            self.open_sub(&it, &row);
            return Ok(());
        }
        self.run(&it);
        Ok(())
    }

    /// Asks for a name in the menu (Rename): `title`, the name as it is, the button's word; `done` with what's typed.
    pub fn ask(&self, title: &str, value: &str, ok: &str, done: impl Fn(String) + 'static) {
        self.close_sub();
        let col = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["stack-menu-ask"])
            .build();
        col.append(&gtk::Label::builder().label(title).xalign(0.0).css_classes(["stack-menu-ask-title"]).build());
        let entry = gtk::Entry::builder().text(value).placeholder_text(value).activates_default(false).build();
        entry.set_width_chars(24);
        col.append(&entry);
        let row = gtk::Box::builder().spacing(6).halign(gtk::Align::End).build();
        let cancel = gtk::Button::builder().label("Cancel").css_classes(["stack-menu-btn"]).build();
        let go = gtk::Button::builder().label(ok).css_classes(["stack-menu-btn", "go"]).build();
        row.append(&cancel);
        row.append(&go);
        col.append(&row);
        self.popover.set_child(Some(&col));
        let done: Done = Rc::new(done);
        *self.ask.borrow_mut() = Some((entry.clone(), done));
        let w = self.me.clone();
        cancel.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.close();
            }
        });
        let w = self.me.clone();
        go.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.answer(None);
            }
        });
        let w = self.me.clone();
        entry.connect_activate(move |_| {
            if let Some(m) = w.upgrade() {
                m.answer(None);
            }
        });
        entry.grab_focus();
        entry.select_region(0, -1);
    }

    /// The ask answered (`text`, or what's typed): done, and closed. False if it isn't asking.
    pub fn answer(&self, text: Option<&str>) -> bool {
        let Some((entry, done)) = self.ask.borrow_mut().take() else { return false };
        let typed = text.map(str::to_string).unwrap_or_else(|| entry.text().to_string());
        self.popover.popdown();
        if !typed.trim().is_empty() {
            glib::idle_add_local_once(move || done(typed));
        }
        true
    }

    // ------------------------------------------------------------ drawing

    fn list(&self, entries: &[Entry], top: bool) -> gtk::Box {
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["stack-menu-list"]).build();
        for e in entries {
            match e {
                Entry::Heading(h) => list.append(
                    &gtk::Label::builder()
                        .label(h.to_uppercase())
                        .xalign(0.0)
                        .css_classes(["stack-menu-heading"])
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .max_width_chars(32)
                        .build(),
                ),
                Entry::Sep => list.append(&gtk::Box::builder().css_classes(["stack-menu-sep"]).build()),
                Entry::Item(it) => list.append(&self.row(it, top)),
            }
        }
        list
    }

    fn row(&self, it: &Item, top: bool) -> gtk::Button {
        let b = gtk::Button::builder().css_classes(["stack-menu-item"]).halign(gtk::Align::Fill).build();
        if it.danger {
            b.add_css_class("danger");
        }
        b.set_widget_name(&it.id);
        let h = gtk::Box::builder().spacing(10).hexpand(true).build();
        let ic = gtk::Box::builder().css_classes(["stack-menu-ic"]).valign(gtk::Align::Center).hexpand(false).build();
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
            .valign(gtk::Align::Center)
            .build();
        text.append(
            &gtk::Label::builder()
                .label(&it.label)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .max_width_chars(30)
                .css_classes(["stack-menu-label"])
                .build(),
        );
        if let Some(hint) = &it.hint {
            text.append(
                &gtk::Label::builder()
                    .label(hint)
                    .xalign(0.0)
                    .wrap(true)
                    .max_width_chars(28)
                    .css_classes(["stack-menu-hint"])
                    .build(),
            );
        }
        h.append(&text);
        if !it.submenu.is_empty() {
            h.append(&wad_gtk::Icon::new("chevron-right", 14));
        }
        b.set_child(Some(&h));

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
                    if item.submenu.is_empty() {
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

    fn row_of(&self, id: &str) -> Option<gtk::Button> {
        find_row(&self.popover, id)
    }

    fn open_sub(&self, it: &Item, row: &gtk::Button) {
        if self.sub.borrow().as_ref().is_some_and(|s| s.id == it.id) {
            return;
        }
        self.close_sub();
        let p = popover(self.autohide);
        p.set_position(gtk::PositionType::Right);
        p.set_halign(gtk::Align::Fill);
        p.set_parent(row);
        p.set_child(Some(&self.list(&it.submenu, false)));
        p.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        p.popup();
        row.add_css_class("open");
        *self.sub.borrow_mut() = Some(Sub { id: it.id.clone(), popover: p });
    }

    fn close_sub(&self) {
        if let Some(s) = self.sub.borrow_mut().take() {
            if let Some(row) = s.popover.parent() {
                row.remove_css_class("open");
            }
            s.popover.popdown();
        }
    }

    fn run(&self, it: &Item) {
        let Some(act) = it.act.clone() else { return };
        if act == Act::Rename {
            // It asks here, in the menu.
            (self.on_act)(&act);
            return;
        }
        self.close();
        let f = self.on_act.clone();
        // After the menu's gone (a run may open another).
        glib::idle_add_local_once(move || f(&act));
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
        .css_classes(["stack-menu"])
        .build()
}

fn describe(entries: &[Entry]) -> String {
    let mut parts = vec![];
    for e in entries {
        match e {
            Entry::Heading(h) => parts.push(format!("[{h}]")),
            Entry::Sep => parts.push("|".into()),
            Entry::Item(it) => {
                let mut s = format!("{} \"{}\"", it.id, it.label);
                if it.checked {
                    s.push_str(" ✓");
                }
                if !it.submenu.is_empty() {
                    s.push_str(" ›");
                }
                parts.push(s);
            }
        }
    }
    parts.join(", ")
}
