//! Streams: a wadspace shown in a browser, privately over the tailnet
//! (Selkies in WebSocket mode, Tailscale's HTTPS) or publicly at the
//! machine's host (WebRTC with Cloudflare's TURN, through its tunnel),
//! either one secure (in a VM of its own, libkrun). One stream per
//! wadspace: a stream is known by its wadspace's id.
//!
//! Who serves what (phase 7; wadd's `streams.rs` and `proxy.rs`):
//!
//! - **The socket** (`/run/wadd/wadd.sock`), for the shell and the apps:
//!   `GET /v1/streams` → [`StreamList`] (the streams here, and which modes
//!   can start: also the `streams` event); `POST /v1/wadspaces/{id}/stream`
//!   ([`StreamRequest`]) → a Stream job (`JobInfo`, its stages
//!   [`stages`]); `DELETE /v1/wadspaces/{id}/stream` → the [`StreamInfo`]
//!   as it ended; `GET /v1/streams/{id}/share` → [`StreamShare`].
//! - **The remote API** (the tailnet's addresses, port 8087), for my
//!   machines and trusted devices: `POST /v1/remote/wadspaces/{id}/stream`
//!   ([`StreamRequest`]) → [`StreamShare`] once it's live (it waits for
//!   the job), and `DELETE /v1/remote/wadspaces/{id}/stream` → the
//!   [`StreamInfo`] as it ended. Anyone else gets 403, as on the rest of it.
//! - **The proxy** (`127.0.0.1:8085`, cloudflared's origin; a private
//!   stream's own listener, `[streaming] first_private_port` on, which
//!   `tailscale serve --https=<port>` publishes): `/ws-<guid>/…` to that
//!   stream's sidecar. Not wadd's API: nothing here crosses it.
//!
//! **Secrets.** A stream's key (Selkies' token, in the one-tap link's
//! fragment) and a public stream's password only ever travel in
//! [`StreamShare`], which only the machine's own socket and its remote API's
//! trusted callers (my machines, trusted devices) get. They're never in
//! [`StreamInfo`], events, jobs, logs (its `Debug` leaves them out) or
//! `streams.json` (the keys are podman secrets).

use std::fmt;

use serde::{Deserialize, Serialize};
/// What streams are made of (wad_model's), for callers that only depend on wad-proto.
pub use wad_model::{StreamMode, StreamStage, Transport};

/// Why a mode can't start, as the person reads it (the button's tooltip).
pub const PRIVATE_NEEDS_TAILNET: &str = "Join your tailnet first: private streams only open on it";
/// Why a stream can't be Play together's (the spike's).
pub const PLAYS_PUBLIC: &str = "Only a gamespace’s public stream can be multiplayer (Dolphin or Steam)";
pub const MAX_PLAYERS: u8 = 4;

pub const PUBLIC_NEEDS_TUNNEL: &str =
    "Public streams need this machine's Cloudflare tunnel, which the installer stick sets up";
pub const SECURE_NEEDS_VM: &str = "Secure streams need a VM runtime this machine doesn't have";
/// Public secure streams, always for now: WebRTC's UDP can't reach into the VM yet.
pub const SECURE_PUBLIC_LATER: &str = "Secure public streams come later: secure streams are private for now";

/// A public stream's warning when Cloudflare's TURN didn't answer ([`StreamInfo::turn_down`]).
pub const TURN_DOWN: &str = "Cloudflare's TURN didn't answer: only viewers with a direct path can connect (no relay)";

