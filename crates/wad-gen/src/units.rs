//! A wadspace's quadlet units, which wadd writes into its units folder:
//! `wad-<id>.container` on the screen, and while it's streamed that, its
//! stream sidecar's (`wad-<id>-display.container`) and the display volume
//! they share (`wad-<id>-display.volume`).
//!
//! On the screen it reaches the machine's labwc only through a folder wadd
//! keeps for it, `<screen_dir>/<id>`, mounted at `/run/wadspaces-display`:
//! a Wayland socket labwc serves it alone (`wayland-1`, a security context
//! whose app id is `io.wadspaces.<id>`: [`sandbox_app_id`]), and
//! PulseAudio's folder (`pulse/`). Never the session's runtime dir.
//!
//! Its projects aren't in its image: a GitHub project is cloned and a copy
//! copied in by the wadspace (from wadd's copy, read-only); a mount is the
//! machine's own folder, read-write and never relabelled, so a wadspace
//! with one runs without SELinux's separation. Its keys are mounted from
//! their folders, shared with the other wadspaces that use them. Nothing
//! in a unit is a secret: podman secrets are named, never written.

use std::collections::BTreeMap;

use wad_model::{ProjectKind, Wadspace, Wallpaper};

use crate::extras::{self, SIGNINS_ROOT};
use crate::{Mounts, labels, mountable};

/// Where a wadspace sees the copies it takes in: `<COPIES_ROOT>/<project id>`.
pub const COPIES_ROOT: &str = "/run/wadspaces-copies";
/// Where a wadspace sees its display: its screen folder, or its stream's volume.
pub const DISPLAY_DIR: &str = "/run/wadspaces-display";
/// The Wayland socket in its screen folder (the image's desktop looks for `wayland-*`).
pub const SOCKET: &str = "wayland-1";
/// PulseAudio's folder in its screen folder (PULSE_SERVER: …/pulse/native).
pub const PULSE: &str = "pulse";
/// The security context's engine labwc's window rules match (`sandboxEngine`).
pub const SANDBOX_ENGINE: &str = "io.wadspaces";
/// The stream sidecar's port (plain HTTP: wadd's proxy in front of it ends TLS).
pub const STREAM_PORT: u16 = 3000;
/// Where a machine's wadspaces' ids start (all but abc's 1000): 65536 of
/// them, the `wadspaces` range in the host's subuid and subgid.
pub const USERNS_BASE: u32 = 1_000_000;
const SUBORDINATE_IDS: u32 = 65536;

/// A wadspace's security-context app id: how labwc reports its windows
/// (the shell's wadspace X is a window whose app id is exactly this).
pub fn sandbox_app_id(id: &str) -> String {
    format!("{SANDBOX_ENGINE}.{id}")
}

/// Its container's name.
pub fn container_name(id: &str) -> String {
    format!("wad-{id}")
}

/// The service quadlet makes of its unit.
pub fn service(id: &str) -> String {
    format!("wad-{id}.service")
}

/// Its stream sidecar's container (and the display volume's) name.
pub fn display_name(id: &str) -> String {
    // Not `wad-<id>-display`: that's the container of a wadspace whose id is
    // `<id>-display`. A wadspace's is always `wad-` (a '-' fourth), so
    // `wadstream-` can never be one.
    format!("wadstream-{id}")
}

/// What a wadspace's units get from the machine they're on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Machine {
    /// wadd's state folder: extra folders (`extra/<id>`) and copies (`copies/<project id>`).
    pub state_dir: String,
    /// Keys' folders: `<signins_dir>/<key id>`.
    pub signins_dir: String,
    /// Its time zone (empty: UTC).
    pub tz: String,
    /// Podman secrets every wadspace gets, by name (github_token, when the machine has one).
    pub secrets: Vec<String>,
    /// Rootless podman: the uid inside that the host user is (1000, abc).
    pub rootless_uid: Option<u32>,
    /// Rootful podman: the first of the machine's ids for wadspaces ([`USERNS_BASE`]).
    pub userns_base: Option<u32>,
    /// Devices every wadspace gets (`/dev/dri`, when there is one).
    pub devices: Vec<String>,
    /// Device cgroup rules for devices wadd hands over while one runs on the screen.
    pub device_rules: Vec<String>,
    /// Where the screen folders are (`<screen_dir>/<id>`); none: no screen.
    pub screen_dir: Option<String>,
    /// PulseAudio's folder, mounted from the start where wadd can't bind it in (rootless).
    pub pulse_dir: Option<String>,
    /// The machine's screens: its compositor draws one window per screen.
    pub screens: u32,
    /// Draw in software (NVIDIA's own driver: the images haven't its libraries).
    pub software_render: bool,
}

