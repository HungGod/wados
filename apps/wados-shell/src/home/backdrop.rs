//! Behind everything on the launcher, the card in focus's picture (the
//! spike's `.art` and `.shade`): its wallpaper covering the screen and 30
//! past each edge, blurred (10 px) and a little more saturated (1.15), then
//! shaded: darker toward the edges (a radial gradient) and down the screen
//! (a linear one). A new card's picture fades in over the last (0.5 s).
//!
//! It's all soft, so it's made once per picture on the CPU, small: the
//! picture drawn 320 px wide and blurred there, then composed with the
//! shades at half the screen's points (750 × 500 on the Surface Book), and
//! drawn scaled up as one texture. A fade blends two of those. A frame of it
//! is one texture on any renderer (cairo's gradients and opacity groups at
//! 3000 × 2000 took half a second a frame).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::{gdk, glib, graphene, gsk, prelude::*, subclass::prelude::*};
use wad_model::Wallpaper;

use super::tween::{self, Tween};

/// How wide the blurred drawing is.
const SMALL: u32 = 320;
/// The box blur's radius there, in three passes: about a 2.4 px gaussian (12 points on the screen).
const BLUR: usize = 2;
const SATURATE: f32 = 1.15;
/// How far the picture reaches past the screen's edges (the spike's inset: -30px).
const BLEED: f64 = 30.0;
/// The launcher's own dark under it all (#07040e).
const BASE: [f32; 3] = [7.0, 4.0, 14.0];
/// The shades' ink (rgb(10 6 20)).
const INK: [f32; 3] = [10.0, 6.0, 20.0];
/// The spike's fade in (0.5 s, ease).
const FADE: std::time::Duration = std::time::Duration::from_millis(500);
/// The composed backdrop is this many of its pixels per screen point.
const DENSITY: f64 = 0.5;

/// RGBA pixels, premultiplied.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

/// What a wallpaper is, behind the launcher: a picture (blurred), a colour, or nothing.
#[derive(Debug, Clone)]
pub enum Art {
    Picture(Rc<Image>),
    Color([f32; 3]),
    None,
}

thread_local! {
    static BLURRED: RefCell<HashMap<String, Option<Rc<Image>>>> = RefCell::default();
}

fn art_of(w: &Wallpaper) -> Art {
    match w {
        Wallpaper::Empty => Art::None,
        Wallpaper::Color { value } => gdk::RGBA::parse(value.as_str())
            .map(|c| Art::Color([c.red() * 255.0, c.green() * 255.0, c.blue() * 255.0]))
            .unwrap_or(Art::None),
        Wallpaper::Image { value } => {
            if let Some(t) = BLURRED.with(|b| b.borrow().get(value).cloned()) {
                return t.map(Art::Picture).unwrap_or(Art::None);
            }
            let t = wad_gtk::picture::resolve(value).and_then(|p| blurred(&p)).map(Rc::new);
            BLURRED.with(|b| {
                let mut b = b.borrow_mut();
                if b.len() > 48 {
                    b.clear();
                }
                b.insert(value.clone(), t.clone());
            });
            t.map(Art::Picture).unwrap_or(Art::None)
        }
    }
}

/// The picture at `p`, small, blurred and saturated.
fn blurred(p: &std::path::Path) -> Option<Image> {
    let t = wad_gtk::picture::texture(p, SMALL)?;
    let mut d = gdk::TextureDownloader::new(&t);
    d.set_format(gdk::MemoryFormat::R8g8b8a8Premultiplied);
    let (bytes, stride) = d.download_bytes();
    let (w, h) = (t.width() as usize, t.height() as usize);
    let mut px: Vec<u8> = Vec::with_capacity(w * h * 4);
    for row in bytes.chunks(stride).take(h) {
        px.extend_from_slice(&row[..w * 4]);
    }
    let (mut px, w, h) = shrink(px, w, h, SMALL as usize);
    for _ in 0..3 {
        box_blur(&mut px, w, h, BLUR);
    }
    saturate(&mut px, SATURATE);
    Some(Image { w, h, px })
}

