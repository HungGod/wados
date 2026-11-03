//! Layer-shell surfaces (gtk4-layer-shell): every part of the shell that
//! isn't an ordinary window is one of these, on one monitor, in one layer.

use gtk::{gdk, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

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

/// A new layer surface on `monitor`, not yet shown.
pub fn make(app: &gtk::Application, monitor: &gdk::Monitor, spec: &Spec) -> gtk::ApplicationWindow {
    let w = gtk::ApplicationWindow::new(app);
    w.init_layer_shell();
    w.set_namespace(Some(spec.namespace));
    w.set_layer(spec.layer);
    w.set_monitor(Some(monitor));
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