/// What one run of it gets.
#[derive(Debug, Clone, Copy)]
pub struct Run<'a> {
    /// The image it runs (its set's, [`crate::sets::Set::image`], or the one it built last).
    pub image: &'a str,
    /// Its mount projects found on this machine.
    pub mounts: &'a Mounts,
    /// A wallpaper of its own for this run (opened from an app or a variant with one), over its look's.
    pub wallpaper: Option<&'a Wallpaper>,
}

/// A streamed wadspace's sidecar.
#[derive(Debug, Clone, Copy)]
pub struct Sidecar<'a> {
    /// The stream image (`localhost/wados-stream:44`).
    pub image: &'a str,
    /// The stream's id.
    pub guid: &'a str,
    /// Its web port, published on 127.0.0.1 only: wadd's proxy is the way in.
    pub web_port: u16,
    /// A public stream's WebRTC mux port (UDP and TCP); none: WebSockets only.
    pub mux_port: Option<u16>,
}

/// The SELinux categories a wadspace's containers share: its own and its
/// stream sidecar's, which reads the buffers its desktop draws in. Two of
/// 1024, from its id: wadspaces stay apart from each other as with
/// podman's own.
pub fn mcs_level(id: &str) -> String {
    // FNV-1a.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    let a = (h % 1024) as u32;
    let mut b = ((h >> 20) % 1024) as u32;
    if b == a {
        b = (b + 1) % 1024;
    }
    format!("s0:c{},c{}", a.min(b), a.max(b))
}

/// Rootless podman: the container's root is you and its uid 1000 a
/// subordinate id, so abc couldn't write the folders you own (a key's).
/// These put you at `uid` inside and keep the rest of the usual 65536 ids.
fn rootless_maps(uid: u32) -> Vec<String> {
    let rest = SUBORDINATE_IDS.saturating_sub(uid + 1);
    let ranges = [format!("0:1:{uid}"), format!("{uid}:0:1"), format!("{}:{}:{rest}", uid + 1, uid + 1)];
    ranges.iter().map(|r| format!("UIDMap={r}")).chain(ranges.iter().map(|r| format!("GIDMap={r}"))).collect()
}

/// A machine's rootful podman: a user namespace too, so root inside isn't
/// root on the machine. Every id but abc's is one of the machine's range;
/// abc (1000) stays the machine's user, who owns its screen sockets, keys
/// and the devices handed to it.
fn rootful_maps(base: u32) -> Vec<String> {
    let ranges =
        [format!("0:{base}:1000"), "1000:1000:1".into(), format!("1001:{}:{}", base + 1001, SUBORDINATE_IDS - 1001)];
    ranges.iter().map(|r| format!("UIDMap={r}")).chain(ranges.iter().map(|r| format!("GIDMap={r}"))).collect()
}

fn maps(m: &Machine) -> Vec<String> {
    match (m.rootless_uid, m.userns_base) {
        (Some(uid), _) => rootless_maps(uid),
        (None, Some(base)) => rootful_maps(base),
        (None, None) => vec![],
    }
}

/// Text from a document on one line, `%` as systemd reads it back (a
/// specifier otherwise): no value adds lines of its own to a unit.
fn esc(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().replace('%', "%%")
}

