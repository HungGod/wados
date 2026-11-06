//! Sound through PipeWire's command-line tools (the person's own sound:
//! no root, no D-Bus): `wpctl` for the outputs and inputs (sinks and
//! sources), their volume, mute and which is the default; `pw-play` for
//! the speaker test (a chime left, then right, then both, written as a WAV);
//! `pw-record` for the input level, read from the default input as it comes.
//!
//! The shell's volume keys use [`wpctl`] and [`parse_volume`] too.

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const SINK: &str = "@DEFAULT_AUDIO_SINK@";
pub const SOURCE: &str = "@DEFAULT_AUDIO_SOURCE@";

/// An output (sink) or input (source).
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    /// wpctl's id for it.
    pub id: u32,
    pub name: String,
    pub default: bool,
    pub volume: f64,
    pub muted: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    pub outputs: Vec<Device>,
    pub inputs: Vec<Device>,
}

impl Status {
    pub fn output(&self) -> Option<&Device> {
        self.outputs.iter().find(|d| d.default).or(self.outputs.first())
    }

    pub fn input(&self) -> Option<&Device> {
        self.inputs.iter().find(|d| d.default).or(self.inputs.first())
    }
}

/// `wpctl status`'s Audio sinks and sources.
pub fn parse_status(s: &str) -> Status {
    #[derive(PartialEq)]
    enum In {
        Other,
        Sinks,
        Sources,
    }
    let mut st = Status::default();
    let mut audio = false;
    let mut at = In::Other;
    for line in s.lines() {
        // A section ("Audio", "Video", "Settings") starts at the line's start.
        if !line.starts_with(' ') && !line.trim().is_empty() {
            audio = line.trim() == "Audio";
            at = In::Other;
            continue;
        }
        if !audio {
            continue;
        }
        let t = line.trim_start_matches([' ', '│', '├', '└', '─']).trim();
        if t.ends_with(':') {
            at = match t {
                "Sinks:" => In::Sinks,
                "Sources:" => In::Sources,
                _ => In::Other,
            };
            continue;
        }
        if at == In::Other || t.is_empty() {
            continue;
        }
        let (default, t) = match t.strip_prefix('*') {
            Some(r) => (true, r.trim_start()),
            None => (false, t),
        };
        let Some((id, rest)) = t.split_once('.') else { continue };
        let Ok(id) = id.trim().parse::<u32>() else { continue };
        let (name, vol) = match rest.rfind('[') {
            Some(i) if rest.trim_end().ends_with(']') => {
                (rest[..i].trim(), rest[i + 1..].trim_end().trim_end_matches(']'))
            }
            _ => (rest.trim(), ""),
        };
        let mut words = vol.split_whitespace();
        let (volume, muted) = match words.next() {
            Some("vol:") => (words.next().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0), vol.contains("MUTED")),
            _ => (0.0, false),
        };
        let d = Device { id, name: name.to_string(), default, volume, muted };
        if at == In::Sinks { st.outputs.push(d) } else { st.inputs.push(d) }
    }
    st
}

/// `wpctl get-volume`'s answer: "Volume: 0.45" or "Volume: 0.45 [MUTED]" (level, muted).
pub fn parse_volume(out: &str) -> Option<(f64, bool)> {
    let rest = out.trim().strip_prefix("Volume:")?.trim();
    let mut words = rest.split_whitespace();
    let level = words.next()?.parse::<f64>().ok()?;
    let muted = words.any(|w| w == "[MUTED]");
    Some((level.clamp(0.0, 1.5), muted))
}

