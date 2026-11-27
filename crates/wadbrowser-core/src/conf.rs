//! `wadbrowser.conf`: the browser's settings, which it has no page of its
//! own for. The wadspace's first (`/run/wadspaces-extra/wadbrowser.conf`,
//! which wadd writes from its design, the Builder's add-ons: wad-gen's
//! `extras::wadbrowser_conf`), then the user's own
//! (`~/.config/wadbrowser/wadbrowser.conf`: on the machine, Settings ▸
//! Browser's), whose keys win. The format is wad-model's
//! ([`wad_model::browser`]):
//!
//! ```text
//! default = focus         # full (an address bar) or focus (none)
//! blocker = on            # the ad and tracker blocker
//! search = duckduckgo     # duckduckgo, startpage or wikipedia
//! block = x.com reddit.com
//! ```

use crate::modes::Mode;
use std::path::{Path, PathBuf};
use wad_model::browser::ConfText;
pub use wad_model::browser::{BrowserPrefs, Search};

/// The wadspace's, from wadd.
pub const SYSTEM: &str = "/run/wadspaces-extra/wadbrowser.conf";
pub const FILE: &str = "wadbrowser.conf";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conf {
    /// What the browser's windows open as: Full (an address bar) or Focus.
    pub default_mode: Mode,
    /// Its blocker, search engine and blocked sites.
    pub prefs: BrowserPrefs,
}

impl Default for Conf {
    fn default() -> Conf {
        Conf { default_mode: Mode::Full, prefs: BrowserPrefs::default() }
    }
}

/// The files, in the order they're read: the wadspace's, then the user's
/// (`config_dir` is the user's WadBrowser folder, `~/.config/wadbrowser`).
pub fn files(config_dir: &Path) -> Vec<PathBuf> {
    vec![PathBuf::from(SYSTEM), config_dir.join(FILE)]
}

impl Conf {
    /// Reads `files` (missing ones are fine); a later file's setting wins.
    pub fn load(files: &[PathBuf]) -> Conf {
        let mut c = Conf::default();
        for f in files {
            if let Ok(text) = std::fs::read_to_string(f) {
                c.apply(&text);
            }
        }
        c
    }

    /// One file's settings over these. Lines it can't read are skipped.
    pub fn apply(&mut self, text: &str) {
        let mut bar = self.default_mode == Mode::Full;
        ConfText::parse(text).over(&mut self.prefs, &mut bar);
        self.default_mode = if bar { Mode::Full } else { Mode::Focus };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_files_win() {
        let dir = std::env::temp_dir().join(format!("wadbrowser-conf-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let system = dir.join("system.conf");
        let user = dir.join("user.conf");
        std::fs::write(&system, "# the wadspace's\ndefault = focus\nblock = x.com\n").unwrap();
        assert_eq!(Conf::load(std::slice::from_ref(&system)).default_mode, Mode::Focus);
        std::fs::write(&user, "default = full   # mine\nbogus\nhome = x\n").unwrap();
        let both = Conf::load(&[system.clone(), user.clone(), dir.join("missing.conf")]);
        assert_eq!(both.default_mode, Mode::Full);
        assert_eq!(both.prefs.blocked, vec!["x.com"]);
        // A user file without the key leaves the wadspace's.
        std::fs::write(&user, "# nothing\n").unwrap();
        assert_eq!(Conf::load(&[system, user]).default_mode, Mode::Focus);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn defaults_and_bad_values() {
        let c = Conf::load(&[]);
        assert_eq!(c.default_mode, Mode::Full);
        assert!(c.prefs.ad_blocker);
        assert_eq!(c.prefs.search, Search::DuckDuckGo);
        let mut c = Conf::default();
        c.apply("default = kiosk\nsearch = altavista\nblocker = maybe");
        assert_eq!(c, Conf::default());
        c.apply("default=lite\nblocker = off\nsearch = wikipedia");
        assert_eq!(c.default_mode, Mode::Focus);
        assert!(!c.prefs.ad_blocker);
        assert_eq!(c.prefs.search, Search::Wikipedia);
    }

    #[test]
    fn the_files_are_the_wadspaces_then_the_users() {
        let f = files(Path::new("/home/abc/.config/wadbrowser"));
        assert_eq!(f, [PathBuf::from(SYSTEM), PathBuf::from("/home/abc/.config/wadbrowser/wadbrowser.conf")]);
    }

    #[test]
    fn wad_models_file_reads() {
        let prefs = BrowserPrefs { ad_blocker: false, search: Search::Startpage, blocked: vec!["tiktok.com".into()] };
        let mut c = Conf::default();
        c.apply(&wad_model::browser::conf("A wadspace's", false, &prefs));
        assert_eq!(c, Conf { default_mode: Mode::Focus, prefs });
    }
}