/// An `Environment=` line systemd reads back as exactly `k=v`: quoted when
/// the value has spaces, quotes or backslashes.
fn env_line(k: &str, v: &str) -> String {
    let v = esc(v);
    if v.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\')) {
        format!("Environment=\"{k}={}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        format!("Environment={k}={v}")
    }
}

fn tz(m: &Machine) -> &str {
    if m.tz.is_empty() { "Etc/UTC" } else { &m.tz }
}

/// Its units: on the screen, its own; streamed (`stream`), its own drawing
/// on its sidecar's display, the sidecar, and their display volume.
/// (file name, text) each.
pub fn units(ws: &Wadspace, run: &Run, m: &Machine, stream: Option<&Sidecar>) -> Vec<(String, String)> {
    let file = format!("{}.container", container_name(&ws.id));
    let Some(s) = stream else { return vec![(file, wadspace_unit(ws, run, m, None))] };
    let display = display_name(&ws.id);
    vec![
        (file, wadspace_unit(ws, run, m, Some(&display))),
        (format!("{display}.container"), sidecar_unit(ws, m, s)),
        (format!("{display}.volume"), display_volume(&display)),
    ]
}

/// The wadspace's unit. `display`: its sidecar's name, while it's streamed.
fn wadspace_unit(ws: &Wadspace, run: &Run, m: &Machine, display: Option<&str>) -> String {
    let id = &ws.id;
    let mut lines: Vec<String> = vec![
        format!("# Generated by wadd (wad-gen) for the wadspace {id}. Do not edit."),
        "[Unit]".into(),
        format!("Description=WadSpaces wadspace: {}", esc(&ws.name)),
        "Wants=network-online.target".into(),
        "After=network-online.target".into(),
    ];
    // Streamed: its sidecar starts first, and when it stops, so does this.
    if let Some(d) = display {
        lines.push(format!("BindsTo={d}.service"));
        lines.push(format!("After={d}.service"));
    }
    lines.extend([
        String::new(),
        "[Container]".into(),
        format!("Image={}", run.image),
        format!("ContainerName={}", container_name(id)),
        format!("Label={}={id}", labels::WADSPACE),
    ]);
    lines.extend(maps(m));
    let screen = m.screen_dir.as_deref().filter(|_| display.is_none());

    let mut env: BTreeMap<&str, String> = BTreeMap::new();
    env.insert("PUID", "1000".into());
    env.insert("PGID", "1000".into());
    env.insert("TZ", tz(m).into());
    env.insert("TITLE", ws.name.clone());
    match run.wallpaper.unwrap_or(&ws.look.wallpaper) {
        wp @ Wallpaper::Image { .. } => {
            if let Some(f) = extras::wallpaper_file(wp) {
                env.insert("WALLPAPER", format!("{}/{f}", extras::DIR));
                // Filling the screen, as the Builder shows it.
                env.insert("WALLPAPER_MODE", "fill".into());
            }
        }
        Wallpaper::Color { value } => {
            env.insert("WALLPAPER_COLOR", value.clone());
        }
        Wallpaper::Empty => {}
    }
    if ws.look.title_bars == Some(true) {
        env.insert("WADSPACES_TITLEBARS", "1".into());
    }
    if m.software_render {
        env.insert("WLR_RENDERER", "pixman".into());
    }
    if screen.is_some() && m.screens > 1 {
        env.insert("WLR_WL_OUTPUTS", m.screens.to_string());
    }
    // Sound: the session's PipeWire through its PulseAudio socket (or the sidecar's).
    if screen.is_some() || display.is_some() {
        env.insert("PULSE_SERVER", format!("unix:{DISPLAY_DIR}/{PULSE}/native"));
    }
    lines.extend(env.iter().map(|(k, v)| env_line(k, v)));
    lines.extend(m.secrets.iter().map(|s| format!("Secret={}", esc(s))));

    // Its home: under a machine's maps, it keeps the owners its files have on disk.
    let idmap = if m.rootless_uid.is_none() && m.userns_base.is_some() { ",idmap" } else { "" };
    lines.push(format!("Volume=wad-{id}-config:/config:z{idmap}"));
    let mut mounted = false;
    for p in &ws.look.projects {
        match &p.kind {
            ProjectKind::Mount { .. } => {
                if let Some(path) = run.mounts.get(&p.id).filter(|path| mountable(path, &p.as_name)) {
                    lines.push(format!("Volume={}:/config/Desktop/{}:rw", esc(path), esc(&p.as_name)));
                    mounted = true;
                }
            }
            ProjectKind::Github { .. } | ProjectKind::Copy { .. } => {
                lines.push(format!("Volume={}/copies/{}:{COPIES_ROOT}/{}:ro,z", esc(&m.state_dir), p.id, p.id));
            }
        }
    }
    // Keys: each its folder, read-write, shared with the other wadspaces that use it (:z, not :Z).
    for s in extras::signins(ws) {
        lines.push(format!("Volume={}/{}:{SIGNINS_ROOT}/{}:rw,z", esc(&m.signins_dir), s.key.id, s.key.id));
    }
    lines.push(format!("Volume={}:{}:ro,z", esc(&extras::dir(&m.state_dir, id)), extras::DIR));
    match (display, screen) {
        (Some(d), _) => lines.push(format!("Volume={d}.volume:{DISPLAY_DIR}:z")),
        (None, Some(dir)) => {
            lines.push(format!("Volume={}/{id}:{DISPLAY_DIR}:ro,rslave", esc(dir)));
            if let Some(p) = &m.pulse_dir {
                lines.push(format!("Volume={}:{DISPLAY_DIR}/{PULSE}:ro", esc(p)));
            }
        }
        (None, None) => {}
    }
    // The user's own folders are never relabelled; else categories of its own.
    if mounted {
        lines.push("SecurityLabelDisable=true".into());
    } else {
        lines.push(format!("SecurityLabelLevel={}", mcs_level(id)));
    }
    lines.extend(m.devices.iter().map(|d| format!("AddDevice={}", esc(d))));
    // Devices wadd hands over while it runs (USB, gamepads, webcams): their
    // nodes may be opened, not made. They stay on this machine: not streamed.
    if screen.is_some() {
        lines.extend(m.device_rules.iter().map(|r| format!("PodmanArgs=\"--device-cgroup-rule={}\"", esc(r))));
    }
    lines.extend(
        [
            "ShmSize=1g",
            // Built here, never pulled.
            "Pull=never",
            "",
            "[Service]",
            "Restart=no",
            // Rootless podman copies a whole image the first time it maps its ids.
            "TimeoutStartSec=900",
            "",
            "# No [Install] section: wadd starts it when it's opened.",
            "",
        ]
        .map(String::from),
    );
    lines.join("\n")
}

