//! Springs, as motion (framer-motion) runs them in the web app: a value
//! pulled toward its target by `stiffness`, slowed by `damping`, stepped on
//! the widget's frame clock. The same parameters give the same feel.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::{glib, prelude::*};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub stiffness: f64,
    pub damping: f64,
    pub mass: f64,
}

impl Params {
    pub const fn new(stiffness: f64, damping: f64) -> Self {
        Self { stiffness, damping, mass: 1.0 }
    }
}

/// One step of `dt` seconds (semi-implicit Euler, in small slices so a
/// stiff spring stays stable on a slow frame).
pub fn step(p: Params, x: f64, v: f64, target: f64, dt: f64) -> (f64, f64) {
    let (mut x, mut v) = (x, v);
    let n = (dt / 0.004).ceil().max(1.0);
    let h = dt / n;
    for _ in 0..n as usize {
        let a = (-p.stiffness * (x - target) - p.damping * v) / p.mass;
        v += a * h;
        x += v * h;
    }
    (x, v)
}

/// Close enough to stop.
fn settled(x: f64, v: f64, target: f64) -> bool {
    (x - target).abs() < 0.01 && v.abs() < 0.05
}

struct State {
    value: Cell<f64>,
    velocity: Cell<f64>,
    target: Cell<f64>,
    last: Cell<i64>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
    apply: Box<dyn Fn(f64)>,
}

/// A value on a spring, driven by `widget`'s frame clock; `apply` gets it
/// every frame.
#[derive(Clone)]
pub struct Spring {
    params: Params,
    widget: glib::WeakRef<gtk::Widget>,
    st: Rc<State>,
}

impl Spring {
    pub fn new(widget: &impl IsA<gtk::Widget>, params: Params, start: f64, apply: impl Fn(f64) + 'static) -> Self {
        let st = Rc::new(State {
            value: Cell::new(start),
            velocity: Cell::new(0.0),
            target: Cell::new(start),
            last: Cell::new(0),
            tick: RefCell::new(None),
            apply: Box::new(apply),
        });
        (st.apply)(start);
        Self { params, widget: widget.upcast_ref::<gtk::Widget>().downgrade(), st }
    }

    pub fn value(&self) -> f64 {
        self.st.value.get()
    }

    /// Where it's heading.
    pub fn target(&self) -> f64 {
        self.st.target.get()
    }

    /// There at once (no animation).
    pub fn jump(&self, to: f64) {
        self.stop();
        self.st.value.set(to);
        self.st.target.set(to);
        self.st.velocity.set(0.0);
        (self.st.apply)(to);
    }

    fn stop(&self) {
        if let Some(id) = self.st.tick.borrow_mut().take() {
            id.remove();
        }
    }

    /// Springs toward `to` (from wherever it is, at whatever speed).
    pub fn to(&self, to: f64) {
        self.st.target.set(to);
        let Some(w) = self.widget.upgrade() else { return };
        let animate = w.settings().is_gtk_enable_animations() && w.is_mapped();
        if !animate {
            self.jump(to);
            return;
        }
        if self.st.tick.borrow().is_some() {
            return;
        }
        self.st.last.set(0);
        let (st, p) = (self.st.clone(), self.params);
        let id = w.add_tick_callback(move |_, clock| {
            let now = clock.frame_time();
            let last = st.last.replace(now);
            let dt = if last == 0 { 1.0 / 60.0 } else { ((now - last) as f64 / 1e6).clamp(0.0, 0.05) };
            let (x, v) = step(p, st.value.get(), st.velocity.get(), st.target.get(), dt);
            let done = settled(x, v, st.target.get());
            let x = if done { st.target.get() } else { x };
            st.value.set(x);
            st.velocity.set(if done { 0.0 } else { v });
            (st.apply)(x);
            if done {
                st.tick.borrow_mut().take();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        *self.st.tick.borrow_mut() = Some(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spring_settles_on_its_target() {
        // The sidebar's: stiffness 400, damping 40 (critically damped-ish).
        let p = Params::new(400.0, 40.0);
        let (mut x, mut v) = (248.0, 0.0);
        let mut frames = 0;
        while !settled(x, v, 76.0) {
            (x, v) = step(p, x, v, 76.0, 1.0 / 60.0);
            frames += 1;
            assert!(frames < 120, "still moving after 2 s: {x} {v}");
        }
        assert!(frames > 10, "too quick to see: {frames} frames");
    }

    #[test]
    fn a_bouncy_one_overshoots_and_comes_back() {
        let p = Params::new(500.0, 10.0);
        let (mut x, mut v, mut max) = (0.0f64, 0.0f64, 0.0f64);
        for _ in 0..240 {
            (x, v) = step(p, x, v, 1.0, 1.0 / 60.0);
            max = max.max(x);
        }
        assert!(max > 1.05 && (x - 1.0).abs() < 0.01, "{max} {x}");
    }
}
