//! Wayland pieces for wadd that don't depend on the shell.
//!
//! - [`security_context`]: a socket the compositor serves to one sandboxed
//!   wadspace, its nested labwc's only way onto the screen.

pub mod security_context;
