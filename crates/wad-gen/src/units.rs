//! A wadspace's quadlet units, which wadd writes into its units folder:
//! `wad-<id>.container` on the screen, and while it's streamed that, its
//! stream sidecar's (`wadstream-<id>.container`) and the display volume
//! they share (`wadstream-<id>.volume`): [`units`]. A wadspace whose desktop
//! moves between the screen and a stream while it runs has both displays
//! ([`moving_units`]).
//!
//! On the screen it reaches the machine's labwc only through a folder wadd
//! keeps for it, `<screen_dir>/<id>`, mounted at `/run/wadspaces-display`:
//! a Wayland socket labwc serves it alone (`wayland-1`, a security context
//! whose app id is `io.wadspaces.<id>`: [`sandbox_app_id`]), and
//! PulseAudio's folder (`pulse/`). Never the session's runtime dir.
//!
//! Its projects aren't in its image, and only those wadd found or made on
//! this machine are in it ([`Run::mounts`]): a GitHub project is wadd's
//! clone, worked in where it is (labelled for containers when wadd made
//! it); a copy is copied in once by the wadspace, from wadd's copy of the
//! files (read-only); a mount is the machine's own folder (or another
//! machine's, over Taildrive), read-write and never relabelled, so a
//! wadspace with one runs without SELinux's separation. Its keys are mounted from
//! their folders, shared with the other wadspaces that use them. Nothing
//! in a unit is a secret: podman secrets are named, never written.
//!
//! Streamed, it draws on its sidecar instead (`images/stream`: upstream
//! Selkies 2.x). Selkies' own Wayland compositor owns the display volume
//! the two share (the sidecar's runtime dir, `/run/wadspaces-display` in
//! it; [`STREAM_DIR`] in the wadspace): its socket (`wayland-*`) and its
//! PulseAudio's (`pulse/native`) are there, and the wadspace's labwc draws
//! on it as one window, as it does on the machine's labwc. Which one it
//! draws on, wadd says in its extra folder's [`DISPLAY_FILE`] (the base's
//! svc-de reads it): `stream` while it's streamed, else `screen`. The
//! sidecar serves the stream under `/ws-<guid>/` on one port,
//! published on 127.0.0.1 alone ([`Sidecar`]), in Selkies' secure mode: a
//! master token (a podman secret) and a token table wadd writes, with the
//! WebRTC ICE servers, in a folder of its own ([`SELKIES_DIR`]).
//!
//! Streamed securely, it runs in a VM of its own (libkrun), with the
//! stream's Selkies inside it: [`secure_units`].

use std::collections::BTreeMap;

use wad_model::{ProjectKind, Wadspace, Wallpaper};

use crate::extras::{self, SIGNINS_ROOT};
use crate::{Mounts, labels, mountable};

/// Where a wadspace sees the copies it takes in: `<COPIES_ROOT>/<project id>`.
pub const COPIES_ROOT: &str = "/run/wadspaces-copies";
/// Where a wadspace sees the screen (its screen folder), and its sidecar its
/// own display (the volume it shares with the wadspace: its runtime dir).
pub const DISPLAY_DIR: &str = "/run/wadspaces-display";
/// Where a wadspace sees its stream's display: the volume it shares with its sidecar.
pub const STREAM_DIR: &str = "/run/wadspaces-stream";
/// The file in its extra folder ([`extras::dir`]) that says which display its
/// desktop is on: `screen` ([`DISPLAY_DIR`]; or no file) or `stream`
/// ([`STREAM_DIR`]). wadd writes it before the wadspace starts, and before
/// it moves one (`wadspaces-session switch`).
pub const DISPLAY_FILE: &str = "display";
/// What the base's svc-de links to the display in use: PULSE_SERVER goes
/// through it, so sound follows the desktop when it moves.
const CURRENT_DIR: &str = "/run/wadspaces-current";
/// The Wayland socket in its screen folder (the image's desktop looks for `wayland-*`).
pub const SOCKET: &str = "wayland-1";
/// PulseAudio's folder in its screen folder (PULSE_SERVER: …/pulse/native).
pub const PULSE: &str = "pulse";
/// The security context's engine labwc's window rules match (`sandboxEngine`).
pub const SANDBOX_ENGINE: &str = "io.wadspaces";
/// The stream sidecar's port (plain HTTP: wadd's proxy in front of it ends TLS).
pub const STREAM_PORT: u16 = 3000;
/// Where the sidecar sees wadd's folder for its stream ([`Sidecar::dir`]), read-only.
pub const SELKIES_DIR: &str = "/run/wadspaces-selkies";
/// In the display volume: Selkies' Input Interposer and fake-udev (Play together).
pub const INTERPOSER_DIR: &str = "interposer";
/// Selkies' token table in it (`SELKIES_TOKENS_FILE`): who may watch.
pub const TOKENS_FILE: &str = "tokens.json";
/// The WebRTC ICE servers in it (`SELKIES_RTC_CONFIG_JSON`): a public stream's.
pub const RTC_FILE: &str = "rtc.json";
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

