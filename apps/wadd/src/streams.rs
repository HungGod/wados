//! Streams (v1's streams.rs, on Selkies 2.x): a wadspace shown in a browser,
//! privately over the tailnet or publicly at the machine's host, through
//! wadd's proxy (proxy.rs) to the wadspace's stream sidecar (images/stream),
//! which the wadspace draws on instead of the screen. [`MachineStreams`] is
//! wadd's [`Streams`] (the socket's routes in api_phase7.rs, the remote
//! API's in remote.rs).
//!
//! **Starting one** (`POST /v1/wadspaces/{id}/stream`) is a Stream job with
//! [`stages`], one start or end at a time per wadspace:
//!
//! 1. what can start: the mode's [`StreamsAvailability`] (private: the
//!    tailnet running; public: the machine's host and its Cloudflare tunnel;
//!    secure: a VM runtime here), the wadspace built, the sidecar's image here;
//! 2. on the screen (or in a window or the background): 409, unless
//!    `takeover`, the person's confirm. Then it moves to its stream as it
//!    runs, as v1's did (runs.rs's [`Runs::switch`]: `wadspaces-session
//!    switch`), when it was opened to move (a moving image, on a machine
//!    with a screen); else it's stopped and started again on its stream.
//! 3. its secrets, new each time: a guid (128 bits: `/ws-<guid>/`), its key
//!    (Selkies' token: the one-tap link's `#token=`), Selkies' master token,
//!    and a public stream's user and password ([`new_user`], [`new_password`]).
//!    The key, master token and password are podman secrets
//!    ([`key_secret`], [`units::master_secret`], [`password_secret`]), and
//!    nowhere else but the sidecar's folder; never in `streams.json`, units,
//!    events, jobs or logs;
//! 4. the sidecar's folder, `<state>/streams/<id>/` (0700, files 0600, the
//!    machine's user's: Selkies refuses a file another user could write):
//!    its token table (one controller row: the key) and, public, its ICE
//!    servers (turn.rs: Cloudflare's TURN, minted for it);
//! 5. its units (runs.rs: the wadspace, its sidecar, their display volume),
//!    the wadspace started streamed, its desktop up; the sidecar answering;
//! 6. the proxy's route (`/ws-<guid>/`); private, `tailscale serve
//!    --https=<port>` to the stream's own listener;
//! 7. live, and the `streams` event.
//!
//! **Secure** (private only, for now): the wadspace runs in a VM of its own
//! (libkrun: `[streaming] secure_runtime`, with `/dev/kvm`), always started
//! again (never moved), with its stream's Selkies inside it instead of a
//! sidecar beside it (`wad_gen::units::secure_units`, runs.rs). It's ready
//! when Selkies' health answers on the port the VM publishes (nothing execs
//! into a VM), and its podman stop is a SIGINT that its s6 stops cleanly on
//! (30 s). A wadspace with a folder from this machine (a Mount project)
//! isn't streamed securely: that folder turns off SELinux's separation,
//! which the VM relies on. Public secure streams come later (WebRTC's UDP
//! can't reach into the VM yet).
//!
//! **It ends** when it's stopped; when its wadspace stops; when the person
//! at the machine opens it on the screen (they win: runs.rs's open asks,
//! [`StreamEnds`]; one that moves goes back on the screen as it runs, else
//! it stops and opens there); or when nobody has watched it (the proxy's
//! activity) for `idle_minutes`. Ending takes everything away: the route (its links get the
//! expired page), the tailnet's serve, the wadspace and its sidecar, their
//! units and volume, the secrets, the folder, and its TURN credentials
//! (revoked).
//!
//! **Across restarts**: `streams.json` (0600) keeps what a stream is without
//! its secrets. At start ([`MachineStreams::adopt`]) the ones whose
//! wadspace and sidecar still run are routed again (their key from podman),
//! the rest end; secrets, folders, sidecars and tailnet serves no stream has
//! are taken away.
//!
//! **Another machine's** ("Stream from {machine}"): a stream of one of my
//! machines' wadspaces, started there through its remote API
//! ([`PeerStreams`], peers.rs) and opened here in a WadBrowser window. It's
//! in the list with `peer` set, known by (peer, wadspace) since ids are
//! per machine, and followed from the peer every few seconds.

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wad_gen::units::{self, container_name, display_name, master_secret};
use wad_model::{JobKind, PartState, Phase, ProjectKind, SessionDisplay, StreamMode, StreamStage, Transport, Wadspace};
use wad_proto::streams::{MAX_PLAYERS, PLAYS_PUBLIC, PlayerLink, SECURE_PUBLIC_LATER, TURN_DOWN, stages};
use wad_proto::{
    ApiError, Availability, ErrorCode, Event, JobInfo, Level, StreamInfo, StreamList, StreamRequest, StreamShare,
    StreamsAvailability,
};

use crate::api_phase7::Streams;
use crate::backend::{Backend, MISSING, RUNNING, drop_volume};
use crate::builds::Builds;
use crate::events::Bus;
use crate::jobs::{Jobs, now_ms};
use crate::library::Docs;
use crate::runs::{Runs, SecureVm, StreamEnds, StreamUnit};
use crate::turn::{self, Ice, Turn};

/// Every stream secret's name starts so (its wadspace's id after the kind).
const SECRET_PREFIX: &str = "wad_stream_";
/// An ended stream stays in the list this long (the shell shows it ended).
const ENDED_FOR: Duration = Duration::from_secs(5);
/// How often streams are checked (ended, watched, TURN renewed).
pub const TICK: Duration = Duration::from_secs(3);
/// How often another machine's streams here are asked after.
const PEER_EVERY: Duration = Duration::from_secs(10);
/// TURN that didn't answer is asked again after this.
const MINT_RETRY_MS: i64 = 60_000;
/// How long a stream's containers have to go before its display volume is removed.
const VOLUME_WAIT: Duration = Duration::from_secs(120);
/// Starting its wadspace and its desktop may take this much longer than `ready_timeout`.
const START_SLACK: Duration = Duration::from_secs(30);

/// A stream's key (Selkies' token: the one-tap link's), as a podman secret.
pub fn key_secret(id: &str) -> String {
    format!("{SECRET_PREFIX}key_{id}")
}

/// Play together's other players' keys (a JSON list, slot 2 first), as a podman secret.
pub fn players_secret(id: &str) -> String {
    format!("{SECRET_PREFIX}players_{id}")
}

/// A public stream's password, as a podman secret.
pub fn password_secret(id: &str) -> String {
    format!("{SECRET_PREFIX}password_{id}")
}

/// Selkies' token table: an ordinary stream's one controller, or Play
/// together's: player 1 at the controls, and the others in their own slots.
fn token_table(key: &str, others: &[String], players: u8) -> Value {
    if players == 0 {
        return json!({ key: { "role": "controller", "slot": null } });
    }
    let mut table = serde_json::Map::new();
    table.insert(key.to_string(), json!({ "role": "controller", "slot": 1 }));
    for (i, k) in others.iter().enumerate() {
        table.insert(k.clone(), json!({ "role": "viewer", "slot": i + 2 }));
    }
    Value::Object(table)
}

/// Which wadspace a stream secret is of, by its name.
fn secret_owner(name: &str) -> Option<&str> {
    let rest = name.strip_prefix(SECRET_PREFIX)?;
    ["master_", "key_", "password_", "players_"].iter().find_map(|k| rest.strip_prefix(k))
}

// ------------------------------------------------------------ secrets

/// `n` bytes from the kernel's randomness.
pub(crate) fn random_bytes(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    let mut got = 0;
    while got < n {
        // SAFETY: the buffer is ours, and `n - got` bytes from `got` are in it.
        let r = unsafe { libc::getrandom(b[got..].as_mut_ptr().cast(), n - got, 0) };
        if r < 0 {
            let e = std::io::Error::last_os_error();
            assert!(e.kind() == std::io::ErrorKind::Interrupted, "the system's randomness: {e}");
            continue;
        }
        got += r as usize;
    }
    b
}

/// `n` random bytes as lowercase hex.
pub fn random_hex(n: usize) -> String {
    random_bytes(n).iter().map(|x| format!("{x:02x}")).collect()
}

/// `len` characters from `alphabet`, uniformly (no modulo bias).
fn random_from(alphabet: &[u8], len: usize) -> String {
    let limit = 256 - 256 % alphabet.len();
    let mut out = String::with_capacity(len);
    while out.len() < len {
        for x in random_bytes(64).into_iter().map(usize::from).filter(|x| *x < limit) {
            if out.len() < len {
                out.push(alphabet[x % alphabet.len()] as char);
            }
        }
    }
    out
}

/// No 0/O, 1/l/I: easy to read out and type on a phone. 56 symbols, so 20
/// of them are about 116 bits.
const PASSWORD_ALPHABET: &[u8] = b"23456789abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ";

/// A public stream's password.
pub fn new_password() -> String {
    random_from(PASSWORD_ALPHABET, 20)
}

/// A public stream's user: `wad-` and six letters.
pub fn new_user() -> String {
    format!("wad-{}", random_from(b"abcdefghijkmnpqrstuvwxyz", 6))
}

// ------------------------------------------------------------ what streams work with

/// One stream's way in, as the proxy takes it.
pub use crate::proxy::Route;
use crate::proxy::RouteTable;
use crate::tunnel::Tunnel;