/// A stream as the shell and the phone show it (`GET /v1/streams`, the
/// `streams` event). Never its key or password: [`StreamShare`] has those.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    /// The wadspace it streams (another machine's, with `peer`).
    pub id: String,
    pub mode: StreamMode,
    /// How viewers get it: WebSocket (private), WebRTC (public), or WebRTC
    /// relayed through Cloudflare's TURN (a viewer without a direct path).
    pub transport: Transport,
    /// Its Stream job's stage while it starts, then live, then ended.
    pub stage: StreamStage,
    /// The proxy routes to it: its link opens.
    pub live: bool,
    /// Viewers watching now.
    pub viewers: u32,
    /// When it started, and when someone last watched or touched it (ms since the epoch).
    pub started: i64,
    pub last_active: i64,
    /// Its link without its key: `https://<host>/ws-<guid>/` (public), or
    /// `https://<machine>.<tailnet>:<port>/ws-<guid>/` (private).
    pub link: String,
    /// Public, and Cloudflare's TURN didn't answer: only viewers with a
    /// direct path connect ([`TURN_DOWN`] is among its warnings).
    #[serde(default)]
    pub turn_down: bool,
    /// What to tell the person about it, as they read it.
    #[serde(default)]
    pub warnings: Vec<String>,
    /// The Stream job starting it, while wadd has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// Another machine of mine it streams from (its wadd machine id: "Streaming
    /// from {peer}", named by `GET /v1/peers`); none: this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
}

/// What to hand on with a stream (`GET /v1/streams/{id}/share`; the remote
/// API's start): the share sheet's QR code, its link, and a public stream's
/// sign-in. The only type with a stream's secrets: its `Debug` leaves them out.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamShare {
    pub mode: StreamMode,
    /// The one-tap link, signed in by its key: `https://<host>/ws-<guid>/#token=<key>`
    /// (private: `https://<machine>.<tailnet>:<port>/ws-<guid>/#token=<key>`).
    /// The key is in the fragment, so it never reaches a server. The QR code's.
    pub open_link: String,
    /// The link without the key ([`StreamInfo::link`]): it asks for the user and password.
    pub link: String,
    /// A public stream's sign-in at its link (wadd's proxy asks for it); none for a private one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Play together's links, player 1 first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub players: Vec<PlayerLink>,
}

// Never the key (the one-tap link) nor the password in a log line.
impl fmt::Debug for StreamShare {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamShare")
            .field("mode", &self.mode)
            .field("link", &self.link)
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

/// `POST /v1/wadspaces/{id}/stream` (and the remote API's).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamRequest {
    pub mode: StreamMode,
    /// It's on the screen (or in a window): take it off the screen to
    /// stream it, as the person confirmed. Without it, that's a conflict (409).
    #[serde(default)]
    pub takeover: bool,
    /// Play together: 2 to 4 players, each with a link of their own and a
    /// gamepad. 0: an ordinary stream. Only a public stream of a wadspace
    /// with Dolphin or Steam.
    #[serde(default, skip_serializing_if = "no_players")]
    pub players: u8,
}

fn no_players(n: &u8) -> bool {
    *n == 0
}

/// A player's link: who opens it plays as that player.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerLink {
    pub slot: u8,
    /// Signed in by that player's own key.
    pub open_link: String,
}

impl fmt::Debug for PlayerLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlayerLink").field("slot", &self.slot).finish_non_exhaustive()
    }
}

/// Whether one mode can start on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Availability {
    pub available: bool,
    /// Why not, as the person reads it (the disabled button's tooltip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Availability {
    pub fn yes() -> Self {
        Self { available: true, reason: None }
    }

    pub fn no(reason: impl Into<String>) -> Self {
        Self { available: false, reason: Some(reason.into()) }
    }
}

/// Which modes can start on this machine, each with why not. Keyed by the
/// mode as it's written ('private-secure'), so `available[mode]` reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct StreamsAvailability {
    pub private: Availability,
    pub private_secure: Availability,
    pub public: Availability,
    pub public_secure: Availability,
}

