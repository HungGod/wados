//! The machine's screens: the launcher and, a scroll down, the desktop
//! (`shell/Machine.svelte`). One spring, `progress` (0 the launcher, 1 the
//! desktop), drives both pages:
//!
//! - **chrome** (TOP layer, the whole primary, opaque): the launcher page and
//!   every transition. Its backdrop fades, the carousel rises away, and the
//!   dock's halves fly from the bottom middle to the top corners.
//! - **desktop** (BOTTOM layer, under windows): its icons rise in.
//! - **bar** (TOP layer, along the top): the halves where they land. At
//!   progress 1 it's shown with the same pixels chrome ends on, then chrome
//!   goes away, so windows (between the two layers) come back into view.
//! - **reserve** (TOP, transparent, always there): the bar's room, kept from
//!   windows on both pages, so nothing reflows when the page changes.
//! - **wallpaper** (BACKGROUND, every monitor).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use wad_gtk::{Params, Spring};

use crate::dock::{Ask, Halves};
use crate::surface::{self, Anchor, Spec};

/// The bar's height, and the room windows leave for it.
pub const BAR: i32 = 78;
const DOCK_BOTTOM: f64 = 12.0;
const EDGE: f64 = 10.0;
const TOP: f64 = 8.0;
const PAGE: Params = Params::new(260.0, 30.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Launcher,
    Desktop,
}

pub struct Machine {
    monitor: gdk::Monitor,
    chrome: gtk::ApplicationWindow,
    chrome_fixed: gtk::Fixed,
    backdrop: gtk::Box,
    launcher: gtk::Box,
    chrome_halves: Halves,
    desktop: gtk::ApplicationWindow,
    desktop_fixed: gtk::Fixed,
    desktop_icons: gtk::FlowBox,
    bar: gtk::ApplicationWindow,
    bar_halves: Halves,
    _reserve: gtk::ApplicationWindow,
    _wallpapers: RefCell<Vec<gtk::ApplicationWindow>>,
    overlay: gtk::ApplicationWindow,
    overlay_label: gtk::Label,
    /// Which flash is showing: a newer one keeps the card up past an older one's timeout.
    overlay_gen: Rc<Cell<u64>>,
    page: Cell<Page>,
    spring: RefCell<Option<Spring>>,
    /// What the shell was last told (tests read it back).
    pub log: RefCell<Vec<String>>,
}

impl Machine {
    pub fn new(app: &gtk::Application, monitor: &gdk::Monitor, others: &[gdk::Monitor]) -> Rc<Self> {
        let wallpapers: Vec<_> =
            std::iter::once(monitor).chain(others).map(|m| crate::wallpaper::surface(app, m)).collect();

        let reserve = surface::make(
            app,
            monitor,
            &Spec {
                namespace: "wados-reserve",
                layer: Layer::Top,
                anchor: Anchor::Top,
                exclusive: BAR,
                keyboard: KeyboardMode::None,
                height: BAR,
            },
        );
        surface::click_through(&reserve);

        let desktop = surface::make(
            app,
            monitor,
            &Spec {
                namespace: "wados-desktop",
                layer: Layer::Bottom,
                anchor: Anchor::Fill,
                exclusive: 0,
                keyboard: KeyboardMode::OnDemand,
                height: 0,
            },
        );
        let desktop_fixed = gtk::Fixed::new();
        let desktop_icons = gtk::FlowBox::builder()
            .orientation(gtk::Orientation::Vertical)
            .max_children_per_line(6)
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["desk-grid"])
            .build();
        desktop_fixed.put(&desktop_icons, 0.0, 0.0);
        desktop.set_child(Some(&desktop_fixed));

        let bar = surface::make(
            app,
            monitor,
            &Spec {
                namespace: "wados-bar",
                layer: Layer::Top,
                anchor: Anchor::Top,
                exclusive: -1,
                keyboard: KeyboardMode::OnDemand,
                height: BAR,
            },
        );

