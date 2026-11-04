//! The media keys' work: the volume through `wpctl` (PipeWire's, on the
//! default output), in a thread of its own so a held key never stalls the
//! screen, and the backlight's steps (set through logind, `logind.rs`).

use std::path::Path;
use std::process::Command;
use std::sync::mpsc;

use crate::bar::Volume;

/// A volume key's step, of the whole.
const VOLUME_STEP: &str = "5%";
/// A brightness key's step, of the whole.
const BRIGHTNESS_STEP: f64 = 0.05;
/// The darkest the keys take it (the spike's brightness slider's least).
const BRIGHTNESS_FLOOR: f64 = 0.15;

const SINK: &str = "@DEFAULT_AUDIO_SINK@";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vol {
    Up,
    Down,
    Mute,
    /// Only look.
    Read,
}

/// The `wpctl` commands for `v`, before reading it back. Up unmutes (as
/// the spike's slider does); it stops at 100%.
pub fn commands(v: Vol) -> Vec<Vec<String>> {
    let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let up = format!("{VOLUME_STEP}+");
    let down = format!("{VOLUME_STEP}-");
    match v {
        Vol::Up => vec![s(&["set-mute", SINK, "0"]), s(&["set-volume", "-l", "1.0", SINK, &up])],
        Vol::Down => vec![s(&["set-volume", SINK, &down])],
        Vol::Mute => vec![s(&["set-mute", SINK, "toggle"])],
        Vol::Read => vec![],
    }
}

/// `wpctl get-volume`'s answer: "Volume: 0.45" or "Volume: 0.45 [MUTED]".
pub fn parse(out: &str) -> Option<Volume> {
    let rest = out.trim().strip_prefix("Volume:")?.trim();
    let mut words = rest.split_whitespace();
    let level = words.next()?.parse::<f64>().ok()?;
    let muted = words.any(|w| w == "[MUTED]");
    Some(Volume { level: level.clamp(0.0, 1.5), muted })
}

/// What `wadsctl volume` says.
pub fn describe(v: &Volume) -> String {
    format!("{}%{}", (v.level * 100.0).round() as i64, if v.muted { " muted" } else { "" })
}

fn wpctl(args: &[String]) -> Result<String, String> {
    let out = Command::new("wpctl").args(args).output().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "there's no wpctl (PipeWire) on this machine".to_string(),
        _ => format!("wpctl: {e}"),
    })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { format!("wpctl {} failed", args.join(" ")) } else { format!("wpctl: {err}") });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The thread that runs `wpctl`, a key at a time, in order.
pub struct Mixer {
    tx: mpsc::Sender<Vol>,
}

impl Mixer {
    /// Starts it; after each key, the volume as it is comes on the channel.
    pub fn start() -> (Self, async_channel::Receiver<Result<Volume, String>>) {
        let (tx, rx) = mpsc::channel::<Vol>();
        let (out, said) = async_channel::unbounded();
        let spawned = std::thread::Builder::new().name("mixer".into()).spawn(move || {
            while let Ok(v) = rx.recv() {
                let done = commands(v).iter().try_for_each(|c| wpctl(c).map(|_| ()));
                let now = done.and_then(|()| {
                    let o = wpctl(&["get-volume".into(), SINK.into()])?;
                    parse(&o).ok_or_else(|| format!("wpctl said {:?}", o.trim()))
                });
                if out.send_blocking(now).is_err() {
                    return;
                }
            }
        });
        if let Err(e) = spawned {
            tracing::error!("mixer: {e}");
        }
        (Self { tx }, said)
    }

    pub fn ask(&self, v: Vol) {
        let _ = self.tx.send(v);
    }
}

/// The screen's backlight (the first in /sys/class/backlight): its name and
/// where it is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlight {
    pub name: String,
    pub now: u32,
    pub max: u32,
}

pub fn backlight() -> Option<Backlight> {
    backlight_in(Path::new("/sys/class/backlight"))
}

fn backlight_in(dir: &Path) -> Option<Backlight> {
    let mut names: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.file_name()).collect();
    names.sort();
    let name = names.into_iter().next()?.to_string_lossy().into_owned();
    let read = |f: &str| std::fs::read_to_string(dir.join(&name).join(f)).ok()?.trim().parse::<u32>().ok();
    let (now, max) = (read("brightness")?, read("max_brightness")?);
    (max > 0).then_some(Backlight { name, now, max })
}

/// A brightness key's next value: a step up (to the top at most) or down
/// (never below the floor, nor up to it if something else went darker).
pub fn step(now: u32, max: u32, up: bool) -> u32 {
    let s = ((max as f64 * BRIGHTNESS_STEP).round() as u32).max(1);
    let floor = (max as f64 * BRIGHTNESS_FLOOR).round() as u32;
    if up { now.saturating_add(s).min(max) } else { now.saturating_sub(s).max(floor.min(now)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wpctl_s_answer() {
        assert_eq!(parse("Volume: 0.40\n"), Some(Volume { level: 0.4, muted: false }));
        assert_eq!(parse("Volume: 0.45 [MUTED]"), Some(Volume { level: 0.45, muted: true }));
        assert_eq!(parse("Volume: 1.00"), Some(Volume { level: 1.0, muted: false }));
        assert_eq!(parse("Translate ID error: '@DEFAULT_AUDIO_SINK@' is not a valid ID"), None);
        assert_eq!(parse(""), None);
        assert_eq!(describe(&Volume { level: 0.45, muted: true }), "45% muted");
        assert_eq!(describe(&Volume { level: 0.4, muted: false }), "40%");
    }

    #[test]
    fn the_keys_commands() {
        let line = |v| commands(v).iter().map(|c| c.join(" ")).collect::<Vec<_>>();
        assert_eq!(line(Vol::Up), ["set-mute @DEFAULT_AUDIO_SINK@ 0", "set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%+"]);
        assert_eq!(line(Vol::Down), ["set-volume @DEFAULT_AUDIO_SINK@ 5%-"]);
        assert_eq!(line(Vol::Mute), ["set-mute @DEFAULT_AUDIO_SINK@ toggle"]);
        assert!(commands(Vol::Read).is_empty());
    }

    #[test]
    fn brightness_steps() {
        // The Surface Book's panel: 7500 at the top, 375 a step, 1125 the floor.
        assert_eq!(step(1238, 7500, true), 1613);
        assert_eq!(step(7400, 7500, true), 7500);
        assert_eq!(step(7500, 7500, true), 7500);
        assert_eq!(step(1613, 7500, false), 1238);
        assert_eq!(step(1238, 7500, false), 1125, "not below the floor");
        assert_eq!(step(1125, 7500, false), 1125);
        assert_eq!(step(400, 7500, false), 400, "darker than the floor already: left there");
        assert_eq!(step(400, 7500, true), 775);
        // A backlight with few levels still moves.
        assert_eq!(step(3, 10, true), 4);
        assert_eq!(step(4, 10, false), 3);
    }

    #[test]
    fn finds_the_backlight() {
        let dir = tempdir();
        assert_eq!(backlight_in(&dir), None);
        for (name, now, max) in [("intel_backlight", "1238\n", "7500\n"), ("zz_other", "1", "2")] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            std::fs::write(dir.join(name).join("brightness"), now).unwrap();
            std::fs::write(dir.join(name).join("max_brightness"), max).unwrap();
        }
        assert_eq!(backlight_in(&dir), Some(Backlight { name: "intel_backlight".into(), now: 1238, max: 7500 }));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn tempdir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wados-backlight-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
