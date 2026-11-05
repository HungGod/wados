//! The machine's Cloudflare tunnel, as public streams need it: cloudflared
//! (cloudflared.service, the image's) carries `https://<host>` from
//! Cloudflare to wadd's proxy (`[streaming] listen`, 127.0.0.1:8085), on a
//! machine the internet can't otherwise reach. The installer stick sets it
//! up (phase 9): the host in `/etc/wados/wadd.toml`, the token in
//! `/etc/cloudflared/token`.
//!
//! Public streams are available ([`Tunnel::status`]) only when all of it is
//! there, checked in this order, each with why not:
//! 1. `[streaming] host` set; else [`PUBLIC_NEEDS_TUNNEL`];
//! 2. the token file present; else [`PUBLIC_NEEDS_TUNNEL`];
//! 3. cloudflared.service active; else [`TUNNEL_STOPPED`];
//! 4. cloudflared's metrics `/ready` (`[streaming] metrics`) answering 200
//!    with a connection to Cloudflare; else [`TUNNEL_DISCONNECTED`].
//!
//! **Tests** (a quick tunnel, `cloudflared tunnel --url
//! http://localhost:8085`, which the harness runs, never wadd): with
//! `[streaming] tunnel = "external"`, a set host is all it takes. Never on
//! a real machine.
//!
//! wadd only looks: it never reads the token's contents, logs it, or
//! talks to Cloudflare here.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use wad_proto::streams::PUBLIC_NEEDS_TUNNEL;
use wad_proto::{Availability, StreamsAvailability};
use wad_systemd::Systemd;

/// The tunnel's token (cloudflared.service's `--token-file`).
pub const TOKEN_FILE: &str = "/etc/cloudflared/token";
pub const UNIT: &str = "cloudflared.service";
/// Why public can't start, past [`PUBLIC_NEEDS_TUNNEL`]: the tunnel is set
/// up, but cloudflared isn't running, or isn't connected to Cloudflare.
pub const TUNNEL_STOPPED: &str = "This machine's Cloudflare tunnel isn't running";
pub const TUNNEL_DISCONNECTED: &str = "This machine's Cloudflare tunnel isn't connected";
/// `[streaming] tunnel`: tests only (see the module's doc).
pub const EXTERNAL: &str = "external";
const READY_TIMEOUT: Duration = Duration::from_secs(2);

/// Whether public streams can start here, and why not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelStatus {
    pub available: bool,
    /// Why not, as the person reads it (the public buttons' tooltip).
    pub reason: Option<String>,
}

impl TunnelStatus {
    fn yes() -> Self {
        Self { available: true, reason: None }
    }

    fn no(reason: &str) -> Self {
        Self { available: false, reason: Some(reason.into()) }
    }

    /// As wad-proto's [`Availability`].
    pub fn availability(&self) -> Availability {
        match (&self.reason, self.available) {
            (_, true) => Availability::yes(),
            (Some(r), false) => Availability::no(r.clone()),
            (None, false) => Availability::no(PUBLIC_NEEDS_TUNNEL),
        }
    }

    /// Its reason on the public modes of `a` (made by
    /// [`StreamsAvailability::of`] with `tunnel: self.available`): a more
    /// specific one than [`PUBLIC_NEEDS_TUNNEL`]. A secure mode without its
    /// VM runtime still says that first.
    pub fn apply(&self, a: &mut StreamsAvailability) {
        if self.available {
            return;
        }
        for m in [&mut a.public, &mut a.public_secure] {
            if m.available || m.reason.as_deref() == Some(PUBLIC_NEEDS_TUNNEL) {
                *m = self.availability();
            }
        }
    }
}

/// How cloudflared's unit is looked at: systemd's (the system manager), or
/// a fixed state (tests).
enum Units {
    System(tokio::sync::OnceCell<Option<Systemd>>),
    #[cfg(test)]
    Fixed(&'static str),
}

pub struct Tunnel {
    host: String,
    external: bool,
    metrics: String,
    token_file: PathBuf,
    units: Units,
    http: reqwest::Client,
}

impl Tunnel {
    pub fn new(s: &wad_config::Streaming) -> Self {
        let external = s.tunnel == EXTERNAL;
        if external && !s.host.is_empty() {
            tracing::warn!(
                "[streaming] tunnel = \"external\": public streams at {} without checking a tunnel (tests only)",
                s.host
            );
        }
        Self {
            host: s.host.clone(),
            external,
            metrics: s.metrics.clone(),
            token_file: TOKEN_FILE.into(),
            units: Units::System(tokio::sync::OnceCell::new()),
            http: reqwest::Client::builder().no_proxy().timeout(READY_TIMEOUT).build().expect("an HTTP client"),
        }
    }

    /// Another token file than [`TOKEN_FILE`] (tests).
    pub fn with_token_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.token_file = path.into();
        self
    }

    /// The public host (`[streaming] host`), for links: empty, none.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Whether public streams can start now, and why not. It asks systemd
    /// and cloudflared's metrics each time (both local, quick).
    pub async fn status(&self) -> TunnelStatus {
        if self.host.is_empty() {
            return TunnelStatus::no(PUBLIC_NEEDS_TUNNEL);
        }
        if self.external {
            return TunnelStatus::yes();
        }
        if !present(&self.token_file) {
            return TunnelStatus::no(PUBLIC_NEEDS_TUNNEL);
        }
        match self.unit_state().await.as_deref() {
            Some("active") => {}
            // On its way: up, but not yet connected.
            Some("activating" | "reloading") => return TunnelStatus::no(TUNNEL_DISCONNECTED),
            _ => return TunnelStatus::no(TUNNEL_STOPPED),
        }
        match self.connected().await {
            true => TunnelStatus::yes(),
            false => TunnelStatus::no(TUNNEL_DISCONNECTED),
        }
    }