/// `px` (RGBA, `w`×`h`) shrunk to about `to` wide (by whole steps, averaging), if it's much wider.
pub fn shrink(px: Vec<u8>, w: usize, h: usize, to: usize) -> (Vec<u8>, usize, usize) {
    let k = w / to.max(1);
    if k < 2 {
        return (px, w, h);
    }
    let (nw, nh) = (w / k, (h / k).max(1));
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        for x in 0..nw {
            let mut sum = [0u32; 4];
            for dy in 0..k {
                let row = ((y * k + dy).min(h - 1)) * w;
                for dx in 0..k {
                    let i = (row + x * k + dx) * 4;
                    for c in 0..4 {
                        sum[c] += px[i + c] as u32;
                    }
                }
            }
            let n = (k * k) as u32;
            let o = (y * nw + x) * 4;
            for c in 0..4 {
                out[o + c] = (sum[c] / n) as u8;
            }
        }
    }
    (out, nw, nh)
}

/// One box blur of radius `r` across and down (the edges held), in place.
pub fn box_blur(px: &mut [u8], w: usize, h: usize, r: usize) {
    if w == 0 || h == 0 || r == 0 {
        return;
    }
    let mut line: Vec<[u32; 4]> = Vec::new();
    let mut pass = |px: &mut [u8], len: usize, lines: usize, at: &dyn Fn(usize, usize) -> usize| {
        let n = (2 * r + 1) as u32;
        for l in 0..lines {
            line.clear();
            line.extend((0..len).map(|i| {
                let j = at(l, i);
                [px[j] as u32, px[j + 1] as u32, px[j + 2] as u32, px[j + 3] as u32]
            }));
            let get = |i: isize| line[i.clamp(0, len as isize - 1) as usize];
            let mut sum = [0u32; 4];
            for i in -(r as isize)..=(r as isize) {
                let v = get(i);
                for c in 0..4 {
                    sum[c] += v[c];
                }
            }
            for i in 0..len {
                let j = at(l, i);
                for c in 0..4 {
                    px[j + c] = ((sum[c] + n / 2) / n) as u8;
                }
                let (add, sub) = (get(i as isize + r as isize + 1), get(i as isize - r as isize));
                for c in 0..4 {
                    sum[c] = sum[c] + add[c] - sub[c];
                }
            }
        }
    };
    pass(px, w, h, &|l, i| (l * w + i) * 4);
    pass(px, h, w, &|l, i| (i * w + l) * 4);
}

/// CSS's saturate(`s`) on premultiplied RGBA, in place.
pub fn saturate(px: &mut [u8], s: f32) {
    let m = [
        [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s],
        [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s],
        [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s],
    ];
    for p in px.as_chunks_mut::<4>().0 {
        let (r, g, b, a) = (p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32);
        for (c, row) in m.iter().enumerate() {
            p[c] = (row[0] * r + row[1] * g + row[2] * b).round().clamp(0.0, a) as u8;
        }
    }
}

/// `img` sampled (bilinear, edges held) at pixel position (`u`, `v`): premultiplied RGBA, 0–255.
fn sample(img: &Image, u: f64, v: f64) -> [f32; 4] {
    let (u, v) = (u.clamp(0.0, (img.w - 1) as f64), v.clamp(0.0, (img.h - 1) as f64));
    let (x0, y0) = (u.floor() as usize, v.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(img.w - 1), (y0 + 1).min(img.h - 1));
    let (fx, fy) = ((u - x0 as f64) as f32, (v - y0 as f64) as f32);
    let at = |x: usize, y: usize, c: usize| img.px[(y * img.w + x) * 4 + c] as f32;
    let mut out = [0.0; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let top = at(x0, y0, c) * (1.0 - fx) + at(x1, y0, c) * fx;
        let bottom = at(x0, y1, c) * (1.0 - fx) + at(x1, y1, c) * fx;
        *o = top * (1.0 - fy) + bottom * fy;
    }
    out
}

