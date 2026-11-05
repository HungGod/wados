//! What the streams' surfaces say, worked out from what wadd says (no GTK):
//! the modes' names and hints, the Stream menu (`shell/common/menus.ts`'s
//! `streamMenu`), the sheets' words (`shell/stream/StreamSheet.svelte`,
//! `ShareSheet.svelte`, rewritten for the real design: no SWAG, no Kata),
//! the badges, the status line, the share's fields, the Streams flyout's
//! rows and the toasts (`sim/jobs/stream.ts`).

use wad_model::{StreamMode, Transport};
use wad_proto::streams::TURN_DOWN;
use wad_proto::{StreamInfo, StreamShare, StreamsAvailability};

/// The modes, in the menu's order.
pub const MODES: [StreamMode; 4] =
    [StreamMode::Private, StreamMode::PrivateSecure, StreamMode::Public, StreamMode::PublicSecure];

/// What the Public half of the sheet says.
pub const PUBLIC_TEXT: &str = "Anyone with the link can use it, in any browser: WebRTC with Cloudflare’s TURN for the video, at this machine’s address.";
/// What the Private half says.
pub const PRIVATE_TEXT: &str = "Only your own devices on your tailnet can open it: a WebSocket stream, served by Tailscale on this machine’s tailnet name. Nothing goes through Cloudflare.";
/// Under the Secure toggle.
pub const SECURE_TEXT: &str = "Its container runs in its own small VM: its own kernel, so a break-out reaches the VM, not this machine. A little slower to start.";
/// The Secure toggle's tooltip.
pub const SECURE_TIP: &str = "Secure: in a VM of its own (libkrun)";
/// What the Streams flyout says with nothing streaming.
pub const NOTHING: &str = "Nothing’s streaming. Right-click a wadspace ▸ Stream.";
/// The More flyout's Streams tile's tooltip.
pub const TILE_TIP: &str = "Streams: what’s live, its link, and stopping it";
/// What a mode says while wadd hasn't said which can start.
pub const NOT_KNOWN: &str = "wadd hasn’t said what can stream yet";

pub fn mode_label(m: StreamMode) -> &'static str {
    match m {
        StreamMode::Private => "Private",
        StreamMode::PrivateSecure => "Private (secure)",
        StreamMode::Public => "Public",
        StreamMode::PublicSecure => "Public (secure)",
    }
}

/// The Stream menu's hint under a mode.
pub fn mode_hint(m: StreamMode) -> &'static str {
    match m {
        StreamMode::Private => "WebSocket, only on your tailnet",
        StreamMode::PrivateSecure => "On your tailnet, in a VM of its own",
        StreamMode::Public => "WebRTC at this machine’s address",
        StreamMode::PublicSecure => "At this machine’s address, in a VM of its own",
    }
}

pub fn mode_icon(m: StreamMode) -> &'static str {
    match m {
        StreamMode::Private => "network",
        StreamMode::Public => "globe",
        StreamMode::PrivateSecure | StreamMode::PublicSecure => "shield-check",
    }
}

/// The mode the sheet's two choices make.
pub fn mode_of(public: bool, secure: bool) -> StreamMode {
    match (public, secure) {
        (false, false) => StreamMode::Private,
        (false, true) => StreamMode::PrivateSecure,
        (true, false) => StreamMode::Public,
        (true, true) => StreamMode::PublicSecure,
    }
}

pub fn transport_label(t: Transport) -> &'static str {
    match t {
        Transport::Websocket => "WebSocket",
        Transport::Webrtc => "WebRTC",
        Transport::WebrtcRelay => "WebRTC (relayed)",
    }
}

/// Why `mode` can't start here (none: it can).
pub fn why_not(avail: Option<&StreamsAvailability>, mode: StreamMode) -> Option<String> {
    let Some(a) = avail else { return Some(NOT_KNOWN.into()) };
    let a = a.get(mode);
    (!a.available).then(|| a.reason.clone().unwrap_or_else(|| format!("{} streams can’t start here", mode_label(mode))))
}

/// The sheet shows its Secure toggle: a secure mode can start here.
pub fn secure_shown(avail: Option<&StreamsAvailability>) -> bool {
    avail.is_some_and(|a| a.private_secure.available || a.public_secure.available)
}