/// The path a stream is served under, on the sidecar as on wadd's proxy
/// (`SELKIES_SUBFOLDER`): its page is `<that>/`, with the slash.
pub fn stream_path(guid: &str) -> String {
    format!("/ws-{guid}")
}

/// The podman secret holding a streamed wadspace's Selkies master token
/// (`SELKIES_MASTER_TOKEN`: secure mode, and the control plane's key).
pub fn master_secret(id: &str) -> String {
    format!("wad_stream_master_{id}")
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
    /// wadd's state folder: extra folders (`extra/<id>`).
    pub state_dir: String,
    /// Keys' folders: `<signins_dir>/<key id>`.
    pub signins_dir: String,
    /// The MCP sockets' folders: `<agents_dir>/<wadspace id>`.
    pub agents_dir: String,
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
    /// Draw in software: NVIDIA's own driver (the images haven't its libraries), or a GPU
    /// with no 3D (a VM's virtio GPU without virgl), where wlroots won't fall back by itself.
    pub software_render: bool,
}

/// What one run of it gets.
#[derive(Debug, Clone, Copy)]
pub struct Run<'a> {
    /// The image it runs (its set's, [`crate::sets::Set::image`], or the one it built last).
    pub image: &'a str,
    /// Its projects found or made on this machine (a mount's folder, a clone, a copy).
    pub mounts: &'a Mounts,
    /// A wallpaper of its own for this run (opened from an app or a variant with one), over its look's.
    pub wallpaper: Option<&'a Wallpaper>,
}

/// How a stream reaches its viewers: Selkies' transport, by the stream's mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// A private stream, on the tailnet: Selkies' WebSocket transport alone
    /// (the page decodes with WebCodecs, so it needs a secure context:
    /// Tailscale's HTTPS, or `http://127.0.0.1`).
    WebSockets,
    /// A public stream: WebRTC, its media through the ICE servers wadd writes
    /// to [`RTC_FILE`] (Cloudflare's STUN and TURN), with WebSockets kept
    /// (dual mode) for a viewer WebRTC can't reach. `mux_port`: the one port
    /// (UDP and TCP) its host candidates share, published on the machine.
    WebRtc { mux_port: u16 },
}

/// A streamed wadspace's sidecar: what its unit needs that the wadspace and
/// the machine don't say.
///
/// What wadd does around it (the stream's lifecycle):
///
/// - **The master token**: random per stream, in the podman secret
///   [`master_secret`] before the unit starts (the sidecar won't start
///   without it). It's Selkies' control-plane key (`POST /api/tokens`,
///   `/api/switch` as `Authorization: Bearer`), never a viewer's.
/// - **[`Sidecar::dir`]**, wadd's folder for the stream, mounted read-only
///   at [`SELKIES_DIR`] (the folder, not its files, so a file replaced by a
///   rename is seen). Selkies refuses a file another user owns or can write:
///   both are the machine's user's (uid 1000, abc in the sidecar; not
///   root's, which isn't mapped in its user namespace), mode 0600, in a
///   folder of mode 0700. Write each as a new file in the folder, then rename
///   it over the old (Selkies polls them, and reads a whole file).
///   - [`TOKENS_FILE`]: Selkies' token table,
///     `{"<key>": {"role": "controller", "slot": null}}` with the stream's
///     key (the one-tap link's `#token=<key>`, and the `selkies_token`
///     cookie's). No viewer streams until it's there. A key taken out is cut
///     off at once. HOOK(gamepads): phase 10 adds players' rows (slots 2-4).
///   - [`RTC_FILE`] (public streams): `{"iceServers": [...]}`, Cloudflare's
///     STUN and the TURN credentials wadd mints, rewritten before they
///     expire; with TURN down, STUN alone. Never Cloudflare's key.
/// - **Ready**: `GET http://127.0.0.1:<web_port>/ws-<guid>/api/health` (no
///   token) answers 200 once Selkies serves.
/// - **The proxy** sends `/ws-<guid>/…` here unchanged (Host kept,
///   WebSockets upgraded, no buffering); anything else this sidecar 404s,
///   and `/ws-<guid>` without its slash is 403 here: the proxy redirects it.
///
/// Secure streams (libkrun) aren't these units: the wadspace reaches its
/// sidecar through Unix sockets in their shared volume (Wayland's and
/// PulseAudio's), which don't cross a VM's boundary, so a sidecar in a krun
/// VM can't serve a wadspace outside it (nor the other way round). Its
/// Selkies runs in the wadspace's own VM instead: [`secure_units`].
#[derive(Debug, Clone, Copy)]
pub struct Sidecar<'a> {
    /// The stream image (`localhost/wados-stream:44`).
    pub image: &'a str,
    /// The stream's id: it's served under [`stream_path`].
    pub guid: &'a str,
    /// Its web port, published on 127.0.0.1 only: wadd's proxy is the way in.
    pub web_port: u16,
    /// Private (WebSockets) or public (WebRTC and its mux port).
    pub transport: Transport,
    /// wadd's folder for the stream (`<state>/streams/<id>`): its token
    /// table and ICE servers, mounted read-only at [`SELKIES_DIR`].
    pub dir: &'a str,
    /// The origin its link opens it at (`https://<host>`, or
    /// `https://<machine>.<tailnet>:<port>`): a page there may open its
    /// streaming socket even if what's in front of wadd's proxy rewrites
    /// Host. None: only a page whose origin matches the request's Host.
    pub origin: Option<&'a str>,
    /// Play together's players, or 0: gamepads are on with 2 to 4.
    pub players: u8,
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

