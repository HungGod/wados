//! A full-screen wadspace's bar (`shell/Fullscreen.svelte`), on each display
//! with one: a few pixels along the top edge that, touched, bring the bar
//! down (its name, what's in front, back to the desktop, in a window, to
//! another display, stream, stop). It shows for a moment when the wadspace
//! comes up full screen, then tucks away. Everywhere else, clicks go
//! through to the wadspace.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{KeyboardMode, Layer};
use wad_gtk::Icon;
use wad_model::SessionDisplay;

use crate::machine::Page;
use crate::shell::Shell;
use crate::surface::{self, Anchor, Spec};
use crate::wm::Top;

/// The strip along the top that brings it down.
const EDGE: i32 = 4;
const HEIGHT: i32 = 64;

/// The wadspace full screen on `output`, if one is (its window, in front or not, but not minimized).
pub fn fullscreen_on<'a>(windows: &'a [Top], output: &str) -> Option<&'a Top> {
    windows
        .iter()
        .find(|t| t.wadspace().is_some() && t.fullscreen && !t.minimized && t.outputs.iter().any(|o| o == output))
}

/// What's in front inside it, if its window says (not the nested labwc's own title).
pub fn in_front_of(t: &Top, name: &str) -> Option<String> {
    let title = t.title.trim();
    (!title.is_empty() && title != name && !title.starts_with("labwc")).then(|| title.to_string())
}

pub struct Reveal {
    bars: Vec<Rc<Bar>>,
}

impl Reveal {
    pub fn new(shell: &Rc<Shell>) -> Self {
        let monitors: Vec<gdk::Monitor> =
            std::iter::once(shell.primary.clone()).chain(shell.others.iter().cloned()).collect();
        let n = monitors.len();
        let bars = monitors.iter().enumerate().map(|(i, m)| Bar::new(shell, m, i + 1, n)).collect();
        Self { bars }
    }

    pub fn windows_changed(&self) {
        for b in &self.bars {
            b.update();
        }
    }

    /// The bar on display `n` (1: the primary) down, or up.
    pub fn show(&self, n: usize, on: bool) -> bool {
        match self.bars.get(n.saturating_sub(1)) {
            Some(b) if b.ws.borrow().is_some() => {
                b.pinned.set(on);
                b.reveal(on);
                true
            }
            _ => false,
        }
    }

    pub fn state(&self) -> String {
        let s: Vec<String> = self
            .bars
            .iter()
            .map(|b| {
                format!(
                    "{}={}{}",
                    b.connector,
                    b.ws.borrow().clone().unwrap_or_else(|| "-".into()),
                    if b.shown.get() { "(down)" } else { "" }
                )
            })
            .collect();
        format!("ok {}", s.join(" "))
    }

    /// The wadspace full screen on the primary, if one is.
    pub fn primary_ws(&self) -> Option<String> {
        self.bars.first().and_then(|b| b.ws.borrow().clone())
    }
}

struct Bar {
    shell: Rc<Shell>,
    win: gtk::ApplicationWindow,
    bar: gtk::Box,
    name: gtk::Label,
    front: gtk::Label,
    connector: String,
    ws: RefCell<Option<String>>,
    shown: Cell<bool>,
    pinned: Cell<bool>,
    hiding: RefCell<Option<glib::SourceId>>,
}

