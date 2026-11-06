//! The person's session settings, shared by Settings (which writes them)
//! and the shell (which follows them):
//!
//! - [`settings`]: the settings file (night light, the idle times, the lid,
//!   locking, the background, the mouse and keyboard, the clock);
//! - [`layout`]: the displays as data, the layouts kept for each set of
//!   them (and the primary), and the arrangement's snapping;
//! - [`outputs`]: labwc's output management, gamma (night light) and output
//!   power (screen blank), on a Wayland connection of its own;
//! - [`sound`]: outputs, inputs and volumes through `wpctl`, the speaker
//!   test and the input level;
//! - [`labwc`]: the person's rc.xml and environment (mouse, touchpad,
//!   keyboard), and `labwc --reconfigure`;
//! - [`pam`]: checking a password, and changing it.

pub mod labwc;
pub mod layout;
pub mod outputs;
pub mod pam;
pub mod settings;
pub mod sound;
