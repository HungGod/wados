//! The filter lists: EasyList and EasyPrivacy, downloaded into
//! `<cache>/filters/` and refreshed once a week. Reading them can download,
//! so it's done off the main thread (`PrivacyEngine::load`).
//!
//! `WADBROWSER_FILTER_LISTS` (paths, `:`-separated) puts other lists in
//! their place and downloads nothing: tests use small local ones.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tracing::{info, warn};
use wadbrowser_core::AppPaths;

pub const DEFAULT_FILTER_URLS: &[&str] =
    &["https://easylist.to/easylist/easylist.txt", "https://easylist.to/easylist/easyprivacy.txt"];

/// Lists to use instead of EasyList and EasyPrivacy (`:`-separated paths).
pub const LISTS_ENV: &str = "WADBROWSER_FILTER_LISTS";

const MAX_AGE: Duration = Duration::from_secs(60 * 60 * 24 * 7);

pub struct FilterListManager {
    cache_dir: PathBuf,
    local: Option<Vec<PathBuf>>,
}

impl FilterListManager {
    pub fn new(paths: &AppPaths) -> Self {
        let local = std::env::var_os(LISTS_ENV)
            .filter(|v| !v.is_empty())
            .map(|v| std::env::split_paths(&v).filter(|p| !p.as_os_str().is_empty()).collect());
        Self { cache_dir: paths.filter_cache_dir(), local }
    }

    /// Only these lists (no download).
    pub fn with_lists(paths: &AppPaths, lists: Vec<PathBuf>) -> Self {
        Self { cache_dir: paths.filter_cache_dir(), local: Some(lists) }
    }

    /// The lists' text, in order: the cached ones (downloaded first when
    /// they're missing or a week old), or the override's. Blocking.
    pub fn ensure_lists(&self) -> Result<Vec<String>> {
        if let Some(local) = &self.local {
            info!(
                "filter lists from {LISTS_ENV}: {}",
                local.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
            );
            return local
                .iter()
                .map(|p| fs::read_to_string(p).with_context(|| format!("reading {}", p.display())))
                .collect();
        }
        fs::create_dir_all(&self.cache_dir)?;
        let mut rules = Vec::new();
        for (idx, url) in DEFAULT_FILTER_URLS.iter().enumerate() {
            let path = self.cache_dir.join(format!("list-{idx}.txt"));
            if needs_refresh(&path) {
                match download(url, &path) {
                    Ok(()) => info!("updated filter list from {url}"),
                    Err(err) => {
                        warn!("failed to refresh {url}: {err}");
                        if !path.exists() {
                            // Seed a minimal offline set so the engine still works.
                            fs::write(&path, MINIMAL_FILTERS)?;
                        }
                    }
                }
            }
            let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            rules.push(text);
        }
        Ok(rules)
    }
}

/// What the compiled content filter's cache is keyed by: the lists, and how
/// they're turned into WebKit's rules (the adblock crate's version and this
/// crate's [`crate::content::FORMAT`]). The same lists give the same id, so a
/// start with them only loads the filter compiled before.
pub fn filter_identifier(lists: &[String]) -> String {
    let mut h = Sha256::new();
    h.update(crate::content::FORMAT.as_bytes());
    for l in lists {
        h.update((l.len() as u64).to_le_bytes());
        h.update(l.as_bytes());
    }
    let hex: String = h.finalize().iter().take(12).map(|b| format!("{b:02x}")).collect();
    format!("{}{hex}", crate::content::LISTS_PREFIX)
}

fn needs_refresh(path: &Path) -> bool {
    match fs::metadata(path).and_then(|m| m.modified()) {
        Ok(modified) => SystemTime::now().duration_since(modified).map(|d| d > MAX_AGE).unwrap_or(true),
        Err(_) => true,
    }
}

fn download(url: &str, dest: &Path) -> Result<()> {
    let mut response = ureq::get(url)
        .header("User-Agent", format!("WadBrowser/{}", wadbrowser_core::APP_VERSION))
        .call()
        .with_context(|| format!("GET {url}"))?;
    let text = response.body_mut().read_to_string().context("reading filter body")?;
    let tmp = dest.with_extension("tmp");
    fs::write(&tmp, &text)?;
    fs::rename(tmp, dest)?;
    Ok(())
}

const MINIMAL_FILTERS: &str = r#"
! WadBrowser offline bootstrap filters
||doubleclick.net^
||googlesyndication.com^
||google-analytics.com^
||googletagmanager.com^
||facebook.net^
||scorecardresearch.com^
||adservice.google.com^
||pagead2.googlesyndication.com^
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_id_is_the_lists() {
        let a = filter_identifier(&["||ads.test^\n".into(), "".into()]);
        assert_eq!(a, filter_identifier(&["||ads.test^\n".into(), "".into()]), "the same lists, the same id");
        assert!(a.starts_with("wadbrowser-lists-") && a.len() == "wadbrowser-lists-".len() + 24, "{a}");
        // Any change, even moving a line from one list to the next, is another filter.
        assert_ne!(a, filter_identifier(&["||ads.test^\n".into(), "!".into()]));
        assert_ne!(a, filter_identifier(&["".into(), "||ads.test^\n".into()]));
        assert_ne!(a, filter_identifier(&["||ads.test^\n".into()]));
        // Only characters WebKit's store takes in a file name.
        assert!(a.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    }

    #[test]
    fn the_override_reads_local_lists() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        std::fs::write(&a, "||ads.test^\n").unwrap();
        let paths = AppPaths {
            config_dir: dir.path().join("c"),
            data_dir: dir.path().join("d"),
            cache_dir: dir.path().join("x"),
        };
        let lists = FilterListManager::with_lists(&paths, vec![a.clone()]).ensure_lists().unwrap();
        assert_eq!(lists, ["||ads.test^\n"]);
        assert!(FilterListManager::with_lists(&paths, vec![dir.path().join("none")]).ensure_lists().is_err());
        assert!(!paths.filter_cache_dir().exists(), "nothing downloaded");
    }
}
