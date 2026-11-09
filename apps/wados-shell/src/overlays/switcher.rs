//! Super+Tab (`shell/switcher/Switcher.svelte`): the running wadspaces, and
//! only them (never the machine's own apps), the one used most lately
//! first, and last the home page itself (WadOS: the launcher and the
//! desktop), so a screen full of wadspaces can always come back to it.
//! Held, each Tab moves on one (Shift+Tab back); letting go of Super
//! brings the one picked up (full screen, or its window to the front), or
//! for the home page sends the full-screen wadspaces to the background and
//! goes there. Esc leaves things as they were.
//!
//! labwc runs `wadsctl switcher next|prev` for Super+Tab and Super+Shift+Tab
//! (host/etc/xdg/labwc/rc.xml). The switcher takes the keyboard while it's
//! open, so it sees Super let go; a Super already let go before it's up (a
//! quick Super+Tab) picks at once: the one used before this one.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};

use crate::shell::Shell;
use crate::surface::{self, Anchor, Spec};

/// The home page in the switcher's list: not a wadspace's id (those never start with one).
pub const HOME: &str = "@home";

/// WadOS's mark, on the home page's tile.
static MARK: &[u8] = include_bytes!("../../../../assets/brand/wadspaces-icon-dark-transparent.svg");

/// Which running wadspaces were used most lately (brought up, in front).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Mru {
    order: Vec<String>,
}

impl Mru {
    /// `id` was just used.
    pub fn touch(&mut self, id: &str) {
        self.order.retain(|x| x != id);
        self.order.insert(0, id.to_string());
    }

    /// The running ones (with when each was last launched), most lately used
    /// first; those not used this session after them, the latest launched
    /// first. The home page is last, and alone when nothing runs: Super+Shift+Tab
    /// from the first wadspace goes there at once, and Tab goes round to it.
    pub fn list(&self, running: &[(String, i64)]) -> Vec<String> {
        let mut r: Vec<&(String, i64)> = running.iter().collect();
        r.sort_by_key(|(id, launched)| {
            (self.order.iter().position(|x| x == id).unwrap_or(usize::MAX), std::cmp::Reverse(*launched))
        });
        let mut out: Vec<String> = r.into_iter().map(|(id, _)| id.clone()).collect();
        out.push(HOME.to_string());
        out
    }

    /// Stopped ones are forgotten.
    pub fn keep(&mut self, running: &[String]) {
        self.order.retain(|x| running.contains(x));
    }
}

/// The switcher open: what it lists and the one picked.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub ids: Vec<String>,
    pub at: usize,
}

impl Pick {
    /// Opened by Super+Tab (the next: the one used before this one) or
    /// Super+Shift+Tab (the last).
    pub fn open(ids: Vec<String>, back: bool) -> Option<Self> {
        if ids.is_empty() {
            return None;
        }
        let n = ids.len();
        let at = if n > 1 { if back { n - 1 } else { 1 } } else { 0 };
        Some(Pick { ids, at })
    }

    pub fn step(&mut self, back: bool) {
        let n = self.ids.len();
        if n > 0 {
            self.at = (self.at + if back { n - 1 } else { 1 }) % n;
        }
    }

    pub fn picked(&self) -> Option<&str> {
        self.ids.get(self.at).map(String::as_str)
    }
}

type OnPick = Box<dyn Fn(&str)>;

pub struct Switcher {
    shell: Rc<Shell>,
    win: gtk::ApplicationWindow,
    row: gtk::Box,
    pub mru: RefCell<Mru>,
    pick: RefCell<Option<Pick>>,
    /// Each opening, so a late check is for this one.
    generation: Cell<u64>,
    /// Opened to stay (`wadsctl switcher hold`): no looking for Super.
    held: Cell<bool>,
    on_pick: RefCell<Option<OnPick>>,
}