/// The linear shade's darkness `f` (0–1) down the screen: the spike's stops.
fn shade_down(f: f64) -> f32 {
    const STOPS: [(f64, f64); 5] = [(0.0, 0.55), (0.3, 0.45), (0.6, 0.55), (0.9, 0.9), (1.0, 0.9)];
    for w in STOPS.windows(2) {
        let ((a, va), (b, vb)) = (w[0], w[1]);
        if f <= b {
            let t = ((f - a) / (b - a)).clamp(0.0, 1.0);
            return (va + (vb - va) * t) as f32;
        }
    }
    0.9
}

/// The backdrop for a `width`×`height` point screen, `sw`×`sh` pixels: `art`
/// covering it and BLEED past, on the dark, under the shades (the radial
/// one, then the linear one over it, as CSS stacks the spike's two).
pub fn compose(art: &Art, width: f64, height: f64, sw: usize, sh: usize) -> Image {
    let mut px = vec![0u8; sw * sh * 4];
    // The picture's place: covering (-BLEED, -BLEED, w + 2 BLEED, h + 2 BLEED).
    let place = match art {
        Art::Picture(img) => {
            let (dw, dh) = (width + 2.0 * BLEED, height + 2.0 * BLEED);
            let a = img.w as f64 / img.h.max(1) as f64;
            let (pw, ph) = if dw / a >= dh { (dw, dw / a) } else { (dh * a, dh) };
            Some((img, -BLEED + (dw - pw) / 2.0, -BLEED + (dh - ph) / 2.0, pw, ph))
        }
        _ => None,
    };
    for j in 0..sh {
        let y = (j as f64 + 0.5) * height / sh as f64;
        let down = shade_down(y / height);
        for i in 0..sw {
            let x = (i as f64 + 0.5) * width / sw as f64;
            let mut c = BASE;
            match (art, place) {
                (_, Some((img, dx, dy, pw, ph))) => {
                    let s = sample(img, (x - dx) / pw * img.w as f64 - 0.5, (y - dy) / ph * img.h as f64 - 0.5);
                    let a = s[3] / 255.0;
                    for k in 0..3 {
                        c[k] = s[k] + c[k] * (1.0 - a);
                    }
                }
                (Art::Color(rgb), None) => c = *rgb,
                _ => {}
            }
            // radial-gradient(70% 60% at 50% 45%, transparent, ink / 0.5)
            let (rx, ry) = ((x - 0.5 * width) / (0.7 * width), (y - 0.45 * height) / (0.6 * height));
            let edge = 0.5 * (rx * rx + ry * ry).sqrt().min(1.0) as f32;
            for k in 0..3 {
                c[k] = c[k] * (1.0 - edge) + INK[k] * edge;
                c[k] = c[k] * (1.0 - down) + INK[k] * down;
            }
            let o = (j * sw + i) * 4;
            for k in 0..3 {
                px[o + k] = c[k].round().clamp(0.0, 255.0) as u8;
            }
            px[o + 3] = 255;
        }
    }
    Image { w: sw, h: sh, px }
}

/// `a` faded into `b` by `t` (0 all `a`, 1 all `b`); the same size.
pub fn blend(a: &Image, b: &Image, t: f64) -> Image {
    let t = (t.clamp(0.0, 1.0) * 256.0) as u32;
    let px = a.px.iter().zip(&b.px).map(|(&x, &y)| ((x as u32 * (256 - t) + y as u32 * t) >> 8) as u8).collect();
    Image { w: a.w, h: a.h, px }
}

