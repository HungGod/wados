//! WadBrowser's ad and tracker blocking: EasyList and EasyPrivacy as WebKit
//! content filters (made by Brave's adblock crate), the social media
//! blocker's filter, the per-site allowlist, and Brave's engine for what
//! WebKit's filters can't do (counting what they blocked, each page's
//! cosmetic CSS).
//!
//! Forked from Cactus (github.com/yohannes-mesay/cactus-browser, commit
//! 8ae5a66, MPL-2.0).

mod allowlist;
pub mod content;
mod engine;
mod lists;
mod request_type;
mod stats;

pub use allowlist::{Allowlist, site_key};
pub use content::{SOCIAL, social_site};
pub use engine::{BlockDecision, BlockReason, Prepared, PrivacyEngine, Ready};
pub use lists::{DEFAULT_FILTER_URLS, FilterListManager, LISTS_ENV, filter_identifier};
pub use request_type::classify;
pub use stats::{PrivacyStats, StatsSnapshot};
