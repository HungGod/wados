//! The session's settings: `$XDG_CONFIG_HOME/wados/settings.toml`.
//! Settings writes it (whole, atomically: a new file renamed over the
//! old), the shell reads it at its start and again each time it changes
//! (it watches the folder). What isn't in it is its default, so an empty
//! file, or none, is the machine as it ships.
//!
//! What's here is what the shell (or labwc, through `labwc.rs`) acts on:
//! night light, the idle times, the lid, locking, the desktop's background,
//! the mouse and keyboard, the clock's format. The displays' arrangement
//! is `layout.rs`'s (`displays.toml`): it's kept per set of displays.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub display: Display,
    pub power: Power,
    pub lock: Lock,
    pub background: Background,
    pub mouse: Mouse,
    pub keyboard: Keyboard,
    pub clock: Clock,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    pub night_light: bool,
    /// The Color Temperature slider: 0 cooler, 1 warmer.
    pub night_temp: f64,
}

impl Default for Display {
    fn default() -> Self {
        Self { night_light: false, night_temp: 0.5 }
    }
}

/// What closing the lid does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lid {
    /// logind's own: it suspends (and the shell locks first).
    #[default]
    Suspend,
    /// The shell holds logind's lid switch and locks.
    Lock,
    /// The shell holds logind's lid switch and does nothing.
    Nothing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Power {
    /// The screen goes off after this long idle (minutes; 0 never).
    pub blank_min: u32,
    /// The machine suspends after this long idle (minutes; 0 never).
    pub suspend_min: u32,
    pub lid: Lid,
}

impl Default for Power {
    fn default() -> Self {
        // Never suspended by itself: a WadSpaces machine streams, and a
        // stream isn't the keyboard's or the mouse's activity.
        Self { blank_min: 5, suspend_min: 0, lid: Lid::Suspend }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Lock {
    /// The screen locks after this long idle (minutes; 0 never).
    pub auto_lock_min: u32,
    /// Lock before the machine sleeps.
    pub on_suspend: bool,
    /// Notifications show on the lock screen.
    pub notifications: bool,
}

impl Default for Lock {
    fn default() -> Self {
        Self { auto_lock_min: 5, on_suspend: true, notifications: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Background {
    /// One of the art pictures' ids (`wad_model::wallpapers::PICTURES`).
    pub picture: String,
}

impl Default for Background {
    fn default() -> Self {
        Self { picture: "duo".into() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Hand {
    #[default]
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mouse {
    /// The primary button.
    pub primary: Hand,
    /// A mouse's pointer speed, 0 slow to 1 fast (libinput's -1..1).
    pub speed: f64,
    pub touchpad_speed: f64,
    /// Natural scrolling (mice and touchpads both, as the spike has it).
    pub natural: bool,
    pub tap_to_click: bool,
}

impl Default for Mouse {
    fn default() -> Self {
        // libinput's and labwc's own: what the machine does with no file.
        Self { primary: Hand::Left, speed: 0.5, touchpad_speed: 0.5, natural: false, tap_to_click: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keyboard {
    /// An XKB layout and variant: "us", "gb", "us(dvorak)".
    pub layout: String,
    /// Milliseconds before a held key repeats.
    pub repeat_delay: u32,
    /// Repeats a second.
    pub repeat_rate: u32,
}

impl Default for Keyboard {
    fn default() -> Self {
        // labwc's own.
        Self { layout: "us".into(), repeat_delay: 600, repeat_rate: 25 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Clock {
    pub h24: bool,
}

/// `$XDG_CONFIG_HOME/wados` (`~/.config/wados`).
pub fn dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
        .join("wados")
}

/// The settings file.
pub fn path() -> PathBuf {
    dir().join("settings.toml")
}

impl Settings {
    /// The settings file's (its defaults where it's missing or can't be read).
    pub fn load() -> Self {
        Self::load_from(&path())
    }

    pub fn load_from(p: &Path) -> Self {
        match std::fs::read_to_string(p) {
            Ok(s) => Self::parse(&s).unwrap_or_else(|e| {
                tracing::warn!("{}: {e} (the defaults instead)", p.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        toml::from_str(s).map_err(|e| e.to_string())
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&path())
    }

    pub fn save_to(&self, p: &Path) -> Result<(), String> {
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        write_atomic(p, &text)
    }

    /// Changes it with `f` and saves it, from what's in the file now (so two
    /// windows changing different things don't undo each other).
    pub fn update(f: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
        let mut s = Self::load();
        f(&mut s);
        s.save()?;
        Ok(s)
    }
}

/// Writes `text` to `p` by way of a file beside it, renamed over it: a
/// reader sees the old file or the new, never half of one.
pub fn write_atomic(p: &Path, text: &str) -> Result<(), String> {
    let dir = p.parent().ok_or_else(|| format!("{} has no folder", p.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, p).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", p.display())
    })
}

/// "1 minute", "15 minutes", "1 hour", "Never" (0) (the spike's `minutesLabel`).
pub fn minutes_label(n: u32) -> String {
    match n {
        0 => "Never".into(),
        60 => "1 hour".into(),
        n if n > 60 && n % 60 == 0 => format!("{} hours", n / 60),
        1 => "1 minute".into(),
        n => format!("{n} minutes"),
    }
}

/// A drop-down's minutes, with Never last (the spike's `minuteOptions`).
pub fn minute_options(ns: &[u32]) -> Vec<(String, u32)> {
    ns.iter().filter(|n| **n > 0).map(|n| (minutes_label(*n), *n)).chain([("Never".to_string(), 0)]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_is_the_default() {
        assert_eq!(Settings::parse("").unwrap(), Settings::default());
        let s = Settings::parse("[lock]\nauto_lock_min = 10\n[mouse]\nprimary = \"right\"\n").unwrap();
        assert_eq!(s.lock.auto_lock_min, 10);
        assert!(s.lock.on_suspend, "what isn't said is its default");
        assert_eq!(s.mouse.primary, Hand::Right);
        assert_eq!(s.keyboard, Keyboard::default());
        assert!(Settings::parse("[lock]\nauto_lock_min = \"soon\"").is_err());
    }

    #[test]
    fn it_round_trips_through_its_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("wados").join("settings.toml");
        assert_eq!(Settings::load_from(&p), Settings::default());
        let mut s = Settings::default();
        s.power.lid = Lid::Lock;
        s.display.night_light = true;
        s.background.picture = "silk".into();
        s.save_to(&p).unwrap();
        assert_eq!(Settings::load_from(&p), s);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("lid = \"lock\""), "{text}");
        // Nothing left beside it.
        assert_eq!(std::fs::read_dir(p.parent().unwrap()).unwrap().count(), 1);
        std::fs::write(&p, "not = [toml").unwrap();
        assert_eq!(Settings::load_from(&p), Settings::default());
    }

    #[test]
    fn minutes_in_words() {
        assert_eq!(minutes_label(0), "Never");
        assert_eq!(minutes_label(1), "1 minute");
        assert_eq!(minutes_label(15), "15 minutes");
        assert_eq!(minutes_label(60), "1 hour");
        assert_eq!(minutes_label(120), "2 hours");
        assert_eq!(minutes_label(90), "90 minutes");
        let o = minute_options(&[1, 5, 10, 30]);
        assert_eq!(o.iter().map(|(_, n)| *n).collect::<Vec<_>>(), [1, 5, 10, 30, 0]);
        assert_eq!(o[4].0, "Never");
    }
}
