//! Fixed-size cells for the stack's desktop preview (the wrapping pills and
//! chips are `wad_gtk::Wrap`).

use std::cell::Cell;

use gtk::{glib, prelude::*, subclass::prelude::*};

mod cells_imp {
    use super::*;

    #[derive(Default)]
    pub struct Cells {
        pub cell: Cell<(i32, i32)>,
        pub rows: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Cells {
        const NAME: &'static str = "WadosStackCells";
        type Type = super::Cells;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("stack-cells");
        }
    }

    impl ObjectImpl for Cells {
        fn dispose(&self) {
            while let Some(c) = self.obj().first_child() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for Cells {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let n = self.n();
            let rows = self.rows.get().max(1);
            let (w, h) = self.cell.get();
            let v =
                if orientation == gtk::Orientation::Horizontal { w * ((n + rows - 1) / rows) } else { h * n.min(rows) };
            (v, v, -1, -1)
        }

        fn size_allocate(&self, _width: i32, _height: i32, _baseline: i32) {
            let rows = self.rows.get().max(1);
            let (w, h) = self.cell.get();
            let mut c = self.obj().first_child();
            let mut i = 0;
            while let Some(k) = c {
                if k.should_layout() {
                    // Each cell exactly its size (a long name ellipsizes), down each column, then across.
                    k.measure(gtk::Orientation::Horizontal, -1);
                    let at = gtk::graphene::Point::new(((i / rows) * w) as f32, ((i % rows) * h) as f32);
                    k.allocate(w, h, -1, Some(gtk::gsk::Transform::new().translate(&at)));
                    i += 1;
                }
                c = k.next_sibling();
            }
        }
    }

    impl Cells {
        fn n(&self) -> i32 {
            let mut n = 0;
            let mut c = self.obj().first_child();
            while let Some(k) = c {
                if k.should_layout() {
                    n += 1;
                }
                c = k.next_sibling();
            }
            n
        }
    }
}

glib::wrapper! {
    /// Cells all one size, down each column of `rows`, then across (a stack's desktop's icons).
    pub struct Cells(ObjectSubclass<cells_imp::Cells>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Cells {
    pub fn new(cell: (i32, i32), rows: i32) -> Self {
        let me: Self = glib::Object::new();
        me.imp().cell.set(cell);
        me.imp().rows.set(rows);
        me
    }

    pub fn append(&self, child: &impl IsA<gtk::Widget>) {
        child.set_parent(self);
    }
}

mod column_imp {
    use super::*;

    #[derive(Default)]
    pub struct Column {
        pub width: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Column {
        const NAME: &'static str = "WadosStackColumn";
        type Type = super::Column;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("stack-column");
        }
    }

    impl ObjectImpl for Column {
        fn dispose(&self) {
            while let Some(c) = self.obj().first_child() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for Column {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(c) = self.obj().first_child() else { return (0, 0, -1, -1) };
            if orientation == gtk::Orientation::Horizontal {
                // Its width, however wide what's in it would like to be.
                let w = self.width.get().max(c.measure(orientation, -1).0);
                return (w, w, -1, -1);
            }
            let width = if for_size < 0 { self.width.get() } else { for_size };
            let (min, nat, _, _) = c.measure(orientation, width);
            (min, nat, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(c) = self.obj().first_child() {
                c.allocate(width, height, baseline, None);
            }
        }
    }
}

glib::wrapper! {
    /// A stack's column: exactly `width` across (the board's half), so a
    /// stack alone is as wide as two side by side are, and its height is
    /// asked for at that width (a box would give it its natural width: a
    /// line of every pill).
    pub struct Column(ObjectSubclass<column_imp::Column>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Column {
    pub fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let me: Self = glib::Object::new();
        child.set_parent(&me);
        me
    }

    pub fn set_width(&self, w: i32) {
        if self.imp().width.replace(w) != w {
            self.queue_resize();
        }
    }
}
