//! The launcher's layout, in points, from the screen's size (no GTK): the
//! bar along the top, then the carousel of square cards (the one in focus
//! in the middle), its buttons under it. The spike's numbers (Home.svelte's
//! PAD, barTop, S, rowTop and `place`); at 1500 × 1000 the card in focus is
//! 380 square.

/// A card beside the one in focus is this much of its size.
pub const SIDE: f64 = 0.62;
/// The dock's room at the bottom, as the spike leaves it.
const DOCK: f64 = 82.0;
/// The bar's height.
pub const BAR_H: f64 = 48.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// The screen.
    pub w: f64,
    pub h: f64,
    /// The bar's inset from the sides (it starts 14 in from it, its tabs' padding).
    pub pad: f64,
    pub bar_top: f64,
    /// The card in focus's side.
    pub side: f64,
    /// The cards' top (the one in focus's).
    pub row_top: f64,
}

impl Layout {
    pub fn new(w: f64, h: f64) -> Self {
        let pad = (w * 0.055).clamp(48.0, 110.0).round();
        let bar_top = (h * 0.04).round();
        let side = ((h - DOCK) * 0.5).min(w * 0.3).clamp(240.0, 380.0).round();
        let row_top = (bar_top + BAR_H + 24.0).max(((h - DOCK) / 2.0 + 6.0 - side / 2.0).round());
        Self { w, h, pad, bar_top, side, row_top }
    }

    /// The buttons' top: 20 under the card in focus.
    pub fn acts_top(&self) -> f64 {
        self.row_top + self.side + 20.0
    }

    /// The tabs' text size (the spike's clamp(20px, 3.2vh, 32px)).
    pub fn tab_font(&self) -> f64 {
        (self.h * 0.032).clamp(20.0, 32.0)
    }

    /// Where a card `d` places from the one in focus is (`d` fractional
    /// while the row moves).
    pub fn place(&self, d: f64) -> Placed {
        place(d, self.side)
    }
}

/// A card's place: its middle's offset from the screen's, its scale and opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub x: f64,
    pub scale: f64,
    pub opacity: f64,
}

/// Where a card `d` places from the one in focus is, for side `s`: the one
/// in focus at the middle, full size; the others 62% of it, 40 from it and
/// 24 apart, fading out past the third.
pub fn place(d: f64, s: f64) -> Placed {
    let a = d.abs();
    let first = s / 2.0 + 40.0 + s * SIDE / 2.0;
    let x = if a <= 1.0 { a * first } else { first + (a - 1.0) * (s * SIDE + 24.0) };
    let scale = 1.0 - (1.0 - SIDE) * a.min(1.0);
    let opacity = if a <= 1.0 {
        1.0 - 0.32 * a
    } else if a <= 3.0 {
        0.8 - 0.12 * a
    } else {
        (0.44 * (4.0 - a)).max(0.0)
    };
    Placed { x: x * d.signum(), scale, opacity }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn at_the_surface_book_s_size_its_the_spikes_numbers() {
        let l = Layout::new(1500.0, 1000.0);
        assert_eq!((l.pad, l.bar_top, l.side, l.row_top), (83.0, 40.0, 380.0, 275.0));
        assert_eq!(l.acts_top(), 675.0);
        assert_eq!(l.tab_font(), 32.0);
        // The spike's screenshots (1280 × 800).
        let l = Layout::new(1280.0, 800.0);
        assert_eq!((l.pad, l.bar_top, l.side, l.row_top), (70.0, 32.0, 359.0, 186.0));
        assert!(near(l.tab_font(), 25.6));
        // A small screen keeps the bar clear.
        let l = Layout::new(1024.0, 600.0);
        assert_eq!(l.side, 259.0);
        assert!(l.row_top >= l.bar_top + BAR_H + 24.0);
    }

    #[test]
    fn cards_sit_as_the_spikes_do() {
        let s = 380.0;
        assert_eq!(place(0.0, s), Placed { x: 0.0, scale: 1.0, opacity: 1.0 });
        let one = place(1.0, s);
        assert!(near(one.x, 190.0 + 40.0 + 117.8) && near(one.scale, SIDE) && near(one.opacity, 0.68));
        let left = place(-2.0, s);
        assert!(near(left.x, -(347.8 + 235.6 + 24.0)) && near(left.opacity, 0.56));
        assert!(near(place(3.0, s).opacity, 0.44));
        assert_eq!(place(4.0, s).opacity, 0.0);
        assert_eq!(place(7.0, s).opacity, 0.0);
        // Moving, it's in between.
        let half = place(0.5, s);
        assert!(near(half.x, 347.8 / 2.0) && near(half.scale, 0.81) && near(half.opacity, 0.84));
    }
}
