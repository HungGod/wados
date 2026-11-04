use crate::allowlist::Allowlist;
use crate::lists::FilterListManager;
use crate::request_type::classify;
use crate::stats::{PrivacyStats, StatsSnapshot};
use adblock::Engine;
use anyhow::Result;
use std::cell::RefCell;
use std::rc::Rc;
use tracing::info;
use wadbrowser_core::AppPaths;

#[derive(Debug, Clone)]
pub struct BlockDecision {
    pub matched: bool,
    pub important: bool,
    pub redirect: Option<String>,
    pub rewritten_url: Option<String>,
    pub reason: BlockReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    Disabled,
    Allowlisted,
    EngineNotReady,
    NetworkMatch,
    Allowed,
}

/// Adblock engine wrapper.
///
/// NOTE: `adblock::Engine` is intentionally `!Send`/`!Sync`, so this lives on the
/// GTK main thread behind `Rc<RefCell<_>>`.
pub struct PrivacyEngine {
    engine: RefCell<Option<Engine>>,
    enabled: RefCell<bool>,
    cosmetic_enabled: RefCell<bool>,
    allowlist: Allowlist,
    stats: PrivacyStats,
}

impl PrivacyEngine {
    pub fn new(enabled: bool) -> Rc<Self> {
        Rc::new(Self {
            engine: RefCell::new(None),
            enabled: RefCell::new(enabled),
            cosmetic_enabled: RefCell::new(true),
            allowlist: Allowlist::new(),
            stats: PrivacyStats::new(),
        })
    }

    pub fn set_enabled(&self, enabled: bool) {
        *self.enabled.borrow_mut() = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        *self.enabled.borrow()
    }

    pub fn set_cosmetic_enabled(&self, enabled: bool) {
        *self.cosmetic_enabled.borrow_mut() = enabled;
    }

    pub fn allowlist(&self) -> &Allowlist {
        &self.allowlist
    }

    pub fn stats(&self) -> StatsSnapshot {
        self.stats.snapshot()
    }

    pub fn engine_loaded(&self) -> bool {
        self.engine.borrow().is_some()
    }

    /// Build the engine on the calling thread (must be the GTK main thread).
    pub fn load(&self, paths: &AppPaths) -> Result<()> {
        let manager = FilterListManager::new(paths);
        let lists = manager.ensure_lists()?;
        let count = lists.len();
        let mut filter_set = adblock::lists::FilterSet::new(true);
        for list in lists {
            filter_set.add_filter_list(list, adblock::lists::ParseOptions::default());
        }
        let engine = Engine::new_with_filter_set(filter_set);
        info!("privacy engine ready ({} filter list files, enabled={})", count, self.is_enabled());
        *self.engine.borrow_mut() = Some(engine);
        Ok(())
    }

    pub fn should_block(&self, url: &str, source_url: &str, request_type: &str) -> BlockDecision {
        self.evaluate(url, source_url, request_type, None)
    }

    pub fn should_block_auto(&self, url: &str, source_url: &str, mime_hint: Option<&str>) -> BlockDecision {
        let request_type = classify(url, mime_hint);
        self.evaluate(url, source_url, request_type, mime_hint)
    }

