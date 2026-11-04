use std::cell::RefCell;
use std::collections::HashSet;
use wadbrowser_core::origin_from_url;

/// In-memory allowlist mirrored from SQLite for fast checks on the UI thread.
#[derive(Default)]
pub struct Allowlist {
    origins: RefCell<HashSet<String>>,
}

impl Allowlist {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace_all(&self, origins: impl IntoIterator<Item = String>) {
        let mut set = self.origins.borrow_mut();
        set.clear();
        set.extend(origins);
    }

    pub fn add(&self, origin: &str) {
        self.origins.borrow_mut().insert(origin.to_string());
    }

    pub fn remove(&self, origin: &str) {
        self.origins.borrow_mut().remove(origin);
    }

    pub fn contains_origin(&self, origin: &str) -> bool {
        self.origins.borrow().contains(origin)
    }

    pub fn contains_url(&self, url: &str) -> bool {
        match origin_from_url(url) {
            Some(origin) => self.contains_origin(&origin),
            None => false,
        }
    }

    pub fn list(&self) -> Vec<String> {
        let mut v: Vec<_> = self.origins.borrow().iter().cloned().collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_matches_origin() {
        let list = Allowlist::new();
        list.add("https://example.com");
        assert!(list.contains_url("https://example.com/ads.js"));
        assert!(!list.contains_url("https://tracker.test/x"));
    }
}