/// The proxy's route table: what it serves at `/ws-<guid>/`, and what it saw.
pub trait Routes: Send + Sync + 'static {
    /// Routes `guid` (a private one gets its own listener): why not, if it can't.
    fn add(&self, guid: &str, route: Route) -> Result<(), String>;
    fn remove(&self, guid: &str);
    /// When someone last watched or touched it (ms since the epoch), and how
    /// many watch now; none: it isn't routed.
    fn activity(&self, guid: &str) -> Option<(i64, u32)>;
    /// Changes whenever a viewer comes or goes (none: it doesn't say).
    fn viewers_changed(&self) -> Option<tokio::sync::watch::Receiver<u64>> {
        None
    }
}

/// The proxy's own (proxy.rs).
impl Routes for Arc<RouteTable> {
    fn add(&self, guid: &str, route: Route) -> Result<(), String> {
        RouteTable::add(self, guid, route).map_err(|e| e.to_string())
    }

    fn remove(&self, guid: &str) {
        RouteTable::remove(self, guid);
    }

    fn activity(&self, guid: &str) -> Option<(i64, u32)> {
        RouteTable::activity(self, guid)
    }

    fn viewers_changed(&self) -> Option<tokio::sync::watch::Receiver<u64>> {
        Some(self.subscribe())
    }
}

/// Routes in memory: for tests, and a machine without the proxy.
#[derive(Default)]
pub struct MemRoutes {
    routes: Mutex<HashMap<String, (Route, i64, u32)>>,
}

impl MemRoutes {
    pub fn get(&self, guid: &str) -> Option<Route> {
        self.routes.lock().unwrap().get(guid).map(|r| r.0.clone())
    }

    pub fn guids(&self) -> Vec<String> {
        self.routes.lock().unwrap().keys().cloned().collect()
    }

    /// As if viewers watched (or last did at `last`).
    pub fn set_activity(&self, guid: &str, last: i64, viewers: u32) {
        if let Some(r) = self.routes.lock().unwrap().get_mut(guid) {
            (r.1, r.2) = (last, viewers);
        }
    }
}

impl Routes for MemRoutes {
    fn add(&self, guid: &str, route: Route) -> Result<(), String> {
        let mut routes = self.routes.lock().unwrap();
        if routes.contains_key(guid) {
            return Err(format!("{guid} is routed already"));
        }
        routes.insert(guid.into(), (route, now_ms(), 0));
        Ok(())
    }

    fn remove(&self, guid: &str) {
        self.routes.lock().unwrap().remove(guid);
    }

    fn activity(&self, guid: &str) -> Option<(i64, u32)> {
        self.routes.lock().unwrap().get(guid).map(|r| (r.1, r.2))
    }
}

/// The tailnet, for private streams: `tailscale serve`.
#[async_trait]
pub trait PrivateServe: Send + Sync + 'static {
    /// The machine's MagicDNS name while the tailnet runs ('desk.tail1234.ts.net'); none: it isn't on it now.
    fn host(&self) -> Option<String>;
    /// `https://<host>:<port>/` on the tailnet, to `http://127.0.0.1:<port>` (the stream's listener).
    async fn serve(&self, port: u16) -> Result<(), String>;
    async fn unserve(&self, port: u16);
    /// The ports served now.
    async fn served(&self) -> Vec<u16>;
}

/// The machine's Tailscale (its CLI), for private streams.
pub struct TailscaleServe {
    cli: wad_tailscale::Cli,
    host: Box<dyn Fn() -> Option<String> + Send + Sync>,
}

impl TailscaleServe {
    pub fn new(cli: wad_tailscale::Cli, host: impl Fn() -> Option<String> + Send + Sync + 'static) -> Self {
        Self { cli, host: Box::new(host) }
    }

    /// The machine's tailnet: its name while it's running.
    pub fn of(config: &wad_config::Config, tailnet: Arc<crate::tailnet::Tailnet>) -> Self {
        let cli = wad_tailscale::Cli::new(&config.tailnet.cli, &config.tailnet.socket);
        Self::new(cli, move || {
            let s = tailnet.status().filter(|s| tailnet.enabled() && s.running())?;
            let dns = s.me?.dns().to_string();
            (!dns.is_empty()).then_some(dns)
        })
    }
}

#[async_trait]
impl PrivateServe for TailscaleServe {
    fn host(&self) -> Option<String> {
        (self.host)()
    }

    async fn serve(&self, port: u16) -> Result<(), String> {
        self.cli.serve(port, &format!("http://127.0.0.1:{port}")).await.map_err(|e| e.to_string())
    }

    async fn unserve(&self, port: u16) {
        if let Err(e) = self.cli.serve_off(port).await {
            tracing::info!("streams: the tailnet's serve of :{port}: {e}");
        }
    }

    async fn served(&self) -> Vec<u16> {
        self.cli.served().await.unwrap_or_default()
    }
}

/// My other machines' streams, through their remote API (peers.rs).
#[async_trait]
pub trait PeerStreams: Send + Sync + 'static {
    /// Streams `ws` there: its share once it's live.
    async fn start(&self, peer: &str, ws: &str, req: StreamRequest) -> Result<StreamShare, ApiError>;
    async fn stop(&self, peer: &str, ws: &str) -> Result<StreamInfo, ApiError>;
    /// Its stream there; none: it has none.
    async fn info(&self, peer: &str, ws: &str) -> Result<Option<StreamInfo>, ApiError>;
}

// ------------------------------------------------------------ settings

#[derive(Debug, Clone)]
pub struct Settings {
    /// `[streaming] enabled`.
    pub enabled: bool,
    pub state_dir: PathBuf,
    /// The sidecar's image.
    pub image: String,
    pub first_web_port: u16,
    pub first_mux_port: u16,
    pub first_private_port: u16,
    pub ports: u16,
    pub idle: Duration,
    /// How long its wadspace and sidecar may take to come up.
    pub ready_timeout: Duration,
    /// Between readiness checks.
    pub poll: Duration,
    /// The VM runtime secure streams run in, when it's truly here (and KVM).
    pub vm_runtime: Option<String>,
    /// A secure stream's VM: its vCPUs and memory (MiB).
    pub vm_cpus: u32,
    pub vm_ram_mib: u32,
    /// Who owns the sidecar's folder when wadd is root (abc's 1000); none: wadd's user.
    pub owner: Option<u32>,
    pub turn: turn::Settings,
}

impl Settings {
    pub fn new(config: &wad_config::Config) -> Self {
        Self::with_kvm(config, Path::new("/dev/kvm"))
    }

    /// With KVM's device at `kvm` (tests: a file, or none).
    pub fn with_kvm(config: &wad_config::Config, kvm: &Path) -> Self {
        let s = &config.streaming;
        let root = nix::unistd::geteuid().is_root();
        Self {
            enabled: s.enabled,
            state_dir: config.daemon.state_dir.clone(),
            image: s.image.clone(),
            first_web_port: s.first_web_port,
            first_mux_port: s.first_mux_port,
            first_private_port: s.first_private_port,
            ports: s.ports.max(1),
            idle: Duration::from_secs(s.idle_minutes.max(1) * 60),
            ready_timeout: Duration::from_secs(config.daemon.ready_timeout_s.max(1)),
            poll: Duration::from_secs(1),
            vm_runtime: vm_runtime(&s.secure_runtime, kvm),
            vm_cpus: s.secure_cpus.max(1),
            vm_ram_mib: s.secure_ram_mib.max(512),
            owner: root.then_some(config.daemon.projects_uid),
            turn: turn::Settings::new(config),
        }
    }
}

/// The OCI runtime `name` (`krun`) when it's here and so is KVM.
pub fn vm_runtime(name: &str, kvm: &Path) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || !kvm.exists() {
        return None;
    }
    let found = if name.contains('/') {
        Path::new(name).is_file()
    } else {
        ["/usr/bin", "/usr/local/bin", "/usr/sbin", "/usr/libexec/podman"]
            .iter()
            .any(|d| Path::new(d).join(name).is_file())
    };
    found.then(|| name.to_string())
}

/// What [`MachineStreams`] works with.
pub struct Parts {
    pub backend: Arc<dyn Backend>,
    pub docs: Arc<Docs>,
    pub bus: Bus,
    pub jobs: Arc<Jobs>,
    pub runs: Arc<Runs>,
    pub builds: Arc<Builds>,
    pub routes: Arc<dyn Routes>,
    pub private: Arc<dyn PrivateServe>,
    /// The Cloudflare tunnel, and the public host (tunnel.rs).
    pub tunnel: Arc<Tunnel>,
    /// My other machines (none: no tailnet, so no streams from them).
    pub peers: Option<Arc<dyn PeerStreams>>,
}

// ------------------------------------------------------------ state

/// A stream as `streams.json` keeps it: never its key, master token or password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    /// Its wadspace.
    id: String,
    guid: String,
    mode: StreamMode,
    web_port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mux_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    private_port: Option<u16>,
    /// A public stream's sign-in user (not a secret: the password is).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    started: i64,
    link: String,
    /// The TURN usernames minted for it (revoked when it ends).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    turn_users: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    job: Option<String>,
    /// It moved here from the screen as it ran, shown so (stopped, it moves back).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    moved_from: Option<SessionDisplay>,
    /// Play together's players, or 0.
    #[serde(default)]
    players: u8,
}

/// Another machine's stream here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PeerEntry {
    peer: String,
    id: String,
    mode: StreamMode,
    link: String,
    started: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    streams: Vec<Entry>,
    #[serde(default)]
    peers: Vec<PeerEntry>,
}

