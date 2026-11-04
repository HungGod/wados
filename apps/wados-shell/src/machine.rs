//! The machine's screens: the launcher and, a scroll down, the desktop
//! (`shell/Machine.svelte`). One spring, `progress` (0 the launcher, 1 the
//! desktop), drives both pages:
//!
//! - **chrome** (TOP layer, the whole primary, opaque): the launcher page
//!   (home::Launcher) and every transition. Its backdrop fades, the launcher
//!   rises away, and the bar's halves fly from the bottom middle (the dock)
//!   to the top corners.
//! - **desktop** (BOTTOM layer, under windows): desktop::Desktop rises in.
//! - **bar** (TOP layer, along the top): the halves where they land, with
//!   bar::Middle between them (what's running, the machine's apps). At
//!   progress 1 it's shown with the same pixels chrome ends on, then chrome
//!   goes away, so windows (between the two layers) come back into view.
//! - **reserve** (TOP, transparent, always there): the bar's room, kept from
//!   windows on both pages, so nothing reflows when the page changes.
//! - **wallpaper** (BACKGROUND, every monitor).
//!
//! This module owns the surfaces, the spring and the page keys; what's on
//! each page is its module's.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use wad_gtk::{Params, Spring};

use crate::bar::{Halves, Middle};
use crate::desktop::Desktop;
use crate::home::Launcher;
use crate::shell::Shell;
use crate::surface::{self, Anchor, Spec};

/// The bar's height, and the room windows leave for it.
pub const BAR: i32 = 78;
const DOCK_BOTTOM: f64 = 12.0;
const EDGE: f64 = 10.0;
const TOP: f64 = 8.0;
/// The room between a half and what's between them on the bar.
const GAP: f64 = 10.0;
const PAGE: Params = Params::new(260.0, 30.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Launcher,
    Desktop,
}

pub struct Machine {
    shell: Rc<Shell>,
    chrome: gtk::ApplicationWindow,
    chrome_fixed: gtk::Fixed,
    backdrop: gtk::Box,
    launcher: Rc<Launcher>,
    chrome_halves: Rc<Halves>,
    desktop_win: gtk::ApplicationWindow,
    desktop_fixed: gtk::Fixed,
    desktop: Rc<Desktop>,
    bar: gtk::ApplicationWindow,
    bar_fixed: gtk::Fixed,
    bar_halves: Rc<Halves>,
    middle: Rc<Middle>,
    _reserve: gtk::ApplicationWindow,
    _wallpapers: Vec<gtk::ApplicationWindow>,
    page: Cell<Page>,
    spring: RefCell<Option<Spring>>,
}

impl Machine {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let app = &shell.app;
        let monitor = &shell.primary;
        let wallpapers: Vec<_> =
            std::iter::once(monitor).chain(shell.others.iter()).map(|m| crate::wallpaper::surface(app, m)).collect();

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
        // Something to paint: GTK draws no frame for a window with nothing in
        // it, and a layer surface with no frame isn't mapped, so its room for
        // the bar wouldn't count (the desktop, and the Builder snapped into
        // its region, would start under the bar). This strip is the bar's
        // height and 1/255 opaque: never seen, but drawn on the first frame.
        let fill = gtk::Box::builder().css_classes(["reserve-fill"]).hexpand(true).build();
        fill.set_size_request(-1, BAR);
        reserve.set_child(Some(&fill));

        let desktop_win = surface::make(
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
        let desktop = Desktop::new(shell);
        desktop_fixed.put(&desktop.widget(), 0.0, 0.0);
        desktop_win.set_child(Some(&desktop_fixed));

        let bar = surface::make(
            app,
            monitor,
            &Spec {
                namespace: "wados-bar",
                layer: Layer::Top,
                anchor: Anchor::Top,
                exclusive: -1,
                // None: on-demand, it would take the keys each time it maps
                // (labwc focuses top-layer on-demand surfaces then). Its
                // flyouts are popovers that grab, which labwc gives the keys.
                keyboard: KeyboardMode::None,
                height: BAR,
            },
        );
        let bar_fixed = gtk::Fixed::new();
        let bar_halves = Halves::new(shell, true);
        let middle = Middle::new(shell);
        bar_fixed.put(&bar_halves.left(), 0.0, 0.0);
        bar_fixed.put(&middle.widget(), 0.0, 0.0);
        bar_fixed.put(&bar_halves.right(), 0.0, 0.0);
        bar.set_child(Some(&bar_fixed));

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
        let launcher = Launcher::new(shell);
        chrome_fixed.put(&launcher.widget(), 0.0, 0.0);
        let chrome_halves = Halves::new(shell, false);
        chrome_fixed.put(&chrome_halves.left(), 0.0, 0.0);
        chrome_fixed.put(&chrome_halves.right(), 0.0, 0.0);
        chrome.set_child(Some(&chrome_fixed));

        let me = Rc::new(Machine {
            shell: shell.clone(),
            chrome,
            chrome_fixed,
            backdrop,
            launcher,
            chrome_halves,
            desktop_win,
            desktop_fixed,
            desktop,
            bar,
            bar_fixed,
            bar_halves,
            middle,
            _reserve: reserve,
            _wallpapers: wallpapers,
            page: Cell::new(Page::Launcher),
            spring: RefCell::new(None),
        });
        shell.set_machine(&me);
        me.keys();

        let weak = Rc::downgrade(&me);
        let spring = Spring::new(&me.chrome, PAGE, 0.0, move |p| {
            if let Some(m) = weak.upgrade() {
                m.lay_out(p);
            }
        });
        *me.spring.borrow_mut() = Some(spring);
        // A half that changes size (a task, the jobs' ring, the clock) is laid out again.
        for h in [&me.chrome_halves, &me.bar_halves] {
            let w = Rc::downgrade(&me);
            h.on_resize(move || {
                if let Some(m) = w.upgrade() {
                    m.lay_out(m.progress());
                }
            });
        }

        for w in &me._wallpapers {
            w.present();
        }
        me._reserve.present();
        me.desktop_win.present();
        me.chrome.present();
        me.lay_out(0.0);
        me.launcher.shown();
        me
    }

