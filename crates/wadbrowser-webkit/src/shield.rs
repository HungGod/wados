//! The shield (the header bar's popover): what the blockers stopped on a
//! page and in all, whether they're off for a site, and "Block all social
//! media". Its toggles are kept (the allowlist in the store, the social
//! blocker in the settings) and put into every view at once.

use crate::engine::engine;
use crate::view::{self, state};
use std::cell::RefCell;
use std::rc::Rc;
use wadbrowser_privacy::site_key;
use webkit6::WebView;
use webkit6::prelude::*;

type Listener = Rc<dyn Fn(Option<&WebView>)>;

thread_local! {
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
}

/// What the shield shows for a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShieldState {
    /// Blocked on the page the view shows.
    pub page: u64,
    /// Blocked in all, since WadBrowser was first used (kept in the store).
    pub total: u64,
    /// The page's site, as the toggle names it ("reddit.com"; none for WadBrowser's own pages).
    pub site: Option<String>,
    /// The blockers are off for the site.
    pub allowed: bool,
    /// The ad and tracker blocker is on at all.
    pub enabled: bool,
    /// "Block all social media".
    pub block_social: bool,
}

/// The shield for `view`'s page.
pub fn shield(view: &WebView) -> ShieldState {
    let uri = view.uri().map(String::from).unwrap_or_default();
    ShieldState {
        page: page_blocked(view),
        total: total_blocked(),
        site: site_key(&uri).filter(|_| uri.starts_with("http")),
        allowed: site_allowed(&uri),
        enabled: engine().is_some_and(|e| e.privacy.is_enabled()),
        block_social: social_blocking(),
    }
}

/// Blocked on the page `view` shows.
pub fn page_blocked(view: &WebView) -> u64 {
    state(view).map_or(0, |s| s.blocked())
}

/// Blocked in all.
pub fn total_blocked() -> u64 {
    engine().map_or(0, |e| e.total_blocked())
}

/// Whether the blockers are off for `uri`'s site.
pub fn site_allowed(uri: &str) -> bool {
    engine().is_some_and(|e| e.privacy.allowlist().contains_url(uri))
}

/// The shield's toggle: the blockers off (`allowed`) or back on for
/// `uri`'s site (a page's address, or a site: "reddit.com"). Kept, put into
/// every view, and the site's pages reload so it counts at once.
pub fn set_site_allowed(uri: &str, allowed: bool) {
    let Some(e) = engine() else { return };
    let Some(site) = site_key(uri) else { return };
    // Turning it back on takes off whatever listed site covers it ("www." and all).
    let key = if allowed { site.clone() } else { e.privacy.allowlist().covering(uri).unwrap_or(site.clone()) };
    let saved = if allowed { e.store.allowlist_add(&key) } else { e.store.allowlist_remove(&key) };
    if let Err(err) = saved {
        tracing::warn!("the allowlist: {err}");
    }
    if allowed {
        e.privacy.allowlist().add(&key);
    } else {
        e.privacy.allowlist().remove(&key);
    }
    crate::filters::reapply_all();
    for v in view::live_views() {
        let u = v.uri().map(String::from).unwrap_or_default();
        if site_key(&u).is_some_and(|s| s == key || s.ends_with(&format!(".{key}"))) {
            v.reload();
        }
    }
    changed(None);
}

/// "Block all social media" is on.
pub fn social_blocking() -> bool {
    engine().is_some_and(|e| e.prefs.borrow().block_social)
}

/// "Block all social media" on or off (kept in the settings); the social
/// sites' pages open reload.
pub fn set_social_blocking(on: bool) {
    let Some(e) = engine() else { return };
    {
        let mut prefs = e.prefs.borrow_mut();
        if prefs.block_social == on {
            return;
        }
        prefs.block_social = on;
        if let Err(err) = prefs.save(&e.paths) {
            tracing::warn!("saving the settings: {err}");
        }
    }
    crate::filters::reapply_all();
    for v in view::live_views() {
        let host = wadbrowser_core::site::host_of(&v.uri().unwrap_or_default());
        if wadbrowser_privacy::social_site(&host).is_some() {
            v.reload();
        }
    }
    changed(None);
}

/// The ad and tracker blocker on or off altogether (the settings' `block_ads_trackers`).
pub fn set_blocker_enabled(on: bool) {
    let Some(e) = engine() else { return };
    e.privacy.set_enabled(on);
    {
        let mut prefs = e.prefs.borrow_mut();
        prefs.block_ads_trackers = on;
        if let Err(err) = prefs.save(&e.paths) {
            tracing::warn!("saving the settings: {err}");
        }
    }
    crate::filters::reapply_all();
    changed(None);
}

/// Called when a view's count changes (`Some(view)`), or a toggle (`None`):
/// for the shield's number and its popover.
pub fn connect_changed(f: impl Fn(Option<&WebView>) + 'static) {
    LISTENERS.with_borrow_mut(|l| l.push(Rc::new(f)));
}

pub(crate) fn changed(view: Option<&WebView>) {
    let all = LISTENERS.with_borrow(|l| l.clone());
    for f in all {
        f(view);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_page_without_an_engine_counts_nothing() {
        assert_eq!(super::total_blocked(), 0);
        assert!(!super::site_allowed("https://example.com/"));
        assert!(!super::social_blocking());
    }
}
