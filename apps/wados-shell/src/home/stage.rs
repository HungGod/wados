//! The launcher's stage: a container that puts each child where it's told,
//! at a scale (the carousel's cards), without growing for what's off its
//! edges (a GtkFixed measures every child it holds, through its transform,
//! and would make the chrome wider than the screen for the cards off it).
//! Its own size is what machine.rs asks of it.

use std::cell::RefCell;

use gtk::{glib, graphene, gsk, prelude::*, subclass::prelude::*};

/// Where a child goes: the point `(x, y)` is where its own point
/// `(ax, ay)` (fractions of its size: 0.5, 0.5 its middle) lands, scaled
/// `scale` about that point. `w`/`h` -1: its natural size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub x: f64,
    pub y: f64,
    pub ax: f64,
    pub ay: f64,
    pub w: i32,
    pub h: i32,
    pub scale: f64,
}

impl Place {
    /// Its top left at `(x, y)`, its natural size.
    pub fn at(x: f64, y: f64) -> Self {
        Self { x, y, ax: 0.0, ay: 0.0, w: -1, h: -1, scale: 1.0 }
    }

    /// Its own point `(ax, ay)` at `(x, y)`.
    pub fn anchored(x: f64, y: f64, ax: f64, ay: f64) -> Self {
        Self { x, y, ax, ay, w: -1, h: -1, scale: 1.0 }
    }

    pub fn sized(self, w: i32, h: i32) -> Self {
        Self { w, h, ..self }
    }

    pub fn scaled(self, scale: f64) -> Self {
        Self { scale, ..self }
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Stage {
        pub places: RefCell<Vec<(gtk::Widget, Place)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Stage {
        const NAME: &'static str = "WadosHomeStage";
        type Type = super::Stage;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("home-stage");
        }
    }

    impl ObjectImpl for Stage {
        fn dispose(&self) {
            while let Some(c) = self.obj().first_child() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for Stage {
        fn measure(&self, _: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            // Its size is what it's asked for (machine.rs's size request).
            (0, 0, -1, -1)
        }

        fn size_allocate(&self, _: i32, _: i32, _: i32) {
            // Its menus (popovers) are placed by GTK, once told.
            let mut c = self.obj().first_child();
            while let Some(w) = c {
                if let Some(p) = w.downcast_ref::<gtk::Popover>() {
                    p.present();
                }
                c = w.next_sibling();
            }
            for (child, p) in self.places.borrow().iter() {
                if !child.should_layout() {
                    continue;
                }
                let (min_w, nat_w, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
                let w = if p.w >= 0 { p.w.max(min_w) } else { nat_w };
                let (min_h, nat_h, _, _) = child.measure(gtk::Orientation::Vertical, w);
                let h = if p.h >= 0 { p.h.max(min_h) } else { nat_h };
                let (sw, sh) = (w as f64 * p.scale, h as f64 * p.scale);
                let (x, y) = (p.x - p.ax * sw, p.y - p.ay * sh);
                let t = gsk::Transform::new()
                    .translate(&graphene::Point::new(x as f32, y as f32))
                    .scale(p.scale as f32, p.scale as f32);
                child.allocate(w, h, -1, Some(t));
            }
        }
    }
}

glib::wrapper! {
    pub struct Stage(ObjectSubclass<imp::Stage>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Stage {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Stage {
    /// Adds `child` on top of the rest, at `p`.
    pub fn put(&self, child: &impl IsA<gtk::Widget>, p: Place) {
        let child = child.upcast_ref::<gtk::Widget>();
        child.set_parent(self);
        self.imp().places.borrow_mut().push((child.clone(), p));
        self.queue_allocate();
    }

    /// Adds `child` under `above` (or on top, with none), at `p`.
    pub fn put_under(&self, child: &impl IsA<gtk::Widget>, above: Option<&gtk::Widget>, p: Place) {
        let child = child.upcast_ref::<gtk::Widget>();
        child.insert_before(self, above);
        self.imp().places.borrow_mut().push((child.clone(), p));
        self.queue_allocate();
    }

    /// Moves `child` to `p`.
    pub fn place(&self, child: &impl IsA<gtk::Widget>, p: Place) {
        let child = child.upcast_ref::<gtk::Widget>();
        let mut places = self.imp().places.borrow_mut();
        if let Some(e) = places.iter_mut().find(|(c, _)| c == child)
            && e.1 != p
        {
            e.1 = p;
            drop(places);
            self.queue_allocate();
        }
    }

    pub fn remove(&self, child: &impl IsA<gtk::Widget>) {
        let child = child.upcast_ref::<gtk::Widget>();
        self.imp().places.borrow_mut().retain(|(c, _)| c != child);
        if child.parent().as_ref() == Some(self.upcast_ref()) {
            child.unparent();
        }
    }
}