struct Live {
    e: Entry,
    info: StreamInfo,
    /// Its Stream job (or, from before a restart, adopt) hasn't finished.
    starting: bool,
    /// It's ending (or ended): nothing more happens to it.
    ending: bool,
    ice: Option<Ice>,
    /// When to mint its TURN credentials again (ms; 0: never, private).
    next_mint: i64,
}

struct PeerLive {
    e: PeerEntry,
    info: StreamInfo,
    ending: bool,
}

#[derive(Default)]
struct St {
    live: Vec<Live>,
    peers: Vec<PeerLive>,
    available: Option<StreamsAvailability>,
    published: Option<StreamList>,
    written: Vec<u8>,
    peers_checked: Option<std::time::Instant>,
    adopted: bool,
}

/// TURN being asked for a stream's credentials.
type Mint = tokio::task::JoinHandle<Result<Ice, String>>;

fn transport(mode: StreamMode) -> Transport {
    if mode.is_public() { Transport::Webrtc } else { Transport::Websocket }
}

fn info_of(e: &Entry, stage: StreamStage, live: bool) -> StreamInfo {
    StreamInfo {
        id: e.id.clone(),
        mode: e.mode,
        transport: transport(e.mode),
        stage,
        live,
        viewers: 0,
        started: e.started,
        last_active: e.started,
        link: e.link.clone(),
        turn_down: false,
        warnings: vec![],
        job: e.job.clone(),
        peer: None,
    }
}

fn peer_info(e: &PeerEntry, stage: StreamStage, live: bool) -> StreamInfo {
    StreamInfo {
        id: e.id.clone(),
        mode: e.mode,
        transport: transport(e.mode),
        stage,
        live,
        viewers: 0,
        started: e.started,
        last_active: e.started,
        link: e.link.clone(),
        turn_down: false,
        warnings: vec![],
        job: None,
        peer: Some(e.peer.clone()),
    }
}

fn conflict(m: impl Into<String>) -> ApiError {
    ApiError::new(ErrorCode::Conflict, m)
}

fn not_streaming(ws: &str) -> ApiError {
    ApiError::not_found(format!("{ws:?} isn't streaming"))
}

fn active(p: Phase) -> bool {
    matches!(p, Phase::Starting | Phase::Waiting | Phase::Ready | Phase::Stopping)
}

/// The machine's streams.
pub struct MachineStreams {
    s: Settings,
    backend: Arc<dyn Backend>,
    docs: Arc<Docs>,
    bus: Bus,
    jobs: Arc<Jobs>,
    runs: Arc<Runs>,
    builds: Arc<Builds>,
    routes: Arc<dyn Routes>,
    private: Arc<dyn PrivateServe>,
    tunnel: Arc<Tunnel>,
    peers: Option<Arc<dyn PeerStreams>>,
    turn: Turn,
    file: PathBuf,
    st: Mutex<St>,
    /// One start or end at a time, per wadspace (and per peer's wadspace).
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    me: Weak<Self>,
}

impl MachineStreams {
    /// The streams from before (`streams.json`): not routed until [`Self::adopt`] has looked at them.
    pub fn new(s: Settings, p: Parts) -> Arc<Self> {
        let file = s.state_dir.join("streams.json");
        let saved: Saved = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let live = saved
            .streams
            .into_iter()
            .map(|e| Live {
                info: info_of(&e, StreamStage::Sidecar, false),
                e,
                starting: true,
                ending: false,
                ice: None,
                next_mint: 0,
            })
            .collect();
        let peers = saved
            .peers
            .into_iter()
            .map(|e| PeerLive { info: peer_info(&e, StreamStage::Live, true), e, ending: false })
            .collect();
        Arc::new_cyclic(|me| Self {
            turn: Turn::new(s.turn.clone()),
            s,
            backend: p.backend,
            docs: p.docs,
            bus: p.bus,
            jobs: p.jobs,
            runs: p.runs,
            builds: p.builds,
            routes: p.routes,
            private: p.private,
            tunnel: p.tunnel,
            peers: p.peers,
            file,
            st: Mutex::new(St { live, peers, ..Default::default() }),
            locks: Mutex::default(),
            me: me.clone(),
        })
    }

    fn lock_for(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks.lock().unwrap().entry(key.into()).or_default().clone()
    }

    fn folder(&self, id: &str) -> PathBuf {
        self.s.state_dir.join("streams").join(id)
    }

    // ------------------------------------------------------------ the list

    fn list(&self) -> StreamList {
        let st = self.st.lock().unwrap();
        let mut streams: Vec<StreamInfo> =
            st.live.iter().map(|l| l.info.clone()).chain(st.peers.iter().map(|p| p.info.clone())).collect();
        streams.sort_by_key(|s| s.started);
        StreamList { streams, available: st.available.clone().unwrap_or_else(|| self.unavailable()) }
    }

    /// Before the machine's been looked at: nothing yet.
    fn unavailable(&self) -> StreamsAvailability {
        if !self.s.enabled {
            let off = Availability::no("Streaming is off on this machine");
            return StreamsAvailability {
                private: off.clone(),
                private_secure: off.clone(),
                public: off.clone(),
                public_secure: off,
            };
        }
        let mut a = StreamsAvailability::of(false, false, self.s.vm_runtime.is_some());
        a.public_secure = Availability::no(SECURE_PUBLIC_LATER);
        a
    }

    /// Watchers hear the streams, if they changed.
    fn publish(&self) {
        let l = self.list();
        {
            let mut st = self.st.lock().unwrap();
            if st.published.as_ref() == Some(&l) {
                return;
            }
            st.published = Some(l.clone());
        }
        self.bus.publish(l.into());
    }

    /// `streams.json`: the streams that haven't ended, without secrets.
    fn save(&self) {
        let bytes = {
            let st = self.st.lock().unwrap();
            let saved = Saved {
                streams: st.live.iter().filter(|l| !l.ending).map(|l| l.e.clone()).collect(),
                peers: st.peers.iter().filter(|p| !p.ending).map(|p| p.e.clone()).collect(),
            };
            let bytes = serde_json::to_vec_pretty(&saved).expect("json");
            if bytes == st.written {
                return;
            }
            bytes
        };
        match write_private(&self.file, &bytes, None) {
            Ok(()) => self.st.lock().unwrap().written = bytes,
            Err(e) => tracing::warn!("{}: {e}", self.file.display()),
        }
    }

    /// Which modes can start now (told when it changed).
    async fn available(&self) -> StreamsAvailability {
        let a = if self.s.enabled {
            let tailnet = self.private.host().is_some();
            let tunnel = self.tunnel.status().await;
            let mut a = StreamsAvailability::of(tailnet, tunnel.available, self.s.vm_runtime.is_some());
            tunnel.apply(&mut a);
            a.public_secure = Availability::no(SECURE_PUBLIC_LATER);
            a
        } else {
            self.unavailable()
        };
        let changed = {
            let mut st = self.st.lock().unwrap();
            let changed = st.available.as_ref() != Some(&a);
            st.available = Some(a.clone());
            changed
        };
        if changed {
            self.publish();
        }
        a
    }

    /// Changes a stream of this machine's (by wadspace and guid), unless it's ending.
    fn with(&self, id: &str, guid: &str, f: impl FnOnce(&mut Live)) -> bool {
        let found = {
            let mut st = self.st.lock().unwrap();
            match st.live.iter_mut().find(|l| l.e.id == id && l.e.guid == guid && !l.ending) {
                Some(l) => {
                    f(l);
                    true
                }
                None => false,
            }
        };
        if found {
            self.publish();
        }
        found
    }

    /// The guid of `id`'s stream, if it has one that isn't ending.
    fn streaming(&self, id: &str) -> Option<String> {
        self.st.lock().unwrap().live.iter().find(|l| l.e.id == id && !l.ending).map(|l| l.e.guid.clone())
    }

    /// Tests: the stream `id` has now, if any.
    #[doc(hidden)]
    pub fn guid_of(&self, id: &str) -> Option<String> {
        self.streaming(id)
    }

    // ------------------------------------------------------------ start

    /// Free ports for a stream: its sidecar's web port; public, its WebRTC
    /// mux port (UDP and TCP, on the machine); private, its listener.
    fn ports(&self, mode: StreamMode) -> Result<(u16, Option<u16>, Option<u16>), ApiError> {
        let used: Vec<u16> = {
            let st = self.st.lock().unwrap();
            st.live.iter().flat_map(|l| [Some(l.e.web_port), l.e.mux_port, l.e.private_port]).flatten().collect()
        };
        let s = &self.s;
        let free = |first: u16, ok: &dyn Fn(u16) -> bool| {
            (first..first.saturating_add(s.ports)).find(|p| !used.contains(p) && ok(*p))
        };
        let loopback = |p: u16| std::net::TcpListener::bind(("127.0.0.1", p)).is_ok();
        let machine = |p: u16| {
            std::net::TcpListener::bind(("0.0.0.0", p)).is_ok() && std::net::UdpSocket::bind(("0.0.0.0", p)).is_ok()
        };
        let full = || conflict("There's no free port for another stream on this machine");
        let web = free(s.first_web_port, &loopback).ok_or_else(full)?;
        let mux = match mode.is_public() {
            true => Some(free(s.first_mux_port, &machine).ok_or_else(full)?),
            false => None,
        };
        let private = match mode.is_public() {
            false => Some(free(s.first_private_port, &loopback).ok_or_else(full)?),
            true => None,
        };
        Ok((web, mux, private))
    }

