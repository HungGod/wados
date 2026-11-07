//! The shield (the spike's `PrivacyPopover.svelte` and the header button it
//! hangs from): the button shows how many ads and trackers were blocked in
//! all; its popover, how many on this page, the blockers on or off for this
//! site, "Block all social media", and the privacy dashboard.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::{glib, prelude::*};

use crate::data::{compact, thousands};
use crate::menus::Run;

/// What the shield shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShieldInfo {
    /// This page's site ("github.com"); None on the browser's own pages.
    pub site: Option<String>,
    /// Ads and trackers blocked on this page.
    pub here: u32,
    /// Blocked in all.
    pub total: u64,
    /// The blockers are off for this site.
    pub allowed: bool,
    /// "Block all social media" is on.
    pub social: bool,
}

/// What it does.
#[derive(Clone)]
pub struct ShieldActions {
    /// The blockers on (true) or off for this page's site.
    pub set_blockers: Rc<dyn Fn(bool)>,
    /// "Block all social media", on or off.
    pub set_social: Rc<dyn Fn(bool)>,
    /// "Privacy dashboard": wadbrowser://privacy.
    pub open_dashboard: Run,
}

impl Default for ShieldActions {
    fn default() -> Self {
        Self { set_blockers: Rc::new(|_| {}), set_social: Rc::new(|_| {}), open_dashboard: Rc::new(|| {}) }
    }
}

/// The header button and its popover.
pub struct Shield {
    button: gtk::Button,
    icon: wad_gtk::Icon,
    count: gtk::Label,
    popover: gtk::Popover,
    badge: gtk::Box,
    badge_icon: wad_gtk::Icon,
    title: gtk::Label,
    note: gtk::Label,
    here: gtk::Label,
    total: gtk::Label,
    site_row: gtk::Box,
    site_switch: gtk::Switch,
    social_switch: gtk::Switch,
    info: RefCell<ShieldInfo>,
    /// Set while [`Shield::update`] moves the switches (they don't ask then).
    quiet: Cell<bool>,
}

