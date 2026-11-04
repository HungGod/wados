//! `wadbrowser.conf`: what links open in. The wadspace's first
//! (`/run/wadspaces-extra/wadbrowser.conf`, which wadd writes from its
//! design: wad-gen's `extras::wadbrowser_conf`), then the user's own
//! (`~/.config/wadbrowser/wadbrowser.conf`), which wins. `key = value`
//! lines; `#` starts a comment.
//!
//! ```text
//! default = focus     # what links open in (--default): full (an address bar) or focus
//! ```

use crate::modes::Mode;
use std::path::{Path, PathBuf};

/// The wadspace's, from wadd.
pub const SYSTEM: &str = "/run/wadspaces-extra/wadbrowser.conf";
pub const FILE: &str = "wadbrowser.conf";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conf {
    /// What `--default` opens (links, `BROWSER`, xdg-open).
    pub default_mode: Mode,
}

impl Default for Conf {
    fn default() -> Conf {
        Conf { default_mode: Mode::Full }
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
        for line in text.lines() {
            let line = line.split_once('#').map_or(line, |(l, _)| l).trim();
            let Some((key, value)) = line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) else { continue };
            if key == "default" {
                match value {
                    "full" => self.default_mode = Mode::Full,
                    "focus" | "lite" => self.default_mode = Mode::Focus,
                    _ => {}
                }
            }
        }
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
        std::fs::write(&system, "# the wadspace's\ndefault = focus\n").unwrap();
        assert_eq!(Conf::load(std::slice::from_ref(&system)).default_mode, Mode::Focus);
        std::fs::write(&user, "default = full   # mine\nbogus\nhome = x\n").unwrap();
        assert_eq!(Conf::load(&[system.clone(), user.clone(), dir.join("missing.conf")]).default_mode, Mode::Full);
        // A user file without the key leaves the wadspace's.
        std::fs::write(&user, "# nothing\n").unwrap();
        assert_eq!(Conf::load(&[system, user]).default_mode, Mode::Focus);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn defaults_and_bad_values() {
        assert_eq!(Conf::load(&[]).default_mode, Mode::Full);
        let mut c = Conf::default();
        c.apply("default = kiosk");
        assert_eq!(c.default_mode, Mode::Full);
        c.apply("default=lite");
        assert_eq!(c.default_mode, Mode::Focus);
    }

    #[test]
    fn the_files_are_the_wadspaces_then_the_users() {
        let f = files(Path::new("/home/abc/.config/wadbrowser"));
        assert_eq!(f, [PathBuf::from(SYSTEM), PathBuf::from("/home/abc/.config/wadbrowser/wadbrowser.conf")]);
    }

    #[test]
    fn wad_gens_file_reads() {
        // As crates/wad-gen/src/extras.rs writes it.
        let mut c = Conf::default();
        c.apply(
            "# WadBrowser's settings for this wadspace, from its design (a\n\
             # user's own go in ~/.config/wadbrowser/wadbrowser.conf).\n\
             #\n\
             # What links open in: full (tabs and an address bar) or focus (no address bar).\n\
             default = focus\n",
        );
        assert_eq!(c.default_mode, Mode::Focus);
    }
}