/// Its units: on the screen, its own; streamed (`stream`) from the start,
/// its own drawing on its sidecar's display ([`STREAM_DIR`]: wadd writes
/// `stream` in its [`DISPLAY_FILE`]), the sidecar, bound to it (they start
/// and stop together), and their display volume. (file name, text) each.
pub fn units(ws: &Wadspace, run: &Run, m: &Machine, stream: Option<&Sidecar>) -> Vec<(String, String)> {
    let file = format!("{}.container", container_name(&ws.id));
    let Some(s) = stream else { return vec![(file, wadspace_unit(ws, run, m, Draws::Screen))] };
    let display = display_name(&ws.id);
    vec![
        (file, with_pads(wadspace_unit(ws, run, m, Draws::Stream(&display)), s.players)),
        (format!("{display}.container"), sidecar_unit(ws, m, s, true)),
        (format!("{display}.volume"), display_volume(&display)),
    ]
}

/// The units of a wadspace whose desktop moves between the screen and a
/// stream as it runs (v1's `moving_quadlets`), for an image that can (the
/// base's `io.wadspaces.display` label, "2"): its own, with both displays
/// (its screen folder at [`DISPLAY_DIR`], the display volume at
/// [`STREAM_DIR`]) and sound through the one in use; the display volume,
/// always; and while it's streamed (`stream`), its sidecar, which wadd
/// starts and stops itself: the wadspace doesn't stop with it.
///
/// A move, either way: wadd writes the [`DISPLAY_FILE`] (`stream` or
/// `screen`; the sidecar's started first, for `stream`), then runs
/// `wadspaces-session switch` in the wadspace (`podman exec` as abc): its
/// apps close as their × would, labwc ends, and svc-de starts it again on
/// the other display, where `wadspaces-session restore` opens them again
/// (terminals keep their jobs, in tmux). A non-empty `{"still_open": [...]}`
/// from switch: those didn't close in time and nothing moved (`--force`
/// moves anyway); to call it off, wadd writes the file back and runs
/// `wadspaces-session restore`, which opens again what did close. Back on
/// the screen, wadd stops the sidecar and writes these units again without
/// it.
///
/// A machine with no screen has nothing to move between: [`units`].
pub fn moving_units(ws: &Wadspace, run: &Run, m: &Machine, stream: Option<&Sidecar>) -> Vec<(String, String)> {
    if m.screen_dir.is_none() {
        return units(ws, run, m, stream);
    }
    let display = display_name(&ws.id);
    let unit = with_pads(wadspace_unit(ws, run, m, Draws::Moves(&display)), stream.map_or(0, |s| s.players));
    let mut out = vec![(format!("{}.container", container_name(&ws.id)), unit)];
    if let Some(s) = stream {
        out.push((format!("{display}.container"), sidecar_unit(ws, m, s, false)));
    }
    out.push((format!("{display}.volume"), display_volume(&display)));
    out
}