impl Shield {
    pub fn new(actions: ShieldActions) -> Rc<Shield> {
        let icon = wad_gtk::Icon::new("shield-check", 16);
        let count = gtk::Label::builder().css_classes(["wb-shield-count"]).build();
        let inner = gtk::Box::builder().spacing(6).build();
        inner.append(&icon);
        inner.append(&count);
        let button = gtk::Button::builder()
            .child(&inner)
            .css_classes(["wb-hb", "wb-shield"])
            .tooltip_text("Privacy")
            .valign(gtk::Align::Center)
            .build();
        button.update_property(&[gtk::accessible::Property::Label("Privacy")]);

        // The popover.
        let col =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).css_classes(["wb-pop-col"]).build();
        col.set_size_request(266, -1);
        let header = gtk::Box::builder().spacing(10).build();
        let badge_icon = wad_gtk::Icon::new("shield-check", 18);
        badge_icon.set_halign(gtk::Align::Center);
        badge_icon.set_hexpand(true);
        let badge =
            gtk::Box::builder().css_classes(["wb-shield-badge"]).valign(gtk::Align::Center).hexpand(false).build();
        badge.set_size_request(34, 34);
        badge.append(&badge_icon);
        let words = gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).build();
        let title = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["wb-pop-title"])
            .build();
        let note = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["wb-pop-small"]).build();
        words.append(&title);
        words.append(&note);
        header.append(&badge);
        header.append(&words);
        col.append(&header);

        let counts = gtk::Box::builder().spacing(8).homogeneous(true).build();
        let count_box = |what: &str| {
            let b =
                gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["wb-shield-tally"]).build();
            let n = gtk::Label::builder().xalign(0.0).css_classes(["wb-shield-n"]).build();
            b.append(&n);
            b.append(&gtk::Label::builder().label(what).xalign(0.0).css_classes(["wb-pop-small"]).build());
            counts.append(&b);
            n
        };
        let here = count_box("on this page");
        let total = count_box("in all");
        col.append(&counts);

        let switch_row = |label: &str| {
            let row = gtk::Box::builder().spacing(10).css_classes(["wb-pop-row"]).build();
            row.append(&gtk::Label::builder().label(label).xalign(0.0).hexpand(true).build());
            let s = gtk::Switch::builder().css_classes(["wb-toggle"]).valign(gtk::Align::Center).build();
            s.update_property(&[gtk::accessible::Property::Label(label)]);
            row.append(&s);
            col.append(&row);
            (row, s)
        };
        let (site_row, site_switch) = switch_row("Blockers on for this site");
        let (_, social_switch) = switch_row("Block all social media");

        let dash_inner = gtk::Box::builder().spacing(8).build();
        dash_inner.append(&wad_gtk::Icon::new("gauge", 15));
        dash_inner.append(&gtk::Label::new(Some("Privacy dashboard")));
        let dash = gtk::Button::builder().child(&dash_inner).css_classes(["wb-pop-link"]).build();
        col.append(&dash);

        let popover = gtk::Popover::builder()
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .halign(gtk::Align::End)
            .css_classes(["wb-pop"])
            .child(&col)
            .build();
        popover.set_offset(0, 6);
        popover.set_parent(&button);

        let me = Rc::new(Shield {
            button,
            icon,
            count,
            popover,
            badge,
            badge_icon,
            title,
            note,
            here,
            total,
            site_row,
            site_switch,
            social_switch,
            info: RefCell::default(),
            quiet: Cell::new(false),
        });

        let w = Rc::downgrade(&me);
        me.button.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                s.popup();
            }
        });
        let (w, set) = (Rc::downgrade(&me), actions.set_blockers.clone());
        me.site_switch.connect_active_notify(move |sw| {
            let Some(s) = w.upgrade() else { return };
            if s.quiet.get() {
                return;
            }
            let on = sw.is_active();
            let info = ShieldInfo { allowed: !on, ..s.info.borrow().clone() };
            s.update(&info);
            set(on);
        });
        let (w, set) = (Rc::downgrade(&me), actions.set_social.clone());
        me.social_switch.connect_active_notify(move |sw| {
            let Some(s) = w.upgrade() else { return };
            if s.quiet.get() {
                return;
            }
            s.info.borrow_mut().social = sw.is_active();
            set(sw.is_active());
        });
        let (pop, open) = (me.popover.downgrade(), actions.open_dashboard.clone());
        dash.connect_clicked(move |_| {
            if let Some(p) = pop.upgrade() {
                p.popdown();
            }
            let open = open.clone();
            glib::idle_add_local_once(move || open());
        });
        me.update(&ShieldInfo::default());
        me
    }

    /// The header button, to put in the header bar.
    pub fn button(&self) -> &gtk::Button {
        &self.button
    }

    pub fn popover(&self) -> &gtk::Popover {
        &self.popover
    }

    pub fn popup(&self) {
        self.popover.popup();
    }

    pub fn popdown(&self) {
        self.popover.popdown();
    }

    /// Shows `info` (a page loaded, something was blocked, a setting changed).
    pub fn update(&self, info: &ShieldInfo) {
        *self.info.borrow_mut() = info.clone();
        let web = info.site.is_some();
        let off = web && info.allowed;
        let shield = if off { "shield-off" } else { "shield-check" };
        self.icon.set(shield);
        self.badge_icon.set(shield);
        for w in [self.button.upcast_ref::<gtk::Widget>(), self.badge.upcast_ref()] {
            if off {
                w.add_css_class("off");
            } else {
                w.remove_css_class("off");
            }
        }
        self.count.set_label(&compact(info.total));
        self.title.set_label(info.site.as_deref().unwrap_or("WadBrowser"));
        self.note.set_label(if !web {
            "A page of the browser’s own"
        } else if off {
            "The blockers are off here"
        } else {
            "Ads and trackers are blocked here"
        });
        self.here.set_label(&thousands(info.here as u64));
        self.total.set_label(&thousands(info.total));
        self.site_row.set_visible(web);
        self.quiet.set(true);
        self.site_switch.set_active(!info.allowed);
        self.social_switch.set_active(info.social);
        self.quiet.set(false);
    }
}
