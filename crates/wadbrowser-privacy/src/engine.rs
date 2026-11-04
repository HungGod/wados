use crate::allowlist::Allowlist;
use crate::lists::{FilterListManager, filter_identifier};
use crate::request_type::classify;
use crate::stats::{PrivacyStats, StatsSnapshot};
use adblock::Engine;
use anyhow::Result;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{info, warn};
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

impl BlockDecision {
    fn not(reason: BlockReason) -> Self {
        Self { matched: false, important: false, redirect: None, rewritten_url: None, reason }
    }
}

/// The lists, read and parsed off the main thread, waiting for it.
pub struct Prepared {
    /// The content filter's cache id for these lists.
    pub identifier: String,
    /// The lists' text, to compile the content filter from when it isn't cached.
    pub lists: Arc<Vec<String>>,
    /// The adblock engine, serialized (it can't cross threads as it is).
    engine: Vec<u8>,
    /// How long reading and parsing took.
    pub took: Duration,
}

/// What [`PrivacyEngine::poll`] gives once the engine is ready: the
/// content filter to load (or compile) for these lists.
#[derive(Clone)]
pub struct Ready {
    pub identifier: String,
    pub lists: Arc<Vec<String>>,
}

type Pending = Arc<Mutex<Option<Result<Prepared, String>>>>;

/// The adblock engine, for what WebKit's content filter can't do: counting
/// what it blocked (is this URL one the lists block?) and each page's
/// cosmetic CSS. WebKit's content filter does the blocking.
///
/// NOTE: `adblock::Engine` is `!Send`, so this lives on the GTK main
/// thread; [`PrivacyEngine::load`] reads and parses the lists on another
/// and hands the engine over serialized.
pub struct PrivacyEngine {
    engine: RefCell<Option<Engine>>,
    enabled: Cell<bool>,
    cosmetic_enabled: Cell<bool>,
    allowlist: Allowlist,
    stats: PrivacyStats,
    pending: Pending,
    started: Cell<bool>,
    ready: RefCell<Option<Ready>>,
}

impl PrivacyEngine {
    pub fn new(enabled: bool) -> Rc<Self> {
        Rc::new(Self {
            engine: RefCell::new(None),
            enabled: Cell::new(enabled),
            cosmetic_enabled: Cell::new(true),
            allowlist: Allowlist::new(),
            stats: PrivacyStats::new(),
            pending: Arc::default(),
            started: Cell::new(false),
            ready: RefCell::new(None),
        })
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.get()
    }

    pub fn set_cosmetic_enabled(&self, enabled: bool) {
        self.cosmetic_enabled.set(enabled);
    }

    pub fn allowlist(&self) -> &Allowlist {
        &self.allowlist
    }

    pub fn stats(&self) -> StatsSnapshot {
        self.stats.snapshot()
    }

    /// Counts requests WebKit's filter blocked (each once).
    pub fn record_blocked(&self, n: u64) {
        for _ in 0..n {
            self.stats.record_block();
        }
    }

    pub fn engine_loaded(&self) -> bool {
        self.poll();
        self.engine.borrow().is_some()
    }

    /// Starts reading the lists (downloading them if they're old) and
    /// parsing them, on a thread of its own; it returns at once. The engine
    /// is ready when [`poll`](Self::poll) says so. A second call does nothing.
    pub fn load(&self, paths: &AppPaths) -> Result<()> {
        self.load_from(FilterListManager::new(paths));
        Ok(())
    }

    pub fn load_from(&self, lists: FilterListManager) {
        if self.started.replace(true) {
            return;
        }
        let pending = self.pending.clone();
        let spawned = std::thread::Builder::new().name("wadbrowser-lists".into()).spawn(move || {
            let out = prepare(&lists).map_err(|e| format!("{e:#}"));
            *pending.lock().unwrap() = Some(out);
        });
        if let Err(e) = spawned {
            warn!("can't read the filter lists: {e}");
        }
    }

    /// Reads and parses the lists here and now (tests).
    pub fn load_now(&self, lists: FilterListManager) -> Result<Ready> {
        self.started.set(true);
        let p = prepare(&lists)?;
        *self.pending.lock().unwrap() = Some(Ok(p));
        self.poll().ok_or_else(|| anyhow::anyhow!("not ready"))
    }

    /// Takes in the lists once they're read (on the main thread): the
    /// content filter to use, the first time it's ready; None before then
    /// and after.
    pub fn poll(&self) -> Option<Ready> {
        let done = self.pending.lock().unwrap().take()?;
        match done {
            Ok(p) => {
                let started = Instant::now();
                let mut engine = Engine::default();
                if let Err(e) = engine.deserialize(&p.engine) {
                    warn!("the blocker's engine didn't load: {e:?}");
                    return None;
                }
                info!(
                    "privacy engine ready ({} lists, {} ms reading and parsing, {} ms here, enabled={})",
                    p.lists.len(),
                    p.took.as_millis(),
                    started.elapsed().as_millis(),
                    self.is_enabled()
                );
                *self.engine.borrow_mut() = Some(engine);
                let ready = Ready { identifier: p.identifier, lists: p.lists };
                *self.ready.borrow_mut() = Some(ready.clone());
                Some(ready)
            }
            Err(e) => {
                warn!("privacy engine load failed: {e}");
                None
            }
        }
    }

    /// The lists, once they're read (for compiling the content filter again).
    pub fn ready(&self) -> Option<Ready> {
        self.ready.borrow().clone()
    }

