//! Local-only usage counters (never sent over the network).

use crate::store::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const STAT_PAGES: &str = "pages_loaded";
pub const STAT_BLOCKS: &str = "trackers_blocked";
pub const STAT_DOWNLOADS: &str = "downloads_completed";
pub const STAT_BOOKMARKS_ADDED: &str = "bookmarks_added";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalMetrics {
    pub pages_loaded: i64,
    pub trackers_blocked: i64,
    pub downloads_completed: i64,
    pub bookmarks_added: i64,
}

impl LocalMetrics {
    pub fn load(store: &Store) -> Result<Self> {
        Ok(Self {
            pages_loaded: store.get_stat(STAT_PAGES)?,
            trackers_blocked: store.get_stat(STAT_BLOCKS)?,
            downloads_completed: store.get_stat(STAT_DOWNLOADS)?,
            bookmarks_added: store.get_stat(STAT_BOOKMARKS_ADDED)?,
        })
    }

    pub fn summary(&self) -> String {
        format!(
            "Pages {} · Blocked {} · Downloads {} · Bookmarks added {}",
            self.pages_loaded, self.trackers_blocked, self.downloads_completed, self.bookmarks_added
        )
    }
}

pub fn record_page_load(store: &Store) {
    let _ = store.bump_stat(STAT_PAGES, 1);
}

pub fn record_blocks(store: &Store, n: i64) {
    if n > 0 {
        let _ = store.bump_stat(STAT_BLOCKS, n);
    }
}

pub fn record_download_completed(store: &Store) {
    let _ = store.bump_stat(STAT_DOWNLOADS, 1);
}

pub fn record_bookmark_added(store: &Store) {
    let _ = store.bump_stat(STAT_BOOKMARKS_ADDED, 1);
}
