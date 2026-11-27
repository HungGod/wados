//! WadBrowser's settings, as a wadspace's design and the machine's Settings
//! keep them, and `wadbrowser.conf`, the file they reach the browser in.
//!
//! The browser itself has no settings page: a wadspace's come from the
//! Builder's add-ons (wad-gen writes them to
//! `/run/wadspaces-extra/wadbrowser.conf`), the machine's from Settings ▸
//! Browser (`~/.config/wadbrowser/wadbrowser.conf`). `key = value` lines,
//! `#` starts a comment; a later file's key wins, and one it doesn't name
//! is left as it was:
//!
//! ```text
//! default = full          # full (an address bar) or focus (none)
//! blocker = on            # the ad and tracker blocker
//! search = duckduckgo     # duckduckgo, startpage or wikipedia
//! block = x.com reddit.com  # sites that don't open
//! ```

use serde::{Deserialize, Serialize};

/// Sites offered in the site blocker, a tap each: social media.
pub const SUGGESTED_BLOCKS: [&str; 12] = [
    "facebook.com",
    "instagram.com",
    "tiktok.com",
    "x.com",
    "twitter.com",
    "reddit.com",
    "youtube.com",
    "snapchat.com",
    "threads.net",
    "linkedin.com",
    "pinterest.com",
    "bsky.app",
];

/// The search engines the address bar can use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Search {
    #[default]
    DuckDuckGo,
    Startpage,
    Wikipedia,
}

impl Search {
    pub const ALL: [Search; 3] = [Search::DuckDuckGo, Search::Startpage, Search::Wikipedia];

    pub fn id(self) -> &'static str {
        match self {
            Search::DuckDuckGo => "duckduckgo",
            Search::Startpage => "startpage",
            Search::Wikipedia => "wikipedia",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Search::DuckDuckGo => "DuckDuckGo",
            Search::Startpage => "Startpage",
            Search::Wikipedia => "Wikipedia",
        }
    }

    pub fn from_id(id: &str) -> Option<Search> {
        Search::ALL.into_iter().find(|s| s.id() == id)
    }
}

/// A browser's settings (a wadspace's, or the machine's).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BrowserPrefs {
    /// The ad and tracker blocker.
    pub ad_blocker: bool,
    /// What the address bar searches with.
    pub search: Search,
    /// Sites that don't open (each with its subdomains), as [`site`] keeps them.
    pub blocked: Vec<String>,
}

impl Default for BrowserPrefs {
    fn default() -> Self {
        BrowserPrefs { ad_blocker: true, search: Search::default(), blocked: vec![] }
    }
}

impl BrowserPrefs {
    pub fn is_default(&self) -> bool {
        *self == BrowserPrefs::default()
    }

    /// `text` blocked too (made a [`site`] first): whether it was added.
    pub fn block(&mut self, text: &str) -> bool {
        match site(text) {
            Some(s) if !self.blocked.contains(&s) => {
                self.blocked.push(s);
                true
            }
            _ => false,
        }
    }

    /// `site` no longer blocked: whether it was.
    pub fn unblock(&mut self, site: &str) -> bool {
        let before = self.blocked.len();
        self.blocked.retain(|s| s != site);
        self.blocked.len() != before
    }

    /// Every blocked site as a [`site`], once each, in order.
    pub fn tidied(mut self) -> Self {
        let mut seen = vec![];
        for s in std::mem::take(&mut self.blocked) {
            if let Some(s) = site(&s)
                && !seen.contains(&s)
            {
                seen.push(s);
            }
        }
        self.blocked = seen;
        self
    }
}

/// What someone typed as a site to block, as the blocker keeps it: its host,
/// lower case, without `www.`, a scheme, a port or a path
/// (`https://www.Reddit.com/r/x` → `reddit.com`). None if it isn't a host.
pub fn site(text: &str) -> Option<String> {
    let t = text.trim().to_ascii_lowercase();
    let t = t.split_once("://").map_or(t.as_str(), |(_, rest)| rest);
    let host = t.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split(':').next().unwrap_or("");
    let host = host.trim_end_matches('.');
    let host = host.strip_prefix("www.").unwrap_or(host);
    let ok = host.contains('.')
        && host.len() <= 253
        && host.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    ok.then(|| host.to_string())
}

/// Whether `host` is `blocked` or under one of them (`m.reddit.com` under `reddit.com`).
pub fn blocked_site<'a>(blocked: &'a [String], host: &str) -> Option<&'a str> {
    let h = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let h = h.strip_prefix("www.").unwrap_or(&h);
    blocked.iter().map(String::as_str).find(|s| h == *s || h.ends_with(&format!(".{s}")))
}

/// What a `wadbrowser.conf` says (each key it has; the rest are None).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfText {
    /// `default`: the address bar (true) or not (focus).
    pub address_bar: Option<bool>,
    pub ad_blocker: Option<bool>,
    pub search: Option<Search>,
    pub blocked: Option<Vec<String>>,
}

