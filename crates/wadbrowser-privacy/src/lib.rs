//! WadBrowser's ad and tracker blocking: Brave's adblock engine over
//! EasyList and EasyPrivacy, the per-site allowlist and session counters.
//!
//! Forked from Cactus (github.com/yohannes-mesay/cactus-browser, commit
//! 8ae5a66, MPL-2.0).

mod allowlist;
mod engine;
mod lists;
mod request_type;
mod stats;

pub use allowlist::Allowlist;
pub use engine::{BlockDecision, BlockReason, PrivacyEngine};
pub use lists::{DEFAULT_FILTER_URLS, FilterListManager};
pub use request_type::classify;
pub use stats::{PrivacyStats, StatsSnapshot};
