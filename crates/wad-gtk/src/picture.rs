//! A wadspace's picture, drawn: a wallpaper filling its widget (cover, as the
//! spike's `Wallpaper.svelte`), or cut out as a wadspace's icon says
//! (`IconPicture.svelte`, wad_model::look): the picture as it fills a 16:10
//! screen, zoomed and moved so the square the icon asks for fills the widget.
//!
//! Pictures are WadSpaces' own (`/wallpapers/art/<name>.svg`, from its
//! assets) or files on the machine (`~/Pictures/…`, a path). SVGs are drawn
//! at the size they're shown, on the screen's scale, so they stay sharp;
//! drawings are cached.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gtk::{gdk, glib, graphene, gsk, prelude::*, subclass::prelude::*};
use wad_model::{IconLook, Wallpaper};

/// Where a wallpaper's picture is on this machine.
pub fn resolve(value: &str) -> Option<PathBuf> {
    if value.split('/').any(|p| p == "..") {
        return None;
    }
    if value.starts_with("/wallpapers/")
        || value.starts_with("/apps/")
        || value.starts_with("/icons/")
        || value.starts_with("/brand/")
    {
        return crate::assets::path(value.trim_start_matches('/'));
    }
    let p = match value.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var_os("HOME")?).join(rest),
        None => PathBuf::from(value),
    };
    (p.is_absolute() && p.is_file()).then_some(p)
}

/// A drawing of a picture: (file, pixel width bucket).
type Key = (PathBuf, u32);

thread_local! {
    static DRAWN: RefCell<HashMap<Key, Option<gdk::Texture>>> = RefCell::default();
    static SVG_SIZE: RefCell<HashMap<PathBuf, Option<(f32, f32)>>> = RefCell::default();
}

const CACHE_MAX: usize = 96;

fn is_svg(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("svg"))
}

/// An SVG's own size (its viewBox).
fn svg_size(p: &Path) -> Option<(f32, f32)> {
    if let Some(s) = SVG_SIZE.with(|c| c.borrow().get(p).copied()) {
        return s;
    }
    let size = std::fs::read_to_string(p).ok().and_then(|text| {
        let tree = resvg::usvg::Tree::from_str(&text, &resvg::usvg::Options::default()).ok()?;
        Some((tree.size().width(), tree.size().height()))
    });
    SVG_SIZE.with(|c| c.borrow_mut().insert(p.to_path_buf(), size));
    size
}

/// The picture at `p`, drawn about `px_wide` pixels wide (an SVG), or as it is (anything else).
pub fn texture(p: &Path, px_wide: u32) -> Option<gdk::Texture> {
    // SVGs in 64 px steps (a resize doesn't redraw every frame); others once.
    let bucket = if is_svg(p) { px_wide.clamp(64, 8192).div_ceil(64) * 64 } else { 0 };
    let key = (p.to_path_buf(), bucket);
    if let Some(t) = DRAWN.with(|d| d.borrow().get(&key).cloned()) {
        return t;
    }
    let made = if is_svg(p) { draw_svg(p, bucket) } else { gdk::Texture::from_filename(p).ok() };
    DRAWN.with(|d| {
        let mut d = d.borrow_mut();
        if d.len() >= CACHE_MAX {
            d.clear();
        }
        d.insert(key, made.clone());
    });
    made
}

fn draw_svg(p: &Path, px_wide: u32) -> Option<gdk::Texture> {
    use resvg::{tiny_skia, usvg};
    let text = std::fs::read_to_string(p).ok()?;
    let tree = usvg::Tree::from_str(&text, &usvg::Options::default()).ok()?;
    let (w, h) = (tree.size().width(), tree.size().height());
    let k = px_wide as f32 / w;
    let (pw, ph) = (px_wide, ((h * k).ceil() as u32).max(1));
    let mut pm = tiny_skia::Pixmap::new(pw, ph)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(k, k), &mut pm.as_mut());
    let bytes = glib::Bytes::from_owned(pm.take());
    Some(
        gdk::MemoryTexture::new(
            pw as i32,
            ph as i32,
            gdk::MemoryFormat::R8g8b8a8Premultiplied,
            &bytes,
            pw as usize * 4,
        )
        .upcast(),
    )
}