    async fn begin(&self, ws: &str, req: StreamRequest) -> Result<JobInfo, ApiError> {
        let me = self.me.upgrade().ok_or_else(|| ApiError::internal("wadd is stopping"))?;
        let doc = self
            .docs
            .live()
            .into_iter()
            .find(|w| w.id == ws)
            .ok_or_else(|| ApiError::not_found(format!("There's no wadspace {ws:?} here")))?;
        let already = || conflict(format!("{} is streaming already: stop it first", doc.name));
        if self.streaming(ws).is_some() {
            return Err(already());
        }
        if req.players != 0 {
            let right = (2..=MAX_PLAYERS).contains(&req.players);
            if !right || !req.mode.is_public() || req.mode.is_secure() || !wad_model::games::plays_together(&doc) {
                return Err(conflict(PLAYS_PUBLIC));
            }
        }
        // After an end that's still tidying up, if there is one.
        let held = self.lock_for(ws).lock_owned().await;
        if self.streaming(ws).is_some() {
            return Err(already());
        }
        let avail = self.available().await;
        if let Some(why) = &avail.get(req.mode).reason {
            return Err(conflict(why.clone()));
        }
        // Its VM relies on SELinux's separation, which a folder from this machine turns off.
        if req.mode.is_secure()
            && let Some(p) = doc.look.projects.iter().find(|p| matches!(p.kind, ProjectKind::Mount { .. }))
        {
            return Err(conflict(units::SecureRefused::Mount(p.as_name.clone()).to_string()));
        }
        // runs.rs runs the image it built last, as on the screen.
        if self.builds.image_for(&doc).is_none() {
            return Err(conflict(format!("{} isn't built on this machine yet: build it first", doc.name)));
        }
        match self.backend.image_info(&self.s.image).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(conflict(format!(
                    "The stream sidecar ({}) isn't on this machine: images/build.sh --only stream",
                    self.s.image
                )));
            }
            Err(e) => return Err(ApiError::new(ErrorCode::Offline, e)),
        }
        let run = self.runs.state(ws);
        if run.state.phase == Phase::Stopping {
            return Err(conflict(format!("{} is stopping", doc.name)));
        }
        let up = active(run.state.phase);
        let shown = run.display.filter(|_| up);
        if shown.is_some_and(|d| d != SessionDisplay::Stream) && !req.takeover {
            return Err(conflict(format!("{} is on the screen: streaming it takes it off the screen", doc.name)));
        }
        // A game's own stream has player 1's gamepad (a phone's is the touch
        // one on the stream's page), as Play together's has: Dolphin's and
        // Steam's wadspaces. Not a secure one's (its VM has no interposer).
        let players = match req.players {
            0 if !req.mode.is_secure() && wad_model::games::plays_together(&doc) => 1,
            n => n,
        };
        // Off the screen as it runs, when it can (a secure one starts again in its VM).
        // Gamepads need its apps started with the interposer: it starts again too.
        let moving = players == 0
            && shown.is_some_and(|d| d != SessionDisplay::Stream)
            && !req.mode.is_secure()
            && self.runs.moves(ws);
        let (web_port, mux_port, private_port) = self.ports(req.mode)?;
        let guid = random_hex(16);
        let link = match private_port {
            Some(port) => {
                let host = self.private.host().ok_or_else(|| conflict(wad_proto::streams::PRIVATE_NEEDS_TAILNET))?;
                format!("https://{host}:{port}{}/", units::stream_path(&guid))
            }
            None => format!("https://{}{}/", public_host(self.tunnel.host()), units::stream_path(&guid)),
        };
        let plan = stages(req.mode, !moving);
        let labelled: Vec<(String, String)> = plan.iter().map(|(s, l)| (s.as_str().into(), l.to_string())).collect();
        let job = self.jobs.create(JobKind::Stream, &format!("Streaming {}", doc.name), ws, &labelled);
        let mode = serde_json::to_value(req.mode).unwrap_or(Value::Null);
        self.jobs.update(&job, true, |j| {
            j.params.insert("mode".into(), mode);
        });
        let e = Entry {
            id: ws.into(),
            guid,
            mode: req.mode,
            web_port,
            mux_port,
            private_port,
            user: req.mode.is_public().then(new_user),
            started: now_ms(),
            link,
            turn_users: vec![],
            job: Some(job.clone()),
            moved_from: shown.filter(|_| moving),
            players,
        };
        {
            let mut st = self.st.lock().unwrap();
            st.live.retain(|l| l.e.id != ws);
            st.live.push(Live {
                info: info_of(&e, plan[0].0, false),
                e: e.clone(),
                starting: true,
                ending: false,
                ice: None,
                next_mint: 0,
            });
        }
        self.save();
        self.publish();
        tracing::info!("{ws}: streaming it ({:?})", req.mode);
        let took = shown.filter(|_| up);
        let took = took.map(|d| (d, moving));
        let task = tokio::spawn(me.bring_up(held, doc, e, plan.into_iter().map(|(s, _)| s).collect(), took));
        self.jobs.attach(&job, task.abort_handle());
        self.jobs.info(&job).ok_or_else(|| ApiError::internal("the job went"))
    }

    /// Its Stream job's work, holding its wadspace's lock.
    async fn bring_up(
        self: Arc<Self>,
        _held: tokio::sync::OwnedMutexGuard<()>,
        ws: Wadspace,
        e: Entry,
        plan: Vec<StreamStage>,
        took: Option<(SessionDisplay, bool)>,
    ) {
        let job = e.job.clone().unwrap_or_default();
        let mut guard = Starting { me: self.clone(), id: e.id.clone(), guid: e.guid.clone(), armed: true };
        self.jobs.running(&job);
        let r = self.stages(&ws, &e, &plan, &job, took).await;
        guard.armed = false;
        match r {
            Ok(()) => {
                self.with(&e.id, &e.guid, |l| {
                    l.starting = false;
                    l.info.stage = StreamStage::Live;
                    l.info.live = true;
                });
                self.save();
                self.jobs.finish(&job, Ok(()));
                tracing::info!("{}: streamed at {}", e.id, e.link);
            }
            Err(why) => {
                tracing::warn!("{}: no stream: {why}", e.id);
                let ended = {
                    let mut st = self.st.lock().unwrap();
                    st.live.iter_mut().find(|l| l.e.guid == e.guid && !l.ending).map(|l| {
                        l.ending = true;
                        l.info.stage = StreamStage::Ended;
                        l.info.live = false;
                        l.e.clone()
                    })
                };
                self.publish();
                self.save();
                if let Some(entry) = ended {
                    self.teardown(&entry, None).await;
                    self.forget_later(&entry);
                }
                self.jobs.finish(&job, Err(why));
                // Taken off the screen for it: back on the screen (a move called off is there still).
                if let Some((d, _)) = took
                    && let Err(e) = self.runs.open(&ws.id, d)
                {
                    tracing::info!("{}: back on the screen: {}", ws.id, e.message);
                }
            }
        }
    }

    /// Its stages, in turn. TURN minted for one that then doesn't start is revoked.
    async fn stages(
        &self,
        ws: &Wadspace,
        e: &Entry,
        plan: &[StreamStage],
        job: &str,
        took: Option<(SessionDisplay, bool)>,
    ) -> Result<(), String> {
        let mut mint: Option<Mint> = None;
        let r = self.each_stage(ws, e, plan, job, took, &mut mint).await;
        if r.is_err()
            && let (Some(m), Some(me)) = (mint, self.me.upgrade())
        {
            tokio::spawn(async move {
                if let Ok(Ok(ice)) = m.await {
                    me.turn.revoke(&ice.users).await;
                }
            });
        }
        r
    }

    async fn each_stage(
        &self,
        ws: &Wadspace,
        e: &Entry,
        plan: &[StreamStage],
        job: &str,
        took: Option<(SessionDisplay, bool)>,
        mint: &mut Option<Mint>,
    ) -> Result<(), String> {
        let id = &e.id;
        let moving = took.is_some_and(|(_, m)| m);
        let mut key = String::new();
        let mut password = String::new();
        let unit = StreamUnit {
            image: self.s.image.clone(),
            guid: e.guid.clone(),
            web_port: e.web_port,
            transport: match e.mux_port {
                Some(mux_port) => units::Transport::WebRtc { mux_port },
                None => units::Transport::WebSockets,
            },
            dir: self.folder(id).to_string_lossy().into_owned(),
            origin: origin_of(&e.link),
            vm: match e.mode.is_secure() {
                true => self.s.vm_runtime.clone().map(|runtime| SecureVm {
                    runtime,
                    cpus: self.s.vm_cpus,
                    ram_mib: self.s.vm_ram_mib,
                }),
                false => None,
            },
            players: e.players,
        };
        for stage in plan {
            self.jobs.stage(job, stage.as_str(), PartState::Working, None);
            if !self.with(id, &e.guid, |l| l.info.stage = *stage) {
                return Err("it was stopped".into());
            }
            match stage {
                StreamStage::Vm => {
                    // Its unit runs it in the VM runtime (runs.rs, from
                    // StreamUnit::vm), with Selkies inside: the Container stage.
                    if unit.vm.is_none() {
                        return Err(wad_proto::streams::SECURE_NEEDS_VM.into());
                    }
                }
                StreamStage::Container => {
                    if took.is_some() {
                        tracing::info!("{id}: off the screen, to be streamed (it starts again)");
                        self.runs.stop(id).await.map_err(|e| e.message)?;
                    }
                    (key, password, *mint) = self.prepare(e).await?;
                    self.runs.open_streamed(id, unit.clone()).map_err(|e| e.message)?;
                    self.wait_desktop(ws).await?;
                }
                StreamStage::Sidecar if moving => {
                    // Its sidecar beside it, then its desktop moves there as it runs.
                    (key, password, *mint) = self.prepare(e).await?;
                    self.runs.add_sidecar(id, &unit).await.map_err(|e| e.message)?;
                    self.wait_sidecar(e).await?;
                    tracing::info!("{id}: moving to its stream as it runs");
                    self.runs.switch(id, SessionDisplay::Stream).await.map_err(|e| e.message)?;
                }
                StreamStage::Sidecar => self.wait_sidecar(e).await?,
                StreamStage::Route | StreamStage::Serve => {
                    let route = Route {
                        web_port: e.web_port,
                        key: key.clone(),
                        players: self.player_keys(id).await,
                        user: e.user.clone().unwrap_or_default(),
                        password: password.clone(),
                        name: ws.name.clone(),
                        mode: e.mode,
                        private_port: e.private_port,
                    };
                    self.routes.add(&e.guid, route).map_err(|why| format!("the proxy didn't take it: {why}"))?;
                    if let Some(port) = e.private_port {
                        self.private.serve(port).await.map_err(|why| format!("the tailnet won't serve it: {why}"))?;
                    }
                }
                StreamStage::Turn => {
                    let r = match mint.take() {
                        Some(m) => m.await.unwrap_or_else(|e| Err(e.to_string())),
                        None => Err("TURN wasn't asked".into()),
                    };
                    if let Err(why) = &r {
                        self.jobs.line(job, &format!("TURN: {why}"));
                    }
                    let down = r.is_err();
                    self.minted(id, &e.guid, r);
                    if down {
                        self.jobs.stage(job, stage.as_str(), PartState::Done, Some(TURN_DOWN.into()));
                        continue;
                    }
                }
                StreamStage::Live | StreamStage::Ended => {}
            }
            self.jobs.stage(job, stage.as_str(), PartState::Done, None);
        }
        Ok(())
    }

    /// Its secrets and folder, and (public) TURN asked: its key and password,
    /// and the mint coming.
    async fn prepare(&self, e: &Entry) -> Result<(String, String, Option<Mint>), String> {
        let (key, password) = self.make_secrets(e).await?;
        let others = self.player_keys(&e.id).await;
        self.write_folder(e, &key, &others)?;
        let mint = match e.mode.is_public() {
            true => {
                let me = self.me.upgrade().ok_or("wadd is stopping")?;
                Some(tokio::spawn(async move { me.turn.mint().await }))
            }
            false => None,
        };
        Ok((key, password, mint))
    }

    /// Play together's other players' keys (none: not Play together).
    async fn player_keys(&self, id: &str) -> Vec<String> {
        let kept = self.backend.secret_value(&players_secret(id)).await.ok().flatten();
        kept.and_then(|k| serde_json::from_str(&k).ok()).unwrap_or_default()
    }

    /// Its secrets, new: the master token, its key and (public) its password.
    async fn make_secrets(&self, e: &Entry) -> Result<(String, String), String> {
        let key = random_hex(16);
        let master = random_hex(32);
        let password = if e.mode.is_public() { new_password() } else { String::new() };
        let make = |name: String, value: String| {
            let backend = self.backend.clone();
            async move { backend.create_secret(&name, value.as_bytes()).await.map_err(|e| format!("{name}: {e}")) }
        };
        make(master_secret(&e.id), master).await?;
        make(key_secret(&e.id), key.clone()).await?;
        if e.players > 1 {
            let others: Vec<String> = (1..e.players).map(|_| random_hex(16)).collect();
            make(players_secret(&e.id), serde_json::to_string(&others).expect("json")).await?;
        }
        if e.mode.is_public() {
            make(password_secret(&e.id), password.clone()).await?;
        }
        Ok((key, password))
    }

    /// The sidecar's folder: its token table, and (public) its ICE servers
    /// (STUN only until TURN answers).
    fn write_folder(&self, e: &Entry, key: &str, others: &[String]) -> Result<(), String> {
        let dir = self.folder(&e.id);
        let say = |e: std::io::Error| format!("{}: {e}", dir.display());
        // Never a link someone left there.
        if std::fs::symlink_metadata(&dir).is_ok_and(|m| !m.is_dir()) {
            std::fs::remove_file(&dir).map_err(say)?;
        }
        std::fs::create_dir_all(&dir).map_err(say)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(say)?;
        if let Some(uid) = self.s.owner {
            nix::unistd::chown(&dir, Some(uid.into()), Some(uid.into()))
                .map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let tokens = serde_json::to_vec_pretty(&token_table(key, others, e.players)).expect("json");
        write_private(&dir.join(units::TOKENS_FILE), &tokens, self.s.owner).map_err(say)?;
        if e.mode.is_public() {
            write_private(&dir.join(units::RTC_FILE), &turn::rtc_json(&turn::stun_only()), self.s.owner)
                .map_err(say)?;
        }
        Ok(())
    }

    /// Until its desktop is up on its stream (runs.rs's bring-up says).
    async fn wait_desktop(&self, ws: &Wadspace) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + self.s.ready_timeout + START_SLACK;
        loop {
            let st = self.runs.state(&ws.id);
            match st.state.phase {
                Phase::Ready if st.display == Some(SessionDisplay::Stream) => return Ok(()),
                Phase::Error => return Err(st.state.error.unwrap_or_else(|| format!("{} didn't start", ws.name))),
                Phase::Idle => return Err(format!("{} stopped before its desktop was up", ws.name)),
                Phase::Ready => return Err(format!("{} was opened on the screen", ws.name)),
                _ => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("{}'s desktop wasn't up in time", ws.name));
            }
            tokio::time::sleep(self.s.poll).await;
        }
    }

    /// Until Selkies serves under its path.
    async fn wait_sidecar(&self, e: &Entry) -> Result<(), String> {
        let url = format!("http://127.0.0.1:{}{}/api/health", e.web_port, units::stream_path(&e.guid));
        // Secure: Selkies is in the wadspace's VM, not a sidecar of its own.
        let name = if e.mode.is_secure() { container_name(&e.id) } else { display_name(&e.id) };
        let deadline = tokio::time::Instant::now() + self.s.ready_timeout;
        loop {
            if self.backend.fetch(&url).await.is_ok() {
                return Ok(());
            }
            if self.backend.container_status(&name).await.is_ok_and(|s| s != RUNNING) {
                let logs = self.backend.container_logs(&name, 15).await.ok().flatten().unwrap_or_default();
                let last: Vec<&str> = logs.lines().rev().take(3).collect();
                let why = last.into_iter().rev().collect::<Vec<_>>().join(" / ");
                return Err(format!(
                    "the Selkies sidecar stopped{}",
                    if why.is_empty() { String::new() } else { format!(": {why}") }
                ));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("the Selkies sidecar didn't answer within {} s", self.s.ready_timeout.as_secs()));
            }
            tokio::time::sleep(self.s.poll).await;
        }
    }

    /// A TURN mint's answer, for a public stream: its file rewritten, or
    /// (TURN down, once its credentials have run out) STUN alone, said.
    fn minted(&self, id: &str, guid: &str, r: Result<Ice, String>) {
        let now = now_ms();
        let dir = self.folder(id);
        let owner = self.s.owner;
        let mut write: Option<Vec<u8>> = None;
        let mut flipped = None;
        self.with(id, guid, |l| match r {
            Ok(ice) => {
                write = Some(turn::rtc_json(&ice.servers));
                l.next_mint = self.turn.refresh_at(&ice);
                for u in &ice.users {
                    if !l.e.turn_users.contains(u) {
                        l.e.turn_users.push(u.clone());
                    }
                }
                l.ice = Some(ice);
                if l.info.turn_down {
                    flipped = Some(false);
                }
                l.info.turn_down = false;
                l.info.warnings.retain(|w| w != TURN_DOWN);
            }
            Err(why) => {
                l.next_mint = now + MINT_RETRY_MS;
                if l.ice.as_ref().is_none_or(|i| i.expires <= now) {
                    if l.ice.is_some() {
                        write = Some(turn::rtc_json(&turn::stun_only()));
                    }
                    l.ice = None;
                    if !l.info.turn_down {
                        flipped = Some(true);
                        tracing::info!("{id}: TURN is down for its stream: {why}");
                    }
                    l.info.turn_down = true;
                    if !l.info.warnings.iter().any(|w| w == TURN_DOWN) {
                        l.info.warnings.push(TURN_DOWN.into());
                    }
                }
            }
        });
        if let Some(bytes) = write
            && let Err(e) = write_private(&dir.join(units::RTC_FILE), &bytes, owner)
        {
            tracing::warn!("{id}: its stream's ICE servers: {e}");
        }
        if flipped == Some(false) {
            tracing::info!("{id}: TURN answers again for its stream");
        }
        self.save();
    }

    // ------------------------------------------------------------ end

    /// Ends this stream, if it's still the one there (a late tick mustn't
    /// end the next): everything it had goes. It, ended.
    pub async fn end(&self, id: &str, guid: &str, why: &str) -> Option<StreamInfo> {
        self.end_to(id, guid, why, None).await
    }

    /// Ends it; `back`: its wadspace goes back on the screen, shown so (the
    /// person at the machine opened it), as it runs if it can.
    async fn end_to(&self, id: &str, guid: &str, why: &str, back: Option<SessionDisplay>) -> Option<StreamInfo> {
        let (ended, entry, job) = {
            let mut st = self.st.lock().unwrap();
            let l = st.live.iter_mut().find(|l| l.e.id == id && l.e.guid == guid && !l.ending)?;
            l.ending = true;
            l.info.stage = StreamStage::Ended;
            l.info.live = false;
            l.info.viewers = 0;
            (l.info.clone(), l.e.clone(), l.e.job.clone().filter(|_| l.starting))
        };
        tracing::info!("{id}: its stream ends ({why})");
        self.publish();
        self.save();
        // Still starting: its job stops (and lets go of the lock).
        if let Some(job) = job
            && self.jobs.info(&job).is_some_and(|j| !crate::jobs::finished(j.job.status))
        {
            let _ = self.jobs.cancel(&job);
        }
        let lock = self.lock_for(id);
        let _held = lock.lock().await;
        self.teardown(&entry, back).await;
        self.forget_later(&entry);
        Some(ended)
    }

    /// An ended stream leaves the list a moment later.
    fn forget_later(&self, e: &Entry) {
        let Some(me) = self.me.upgrade() else { return };
        let guid = e.guid.clone();
        tokio::spawn(async move {
            tokio::time::sleep(ENDED_FOR).await;
            me.st.lock().unwrap().live.retain(|l| !(l.e.guid == guid && l.ending));
            me.publish();
        });
    }

    /// Everything a stream had, gone: its route, the tailnet's serve, its
    /// wadspace (while it's streamed) and sidecar, their units and volume,
    /// its secrets and folder, its TURN credentials.
    async fn teardown(&self, e: &Entry, back: Option<SessionDisplay>) {
        let id = &e.id;
        self.routes.remove(&e.guid);
        if let Some(port) = e.private_port {
            self.private.unserve(port).await;
        }
        let run = self.runs.state(id);
        if run.display == Some(SessionDisplay::Stream) && run.state.phase != Phase::Idle {
            // Back on the screen as it runs, if it can; else it stops.
            let moved = match back {
                Some(d) if self.runs.moves(id) => match self.runs.switch(id, d).await {
                    Ok(()) => true,
                    Err(err) => {
                        tracing::info!("{id}: it didn't move back to the screen: {}", err.message);
                        false
                    }
                },
                _ => false,
            };
            if !moved && let Err(err) = self.runs.stop(id).await {
                tracing::warn!("{id}: stopping it as its stream ends: {}", err.message);
            }
        }
        let display = display_name(id);
        if self.backend.container_status(&display).await.is_ok_and(|s| s == RUNNING) {
            let _ = self.backend.stop(&format!("{display}.service")).await;
        }
        // A moving wadspace back on the screen keeps its display volume: its
        // sidecar's unit goes. Else the volume goes, once nothing uses it.
        let up = self.backend.container_status(&container_name(id)).await.is_ok_and(|s| s == RUNNING);
        if up {
            if let Err(err) = self.backend.write_units(&[], &[format!("{display}.container")]).await {
                tracing::warn!("{id}: its sidecar's unit: {err}");
            }
        } else {
            self.drop_display(id).await;
        }
        if e.mode.is_secure() {
            self.runs.drop_secure_volume(id).await;
        }
        for name in [master_secret(id), key_secret(id), password_secret(id), players_secret(id)] {
            if let Err(err) = self.backend.delete_secret(&name).await {
                tracing::warn!("removing {name}: {err}");
            }
        }
        if crate::store::safe_id(id) {
            let _ = std::fs::remove_dir_all(self.folder(id));
        }
        self.turn.revoke(&e.turn_users).await;
    }

    /// `id`'s display volume and its units gone, deterministically
    /// ([`crate::backend::drop_volume`]: once its wadspace and sidecar have).
    async fn drop_display(&self, id: &str) {
        let display = display_name(id);
        let files = [format!("{display}.container"), format!("{display}.volume")];
        let users = [container_name(id), display.clone()];
        let unit = format!("{display}.volume");
        let r = drop_volume(&*self.backend, &display, &unit, &files, &users, self.s.poll, VOLUME_WAIT).await;
        if let Err(err) = r {
            tracing::warn!("{id}: its stream's display volume is left: {err}");
        }
    }

    // ------------------------------------------------------------ adopt and tick

    /// After a restart: streams whose wadspace and sidecar still run are
    /// routed again; the rest end; what no stream has goes. False: podman
    /// isn't answering yet (try again).
    pub async fn adopt(&self) -> bool {
        if !self.backend.available().await {
            return false;
        }
        let found: Vec<Entry> =
            self.st.lock().unwrap().live.iter().filter(|l| l.starting).map(|l| l.e.clone()).collect();
        for e in found {
            if let Err(why) = self.adopt_one(&e).await {
                self.end(&e.id, &e.guid, &why).await;
            }
        }
        self.tidy().await;
        self.st.lock().unwrap().adopted = true;
        let n = self.st.lock().unwrap().live.iter().filter(|l| !l.ending).count();
        if n > 0 {
            tracing::info!("streams: {n} still running");
        }
        self.publish();
        true
    }

    async fn adopt_one(&self, e: &Entry) -> Result<(), String> {
        let status = |name: String| {
            let backend = self.backend.clone();
            async move { backend.container_status(&name).await.is_ok_and(|s| s == RUNNING) }
        };
        // Secure: its Selkies is in the wadspace's VM (no sidecar), and up.
        let sidecar = !e.mode.is_secure();
        if !status(container_name(&e.id)).await || (sidecar && !status(display_name(&e.id)).await) {
            return Err("it ended while wadd was away".into());
        }
        if !sidecar {
            let url = format!("http://127.0.0.1:{}{}/api/health", e.web_port, units::stream_path(&e.guid));
            self.backend.fetch(&url).await.map_err(|_| "its VM doesn't answer".to_string())?;
            self.runs.taken_on_secure(&e.id);
        }
        // It moved here from the screen as it ran: runs knows again it can move back.
        if e.moved_from.is_some() {
            self.runs.taken_on_moved(&e.id);
        }
        let secret = |name: String| {
            let backend = self.backend.clone();
            async move { backend.secret_value(&name).await.ok().flatten().unwrap_or_default() }
        };
        let key = secret(key_secret(&e.id)).await;
        let password = if e.mode.is_public() { secret(password_secret(&e.id)).await } else { String::new() };
        if key.is_empty() || (e.mode.is_public() && password.is_empty()) {
            return Err("its secrets are gone".into());
        }
        let name = self.docs.wadspace(&e.id).map_or_else(|| e.id.clone(), |w| w.name);
        let route = Route {
            players: self.player_keys(&e.id).await,
            web_port: e.web_port,
            key,
            user: e.user.clone().unwrap_or_default(),
            password,
            name,
            mode: e.mode,
            private_port: e.private_port,
        };
        self.routes.remove(&e.guid);
        self.routes.add(&e.guid, route)?;
        if let Some(port) = e.private_port
            && !self.private.served().await.contains(&port)
        {
            self.private.serve(port).await.map_err(|why| format!("the tailnet won't serve it: {why}"))?;
        }
        self.with(&e.id, &e.guid, |l| {
            l.starting = false;
            l.info.stage = StreamStage::Live;
            l.info.live = true;
        });
        if e.mode.is_public() {
            let r = self.turn.mint().await;
            self.minted(&e.id, &e.guid, r);
        }
        tracing::info!("{}: its stream carries on at {}", e.id, e.link);
        Ok(())
    }

    /// What no stream has: secrets, folders, sidecars, the tailnet's serves.
    async fn tidy(&self) {
        let ours = |id: &str| self.st.lock().unwrap().live.iter().any(|l| l.e.id == id && !l.ending);
        for name in self.backend.secret_names().await.unwrap_or_default() {
            if let Some(id) = secret_owner(&name)
                && !ours(id)
            {
                tracing::info!("streams: {name} is no stream's: removing it");
                let _ = self.backend.delete_secret(&name).await;
            }
        }
        for d in std::fs::read_dir(self.s.state_dir.join("streams")).into_iter().flatten().flatten() {
            let id = d.file_name().to_string_lossy().into_owned();
            if !ours(&id) {
                let _ = std::fs::remove_dir_all(d.path());
            }
        }
        let mut ids: Vec<String> = self.docs.live().into_iter().map(|w| w.id).collect();
        ids.extend(self.runs.states().into_iter().map(|r| r.id));
        ids.sort();
        ids.dedup();
        for id in &ids {
            let side = display_name(id);
            if !ours(id) && self.backend.container_status(&side).await.is_ok_and(|s| s == RUNNING) {
                tracing::warn!("streams: {side} has no stream here: stopping it");
                let _ = self.backend.stop(&format!("{side}.service")).await;
            }
        }
        // Display volumes (and their units) no stream and no running wadspace has.
        let gone = |name: String| {
            let backend = self.backend.clone();
            async move { backend.container_status(&name).await.is_ok_and(|s| s == MISSING) }
        };
        for v in self.backend.volume_names().await.unwrap_or_default() {
            let Some(id) = v.strip_prefix("wadstream-") else { continue };
            if !ours(id) && gone(container_name(id)).await {
                tracing::info!("streams: the display volume {v} is no stream's: removing it");
                self.drop_display(id).await;
            }
        }
        // A display volume's oneshot left active with its volume gone (it stays active).
        let volumes = self.backend.volume_names().await.unwrap_or_default();
        for id in &ids {
            let display = display_name(id);
            if !ours(id) && !volumes.contains(&display) && gone(container_name(id)).await {
                let oneshot = format!("{display}-volume.service");
                let _ = self.backend.stop(&oneshot).await;
                let _ = self.backend.reset_failed(&oneshot).await;
            }
        }
        if self.private.host().is_some() {
            let range = self.s.first_private_port..self.s.first_private_port.saturating_add(self.s.ports);
            let live: Vec<u16> = self.st.lock().unwrap().live.iter().filter_map(|l| l.e.private_port).collect();
            for port in self.private.served().await.into_iter().filter(|p| range.contains(p) && !live.contains(p)) {
                tracing::info!("streams: the tailnet served :{port} for a stream that's over");
                self.private.unserve(port).await;
            }
        }
    }

    /// Every few seconds: streams whose wadspace stopped, that were opened on
    /// the screen, or that nobody watched for `idle_minutes`, end; viewers are
    /// told; TURN credentials are renewed; another machine's are asked after.
    pub async fn tick(&self) {
        self.available().await;
        let now = now_ms();
        let idle = self.s.idle.as_millis() as i64;
        let lives: Vec<(Entry, i64)> = {
            let st = self.st.lock().unwrap();
            st.live.iter().filter(|l| !l.starting && !l.ending).map(|l| (l.e.clone(), l.next_mint)).collect()
        };
        for (e, next_mint) in lives {
            if self.backend.container_status(&container_name(&e.id)).await.is_ok_and(|s| s != RUNNING) {
                self.end(&e.id, &e.guid, "its wadspace stopped").await;
                continue;
            }
            let run = self.runs.state(&e.id);
            if active(run.state.phase) && run.display.is_some_and(|d| d != SessionDisplay::Stream) {
                self.end(&e.id, &e.guid, "it was opened on the screen").await;
                continue;
            }
            let (last, viewers) = self.routes.activity(&e.guid).unwrap_or((e.started, 0));
            let last = last.max(e.started);
            let mut changed = false;
            {
                let mut st = self.st.lock().unwrap();
                if let Some(l) = st.live.iter_mut().find(|l| l.e.guid == e.guid && !l.ending) {
                    changed = l.info.viewers != viewers || l.info.last_active != last;
                    (l.info.viewers, l.info.last_active) = (viewers, last);
                }
            }
            if changed {
                self.publish();
            }
            if viewers == 0 && now - last > idle {
                let mins = idle / 60_000;
                self.end(&e.id, &e.guid, &format!("nobody watched it for {mins} minutes")).await;
                continue;
            }
            if e.mode.is_public() {
                self.follow_transport(&e).await;
            }
            if e.mode.is_public() && next_mint > 0 && now >= next_mint {
                let r = self.turn.mint().await;
                self.minted(&e.id, &e.guid, r);
            }
        }
        self.check_peers().await;
    }

    /// A public stream's transport as Selkies has it now (its open
    /// `/api/status`'s `current_mode`): in dual mode a controller may switch
    /// every viewer to WebSockets (`POST /api/switch`), and back.
    async fn follow_transport(&self, e: &Entry) {
        let url = format!("http://127.0.0.1:{}{}/api/status", e.web_port, units::stream_path(&e.guid));
        let Ok(body) = self.backend.fetch(&url).await else { return };
        let mode =
            serde_json::from_slice::<Value>(&body).ok().and_then(|v| v["current_mode"].as_str().map(String::from));
        let now = match mode.as_deref() {
            Some("websockets") => Transport::Websocket,
            Some("webrtc") => Transport::Webrtc,
            _ => return,
        };
        let changed = {
            let mut st = self.st.lock().unwrap();
            match st.live.iter_mut().find(|l| l.e.guid == e.guid && !l.ending) {
                // WebRTC through TURN (WebrtcRelay) is WebRTC still.
                Some(l)
                    if l.info.transport != now
                        && !(now == Transport::Webrtc && l.info.transport == Transport::WebrtcRelay) =>
                {
                    l.info.transport = now;
                    true
                }
                _ => false,
            }
        };
        if changed {
            tracing::info!("{}: its stream is on {now:?} now", e.id);
            self.publish();
        }
    }

    /// Adopts what was running, then ticks, until wadd stops.
    pub async fn run(self: Arc<Self>) {
        let mut tries = 0u32;
        while !self.adopt().await {
            tries += 1;
            if tries == 10 {
                tracing::info!("streams: podman isn't answering yet; checking again");
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let mut viewers = self.routes.viewers_changed();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(TICK) => self.tick().await,
                // Someone came or went: the shell hears it now.
                _ = changed(&mut viewers) => self.watched(),
            }
        }
    }

    /// Who's watching each stream now, from the proxy (told when it changed).
    fn watched(&self) {
        let changed = {
            let mut st = self.st.lock().unwrap();
            let mut changed = false;
            for l in st.live.iter_mut().filter(|l| !l.starting && !l.ending) {
                if let Some((last, viewers)) = self.routes.activity(&l.e.guid) {
                    let last = last.max(l.e.started);
                    changed |= l.info.viewers != viewers || l.info.last_active != last;
                    (l.info.viewers, l.info.last_active) = (viewers, last);
                }
            }
            changed
        };
        if changed {
            self.publish();
        }
    }

    // ------------------------------------------------------------ the person at the machine

    /// The person at the machine opened `id` on the screen: its stream ends.
    async fn screen_wins(&self, id: &str, display: SessionDisplay) {
        let Some(guid) = self.streaming(id) else { return };
        let name = self.docs.wadspace(id).map_or_else(|| id.to_string(), |w| w.name);
        if self.end_to(id, &guid, "it was opened on this machine's screen", Some(display)).await.is_some() {
            let title = format!("{name}'s stream ended: it's on this machine's screen now");
            self.bus.publish(Event::notice(Level::Info, title, None));
        }
    }

    // ------------------------------------------------------------ another machine's

    fn peers(&self) -> Result<Arc<dyn PeerStreams>, ApiError> {
        self.peers.clone().ok_or_else(|| ApiError::new(ErrorCode::Offline, "This machine isn't on your tailnet"))
    }

    async fn peer_begin(&self, peer: &str, ws: &str, req: StreamRequest) -> Result<StreamShare, ApiError> {
        let peers = self.peers()?;
        let lock = self.lock_for(&format!("{peer}:{ws}"));
        let _held = lock.lock().await;
        let started = now_ms();
        let pending = PeerEntry { peer: peer.into(), id: ws.into(), mode: req.mode, link: String::new(), started };
        let known = {
            let mut st = self.st.lock().unwrap();
            match st.peers.iter().any(|p| p.e.peer == peer && p.e.id == ws && !p.ending) {
                true => true,
                false => {
                    st.peers.retain(|p| !(p.e.peer == peer && p.e.id == ws));
                    st.peers.push(PeerLive {
                        info: peer_info(&pending, StreamStage::Sidecar, false),
                        e: pending,
                        ending: false,
                    });
                    false
                }
            }
        };
        self.publish();
        let share = match peers.start(peer, ws, req).await {
            Ok(s) => s,
            Err(e) => {
                if !known {
                    self.st
                        .lock()
                        .unwrap()
                        .peers
                        .retain(|p| !(p.e.peer == peer && p.e.id == ws && p.e.started == started));
                    self.publish();
                }
                return Err(e);
            }
        };
        {
            let mut st = self.st.lock().unwrap();
            if let Some(p) = st.peers.iter_mut().find(|p| p.e.peer == peer && p.e.id == ws && !p.ending) {
                p.e.mode = share.mode;
                p.e.link = share.link.clone();
                p.info = StreamInfo { started: p.e.started, ..peer_info(&p.e, StreamStage::Live, true) };
            }
        }
        self.save();
        self.publish();
        tracing::info!("{peer}: streaming its {ws} here");
        Ok(share)
    }

    async fn peer_end(&self, peer: &str, ws: &str) -> Result<StreamInfo, ApiError> {
        let peers = self.peers()?;
        let lock = self.lock_for(&format!("{peer}:{ws}"));
        let _held = lock.lock().await;
        let ours = self
            .st
            .lock()
            .unwrap()
            .peers
            .iter()
            .find(|p| p.e.peer == peer && p.e.id == ws && !p.ending)
            .map(|p| p.info.clone());
        let r = peers.stop(peer, ws).await;
        let ended = match (r, ours) {
            (Ok(mut i), _) => {
                i.peer = Some(peer.into());
                i
            }
            // It had ended there already: here too.
            (Err(e), Some(mut i)) if e.code == ErrorCode::NotFound => {
                i.stage = StreamStage::Ended;
                i.live = false;
                i
            }
            (Err(e), _) => return Err(e),
        };
        self.forget_peer(peer, ws);
        Ok(StreamInfo { stage: StreamStage::Ended, live: false, viewers: 0, ..ended })
    }

    fn forget_peer(&self, peer: &str, ws: &str) {
        self.st.lock().unwrap().peers.retain(|p| !(p.e.peer == peer && p.e.id == ws));
        self.save();
        self.publish();
    }

    /// Another machine's streams here, as it has them now.
    async fn check_peers(&self) {
        let Some(peers) = self.peers.clone() else { return };
        let due = {
            let mut st = self.st.lock().unwrap();
            let due = st.peers_checked.is_none_or(|t| t.elapsed() >= PEER_EVERY);
            if due {
                st.peers_checked = Some(std::time::Instant::now());
            }
            due
        };
        if !due {
            return;
        }
        let known: Vec<PeerEntry> =
            self.st.lock().unwrap().peers.iter().filter(|p| !p.ending && p.info.live).map(|p| p.e.clone()).collect();
        for e in known {
            match peers.info(&e.peer, &e.id).await {
                Ok(None) => {
                    tracing::info!("{}: its stream of {} ended there", e.peer, e.id);
                    self.forget_peer(&e.peer, &e.id);
                }
                Ok(Some(i)) => {
                    {
                        let mut st = self.st.lock().unwrap();
                        if let Some(p) = st.peers.iter_mut().find(|p| p.e == e) {
                            p.info = StreamInfo { peer: Some(e.peer.clone()), job: None, started: e.started, ..i };
                        }
                    }
                    self.publish();
                }
                Err(err) => tracing::debug!("{}: its stream of {}: {}", e.peer, e.id, err.message),
            }
        }
    }
}

