//! The panels, one module each, and the registry: which `make` gives
//! each of `nav.rs`'s panels. Every panel has its line already; one
//! that's not built yet is a stub giving a [`Placeholder`](crate::panel::Placeholder),
//! so building a panel is replacing its module (and nothing here).

use crate::panel::Make;

pub mod accounts;
pub mod background;
pub mod bluetooth;
pub mod displays;
pub mod keyboard;
pub mod lock;
pub mod mouse;
pub mod network;
pub mod power;
pub mod sound;
pub mod storage;
pub mod system;
pub mod wifi;

/// Each panel's `make`, by its id (the sidebar's order is `nav::GROUPS`').
pub const REGISTRY: &[(&str, Make)] = &[
    ("wifi", wifi::make),
    ("network", network::make),
    ("bluetooth", bluetooth::make),
    ("displays", displays::make),
    ("sound", sound::make),
    ("power", power::make),
    ("background", background::make),
    ("lock", lock::make),
    ("accounts", accounts::make),
    ("storage", storage::make),
    ("mouse", mouse::make),
    ("keyboard", keyboard::make),
    ("system", system::make),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav;

    #[test]
    fn every_panel_is_registered_once() {
        let ids: Vec<_> = REGISTRY.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, nav::panels().map(|p| p.id).collect::<Vec<_>>());
    }
}
