//! WadBrowser's windows: libadwaita tabs, the header bar and omnibox, the
//! bookmarks bar, find in page, the settings dialog and shortcuts.
//!
//! Forked from Cactus (github.com/yohannes-mesay/cactus-browser, commit
//! 8ae5a66, MPL-2.0).

mod app;
// Cactus's omnibox suggestion popover: Cactus never wired it into the
// omnibox, and wiring it would add a feature.
#[allow(dead_code)]
mod omnibox_ui;
mod settings;
mod style;
mod window;

pub use app::run;
