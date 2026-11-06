//! Layer-shell surfaces (gtk4-layer-shell): every part of the shell that
//! isn't an ordinary window is one of these, on one monitor, in one layer.

use std::cell::RefCell;

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

thread_local! {
    /// The primary display, and the surfaces made on it: they follow it
    /// when it changes (`set_primary`).
    static PRIMARY: RefCell<Option<gdk::Monitor>> = const { RefCell::new(None) };
    static ON_PRIMARY: RefCell<Vec<glib::WeakRef<gtk::ApplicationWindow>>> = const { RefCell::new(Vec::new()) };
}

/// The primary display is `m` now: every surface made on the primary moves to it.
pub fn set_primary(m: &gdk::Monitor) {
    let was = PRIMARY.with(|p| p.borrow_mut().replace(m.clone()));
    if was.as_ref() == Some(m) {
        return;
    }
    let wins: Vec<gtk::ApplicationWindow> = ON_PRIMARY.with(|l| {
        let mut l = l.borrow_mut();
        l.retain(|w| w.upgrade().is_some());
        l.iter().filter_map(|w| w.upgrade()).collect()
    });
    for w in wins {
        // gtk4-layer-shell maps it again on the new monitor if it's shown.
        w.set_monitor(Some(m));
    }
}

/// Which edges a surface sticks to.
#[derive(Clone, Copy)]
pub enum Anchor {
    /// The whole monitor.
    Fill,
    /// Along the top, the monitor's width.
    Top,
}

pub struct Spec<'a> {
    pub namespace: &'a str,
    pub layer: Layer,
    pub anchor: Anchor,
    /// Room it keeps from windows (and from other surfaces that respect
    /// it): a number of pixels, 0 (respects others'), or -1 (ignores them).
    pub exclusive: i32,
    pub keyboard: KeyboardMode,
    /// Its height, for one along an edge.
    pub height: i32,
}

/// `w` stays on its own display when the primary changes (one of a
/// surface per display: a wallpaper, a display's bar), even if it's the
/// primary's now.
pub fn stay(w: &gtk::ApplicationWindow) {
    ON_PRIMARY.with(|l| l.borrow_mut().retain(|x| x.upgrade().is_some_and(|x| x != *w)));
}

/// A new layer surface on `monitor`, not yet shown. Made on the primary
/// display, it follows the primary when that changes (unless [`stay`]).
pub fn make(app: &gtk::Application, monitor: &gdk::Monitor, spec: &Spec) -> gtk::ApplicationWindow {
    let w = gtk::ApplicationWindow::new(app);
    w.init_layer_shell();
    w.set_namespace(Some(spec.namespace));
    w.set_layer(spec.layer);
    w.set_monitor(Some(monitor));
    if PRIMARY.with(|p| p.borrow().as_ref() == Some(monitor)) {
        ON_PRIMARY.with(|l| l.borrow_mut().push(w.downgrade()));
    }
    // Its display gone (unplugged, turned off), labwc closes it: it's kept
    // (hidden), to be put on another display, rather than destroyed (which
    // would end the shell with its last window).
    w.connect_close_request(|_| glib::Propagation::Stop);
    let (top, bottom, left, right) = match spec.anchor {
        Anchor::Fill => (true, true, true, true),
        Anchor::Top => (true, false, true, true),
    };
    w.set_anchor(Edge::Top, top);
    w.set_anchor(Edge::Bottom, bottom);
    w.set_anchor(Edge::Left, left);
    w.set_anchor(Edge::Right, right);
    w.set_exclusive_zone(spec.exclusive);
    w.set_keyboard_mode(spec.keyboard);
    if spec.height > 0 {
        w.set_default_size(-1, spec.height);
    }
    w.add_css_class(spec.namespace);
    w
}

/// No pointer input at all: clicks fall through to what's below.
pub fn click_through(w: &gtk::ApplicationWindow) {
    w.connect_realize(|w| {
        if let Some(s) = w.surface() {
            s.set_input_region(Some(&gtk::cairo::Region::create()));
        }
    });
}
