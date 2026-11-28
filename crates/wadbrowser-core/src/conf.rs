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
//! groups = social-media   # blocker groups on: the machine's
//! block = x.com reddit.com
//! ```
//!
//! A group's sites are the machine's groups file's ([`GROUPS`], which wadd
//! keeps as the Library has them); they're blocked with the file's own.

use crate::modes::Mode;
use std::path::{Path, PathBuf};
pub use wad_model::browser::BrowserPrefs;
use wad_model::browser::{ConfText, parse_groups};

/// The wadspace's, from wadd.
pub const SYSTEM: &str = "/run/wadspaces-extra/wadbrowser.conf";
pub const FILE: &str = "wadbrowser.conf";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conf {
    /// What the browser's windows open as: Full (an address bar) or Focus.
    pub default_mode: Mode,
    /// Its address bar, blocker and blocker groups.
    pub prefs: BrowserPrefs,
    /// The sites that don't open: `block`'s (a wadspace's file has its
    /// groups' in it), and the sites of the groups on, from the groups file.
    pub blocked: Vec<String>,
}

impl Default for Conf {
    fn default() -> Conf {
        Conf { default_mode: Mode::Full, prefs: BrowserPrefs::default(), blocked: vec![] }
    }
}

/// The machine's blocker groups (wadd's): `id = site site …`.
pub const GROUPS: &str = wad_model::browser::MACHINE_GROUPS;

/// The files, in the order they're read: the wadspace's, then the user's
/// (`config_dir` is the user's WadBrowser folder, `~/.config/wadbrowser`).
pub fn files(config_dir: &Path) -> Vec<PathBuf> {
    vec![PathBuf::from(SYSTEM), config_dir.join(FILE)]
}

impl Conf {
    /// Reads `files` (missing ones are fine; a later file's setting wins),
    /// then blocks the sites of the groups they have on, as `groups` (the
    /// groups file) says, with their own.
    pub fn load(files: &[PathBuf], groups: &Path) -> Conf {
        let mut c = Conf::default();
        for f in files {
            if let Ok(text) = std::fs::read_to_string(f) {
                c.apply(&text);
            }
        }
        if !c.prefs.groups.is_empty() {
            let defs = std::fs::read_to_string(groups).map(|t| parse_groups(&t)).unwrap_or_default();
            for s in c.prefs.sites(&defs) {
                if !c.blocked.contains(&s) {
                    c.blocked.push(s);
                }
            }
        }
        c
    }

    /// One file's settings over these. Lines it can't read are skipped.
    pub fn apply(&mut self, text: &str) {
        let t = ConfText::parse(text);
        t.over(&mut self.prefs);
        if let Some(b) = t.blocked {
            self.blocked = b;
        }
        self.default_mode = if self.prefs.address_bar { Mode::Full } else { Mode::Focus };
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
        let none = dir.join("no-groups.conf");
        assert_eq!(Conf::load(std::slice::from_ref(&system), &none).default_mode, Mode::Focus);
        std::fs::write(&user, "default = full   # mine\nbogus\nhome = x\n").unwrap();
        let both = Conf::load(&[system.clone(), user.clone(), dir.join("missing.conf")], &none);
        assert_eq!(both.default_mode, Mode::Full);
        assert_eq!(both.blocked, vec!["x.com"]);
        // A user file without the key leaves the wadspace's.
        std::fs::write(&user, "# nothing\n").unwrap();
        assert_eq!(Conf::load(&[system.clone(), user.clone()], &none).default_mode, Mode::Focus);
        // Groups on: their sites, from the groups file, with the file's own.
        let groups = dir.join("groups.conf");
        std::fs::write(&groups, "# Social media\nsocial-media = reddit.com tiktok.com\nnews = bbc.co.uk\n").unwrap();
        std::fs::write(&user, "groups = social-media gone\n").unwrap();
        let c = Conf::load(&[system, user], &groups);
        assert_eq!(c.blocked, vec!["x.com", "reddit.com", "tiktok.com"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn defaults_and_bad_values() {
        let c = Conf::load(&[], Path::new("/nonexistent"));
        assert_eq!(c.default_mode, Mode::Full);
        assert!(c.prefs.ad_blocker);
        let mut c = Conf::default();
        c.apply("default = kiosk\nsearch = altavista\nblocker = maybe");
        assert_eq!(c, Conf::default());
        c.apply("default=lite\nblocker = off\nsearch = wikipedia");
        assert_eq!(c.default_mode, Mode::Focus);
        assert!(!c.prefs.ad_blocker);
    }

    #[test]
    fn the_files_are_the_wadspaces_then_the_users() {
        let f = files(Path::new("/home/abc/.config/wadbrowser"));
        assert_eq!(f, [PathBuf::from(SYSTEM), PathBuf::from("/home/abc/.config/wadbrowser/wadbrowser.conf")]);
    }

    #[test]
    fn wad_models_file_reads() {
        let prefs = BrowserPrefs { address_bar: false, ad_blocker: false, ..Default::default() };
        let mut c = Conf::default();
        c.apply(&wad_model::browser::conf("A wadspace's", &prefs, &["tiktok.com".into()]));
        assert_eq!(c, Conf { default_mode: Mode::Focus, prefs, blocked: vec!["tiktok.com".into()] });
    }
}