        let chrome = surface::make(
            app,
            monitor,
            &Spec {
                namespace: "wados-chrome",
                layer: Layer::Top,
                anchor: Anchor::Fill,
                exclusive: -1,
                keyboard: KeyboardMode::Exclusive,
                height: 0,
            },
        );
        let chrome_fixed = gtk::Fixed::new();
        let backdrop = gtk::Box::builder().css_classes(["backdrop"]).build();
        chrome_fixed.put(&backdrop, 0.0, 0.0);
        let launcher = launcher_placeholder();
        chrome_fixed.put(&launcher, 0.0, 0.0);
        chrome.set_child(Some(&chrome_fixed));

        let overlay = surface::make(
            app,
            monitor,
            &Spec {
                namespace: "wados-overlay",
                layer: Layer::Overlay,
                anchor: Anchor::Top,
                exclusive: -1,
                keyboard: KeyboardMode::None,
                height: 120,
            },
        );
        overlay.set_anchor(gtk4_layer_shell::Edge::Left, false);
        overlay.set_anchor(gtk4_layer_shell::Edge::Right, false);
        overlay.set_margin(gtk4_layer_shell::Edge::Top, 160);
        let overlay_label = gtk::Label::builder().css_classes(["overlay-card", "display"]).build();
        overlay.set_child(Some(&overlay_label));
        surface::click_through(&overlay);

        let me = Rc::new_cyclic(|weak: &std::rc::Weak<Machine>| {
            let asker = |w: std::rc::Weak<Machine>| {
                move |a: Ask| {
                    if let Some(m) = w.upgrade() {
                        match a {
                            Ask::Page => m.toggle(),
                            Ask::Search => m.flash("Alt+Space: search"),
                        }
                    }
                }
            };
            let chrome_halves = Halves::new(asker(weak.clone()));
            let bar_halves = Halves::new(asker(weak.clone()));
            Machine {
                monitor: monitor.clone(),
                chrome,
                chrome_fixed,
                backdrop,
                launcher,
                chrome_halves,
                desktop,
                desktop_fixed,
                desktop_icons,
                bar,
                bar_halves,
                _reserve: reserve,
                _wallpapers: RefCell::new(wallpapers),
                overlay,
                overlay_label,
                overlay_gen: Rc::new(Cell::new(0)),
                page: Cell::new(Page::Launcher),
                spring: RefCell::new(None),
                log: RefCell::new(Vec::new()),
            }
        });
        me.chrome_fixed.put(&me.chrome_halves.left, 0.0, 0.0);
        me.chrome_fixed.put(&me.chrome_halves.right, 0.0, 0.0);
        let bar_fixed = gtk::Fixed::new();
        bar_fixed.put(&me.bar_halves.left, 0.0, 0.0);
        bar_fixed.put(&me.bar_halves.right, 0.0, 0.0);
        me.bar.set_child(Some(&bar_fixed));
        me.bar_halves.set_on_desktop(true);
        me.keys();
        me.fill_desktop();

        let weak = Rc::downgrade(&me);
        let spring = Spring::new(&me.chrome, PAGE, 0.0, move |p| {
            if let Some(m) = weak.upgrade() {
                m.lay_out(p);
            }
        });
        *me.spring.borrow_mut() = Some(spring);

