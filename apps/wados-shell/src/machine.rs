//! The machine's screens: the launcher and, a scroll down, the desktop
//! (`shell/Machine.svelte`), two pages one above the other. One spring,
//! `progress` (0 the launcher, 1 the desktop), scrolls them on every display,
//! a page's height for a page's height:
//!
//! - **chrome** (TOP layer, the whole primary): the launcher page, going up
//!   and off the top, its dock with it. It's opaque and the rest of the
//!   surface is clear, so what's coming shows below it.
//! - **desktop** (BOTTOM layer, under windows): desktop::Desktop, coming up
//!   from the bottom with the **wallpaper** (BACKGROUND, every display).
//! - **bar** (TOP layer, along the top): bar::Halves at the corners and
//!   bar::Middle between them, rising with the desktop by its surface's
//!   margin (it's shown once it's whole on the display, so it can't appear
//!   on one below this one).
//! - **reserve** (TOP, transparent, always there): the bar's room, kept from
//!   windows on both pages, so nothing reflows when the page changes.
//! - **Every other display** ([`Other`]) has a desktop of its own, with no
//!   bar, and a **sheet** (TOP) with the launcher's picture on it: they
//!   scroll as the primary's do. A full-screen window on a display keeps it:
//!   labwc hides the TOP layer there.
//!
//! Windows can't move with the page: chrome and the sheets uncover them.
//!
//! This module owns the surfaces, the spring and the page keys; what's on
//! each page is its module's.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use wad_gtk::{Params, Spring};

use crate::bar::{Halves, Middle};
use crate::desktop::Desktop;
use crate::home::Launcher;
use crate::home::backdrop::Backdrop;
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
    /// Each display's wallpaper, made again as displays come and go.
    wallpapers: RefCell<Vec<(gdk::Monitor, gtk::ApplicationWindow)>>,
    /// The displays that aren't the primary, likewise.
    others: RefCell<Vec<Other>>,
    page: Cell<Page>,
    spring: RefCell<Option<Spring>>,
}

/// A display that isn't the primary: its desktop, and its launcher page's sheet.
struct Other {
    monitor: gdk::Monitor,
    desk_win: gtk::ApplicationWindow,
    desk_fixed: gtk::Fixed,
    desktop: Rc<Desktop>,
    sheet: gtk::ApplicationWindow,
    sheet_fixed: gtk::Fixed,
    backdrop: Backdrop,
}

impl Other {
    fn size(&self) -> (i32, i32) {
        let g = self.monitor.geometry();
        (g.width(), g.height())
    }

    /// Everything at `p`.
    fn lay_out(&self, p: f64) {
        let (w, h) = self.size();
        self.backdrop.set_size_request(w, h);
        self.desktop.widget().set_size_request(w, h);
        self.sheet_fixed.move_(&self.backdrop, 0.0, up_by(p, h as f64));
        self.desk_fixed.move_(&self.desktop.widget(), 0.0, rise(p, h as f64));
    }
}

/// How far up the launcher page has gone at `p`, on a display `h` tall.
fn up_by(p: f64, h: f64) -> f64 {
    -(p.clamp(0.0, 1.0) * h).round()
}

/// How far down the desktop page still is at `p`, on a display `h` tall.
fn rise(p: f64, h: f64) -> f64 {
    h + up_by(p, h)
}

