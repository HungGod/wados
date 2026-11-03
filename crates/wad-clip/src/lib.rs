//! One clipboard for the whole machine.
//!
//! Each wadspace draws through its own labwc, and each labwc keeps its own
//! clipboard, so a copy in one couldn't be pasted in another. Two halves fix
//! that, both on wlr-data-control (wl.rs):
//!
//! - `wadspaces-clipboard`, in every wadspace (labwc's autostart), carries
//!   copies both ways between its labwc and the machine's sway.
//! - The HUD watches sway's clipboard: it keeps the history (Super+V), puts
//!   a pick back, and copies the latest again when whoever copied it goes
//!   away (a wadspace stopped), so a copy outlives its window.
//!
//! What's carried, and how a copy is known for one's own: content.rs.

pub mod content;
pub mod wl;

pub use content::{Content, Item};
pub use wl::{Change, Clipboard};