    /// Whether the lists block `url`, loaded by the page at `source_url`
    /// as `request_type` (adblock's names: "script", "image", "sub_frame"…).
    /// It counts nothing.
    pub fn should_block(&self, url: &str, source_url: &str, request_type: &str) -> BlockDecision {
        self.evaluate(url, source_url, request_type)
    }

    pub fn should_block_auto(&self, url: &str, source_url: &str, mime_hint: Option<&str>) -> BlockDecision {
        let request_type = classify(url, mime_hint);
        self.evaluate(url, source_url, request_type)
    }

    fn evaluate(&self, url: &str, source_url: &str, request_type: &str) -> BlockDecision {
        if !self.is_enabled() {
            return BlockDecision::not(BlockReason::Disabled);
        }
        // The page you went to is never the lists' to block.
        if request_type == "document" {
            return BlockDecision::not(BlockReason::Allowed);
        }
        // The blockers off for the page's site (the shield's toggle).
        if self.allowlist.contains_url(source_url) {
            return BlockDecision::not(BlockReason::Allowlisted);
        }
        self.poll();
        let guard = self.engine.borrow();
        let Some(engine) = guard.as_ref() else {
            return BlockDecision::not(BlockReason::EngineNotReady);
        };
        // No method: Cactus never had one to give (adblock 0.9 took none), so
        // `$method=` rules don't match.
        let Ok(req) = adblock::request::Request::new(url, source_url, request_type, "") else {
            return BlockDecision::not(BlockReason::Allowed);
        };
        let result = engine.check_network_request(&req);
        let matched = result.should_block();
        BlockDecision {
            matched,
            important: result.important,
            redirect: result.redirect,
            rewritten_url: result.rewritten_url,
            reason: if matched { BlockReason::NetworkMatch } else { BlockReason::Allowed },
        }
    }

    /// The CSS that hides `url`'s page's ads (its site's hiding rules), or
    /// none: the blocker's off, or off for this site.
    pub fn cosmetic_styles(&self, url: &str) -> String {
        if !self.is_enabled() || !self.cosmetic_enabled.get() || self.allowlist.contains_url(url) {
            return String::new();
        }
        self.poll();
        let guard = self.engine.borrow();
        let Some(engine) = guard.as_ref() else {
            return String::new();
        };
        let result = engine.url_cosmetic_resources(url);
        let mut css = String::new();
        if !result.hide_selectors.is_empty() {
            let mut selectors: Vec<String> = result.hide_selectors.into_iter().collect();
            selectors.sort();
            css.push_str(&selectors.join(", "));
            css.push_str(" { display: none !important; }\n");
            self.stats.record_cosmetic();
        }
        css
    }
}

fn prepare(lists: &FilterListManager) -> Result<Prepared> {
    let started = Instant::now();
    let texts = lists.ensure_lists()?;
    let identifier = filter_identifier(&texts);
    let mut set = adblock::lists::FilterSet::new(false);
    for t in &texts {
        set.add_filter_list(t.clone(), adblock::lists::ParseOptions::default());
    }
    let engine = Engine::new_with_filter_set(set).serialize();
    Ok(Prepared { identifier, lists: Arc::new(texts), engine, took: started.elapsed() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_with(list: &str) -> (Rc<PrivacyEngine>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("list.txt");
        std::fs::write(&file, list).unwrap();
        let paths = AppPaths {
            config_dir: dir.path().join("c"),
            data_dir: dir.path().join("d"),
            cache_dir: dir.path().join("x"),
        };
        let engine = PrivacyEngine::new(true);
        engine.load_now(FilterListManager::with_lists(&paths, vec![file])).unwrap();
        (engine, dir)
    }

    #[test]
    fn blocks_known_tracker_when_loaded() {
        let (engine, _dir) = engine_with("||doubleclick.net^\n");
        let decision = engine.should_block("https://doubleclick.net/ads.js", "https://example.com/", "script");
        assert!(decision.matched);
        assert!(!engine.should_block("https://example.com/app.js", "https://example.com/", "script").matched);
        // Asking counts nothing: what's blocked is counted once, by whoever saw it blocked.
        assert_eq!(engine.stats().blocked_requests, 0);
        engine.record_blocked(2);
        assert_eq!(engine.stats().blocked_requests, 2);
    }

    #[test]
    fn allowlist_bypasses_block() {
        let (engine, _dir) = engine_with("||doubleclick.net^\n");
        engine.allowlist().add("example.com");
        let decision = engine.should_block("https://doubleclick.net/ads.js", "https://www.example.com/", "script");
        assert!(!decision.matched);
        assert_eq!(decision.reason, BlockReason::Allowlisted);
    }

    #[test]
    fn it_loads_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("list.txt");
        std::fs::write(&file, "||ads.test^\n").unwrap();
        let paths = AppPaths {
            config_dir: dir.path().join("c"),
            data_dir: dir.path().join("d"),
            cache_dir: dir.path().join("x"),
        };
        let engine = PrivacyEngine::new(true);
        engine.load_from(FilterListManager::with_lists(&paths, vec![file]));
        let ready = (0..500)
            .find_map(|_| {
                std::thread::sleep(Duration::from_millis(10));
                engine.poll()
            })
            .expect("ready");
        assert_eq!(ready.identifier, filter_identifier(&["||ads.test^\n".to_string()]));
        assert!(engine.should_block("https://ads.test/x.js", "https://example.com/", "script").matched);
        assert!(engine.poll().is_none(), "once");
        assert!(engine.ready().is_some());
    }
}
