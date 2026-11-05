//! The share sheet's QR code (`ui/QR.svelte`): the one-tap link's modules
//! (the `qrcode` crate, error correction M, a quiet zone of one module as
//! the spike's), drawn into a texture at the display's own pixels, so each
//! module is a whole number of them and it stays sharp to scan. Dark
//! #0a0614 on light #f5f2fa, as the spike's.

use gtk::{gdk, glib, prelude::*};
use qrcode::{Color, EcLevel, QrCode};

/// The quiet zone around it, in modules (the spike's `margin: 1`).
pub const QUIET: usize = 1;
const DARK: [u8; 3] = [0x0a, 0x06, 0x14];
const LIGHT: [u8; 3] = [0xf5, 0xf2, 0xfa];

/// Its modules, a square `width` on a side (the quiet zone in), row by row: true is dark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matrix {
    pub width: usize,
    pub dark: Vec<bool>,
}

impl Matrix {
    pub fn at(&self, x: usize, y: usize) -> bool {
        self.dark[y * self.width + x]
    }
}

/// `text`'s QR code (none: too long for one).
pub fn matrix(text: &str) -> Option<Matrix> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).ok()?;
    let inner = code.width();
    let width = inner + 2 * QUIET;
    let colors = code.to_colors();
    let mut dark = vec![false; width * width];
    for y in 0..inner {
        for x in 0..inner {
            dark[(y + QUIET) * width + x + QUIET] = colors[y * inner + x] == Color::Dark;
        }
    }
    Some(Matrix { width, dark })
}

/// How many pixels a module takes to fit `side` pixels (at least one).
pub fn module_px(width: usize, side: usize) -> usize {
    (side / width.max(1)).max(1)
}

/// The modules as RGB rows, `px` pixels a module.
pub fn rgb(m: &Matrix, px: usize) -> Vec<u8> {
    let side = m.width * px;
    let mut out = Vec::with_capacity(side * side * 3);
    for y in 0..side {
        for x in 0..side {
            out.extend_from_slice(if m.at(x / px, y / px) { &DARK } else { &LIGHT });
        }
    }
    out
}

/// A picture of `text`'s QR code, `side` points square on a display of
/// `scale` (its pixels a whole number a module, so a little smaller when
/// they don't divide). None: too long for a QR code.
pub fn picture(text: &str, side: i32, scale: i32) -> Option<gtk::Image> {
    let m = matrix(text)?;
    let scale = scale.max(1);
    let px = module_px(m.width, (side * scale) as usize);
    let pixels = m.width * px;
    let texture = gdk::MemoryTexture::new(
        pixels as i32,
        pixels as i32,
        gdk::MemoryFormat::R8g8b8,
        &glib::Bytes::from_owned(rgb(&m, px)),
        pixels * 3,
    );
    // Drawn at its own pixels: as many points as they make on this display.
    let points = (pixels as i32 + scale - 1) / scale;
    let p = gtk::Image::builder()
        .paintable(&texture)
        .pixel_size(points)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    p.add_css_class("st-qr-code");
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A finder pattern (7 × 7) with its top-left module at (x, y): its ring and its 3 × 3 middle dark.
    fn finder_at(m: &Matrix, x: usize, y: usize) -> bool {
        (0..7).all(|i| {
            (0..7).all(|j| {
                let ring = i == 0 || i == 6 || j == 0 || j == 6;
                let middle = (2..=4).contains(&i) && (2..=4).contains(&j);
                m.at(x + i, y + j) == (ring || middle)
            })
        })
    }

    #[test]
    fn a_share_links_code_has_its_three_finders_in_a_quiet_zone() {
        let link = "https://sample-host.invalid/ws-sample-wadspaces/#token=sample-token-not-a-secret";
        let m = matrix(link).unwrap();
        // Version 5 at M (37 modules) holds 84 bytes; its quiet zone makes it 39.
        let inner = m.width - 2 * QUIET;
        assert_eq!((inner - 17) % 4, 0, "a QR code's width is 17 + 4 × its version");
        assert!(inner >= 33, "{inner}");
        for (x, y) in [(QUIET, QUIET), (m.width - QUIET - 7, QUIET), (QUIET, m.width - QUIET - 7)] {
            assert!(finder_at(&m, x, y), "a finder at {x},{y}");
        }
        assert!(!finder_at(&m, m.width - QUIET - 7, m.width - QUIET - 7), "none at the fourth corner");
        // The quiet zone is light all round.
        for i in 0..m.width {
            assert!(!m.at(i, 0) && !m.at(0, i) && !m.at(i, m.width - 1) && !m.at(m.width - 1, i));
        }
        // The same link, the same code; another, another.
        assert_eq!(matrix(link), Some(m.clone()));
        assert_ne!(matrix("https://sample-host.invalid/ws-sample-other/#token=x"), Some(m));
    }

    #[test]
    fn its_pixels_are_whole_modules() {
        let m = matrix("https://x.invalid/ws-1/#token=k").unwrap();
        // 176 points at 200% over a 25 + 2 module code: 13 px a module (351 px).
        assert_eq!(module_px(27, 352), 13);
        assert_eq!(module_px(500, 100), 1);
        let px = module_px(m.width, 352);
        let bytes = rgb(&m, px);
        assert_eq!(bytes.len(), (m.width * px).pow(2) * 3);
        // Its corner's light (the quiet zone), its first finder's corner dark.
        assert_eq!(&bytes[..3], &LIGHT);
        let row = m.width * px * 3;
        let at = |x: usize, y: usize| &bytes[y * row + x * 3..y * row + x * 3 + 3];
        assert_eq!(at(QUIET * px, QUIET * px), &DARK);
        assert_eq!(at(QUIET * px + px - 1, QUIET * px + px - 1), &DARK);
    }

    #[test]
    fn too_long_for_a_code_is_none() {
        assert!(matrix(&"x".repeat(4000)).is_none());
    }
}