// ------------------------------------------------------------ the Stream menu

/// What a row of the Stream menu does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// The stream sheet, at this mode.
    Sheet(StreamMode),
    /// Its stream's link (the share sheet; the stream sheet while it starts).
    Share,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    /// A test hook (`stream-private`).
    pub id: String,
    pub mode: StreamMode,
    pub label: &'static str,
    pub icon: &'static str,
    pub hint: String,
    pub checked: bool,
    /// Off, and why.
    pub disabled: Option<String>,
    pub pick: Pick,
}

/// A wadspace's Stream menu: Private and Public (and each secure, when it
/// can start here); the mode it streams in now is ticked and shows its
/// link. Another mode waits for that one to stop (one stream per wadspace).
pub fn stream_menu(live: Option<&StreamInfo>, avail: Option<&StreamsAvailability>) -> Vec<MenuItem> {
    MODES
        .into_iter()
        .filter(|&m| !m.is_secure() || live.is_some_and(|s| s.mode == m) || why_not(avail, m).is_none())
        .map(|mode| {
            let on = live.filter(|s| s.mode == mode);
            let disabled = match (on, live) {
                (Some(_), _) => None,
                (None, Some(s)) => Some(format!("Stop its {} stream first", mode_label(s.mode).to_lowercase())),
                (None, None) => why_not(avail, mode),
            };
            MenuItem {
                id: format!("stream-{}", mode_id(mode)),
                mode,
                label: mode_label(mode),
                icon: mode_icon(mode),
                hint: match on {
                    Some(s) if s.live => "Streaming now: show its link".into(),
                    Some(_) => "Starting…".into(),
                    None => mode_hint(mode).into(),
                },
                checked: on.is_some(),
                disabled,
                pick: if on.is_some() { Pick::Share } else { Pick::Sheet(mode) },
            }
        })
        .collect()
}

/// A mode as it's written ('private-secure').
pub fn mode_id(m: StreamMode) -> &'static str {
    match m {
        StreamMode::Private => "private",
        StreamMode::PrivateSecure => "private-secure",
        StreamMode::Public => "public",
        StreamMode::PublicSecure => "public-secure",
    }
}

pub fn parse_mode(s: &str) -> Option<StreamMode> {
    MODES.into_iter().find(|m| mode_id(*m) == s)
}

/// The launcher card's Stream (public) or Private button: what it does,
/// its tooltip, and whether it's on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    pub hint: String,
    pub enabled: bool,
    pub pick: Pick,
}

/// The card's button for `mode` (Public or Private): the sheet, or the
/// stream's link when it streams so; off (why, as its tooltip) when it
/// can't start, or another stream of it has to stop first.
pub fn card_button(mode: StreamMode, live: Option<&StreamInfo>, avail: Option<&StreamsAvailability>) -> Button {
    let hint = if mode.is_public() { "Stream it publicly" } else { "Stream it privately, on your tailnet" };
    match live {
        Some(s) if s.mode.is_public() == mode.is_public() => Button {
            hint: if s.live { "Streaming now: show its link".into() } else { "Starting its stream…".into() },
            enabled: true,
            pick: Pick::Share,
        },
        Some(s) => Button {
            hint: format!("{hint} (stop its {} stream first)", mode_label(s.mode).to_lowercase()),
            enabled: false,
            pick: Pick::Sheet(mode),
        },
        None => match why_not(avail, mode) {
            Some(why) => Button { hint: format!("{hint}: {why}"), enabled: false, pick: Pick::Sheet(mode) },
            None => Button { hint: hint.into(), enabled: true, pick: Pick::Sheet(mode) },
        },
    }
}

// ------------------------------------------------------------ the sheets

pub fn sheet_title(name: &str, started: bool) -> String {
    if started { format!("Streaming {name}") } else { format!("Stream {name}") }
}

pub fn what_text(public: bool) -> &'static str {
    if public { PUBLIC_TEXT } else { PRIVATE_TEXT }
}

/// The confirm, when it's on this screen (or in a window here).
pub fn takeover_text(name: &str) -> String {
    format!("{name} is on this screen. Streaming it takes it off the screen.")
}

