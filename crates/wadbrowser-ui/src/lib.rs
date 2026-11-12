//! WadBrowser's windows, as the v2 design draws them: the tab bar, the
//! header bar (the address, the shield, the profile chip, ⋮), the menus and
//! dialogs, find in page and shortcuts.
//!
//! Forked from Cactus (github.com/yohannes-mesay/cactus-browser, commit
//! 8ae5a66, MPL-2.0).

mod app;
mod window;

// The v2 design's widgets, each taking the data it shows and what it does
// (window/wire.rs wires them; examples/gallery.rs shows them alone).
pub mod chip;
pub mod data;
pub mod dialogs;
pub mod downloads;
pub mod menu_popover;
pub mod menus;
pub mod shield;
pub mod style;

pub use app::run;