/// Where a wadspace's desktop draws: each a sidecar's name ([`display_name`]).
#[derive(Clone, Copy)]
enum Draws<'a> {
    /// The machine's screen (its screen folder), when it has one.
    Screen,
    /// Its stream alone, from the start: it's bound to the sidecar.
    Stream(&'a str),
    /// Either, moving between them as it runs.
    Moves(&'a str),
}

/// The wadspace's unit.
/// Play together: the wadspace's apps get the sidecar's four pads. Selkies'
/// Input Interposer and fake-udev are preloaded from the display volume (the
/// sidecar puts them there), and their sockets are beside them. There's no
/// kernel device, so `/dev/input` only has to exist. Environment lines stay
/// in the `[Container]` section, whose end this finds.
fn with_pads(unit: String, players: u8) -> String {
    if players == 0 {
        return unit;
    }
    let dir = format!("{STREAM_DIR}/{INTERPOSER_DIR}");
    let lines = [
        env_line("SELKIES_JS_SOCKET_PATH", STREAM_DIR),
        env_line("SELKIES_INTERPOSER", &format!("{dir}/selkies_input_interposer.so")),
        env_line("LD_PRELOAD", &format!("{dir}/selkies_input_interposer.so:{dir}/libudev.so.1")),
        env_line(
            "SDL_JOYSTICK_DEVICE",
            "/dev/input/event1000:/dev/input/event1001:/dev/input/event1002:/dev/input/event1003",
        ),
        "Tmpfs=/dev/input:rw,mode=1777".to_string(),
    ];
    let at = unit.find("\n[Service]").or_else(|| unit.find("\n[Install]")).unwrap_or(unit.len());
    let (head, tail) = unit.split_at(at);
    format!("{}\n{}\n{tail}", head.trim_end(), lines.join("\n"))
}

#[cfg(test)]
mod pad_tests {
    use super::with_pads;

    #[test]
    fn only_play_together_preloads_the_interposer() {
        let unit = "[Container]\nImage=x\n\n[Service]\nRestart=no\n".to_string();
        assert_eq!(with_pads(unit.clone(), 0), unit);
        let pads = with_pads(unit, 4);
        let (container, service) = pads.split_once("[Service]").unwrap();
        assert!(container.contains("selkies_input_interposer.so:/run/wadspaces-stream/interposer/libudev.so.1"));
        assert!(container.contains("Tmpfs=/dev/input:rw,mode=1777"));
        assert!(!service.contains("LD_PRELOAD"));
    }
}

fn wadspace_unit(ws: &Wadspace, run: &Run, m: &Machine, draws: Draws) -> String {
    let id = &ws.id;
    let mut lines: Vec<String> = vec![
        format!("# Generated by wadd (wad-gen) for the wadspace {id}. Do not edit."),
        "[Unit]".into(),
        format!("Description=WadSpaces wadspace: {}", esc(&ws.name)),
        "Wants=network-online.target".into(),
        "After=network-online.target".into(),
    ];
    // Streamed: its sidecar starts first, and when it stops, so does this.
    if let Draws::Stream(d) = draws {
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
    let screen = m.screen_dir.as_deref().filter(|_| !matches!(draws, Draws::Stream(_)));
    let stream = match draws {
        Draws::Screen => None,
        Draws::Stream(d) | Draws::Moves(d) => Some(d),
    };

    let mut env = look_env(ws, run, m);
    if m.software_render {
        env.insert("WLR_RENDERER", "pixman".into());
    }
    if screen.is_some() && m.screens > 1 {
        env.insert("WLR_WL_OUTPUTS", m.screens.to_string());
    }
    // Sound: the session's PipeWire through its PulseAudio socket, or the
    // sidecar's; with a stream, through the display in use.
    if stream.is_some() {
        env.insert("PULSE_SERVER", format!("unix:{CURRENT_DIR}/{PULSE}/native"));
    } else if screen.is_some() {
        env.insert("PULSE_SERVER", format!("unix:{DISPLAY_DIR}/{PULSE}/native"));
    }
    lines.extend(env.iter().map(|(k, v)| env_line(k, v)));
    lines.extend(m.secrets.iter().map(|s| format!("Secret={}", esc(s))));

    let mounted = home_volumes(ws, run, m, &mut lines);
    if let Some(dir) = screen {
        lines.push(format!("Volume={}/{id}:{DISPLAY_DIR}:ro,rslave", esc(dir)));
        if let Some(p) = &m.pulse_dir {
            lines.push(format!("Volume={}:{DISPLAY_DIR}/{PULSE}:ro", esc(p)));
        }
    }
    if let Some(d) = stream {
        lines.push(format!("Volume={d}.volume:{STREAM_DIR}:z"));
    }
    // The user's own folders are never relabelled; else categories of its own.
    if mounted {
        lines.push("SecurityLabelDisable=true".into());
    } else {
        lines.push(format!("SecurityLabelLevel={}", mcs_level(id)));
    }
    // Nested containers: Steam's runtime and Proton (bwrap, in
    // pressure-vessel), and podman/buildah themselves (the WadOS wadspace's
    // `podman` tool, which builds and pushes images). That takes mounting
    // (SELinux's type for nested engines, with the categories above), a new
    // /proc, which the kernel refuses while parts of this one are masked,
    // and fuse for the storage (vfs needs none, fuse-overlayfs does).
    let nested = nested(ws);
    if nested {
        if !mounted {
            lines.push("SecurityLabelType=container_engine_t".into());
        }
        lines.push("Unmask=/proc/*".into());
        lines.push("AddDevice=/dev/fuse".into());
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

/// Runs containers of its own: Steam's runtime, or the podman tool (a
/// terminal add-on named `podman`, which the WadOS wadspace has).
pub fn nested(ws: &Wadspace) -> bool {
    ws.has_app(wad_model::AppId::Steam) || ws.ext(wad_model::AppId::Terminal, "podman").is_some()
}

/// Its own environment, wherever it draws: its user, time zone, title and look.
fn look_env(ws: &Wadspace, run: &Run, m: &Machine) -> BTreeMap<&'static str, String> {
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
    env
}

/// Its home, its projects found here, its keys and its extra folder, as
/// `Volume=` lines. True if one is a mount (the machine's own folder,
/// never relabelled).
fn home_volumes(ws: &Wadspace, run: &Run, m: &Machine, lines: &mut Vec<String>) -> bool {
    let id = &ws.id;
    // Its home: under a machine's maps, it keeps the owners its files have on disk.
    let idmap = if m.rootless_uid.is_none() && m.userns_base.is_some() { ",idmap" } else { "" };
    lines.push(format!("Volume=wad-{id}-config:/config:z{idmap}"));
    let mut mounted = false;
    for p in &ws.look.projects {
        // One not here (another machine's that's offline, a clone that failed) isn't in it.
        let Some(path) = run.mounts.get(&p.id).filter(|path| mountable(path, &p.as_name)) else { continue };
        match &p.kind {
            ProjectKind::Mount { .. } => {
                lines.push(format!("Volume={}:/config/Desktop/{}:rw", esc(path), esc(&p.as_name)));
                mounted = true;
            }
            // wadd's clone: labelled for containers when it was made, so no :z (a
            // relabel would walk the whole tree at every start).
            ProjectKind::Github { .. } => {
                lines.push(format!("Volume={}:/config/Desktop/{}:rw", esc(path), esc(&p.as_name)));
            }
            ProjectKind::Copy { .. } => {
                lines.push(format!("Volume={}:{COPIES_ROOT}/{}:ro,z", esc(path), p.id));
            }
        }
    }
    // Keys: each its folder, read-write, shared with the other wadspaces that use it (:z, not :Z).
    for s in extras::signins(ws) {
        lines.push(format!("Volume={}/{}:{SIGNINS_ROOT}/{}:rw,z", esc(&m.signins_dir), s.key.id, s.key.id));
    }
    lines.push(format!("Volume={}:{}:ro,z", esc(&extras::dir(&m.state_dir, id)), extras::DIR));
    // The `wadspaces` MCP server's socket, only with its Agents switch on.
    if ws.agents {
        lines.push(format!("Volume={}/{id}:/run/wadspaces-agents:ro,rslave", esc(&m.agents_dir)));
    }
    // Dolphin's games: the machine's discs (wadd's discs.rs).
    if ws.has_app(wad_model::AppId::Dolphin) {
        lines.push(format!("Volume={}/discs:/config/Games:ro,z", esc(&m.state_dir)));
    }
    mounted
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

/// A wadspace's stream sidecar: Selkies 2.x (`images/stream`) in secure
/// mode, under the stream's path. `bound`: its wadspace binds it (they start
/// and stop together); else wadd starts and stops it ([`moving_units`]).
///
/// Its environment: Selkies' per-stream settings (the image has the rest:
/// port 3000, plain HTTP, no Basic login, its own Wayland compositor,
/// Cloudflare's TURN only through [`RTC_FILE`], no file transfers, printing,
/// webcam or gamepads). Its mounts: the display volume and wadd's folder.
fn sidecar_unit(ws: &Wadspace, m: &Machine, s: &Sidecar, bound: bool) -> String {
    let id = &ws.id;
    let display = display_name(id);
    let mut lines: Vec<String> = vec![
        "# Generated by wadd (wad-gen): the stream a wadspace draws on while it's streamed. Do not edit.".into(),
        "[Unit]".into(),
        format!("Description=WadSpaces stream: {}", esc(&ws.name)),
    ];
    if bound {
        lines.push("StopWhenUnneeded=yes".into());
    }
    lines.extend([
        String::new(),
        "[Container]".into(),
        format!("Image={}", s.image),
        format!("ContainerName={display}"),
        format!("Label={}={id}", labels::WADSPACE),
        format!("Label={}=display", labels::ROLE),
        format!("Label={}={}", labels::STREAM, esc(s.guid)),
    ]);
    lines.extend(maps(m));
    // The wadspace's categories: it reads the buffers the wadspace draws in.
    lines.push(format!("SecurityLabelLevel={}", mcs_level(id)));
    let mut env: BTreeMap<&str, String> = BTreeMap::new();
    env.insert("PUID", "1000".into());
    env.insert("PGID", "1000".into());
    env.insert("TZ", tz(m).into());
    env.extend(selkies_env(ws, s, m.software_render));
    lines.extend(env.iter().map(|(k, v)| env_line(k, v)));
    lines.push(format!("Secret={},type=env,target=SELKIES_MASTER_TOKEN", master_secret(id)));
    lines.push(format!("Volume={display}.volume:{DISPLAY_DIR}:z"));
    lines.push(format!("Volume={}:{SELKIES_DIR}:ro,Z", esc(s.dir)));
    lines.push(format!("PublishPort=127.0.0.1:{}:{STREAM_PORT}", s.web_port));
    if let Transport::WebRtc { mux_port } = s.transport {
        lines.push(format!("PublishPort={mux_port}:{mux_port}/udp"));
        lines.push(format!("PublishPort={mux_port}:{mux_port}/tcp"));
    }
    lines.extend(m.devices.iter().map(|d| format!("AddDevice={}", esc(d))));
    lines.extend(
        ["ShmSize=1g", "Pull=never", "", "[Service]", "Restart=no", "TimeoutStartSec=900", ""].map(String::from),
    );
    lines.join("\n")
}

/// Selkies' per-stream settings (the stream image has the rest), as its
/// sidecar and a secure stream's VM both take them. `software`: its
/// compositor draws, and it encodes, on the CPU.
fn selkies_env(ws: &Wadspace, s: &Sidecar, software: bool) -> BTreeMap<&'static str, String> {
    let mut env: BTreeMap<&str, String> = BTreeMap::new();
    env.insert("SELKIES_UI_TITLE", ws.name.clone());
    env.insert("SELKIES_SUBFOLDER", stream_path(s.guid));
    env.insert("SELKIES_PORT", STREAM_PORT.to_string());
    env.insert("SELKIES_TOKENS_FILE", format!("{SELKIES_DIR}/{TOKENS_FILE}"));
    if s.players > 0 {
        env.insert("SELKIES_GAMEPAD_ENABLED", "true".into());
        env.insert("SELKIES_UINPUT_GAMEPAD", "false".into());
    }
    if let Some(origin) = s.origin {
        env.insert("SELKIES_ALLOWED_ORIGINS", origin.into());
    }
    match s.transport {
        Transport::WebSockets => {
            env.insert("SELKIES_MODE", "websockets".into());
            env.insert("SELKIES_ENABLE_DUAL_MODE", "false".into());
        }
        Transport::WebRtc { mux_port } => {
            // Never a public IP or ICE-lite: the machine has no address a
            // viewer can count on; media goes through Cloudflare's TURN.
            env.insert("SELKIES_MODE", "webrtc".into());
            env.insert("SELKIES_ENABLE_DUAL_MODE", "true".into());
            env.insert("SELKIES_RTC_CONFIG_JSON", format!("{SELKIES_DIR}/{RTC_FILE}"));
            env.insert("SELKIES_WEBRTC_UDP_MUX_PORT", mux_port.to_string());
            env.insert("SELKIES_WEBRTC_TCP_MUX_PORT", mux_port.to_string());
        }
    }
    // As the wadspace's labwc draws in software: its compositor too, and it
    // encodes on the CPU.
    if software {
        env.insert("SELKIES_AUTO_GPU", "false".into());
        env.insert("SELKIES_USE_CPU", "true".into());
    }
    env
}

// ------------------------------------------------------------ secure streams

/// Where a secure stream's VM sees the stream image's files (Selkies,
/// PulseAudio): an image volume, read-only ([`secure_volume`]).
pub const SECURE_STREAM_ROOT: &str = "/opt/wados-stream";
/// The host image's side of a secure stream, mounted into its VM read-only
/// (host/usr/lib/wados/secure): `secure-init`, the s6 service
/// `svc-secure-stream/`, and `empty` (the service's entry in s6's bundle).
pub const SECURE_GUEST_DIR: &str = "/usr/lib/wados/secure";
/// A secure stream's entrypoint, as its wadspace sees it.
pub const SECURE_INIT: &str = "/usr/libexec/wados-secure-init";
/// The s6 service that starts the stream inside the VM.
const SECURE_SERVICE: &str = "svc-secure-stream";
/// How long podman waits for a secure wadspace to stop (s6's own stop, in
/// the VM) before it kills the VM.
pub const SECURE_STOP_TIMEOUT_S: u32 = 30;
/// A secure stream's wadspace container's role ([`labels::ROLE`]).
pub const SECURE_ROLE: &str = "secure";

/// The stream image's own Selkies settings (images/stream/Containerfile's
/// `ENV`). A secure stream runs Selkies in a chroot of the image's files, not
/// as the image, so its `ENV` doesn't apply there: its unit says them again
/// (a test keeps the two the same).
pub const STREAM_IMAGE_SELKIES_ENV: &[(&str, &str)] = &[
    ("SELKIES_PORT", "3000"),
    ("SELKIES_PUBLIC", "true"),
    ("SELKIES_ENABLE_HTTPS", "false"),
    ("SELKIES_ENABLE_BASIC_AUTH", "false"),
    ("SELKIES_WAYLAND", "true"),
    ("SELKIES_ENABLE_CLOUDFLARE_TURN", "false"),
    ("SELKIES_FILE_TRANSFERS", "none"),
    ("SELKIES_UI_SIDEBAR_SHOW_FILES", "false"),
    ("SELKIES_PRINTING_ENABLED", "false"),
    ("SELKIES_WEBCAM_ENABLED", "false|locked"),
    ("SELKIES_UI_SIDEBAR_SHOW_WEBCAM", "false"),
    ("SELKIES_UI_SIDEBAR_SHOW_APPS", "false"),
    ("SELKIES_COMMAND_ENABLED", "false"),
    ("SELKIES_ENABLE_SHARING", "false"),
    ("SELKIES_UI_SIDEBAR_SHOW_SHARING", "false"),
    ("SELKIES_GAMEPAD_ENABLED", "false|locked"),
    ("SELKIES_GAMEPAD_ON_START", "true"),
    ("SELKIES_UINPUT_GAMEPAD", "false"),
    ("SELKIES_UI_SIDEBAR_SHOW_GAMEPADS", "false"),
    ("SELKIES_ENABLE_PLAYER2", "false|locked"),
    ("SELKIES_ENABLE_PLAYER3", "false|locked"),
    ("SELKIES_ENABLE_PLAYER4", "false|locked"),
];

/// A secure stream's image volume: the stream image's files. Never a
/// wadspace's container ([`container_name`]: `wad-`) nor a sidecar's or a
/// display volume ([`display_name`]: `wadstream-`).
pub fn secure_volume(id: &str) -> String {
    format!("wadsecure-{id}")
}

/// A secure stream's VM.
#[derive(Debug, Clone, Copy)]
pub struct Vm<'a> {
    /// The OCI runtime (`[streaming] secure_runtime`, `krun`): `podman --runtime=<it>`.
    pub runtime: &'a str,
    /// Its vCPUs and memory (MiB): libkrun's `krun.cpus` and `krun.ram_mib`.
    pub cpus: u32,
    pub ram_mib: u32,
}

/// Why a wadspace can't be streamed securely, as the person reads it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecureRefused {
    /// A Mount project: the machine's own folder is never relabelled, so the
    /// wadspace runs without SELinux's separation, and so would its VM.
    #[error(
        "Secure streams can't take a folder from this machine ({0}): it turns off the separation the VM relies on. Take it out of the wadspace, or stream it without Secure"
    )]
    Mount(String),
    /// A public stream: WebRTC's mux port is UDP, which libkrun's TSI can't
    /// take in (passt can: not yet).
    #[error("Secure streams are private for now: stream it privately, or publicly without Secure")]
    Public,
}