/// The sheet's main button: `Stream · {Mode}`, or once it's asked, the confirm's.
pub fn go_label(mode: StreamMode, confirming: bool) -> String {
    if confirming { "Take it off the screen and stream".into() } else { format!("Stream · {}", mode_label(mode)) }
}

/// The stream sheet's main button, as it's drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Go {
    pub label: String,
    /// It can be pressed.
    pub enabled: bool,
    /// Why not (its tooltip), when the mode can't start here.
    pub tip: Option<String>,
    /// Drawn as the main action (the accent); a mode that can't start is
    /// drawn plain as well as dimmed, so it never reads as one to press.
    pub primary: bool,
}

/// The sheet's main button for `mode`: off, saying why, when the mode
/// can't start (or wadd hasn't said); off while its start is on its way.
pub fn go_button(mode: StreamMode, avail: Option<&StreamsAvailability>, confirming: bool, busy: bool) -> Go {
    let why = why_not(avail, mode);
    Go { label: go_label(mode, confirming), enabled: why.is_none() && !busy, primary: why.is_none(), tip: why }
}

pub fn share_title(name: &str) -> String {
    format!("{name} is streaming")
}

/// The share sheet's pill: `{Mode} · WebSocket`.
pub fn mode_pill(mode: StreamMode, transport: Transport) -> String {
    format!("{} · {}", mode_label(mode), transport_label(transport))
}

/// What the share sheet says beside its QR code.
pub fn share_text(mode: StreamMode) -> &'static str {
    if mode.is_public() {
        "Scan it to open it signed in, or send the link with its user and password."
    } else {
        "Scan it on a device on your tailnet: it opens signed in as you."
    }
}

/// What the QR code says: the one-tap link (its key in the fragment).
pub fn qr_text(share: &StreamShare) -> &str {
    &share.open_link
}

/// One of the share sheet's fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// A test hook (`link`, `user`, `password`).
    pub id: &'static str,
    pub label: &'static str,
    /// What it shows (a secret's, once revealed).
    pub shown: String,
    /// What its copy button copies.
    pub copied: String,
    /// Masked until it's revealed.
    pub secret: bool,
}

/// The mask over a secret.
pub const MASK: &str = "••••••••••••••••••••";

/// The share's fields: its link (a public one's asks for the user and
/// password, which come next; a private one's only opens by its key, so
/// that's what's copied); a public stream's user, and its password, masked.
pub fn share_fields(share: &StreamShare) -> Vec<Field> {
    let public = share.mode.is_public() && share.user.is_some();
    let mut out = vec![Field {
        id: "link",
        label: "Link",
        shown: share.link.clone(),
        copied: if public { share.link.clone() } else { share.open_link.clone() },
        secret: false,
    }];
    if public {
        let user = share.user.clone().unwrap_or_default();
        out.push(Field { id: "user", label: "User", shown: user.clone(), copied: user, secret: false });
        let pass = share.password.clone().unwrap_or_default();
        out.push(Field { id: "password", label: "Password", shown: pass.clone(), copied: pass, secret: true });
    }
    out
}

/// What the person's told about a stream: its warnings, TURN's first when it didn't answer.
pub fn warnings(s: &StreamInfo) -> Vec<String> {
    let mut out = s.warnings.clone();
    if s.turn_down && !out.iter().any(|w| w == TURN_DOWN) {
        out.insert(0, TURN_DOWN.into());
    }
    out
}

/// How long ago, as the spike's `ago`.
pub fn ago(now_ms: i64, at_ms: i64) -> String {
    wad_gtk::kit::ago(now_ms, at_ms)
}

/// The share sheet's status line: `Streaming[ from {peer}] · {n} watching · since {ago}`.
pub fn status_line(s: &StreamInfo, peer: Option<&str>, now_ms: i64) -> String {
    let from = peer.map(|p| format!(" from {p}")).unwrap_or_default();
    if !s.live {
        return format!("Starting{from}…");
    }
    format!("Streaming{from} · {} watching · since {}", s.viewers, ago(now_ms, s.started))
}

/// A card's badge: `LIVE · {n}`, or `Starting stream`.
pub fn card_badge(s: &StreamInfo) -> String {
    if s.live { format!("LIVE · {}", s.viewers) } else { "Starting stream".into() }
}