impl ConfText {
    /// Reads one file's lines; ones it can't read are skipped.
    pub fn parse(text: &str) -> ConfText {
        let mut c = ConfText::default();
        for line in text.lines() {
            let line = line.split_once('#').map_or(line, |(l, _)| l).trim();
            let Some((key, value)) = line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) else { continue };
            match key {
                "default" => match value {
                    "full" => c.address_bar = Some(true),
                    "focus" | "lite" => c.address_bar = Some(false),
                    _ => {}
                },
                "blocker" => match value {
                    "on" | "true" | "yes" => c.ad_blocker = Some(true),
                    "off" | "false" | "no" => c.ad_blocker = Some(false),
                    _ => {}
                },
                "search" => {
                    if let Some(s) = Search::from_id(value) {
                        c.search = Some(s);
                    }
                }
                "block" => {
                    let mut p = BrowserPrefs { blocked: vec![], ..Default::default() };
                    for s in value.split([' ', ',', '\t']).filter(|s| !s.is_empty()) {
                        p.block(s);
                    }
                    c.blocked = Some(p.blocked);
                }
                _ => {}
            }
        }
        c
    }

    /// These keys over `prefs` and `address_bar`.
    pub fn over(&self, prefs: &mut BrowserPrefs, address_bar: &mut bool) {
        if let Some(a) = self.address_bar {
            *address_bar = a;
        }
        if let Some(b) = self.ad_blocker {
            prefs.ad_blocker = b;
        }
        if let Some(s) = self.search {
            prefs.search = s;
        }
        if let Some(b) = &self.blocked {
            prefs.blocked = b.clone();
        }
    }
}

/// A `wadbrowser.conf` for `prefs`, with `address_bar` (else focus), headed by `about`.
pub fn conf(about: &str, address_bar: bool, prefs: &BrowserPrefs) -> String {
    let mut out = String::new();
    for l in about.lines() {
        out.push_str(if l.is_empty() { "#" } else { "# " });
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("#\n# Tabs and an address bar (full), or tabs alone (focus).\n");
    out.push_str(&format!("default = {}\n", if address_bar { "full" } else { "focus" }));
    out.push_str("# The ad and tracker blocker.\n");
    out.push_str(&format!("blocker = {}\n", if prefs.ad_blocker { "on" } else { "off" }));
    out.push_str("# What the address bar searches with: duckduckgo, startpage or wikipedia.\n");
    out.push_str(&format!("search = {}\n", prefs.search.id()));
    out.push_str("# Sites that don't open (and their subdomains).\n");
    out.push_str(&format!("block = {}\n", prefs.blocked.join(" ")));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_is_its_bare_host() {
        assert_eq!(site("https://www.Reddit.com/r/rust?x#y").as_deref(), Some("reddit.com"));
        assert_eq!(site(" x.com ").as_deref(), Some("x.com"));
        assert_eq!(site("user@m.facebook.com:443/").as_deref(), Some("m.facebook.com"));
        assert_eq!(site("localhost"), None);
        assert_eq!(site("not a site.com"), None);
        assert_eq!(site(""), None);
        assert_eq!(site("-bad.com"), None);
    }

    #[test]
    fn subdomains_are_blocked_with_their_site() {
        let b = vec!["reddit.com".to_string()];
        assert_eq!(blocked_site(&b, "old.reddit.com"), Some("reddit.com"));
        assert_eq!(blocked_site(&b, "www.reddit.com"), Some("reddit.com"));
        assert_eq!(blocked_site(&b, "notreddit.com"), None);
    }

    #[test]
    fn block_and_unblock_keep_one_of_each() {
        let mut p = BrowserPrefs::default();
        assert!(p.block("https://www.x.com/home"));
        assert!(!p.block("x.com"));
        assert!(!p.block("nonsense"));
        assert_eq!(p.blocked, vec!["x.com"]);
        assert!(p.unblock("x.com"));
        assert!(!p.unblock("x.com"));
    }

    #[test]
    fn a_conf_reads_back_what_was_written() {
        let prefs = BrowserPrefs {
            ad_blocker: false,
            search: Search::Startpage,
            blocked: vec!["x.com".into(), "reddit.com".into()],
        };
        let text = conf("WadBrowser's settings\n\nfor tests", false, &prefs);
        assert!(text.starts_with("# WadBrowser's settings\n#\n# for tests\n"));
        let mut back = BrowserPrefs::default();
        let mut bar = true;
        ConfText::parse(&text).over(&mut back, &mut bar);
        assert_eq!(back, prefs);
        assert!(!bar);
    }

    #[test]
    fn a_later_file_changes_only_what_it_names() {
        let mut p = BrowserPrefs::default();
        let mut bar = true;
        ConfText::parse("blocker = off\nblock = x.com, tiktok.com\n").over(&mut p, &mut bar);
        ConfText::parse("default = focus # mine\nsearch = wikipedia\nnonsense\n").over(&mut p, &mut bar);
        assert!(!p.ad_blocker && !bar);
        assert_eq!(p.search, Search::Wikipedia);
        assert_eq!(p.blocked, vec!["x.com", "tiktok.com"]);
        // An empty `block =` unblocks everything.
        ConfText::parse("block =\n").over(&mut p, &mut bar);
        assert!(p.blocked.is_empty());
    }

    #[test]
    fn prefs_keep_only_what_isnt_their_default() {
        assert!(BrowserPrefs::default().is_default());
        let p: BrowserPrefs = serde_json::from_str(r#"{"blocked":["https://www.X.com/", "x.com", "nope"]}"#).unwrap();
        assert!(p.ad_blocker);
        assert_eq!(p.tidied().blocked, vec!["x.com"]);
    }
}