/// The units of a wadspace streamed securely: one container, its own image
/// run as a libkrun micro-VM (`podman --runtime=krun`), with its desktop and
/// Selkies both inside it; and the image volume with the stream image's files
/// ([`secure_volume`]). No sidecar and no display volume: a Wayland socket
/// (and PulseAudio's) can't cross into a VM. tools/krun/NOTES.md says how it
/// was found to work, and what doesn't.
///
/// Inside the VM, the wadspace's own s6 runs as it always does, but as PID 1
/// of a PID namespace ([`SECURE_INIT`]: libkrun's init is the VM's PID 1),
/// with one service more, `svc-secure-stream`: PulseAudio and Selkies, from
/// the stream image's files at [`SECURE_STREAM_ROOT`] (a chroot of them),
/// their sockets in `/run/wadspaces-display` as in a sidecar, and
/// `/run/wadspaces-stream` pointing there. Selkies' compositor is the display
/// the wadspace's labwc draws on, and it draws in software. Its settings are
/// the sidecar's, private (WebSockets) only.
///
/// Refused ([`SecureRefused`]): a wadspace with a Mount project here, and a
/// public stream.
///
/// What wadd does around it, beyond a sidecar's ([`Sidecar`]: the master
/// token's secret, the stream's folder and token table, the proxy):
///
/// - **Available** when `[streaming] secure_runtime` is set, that runtime is
///   installed (`/usr/bin/krun`: the host image's crun-krun), and `/dev/kvm`
///   is there (streams.rs's `vm_runtime`). Never public (above).
/// - **Its extra folder's [`DISPLAY_FILE`]** says `stream`, as for a sidecar:
///   svc-de draws on `/run/wadspaces-stream`, and the clipboard bridge runs.
/// - **Ready**: no `podman exec` into a VM ("the handler does not support
///   exec"), so not the screen's desktop check: poll
///   `GET http://127.0.0.1:<web_port>/ws-<guid>/api/health` until it's 200
///   (Selkies serves once the VM, s6 and Selkies are up: 30-60 s in a
///   nested VM, so a few minutes' timeout).
/// - **Stop**: podman's SIGTERM never reaches the VM, its SIGINT does
///   (`StopSignal=SIGINT`), and `secure-init` hands s6 a SIGTERM; podman
///   waits [`SECURE_STOP_TIMEOUT_S`] before it kills the VM. A stop takes
///   10-15 s.
/// - **The image volume**: quadlet makes it from the stream image once, when
///   it isn't there, and its unit (`wadsecure-<id>-volume.service`, a
///   oneshot) stays active. When the stream ends (and before a new one, in
///   case one was left), stop that service, then `podman volume rm -f
///   wadsecure-<id>`: removing the volume alone leaves podman to make an
///   empty one in its place at the next start, and a new stream image would
///   never be used.
/// - **Units**: write these instead of [`units`]' (remove a sidecar's
///   `wadstream-<id>.container` and `.volume`); when it ends, write the
///   screen's again and remove `wadsecure-<id>.volume`.
/// - **Not here**: moving it to the screen while it runs (the person at the
///   machine opening it stops the stream, and the VM, first), the machine's
///   clipboard, sound from the machine, and a GPU.
pub fn secure_units(
    ws: &Wadspace,
    run: &Run,
    m: &Machine,
    s: &Sidecar,
    vm: &Vm,
) -> Result<Vec<(String, String)>, SecureRefused> {
    if let Transport::WebRtc { .. } = s.transport {
        return Err(SecureRefused::Public);
    }
    if let Some(p) = ws.look.projects.iter().find(|p| {
        matches!(p.kind, ProjectKind::Mount { .. })
            && run.mounts.get(&p.id).is_some_and(|path| mountable(path, &p.as_name))
    }) {
        return Err(SecureRefused::Mount(p.as_name.clone()));
    }
    let id = &ws.id;
    let volume = secure_volume(id);
    Ok(vec![
        (format!("{}.container", container_name(id)), secure_unit(ws, run, m, s, vm, &volume)),
        (format!("{volume}.volume"), secure_image_volume(&volume, s.image)),
    ])
}