/// The Streams flyout's line under a stream's name: `{Mode} · {n} watching | starting[ · on {peer}]`.
pub fn flyout_sub(s: &StreamInfo, peer: Option<&str>) -> String {
    let how = if s.live { format!("{} watching", s.viewers) } else { "starting".into() };
    let on = peer.map(|p| format!(" · on {p}")).unwrap_or_default();
    format!("{} · {how}{on}", mode_label(s.mode))
}

/// The Streams tile's line: how many are live.
pub fn tile_sub(streams: &[StreamInfo]) -> String {
    format!("{} live", streams.iter().filter(|s| s.live).count())
}

/// Where a link goes (`host[:port]`), for the toast.
pub fn host_of(link: &str) -> String {
    let rest = link.split_once("://").map(|(_, r)| r).unwrap_or(link);
    rest.split(['/', '#', '?']).next().unwrap_or(rest).to_string()
}

// ------------------------------------------------------------ toasts

/// Live: `Streaming {name}`, `{Mode} · {host}`.
pub fn live_toast(name: &str, s: &StreamInfo) -> (String, String) {
    (format!("Streaming {name}"), format!("{} · {}", mode_label(s.mode), host_of(&s.link)))
}

pub fn failed_title(name: &str) -> String {
    format!("{name} didn’t start streaming")
}

pub fn stopped_title(name: &str) -> String {
    format!("{name} stopped streaming")
}

/// How a stream changed between two of wadd's lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
    /// It's live now (it was starting, or new).
    Live,
    /// It ended, or went: it had been live, or not (stopped, or failed, while it started).
    Ended { was_live: bool },
}

