//! WadSpaces' look in GTK4, dark only, for the shell and the machine's apps:
//! the spike's tokens as CSS custom properties, its fonts (Inter, Space
//! Grotesk), lucide icons drawn in the text's colour, springs, and lists kept
//! by key.

pub mod fonts;
pub mod icon;
pub mod keyed;
pub mod session_lock;
pub mod spring;
pub mod theme;

pub use icon::{Icon, Svg};
pub use spring::{Params, Spring};
