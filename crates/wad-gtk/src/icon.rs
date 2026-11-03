//! Lucide icons (assets/lucide, made by tools/lucide.mjs) drawn in the
//! widget's own CSS colour, at its size and the screen's scale: what
//! lucide-react draws in the web app. `Icon::new("play", 16)`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use gtk::{gdk, glib, graphene, prelude::*, subclass::prelude::*};

include!(concat!(env!("OUT_DIR"), "/lucide.rs"));

/// An icon's drawing, by lucide name ("circle-check"), if it's one of ours.
pub fn svg(name: &str) -> Option<&'static str> {
    LUCIDE.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// An icon drawn: its name, pixel size, colour and fill.
type Drawn = (&'static str, u32, [u8; 4], bool);

thread_local! {
    static DRAWN: RefCell<HashMap<Drawn, Option<gdk::Texture>>> = RefCell::default();
}

/// `name` drawn `px` pixels square in `color` (`filled`: its shapes filled
/// too, lucide-react's `fill-current`): cached.
pub fn texture(name: &'static str, px: u32, color: &gdk::RGBA, filled: bool) -> Option<gdk::Texture> {
    let rgba = [
        (color.red() * 255.0).round() as u8,
        (color.green() * 255.0).round() as u8,
        (color.blue() * 255.0).round() as u8,
        (color.alpha() * 255.0).round() as u8,
    ];
    let key = (name, px, rgba, filled);
    if let Some(t) = DRAWN.with(|d| d.borrow().get(&key).cloned()) {
        return t;
    }
    let svg = svg(name)?;
    let made = if filled {
        draw(&svg.replace("fill=\"none\"", "fill=\"currentColor\""), px, rgba)
    } else {
        draw(svg, px, rgba)
    };
    DRAWN.with(|d| d.borrow_mut().insert(key, made.clone()));
    made
}

/// Any SVG (a brand mark, a logo) as a texture `px` square, drawn here:
/// GTK would start a loader process (glycin) for each one.
pub fn svg_texture(svg: &[u8], px: u32) -> Option<gdk::Texture> {
    let pm = render(std::str::from_utf8(svg).ok()?, px)?;
    Some(premultiplied(pm, px, px))
}

/// An SVG `height` pixels tall, as wide as its shape makes it.
pub fn svg_texture_tall(svg: &[u8], height: u32) -> Option<gdk::Texture> {
    use resvg::{tiny_skia, usvg};
    let text = std::str::from_utf8(svg).ok()?;
    let tree = usvg::Tree::from_str(text, &options(text)).ok()?;
    let k = height as f32 / tree.size().height();
    let width = (tree.size().width() * k).ceil() as u32;
    let mut pm = tiny_skia::Pixmap::new(width, height)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(k, k), &mut pm.as_mut());
    Some(premultiplied(pm, width, height))
}

/// How SVGs are read: with the system's fonts when one has text in it (the
/// logos' lockups), looked up once; icons never need them.
fn options(svg: &str) -> resvg::usvg::Options<'static> {
    use std::sync::{Arc, OnceLock};
    static FONTS: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    let mut o = resvg::usvg::Options::default();
    if svg.contains("<text") {
        o.fontdb = FONTS
            .get_or_init(|| {
                let mut db = resvg::usvg::fontdb::Database::new();
                db.load_system_fonts();
                Arc::new(db)
            })
            .clone();
    }
    o
}

fn render(svg: &str, px: u32) -> Option<resvg::tiny_skia::Pixmap> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(svg, &options(svg)).ok()?;
    let mut pm = tiny_skia::Pixmap::new(px, px)?;
    let (w, h) = (tree.size().width(), tree.size().height());
    let k = px as f32 / w.max(h);
    // Centred in its square.
    let t = tiny_skia::Transform::from_row(k, 0.0, 0.0, k, (px as f32 - w * k) / 2.0, (px as f32 - h * k) / 2.0);
    resvg::render(&tree, t, &mut pm.as_mut());
    Some(pm)
}

fn premultiplied(pm: resvg::tiny_skia::Pixmap, w: u32, h: u32) -> gdk::Texture {
    let bytes = glib::Bytes::from_owned(pm.take());
    gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8Premultiplied, &bytes, w as usize * 4)
        .upcast()
}

