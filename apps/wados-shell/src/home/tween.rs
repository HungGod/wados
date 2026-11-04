//! The launcher's motion as the spike's CSS transitions run it: a value
//! going from where it is to its target in a set time, on a cubic-bezier
//! curve, by the clock (a slow frame skips ahead rather than slowing it
//! down). Retargeted midway, it starts again from where it is, as CSS does.
//! The row's cards move in 0.45 s on (0.2, 0.85, 0.25, 1); the picture
//! behind fades in 0.5 s, ease.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::{glib, prelude::*};

/// A CSS cubic-bezier timing function.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bezier(pub f64, pub f64, pub f64, pub f64);

/// The row's: cubic-bezier(0.2, 0.85, 0.25, 1).
pub const MOVE: Bezier = Bezier(0.2, 0.85, 0.25, 1.0);
/// CSS's `ease`.
pub const EASE: Bezier = Bezier(0.25, 0.1, 0.25, 1.0);

impl Bezier {
    fn axis(a: f64, b: f64, t: f64) -> f64 {
        // B(t) with P0 = 0, P3 = 1.
        let u = 1.0 - t;
        3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
    }

    /// How far along (0–1) it is at `x` of its time (0–1).
    pub fn at(&self, x: f64) -> f64 {
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        // x(t) only rises: halve until t is close.
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if Self::axis(self.0, self.2, mid) < x {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Self::axis(self.1, self.3, (lo + hi) / 2.0)
    }
}

struct State {
    value: Cell<f64>,
    from: Cell<f64>,
    to: Cell<f64>,
    /// When it started (the frame clock's µs; 0: on the next frame).
    start: Cell<i64>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
    apply: Box<dyn Fn(f64)>,
}

/// A value moving on `widget`'s frame clock; `apply` gets it every frame.
#[derive(Clone)]
pub struct Tween {
    widget: glib::WeakRef<gtk::Widget>,
    time: f64,
    ease: Bezier,
    st: Rc<State>,
}

impl Tween {
    pub fn new(
        widget: &impl IsA<gtk::Widget>,
        time: Duration,
        ease: Bezier,
        start: f64,
        apply: impl Fn(f64) + 'static,
    ) -> Self {
        let st = Rc::new(State {
            value: Cell::new(start),
            from: Cell::new(start),
            to: Cell::new(start),
            start: Cell::new(0),
            tick: RefCell::default(),
            apply: Box::new(apply),
        });
        Self { widget: widget.upcast_ref::<gtk::Widget>().downgrade(), time: time.as_secs_f64(), ease, st }
    }

    pub fn value(&self) -> f64 {
        self.st.value.get()
    }

    pub fn target(&self) -> f64 {
        self.st.to.get()
    }

    /// Still on its way.
    pub fn moving(&self) -> bool {
        self.st.tick.borrow().is_some()
    }

    /// There at once.
    pub fn jump(&self, to: f64) {
        if let Some(id) = self.st.tick.borrow_mut().take() {
            id.remove();
        }
        self.st.value.set(to);
        self.st.from.set(to);
        self.st.to.set(to);
        (self.st.apply)(to);
    }

    /// On its way to `to`, from where it is.
    pub fn to(&self, to: f64) {
        if to == self.st.to.get() && (self.moving() || self.st.value.get() == to) {
            return;
        }
        let Some(w) = self.widget.upgrade() else { return };
        if !w.settings().is_gtk_enable_animations() || !w.is_mapped() || self.time <= 0.0 {
            self.jump(to);
            return;
        }
        self.st.from.set(self.st.value.get());
        self.st.to.set(to);
        self.st.start.set(0);
        if self.moving() {
            return;
        }
        let (st, time, ease) = (self.st.clone(), self.time, self.ease);
        let id = w.add_tick_callback(move |_, clock| {
            let now = clock.frame_time();
            if st.start.get() == 0 {
                st.start.set(now);
            }
            let p = ((now - st.start.get()) as f64 / 1e6 / time).clamp(0.0, 1.0);
            let (from, to) = (st.from.get(), st.to.get());
            let v = if p >= 1.0 { to } else { from + (to - from) * ease.at(p) };
            st.value.set(v);
            (st.apply)(v);
            if p >= 1.0 {
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
    fn the_curves_run_from_nothing_to_all_of_it() {
        for b in [MOVE, EASE] {
            assert_eq!(b.at(0.0), 0.0);
            assert_eq!(b.at(1.0), 1.0);
            let mut last = 0.0;
            for i in 1..20 {
                let v = b.at(i as f64 / 20.0);
                assert!(v >= last, "{b:?} at {i}");
                last = v;
            }
        }
        // The row's is quick to start and slow to land: most of the way by half its time.
        assert!(MOVE.at(0.5) > 0.85);
        // ease at a quarter of its time (CSS: about 0.41).
        assert!((EASE.at(0.25) - 0.4094).abs() < 0.01);
        // linear is linear.
        assert!((Bezier(0.0, 0.0, 1.0, 1.0).at(0.3) - 0.3).abs() < 1e-6);
    }
}