    /// cloudflared.service's ActiveState; none: systemd didn't say.
    async fn unit_state(&self) -> Option<String> {
        match &self.units {
            Units::System(cell) => {
                let systemd = cell
                    .get_or_init(|| async {
                        Systemd::system().await.inspect_err(|e| tracing::debug!("the system manager: {e}")).ok()
                    })
                    .await
                    .as_ref()?;
                systemd.active_state(UNIT).await.inspect_err(|e| tracing::debug!("{UNIT}: {e}")).ok()
            }
            #[cfg(test)]
            Units::Fixed(s) => Some(s.to_string()),
        }
    }

    /// cloudflared's `/ready`: 200, with a connection to Cloudflare.
    async fn connected(&self) -> bool {
        let Ok(r) = self.http.get(format!("http://{}/ready", self.metrics)).send().await else { return false };
        if !r.status().is_success() {
            return false;
        }
        // {"status":200,"readyConnections":4,"connectorId":"…"}; a 200 without the count is ready too.
        let v: Value = r.json().await.unwrap_or_default();
        v.get("readyConnections").and_then(Value::as_u64).is_none_or(|n| n > 0)
    }
}

/// The token's there (its contents aren't wadd's to read).
fn present(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn tunnel(host: &str, metrics: &str, token: &Path, unit: &'static str) -> Tunnel {
        let s = wad_config::Streaming { host: host.into(), metrics: metrics.into(), ..Default::default() };
        let mut t = Tunnel::new(&s).with_token_file(token);
        t.units = Units::Fixed(unit);
        t
    }

    async fn metrics(status: u16, body: serde_json::Value) -> MockServer {
        let m = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ready"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&m)
            .await;
        m
    }

    #[tokio::test]
    async fn public_needs_the_host_the_token_the_unit_and_a_connection() {
        let dir = tempfile::tempdir().unwrap();
        let token = dir.path().join("token");
        let up = metrics(200, serde_json::json!({"status": 200, "readyConnections": 4})).await;
        let addr = up.address().to_string();
        let reason = |s: TunnelStatus| s.reason.unwrap_or_default();

        // No host, then no token: the installer's to set up.
        assert_eq!(reason(tunnel("", &addr, &token, "active").status().await), PUBLIC_NEEDS_TUNNEL);
        assert_eq!(reason(tunnel("h.example", &addr, &token, "active").status().await), PUBLIC_NEEDS_TUNNEL);
        std::fs::write(&token, "not-read").unwrap();
        // cloudflared stopped, or starting.
        assert_eq!(reason(tunnel("h.example", &addr, &token, "failed").status().await), TUNNEL_STOPPED);
        assert_eq!(reason(tunnel("h.example", &addr, &token, "inactive").status().await), TUNNEL_STOPPED);
        assert_eq!(reason(tunnel("h.example", &addr, &token, "activating").status().await), TUNNEL_DISCONNECTED);
        // Running and connected.
        assert_eq!(tunnel("h.example", &addr, &token, "active").status().await, TunnelStatus::yes());

        // Running, but not connected: /ready says 503, or no connections, or nothing answers.
        let down = metrics(503, serde_json::json!({"status": 503, "readyConnections": 0})).await;
        let s = tunnel("h.example", &down.address().to_string(), &token, "active").status().await;
        assert_eq!(reason(s), TUNNEL_DISCONNECTED);
        let none = metrics(200, serde_json::json!({"status": 200, "readyConnections": 0})).await;
        let s = tunnel("h.example", &none.address().to_string(), &token, "active").status().await;
        assert_eq!(reason(s), TUNNEL_DISCONNECTED);
        let gone = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().to_string();
        assert_eq!(reason(tunnel("h.example", &gone, &token, "active").status().await), TUNNEL_DISCONNECTED);
    }

    #[tokio::test]
    async fn a_quick_tunnel_needs_only_the_host() {
        let s = wad_config::Streaming {
            host: "random-words.trycloudflare.com".into(),
            tunnel: EXTERNAL.into(),
            metrics: "127.0.0.1:9".into(),
            ..Default::default()
        };
        let t = Tunnel::new(&s).with_token_file("/nonexistent/token");
        assert_eq!(t.status().await, TunnelStatus::yes());
        assert_eq!(t.host(), "random-words.trycloudflare.com");
        let no_host = Tunnel::new(&wad_config::Streaming { host: String::new(), ..s });
        assert_eq!(no_host.status().await.reason.as_deref(), Some(PUBLIC_NEEDS_TUNNEL));
    }

    #[test]
    fn its_reason_goes_on_the_public_modes() {
        use wad_proto::streams::SECURE_NEEDS_VM;
        let down = TunnelStatus::no(TUNNEL_DISCONNECTED);
        let mut a = StreamsAvailability::of(true, false, true);
        down.apply(&mut a);
        assert_eq!(a.public.reason.as_deref(), Some(TUNNEL_DISCONNECTED));
        assert_eq!(a.public_secure.reason.as_deref(), Some(TUNNEL_DISCONNECTED));
        assert_eq!(a.private, Availability::yes());
        // No VM: secure says so first.
        let mut a = StreamsAvailability::of(true, false, false);
        down.apply(&mut a);
        assert_eq!(a.public_secure.reason.as_deref(), Some(SECURE_NEEDS_VM));
        assert_eq!(a.public.reason.as_deref(), Some(TUNNEL_DISCONNECTED));
        // Up: nothing changes.
        let mut a = StreamsAvailability::of(true, true, true);
        TunnelStatus::yes().apply(&mut a);
        assert_eq!(a, StreamsAvailability::of(true, true, true));
    }
}