/// Runs `wpctl args…`: what it printed, or why it failed in words.
pub fn wpctl(args: &[String]) -> Result<String, String> {
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

fn s(a: &[&str]) -> Vec<String> {
    a.iter().map(|x| x.to_string()).collect()
}

/// The sound as it is.
pub fn status() -> Result<Status, String> {
    let out = wpctl(&s(&["status"]))?;
    Ok(parse_status(&out))
}

pub fn set_default(id: u32) -> Result<(), String> {
    wpctl(&s(&["set-default", &id.to_string()])).map(|_| ())
}

/// `id`'s volume (0 to 1), unmuting it.
pub fn set_volume(id: &str, v: f64) -> Result<(), String> {
    wpctl(&s(&["set-volume", "-l", "1.0", id, &format!("{:.2}", v.clamp(0.0, 1.0))]))?;
    set_mute(id, false)
}

pub fn set_mute(id: &str, muted: bool) -> Result<(), String> {
    wpctl(&s(&["set-mute", id, if muted { "1" } else { "0" }])).map(|_| ())
}

/// An output's glyph (the spike's `outputIcon`).
pub fn output_icon(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if ["xm5", "headphones", "headset", "buds"].iter().any(|w| n.contains(w)) {
        "headphones"
    } else if ["hdmi", "displayport"].iter().any(|w| n.contains(w)) {
        "monitor"
    } else {
        "speaker"
    }
}

// ------------------------------------------------------------ the speaker test

/// The test's rate.
const RATE: u32 = 48000;

/// Which speaker the test sounds at `t` seconds in: left, right, both, then done.
pub fn test_channel(t: f64) -> Option<Channel> {
    match t {
        t if t < 0.0 => None,
        t if t < 1.0 => Some(Channel::Left),
        t if t < 2.0 => Some(Channel::Right),
        t if t < 3.0 => Some(Channel::Both),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Left,
    Right,
    Both,
}

/// The test's sound: a soft two-note chime in each second, left, right, both (16-bit stereo WAV).
pub fn test_wav() -> Vec<u8> {
    let n = RATE as usize * 3;
    let mut pcm = Vec::with_capacity(n * 4);
    for i in 0..n {
        let t = i as f64 / RATE as f64;
        let within = t.fract();
        // Two notes (E5, then A5), each fading out.
        let (f, start) = if within < 0.5 { (659.25, 0.0) } else { (880.0, 0.5) };
        let since = within - start;
        let env = (-since * 7.0).exp() * (since * 400.0).min(1.0);
        let v = (2.0 * std::f64::consts::PI * f * t).sin() * env * 0.35;
        let sample = (v * i16::MAX as f64) as i16;
        let (l, r) = match test_channel(t) {
            Some(Channel::Left) => (sample, 0),
            Some(Channel::Right) => (0, sample),
            _ => (sample, sample),
        };
        pcm.extend_from_slice(&l.to_le_bytes());
        pcm.extend_from_slice(&r.to_le_bytes());
    }
    wav(&pcm, 2, RATE)
}

fn wav(pcm: &[u8], channels: u16, rate: u32) -> Vec<u8> {
    let mut w = Vec::with_capacity(44 + pcm.len());
    let block = channels * 2;
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&channels.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * block as u32).to_le_bytes());
    w.extend_from_slice(&block.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    w.extend_from_slice(pcm);
    w
}

/// Plays the test on the default output (`pw-play`), writing it to `file` first.
pub fn play_test(file: &Path) -> Result<Child, String> {
    std::fs::write(file, test_wav()).map_err(|e| format!("{}: {e}", file.display()))?;
    Command::new("pw-play")
        .args(["--media-role", "Notification"])
        .arg(file)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "there's no pw-play (pipewire-utils) on this machine".to_string(),
            _ => format!("pw-play: {e}"),
        })
}

// ------------------------------------------------------------ the input level

/// The loudest of `samples` (16-bit, little-endian), 0 to 1, on a scale
/// that reads as a meter does (a talking voice well up it).
pub fn level_of(samples: &[u8]) -> f64 {
    let peak = samples.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c).unsigned_abs()).max().unwrap_or(0);
    if peak == 0 {
        return 0.0;
    }
    // -60 dBFS at the bottom, 0 at the top.
    let db = 20.0 * (peak as f64 / i16::MAX as f64).log10();
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

/// The default input's level, as it comes (`pw-record`, 16 kHz mono): `on`
/// gets it about 20 times a second, in a thread, until the [`Listening`]'s dropped.
pub struct Listening {
    child: Child,
    stop: Arc<AtomicBool>,
}