fn draw(svg: &str, px: u32, [r, g, b, a]: [u8; 4]) -> Option<gdk::Texture> {
    use resvg::tiny_skia;
    let svg = svg.replace("currentColor", &format!("#{r:02x}{g:02x}{b:02x}"));
    let mut pm = render(&svg, px)?;
    // The colour's own alpha, on top of the drawing's.
    if a < 255 {
        for p in pm.pixels_mut() {
            let scale = |v: u8| ((v as u16 * a as u16) / 255) as u8;
            *p = tiny_skia::PremultipliedColorU8::from_rgba(
                scale(p.red()),
                scale(p.green()),
                scale(p.blue()),
                scale(p.alpha()),
            )
            .unwrap_or(*p);
        }
    }
    Some(premultiplied(pm, px, px))
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Icon {
        pub name: Cell<&'static str>,
        pub size: Cell<i32>,
        pub filled: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Icon {
        const NAME: &'static str = "WadIcon";
        type Type = super::Icon;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("wad-icon");
            klass.set_accessible_role(gtk::AccessibleRole::Img);
        }
    }

    impl ObjectImpl for Icon {}

    impl WidgetImpl for Icon {
        fn measure(&self, _: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            let s = self.size.get();
            (s, s, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let w = self.obj();
            let (size, name) = (self.size.get(), self.name.get());
            if size <= 0 || name.is_empty() {
                return;
            }
            let px = (size * w.scale_factor()) as u32;
            if let Some(t) = texture(name, px, &w.color(), self.filled.get()) {
                // Centred in what it was given.
                let (x, y) = ((w.width() - size) as f32 / 2.0, (w.height() - size) as f32 / 2.0);
                snapshot.append_texture(&t, &graphene::Rect::new(x, y, size as f32, size as f32));
            }
        }
    }
}

glib::wrapper! {
    /// A lucide icon in the widget's colour (`color:` in CSS), `size` px.
    pub struct Icon(ObjectSubclass<imp::Icon>) @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Icon {
    pub fn new(name: &'static str, size: i32) -> Self {
        let me: Self = glib::Object::new();
        me.set(name);
        me.imp().size.set(size);
        me.set_halign(gtk::Align::Center);
        me.set_valign(gtk::Align::Center);
        if svg(name).is_none() {
            tracing::warn!("no lucide icon {name:?} in assets/lucide (tools/lucide.mjs)");
        }
        me
    }

    /// Its shapes filled in its colour too (lucide-react's `fill-current`).
    pub fn filled(name: &'static str, size: i32) -> Self {
        let me = Self::new(name, size);
        me.imp().filled.set(true);
        me
    }

    /// Another icon in its place.
    pub fn set(&self, name: &'static str) {
        if self.imp().name.replace(name) != name {
            self.queue_draw();
        }
    }

    pub fn set_size(&self, size: i32) {
        if self.imp().size.replace(size) != size {
            self.queue_resize();
        }
    }
}

mod svg_imp {
    use super::*;

    #[derive(Default)]
    pub struct Svg {
        pub data: Cell<&'static [u8]>,
        pub size: Cell<(i32, i32)>,
        /// Fit what's drawn, not the SVG's box (its margins may be uneven).
        pub trim: Cell<bool>,
        /// The last drawing, and the pixel size it was made at.
        pub drawn: RefCell<Option<((u32, u32), gdk::Texture)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Svg {
        const NAME: &'static str = "WadSvg";
        type Type = super::Svg;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("wad-svg");
            klass.set_accessible_role(gtk::AccessibleRole::Img);
        }
    }

    impl ObjectImpl for Svg {}

    impl WidgetImpl for Svg {
        fn measure(&self, o: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            let (w, h) = self.size.get();
            let s = if o == gtk::Orientation::Horizontal { w } else { h };
            (s, s, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let w = self.obj();
            let (lw, lh) = self.size.get();
            let k = w.scale_factor();
            let px = ((lw * k) as u32, (lh * k) as u32);
            let mut drawn = self.drawn.borrow_mut();
            if drawn.as_ref().is_none_or(|(at, _)| *at != px) {
                *drawn = render_fit(self.data.get(), px, self.trim.get()).map(|t| (px, t));
            }
            // Centred in what it's given (a row taller than it, a column wider).
            if let Some((_, t)) = drawn.as_ref() {
                let (x, y) = ((w.width() - lw) as f32 / 2.0, (w.height() - lh) as f32 / 2.0);
                snapshot.append_texture(t, &graphene::Rect::new(x.max(0.0), y.max(0.0), lw as f32, lh as f32));
            }
        }
    }

    /// The SVG fitted into `w`×`h` pixels, centred (`trim`: what it draws,
    /// rather than its whole box).
    fn render_fit(svg: &[u8], (w, h): (u32, u32), trim: bool) -> Option<gdk::Texture> {
        use resvg::{tiny_skia, usvg};
        let text = std::str::from_utf8(svg).ok()?;
        let tree = usvg::Tree::from_str(text, &options(text)).ok()?;
        let (x0, y0, sw, sh) = match trim {
            true => {
                let b = tree.root().abs_stroke_bounding_box();
                (b.x(), b.y(), b.width().max(1.0), b.height().max(1.0))
            }
            false => (0.0, 0.0, tree.size().width(), tree.size().height()),
        };
        let k = (w as f32 / sw).min(h as f32 / sh);
        let mut pm = tiny_skia::Pixmap::new(w.max(1), h.max(1))?;
        let t = tiny_skia::Transform::from_row(
            k,
            0.0,
            0.0,
            k,
            (w as f32 - sw * k) / 2.0 - x0 * k,
            (h as f32 - sh * k) / 2.0 - y0 * k,
        );
        resvg::render(&tree, t, &mut pm.as_mut());
        Some(premultiplied(pm, w.max(1), h.max(1)))
    }
}

glib::wrapper! {
    /// A picture drawn from an SVG at `width`×`height` (sharp at any scale):
    /// the brand's marks and logos.
    pub struct Svg(ObjectSubclass<svg_imp::Svg>) @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Svg {
    pub fn new(data: &'static [u8], width: i32, height: i32) -> Self {
        let me: Self = glib::Object::new();
        me.imp().data.set(data);
        me.imp().size.set((width, height));
        me
    }

    /// What the SVG draws fitted into `width`×`height` and centred, its own
    /// box's margins left out (a logo whose margins aren't even).
    pub fn trimmed(data: &'static [u8], width: i32, height: i32) -> Self {
        let me = Self::new(data, width, height);
        me.imp().trim.set(true);
        me
    }

    /// Another drawing in its place (the theme's other logo).
    pub fn set(&self, data: &'static [u8]) {
        if !std::ptr::eq(self.imp().data.replace(data), data) {
            self.imp().drawn.borrow_mut().take();
            self.queue_draw();
        }
    }
}