impl StreamsAvailability {
    /// From what the machine has: the tailnet running (private), a public
    /// host with its Cloudflare tunnel connected (public), a VM runtime
    /// (secure). A secure mode without the runtime says so first.
    pub fn of(tailnet: bool, tunnel: bool, vm: bool) -> Self {
        let mode = |public: bool, secure: bool| {
            if secure && !vm {
                Availability::no(SECURE_NEEDS_VM)
            } else if public && !tunnel {
                Availability::no(PUBLIC_NEEDS_TUNNEL)
            } else if !public && !tailnet {
                Availability::no(PRIVATE_NEEDS_TAILNET)
            } else {
                Availability::yes()
            }
        };
        Self {
            private: mode(false, false),
            private_secure: mode(false, true),
            public: mode(true, false),
            public_secure: mode(true, true),
        }
    }

    pub fn get(&self, mode: StreamMode) -> &Availability {
        match mode {
            StreamMode::Private => &self.private,
            StreamMode::PrivateSecure => &self.private_secure,
            StreamMode::Public => &self.public,
            StreamMode::PublicSecure => &self.public_secure,
        }
    }
}

/// `GET /v1/streams` (and the `streams` event, the same fields): the
/// streams here, newest last, and which modes can start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamList {
    pub streams: Vec<StreamInfo>,
    pub available: StreamsAvailability,
}

impl From<StreamList> for crate::Event {
    fn from(l: StreamList) -> Self {
        crate::Event::Streams { streams: l.streams, available: l.available }
    }
}