impl Machine {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let app = &shell.app;
        let monitor = &shell.primary;
        let picture = shell.desk.settings().background.picture;
        let wallpapers: Vec<_> = std::iter::once(&**monitor)
            .chain(shell.others.iter())
            .map(|m| (m.clone(), crate::wallpaper::surface(app, m, &picture)))
            .collect();

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
        // the bar wouldn't count (the desktop, and a maximized window, would
        // start under the bar). This strip is the bar's
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
        let desktop = Desktop::new(shell, None);
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
            wallpapers: RefCell::new(wallpapers),
            others: RefCell::default(),
            page: Cell::new(Page::Launcher),
            spring: RefCell::new(None),
        });
        shell.set_machine(&me);
        me.keys();
        me.follow_windows(shell);

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

        for (_, w) in me.wallpapers.borrow().iter() {
            w.present();
        }
        me.others_now();
        let w = Rc::downgrade(&me);
        me.launcher.on_picture(move |picture| {
            if let Some(m) = w.upgrade() {
                for o in m.others.borrow().iter() {
                    let (dw, dh) = o.size();
                    o.backdrop.set(picture, dw as f64, dh as f64);
                }
            }
        });
        let w = Rc::downgrade(&me);
        shell.on_monitors(move || {
            if let Some(m) = w.upgrade() {
                m.monitors_changed();
            }
        });
        let w = Rc::downgrade(&me);
        shell.desk.on_change(move |s| {
            if let Some(m) = w.upgrade() {
                for (_, win) in m.wallpapers.borrow().iter() {
                    crate::wallpaper::set(win, &s.background.picture);
                }
            }
        });
        me._reserve.present();
        me.desktop_win.present();
        me.chrome.present();
        me.lay_out(0.0);
        me.launcher.shown();
        me
    }

    /// The displays changed (monitors.rs): a wallpaper for each one now
    /// (the gone ones' destroyed), and everything laid out for the primary's size.
    fn monitors_changed(self: &Rc<Self>) {
        let now: Vec<gdk::Monitor> =
            std::iter::once(self.shell.primary.clone()).chain(self.shell.others.iter().cloned()).collect();
        let picture = self.shell.desk.settings().background.picture;
        let mut walls = self.wallpapers.borrow_mut();
        walls.retain(|(m, w)| {
            let keep = now.contains(m);
            if !keep {
                w.destroy();
            }
            keep
        });
        for m in &now {
            if !walls.iter().any(|(x, _)| x == m) {
                let w = crate::wallpaper::surface(&self.shell.app, m, &picture);
                w.present();
                walls.push((m.clone(), w));
            }
        }
        for (m, w) in walls.iter() {
            crate::wallpaper::fit(w, m);
        }
        drop(walls);
        self.others_now();
        self.lay_out(self.progress());
        // A surface remapped on another display is drawn again from nothing.
        for w in [&self.chrome, &self.bar, &self.desktop_win] {
            w.queue_resize();
        }
    }

    fn size(&self) -> (f64, f64) {
        let (w, h) = self.shell.size();
        (w as f64, h as f64)
    }

    /// The displays that aren't the primary as they are now: those gone
    /// away (and their icons back on the primary), new ones made.
    fn others_now(self: &Rc<Self>) {
        let now: Vec<gdk::Monitor> = self.shell.others.iter().cloned().collect();
        let mut others = self.others.borrow_mut();
        others.retain(|o| {
            let keep = now.contains(&o.monitor);
            if !keep {
                o.desk_win.destroy();
                o.sheet.destroy();
            }
            keep
        });
        for m in &now {
            if !others.iter().any(|o| o.monitor == *m) {
                others.push(self.other(m));
            }
        }
        drop(others);
        self.desktop.refresh();
    }

    fn other(self: &Rc<Self>, monitor: &gdk::Monitor) -> Other {
        let app = &self.shell.app;
        let desk_win = surface::make(
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
        surface::stay(&desk_win);
        let desk_fixed = gtk::Fixed::new();
        let desktop = Desktop::new(&self.shell, Some(monitor));
        desk_fixed.put(&desktop.widget(), 0.0, 0.0);
        desk_win.set_child(Some(&desk_fixed));
        self.desktop_keys_on(&desk_win, &desktop);

        let sheet = surface::make(
            app,
            monitor,
            &Spec {
                namespace: "wados-sheet",
                layer: Layer::Top,
                anchor: Anchor::Fill,
                exclusive: -1,
                keyboard: KeyboardMode::None,
                height: 0,
            },
        );
        surface::stay(&sheet);
        let sheet_fixed = gtk::Fixed::new();
        let backdrop = Backdrop::new();
        sheet_fixed.put(&backdrop, 0.0, 0.0);
        sheet.set_child(Some(&sheet_fixed));
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let w = Rc::downgrade(self);
        scroll.connect_scroll(move |_, _, dy| {
            if let Some(m) = w.upgrade()
                && dy > 0.0
            {
                m.go(Page::Desktop);
            }
            glib::Propagation::Stop
        });
        sheet.add_controller(scroll);

        let o = Other { monitor: monitor.clone(), desk_win, desk_fixed, desktop, sheet, sheet_fixed, backdrop };
        o.lay_out(self.progress());
        if let Some(picture) = self.launcher.picture() {
            let (w, h) = o.size();
            o.backdrop.set(&picture, w as f64, h as f64);
        }
        o.desk_win.present();
        if self.progress() < 1.0 {
            o.sheet.present();
        }
        o
    }

    /// Where the dock's halves sit on the launcher page: (left x, right x, y).
    fn dock_at(&self) -> (f64, f64, f64) {
        let (w, h) = self.size();
        let (lw, lh) = natural(&self.chrome_halves.left());
        let (rw, _) = natural(&self.chrome_halves.right());
        let left = ((w - (lw + rw)) / 2.0).round();
        (left, left + lw, h - lh - DOCK_BOTTOM)
    }

    /// Where the bar's halves sit on it: (left x, right x, y).
    fn corners(&self) -> (f64, f64, f64) {
        let (w, _) = self.size();
        let (rw, _) = natural(&self.bar_halves.right());
        (EDGE, w - rw - EDGE, TOP)
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
        // The launcher page goes up, whole.
        let up = up_by(p, h);
        self.chrome_fixed.move_(&self.backdrop, 0.0, up);
        self.chrome_fixed.move_(&self.launcher.widget(), 0.0, up);
        let (lx, rx, y) = self.dock_at();
        self.chrome_fixed.move_(&self.chrome_halves.left(), lx, y + up);
        self.chrome_fixed.move_(&self.chrome_halves.right(), rx, y + up);
        // The desktop page comes up after it.
        let below = rise(p, h);
        self.desktop_fixed.move_(&self.desktop.widget(), 0.0, below);
        if below + BAR as f64 <= h {
            self.bar.set_margin(Edge::Top, below as i32);
            if !self.bar.is_visible() {
                self.bar.present();
            }
        } else if self.bar.is_visible() {
            self.bar.set_visible(false);
        }
        for (m, wall) in self.wallpapers.borrow().iter() {
            crate::wallpaper::slide(wall, rise(p, m.geometry().height() as f64));
        }
        for o in self.others.borrow().iter() {
            o.lay_out(p);
        }

        // What's on the bar.
        let (lx, rx, y) = self.corners();
        let (lw, _) = natural(&self.bar_halves.left());
        self.bar_fixed.move_(&self.bar_halves.left(), lx, y);
        self.bar_fixed.move_(&self.bar_halves.right(), rx, y);
        let room = ((rx - GAP) - (lx + lw + GAP)).max(0.0);
        let mid = self.middle.widget();
        mid.set_size_request(room as i32, -1);
        self.bar_fixed.move_(&mid, lx + lw + GAP, y);

        let to_desktop = self.spring.borrow().as_ref().is_some_and(|s| s.target() == 1.0);
        if to_desktop && p >= 1.0 && self.chrome.is_visible() {
            // Landed: nothing's left to see on chrome or the sheets, and
            // they'd keep the pointer from what's under them.
            let chrome = self.chrome.downgrade();
            let sheets: Vec<_> = self.others.borrow().iter().map(|o| o.sheet.downgrade()).collect();
            glib::idle_add_local_once(move || {
                if let Some(c) = chrome.upgrade() {
                    c.set_visible(false);
                }
                for s in sheets.iter().filter_map(|s| s.upgrade()) {
                    s.set_visible(false);
                }
            });
            self.desktop.shown();
            for o in self.others.borrow().iter() {
                o.desktop.shown();
            }
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
                for o in self.others.borrow().iter() {
                    o.sheet.present();
                }
                spring.to(0.0);
                self.launcher.shown();
            }
        }
    }

    pub fn toggle(&self) {
        self.go(if self.page.get() == Page::Launcher { Page::Desktop } else { Page::Launcher });
    }

    /// A window coming forward (opened, or brought up) is on the desktop, so
    /// the launcher goes down to it, as the spike's Machine.svelte does: the
    /// launcher would cover it otherwise (WadBrowser opened from a link, a
    /// second launch handing over to the first).
    fn follow_windows(self: &Rc<Self>, shell: &Rc<Shell>) {
        let seen: RefCell<Option<Forward>> = RefCell::new(None);
        let w = Rc::downgrade(self);
        shell.placer.on_windows(move |list| {
            let Some(m) = w.upgrade() else { return };
            let mut seen = seen.borrow_mut();
            let came = seen.as_ref().is_some_and(|before| before.came_forward(list));
            *seen = Some(Forward::of(list));
            if came && m.page() == Page::Launcher {
                m.go(Page::Desktop);
            }
        });
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

        let (win, desktop) = (self.desktop_win.clone(), self.desktop.clone());
        self.desktop_keys_on(&win, &desktop);
    }

    /// A desktop's own keys first (desktop::Desktop::key), then PageUp back
    /// up; a scroll up too.
    fn desktop_keys_on(self: &Rc<Self>, win: &gtk::ApplicationWindow, desktop: &Rc<Desktop>) {
        let keys = gtk::EventControllerKey::new();
        let (w, d) = (Rc::downgrade(self), Rc::downgrade(desktop));
        keys.connect_key_pressed(move |_, key, _, mods| {
            let (Some(m), Some(d)) = (w.upgrade(), d.upgrade()) else { return glib::Propagation::Proceed };
            if d.key(key, mods) {
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::Page_Up {
                m.go(Page::Launcher);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        win.add_controller(keys);
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
        win.add_controller(scroll);
    }
}

pub fn natural(w: &impl IsA<gtk::Widget>) -> (f64, f64) {
    let (_, nw, _, _) = w.measure(gtk::Orientation::Horizontal, -1);
    let (_, nh, _, _) = w.measure(gtk::Orientation::Vertical, nw);
    (nw as f64, nh as f64)
}

/// The windows as they were, to tell when one comes forward.
struct Forward {
    ids: Vec<u32>,
    front: Option<u32>,
}

impl Forward {
    fn of(list: &[crate::wm::Top]) -> Self {
        Forward {
            ids: list.iter().map(|t| t.id).collect(),
            front: list.iter().find(|t| t.activated && !t.minimized).map(|t| t.id),
        }
    }

    /// A window opened (not minimized), or another one is in front now.
    fn came_forward(&self, list: &[crate::wm::Top]) -> bool {
        let opened = list.iter().any(|t| !t.minimized && !self.ids.contains(&t.id));
        let front = list.iter().find(|t| t.activated && !t.minimized).map(|t| t.id);
        opened || (front.is_some() && front != self.front)
    }
}

#[cfg(test)]
mod tests {
    use super::{Forward, rise, up_by};
    use crate::wm::Top;

    #[test]
    fn the_pages_scroll_a_page_for_a_page() {
        // The launcher, and halfway: the desktop's top meets the launcher's bottom.
        assert_eq!((up_by(0.0, 1000.0), rise(0.0, 1000.0)), (0.0, 1000.0));
        assert_eq!((up_by(0.5, 1000.0), rise(0.5, 1000.0)), (-500.0, 500.0));
        // Landed, and a spring's little overshoot goes no further.
        assert_eq!((up_by(1.0, 1000.0), rise(1.0, 1000.0)), (-1000.0, 0.0));
        assert_eq!((up_by(1.02, 1000.0), rise(1.02, 1000.0)), (-1000.0, 0.0));
    }

    fn top(id: u32, activated: bool, minimized: bool) -> Top {
        Top {
            id,
            app_id: "io.wados.Browser".into(),
            title: String::new(),
            outputs: vec![],
            activated,
            maximized: false,
            minimized,
            fullscreen: false,
        }
    }

    #[test]
    fn a_window_coming_forward_is_seen() {
        let before = Forward::of(&[top(1, true, false)]);
        // Nothing changed, or the one in front went into the background.
        assert!(!before.came_forward(&[top(1, true, false)]));
        assert!(!before.came_forward(&[top(1, false, true)]));
        // A window opened (in front or not), or another one brought up.
        assert!(before.came_forward(&[top(1, false, false), top(2, true, false)]));
        assert!(before.came_forward(&[top(1, true, false), top(2, false, false)]));
        let two = Forward::of(&[top(1, true, false), top(2, false, false)]);
        assert!(two.came_forward(&[top(1, false, false), top(2, true, false)]));
        // One opened minimized (a wadspace in the background) isn't.
        assert!(!before.came_forward(&[top(1, true, false), top(3, false, true)]));
    }
}
