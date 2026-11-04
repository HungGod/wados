//! Children in lines (CSS's `display: flex; flex-wrap: wrap; gap`): each
//! as wide as it wants (no wider than the line), the next line when it
//! doesn't fit, centred in its line's height. GTK has no wrap box (a FlowBox
//! lines its children up in columns), so this is one: the Builder's pills,
//! chips and tiles.
//!
//! [`Wrap::set`] keeps its children in step with a list by key, as
//! `wad_gtk::keyed` does for a box: a child that stays keeps its widget.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use gtk::{glib, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Wrap {
        pub gap: Cell<i32>,
        pub line_gap: Cell<i32>,
        pub keyed: RefCell<Vec<(String, gtk::Widget)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Wrap {
        const NAME: &'static str = "WadWrap";
        type Type = super::Wrap;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("wad-wrap");
        }
    }

    impl ObjectImpl for Wrap {
        fn dispose(&self) {
            while let Some(c) = self.obj().first_child() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for Wrap {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let kids = self.kids();
            if kids.is_empty() {
                return (0, 0, -1, -1);
            }
            match orientation {
                gtk::Orientation::Horizontal => {
                    let mut min = 0;
                    let mut nat = 0;
                    for (i, k) in kids.iter().enumerate() {
                        let (m, n, _, _) = k.measure(gtk::Orientation::Horizontal, -1);
                        min = min.max(m);
                        nat += n + if i > 0 { self.gap.get() } else { 0 };
                    }
                    (min, nat, -1, -1)
                }
                _ => {
                    let width = if for_size < 0 { i32::MAX / 4 } else { for_size };
                    let h = self.lines(&kids, width).last().map(|l| l.y + l.height).unwrap_or(0);
                    (h, h, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let kids = self.kids();
            for line in self.lines(&kids, width) {
                for (k, x, w, h) in line.items {
                    let y = line.y + (line.height - h) / 2;
                    k.size_allocate(&gtk::Allocation::new(x, y, w, h), -1);
                }
            }
        }
    }

    pub struct Line {
        pub y: i32,
        pub height: i32,
        pub items: Vec<(gtk::Widget, i32, i32, i32)>,
    }

    impl Wrap {
        fn kids(&self) -> Vec<gtk::Widget> {
            let mut out = vec![];
            let mut c = self.obj().first_child();
            while let Some(k) = c {
                if k.should_layout() {
                    out.push(k.clone());
                }
                c = k.next_sibling();
            }
            out
        }

        /// The children in lines `width` wide: each at its natural width (no wider than a line).
        fn lines(&self, kids: &[gtk::Widget], width: i32) -> Vec<Line> {
            let (gap, line_gap) = (self.gap.get(), self.line_gap.get());
            let mut lines: Vec<Line> = vec![];
            let mut cur = Line { y: 0, height: 0, items: vec![] };
            let mut x = 0;
            for k in kids {
                let (min, nat, _, _) = k.measure(gtk::Orientation::Horizontal, -1);
                let w = nat.min(width).max(min);
                if !cur.items.is_empty() && x + w > width {
                    let y = cur.y + cur.height + line_gap;
                    lines.push(std::mem::replace(&mut cur, Line { y, height: 0, items: vec![] }));
                    x = 0;
                }
                let (_, h, _, _) = k.measure(gtk::Orientation::Vertical, w);
                cur.height = cur.height.max(h);
                cur.items.push((k.clone(), x, w, h));
                x += w + gap;
            }
            if !cur.items.is_empty() {
                lines.push(cur);
            }
            lines
        }
    }
}

glib::wrapper! {
    pub struct Wrap(ObjectSubclass<imp::Wrap>) @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Wrap {
    fn default() -> Self {
        Self::new(6, 6)
    }
}

impl Wrap {
    /// `gap` between children in a line, `line_gap` between lines.
    pub fn new(gap: i32, line_gap: i32) -> Self {
        let me: Self = glib::Object::new();
        me.imp().gap.set(gap);
        me.imp().line_gap.set(line_gap);
        me
    }

    pub fn append(&self, w: &impl IsA<gtk::Widget>) {
        w.as_ref().insert_before(self, None::<&gtk::Widget>);
        self.queue_resize();
    }

    /// `w` just before `before` (one of its children).
    pub fn insert_before(&self, w: &impl IsA<gtk::Widget>, before: &impl IsA<gtk::Widget>) {
        w.as_ref().insert_before(self, Some(before.as_ref()));
        self.queue_resize();
    }

    /// Its children as `items` say, in order, by key: a key it has keeps its
    /// widget (`update`d), a new one is `make`d, a gone one goes.
    pub fn set<T>(&self, items: &[(String, T)], make: impl Fn(&T) -> gtk::Widget, update: impl Fn(&gtk::Widget, &T)) {
        let mut old: HashMap<String, gtk::Widget> = self.imp().keyed.borrow_mut().drain(..).collect();
        let mut rows = Vec::with_capacity(items.len());
        for (k, t) in items {
            let w = match old.remove(k) {
                Some(w) => {
                    update(&w, t);
                    w
                }
                None => make(t),
            };
            rows.push((k.clone(), w));
        }
        for (_, w) in old {
            w.unparent();
        }
        let mut prev: Option<gtk::Widget> = None;
        for (_, w) in &rows {
            let here = w.parent().as_ref() == Some(self.upcast_ref());
            if !here || w.prev_sibling() != prev {
                if !here && w.parent().is_some() {
                    w.unparent();
                }
                // Moved, if it's here already.
                w.insert_after(self, prev.as_ref());
            }
            prev = Some(w.clone());
        }
        *self.imp().keyed.borrow_mut() = rows;
        self.queue_resize();
    }

    /// Its keyed children.
    pub fn keyed(&self) -> Vec<(String, gtk::Widget)> {
        self.imp().keyed.borrow().clone()
    }
}
