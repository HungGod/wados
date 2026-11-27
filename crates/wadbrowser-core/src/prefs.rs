use crate::paths::AppPaths;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub homepage: String,
    /// From wadbrowser.conf ([`Preferences::take_conf`]), not this file.
    pub search_engine: SearchEngine,
    pub https_first: bool,
    /// From wadbrowser.conf ([`Preferences::take_conf`]), not this file.
    pub block_ads_trackers: bool,
    pub send_dnt: bool,
    pub send_gpc: bool,
    pub show_bookmark_bar: bool,
    pub default_zoom: f64,
    pub ask_before_download: bool,
    pub enable_javascript: bool,
    pub enable_smooth_scrolling: bool,
    pub hardware_acceleration: bool,
    pub clear_history_on_quit: bool,
    pub omnibox_suggestion_limit: usize,
    pub block_cosmetic_filters: bool,
    /// Sites that don't open (and their subdomains), from wadbrowser.conf.
    #[serde(skip)]
    pub blocked_sites: Vec<String>,
    /// A tab not looked at for this long sleeps (its page unloaded, reloaded when it's
    /// shown again); 0: tabs never sleep.
    pub sleep_tabs_after_minutes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SearchEngine {
    DuckDuckGo,
    Startpage,
    Wikipedia,
    Custom { name: String, url_template: String },
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            homepage: "wadbrowser://newtab".into(),
            search_engine: SearchEngine::DuckDuckGo,
            https_first: true,
            block_ads_trackers: true,
            send_dnt: true,
            send_gpc: true,
            show_bookmark_bar: true,
            default_zoom: 1.0,
            ask_before_download: false,
            enable_javascript: true,
            enable_smooth_scrolling: true,
            hardware_acceleration: true,
            clear_history_on_quit: false,
            omnibox_suggestion_limit: 8,
            block_cosmetic_filters: true,
            blocked_sites: vec![],
            sleep_tabs_after_minutes: 30,
        }
    }
}

impl Preferences {
    pub fn load(paths: &AppPaths) -> Result<Self> {
        let path = paths.prefs_path();
        if !path.exists() {
            let prefs = Self::default();
            prefs.save(paths)?;
            return Ok(prefs);
        }
        let text = fs::read_to_string(&path).with_context(|| format!("reading prefs {}", path.display()))?;
        let mut prefs: Self = toml::from_str(&text).context("parsing prefs.toml")?;
        // The browser's pages were at cactus:// once.
        if let Some(rest) = prefs.homepage.strip_prefix("cactus://") {
            prefs.homepage = format!("wadbrowser://{rest}");
        }
        Ok(prefs)
    }

    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        let path = paths.prefs_path();
        let text = toml::to_string_pretty(self).context("serializing prefs")?;
        fs::write(&path, text).with_context(|| format!("writing prefs {}", path.display()))?;
        Ok(())
    }

    /// wadbrowser.conf's settings (the wadspace's design, or Settings ▸
    /// Browser's), which the browser has no page for: they're never this file's.
    pub fn take_conf(&mut self, c: &crate::conf::BrowserPrefs) {
        use crate::conf::Search;
        self.block_ads_trackers = c.ad_blocker;
        self.search_engine = match c.search {
            Search::DuckDuckGo => SearchEngine::DuckDuckGo,
            Search::Startpage => SearchEngine::Startpage,
            Search::Wikipedia => SearchEngine::Wikipedia,
        };
        self.blocked_sites = c.blocked.clone();
    }

    pub fn search_url(&self, query: &str) -> String {
        let q = encode_query(query);
        match &self.search_engine {
            SearchEngine::DuckDuckGo => format!("https://duckduckgo.com/?q={q}"),
            SearchEngine::Startpage => format!("https://www.startpage.com/sp/search?query={q}"),
            SearchEngine::Wikipedia => {
                format!("https://en.wikipedia.org/w/index.php?search={q}")
            }
            SearchEngine::Custom { url_template, .. } => url_template.replace("%s", &q).replace("{searchTerms}", &q),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.default_zoom < 0.3 || self.default_zoom > 5.0 {
            anyhow::bail!("default_zoom out of range (0.3–5.0)");
        }
        if self.omnibox_suggestion_limit == 0 || self.omnibox_suggestion_limit > 50 {
            anyhow::bail!("omnibox_suggestion_limit out of range (1–50)");
        }
        if self.homepage.trim().is_empty() {
            anyhow::bail!("homepage must not be empty");
        }
        Ok(())
    }
}

pub fn encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_duckduckgo() {
        let p = Preferences::default();
        assert!(p.search_url("hello world").contains("duckduckgo.com"));
        assert!(p.search_url("hello world").contains("hello+world"));
    }

    #[test]
    fn the_confs_settings_are_taken_and_never_kept() {
        let mut p = Preferences::default();
        let c = crate::conf::BrowserPrefs {
            ad_blocker: false,
            search: crate::conf::Search::Wikipedia,
            blocked: vec!["x.com".into()],
        };
        p.take_conf(&c);
        assert!(!p.block_ads_trackers);
        assert!(p.search_url("a").starts_with("https://en.wikipedia.org/"));
        assert_eq!(p.blocked_sites, vec!["x.com"]);
        assert!(!toml::to_string(&p).unwrap().contains("x.com"));
        // An old prefs.toml's keys that are gone are skipped.
        let old: Preferences = toml::from_str("restore_session = true\nblock_social = true\n").unwrap();
        assert!(old.blocked_sites.is_empty());
    }
}
