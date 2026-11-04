//! Under the card in focus, its buttons (Home.svelte's `.acts`): round
//! icons on a dark pill, each named in its tooltip, like a pad's. The one
//! the keys are on is lit; what isn't here yet (Focus, streams) is dimmed
//! and does nothing.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;

use super::model::Action;

/// A button's side, and its icon's.
const SIDE: i32 = 52;
const ICON: i32 = 21;

pub struct Buttons {
    pub root: gtk::Box,
    rows: RefCell<Vec<(Action, gtk::Box)>>,
}

impl Buttons {
    pub fn new() -> Self {
        let root = gtk::Box::builder().spacing(10).css_classes(["home-acts"]).build();
        Self { root, rows: RefCell::default() }
    }

    /// Shows `actions` (made again only if they changed), the keys on `keyed`;
    /// a click on one calls `clicked` with its place.
    pub fn set(&self, actions: &[Action], keyed: Option<usize>, clicked: &Rc<dyn Fn(usize)>) {
        let same = {
            let rows = self.rows.borrow();
            rows.len() == actions.len() && rows.iter().zip(actions).all(|((a, _), b)| a == b)
        };
        if !same {
            while let Some(c) = self.root.first_child() {
                self.root.remove(&c);
            }
            let mut rows = Vec::with_capacity(actions.len());
            for (i, a) in actions.iter().enumerate() {
                let b = button(a);
                let click = gtk::GestureClick::new();
                let f = clicked.clone();
                click.connect_released(move |g, n, _, _| {
                    if n == 1 {
                        g.set_state(gtk::EventSequenceState::Claimed);
                        f(i);
                    }
                });
                b.add_controller(click);
                self.root.append(&b);
                rows.push((a.clone(), b));
            }
            *self.rows.borrow_mut() = rows;
        }
        for (i, (_, b)) in self.rows.borrow().iter().enumerate() {
            if keyed == Some(i) {
                b.add_css_class("keys");
            } else {
                b.remove_css_class("keys");
            }
        }
        self.root.set_visible(!actions.is_empty());
    }

    /// The buttons' widgets (for wadsctl's bounds).
    pub fn widgets(&self) -> Vec<gtk::Box> {
        self.rows.borrow().iter().map(|(_, b)| b.clone()).collect()
    }
}

fn button(a: &Action) -> gtk::Box {
    let b = gtk::Box::builder().css_classes(["home-act"]).halign(gtk::Align::Center).valign(gtk::Align::Center).build();
    b.set_size_request(SIDE, SIDE);
    b.set_tooltip_text(Some(&a.hint));
    for (on, class) in [(a.primary, "primary"), (a.danger, "danger"), (!a.enabled, "off")] {
        if on {
            b.add_css_class(class);
        }
    }
    let icon: gtk::Widget =
        if a.icon == "tailscale" { tailscale_mark(ICON).upcast() } else { wad_gtk::Icon::new(a.icon, ICON).upcast() };
    icon.set_hexpand(true);
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    b.append(&icon);
    b
}

/// Tailscale's mark (the spike's TailscaleMark.svelte): its nine dots, the
/// T lit, in the text's colour.
fn tailscale_mark(size: i32) -> gtk::DrawingArea {
    let d = gtk::DrawingArea::builder().content_width(size).content_height(size).build();
    d.set_draw_func(|area, cr, w, h| {
        let c = area.color();
        let k = w.min(h) as f64 / 24.0;
        let (ox, oy) = ((w as f64 - 24.0 * k) / 2.0, (h as f64 - 24.0 * k) / 2.0);
        for r in 0..3 {
            for col in 0..3 {
                let lit = r == 1 || (r == 2 && col == 1);
                let a = c.alpha() as f64 * if lit { 1.0 } else { 0.3 };
                cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, a);
                let (x, y) = (ox + (4.0 + col as f64 * 8.0) * k, oy + (4.0 + r as f64 * 8.0) * k);
                cr.arc(x, y, 2.9 * k, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
        }
    });
    d
}