/// The public host as links have it (`[streaming] host`, maybe written as a URL).
fn public_host(host: &str) -> &str {
    let h = host.trim();
    let h = h.strip_prefix("https://").or_else(|| h.strip_prefix("http://")).unwrap_or(h);
    h.trim_end_matches('/')
}

/// A link's origin (`https://host[:port]`).
fn origin_of(link: &str) -> Option<String> {
    let rest = link.strip_prefix("https://")?;
    let host = rest.split('/').next().filter(|h| !h.is_empty())?;
    Some(format!("https://{host}"))
}

/// Writes `bytes` to `path` whole, 0600 (as a new file renamed over the
/// old: Selkies reads whole files), owned by `owner` when it's given.
pub fn write_private(path: &Path, bytes: &[u8], owner: Option<u32>) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.tmp"));
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    if let Some(uid) = owner {
        nix::unistd::chown(&tmp, Some(uid.into()), Some(uid.into())).map_err(std::io::Error::from)?;
    }
    std::fs::rename(&tmp, path)
}

/// The next change on `rx`; never, without one (or once it's closed).
async fn changed(rx: &mut Option<tokio::sync::watch::Receiver<u64>>) {
    match rx {
        Some(r) => {
            if r.changed().await.is_err() {
                *rx = None;
                std::future::pending::<()>().await;
            }
        }
        None => std::future::pending().await,
    }
}