fn secure_unit(ws: &Wadspace, run: &Run, m: &Machine, s: &Sidecar, vm: &Vm, volume: &str) -> String {
    let id = &ws.id;
    let mut lines: Vec<String> = vec![
        format!("# Generated by wadd (wad-gen) for the wadspace {id}, streamed securely (its own VM). Do not edit."),
        "[Unit]".into(),
        format!("Description=WadSpaces wadspace (secure stream): {}", esc(&ws.name)),
        "Wants=network-online.target".into(),
        "After=network-online.target".into(),
        String::new(),
        "[Container]".into(),
        format!("Image={}", run.image),
        format!("ContainerName={}", container_name(id)),
        format!("Label={}={id}", labels::WADSPACE),
        format!("Label={}={SECURE_ROLE}", labels::ROLE),
        format!("Label={}={}", labels::STREAM, esc(s.guid)),
        // podman --runtime=krun run …: crun's libkrun handler boots the
        // image's files (virtio-fs) under libkrunfw's kernel.
        format!("GlobalArgs=--runtime={}", esc(vm.runtime)),
        format!("Annotation=krun.cpus={}", vm.cpus),
        format!("Annotation=krun.ram_mib={}", vm.ram_mib),
    ];
    lines.extend(maps(m));

    // Its own, then Selkies': the stream image's (its ENV doesn't reach the
    // chroot), then the stream's, which win. No GPU in the VM: software.
    let mut env = look_env(ws, run, m);
    env.insert("WLR_RENDERER", "pixman".into());
    // The VM's own PulseAudio, through the display in use (svc-de's link).
    env.insert("PULSE_SERVER", format!("unix:{CURRENT_DIR}/{PULSE}/native"));
    env.extend(STREAM_IMAGE_SELKIES_ENV.iter().map(|(k, v)| (*k, v.to_string())));
    env.extend(selkies_env(ws, s, true));
    lines.extend(env.iter().map(|(k, v)| env_line(k, v)));
    lines.extend(m.secrets.iter().map(|s| format!("Secret={}", esc(s))));
    lines.push(format!("Secret={},type=env,target=SELKIES_MASTER_TOKEN", master_secret(id)));

    // A Mount project refused it (secure_units): never SecurityLabelDisable.
    home_volumes(ws, run, m, &mut lines);
    lines.push(format!("Volume={}:{SELKIES_DIR}:ro,Z", esc(s.dir)));
    lines.push(format!("Volume={volume}.volume:{SECURE_STREAM_ROOT}:ro"));
    lines.extend([
        format!("Volume={SECURE_GUEST_DIR}/secure-init:{SECURE_INIT}:ro"),
        format!("Volume={SECURE_GUEST_DIR}/{SECURE_SERVICE}:/etc/s6-overlay/s6-rc.d/{SECURE_SERVICE}:ro"),
        format!("Volume={SECURE_GUEST_DIR}/empty:/etc/s6-overlay/s6-rc.d/user/contents.d/{SECURE_SERVICE}:ro"),
        format!("Entrypoint={SECURE_INIT}"),
        // podman's SIGTERM doesn't reach the VM; its SIGINT does.
        "StopSignal=SIGINT".into(),
        format!("StopTimeout={SECURE_STOP_TIMEOUT_S}"),
        // Through libkrun's TSI (the VM has no network interface), on the
        // machine's loopback alone: wadd's proxy is the way in.
        format!("PublishPort=127.0.0.1:{}:{STREAM_PORT}", s.web_port),
        format!("SecurityLabelLevel={}", mcs_level(id)),
    ]);
    lines.extend(
        [
            "ShmSize=1g",
            "Pull=never",
            "",
            "[Service]",
            "Restart=no",
            "TimeoutStartSec=900",
            // Longer than podman's own wait: s6 stops cleanly first.
            "TimeoutStopSec=60",
            "",
            "# No [Install] section: wadd starts it when it's streamed.",
            "",
        ]
        .map(String::from),
    );
    lines.join("\n")
}

/// The stream image's files, for a secure stream's VM: made from the image
/// once, when the volume isn't there (see [`secure_units`] on removing it).
fn secure_image_volume(volume: &str, image: &str) -> String {
    [
        "# Generated by wadd (wad-gen): the stream image's files, for a secure stream's VM. Do not edit.",
        "[Volume]",
        &format!("VolumeName={volume}"),
        "Driver=image",
        &format!("Image={}", esc(image)),
        "",
    ]
    .join("\n")
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
