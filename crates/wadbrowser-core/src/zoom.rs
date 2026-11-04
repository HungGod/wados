//! Page zoom, remembered per site (`<config>/zoom.json`, v1's): a site's
//! level comes back whenever one of its pages opens. The levels a step
//! goes through are v1's.

use crate::paths::AppPaths;
use crate::profiles::bare_host;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const LEVELS: &[f64] = &[0.3, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    In,
    Out,
    Reset,
}

/// The next level from `now` (Reset: `default`).
pub fn step(now: f64, step: Step, default: f64) -> f64 {
    match step {
        Step::In => LEVELS.iter().copied().find(|&l| l > now + 0.001).unwrap_or(now),
        Step::Out => LEVELS.iter().rev().copied().find(|&l| l < now - 0.001).unwrap_or(now),
        Step::Reset => default,
    }
}

/// Each site's level (sites at the default aren't kept).
#[derive(Debug, Default)]
pub struct SiteZoom {
    file: Option<PathBuf>,
    levels: BTreeMap<String, f64>,
}

impl SiteZoom {
    pub fn open(paths: &AppPaths) -> Self {
        let file = paths.config_dir.join("zoom.json");
        let levels = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { file: Some(file), levels }
    }

    /// One that isn't saved (a private window's).
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// The level for `host`'s pages, if it has one of its own.
    pub fn get(&self, host: &str) -> Option<f64> {
        self.levels.get(&bare_host(host)).copied()
    }

    /// `host`'s level from now on (`default`: forgotten).
    pub fn set(&mut self, host: &str, level: f64, default: f64) {
        let h = bare_host(host);
        if h.is_empty() {
            return;
        }
        if (level - default).abs() < 0.001 {
            self.levels.remove(&h);
        } else {
            self.levels.insert(h, level);
        }
        if let Some(file) = &self.file {
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(file, serde_json::to_vec_pretty(&self.levels).unwrap_or_default()) {
                tracing::warn!("zoom.json: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_go_through_the_levels() {
        assert_eq!(step(1.0, Step::In, 1.0), 1.1);
        assert_eq!(step(1.0, Step::Out, 1.0), 0.9);
        assert_eq!(step(1.05, Step::In, 1.0), 1.1);
        assert_eq!(step(3.0, Step::In, 1.0), 3.0);
        assert_eq!(step(0.3, Step::Out, 1.0), 0.3);
        assert_eq!(step(2.0, Step::Reset, 1.25), 1.25);
    }

    #[test]
    fn a_sites_level_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            config_dir: dir.path().join("c"),
            data_dir: dir.path().join("d"),
            cache_dir: dir.path().join("x"),
        };
        let mut z = SiteZoom::open(&paths);
        z.set("www.example.com", 1.5, 1.0);
        z.set("other.test", 1.0, 1.0);
        let z = SiteZoom::open(&paths);
        assert_eq!(z.get("example.com"), Some(1.5));
        assert_eq!(z.get("other.test"), None);
    }
}
