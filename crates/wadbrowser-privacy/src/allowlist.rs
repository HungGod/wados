use std::cell::RefCell;
use std::collections::BTreeSet;

/// The sites the blockers are off for (the shield's per-site toggle):
/// bare hosts ("reddit.com"), each covering its subdomains too, as the
/// spike's `allowedHere`. Mirrored from SQLite for checks on the UI thread.
#[derive(Default)]
pub struct Allowlist {
    sites: RefCell<BTreeSet<String>>,
}

/// A page's site as the allowlist keeps it: "https://www.Reddit.com:443/r" → "reddit.com".
/// Cactus kept origins ("https://example.com"): those read as their host.
pub fn site_key(url_or_host: &str) -> Option<String> {
    let s = url_or_host.trim();
    let host = match s.split_once("://") {
        Some((scheme, rest)) => {
            if scheme.eq_ignore_ascii_case("cactus") || scheme.eq_ignore_ascii_case("about") {
                return None;
            }
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            authority.rsplit_once('@').map_or(authority, |(_, h)| h)
        }
        None => s.split(['/', '?', '#']).next().unwrap_or(""),
    };
    let host = match host.rsplit_once(':') {
        Some((h, port)) if port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host,
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let host = host.strip_prefix("www.").map(str::to_string).unwrap_or(host);
    (!host.is_empty()).then_some(host)
}

impl Allowlist {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace_all(&self, sites: impl IntoIterator<Item = String>) {
        let mut set = self.sites.borrow_mut();
        set.clear();
        set.extend(sites.into_iter().filter_map(|s| site_key(&s)));
    }

    pub fn add(&self, site: &str) {
        if let Some(k) = site_key(site) {
            self.sites.borrow_mut().insert(k);
        }
    }

    pub fn remove(&self, site: &str) {
        if let Some(k) = site_key(site) {
            self.sites.borrow_mut().remove(&k);
        }
    }

    /// Whether `host` is listed, or under a listed site.
    pub fn contains_origin(&self, host: &str) -> bool {
        let Some(h) = site_key(host) else { return false };
        self.sites.borrow().iter().any(|a| h == *a || h.ends_with(&format!(".{a}")))
    }

    /// Whether the blockers are off for `url`'s site (never for WadBrowser's own pages).
    pub fn contains_url(&self, url: &str) -> bool {
        self.contains_origin(url)
    }

    /// The listed site that covers `url`, if one does (the one to take off to block it again).
    pub fn covering(&self, url: &str) -> Option<String> {
        let h = site_key(url)?;
        self.sites.borrow().iter().find(|a| h == **a || h.ends_with(&format!(".{a}"))).cloned()
    }

    pub fn list(&self) -> Vec<String> {
        self.sites.borrow().iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_covers_its_subdomains() {
        let list = Allowlist::new();
        list.add("https://www.example.com/x");
        assert_eq!(list.list(), ["example.com"]);
        assert!(list.contains_url("https://example.com/ads.js"));
        assert!(list.contains_url("https://shop.example.com:8443/"));
        assert!(!list.contains_url("https://notexample.com/"));
        assert!(!list.contains_url("https://tracker.test/x"));
        assert!(!list.contains_url("cactus://privacy"));
        assert_eq!(list.covering("https://a.example.com/").as_deref(), Some("example.com"));
        list.remove("example.com");
        assert!(list.list().is_empty());
    }

    #[test]
    fn cactus_origins_read_as_sites() {
        let list = Allowlist::new();
        list.replace_all(["https://reddit.com".to_string(), "http://localhost:8000".to_string()]);
        assert_eq!(list.list(), ["localhost", "reddit.com"]);
        assert_eq!(site_key("127.0.0.1:8765"), Some("127.0.0.1".into()));
    }
}