fn texture_of(img: &Image) -> gdk::Texture {
    let bytes = glib::Bytes::from(&img.px[..]);
    gdk::MemoryTexture::new(img.w as i32, img.h as i32, gdk::MemoryFormat::R8g8b8a8Premultiplied, &bytes, img.w * 4)
        .upcast()
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Backdrop {
        /// The picture shown, and the screen it's made for (points).
        pub(super) key: RefCell<Option<Wallpaper>>,
        pub(super) screen: Cell<(f64, f64)>,
        /// Composed: what it fades from, and to.
        pub(super) was: RefCell<Option<Rc<Image>>>,
        pub(super) now: RefCell<Option<Rc<Image>>>,
        pub(super) drawn: RefCell<Option<gdk::Texture>>,
        pub(super) fade: Cell<f64>,
        pub(super) fade_in: RefCell<Option<Tween>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Backdrop {
        const NAME: &'static str = "WadosHomeBackdrop";
        type Type = super::Backdrop;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Backdrop {}

    impl WidgetImpl for Backdrop {
        fn snapshot(&self, s: &gtk::Snapshot) {
            let w = self.obj();
            let (width, height) = (w.width() as f32, w.height() as f32);
            if width <= 0.0 || height <= 0.0 {
                return;
            }
            let bounds = graphene::Rect::new(0.0, 0.0, width, height);
            match &*self.drawn.borrow() {
                Some(t) => s.append_scaled_texture(t, gsk::ScalingFilter::Linear, &bounds),
                None => {
                    s.append_color(&gdk::RGBA::new(BASE[0] / 255.0, BASE[1] / 255.0, BASE[2] / 255.0, 1.0), &bounds)
                }
            }
        }
    }
}

glib::wrapper! {
    pub struct Backdrop(ObjectSubclass<imp::Backdrop>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Backdrop {
    pub fn new() -> Self {
        let me: Self = glib::Object::new();
        me.set_can_target(false);
        me.imp().fade.set(1.0);
        let weak = me.downgrade();
        let fade_in = Tween::new(&me, FADE, tween::EASE, 1.0, move |v| {
            if let Some(b) = weak.upgrade() {
                b.faded(v);
            }
        });
        *me.imp().fade_in.borrow_mut() = Some(fade_in);
        me
    }

    /// The fade at `v`: what it draws.
    fn faded(&self, v: f64) {
        let imp = self.imp();
        imp.fade.set(v.clamp(0.0, 1.0));
        let drawn = match (&*imp.was.borrow(), &*imp.now.borrow()) {
            (Some(a), Some(b)) if v < 1.0 => Some(texture_of(&blend(a, b, v))),
            (_, Some(b)) => Some(texture_of(b)),
            _ => None,
        };
        if v >= 1.0 {
            imp.was.borrow_mut().take();
        }
        *imp.drawn.borrow_mut() = drawn;
        self.queue_draw();
    }

    /// Shows `w` (the card in focus's picture) on a `width`×`height` point
    /// screen, fading it in over what was.
    pub fn set(&self, w: &Wallpaper, width: f64, height: f64) {
        let imp = self.imp();
        let resized = imp.screen.get() != (width, height);
        if !resized && imp.key.borrow().as_ref() == Some(w) {
            return;
        }
        let first = imp.key.borrow().is_none() || resized;
        *imp.key.borrow_mut() = Some(w.clone());
        imp.screen.set((width, height));
        let (sw, sh) = (((width * DENSITY).round() as usize).max(1), ((height * DENSITY).round() as usize).max(1));
        let made = Rc::new(compose(&art_of(w), width, height, sw, sh));
        // What shows now (mid-fade too) is what it fades from.
        let showing = match (&*imp.was.borrow(), &*imp.now.borrow()) {
            (Some(a), Some(b)) if imp.fade.get() < 1.0 => Some(Rc::new(blend(a, b, imp.fade.get()))),
            (_, b) => b.clone(),
        };
        *imp.was.borrow_mut() = if first { None } else { showing };
        *imp.now.borrow_mut() = Some(made);
        let Some(fade) = imp.fade_in.borrow().clone() else { return };
        if first {
            fade.jump(1.0);
        } else {
            fade.jump(0.0);
            fade.to(1.0);
        }
    }

    /// Still fading one in.
    pub fn fading(&self) -> bool {
        self.imp().fade_in.borrow().as_ref().is_some_and(|t| t.moving())
    }

    /// The picture it shows (for wadsctl).
    pub fn wallpaper(&self) -> Option<Wallpaper> {
        self.imp().key.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blur_keeps_a_flat_picture_flat_and_spreads_a_dot() {
        let (w, h) = (9, 7);
        let mut flat = [40u8, 80, 120, 255].repeat(w * h);
        box_blur(&mut flat, w, h, 2);
        assert!(flat.chunks(4).all(|p| p == [40, 80, 120, 255]));

        let mut dot = vec![0u8; w * h * 4];
        let mid = (3 * w + 4) * 4;
        dot[mid..mid + 4].copy_from_slice(&[250, 250, 250, 250]);
        box_blur(&mut dot, w, h, 1);
        // A 3 × 3 box: a ninth of it in each, nothing farther.
        assert_eq!(dot[mid], 28);
        assert_eq!(dot[(2 * w + 3) * 4], 28);
        assert_eq!(dot[(w + 4) * 4], 0);
    }

    #[test]
    fn saturating_grey_leaves_it_grey_and_a_colour_goes_further() {
        let mut px = vec![100, 100, 100, 255, 200, 100, 50, 255, 20, 10, 5, 40];
        saturate(&mut px, 1.15);
        assert_eq!(&px[..4], &[100, 100, 100, 255]);
        assert!(px[4] > 200 && px[6] < 50);
        // Premultiplied: never past its alpha.
        assert!(px[8..11].iter().all(|&c| c <= 40));
    }

    #[test]
    fn a_big_picture_is_shrunk_by_averaging() {
        let px = [10u8, 20, 30, 255, 30, 40, 50, 255].repeat(8 * 4 / 2);
        let (out, w, h) = shrink(px, 8, 4, 4);
        assert_eq!((w, h), (4, 2));
        assert_eq!(&out[..4], &[20, 30, 40, 255]);
        let (same, w, _) = shrink(vec![0; 16], 4, 1, 3);
        assert_eq!((same.len(), w), (16, 4));
    }

    #[test]
    fn the_shades_darken_the_bottom_and_the_edges_most() {
        let white = Art::Picture(Rc::new(Image { w: 4, h: 4, px: [255u8; 4].repeat(16) }));
        let img = compose(&white, 1500.0, 1000.0, 150, 100);
        let at = |x: usize, y: usize| img.px[(y * 150 + x) * 4] as i32;
        // Opaque everywhere.
        assert!(img.px.chunks(4).all(|p| p[3] == 255));
        // The middle (a little above it) is the lightest; the bottom's the darkest.
        let (middle, bottom, corner) = (at(75, 40), at(75, 99), at(0, 0));
        assert!(middle > corner && middle > bottom, "{middle} {corner} {bottom}");
        assert!(bottom < 40, "{bottom}");
        // Its middle: 55% → 45% → 55% ink down to there, a little edge shade.
        assert!((110..140).contains(&middle), "{middle}");
        // No picture: the launcher's dark, shaded.
        let none = compose(&Art::None, 1500.0, 1000.0, 15, 10);
        assert!(none.px.chunks(4).all(|p| p[0] <= 10 && p[2] <= 20));
    }

    #[test]
    fn a_fade_goes_from_one_to_the_other() {
        let a = Image { w: 1, h: 1, px: vec![0, 0, 0, 255] };
        let b = Image { w: 1, h: 1, px: vec![200, 100, 50, 255] };
        assert_eq!(blend(&a, &b, 0.0).px, a.px);
        assert_eq!(blend(&a, &b, 1.0).px, vec![200, 100, 50, 255]);
        assert_eq!(blend(&a, &b, 0.5).px, vec![100, 50, 25, 255]);
    }
}