/// A stream starting: if its job's dropped (cancelled) before it's done, it ends.
struct Starting {
    me: Arc<MachineStreams>,
    id: String,
    guid: String,
    armed: bool,
}

impl Drop for Starting {
    fn drop(&mut self) {
        if self.armed {
            let (me, id, guid) = (self.me.clone(), self.id.clone(), self.guid.clone());
            tokio::spawn(async move {
                me.end(&id, &guid, "its start was cancelled").await;
            });
        }
    }
}

#[async_trait]
impl StreamEnds for MachineStreams {
    async fn opened_on_screen(&self, id: &str, display: SessionDisplay) {
        self.screen_wins(id, display).await;
    }
}

#[async_trait]
impl Streams for MachineStreams {
    async fn streams(&self) -> Result<StreamList, ApiError> {
        self.available().await;
        Ok(self.list())
    }

    async fn start(&self, ws: &str, req: StreamRequest) -> Result<JobInfo, ApiError> {
        if !self.s.enabled {
            return Err(conflict("Streaming is off on this machine"));
        }
        let r = self.begin(ws, req).await;
        if let Err(e) = &r {
            tracing::info!("{ws}: no stream: {}", e.message);
        }
        r
    }

    async fn stop(&self, ws: &str) -> Result<StreamInfo, ApiError> {
        let (guid, back) = {
            let st = self.st.lock().unwrap();
            let l = st.live.iter().find(|l| l.e.id == ws && !l.ending).ok_or_else(|| not_streaming(ws))?;
            (l.e.guid.clone(), l.e.moved_from)
        };
        // One that moved here as it ran goes back on the screen (the person had it open there).
        self.end_to(ws, &guid, "it was stopped", back).await.ok_or_else(|| not_streaming(ws))
    }