/// The display a streamed wadspace and its sidecar share. Owned by abc
/// (1000): a plain volume is root's each time it's mounted.
fn display_volume(display: &str) -> String {
    [
        "# Generated by wadd (wad-gen): the display a streamed wadspace and its sidecar share.",
        "[Volume]",
        &format!("VolumeName={display}"),
        "User=1000",
        "Group=1000",
        "",
    ]
    .join("\n")
}

/// A wadspace's stream sidecar, bound to it (they start and stop together).
///
/// TODO(phase 7, streams): Selkies 2.x's own settings (`--mode=webrtc`, the
/// secure mode's token table, the joystick interposer, serving under
/// `/ws-<guid>/` on the machine's host); this carries v1's Selkies 1.x
/// environment. Cloudflare's TURN: wadd mints short-lived credentials per
/// stream from /etc/wados/turn.toml into the podman secret `wad_turn_<id>`,
/// mounted here; how Selkies 2.x reads them is phase 7's.
fn sidecar_unit(ws: &Wadspace, m: &Machine, s: &Sidecar) -> String {
    let id = &ws.id;
    let display = display_name(id);
    let mut lines: Vec<String> = vec![
        "# Generated by wadd (wad-gen): the stream a wadspace draws on while it's streamed. Do not edit.".into(),
        "[Unit]".into(),
        format!("Description=WadSpaces stream: {}", esc(&ws.name)),
        "StopWhenUnneeded=yes".into(),
        String::new(),
        "[Container]".into(),
        format!("Image={}", s.image),
        format!("ContainerName={display}"),
        format!("Label={}={id}", labels::WADSPACE),
        format!("Label={}=display", labels::ROLE),
        format!("Label={}={}", labels::STREAM, esc(s.guid)),
    ];
    lines.extend(maps(m));
    lines.push(format!("SecurityLabelLevel={}", mcs_level(id)));
    let mut env: BTreeMap<&str, String> = BTreeMap::new();
    env.insert("PUID", "1000".into());
    env.insert("PGID", "1000".into());
    env.insert("TZ", tz(m).into());
    env.insert("TITLE", ws.name.clone());
    // WebSockets first: the transport that always works.
    env.insert("SELKIES_MODE", "websockets".into());
    if let Some(mux) = s.mux_port {
        // Never a public IP or ICE-lite: media goes through Cloudflare's STUN and TURN.
        env.insert("SELKIES_ENABLE_DUAL_MODE", "true".into());
        env.insert("SELKIES_WEBRTC_UDP_MUX_PORT", mux.to_string());
        env.insert("SELKIES_WEBRTC_TCP_MUX_PORT", mux.to_string());
        env.insert("SELKIES_STUN_HOST", "stun.cloudflare.com".into());
        env.insert("SELKIES_STUN_PORT", "3478".into());
    }
    lines.extend(env.iter().map(|(k, v)| env_line(k, v)));
    if s.mux_port.is_some() {
        lines.push(format!("Secret=wad_turn_{id},type=mount,target=wadspaces-turn"));
    }
    lines.push(format!("Volume={display}.volume:{DISPLAY_DIR}:z"));
    lines.push(format!("PublishPort=127.0.0.1:{}:{STREAM_PORT}", s.web_port));
    if let Some(mux) = s.mux_port {
        lines.push(format!("PublishPort={mux}:{mux}/udp"));
        lines.push(format!("PublishPort={mux}:{mux}/tcp"));
    }
    lines.extend(m.devices.iter().map(|d| format!("AddDevice={}", esc(d))));
    lines.extend(
        ["ShmSize=1g", "Pull=never", "", "[Service]", "Restart=no", "TimeoutStartSec=900", ""].map(String::from),
    );
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wadspace_and_its_sidecar_share_categories_of_their_own() {
        let a = mcs_level("writing");
        assert_eq!(a, mcs_level("writing"));
        assert_ne!(a, mcs_level("reading"));
        let (lo, hi) = a.strip_prefix("s0:c").and_then(|r| r.split_once(",c")).expect("s0:cA,cB");
        let (lo, hi): (u32, u32) = (lo.parse().expect("a number"), hi.parse().expect("a number"));
        assert!(lo < hi && hi < 1024, "{a}");
    }

    #[test]
    fn id_maps_rootless_and_rootful() {
        assert_eq!(
            rootful_maps(USERNS_BASE),
            [
                "UIDMap=0:1000000:1000",
                "UIDMap=1000:1000:1",
                "UIDMap=1001:1001001:64535",
                "GIDMap=0:1000000:1000",
                "GIDMap=1000:1000:1",
                "GIDMap=1001:1001001:64535",
            ]
        );
        assert_eq!(rootless_maps(1000)[..3], ["UIDMap=0:1:1000", "UIDMap=1000:0:1", "UIDMap=1001:1001:64535"]);
    }

    #[test]
    fn values_stay_on_their_line() {
        assert_eq!(env_line("TITLE", "Writing"), "Environment=TITLE=Writing");
        assert_eq!(env_line("TITLE", "Cosmic 100%"), "Environment=\"TITLE=Cosmic 100%%\"");
        assert_eq!(env_line("TITLE", "a\nExecStart=x"), "Environment=\"TITLE=a ExecStart=x\"");
        assert_eq!(env_line("X", "say \"hi\""), "Environment=\"X=say \\\"hi\\\"\"");
        assert_eq!(sandbox_app_id("iq"), "io.wadspaces.iq");
    }
}