impl Bar {
    fn new(shell: &Rc<Shell>, monitor: &gdk::Monitor, n: usize, of: usize) -> Rc<Self> {
        let win = surface::make(
            &shell.app,
            monitor,
            &Spec {
                namespace: "wados-reveal",
                layer: Layer::Overlay,
                anchor: Anchor::Top,
                exclusive: -1,
                keyboard: KeyboardMode::None,
                height: HEIGHT,
            },
        );
        let bar = gtk::Box::builder()
            .spacing(8)
            .css_classes(["ov-fs"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Start)
            .margin_top(8)
            .build();
        bar.set_size_request(640, -1);
        let name = gtk::Label::builder().css_classes(["ov-fs-name"]).build();
        let head = gtk::Box::builder().spacing(8).build();
        head.append(&Icon::new("monitor-play", 15));
        head.append(&name);
        bar.append(&head);
        let front = gtk::Label::builder()
            .css_classes(["ov-fs-look"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(28)
            .build();
        bar.append(&front);
        bar.append(&gtk::Box::builder().hexpand(true).build());
        let me = Rc::new(Self {
            shell: shell.clone(),
            win: win.clone(),
            bar: bar.clone(),
            name,
            front,
            connector: monitor.connector().map(|c| c.to_string()).unwrap_or_default(),
            ws: RefCell::default(),
            shown: Cell::new(false),
            pinned: Cell::new(false),
            hiding: RefCell::default(),
        });
        let button = |icon: &'static str, text: &str, tip: &str| {
            let r = gtk::Box::builder().spacing(6).build();
            r.append(&Icon::new(icon, 15));
            r.append(&gtk::Label::new(Some(text)));
            let b = gtk::Button::builder().child(&r).css_classes(["ov-fs-b"]).tooltip_text(tip).build();
            bar.append(&b);
            b
        };
        let w = Rc::downgrade(&me);
        button("house", "Spaces", "Back to the desktop (it keeps running)").connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.to_desktop();
            }
        });
        let w = Rc::downgrade(&me);
        button("app-window", "In a window", "In a window").connect_clicked(move |_| {
            if let Some(m) = w.upgrade()
                && let Some(id) = m.ws.borrow().clone()
            {
                m.shell.open(&id, SessionDisplay::Window);
            }
        });
        if of > 1 {
            let next = n % of + 1;
            button("monitor-up", &format!("To {next}"), "Full screen on another display comes in phase 10")
                .set_sensitive(false);
        }
        button("radio", "Stream", "Streams come in phase 7").set_sensitive(false);
        let stop = button("circle-stop", "Stop", "Stop it");
        stop.add_css_class("ov-stop");
        let w = Rc::downgrade(&me);
        stop.connect_clicked(move |_| {
            if let Some(m) = w.upgrade()
                && let Some(id) = m.ws.borrow().clone()
            {
                m.shell.stop(&id);
            }
        });
        win.set_child(Some(&bar));
        // Touching the edge brings it down; leaving it lets it go.
        let motion = gtk::EventControllerMotion::new();
        let w = Rc::downgrade(&me);
        motion.connect_enter(move |_, _, _| {
            if let Some(m) = w.upgrade() {
                m.reveal(true);
            }
        });
        let w = Rc::downgrade(&me);
        motion.connect_leave(move |_| {
            if let Some(m) = w.upgrade() {
                m.hide_soon(500);
            }
        });
        win.add_controller(motion);
        let w = Rc::downgrade(&me);
        win.connect_map(move |_| {
            if let Some(m) = w.upgrade() {
                // After the first frame, when it's sized.
                let w = Rc::downgrade(&m);
                glib::idle_add_local_once(move || {
                    if let Some(m) = w.upgrade() {
                        m.shape();
                    }
                });
            }
        });
        me
    }

    fn update(self: &Rc<Self>) {
        let windows = self.shell.placer.windows();
        let now = fullscreen_on(&windows, &self.connector).cloned();
        let id = now.as_ref().and_then(|t| t.wadspace().map(str::to_string));
        let was = self.ws.replace(id.clone());
        match (&now, id) {
            (Some(t), Some(id)) => {
                let name = self.shell.store.wadspace(&id).map(|w| w.name).unwrap_or_else(|| id.clone());
                self.name.set_label(&name);
                let front = in_front_of(t, &name);
                self.front.set_label(front.as_deref().unwrap_or(""));
                self.front.set_visible(front.is_some());
                if was.as_deref() != Some(id.as_str()) {
                    // Up full screen: the bar shows for a moment, then tucks away.
                    self.win.present();
                    self.reveal(true);
                    self.pinned.set(true);
                    let w = Rc::downgrade(self);
                    glib::timeout_add_local_once(Duration::from_millis(2200), move || {
                        if let Some(m) = w.upgrade() {
                            m.pinned.set(false);
                            m.hide_soon(0);
                        }
                    });
                }
            }
            _ => {
                if was.is_some() {
                    self.shown.set(false);
                    self.win.set_visible(false);
                }
            }
        }
    }

    fn reveal(&self, on: bool) {
        if let Some(id) = self.hiding.borrow_mut().take() {
            id.remove();
        }
        self.shown.set(on);
        if on {
            self.bar.add_css_class("ov-down");
        } else {
            self.bar.remove_css_class("ov-down");
        }
        self.shape();
    }

    fn hide_soon(self: &Rc<Self>, ms: u64) {
        if self.pinned.get() {
            return;
        }
        if let Some(id) = self.hiding.borrow_mut().take() {
            id.remove();
        }
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(ms), move || {
            if let Some(m) = w.upgrade() {
                m.hiding.borrow_mut().take();
                if !m.pinned.get() {
                    m.reveal(false);
                }
            }
        });
        *self.hiding.borrow_mut() = Some(id);
    }

    /// Where it takes the pointer: the edge, and the bar while it's down.
    fn shape(&self) {
        let Some(surface) = self.win.surface() else { return };
        let width = self.win.width().max(1);
        let region = gtk::cairo::Region::create_rectangle(&gtk::cairo::RectangleInt::new(0, 0, width, EDGE));
        if self.shown.get() {
            // Where it is once it's down (not where its slide has it now).
            let (_, bw, _, _) = self.bar.measure(gtk::Orientation::Horizontal, -1);
            let (_, bh, _, _) = self.bar.measure(gtk::Orientation::Vertical, bw);
            let bw = bw.max(self.bar.width());
            let _ = region.union_rectangle(&gtk::cairo::RectangleInt::new(
                ((width - bw) / 2).max(0),
                0,
                bw.min(width),
                self.bar.margin_top() + bh + 4,
            ));
        }
        surface.set_input_region(Some(&region));
    }

    /// Spaces: it goes on in the background, and the desktop comes up.
    fn to_desktop(&self) {
        if let Some(id) = self.ws.borrow().clone() {
            self.shell.open(&id, SessionDisplay::Background);
        }
        self.shell.go(Page::Desktop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top(id: u32, app: &str, out: &str, fullscreen: bool) -> Top {
        Top { id, app_id: app.into(), outputs: vec![out.into()], fullscreen, ..Top::default() }
    }

    #[test]
    fn only_a_wadspace_full_screen_on_that_display() {
        let w = vec![
            top(1, "io.wados.Terminal", "eDP-1", true),
            top(2, "io.wadspaces.studio", "eDP-1", false),
            top(3, "io.wadspaces.play", "DP-1", true),
        ];
        assert_eq!(fullscreen_on(&w, "eDP-1"), None);
        assert_eq!(fullscreen_on(&w, "DP-1").map(|t| t.id), Some(3));
        let mut min = top(4, "io.wadspaces.studio", "eDP-1", true);
        min.minimized = true;
        assert_eq!(fullscreen_on(&[min], "eDP-1"), None);
    }

    #[test]
    fn what_is_in_front() {
        let mut t = top(1, "io.wadspaces.studio", "eDP-1", true);
        t.title = "labwc - WL-1".into();
        assert_eq!(in_front_of(&t, "Studio"), None);
        t.title = "Obsidian".into();
        assert_eq!(in_front_of(&t, "Studio").as_deref(), Some("Obsidian"));
        t.title = "Studio".into();
        assert_eq!(in_front_of(&t, "Studio"), None);
    }
}