    fn size(&self) -> (f64, f64) {
        let (w, h) = self.shell.size();
        (w as f64, h as f64)
    }

    /// Where the halves sit at `p` (0 the launcher's dock, 1 the desktop's
    /// corners): (left x, right x, y).
    fn halves_at(&self, p: f64) -> (f64, f64, f64) {
        let (w, h) = self.size();
        let (lw, lh) = natural(&self.chrome_halves.left());
        let (rw, _) = natural(&self.chrome_halves.right());
        let dock_left = ((w - (lw + rw)) / 2.0).round();
        let dock_y = h - lh - DOCK_BOTTOM;
        let (desk_left, desk_right, desk_y) = (EDGE, w - rw - EDGE, TOP);
        let mix = |a: f64, b: f64| (a + (b - a) * p).round();
        (mix(dock_left, desk_left), mix(dock_left + lw, desk_right), mix(dock_y, desk_y))
    }

    /// Everything at `p`.
    fn lay_out(&self, p: f64) {
        let (w, h) = self.size();
        let (_, lh) = natural(&self.chrome_halves.left());
        if self.backdrop.width_request() != w as i32 || self.backdrop.height_request() != h as i32 {
            self.backdrop.set_size_request(w as i32, h as i32);
            // The launcher has the screen above the dock.
            let room = (h - lh - DOCK_BOTTOM * 2.0).max(0.0) as i32;
            self.launcher.widget().set_size_request(w as i32, room);
            self.desktop.widget().set_size_request(w as i32, (h as i32 - BAR).max(0));
        }
        self.backdrop.set_opacity(1.0 - p);
        let launcher = self.launcher.widget();
        launcher.set_opacity((1.0 - p * 1.6).max(0.0));
        self.chrome_fixed.move_(&launcher, 0.0, (-p * h * 0.25).round());
        let (lx, rx, y) = self.halves_at(p);
        self.chrome_fixed.move_(&self.chrome_halves.left(), lx, y);
        self.chrome_fixed.move_(&self.chrome_halves.right(), rx, y);
        self.chrome_halves.set_on_desktop(p > 0.5);
        // The desktop rises into place and fades in.
        let desk = self.desktop.widget();
        self.desktop_fixed.move_(&desk, 0.0, ((1.0 - p) * 60.0).round());
        desk.set_opacity(p.clamp(0.0, 1.0));

        // The bar where the halves land, and what's between them.
        let (lx, rx, y) = self.halves_at(1.0);
        let (lw, _) = natural(&self.bar_halves.left());
        self.bar_fixed.move_(&self.bar_halves.left(), lx, y);
        self.bar_fixed.move_(&self.bar_halves.right(), rx, y);
        let room = ((rx - GAP) - (lx + lw + GAP)).max(0.0);
        let mid = self.middle.widget();
        mid.set_size_request(room as i32, -1);
        self.bar_fixed.move_(&mid, lx + lw + GAP, y);

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
            self.desktop.shown();
            self.desktop_keys();
        }
    }

    /// Gives the desktop the keys (it's in the BOTTOM layer, so labwc gives
    /// them on a click only): it turns its keyboard off, then, once that's
    /// committed, on-demand again, which host/labwc/0003 takes as asking.
    pub fn desktop_keys(&self) {
        let win = self.desktop_win.clone();
        win.set_keyboard_mode(KeyboardMode::None);
        let Some(clock) = win.frame_clock() else {
            win.set_keyboard_mode(KeyboardMode::OnDemand);
            return;
        };
        let handler = Rc::new(Cell::new(None::<glib::SignalHandlerId>));
        let h = handler.clone();
        let id = clock.connect_after_paint(move |clock| {
            // Painted: the None went with that frame's commit.
            if let Some(id) = h.take() {
                clock.disconnect(id);
            }
            win.set_keyboard_mode(KeyboardMode::OnDemand);
        });
        handler.set(Some(id));
        self.desktop_win.queue_draw();
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
                self.launcher.shown();
            }
        }
    }

    pub fn toggle(&self) {
        self.go(if self.page.get() == Page::Launcher { Page::Desktop } else { Page::Launcher });
    }

    fn keys(self: &Rc<Self>) {
        // The launcher: its own keys first (home::Launcher::key), then
        // PageDown for the desktop; a scroll down too.
        let keys = gtk::EventControllerKey::new();
        let w = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(m) = w.upgrade() else { return glib::Propagation::Proceed };
            if m.launcher.key(key, mods) {
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::Page_Down {
                m.go(Page::Desktop);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
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

        // The desktop: its own keys first (desktop::Desktop::key), then
        // PageUp back up; a scroll up too.
        let keys = gtk::EventControllerKey::new();
        let w = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(m) = w.upgrade() else { return glib::Propagation::Proceed };
            if m.desktop.key(key, mods) {
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::Page_Up {
                m.go(Page::Launcher);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.desktop_win.add_controller(keys);
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
        self.desktop_win.add_controller(scroll);
    }
}

pub fn natural(w: &impl IsA<gtk::Widget>) -> (f64, f64) {
    let (_, nw, _, _) = w.measure(gtk::Orientation::Horizontal, -1);
    let (_, nh, _, _) = w.measure(gtk::Orientation::Vertical, nw);
    (nw as f64, nh as f64)
}