/// What changed from `before` to `after` (each stream by its key, `key`).
pub fn turns(
    before: &[StreamInfo],
    after: &[StreamInfo],
    key: impl Fn(&StreamInfo) -> String,
) -> Vec<(String, Turn, StreamInfo)> {
    use wad_model::StreamStage;
    let on = |s: &StreamInfo| s.stage != StreamStage::Ended;
    let mut out = vec![];
    for a in after.iter().filter(|s| on(s) && s.live) {
        let k = key(a);
        if !before.iter().any(|b| on(b) && b.live && key(b) == k) {
            out.push((k, Turn::Live, a.clone()));
        }
    }
    for b in before.iter().filter(|s| on(s)) {
        let k = key(b);
        if !after.iter().any(|a| on(a) && key(a) == k) {
            out.push((k, Turn::Ended { was_live: b.live }, b.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_model::StreamStage;
    use wad_proto::Availability;
    use wad_proto::streams::{PRIVATE_NEEDS_TAILNET, PUBLIC_NEEDS_TUNNEL, SECURE_NEEDS_VM};

    fn stream(id: &str, mode: StreamMode, live: bool) -> StreamInfo {
        StreamInfo {
            id: id.into(),
            mode,
            transport: if mode.is_public() { Transport::Webrtc } else { Transport::Websocket },
            stage: if live { StreamStage::Live } else { StreamStage::Sidecar },
            live,
            viewers: if live { 2 } else { 0 },
            started: 1_000_000_000 - 25 * 60_000,
            last_active: 1_000_000_000,
            link: if mode.is_public() {
                "https://sample-host.invalid/ws-sample-wadspaces/".into()
            } else {
                "https://desk.sample-tailnet.invalid:48300/ws-sample-notes/".into()
            },
            turn_down: false,
            warnings: vec![],
            job: None,
            peer: None,
        }
    }

    /// The sample's: private and public, secure not.
    fn sample() -> StreamsAvailability {
        StreamsAvailability {
            private: Availability::yes(),
            private_secure: Availability::no(SECURE_NEEDS_VM),
            public: Availability::yes(),
            public_secure: Availability::no(SECURE_NEEDS_VM),
        }
    }

    fn ids(m: &[MenuItem]) -> Vec<&str> {
        m.iter().map(|x| x.id.as_str()).collect()
    }

    #[test]
    fn the_menu_has_private_and_public_and_secure_only_when_it_can_start() {
        let m = stream_menu(None, Some(&sample()));
        assert_eq!(ids(&m), ["stream-private", "stream-public"]);
        assert_eq!(
            (m[0].label, m[0].hint.as_str(), m[0].icon),
            ("Private", "WebSocket, only on your tailnet", "network")
        );
        assert_eq!(
            (m[1].label, m[1].hint.as_str(), m[1].icon),
            ("Public", "WebRTC at this machine’s address", "globe")
        );
        assert!(m.iter().all(|x| x.disabled.is_none() && !x.checked));
        assert_eq!(m[1].pick, Pick::Sheet(StreamMode::Public));
        // With a VM runtime, the secure ones too, each after its own.
        let all = StreamsAvailability::of(true, true, true);
        let m = stream_menu(None, Some(&all));
        assert_eq!(ids(&m), ["stream-private", "stream-private-secure", "stream-public", "stream-public-secure"]);
        assert_eq!(m[1].label, "Private (secure)");
        for x in &m {
            assert!(!x.hint.contains("Kata") && !x.hint.contains("wadspaces.com"), "{}", x.hint);
        }
    }

    #[test]
    fn a_mode_that_cant_start_says_why() {
        let none = StreamsAvailability::of(false, false, false);
        let m = stream_menu(None, Some(&none));
        assert_eq!(ids(&m), ["stream-private", "stream-public"]);
        assert_eq!(m[0].disabled.as_deref(), Some(PRIVATE_NEEDS_TAILNET));
        assert_eq!(m[1].disabled.as_deref(), Some(PUBLIC_NEEDS_TUNNEL));
        // wadd hasn't said: nothing starts yet.
        let m = stream_menu(None, None);
        assert!(m.iter().all(|x| x.disabled.as_deref() == Some(NOT_KNOWN)));
        assert_eq!(why_not(Some(&sample()), StreamMode::PublicSecure).as_deref(), Some(SECURE_NEEDS_VM));
        assert!(!secure_shown(Some(&sample())) && secure_shown(Some(&StreamsAvailability::of(true, true, true))));
    }

    #[test]
    fn the_mode_it_streams_in_shows_its_link_and_the_others_wait() {
        let live = stream("wadspaces", StreamMode::Public, true);
        let m = stream_menu(Some(&live), Some(&sample()));
        assert_eq!(ids(&m), ["stream-private", "stream-public"]);
        assert!(m[1].checked && m[1].disabled.is_none());
        assert_eq!((m[1].hint.as_str(), m[1].pick), ("Streaming now: show its link", Pick::Share));
        assert_eq!(m[0].disabled.as_deref(), Some("Stop its public stream first"));
        let starting = stream("wadspaces", StreamMode::Private, false);
        let m = stream_menu(Some(&starting), Some(&sample()));
        assert_eq!(m[0].hint, "Starting…");
        // A secure stream that's on shows, even if its mode can't start again now.
        let secure = stream("wadspaces", StreamMode::PrivateSecure, true);
        let m = stream_menu(Some(&secure), Some(&sample()));
        assert_eq!(ids(&m), ["stream-private", "stream-private-secure", "stream-public"]);
        assert!(m[1].checked);
    }

    #[test]
    fn the_cards_buttons_stream_publicly_and_privately() {
        let b = card_button(StreamMode::Public, None, Some(&sample()));
        assert_eq!((b.hint.as_str(), b.enabled, b.pick), ("Stream it publicly", true, Pick::Sheet(StreamMode::Public)));
        let b = card_button(StreamMode::Private, None, Some(&StreamsAvailability::of(false, true, false)));
        assert_eq!(b.hint, format!("Stream it privately, on your tailnet: {PRIVATE_NEEDS_TAILNET}"));
        assert!(!b.enabled);
        let live = stream("w", StreamMode::Public, true);
        let b = card_button(StreamMode::Public, Some(&live), Some(&sample()));
        assert_eq!((b.hint.as_str(), b.pick, b.enabled), ("Streaming now: show its link", Pick::Share, true));
        let b = card_button(StreamMode::Private, Some(&live), Some(&sample()));
        assert_eq!(
            (b.hint.as_str(), b.enabled),
            ("Stream it privately, on your tailnet (stop its public stream first)", false)
        );
    }

    #[test]
    fn the_sheet_says_the_real_design_without_swag_or_kata() {
        assert_eq!(sheet_title("WadOS", false), "Stream WadOS");
        assert_eq!(sheet_title("WadOS", true), "Streaming WadOS");
        assert_eq!(go_label(StreamMode::Public, false), "Stream · Public");
        assert_eq!(go_label(StreamMode::PrivateSecure, false), "Stream · Private (secure)");
        assert_eq!(go_label(StreamMode::Private, true), "Take it off the screen and stream");
        assert_eq!(takeover_text("WadOS"), "WadOS is on this screen. Streaming it takes it off the screen.");
        assert!(what_text(true).starts_with("Anyone with the link can use it, in any browser"));
        assert!(what_text(false).ends_with("Nothing goes through Cloudflare."));
        for t in [PUBLIC_TEXT, PRIVATE_TEXT, SECURE_TEXT, SECURE_TIP] {
            assert!(!t.contains("SWAG") && !t.contains("Kata") && !t.contains("wadspaces.com"), "{t}");
        }
        assert_eq!(mode_of(true, false), StreamMode::Public);
        assert_eq!(mode_of(false, true), StreamMode::PrivateSecure);
        for m in MODES {
            assert_eq!(parse_mode(mode_id(m)), Some(m));
        }
    }

    #[test]
    fn the_sheets_button_is_off_saying_why_when_its_mode_cant_start() {
        let no_tunnel = StreamsAvailability::of(true, false, false);
        let go = go_button(StreamMode::Public, Some(&no_tunnel), false, false);
        assert_eq!(
            go,
            Go {
                label: "Stream · Public".into(),
                enabled: false,
                tip: Some(PUBLIC_NEEDS_TUNNEL.into()),
                primary: false
            }
        );
        // The other mode can: on, the main action, no tooltip.
        let go = go_button(StreamMode::Private, Some(&no_tunnel), false, false);
        assert_eq!((go.enabled, go.primary, go.tip), (true, true, None));
        // Off the tailnet: Private is off, saying so; the confirm's words don't turn it on.
        let off_tailnet = StreamsAvailability::of(false, true, false);
        let go = go_button(StreamMode::Private, Some(&off_tailnet), true, false);
        assert_eq!(go.label, "Take it off the screen and stream");
        assert_eq!((go.enabled, go.tip.as_deref()), (false, Some(PRIVATE_NEEDS_TAILNET)));
        // Secure without a VM runtime, and wadd not having said yet.
        let go = go_button(StreamMode::PublicSecure, Some(&sample()), false, false);
        assert_eq!((go.enabled, go.tip.as_deref()), (false, Some(SECURE_NEEDS_VM)));
        assert_eq!(go_button(StreamMode::Private, None, false, false).tip.as_deref(), Some(NOT_KNOWN));
        // Its start on its way: off for now, still the main action.
        let go = go_button(StreamMode::Private, Some(&sample()), false, true);
        assert_eq!((go.enabled, go.primary, go.tip), (false, true, None));
    }

    #[test]
    fn the_share_has_its_link_and_a_public_ones_sign_in() {
        let public = StreamShare {
            mode: StreamMode::Public,
            open_link: "https://sample-host.invalid/ws-sample-w/#token=sample-token-not-a-secret".into(),
            link: "https://sample-host.invalid/ws-sample-w/".into(),
            user: Some("sample-viewer".into()),
            password: Some("sample-password-not-a-secret".into()),
        };
        let f = share_fields(&public);
        let names: Vec<&str> = f.iter().map(|x| x.label).collect();
        assert_eq!(names, ["Link", "User", "Password"]);
        // The link shown and sent is the one that asks for the user and password.
        assert_eq!((f[0].shown.as_str(), f[0].copied.as_str()), (public.link.as_str(), public.link.as_str()));
        assert!(!f[0].shown.contains("token"));
        assert_eq!((f[1].copied.as_str(), f[1].secret), ("sample-viewer", false));
        assert_eq!((f[2].copied.as_str(), f[2].secret), ("sample-password-not-a-secret", true));
        // The QR code is the one-tap link, its key in the fragment.
        assert_eq!(qr_text(&public), public.open_link);
        let private = StreamShare {
            mode: StreamMode::Private,
            open_link: "https://desk.sample-tailnet.invalid:48300/ws-sample-notes/#token=k".into(),
            link: "https://desk.sample-tailnet.invalid:48300/ws-sample-notes/".into(),
            user: None,
            password: None,
        };
        let f = share_fields(&private);
        assert_eq!(f.len(), 1);
        // A private link only opens by its key: that's what's copied (the field shows it without).
        assert_eq!((f[0].shown.as_str(), f[0].copied.as_str()), (private.link.as_str(), private.open_link.as_str()));
        assert_eq!(share_text(StreamMode::Private), "Scan it on a device on your tailnet: it opens signed in as you.");
        assert_eq!(
            share_text(StreamMode::PublicSecure),
            "Scan it to open it signed in, or send the link with its user and password."
        );
        assert_eq!(mode_pill(StreamMode::Public, Transport::WebrtcRelay), "Public · WebRTC (relayed)");
        assert_eq!(mode_pill(StreamMode::Private, Transport::Websocket), "Private · WebSocket");
        assert_eq!(share_title("WadOS"), "WadOS is streaming");
    }

    #[test]
    fn the_status_line_says_where_from_who_watches_and_since_when() {
        let now = 1_000_000_000;
        let s = stream("w", StreamMode::Public, true);
        assert_eq!(status_line(&s, None, now), "Streaming · 2 watching · since 25 min ago");
        assert_eq!(status_line(&s, Some("Desk"), now), "Streaming from Desk · 2 watching · since 25 min ago");
        let starting = stream("notes", StreamMode::Private, false);
        assert_eq!(status_line(&starting, Some("Desk"), now), "Starting from Desk…");
    }

    #[test]
    fn badges_and_the_flyouts_lines() {
        let live = stream("w", StreamMode::Public, true);
        let starting = stream("notes", StreamMode::Private, false);
        assert_eq!(card_badge(&live), "LIVE · 2");
        assert_eq!(card_badge(&starting), "Starting stream");
        assert_eq!(flyout_sub(&live, None), "Public · 2 watching");
        assert_eq!(flyout_sub(&starting, Some("Desk")), "Private · starting · on Desk");
        assert_eq!(tile_sub(&[live.clone(), starting.clone()]), "1 live");
        assert_eq!(tile_sub(&[]), "0 live");
    }

    #[test]
    fn a_turn_down_warning_comes_first_once() {
        let mut s = stream("w", StreamMode::Public, true);
        assert!(warnings(&s).is_empty());
        s.turn_down = true;
        assert_eq!(warnings(&s), [TURN_DOWN]);
        s.warnings = vec!["Other".into(), TURN_DOWN.into()];
        assert_eq!(warnings(&s), ["Other", TURN_DOWN]);
    }

    #[test]
    fn the_toasts_say_live_where_and_failed_or_stopped() {
        let s = stream("w", StreamMode::Public, true);
        assert_eq!(live_toast("WadOS", &s), ("Streaming WadOS".into(), "Public · sample-host.invalid".into()));
        let p = stream("n", StreamMode::Private, true);
        assert_eq!(live_toast("Notes", &p).1, "Private · desk.sample-tailnet.invalid:48300");
        assert_eq!(failed_title("WadOS"), "WadOS didn’t start streaming");
        assert_eq!(stopped_title("WadOS"), "WadOS stopped streaming");
        assert_eq!(host_of("no-scheme/x"), "no-scheme");
    }

    #[test]
    fn what_changed_between_two_lists() {
        let key = |s: &StreamInfo| s.id.clone();
        let starting = stream("a", StreamMode::Private, false);
        let live = stream("a", StreamMode::Private, true);
        let other = stream("b", StreamMode::Public, true);
        // Starting → live; one live from the start isn't news of this list's.
        let t = turns(&[starting.clone(), other.clone()], &[live.clone(), other.clone()], key);
        assert_eq!(t.iter().map(|(k, t, _)| (k.as_str(), *t)).collect::<Vec<_>>(), [("a", Turn::Live)]);
        // Live → ended (or gone).
        let mut ended = live.clone();
        ended.stage = StreamStage::Ended;
        ended.live = false;
        let t = turns(std::slice::from_ref(&live), &[ended], key);
        assert_eq!(t[0].1, Turn::Ended { was_live: true });
        // Starting → gone: it didn't start (or was stopped as it started).
        let t = turns(&[starting], &[], key);
        assert_eq!(t[0].1, Turn::Ended { was_live: false });
        assert!(turns(std::slice::from_ref(&other), std::slice::from_ref(&other), key).is_empty());
    }
}
