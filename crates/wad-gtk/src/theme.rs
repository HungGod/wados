//! WadSpaces' colours, dark only (v2 has no light theme): the spike's
//! `src/styles/tokens.css` as GTK CSS custom properties on every window, so
//! a stylesheet says `var(--accent)`. Layer-shell surfaces are windows too.

use gtk::gdk;

/// The tokens, as the spike defines them.
pub const TOKENS: &str = "window, window.background {
  --bg: #0a0614;
  --bg-2: #0e0919;
  --surface: #140d22;
  --surface-2: #1b132d;
  --surface-3: #241a39;
  --line: rgb(240 234 250 / 0.09);
  --line-strong: rgb(240 234 250 / 0.18);
  --fg: #f5f2fa;
  --muted: rgb(245 242 250 / 0.62);
  --faint: rgb(245 242 250 / 0.4);

  --accent: #c6ff1f;
  --accent-fg: #0a0614;
  --accent-soft: rgb(198 255 31 / 0.2);
  --accent-2: #ff3d81;
  --accent-2-soft: rgb(255 61 129 / 0.24);
  --danger: #ff5d7a;
  --danger-soft: rgb(255 93 122 / 0.16);
  --warn: #e8c36a;
  --warn-soft: rgb(232 195 106 / 0.16);
  --unbuilt: #ffb36b;
  --ok: #c6ff1f;

  --c-app: #7fd3ff;
  --c-ext: #c6ff1f;
  --c-key: #ff3d81;
  --c-custom: #ffffff;

  --glow-ring: rgb(198 255 31 / 0.35);
  --glow-haze: rgb(198 255 31 / 0.25);
  --shadow: rgb(3 2 8 / 0.85);
  --dim: rgb(3 2 8 / 0.62);
}
";

/// What every window of ours starts from: the faces, the text colour, no
/// stock window background (each surface paints its own).
pub const BASE: &str = "window, window.background {
  color: var(--fg);
  font-family: Inter, system-ui, sans-serif;
  font-size: 14px;
}
.display { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; letter-spacing: -0.01em; }
.mono { font-family: monospace; }
.muted { color: var(--muted); }
.faint { color: var(--faint); }
";

/// The radii (px), as the spike's `--r-*` (GTK CSS has no var() in
/// border-radius before 4.16's custom properties reach every property; these
/// keep Rust-side drawing in step).
pub mod radius {
    pub const SM: f64 = 8.0;
    pub const R: f64 = 12.0;
    pub const MD: f64 = 16.0;
    pub const LG: f64 = 20.0;
    pub const XL: f64 = 24.0;
}

/// Installs the tokens and the base on `display`, prefers dark for stock
/// widgets, and makes sure the fonts are there. Then add each app's own CSS
/// with [`add_css`].
pub fn install(display: &gdk::Display) {
    crate::fonts::install();
    gtk::Settings::for_display(display).set_gtk_interface_color_scheme(gtk::InterfaceColorScheme::Dark);
    add_css(display, TOKENS);
    add_css(display, BASE);
    add_css(display, crate::tile::CSS);
}

/// A stylesheet of an app's own, above the theme's.
pub fn add_css(display: &gdk::Display, css: &str) {
    let p = gtk::CssProvider::new();
    p.load_from_string(css);
    gtk::style_context_add_provider_for_display(display, &p, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
}