    async fn share(&self, ws: &str) -> Result<StreamShare, ApiError> {
        let e = {
            let st = self.st.lock().unwrap();
            st.live.iter().find(|l| l.e.id == ws && !l.ending).map(|l| l.e.clone())
        };
        let e = e.ok_or_else(|| not_streaming(ws))?;
        let read = |name: String| {
            let backend = self.backend.clone();
            async move {
                backend
                    .secret_value(&name)
                    .await
                    .map_err(|e| ApiError::new(ErrorCode::Offline, e))?
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| ApiError::not_found(format!("{ws:?}'s stream isn't ready to share yet")))
            }
        };
        let key = read(key_secret(ws)).await?;
        let password = match e.mode.is_public() {
            true => Some(read(password_secret(ws)).await?),
            false => None,
        };
        let others = self.player_keys(ws).await;
        let players = std::iter::once(&key)
            .chain(&others)
            .enumerate()
            .map(|(i, k)| PlayerLink { slot: i as u8 + 1, open_link: format!("{}#token={k}", e.link) })
            .collect();
        Ok(StreamShare {
            mode: e.mode,
            open_link: format!("{}#token={key}", e.link),
            link: e.link,
            user: e.user.filter(|_| e.mode.is_public()),
            password,
            // Play together's links (one player's stream is its own link).
            players: if e.players <= 1 { vec![] } else { players },
        })
    }

    async fn start_peer(&self, peer: &str, ws: &str, req: StreamRequest) -> Result<StreamShare, ApiError> {
        self.peer_begin(peer, ws, req).await
    }

    async fn stop_peer(&self, peer: &str, ws: &str) -> Result<StreamInfo, ApiError> {
        self.peer_end(peer, ws).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logins_are_long_random_and_easy_to_type() {
        let p = new_password();
        assert_eq!(p.len(), 20);
        assert!(p.bytes().all(|b| PASSWORD_ALPHABET.contains(&b)));
        assert!(!p.contains(['0', 'O', '1', 'l', 'I']));
        assert_ne!(new_password(), p);
        let u = new_user();
        assert!(u.starts_with("wad-") && u.len() == 10 && u[4..].bytes().all(|b| b.is_ascii_lowercase()));
        let g = random_hex(16);
        assert!(g.len() == 32 && g.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert_ne!(random_hex(16), g);
        assert_eq!(PASSWORD_ALPHABET.len(), 56);
    }

    #[test]
    fn every_symbol_comes_up() {
        let s = random_from(b"abc", 3000);
        for c in ['a', 'b', 'c'] {
            let n = s.matches(c).count();
            assert!((800..1200).contains(&n), "{c}: {n}");
        }
    }

    #[test]
    fn an_ordinary_stream_has_one_controller_and_play_together_a_slot_each() {
        let one = token_table("k1", &[], 0);
        assert_eq!(one["k1"]["role"], "controller");
        assert!(one["k1"]["slot"].is_null());
        let four = token_table("k1", &["k2".into(), "k3".into(), "k4".into()], 4);
        assert_eq!(four["k1"], json!({ "role": "controller", "slot": 1 }));
        assert_eq!(four["k3"], json!({ "role": "viewer", "slot": 3 }));
        assert_eq!(four["k4"]["slot"], 4);
    }

    #[test]
    fn secrets_are_known_by_their_wadspace() {
        assert_eq!(secret_owner(&key_secret("iq")), Some("iq"));
        assert_eq!(secret_owner(&password_secret("a-b")), Some("a-b"));
        assert_eq!(secret_owner(&master_secret("iq")), Some("iq"));
        assert_eq!(secret_owner("github_token"), None);
        assert_eq!(secret_owner("wad_stream_other_iq"), None);
    }

    #[test]
    fn a_links_origin() {
        assert_eq!(origin_of("https://desk.ts.test:48300/ws-1/").as_deref(), Some("https://desk.ts.test:48300"));
        assert_eq!(origin_of("https://h.example/ws-1/").as_deref(), Some("https://h.example"));
        assert_eq!(origin_of("http://x/"), None);
    }

    #[test]
    fn a_vm_runtime_is_there_only_with_kvm() {
        let d = tempfile::tempdir().unwrap();
        let rt = d.path().join("krun");
        std::fs::write(&rt, "").unwrap();
        let kvm = d.path().join("kvm");
        assert_eq!(vm_runtime(&rt.to_string_lossy(), &kvm), None, "no KVM");
        std::fs::write(&kvm, "").unwrap();
        assert_eq!(vm_runtime(&rt.to_string_lossy(), &kvm).as_deref(), Some(rt.to_str().unwrap()));
        assert_eq!(vm_runtime("", &kvm), None);
        assert_eq!(vm_runtime(&d.path().join("none").to_string_lossy(), &kvm), None);
    }

    #[test]
    fn private_files_are_whole_and_only_yours() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s/tokens.json");
        write_private(&p, b"{}", None).unwrap();
        write_private(&p, b"{\"a\": 1}", None).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"{\"a\": 1}");
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(d.path().join("s")).unwrap().count(), 1, "no temporary file left");
    }
}
