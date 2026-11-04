//! WadBrowser's WebKitGTK layer: web views, network sessions, the request
//! policy that blocks ads and trackers, and download paths.
//!
//! Forked from Cactus (github.com/yohannes-mesay/cactus-browser, commit
//! 8ae5a66, MPL-2.0).

mod downloads;
mod policy;
mod session;
mod view;

pub use downloads::{
    file_uri_for_path, filename_from_url, path_from_destination, sanitize_filename, unique_download_path,
};
pub use policy::{attach_blocking_policy, wire_privacy};
pub use session::SessionBundle;
pub use view::{
    apply_privacy_headers, attach_request_filter, create_web_view, inject_cosmetic_css, load_html, navigate,
    webkit_version_string,
};