        for w in me._wallpapers.borrow().iter() {
            w.present();
        }
        me._reserve.present();
        me.desktop.present();
        me.chrome.present();
        me.lay_out(0.0);
        me
    }

    fn size(&self) -> (f64, f64) {
        let g = self.monitor.geometry();
        (g.width() as f64, g.height() as f64)
    }

    /// Where the halves sit at `p` (0 the launcher's dock, 1 the desktop's
    /// corners): (left x, right x, y).
    fn halves_at(&self, p: f64) -> (f64, f64, f64) {
        let (w, h) = self.size();
        let (lw, lh) = natural(&self.chrome_halves.left);
        let (rw, _) = natural(&self.chrome_halves.right);
        let dock_left = ((w - (lw + rw)) / 2.0).round();
        let dock_y = h - lh - DOCK_BOTTOM;
        let (desk_left, desk_right, desk_y) = (EDGE, w - rw - EDGE, TOP);
        let mix = |a: f64, b: f64| (a + (b - a) * p).round();
        (mix(dock_left, desk_left), mix(dock_left + lw, desk_right), mix(dock_y, desk_y))
    }

    /// Everything at `p`.
    fn lay_out(&self, p: f64) {
        let (w, h) = self.size();
        if self.backdrop.width_request() != w as i32 || self.backdrop.height_request() != h as i32 {
            self.backdrop.set_size_request(w as i32, h as i32);
            self.launcher.set_size_request(w as i32, -1);
        }
        self.backdrop.set_opacity(1.0 - p);
        self.launcher.set_opacity((1.0 - p * 1.6).max(0.0));
        self.chrome_fixed.move_(&self.launcher, 0.0, (h * 0.18 - p * h * 0.25).round());
        let (lx, rx, y) = self.halves_at(p);
        self.chrome_fixed.move_(&self.chrome_halves.left, lx, y);
        self.chrome_fixed.move_(&self.chrome_halves.right, rx, y);
        self.chrome_halves.set_on_desktop(p > 0.5);
        // The desktop rises into place and fades in.
        self.desktop_fixed.move_(&self.desktop_icons, 16.0, 16.0 + ((1.0 - p) * 60.0).round());
        self.desktop_icons.set_opacity(p.clamp(0.0, 1.0));

        let (lx, rx, y) = self.halves_at(1.0);
        let bar_fixed = self.bar.child().and_downcast::<gtk::Fixed>();
        if let Some(f) = bar_fixed {
            f.move_(&self.bar_halves.left, lx, y);
            f.move_(&self.bar_halves.right, rx, y);
        }
        let to_desktop = self.spring.borrow().as_ref().is_some_and(|s| s.target() == 1.0);
        if to_desktop && p >= 1.0 && self.chrome.is_visible() {
            // Landed: the bar takes over with the same pixels, then chrome goes.
            self.bar.present();
            let chrome = self.chrome.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(c) = chrome.upgrade() {
                    c.set_visible(false);
                }
            });
        }
    }

    pub fn page(&self) -> Page {
        self.page.get()
    }

    pub fn progress(&self) -> f64 {
        self.spring.borrow().as_ref().map(|s| s.value()).unwrap_or(0.0)
    }

    pub fn go(&self, page: Page) {
        if self.page.replace(page) == page {
            return;
        }
        let Some(spring) = self.spring.borrow().clone() else { return };
        match page {
            Page::Desktop => {
                self.chrome.set_keyboard_mode(KeyboardMode::None);
                spring.to(1.0);
            }
            Page::Launcher => {
                self.chrome.set_keyboard_mode(KeyboardMode::Exclusive);
                self.lay_out(spring.value());
                self.chrome.present();
                self.bar.set_visible(false);
                spring.to(0.0);
            }
        }
    }

    pub fn toggle(&self) {
        self.go(if self.page.get() == Page::Launcher { Page::Desktop } else { Page::Launcher });
    }

    /// A card in the middle of the screen for a moment (phase 0's stand-in
    /// for the switcher and the search).
    pub fn flash(&self, text: &str) {
        self.log.borrow_mut().push(text.to_string());
        self.overlay_label.set_label(text);
        self.overlay.present();
        let generation = self.overlay_gen.get() + 1;
        self.overlay_gen.set(generation);
        let (o, current) = (self.overlay.downgrade(), self.overlay_gen.clone());
        glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
            if current.get() == generation
                && let Some(o) = o.upgrade()
            {
                o.set_visible(false);
            }
        });
    }

    fn keys(self: &Rc<Self>) {
        // The launcher: ↓ / PageDown for the desktop; a scroll down too.
        let keys = gtk::EventControllerKey::new();
        let w = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(m) = w.upgrade() else { return glib::Propagation::Proceed };
            match key {
                gdk::Key::Page_Down | gdk::Key::Down => {
                    m.go(Page::Desktop);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        self.chrome.add_controller(keys);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let w = Rc::downgrade(self);
        scroll.connect_scroll(move |_, _, dy| {
            if let Some(m) = w.upgrade() {
                if dy > 0.0 && m.page() == Page::Launcher {
                    m.go(Page::Desktop);
                } else if dy < 0.0 && m.page() == Page::Desktop {
                    m.go(Page::Launcher);
                }
            }
            glib::Propagation::Stop
        });
        self.chrome.add_controller(scroll);

        // The desktop: ↑ / PageUp back up; a scroll up too.
        let keys = gtk::EventControllerKey::new();
        let w = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(m) = w.upgrade() else { return glib::Propagation::Proceed };
            match key {
                gdk::Key::Page_Up | gdk::Key::Up => {
                    m.go(Page::Launcher);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        self.desktop.add_controller(keys);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let w = Rc::downgrade(self);
        scroll.connect_scroll(move |_, _, dy| {
            if let Some(m) = w.upgrade()
                && dy < 0.0
            {
                m.go(Page::Launcher);
            }
            glib::Propagation::Stop
        });
        self.desktop.add_controller(scroll);
    }

    /// Phase 0's desktop: a few icons, and a drop target that says what landed.
    fn fill_desktop(self: &Rc<Self>) {
        for (icon, name) in [
            ("folder", "Studio"),
            ("folder", "Writing"),
            ("file-text", "notes.txt"),
            ("server", "Machines"),
            ("trash", "Trash"),
        ] {
            let tile = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(6)
                .css_classes(["desk-icon"])
                .build();
            let glass = gtk::Box::builder().css_classes(["desk-glass"]).halign(gtk::Align::Center).build();
            glass.append(&wad_gtk::Icon::new(icon, 28));
            tile.append(&glass);
            tile.append(&gtk::Label::builder().label(name).css_classes(["desk-label"]).build());
            self.desktop_icons.append(&tile);
        }
        let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY | gdk::DragAction::MOVE);
        let w = Rc::downgrade(self);
        drop.connect_drop(move |_, value, x, y| {
            let Some(m) = w.upgrade() else { return false };
            let what = value.get::<String>().unwrap_or_default();
            let line = format!("dropped {what:?} on the desktop at {x:.0},{y:.0}");
            tracing::info!("{line}");
            m.flash(&line);
            true
        });
        self.desktop.add_controller(drop);
    }
}

fn natural(w: &impl IsA<gtk::Widget>) -> (f64, f64) {
    let (_, nw, _, _) = w.measure(gtk::Orientation::Horizontal, -1);
    let (_, nh, _, _) = w.measure(gtk::Orientation::Vertical, nw);
    (nw as f64, nh as f64)
}

/// Phase 0's launcher: the tabs and three cards (the carousel comes in
/// phase 2, from `shell/home/Home.svelte`).
fn launcher_placeholder() -> gtk::Box {
    let col =
        gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(40).halign(gtk::Align::Center).build();
    let tabs =
        gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(28).halign(gtk::Align::Center).build();
    for (i, t) in ["Running", "Recent", "Work", "Gaming", "Studio"].iter().enumerate() {
        let l = gtk::Label::builder().label(*t).css_classes(["tab", "display"]).build();
        if i == 2 {
            l.add_css_class("tab-on");
        }
        tabs.append(&l);
    }
    col.append(&tabs);
    let cards = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(24)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    for (i, name) in ["VanuaAcademy", "Cosmic Bodybuilding", "WadOS"].iter().enumerate() {
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::End)
            .css_classes(["card"])
            .build();
        if i == 1 {
            card.add_css_class("card-focus");
        }
        let spacer = gtk::Box::builder().vexpand(true).build();
        card.append(&spacer);
        card.append(&gtk::Label::builder().label(*name).xalign(0.0).css_classes(["card-name", "display"]).build());
        cards.append(&card);
    }
    col.append(&cards);
    col
}
