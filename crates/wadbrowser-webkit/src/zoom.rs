//! Page zoom, remembered per site (wadbrowser-core's `zoom`): a site's
//! level comes back as each of its pages commits; a private window's
//! changes aren't kept.

use crate::engine::engine;
use crate::view::state;
pub use wadbrowser_core::zoom::Step;
use webkit6::WebView;
use webkit6::prelude::*;

fn host(view: &WebView) -> Option<String> {
    let uri = view.uri()?;
    let h = wadbrowser_core::site::host_of(&uri);
    (!h.is_empty() && uri.starts_with("http")).then_some(h)
}

/// The site's level (else the settings' default), as a page commits.
pub(crate) fn apply(view: &WebView) {
    let Some(e) = engine() else { return };
    let default = e.prefs.borrow().default_zoom;
    let level = host(view).and_then(|h| e.zoom.borrow().get(&h)).unwrap_or(default);
    if (view.zoom_level() - level).abs() > 0.001 {
        view.set_zoom_level(level);
    }
}

/// Zooms in, out, or back to the default, and remembers it for the site.
pub fn change(view: &WebView, step: Step) {
    let default = engine().map_or(1.0, |e| e.prefs.borrow().default_zoom);
    let level = wadbrowser_core::zoom::step(view.zoom_level(), step, default);
    view.set_zoom_level(level);
    let private = state(view).is_some_and(|s| s.private);
    if let (Some(e), Some(h), false) = (engine(), host(view), private) {
        e.zoom.borrow_mut().set(&h, level, default);
    }
}

pub fn zoom_in(view: &WebView) {
    change(view, Step::In);
}

pub fn zoom_out(view: &WebView) {
    change(view, Step::Out);
}

pub fn zoom_reset(view: &WebView) {
    change(view, Step::Reset);
}
