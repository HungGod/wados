//! WadBrowser's WebKitGTK layer: a network session per profile (cookies
//! kept), web views with a UserContentManager each, popups that keep
//! `window.opener`, per-site keys, the content filters and the shield,
//! the `cactus://` scheme, permission prompts, per-site zoom, downloads and
//! tab sleeping.
//!
//! The UI starts it ([`init`]), says what windows do for pages ([`set_hooks`])
//! and serves the `cactus://` pages ([`scheme::set_page_source`]).
//!
//! Forked from Cactus (github.com/yohannes-mesay/cactus-browser, commit
//! 8ae5a66, MPL-2.0).

pub mod downloads;
mod engine;
pub mod filters;
pub mod permissions;
pub mod scheme;
mod session;
pub mod shield;
pub mod sleep;
pub mod testlog;
mod view;
pub mod wadd;
pub mod zoom;

pub use downloads::{
    DownloadEvent, file_uri_for_path, filename_from_url, path_from_destination, sanitize_filename, unique_download_path,
};
pub use engine::{Engine, Hooks, engine, init, set_hooks};
pub use permissions::Ask;
pub use scheme::{Page, PageRequest, set_page_source};
pub use session::{
    SessionBundle, ephemeral_run, forget, network_session, set_ephemeral_run, set_machine_keys, signins,
};
pub use view::{
    PAGE_HANDLER, ViewState, add_user_script, apply_privacy_headers, create_related_view, create_web_view,
    inject_cosmetic_css, live_views, load_html, navigate, profile_for_uri, rehome, state, webkit_version_string,
};
