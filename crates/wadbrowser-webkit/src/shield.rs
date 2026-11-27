//! The shield (the header bar's popover): what the blockers stopped on a
//! page and in all, and whether the ad blocker's off for a site (its toggle,
//! kept in the store's allowlist and put into every view at once). Whether
//! it's on at all, and which sites are blocked, are wadbrowser.conf's
//! ([`settings_changed`]).

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

/// The engine's side of new settings (wadbrowser.conf, already in the
/// preferences and the blocker): the site blocker compiled again if its sites
/// changed, every view's filters put right, and the pages that are blocked
/// now (or no longer) loaded again.
pub fn settings_changed(blocked_before: &[String]) {
    let Some(e) = engine() else { return };
    let blocked = e.prefs.borrow().blocked_sites.clone();
    if blocked != blocked_before {
        crate::filters::sites_changed();
    }
    crate::filters::reapply_all();
    for v in view::live_views() {
        let uri = v.uri().map(String::from).unwrap_or_default();
        // A blocked page: the page it stopped, if its site's no longer blocked.
        if uri.starts_with("wadbrowser://blocked")
            && let Some(back) = crate::scheme::query_value(&uri, "uri")
        {
            let host = wadbrowser_core::site::host_of(&back);
            if wadbrowser_privacy::blocked_site(&blocked, &host).is_none() {
                crate::navigate(&v, &back);
            }
            continue;
        }
        let host = wadbrowser_core::site::host_of(&uri);
        let now = wadbrowser_privacy::blocked_site(&blocked, &host).is_some();
        let was = wadbrowser_privacy::blocked_site(blocked_before, &host).is_some();
        if now != was {
            v.reload();
        }
    }
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
    }
}