    fn evaluate(&self, url: &str, source_url: &str, request_type: &str, _mime_hint: Option<&str>) -> BlockDecision {
        if !self.is_enabled() {
            self.stats.record_allow();
            return BlockDecision {
                matched: false,
                important: false,
                redirect: None,
                rewritten_url: None,
                reason: BlockReason::Disabled,
            };
        }

        // First-party document navigations should never be filter-blocked here.
        if request_type == "document" {
            self.stats.record_allow();
            return BlockDecision {
                matched: false,
                important: false,
                redirect: None,
                rewritten_url: None,
                reason: BlockReason::Allowed,
            };
        }

        if self.allowlist.contains_url(source_url) || self.allowlist.contains_url(url) {
            self.stats.record_allow();
            return BlockDecision {
                matched: false,
                important: false,
                redirect: None,
                rewritten_url: None,
                reason: BlockReason::Allowlisted,
            };
        }

        let guard = self.engine.borrow();
        let Some(engine) = guard.as_ref() else {
            return BlockDecision {
                matched: false,
                important: false,
                redirect: None,
                rewritten_url: None,
                reason: BlockReason::EngineNotReady,
            };
        };

        // No method: Cactus never had one to give (adblock 0.9 took none), so
        // `$method=` rules don't match.
        let req = match adblock::request::Request::new(url, source_url, request_type, "") {
            Ok(r) => r,
            Err(_) => {
                self.stats.record_allow();
                return BlockDecision {
                    matched: false,
                    important: false,
                    redirect: None,
                    rewritten_url: None,
                    reason: BlockReason::Allowed,
                };
            }
        };
        let result = engine.check_network_request(&req);
        // adblock 0.9's `matched`: a blocking rule matched and no exception did.
        if result.should_block() {
            self.stats.record_block();
            BlockDecision {
                matched: true,
                important: result.important,
                redirect: result.redirect,
                rewritten_url: result.rewritten_url,
                reason: BlockReason::NetworkMatch,
            }
        } else {
            self.stats.record_allow();
            BlockDecision {
                matched: false,
                important: result.important,
                redirect: result.redirect,
                rewritten_url: result.rewritten_url,
                reason: BlockReason::Allowed,
            }
        }
    }

    pub fn cosmetic_styles(&self, url: &str) -> String {
        if !self.is_enabled() || !*self.cosmetic_enabled.borrow() {
            return String::new();
        }
        if self.allowlist.contains_url(url) {
            return String::new();
        }
        let guard = self.engine.borrow();
        let Some(engine) = guard.as_ref() else {
            return String::new();
        };
        let result = engine.url_cosmetic_resources(url);
        let mut css = String::new();
        if !result.hide_selectors.is_empty() {
            let selectors: Vec<String> = result.hide_selectors.into_iter().collect();
            css.push_str(&selectors.join(", "));
            css.push_str(" { display: none !important; }\n");
            self.stats.record_cosmetic();
        }
        css
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_known_tracker_when_loaded() {
        let paths = AppPaths {
            config_dir: std::env::temp_dir().join("wadbrowser-priv-cfg3"),
            data_dir: std::env::temp_dir().join("wadbrowser-priv-data3"),
            cache_dir: std::env::temp_dir().join("wadbrowser-priv-cache3"),
        };
        let _ = std::fs::remove_dir_all(&paths.cache_dir);
        paths.ensure().unwrap();
        std::fs::create_dir_all(paths.filter_cache_dir()).unwrap();
        std::fs::write(paths.filter_cache_dir().join("list-0.txt"), "||doubleclick.net^\n").unwrap();
        std::fs::write(paths.filter_cache_dir().join("list-1.txt"), "").unwrap();

        let engine = PrivacyEngine::new(true);
        engine.load(&paths).unwrap();
        let decision = engine.should_block("https://doubleclick.net/ads.js", "https://example.com/", "script");
        assert!(decision.matched);
        assert!(engine.stats().blocked_requests >= 1);
    }

    #[test]
    fn allowlist_bypasses_block() {
        let paths = AppPaths {
            config_dir: std::env::temp_dir().join("wadbrowser-priv-cfg4"),
            data_dir: std::env::temp_dir().join("wadbrowser-priv-data4"),
            cache_dir: std::env::temp_dir().join("wadbrowser-priv-cache4"),
        };
        let _ = std::fs::remove_dir_all(&paths.cache_dir);
        paths.ensure().unwrap();
        std::fs::create_dir_all(paths.filter_cache_dir()).unwrap();
        std::fs::write(paths.filter_cache_dir().join("list-0.txt"), "||doubleclick.net^\n").unwrap();
        std::fs::write(paths.filter_cache_dir().join("list-1.txt"), "").unwrap();

        let engine = PrivacyEngine::new(true);
        engine.load(&paths).unwrap();
        engine.allowlist().add("https://example.com");
        let decision = engine.should_block("https://doubleclick.net/ads.js", "https://example.com/", "script");
        assert!(!decision.matched);
        assert_eq!(decision.reason, BlockReason::Allowlisted);
    }
}