/// A picture's own aspect (width over height), without drawing it.
fn aspect(p: &Path) -> Option<f32> {
    if is_svg(p) {
        let (w, h) = svg_size(p)?;
        return (h > 0.0).then(|| w / h);
    }
    let t = texture(p, 0)?;
    (t.height() > 0).then(|| t.width() as f32 / t.height() as f32)
}

/// Draws `p` covering `dest` (cut to it), sharp at `scale`.
fn draw_cover(s: &gtk::Snapshot, p: &Path, dest: &graphene::Rect, scale: f32) {
    let Some(a) = aspect(p) else { return };
    // The picture's size covering dest.
    let (mut w, mut h) = (dest.width(), dest.width() / a);
    if h < dest.height() {
        h = dest.height();
        w = h * a;
    }
    let Some(t) = texture(p, (w * scale).ceil() as u32) else { return };
    let at = graphene::Rect::new(dest.x() + (dest.width() - w) / 2.0, dest.y() + (dest.height() - h) / 2.0, w, h);
    s.push_clip(dest);
    s.append_texture(&t, &at);
    s.pop();
}

/// A colour as CSS gives it ("#123456", "rgb(…)").
fn color(value: &str) -> Option<gdk::RGBA> {
    gdk::RGBA::parse(value).ok()
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Picture {
        pub wallpaper: RefCell<Wallpaper>,
        pub crop: RefCell<Option<IconLook>>,
        pub radius: Cell<f32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Picture {
        const NAME: &'static str = "WadPicture";
        type Type = super::Picture;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Picture {}

    impl WidgetImpl for Picture {
        fn snapshot(&self, s: &gtk::Snapshot) {
            let w = self.obj();
            let (width, height) = (w.width() as f32, w.height() as f32);
            if width <= 0.0 || height <= 0.0 {
                return;
            }
            let bounds = graphene::Rect::new(0.0, 0.0, width, height);
            let r = self.radius.get();
            let rounded = r > 0.0;
            if rounded {
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(bounds, r));
            }
            match &*self.wallpaper.borrow() {
                Wallpaper::Empty => {}
                Wallpaper::Color { value } => {
                    if let Some(c) = color(value) {
                        s.append_color(&c, &bounds);
                    }
                }
                Wallpaper::Image { value } => {
                    if let Some(p) = resolve(value) {
                        let scale = w.scale_factor() as f32;
                        let dest = match &*self.crop.borrow() {
                            // The picture as it fills a 16:10 screen, zoomed,
                            // moved so (x, y) is in the middle of this square.
                            Some(icon) => {
                                let (cw, ch) = (
                                    wad_model::look::ICON_ASPECT as f32 * icon.zoom as f32 * width,
                                    icon.zoom as f32 * height,
                                );
                                graphene::Rect::new(
                                    width / 2.0 - icon.x as f32 * cw,
                                    height / 2.0 - icon.y as f32 * ch,
                                    cw,
                                    ch,
                                )
                            }
                            None => bounds,
                        };
                        s.push_clip(&bounds);
                        draw_cover(s, &p, &dest, scale);
                        s.pop();
                    }
                }
            }
            if rounded {
                s.pop();
            }
        }
    }
}

glib::wrapper! {
    /// A wallpaper filling the widget, or a wadspace's icon cut out of one.
    pub struct Picture(ObjectSubclass<imp::Picture>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Picture {
    /// `wallpaper` covering the widget, its corners `radius` round.
    pub fn new(wallpaper: &Wallpaper, radius: f32) -> Self {
        let me: Self = glib::Object::new();
        me.imp().radius.set(radius);
        me.set_wallpaper(wallpaper);
        me
    }

    /// A wadspace's icon: `wallpaper` cut out as `icon` says.
    pub fn icon(wallpaper: &Wallpaper, icon: &IconLook, radius: f32) -> Self {
        let me = Self::new(wallpaper, radius);
        me.set_crop(Some(icon));
        me
    }

    pub fn set_wallpaper(&self, wallpaper: &Wallpaper) {
        if *self.imp().wallpaper.borrow() != *wallpaper {
            *self.imp().wallpaper.borrow_mut() = wallpaper.clone();
            self.queue_draw();
        }
    }

    pub fn set_crop(&self, icon: Option<&IconLook>) {
        *self.imp().crop.borrow_mut() = icon.cloned();
        self.queue_draw();
    }

    pub fn set_radius(&self, r: f32) {
        self.imp().radius.set(r);
        self.queue_draw();
    }
}
