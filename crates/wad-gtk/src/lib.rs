//! WadSpaces' look in GTK4, dark only, for the shell and the machine's apps:
//! the spike's tokens as CSS custom properties, its fonts (Inter, Space
//! Grotesk), lucide icons drawn in the text's colour, springs, and lists kept
//! by key, and the machine apps' pieces ([`kit`]).

pub mod app_icon;
pub mod assets;
pub mod fonts;
pub mod icon;
pub mod keyed;
pub mod kit;
pub mod picture;
pub mod session_lock;
pub mod spring;
pub mod theme;
pub mod tile;
pub mod wrap;

pub use icon::{Icon, Svg};
pub use picture::Picture;
pub use spring::{Params, Spring};
pub use tile::{TileState, WadspaceTile};
pub use wrap::Wrap;