impl Switcher {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let win = surface::make(
            &shell.app,
            &shell.primary,
            &Spec {
                namespace: "wados-switcher",
                layer: Layer::Overlay,
                anchor: Anchor::Top,
                exclusive: -1,
                keyboard: KeyboardMode::Exclusive,
                height: 0,
            },
        );
        // In the middle of the screen.
        for e in [gtk4_layer_shell::Edge::Top, gtk4_layer_shell::Edge::Left, gtk4_layer_shell::Edge::Right] {
            win.set_anchor(e, false);
        }
        let row = gtk::Box::builder().spacing(10).css_classes(["ov-switcher"]).build();
        win.set_child(Some(&row));
        let me = Rc::new(Self {
            shell: shell.clone(),
            win,
            row,
            mru: RefCell::default(),
            pick: RefCell::default(),
            generation: Cell::new(0),
            held: Cell::new(false),
            on_pick: RefCell::default(),
        });
        me.keys();
        let w = Rc::downgrade(&me);
        me.win.connect_is_active_notify(move |win| {
            let Some(m) = w.upgrade() else { return };
            if win.is_active() && m.is_open() && !m.held.get() {
                // Super may have been let go before the keyboard came here.
                let g = m.generation.get();
                let w = Rc::downgrade(&m);
                glib::timeout_add_local_once(Duration::from_millis(60), move || {
                    if let Some(m) = w.upgrade()
                        && m.generation.get() == g
                        && m.is_open()
                        && !super_held()
                    {
                        tracing::debug!(
                            "switcher: Super was let go before it was up ({:?})",
                            keyboard().map(|k| k.modifier_state())
                        );
                        m.commit();
                    }
                });
            }
        });
        me
    }

    /// `f` with each one picked (the log).
    pub fn on_pick(&self, f: impl Fn(&str) + 'static) {
        *self.on_pick.borrow_mut() = Some(Box::new(f));
    }

    pub fn is_open(&self) -> bool {
        self.pick.borrow().is_some()
    }

    pub fn state(&self) -> String {
        match self.pick.borrow().as_ref() {
            Some(p) => format!("ok open at={} list={}", p.picked().unwrap_or(""), p.ids.join(",")),
            None => {
                let running = self.running();
                format!("ok closed mru={}", self.mru.borrow().list(&running).join(","))
            }
        }
    }

    fn running(&self) -> Vec<(String, i64)> {
        self.shell.store.running().into_iter().map(|w| (w.id, w.last_launched.unwrap_or(0))).collect()
    }

    /// Open, staying open until it's told (Enter, Esc, a click, Super let go, `commit`).
    pub fn hold(self: &Rc<Self>) -> String {
        let was = self.is_open();
        let s = if was { self.state() } else { self.step(false) };
        if self.is_open() {
            self.held.set(true);
        }
        s
    }

    /// Super+Tab (or Super+Shift+Tab): open it, or move on.
    pub fn step(self: &Rc<Self>, back: bool) -> String {
        if let Some(p) = self.pick.borrow_mut().as_mut() {
            p.step(back);
        } else {
            let ids = self.mru.borrow().list(&self.running());
            let Some(p) = Pick::open(ids, back) else { return "ok none running".into() };
            *self.pick.borrow_mut() = Some(p);
            self.held.set(false);
            self.generation.set(self.generation.get() + 1);
            self.win.present();
            // No keyboard here in a moment (another surface kept it): pick now,
            // rather than wait for a Super it can't see.
            let (w, g) = (Rc::downgrade(self), self.generation.get());
            glib::timeout_add_local_once(Duration::from_millis(700), move || {
                if let Some(m) = w.upgrade()
                    && m.generation.get() == g
                    && m.is_open()
                    && !m.held.get()
                    && !m.win.is_active()
                {
                    tracing::debug!("switcher: no keyboard here; picking now");
                    m.commit();
                }
            });
        }
        self.draw();
        self.state()
    }

    /// The one picked, brought up. The home page sends the full-screen
    /// wadspaces to the background (they keep running) and goes there.
    pub fn commit(&self) {
        let Some(p) = self.pick.borrow_mut().take() else { return };
        self.win.set_visible(false);
        if let Some(id) = p.picked() {
            if let Some(f) = self.on_pick.borrow().as_ref() {
                f(id);
            }
            if id == HOME {
                self.shell.go_home();
            } else {
                self.mru.borrow_mut().touch(id);
                self.shell.show(id);
            }
        }
    }

    /// Esc: as things were.
    pub fn cancel(&self) {
        if self.pick.borrow_mut().take().is_some() {
            self.win.set_visible(false);
        }
    }

    fn draw(self: &Rc<Self>) {
        while let Some(c) = self.row.first_child() {
            self.row.remove(&c);
        }
        let Some(p) = self.pick.borrow().clone() else { return };
        for (i, id) in p.ids.iter().enumerate() {
            let col = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(8)
                .css_classes(["ov-sw-item"])
                .build();
            if i == p.at {
                col.add_css_class("ov-on");
            }
            col.set_size_request(132, -1);
            if id == HOME {
                // The home page's own tile: the mark on glass, and its name.
                let tile = gtk::Box::builder()
                    .css_classes(["ov-sw-home"])
                    .halign(gtk::Align::Center)
                    .valign(gtk::Align::Center)
                    .build();
                tile.append(&wad_gtk::Svg::new(MARK, 44, 44));
                col.append(&tile);
                col.append(
                    &gtk::Label::builder()
                        .label("WadOS")
                        .css_classes(["ov-sw-name"])
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .max_width_chars(14)
                        .build(),
                );
            } else {
                let Some(ws) = self.shell.store.wadspace(id) else { continue };
                let tile = wad_gtk::WadspaceTile::new(1.0, false);
                tile.set(&ws, &self.shell.store.tile_state(id));
                col.append(&tile.root);
                col.append(
                    &gtk::Label::builder()
                        .label(&ws.name)
                        .css_classes(["ov-sw-name"])
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .max_width_chars(14)
                        .build(),
                );
            }
            // A click picks it too.
            let click = gtk::GestureClick::new();
            let (w, at) = (Rc::downgrade(self), i);
            click.connect_released(move |_, _, _, _| {
                if let Some(m) = w.upgrade() {
                    if let Some(p) = m.pick.borrow_mut().as_mut() {
                        p.at = at;
                    }
                    m.commit();
                }
            });
            col.add_controller(click);
            self.row.append(&col);
        }
        // As wide as what's in it.
        self.win.set_default_size(1, 1);
    }

    fn keys(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        let w = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(m) = w.upgrade() else { return glib::Propagation::Proceed };
            match key {
                gdk::Key::Escape => m.cancel(),
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => m.commit(),
                gdk::Key::Tab | gdk::Key::ISO_Left_Tab => {
                    m.step(key == gdk::Key::ISO_Left_Tab || mods.contains(gdk::ModifierType::SHIFT_MASK));
                }
                gdk::Key::Right => {
                    m.step(false);
                }
                gdk::Key::Left => {
                    m.step(true);
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        let w = Rc::downgrade(self);
        keys.connect_key_released(move |_, key, _, _| {
            if let Some(m) = w.upgrade()
                && matches!(key, gdk::Key::Super_L | gdk::Key::Super_R | gdk::Key::Meta_L | gdk::Key::Meta_R)
            {
                tracing::debug!("switcher: Super let go");
                m.commit();
            }
        });
        let w = Rc::downgrade(self);
        keys.connect_modifiers(move |_, mods| {
            if let Some(m) = w.upgrade()
                && m.is_open()
                && m.win.is_active()
                && !m.held.get()
                && !mods.contains(gdk::ModifierType::SUPER_MASK)
            {
                tracing::debug!("switcher: modifiers now {mods:?}");
                m.commit();
            }
            glib::Propagation::Proceed
        });
        self.win.add_controller(keys);
    }
}

fn keyboard() -> Option<gdk::Device> {
    gdk::Display::default().and_then(|d| d.default_seat()).and_then(|s| s.keyboard())
}

/// Super is down (on the seat's keyboard, as GDK last heard).
fn super_held() -> bool {
    keyboard().is_some_and(|k| k.modifier_state().contains(gdk::ModifierType::SUPER_MASK))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(ids: &[(&str, i64)]) -> Vec<(String, i64)> {
        ids.iter().map(|(i, t)| (i.to_string(), *t)).collect()
    }

    #[test]
    fn most_lately_used_first() {
        let mut m = Mru::default();
        let r = running(&[("a", 10), ("b", 30), ("c", 20)]);
        // Nothing used yet this session: the latest launched first.
        assert_eq!(m.list(&r), ["b", "c", "a", HOME]);
        m.touch("a");
        assert_eq!(m.list(&r), ["a", "b", "c", HOME]);
        m.touch("c");
        assert_eq!(m.list(&r), ["c", "a", "b", HOME]);
        m.touch("a");
        assert_eq!(m.list(&r), ["a", "c", "b", HOME]);
        // Only what's running is listed; stopped ones are forgotten.
        let r = running(&[("b", 30), ("c", 20)]);
        assert_eq!(m.list(&r), ["c", "b", HOME]);
        m.keep(&["b".into(), "c".into()]);
        assert_eq!(m.order, ["c"]);
        // The home page is there whatever runs, even when nothing does.
        assert_eq!(Mru::default().list(&[]), [HOME]);
    }

    #[test]
    fn the_home_page_is_last_and_a_step_away() {
        let m = Mru::default();
        let r = running(&[("a", 0), ("b", 0)]);
        // A quick Super+Tab still swaps the last two (the latest launched here: b)…
        let mut p = Pick::open(m.list(&r), false).unwrap();
        assert_eq!(p.picked(), Some("b"));
        // Backwards from there: the first, then round to the home page (the end).
        p.step(true);
        assert_eq!(p.picked(), Some("a"));
        p.step(true);
        assert_eq!(p.picked(), Some(HOME));
        // From the home page, forward goes round to the first.
        p.step(false);
        assert_eq!(p.picked(), Some("a"));
        // Nothing running: the home page alone, so a screen full of nothing
        // still comes back to the desktop.
        let alone = Pick::open(Mru::default().list(&[]), false).unwrap();
        assert_eq!(alone.picked(), Some(HOME));
    }

    #[test]
    fn super_tab_starts_on_the_one_before() {
        let ids = vec!["a".to_string(), "b".into(), "c".into()];
        let p = Pick::open(ids.clone(), false).unwrap();
        assert_eq!(p.picked(), Some("b"));
        let p = Pick::open(ids, true).unwrap();
        assert_eq!(p.picked(), Some("c"));
        let one = Pick::open(vec!["a".into()], false).unwrap();
        assert_eq!(one.picked(), Some("a"));
        assert!(Pick::open(vec![], false).is_none());
    }

    #[test]
    fn stepping_goes_round() {
        let mut p = Pick::open(vec!["a".into(), "b".into(), "c".into()], false).unwrap();
        p.step(false);
        assert_eq!(p.picked(), Some("c"));
        p.step(false);
        assert_eq!(p.picked(), Some("a"));
        p.step(true);
        assert_eq!(p.picked(), Some("c"));
        p.step(true);
        p.step(true);
        assert_eq!(p.picked(), Some("a"));
    }

    #[test]
    fn a_quick_super_tab_swaps_the_last_two() {
        // Tap Super+Tab: b comes up; tap again: back to a.
        let mut m = Mru::default();
        let r = running(&[("a", 0), ("b", 0), ("c", 0)]);
        m.touch("b");
        m.touch("a");
        let p = Pick::open(m.list(&r), false).unwrap();
        m.touch(p.picked().unwrap());
        assert_eq!(m.list(&r)[0], "b");
        let p = Pick::open(m.list(&r), false).unwrap();
        assert_eq!(p.picked(), Some("a"));
    }
}
