//! The media keys' work, and the tray's sound flyout's: the volume through
//! `wpctl` (PipeWire's, on the default output; wad-desk's sound module), in
//! a thread of its own so a held key or a dragged slider never stalls the
//! screen, and the backlight's steps (set through logind, `logind.rs`).

use std::path::Path;
use std::sync::mpsc;

use crate::bar::Volume;

/// A volume key's step, of the whole.
const VOLUME_STEP: &str = "5%";
/// A brightness key's step, of the whole.
const BRIGHTNESS_STEP: f64 = 0.05;
/// The darkest the keys take it (the spike's brightness slider's least).
const BRIGHTNESS_FLOOR: f64 = 0.15;

const SINK: &str = wad_desk::sound::SINK;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Vol {
    Up,
    Down,
    Mute,
    /// Only look.
    Read,
    /// To this (0 to 1), unmuted (the flyout's slider).
    Set(f64),
    /// Muted, or not (the flyout's switch).
    Muted(bool),
    /// This output (wpctl's id) the default.
    Output(u32),
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
        Vol::Set(v) => {
            vec![
                s(&["set-volume", "-l", "1.0", SINK, &format!("{:.2}", v.clamp(0.0, 1.0))]),
                s(&["set-mute", SINK, "0"]),
            ]
        }
        Vol::Muted(m) => vec![s(&["set-mute", SINK, if m { "1" } else { "0" }])],
        Vol::Output(id) => vec![s(&["set-default", &id.to_string()])],
    }
}

/// What to run of `asked` (in order): a run of slider moves is its last.
pub fn coalesce(asked: Vec<Vol>) -> Vec<Vol> {
    let mut out: Vec<Vol> = vec![];
    for v in asked {
        if let (Vol::Set(_), Some(Vol::Set(_))) = (v, out.last()) {
            out.pop();
        }
        out.push(v);
    }
    out
}

/// `wpctl get-volume`'s answer: "Volume: 0.45" or "Volume: 0.45 [MUTED]".
pub fn parse(out: &str) -> Option<Volume> {
    wad_desk::sound::parse_volume(out).map(|(level, muted)| Volume { level, muted })
}

/// What `wadsctl volume` says.
pub fn describe(v: &Volume) -> String {
    format!("{}%{}", (v.level * 100.0).round() as i64, if v.muted { " muted" } else { "" })
}

use wad_desk::sound::wpctl;

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
            while let Ok(first) = rx.recv() {
                let asked = coalesce(std::iter::once(first).chain(rx.try_iter()).collect());
                let done = asked.into_iter().flat_map(commands).try_for_each(|c| wpctl(&c).map(|_| ()));
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
        assert_eq!(
            line(Vol::Set(0.456)),
            ["set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 0.46", "set-mute @DEFAULT_AUDIO_SINK@ 0"]
        );
        assert_eq!(line(Vol::Muted(true)), ["set-mute @DEFAULT_AUDIO_SINK@ 1"]);
        assert_eq!(line(Vol::Output(48)), ["set-default 48"]);
        assert_eq!(
            coalesce(vec![Vol::Set(0.1), Vol::Set(0.2), Vol::Muted(true), Vol::Set(0.3), Vol::Set(0.4)]),
            [Vol::Set(0.2), Vol::Muted(true), Vol::Set(0.4)]
        );
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