/// A Stream job's stages, in order: each its [`StreamStage`] (the job's
/// stage id is its name, [`StreamStage::as_str`]) and its label (the
/// spike's, without SWAG or Kata). `container`: the wadspace has to be
/// started (or started again: off the screen, or into its VM) first.
pub fn stages(mode: StreamMode, container: bool) -> Vec<(StreamStage, &'static str)> {
    let mut out = vec![];
    if mode.is_secure() {
        out.push((StreamStage::Vm, "Starting its VM"));
    }
    if container {
        out.push((StreamStage::Container, "Starting its container"));
    }
    out.push((StreamStage::Sidecar, "Starting the Selkies sidecar"));
    if mode.is_public() {
        out.push((StreamStage::Route, "Routing it through Cloudflare"));
        out.push((StreamStage::Turn, "Checking Cloudflare's STUN and TURN"));
    } else {
        out.push((StreamStage::Serve, "Serving it on your tailnet"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn info() -> StreamInfo {
        StreamInfo {
            id: "studio".into(),
            mode: StreamMode::PublicSecure,
            transport: Transport::WebrtcRelay,
            stage: StreamStage::Live,
            live: true,
            viewers: 2,
            started: 10,
            last_active: 20,
            link: "https://surface.wadspaces.com/ws-0123/".into(),
            turn_down: false,
            warnings: vec![],
            job: None,
            peer: None,
        }
    }

    #[test]
    fn a_stream_reads_as_the_spikes_without_its_secrets() {
        let v = serde_json::to_value(info()).unwrap();
        assert_eq!(
            (v["mode"].as_str(), v["transport"].as_str(), v["stage"].as_str()),
            (Some("public-secure"), Some("webrtc-relay"), Some("live"))
        );
        assert_eq!((v["lastActive"].as_i64(), v["turnDown"].as_bool()), (Some(20), Some(false)));
        assert!(v.get("job").is_none() && v.get("peer").is_none(), "{v}");
        for secret in ["key", "token", "password", "user", "openLink"] {
            assert!(v.get(secret).is_none(), "{secret} in {v}");
        }
        assert_eq!(serde_json::from_value::<StreamInfo>(v).unwrap(), info());
        // The spike's name for a VM's stage still reads.
        let mut v = serde_json::to_value(info()).unwrap();
        v["stage"] = json!("kata");
        assert_eq!(serde_json::from_value::<StreamInfo>(v).unwrap().stage, StreamStage::Vm);
        assert_eq!(serde_json::to_value(StreamStage::Vm).unwrap(), StreamStage::Vm.as_str());
    }

    #[test]
    fn a_share_never_logs_its_key_or_password() {
        let s = StreamShare {
            mode: StreamMode::Public,
            open_link: "https://h.example/ws-0123/#token=KEY-not-for-logs".into(),
            link: "https://h.example/ws-0123/".into(),
            user: Some("viewer".into()),
            password: Some("PASSWORD-not-for-logs".into()),
            players: vec![],
        };
        let debug = format!("{s:?}");
        assert!(!debug.contains("KEY-not-for-logs") && !debug.contains("PASSWORD-not-for-logs"), "{debug}");
        assert!(debug.contains("https://h.example/ws-0123/") && debug.contains("viewer"), "{debug}");
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["openLink"], "https://h.example/ws-0123/#token=KEY-not-for-logs");
        assert_eq!(serde_json::from_value::<StreamShare>(v).unwrap(), s);
        // A private stream's has no sign-in.
        let private: StreamShare = serde_json::from_value(json!({
            "mode": "private", "openLink": "https://desk.ts.test:48300/ws-1/#token=k", "link": "https://desk.ts.test:48300/ws-1/",
        }))
        .unwrap();
        assert_eq!((private.user, private.password), (None, None));
    }

    #[test]
    fn a_request_takes_it_off_the_screen_only_when_asked() {
        let r: StreamRequest = serde_json::from_value(json!({"mode": "private"})).unwrap();
        assert_eq!(r, StreamRequest { mode: StreamMode::Private, takeover: false, players: 0 });
        assert!(serde_json::from_value::<StreamRequest>(json!({"takeover": true})).is_err(), "a mode is needed");
    }

    #[test]
    fn each_mode_says_why_it_cant_start() {
        let all = StreamsAvailability::of(true, true, true);
        for m in [StreamMode::Private, StreamMode::PrivateSecure, StreamMode::Public, StreamMode::PublicSecure] {
            assert_eq!(all.get(m), &Availability::yes(), "{m:?}");
        }
        let none = StreamsAvailability::of(false, false, false);
        assert_eq!(none.private.reason.as_deref(), Some(PRIVATE_NEEDS_TAILNET));
        assert_eq!(none.public.reason.as_deref(), Some(PUBLIC_NEEDS_TUNNEL));
        assert_eq!(none.get(StreamMode::PrivateSecure).reason.as_deref(), Some(SECURE_NEEDS_VM));
        assert_eq!(none.public_secure.reason.as_deref(), Some(SECURE_NEEDS_VM));
        // Keyed as the modes are written; a reason only when it's not available.
        let v = serde_json::to_value(StreamsAvailability::of(true, true, false)).unwrap();
        assert_eq!(v["public"], json!({"available": true}));
        assert_eq!(v["private-secure"], json!({"available": false, "reason": SECURE_NEEDS_VM}));
        assert_eq!(
            serde_json::from_value::<StreamsAvailability>(v).unwrap(),
            StreamsAvailability::of(true, true, false)
        );
    }

    #[test]
    fn a_stream_jobs_stages() {
        let ids = |m, c| stages(m, c).into_iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>();
        assert_eq!(ids(StreamMode::Private, false), ["sidecar", "serve"]);
        assert_eq!(ids(StreamMode::Public, true), ["container", "sidecar", "route", "turn"]);
        assert_eq!(ids(StreamMode::PublicSecure, true), ["vm", "container", "sidecar", "route", "turn"]);
        let labels: Vec<&str> = stages(StreamMode::Public, false).into_iter().map(|(_, l)| l).collect();
        assert_eq!(
            labels,
            ["Starting the Selkies sidecar", "Routing it through Cloudflare", "Checking Cloudflare's STUN and TURN"]
        );
        for (_, label) in stages(StreamMode::PublicSecure, true) {
            assert!(!label.contains("SWAG") && !label.contains("Kata"), "{label}");
        }
    }
}