impl Drop for Listening {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn listen(on: impl Fn(f64) + Send + 'static) -> Result<Listening, String> {
    let mut child = Command::new("pw-record")
        .args(["--rate", "16000", "--channels", "1", "--format", "s16", "--raw", "--media-role", "Communication", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "there's no pw-record (pipewire-utils) on this machine".to_string(),
            _ => format!("pw-record: {e}"),
        })?;
    let mut out = child.stdout.take().ok_or("pw-record gave no output")?;
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    std::thread::Builder::new()
        .name("input-level".into())
        .spawn(move || {
            // 50 ms at 16 kHz, 2 bytes a sample.
            let mut buf = vec![0u8; 1600];
            while !s.load(Ordering::Relaxed) {
                match out.read_exact(&mut buf) {
                    Ok(()) => on(level_of(&buf)),
                    Err(_) => return,
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Listening { child, stop })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "PipeWire 'pipewire-0' [1.6.11, sam@wados, cookie:1]
 └─ Clients:
        33. WirePlumber                         [1.6.11, sam@wados, pid:1618]

Audio
 ├─ Devices:
 │      44. Built-in Audio                      [alsa]
 │
 ├─ Sinks:
 │      47. HDMI / DisplayPort 1 Output          [vol: 1.00]
 │  *   48. Built-in Audio Analog Stereo        [vol: 0.40]
 │
 ├─ Sources:
 │  *   49. Built-in Audio Analog Stereo        [vol: 0.00 MUTED]
 │
 ├─ Filters:
 │
 └─ Streams:
        90. Firefox
             91. output_FL       > Built-in Audio:playback_FL	[active]

Video
 ├─ Devices:
 │      53. ipu3-imgu                           [v4l2]
 ├─ Sources:
 │  *   78. Built-in Front Camera
";

    #[test]
    fn wpctl_status_s_outputs_and_inputs() {
        let s = parse_status(STATUS);
        assert_eq!(s.outputs.len(), 2);
        assert_eq!(
            s.outputs[1],
            Device { id: 48, name: "Built-in Audio Analog Stereo".into(), default: true, volume: 0.4, muted: false }
        );
        assert_eq!(s.outputs[0].name, "HDMI / DisplayPort 1 Output");
        assert!(!s.outputs[0].default);
        assert_eq!(s.output().unwrap().id, 48);
        assert_eq!(
            s.inputs,
            [Device { id: 49, name: "Built-in Audio Analog Stereo".into(), default: true, volume: 0.0, muted: true }]
        );
        assert_eq!(parse_status(""), Status::default());
        assert!(parse_status("").output().is_none());
    }

    #[test]
    fn wpctl_s_volume() {
        assert_eq!(parse_volume("Volume: 0.40\n"), Some((0.4, false)));
        assert_eq!(parse_volume("Volume: 0.45 [MUTED]"), Some((0.45, true)));
        assert_eq!(parse_volume("nope"), None);
    }

    #[test]
    fn output_glyphs() {
        assert_eq!(output_icon("WH-1000XM5"), "headphones");
        assert_eq!(output_icon("HDMI / DisplayPort 1 Output"), "monitor");
        assert_eq!(output_icon("Built-in Audio Analog Stereo"), "speaker");
    }

    #[test]
    fn the_speaker_test() {
        assert_eq!(test_channel(0.2), Some(Channel::Left));
        assert_eq!(test_channel(1.5), Some(Channel::Right));
        assert_eq!(test_channel(2.9), Some(Channel::Both));
        assert_eq!(test_channel(3.0), None);
        let w = test_wav();
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(w.len(), 44 + 48000 * 3 * 4);
        // The first second's right channel is silent, its left isn't.
        let frame = |i: usize| {
            let o = 44 + i * 4;
            (i16::from_le_bytes([w[o], w[o + 1]]), i16::from_le_bytes([w[o + 2], w[o + 3]]))
        };
        assert!((100..2000).any(|i| frame(i).0 != 0));
        assert!((100..2000).all(|i| frame(i).1 == 0));
        assert!((48100..50000).all(|i| frame(i).0 == 0));
    }

    #[test]
    fn the_input_level() {
        assert_eq!(level_of(&[]), 0.0);
        assert_eq!(level_of(&[0, 0, 0, 0]), 0.0);
        assert_eq!(level_of(&i16::MAX.to_le_bytes()), 1.0);
        let quiet = level_of(&33i16.to_le_bytes());
        assert!(quiet > 0.0 && quiet < 0.1, "{quiet}");
        let half = level_of(&(-16384i16).to_le_bytes());
        assert!(half > 0.85 && half < 0.95, "{half}");
    }
}
