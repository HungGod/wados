//! WadBrowser's core: preferences, the SQLite store, the omnibox, the
//! internal `cactus://` pages and bookmarks import/export.
//!
//! Forked from Cactus (github.com/yohannes-mesay/cactus-browser, commit
//! 8ae5a66, MPL-2.0).

pub mod bookmarks_io;
pub mod cli;
pub mod errors;
pub mod internal;
pub mod metrics;
pub mod omnibox;
pub mod paths;
pub mod permissions;
pub mod prefs;
pub mod project;
pub mod sanitize;
pub mod search;
pub mod store;
pub mod url_util;

pub use cli::{LaunchArgs, help_text};
pub use errors::{BrowserError, BrowserResult};
pub use omnibox::{Suggestion, SuggestionKind, suggest};
pub use paths::AppPaths;
pub use permissions::{PermissionDecision, PermissionKind, SitePermission, origin_from_url};
pub use prefs::{Preferences, SearchEngine, encode_query};
pub use store::{Bookmark, DownloadRecord, HistoryEntry, SessionState, SessionWindow, Store};
pub use url_util::normalize_navigation_input;

pub const APP_ID: &str = "io.wados.Browser";
pub const APP_NAME: &str = "WadBrowser";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const NEW_TAB_URI: &str = "cactus://newtab";
pub const HOME_URI: &str = "cactus://newtab";

pub use project::{CACTUS_COMMIT, CACTUS_LICENSE, CACTUS_REPO, CACTUS_VERSION};
