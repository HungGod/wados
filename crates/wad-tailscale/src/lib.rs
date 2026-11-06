//! tailscaled, for wadd: the machine on its owner's tailnet.
//!
//! [`LocalApi`] reads tailscaled's LocalAPI over its Unix socket
//! ([`SOCKET`], mode 0666: anyone may read, only root and the operator may
//! change things): [`LocalApi::status`] (the machine's state, its addresses,
//! the devices on the tailnet and whose they are), [`LocalApi::whois`] (which
//! device a connection came from), [`LocalApi::prefs`] (its settings) and
//! [`LocalApi::watch_ipn_bus`] (each change of state as it happens, and the
//! login link). Requests carry the Host the LocalAPI wants ([`HOST`]) and no
//! Origin, which it refuses.
//!
//! [`Cli`] changes things through the `tailscale` CLI, as a person would:
//! joining ([`Cli::up`], `--json`, which hands back the login link, then says
//! when it's running), leaving, the name, and Taildrive's shares. Tailscale's
//! own CLI sets its settings whole, where a hand-made LocalAPI call could
//! leave some at zero. **Taildrive runs as the person** ([`Cli::as_user`]:
//! `runuser -u <user> -- tailscale drive share …`): tailscaled serves a share
//! as the user who asked for it over the LocalAPI, so a share asked for by
//! root would be served as root.
//!
//! The types read what wadd uses and ignore the rest; Go writes empty lists
//! and maps as `null`, which read as empty. The LocalAPI isn't a stable
//! interface: the image pins Tailscale's version (1.104), and `fixtures/`
//! has the shapes these read. **Tagged devices** come back owned by a shared
//! pseudo-user (Tailscale's "tagged-devices"), or by whoever tagged them
//! (Headscale): always look at the tags before comparing owners.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Deserializer};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::sync::mpsc;

#[cfg(feature = "fake")]
pub mod fake;

/// Where tailscaled listens on a machine.
pub const SOCKET: &str = "/run/tailscale/tailscaled.sock";
/// The Host the LocalAPI wants.
pub const HOST: &str = "local-tailscaled.sock";
/// How long a LocalAPI call may take.
const TIMEOUT: Duration = Duration::from_secs(5);
/// `watch-ipn-bus`'s mask: the state now first (NotifyInitialState), then each change.
pub const WATCH_INITIAL_STATE: u64 = 2;
/// Where `runuser` is (Taildrive's commands run as the person through it).
pub const RUNUSER: &str = "/usr/sbin/runuser";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("tailscaled isn't answering: {0}")]
    Unreachable(String),
    #[error("tailscaled answered {status}: {message}")]
    Refused { status: u16, message: String },
    #[error("tailscaled's answer didn't read: {0}")]
    BadAnswer(String),
    /// The `tailscale` CLI failed or couldn't run: what it said.
    #[error("{0}")]
    Cli(String),
}

impl Error {
    /// Taildrive refused to share: the tailnet's policy doesn't give this
    /// machine `drive:share` (tailscaled's 403 says so).
    pub fn drive_not_allowed(&self) -> bool {
        let t = self.to_string();
        t.contains("drive:share") || t.contains("taildrive sharing not enabled")
    }
}

/// A `null` (Go's empty list or map) reads as empty.
fn nullable<'de, D: Deserializer<'de>, T: Default + Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// GET /localapi/v0/status (`ipnstate.Status`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Status {
    #[serde(rename = "Version")]
    pub version: String,
    /// NoState, NeedsLogin, NeedsMachineAuth, Stopped, Starting or Running.
    #[serde(rename = "BackendState")]
    pub backend_state: String,
    /// While logging in: the link to sign in at.
    #[serde(rename = "AuthURL")]
    pub auth_url: String,
    #[serde(rename = "TailscaleIPs", deserialize_with = "nullable")]
    pub tailscale_ips: Vec<String>,
    #[serde(rename = "Self")]
    pub me: Option<Peer>,
    #[serde(rename = "Health", deserialize_with = "nullable")]
    pub health: Vec<String>,
    #[serde(rename = "MagicDNSSuffix")]
    pub magic_dns_suffix: String,
    #[serde(rename = "CurrentTailnet")]
    pub tailnet: Option<Tailnet>,
    /// The other devices, by node key.
    #[serde(rename = "Peer", deserialize_with = "nullable")]
    pub peers: HashMap<String, Peer>,
    /// The people the devices are owned by, by user id.
    #[serde(rename = "User", deserialize_with = "nullable")]
    pub users: HashMap<String, UserProfile>,
    /// The exit node in use (`tailscale set --exit-node`); none: straight out.
    #[serde(rename = "ExitNodeStatus")]
    pub exit_node_status: Option<ExitNodeStatus>,
}

/// The exit node in use (`ipnstate.ExitNodeStatus`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ExitNodeStatus {
    /// Its stable node id.
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "Online")]
    pub online: bool,
    #[serde(rename = "TailscaleIPs", deserialize_with = "nullable")]
    pub tailscale_ips: Vec<String>,
}

/// A device in the status (`ipnstate.PeerStatus`): the machine itself, or another.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Peer {
    /// Its stable node id (what wadd knows a device by).
    #[serde(rename = "ID")]
    pub id: String,
    /// The name it gave itself.
    #[serde(rename = "HostName")]
    pub host_name: String,
    /// Its MagicDNS name, with a dot at the end.
    #[serde(rename = "DNSName")]
    pub dns_name: String,
    #[serde(rename = "OS")]
    pub os: String,
    #[serde(rename = "UserID")]
    pub user_id: i64,
    #[serde(rename = "TailscaleIPs", deserialize_with = "nullable")]
    pub tailscale_ips: Vec<String>,
    #[serde(rename = "Tags", deserialize_with = "nullable")]
    pub tags: Vec<String>,
    #[serde(rename = "Online")]
    pub online: bool,
    /// RFC 3339 ("0001-01-01T00:00:00Z": never).
    #[serde(rename = "LastSeen")]
    pub last_seen: String,
    /// It's here because it was shared in from another tailnet.
    #[serde(rename = "ShareeNode")]
    pub sharee_node: bool,
    #[serde(rename = "Expired")]
    pub expired: bool,
    /// When its key runs out (RFC 3339); None when it never does.
    #[serde(rename = "KeyExpiry")]
    pub key_expiry: Option<String>,
    /// It offers to be an exit node (advertised, and approved).
    #[serde(rename = "ExitNodeOption")]
    pub exit_node_option: bool,
    /// It's the exit node this machine uses.
    #[serde(rename = "ExitNode")]
    pub exit_node: bool,
}

/// The tailnet the machine is on (`ipnstate.TailnetStatus`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Tailnet {
    /// Its name (the netmap's domain: 'example.com', 'you@example.com', or
    /// Headscale's base domain). Taildrive's paths start with it:
    /// `http://100.100.100.100:8080/<name>/<machine>/<share>`.
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "MagicDNSSuffix")]
    pub magic_dns_suffix: String,
    #[serde(rename = "MagicDNSEnabled")]
    pub magic_dns_enabled: bool,
}

/// A person (`tailcfg.UserProfile`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct UserProfile {
    #[serde(rename = "ID")]
    pub id: i64,
    #[serde(rename = "LoginName")]
    pub login_name: String,
    #[serde(rename = "DisplayName")]
    pub display_name: String,
}

/// GET /localapi/v0/whois (`apitype.WhoIsResponse`): the device a connection
/// came from, and whose it is.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WhoIs {
    #[serde(rename = "Node")]
    pub node: Node,
    #[serde(rename = "UserProfile")]
    pub user: UserProfile,
}

/// A device as the control plane knows it (`tailcfg.Node`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Node {
    #[serde(rename = "StableID")]
    pub stable_id: String,
    /// Its MagicDNS name, with a dot at the end.
    #[serde(rename = "Name")]
    pub name: String,
    /// Whose it is. For a tagged device it's the tags' own pseudo-user
    /// (Tailscale) or whoever tagged it (Headscale): look at `tags` first.
    #[serde(rename = "User")]
    pub user: i64,
    /// Who shared it in, from another tailnet (0: nobody).
    #[serde(rename = "Sharer")]
    pub sharer: i64,
    #[serde(rename = "Tags", deserialize_with = "nullable")]
    pub tags: Vec<String>,
    #[serde(rename = "Hostinfo")]
    pub hostinfo: Hostinfo,
    /// Its machine name (its MagicDNS name's first label).
    #[serde(rename = "ComputedName")]
    pub computed_name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Hostinfo {
    #[serde(rename = "OS")]
    pub os: String,
    #[serde(rename = "Hostname")]
    pub hostname: String,
    #[serde(rename = "DeviceModel")]
    pub device_model: String,
}

/// GET /localapi/v0/prefs (`ipn.Prefs`): the settings wadd looks at.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// The control server ('https://controlplane.tailscale.com', or a Headscale's).
    #[serde(rename = "ControlURL")]
    pub control_url: String,
    #[serde(rename = "WantRunning")]
    pub want_running: bool,
    #[serde(rename = "LoggedOut")]
    pub logged_out: bool,
    #[serde(rename = "Hostname")]
    pub hostname: String,
    /// Who besides root may change things.
    #[serde(rename = "OperatorUser")]
    pub operator_user: String,
    #[serde(rename = "ExitNodeID")]
    pub exit_node_id: String,
    #[serde(rename = "DriveShares", deserialize_with = "nullable")]
    pub drive_shares: Vec<PrefsShare>,
}

/// A Taildrive share in the prefs (`drive.Share`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PrefsShare {
    pub name: String,
    pub path: String,
    /// Who it's served as.
    #[serde(rename = "who")]
    pub who: String,
}

/// One change on the IPN bus (`ipn.Notify`): what wadd reads of it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Notify {
    /// The backend's state now ([`state_name`]).
    #[serde(rename = "State")]
    pub state: Option<i64>,
    /// A link to open in a browser (the login's).
    #[serde(rename = "BrowseToURL")]
    pub browse_to_url: Option<String>,
    #[serde(rename = "ErrMessage")]
    pub err_message: Option<String>,
}

/// An IPN bus state's name, as the status's `BackendState` says it.
pub fn state_name(n: i64) -> &'static str {
    match n {
        1 => "InUseOtherUser",
        2 => "NeedsLogin",
        3 => "NeedsMachineAuth",
        4 => "Stopped",
        5 => "Starting",
        6 => "Running",
        _ => "NoState",
    }
}

impl Status {
    /// Joined and up.
    pub fn running(&self) -> bool {
        self.backend_state == "Running"
    }

    /// The machine's tailnet addresses.
    pub fn ips(&self) -> Vec<IpAddr> {
        self.tailscale_ips.iter().filter_map(|a| a.parse().ok()).collect()
    }

    /// Its IPv4 tailnet address.
    pub fn ipv4(&self) -> Option<IpAddr> {
        self.ips().into_iter().find(IpAddr::is_ipv4)
    }

    /// The person the machine is logged in as.
    pub fn me_user(&self) -> Option<&UserProfile> {
        self.users.get(&self.me.as_ref()?.user_id.to_string())
    }

    /// The tailnet's name ("" until it's logged in).
    pub fn tailnet_name(&self) -> &str {
        self.tailnet.as_ref().map(|t| t.name.as_str()).unwrap_or_default()
    }

    /// The other devices, by name.
    pub fn peer_list(&self) -> Vec<&Peer> {
        let mut v: Vec<&Peer> = self.peers.values().collect();
        v.sort_by(|a, b| a.host_name.to_lowercase().cmp(&b.host_name.to_lowercase()).then(a.id.cmp(&b.id)));
        v
    }

    /// The device with this stable id (the machine itself too).
    pub fn peer(&self, id: &str) -> Option<&Peer> {
        self.me.iter().chain(self.peers.values()).find(|p| p.id == id)
    }
}

impl Peer {
    /// Its MagicDNS name without the dot at the end.
    pub fn dns(&self) -> &str {
        self.dns_name.trim_end_matches('.')
    }

    /// Its machine name on the tailnet (its MagicDNS name's first label;
    /// what Taildrive's paths and other devices call it).
    pub fn machine_name(&self) -> String {
        match self.dns().split('.').next() {
            Some(l) if !l.is_empty() => l.to_string(),
            _ => self.host_name.clone(),
        }
    }

    /// Its IPv4 tailnet address.
    pub fn ipv4(&self) -> Option<IpAddr> {
        self.tailscale_ips.iter().filter_map(|a| a.parse::<IpAddr>().ok()).find(IpAddr::is_ipv4)
    }

    /// When it was last seen (ms since the epoch); None: never, or not said.
    pub fn last_seen_ms(&self) -> Option<i64> {
        rfc3339_ms(&self.last_seen)
    }
}

impl WhoIs {
    /// What to call the device: its own name for itself, else its machine name.
    pub fn device_name(&self) -> String {
        let h = &self.node.hostinfo.hostname;
        if !h.is_empty() {
            return h.clone();
        }
        self.machine_name()
    }

    /// Its machine name on the tailnet (its MagicDNS name's first label).
    pub fn machine_name(&self) -> String {
        if !self.node.computed_name.is_empty() {
            return self.node.computed_name.clone();
        }
        self.node.name.split('.').next().unwrap_or_default().to_string()
    }
}

/// tailscaled's LocalAPI.
#[derive(Clone)]
pub struct LocalApi {
    http: reqwest::Client,
    /// For the IPN bus, which streams for as long as it's watched.
    stream: reqwest::Client,
    sock: PathBuf,
}

impl LocalApi {
    pub fn new(sock: impl Into<PathBuf>) -> Self {
        let sock = sock.into();
        let http = reqwest::Client::builder().unix_socket(sock.clone()).timeout(TIMEOUT).build().expect("a client");
        let stream =
            reqwest::Client::builder().unix_socket(sock.clone()).connect_timeout(TIMEOUT).build().expect("a client");
        Self { http, stream, sock }
    }

    pub fn socket(&self) -> &Path {
        &self.sock
    }

    fn url(path: &str) -> String {
        format!("http://{HOST}/localapi/v0/{path}")
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<reqwest::Response, Error> {
        self.http.get(Self::url(path)).query(query).send().await.map_err(|e| Error::Unreachable(e.to_string()))
    }

    async fn read<T: for<'de> Deserialize<'de>>(res: reqwest::Response) -> Result<T, Error> {
        let status = res.status().as_u16();
        let body = res.bytes().await.map_err(|e| Error::Unreachable(e.to_string()))?;
        if !(200..300).contains(&status) {
            let message = String::from_utf8_lossy(&body).trim().to_string();
            return Err(Error::Refused { status, message });
        }
        serde_json::from_slice(&body).map_err(|e| Error::BadAnswer(e.to_string()))
    }

    pub async fn status(&self) -> Result<Status, Error> {
        Self::read(self.get("status", &[]).await?).await
    }

    pub async fn prefs(&self) -> Result<Prefs, Error> {
        Self::read(self.get("prefs", &[]).await?).await
    }

    /// Which device the TCP connection from `addr` is from; None when it isn't
    /// one on the tailnet. (With tailscaled in userspace mode, a connection it
    /// forwarded arrives from a loopback port, which tailscaled maps back.)
    pub async fn whois(&self, addr: SocketAddr) -> Result<Option<WhoIs>, Error> {
        let res = self.get("whois", &[("addr", &addr.to_string()), ("proto", "tcp")]).await?;
        if res.status().as_u16() == 404 {
            return Ok(None);
        }
        Self::read(res).await.map(Some)
    }

    /// The IPN bus: with `mask` [`WATCH_INITIAL_STATE`], the state now, then
    /// each change, for as long as it's read.
    pub async fn watch_ipn_bus(&self, mask: u64) -> Result<IpnBus, Error> {
        let res = self
            .stream
            .get(Self::url("watch-ipn-bus"))
            .query(&[("mask", mask.to_string())])
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        let status = res.status().as_u16();
        if !(200..300).contains(&status) {
            let message = res.text().await.unwrap_or_default().trim().to_string();
            return Err(Error::Refused { status, message });
        }
        Ok(IpnBus { res, buf: vec![] })
    }
}

/// The IPN bus being watched: one [`Notify`] per line.
pub struct IpnBus {
    res: reqwest::Response,
    buf: Vec<u8>,
}

impl IpnBus {
    /// The next change; None when tailscaled closed the stream.
    pub async fn next(&mut self) -> Option<Result<Notify, Error>> {
        loop {
            if let Some(i) = self.buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=i).collect();
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                return Some(serde_json::from_slice(&line).map_err(|e| Error::BadAnswer(e.to_string())));
            }
            match self.res.chunk().await {
                Ok(Some(c)) => self.buf.extend_from_slice(&c),
                Ok(None) => return None,
                Err(e) => return Some(Err(Error::Unreachable(e.to_string()))),
            }
        }
    }
}

/// The `tailscale` CLI, pointed at one tailscaled, run as wadd or as someone.
#[derive(Debug, Clone)]
pub struct Cli {
    bin: PathBuf,
    sock: PathBuf,
    runuser: PathBuf,
    /// Run as them (`runuser -u <user> --`); None: as wadd.
    user: Option<String>,
}

/// How to join ([`Cli::up`]).
#[derive(Debug, Clone, Default)]
pub struct UpOptions {
    /// The machine's name on the tailnet.
    pub hostname: Option<String>,
    /// The control server (a Headscale's); None: Tailscale's.
    pub login_server: Option<String>,
    /// How long it waits for the sign-in (and an admin's approval).
    pub timeout: Duration,
}

/// What a `tailscale up --json` says as it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpEvent {
    /// Sign in here.
    AuthUrl(String),
    /// The backend's state ('NeedsMachineAuth', 'Running').
    State(String),
    /// It gave up (its `Error`).
    Error(String),
}

/// A `tailscale up` under way: what it says ([`Login::next`]), then how it
/// ended ([`Login::wait`]). Dropping it stops it (the process is killed).
pub struct Login {
    child: tokio::process::Child,
    events: mpsc::UnboundedReceiver<UpEvent>,
    /// What it printed besides its JSON (stderr, and stdout's other lines).
    text: Arc<Mutex<String>>,
}

/// A Taildrive share (`tailscale drive list`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    pub name: String,
    pub path: String,
    /// Who it's served as.
    pub as_user: String,
}

impl Cli {
    pub fn new(bin: impl Into<PathBuf>, sock: impl Into<PathBuf>) -> Self {
        Self { bin: bin.into(), sock: sock.into(), runuser: RUNUSER.into(), user: None }
    }

    /// `runuser` from here instead (tests).
    pub fn with_runuser(mut self, runuser: impl Into<PathBuf>) -> Self {
        self.runuser = runuser.into();
        self
    }

    /// The same CLI, run as `user` (`runuser -u <user> --`): what Taildrive's
    /// commands need, since tailscaled serves a share as the user who asked.
    pub fn as_user(&self, user: &str) -> Self {
        Self { user: Some(user.to_string()), ..self.clone() }
    }

    fn command(&self) -> tokio::process::Command {
        let mut c = match &self.user {
            Some(u) => {
                let mut c = tokio::process::Command::new(&self.runuser);
                c.args(["-u", u, "--"]).arg(&self.bin);
                c
            }
            None => tokio::process::Command::new(&self.bin),
        };
        c.arg(format!("--socket={}", self.sock.display())).stdin(Stdio::null()).kill_on_drop(true);
        c
    }

    /// Join (or log in again): `tailscale up --json`, with every setting at
    /// its default but the name and the control server (`--reset`), MagicDNS
    /// included: your machines reach each other by name (`wados-b.<tailnet>`,
    /// as Files and Settings show them; wadd itself uses addresses). It waits
    /// up to `timeout` for the sign-in at the link
    /// it gives ([`UpEvent::AuthUrl`]), and for an admin's approval if the
    /// tailnet wants one, then says Running and ends.
    pub fn up(&self, opts: &UpOptions) -> Result<Login, Error> {
        let mut c = self.command();
        c.args(["up", "--json", "--reset"]).arg(format!("--timeout={}s", opts.timeout.as_secs()));
        if let Some(h) = opts.hostname.as_deref().filter(|h| !h.is_empty()) {
            c.arg(format!("--hostname={h}"));
        }
        if let Some(s) = opts.login_server.as_deref().filter(|s| !s.is_empty()) {
            c.arg(format!("--login-server={s}"));
        }
        let mut child = c
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::Cli(format!("tailscale didn't run: {e}")))?;
        let text = Arc::new(Mutex::new(String::new()));
        let (tx, events) = mpsc::unbounded_channel();
        if let Some(o) = child.stdout.take() {
            tokio::spawn(read_up_json(o, tx, text.clone()));
        }
        if let Some(e) = child.stderr.take() {
            tokio::spawn(collect(e, text.clone()));
        }
        Ok(Login { child, events, text })
    }

    /// Leave: the machine's key is let go of, and it's off the tailnet.
    pub async fn logout(&self) -> Result<(), Error> {
        self.run(&["logout"]).await.map(drop)
    }

    /// Its name on the tailnet.
    pub async fn set_hostname(&self, hostname: &str) -> Result<(), Error> {
        self.run(&["set", &format!("--hostname={hostname}")]).await.map(drop)
    }

    /// This machine's internet traffic out through `node` (a peer's
    /// address or name: `tailscale set --exit-node`), still reaching its own
    /// network; none: straight out again.
    pub async fn set_exit_node(&self, node: Option<&str>) -> Result<(), Error> {
        match node {
            Some(n) => {
                self.run(&["set", &format!("--exit-node={n}"), "--exit-node-allow-lan-access=true"]).await.map(drop)
            }
            None => self.run(&["set", "--exit-node="]).await.map(drop),
        }
    }

    /// Shares `path` on the tailnet as `name` (Taildrive), served as the
    /// user this runs as ([`Cli::as_user`]). A tailnet whose policy doesn't
    /// give the machine `drive:share` refuses ([`Error::drive_not_allowed`]).
    pub async fn drive_share(&self, name: &str, path: &Path) -> Result<(), Error> {
        let path = path.to_string_lossy();
        self.run(&["drive", "share", name, &path]).await.map(drop)
    }

    /// Stops sharing `name`.
    pub async fn drive_unshare(&self, name: &str) -> Result<(), Error> {
        self.run(&["drive", "unshare", name]).await.map(drop)
    }

    /// The machine's shares.
    pub async fn drive_list(&self) -> Result<Vec<Share>, Error> {
        Ok(parse_drive_list(&self.run(&["drive", "list"]).await?))
    }

    /// Serves `target` (a local http:// address) on the tailnet at
    /// https://<this machine>:<port>/, with Tailscale's certificate, in the
    /// background (`tailscale serve --bg`), until [`Cli::serve_off`]. Only
    /// devices on the tailnet reach it. A tailnet without Serve or HTTPS
    /// certificates refuses: the error has the link to turn them on.
    pub async fn serve(&self, port: u16, target: &str) -> Result<(), Error> {
        let https = format!("--https={port}");
        let mut child = self
            .command()
            .args(["serve", "--bg", "--yes", &https, target])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::Cli(format!("tailscale didn't run: {e}")))?;
        // What it says as it goes: with Serve or HTTPS off for the tailnet it
        // prints the link to turn it on, then waits there (not that long).
        let out = Arc::new(Mutex::new(String::new()));
        let mut readers = vec![];
        if let Some(o) = child.stdout.take() {
            readers.push(tokio::spawn(collect(o, out.clone())));
        }
        if let Some(e) = child.stderr.take() {
            readers.push(tokio::spawn(collect(e, out.clone())));
        }
        let done = match tokio::time::timeout(Duration::from_secs(25), child.wait()).await {
            Ok(r) => Some(r.map_err(|e| Error::Cli(format!("tailscale didn't run: {e}")))?),
            Err(_) => {
                let _ = child.kill().await;
                None
            }
        };
        for r in readers {
            let _ = tokio::time::timeout(Duration::from_secs(1), r).await;
        }
        let text = out.lock().unwrap().clone();
        let Some(status) = done else {
            return Err(Error::Cli(match url_in(&text) {
                Some(url) => {
                    format!("Tailscale is waiting for Serve (or HTTPS) to be turned on for your tailnet: {url}")
                }
                None => "tailscale serve took too long".into(),
            }));
        };
        if status.success() {
            return Ok(());
        }
        Err(Error::Cli(match url_in(&text) {
            Some(url) if text.contains("HTTPS") || text.contains("enable") => {
                format!("HTTPS isn't on for your tailnet: turn it on at {url}")
            }
            _ if text.trim().is_empty() => "tailscale serve failed".into(),
            _ => text.trim().to_string(),
        }))
    }

    /// Stops serving on `port`.
    pub async fn serve_off(&self, port: u16) -> Result<(), Error> {
        self.run(&["serve", &format!("--https={port}"), "off"]).await.map(drop)
    }

    /// The ports served now (`tailscale serve status --json`).
    pub async fn served(&self) -> Result<Vec<u16>, Error> {
        Ok(served_ports(&self.run(&["serve", "status", "--json"]).await?))
    }

    /// Runs it to the end: what it printed, or its complaint.
    async fn run(&self, args: &[&str]) -> Result<String, Error> {
        let what = args.iter().take(2).copied().collect::<Vec<_>>().join(" ");
        let out = tokio::time::timeout(Duration::from_secs(30), self.command().args(args).output())
            .await
            .map_err(|_| Error::Cli(format!("tailscale {what} took too long")))?
            .map_err(|e| Error::Cli(format!("tailscale didn't run: {e}")))?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        let text = String::from_utf8_lossy(&out.stderr);
        let text = text.trim();
        Err(Error::Cli(if text.is_empty() { format!("tailscale {what} failed") } else { text.to_string() }))
    }
}

/// What `tailscale up --json` prints, each time (`upOutputJSON`).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UpJson {
    #[serde(rename = "AuthURL")]
    auth_url: String,
    #[serde(rename = "BackendState")]
    backend_state: String,
    #[serde(rename = "Error")]
    error: String,
}

impl UpJson {
    fn events(self) -> Vec<UpEvent> {
        let mut out = vec![];
        if !self.error.is_empty() {
            out.push(UpEvent::Error(self.error));
        }
        if !self.auth_url.is_empty() {
            out.push(UpEvent::AuthUrl(self.auth_url));
        } else if !self.backend_state.is_empty() {
            out.push(UpEvent::State(self.backend_state));
        }
        out
    }
}

/// The JSON objects in what `up --json` has printed so far (`buf`), taken
/// out as they're whole; other lines (warnings) go to `text`.
fn take_up_json(buf: &mut Vec<u8>, text: &Mutex<String>) -> Vec<UpEvent> {
    let mut out = vec![];
    loop {
        let Some(start) = buf.iter().position(|b| !b.is_ascii_whitespace()) else {
            buf.clear();
            return out;
        };
        let skip_line = |buf: &mut Vec<u8>| -> bool {
            let Some(i) = buf[start..].iter().position(|b| *b == b'\n') else { return false };
            let line: Vec<u8> = buf.drain(..start + i + 1).collect();
            let mut t = text.lock().unwrap();
            t.push_str(String::from_utf8_lossy(&line).trim());
            t.push('\n');
            true
        };
        if buf[start] != b'{' {
            if skip_line(buf) {
                continue;
            }
            return out;
        }
        let mut values = serde_json::Deserializer::from_slice(&buf[start..]).into_iter::<UpJson>();
        match values.next() {
            Some(Ok(v)) => {
                let used = start + values.byte_offset();
                buf.drain(..used);
                out.extend(v.events());
            }
            Some(Err(e)) if e.is_eof() => return out,
            Some(Err(_)) => {
                if !skip_line(buf) {
                    return out;
                }
            }
            None => return out,
        }
    }
}

async fn read_up_json(mut from: impl AsyncRead + Unpin, tx: mpsc::UnboundedSender<UpEvent>, text: Arc<Mutex<String>>) {
    let mut buf = vec![];
    let mut chunk = [0u8; 4096];
    loop {
        let n = match from.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..n]);
        for e in take_up_json(&mut buf, &text) {
            let _ = tx.send(e);
        }
    }
    // What's left that isn't JSON.
    buf.push(b'\n');
    for e in take_up_json(&mut buf, &text) {
        let _ = tx.send(e);
    }
}

async fn collect(from: impl AsyncRead + Unpin, into: Arc<Mutex<String>>) {
    let mut lines = BufReader::new(from).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let mut o = into.lock().unwrap();
        o.push_str(&line);
        o.push('\n');
    }
}

impl Login {
    /// The next thing it says; None once it has nothing more to say (it's
    /// ending: [`Login::wait`]).
    pub async fn next(&mut self) -> Option<UpEvent> {
        self.events.recv().await
    }

    /// What it printed besides its JSON.
    pub fn output(&self) -> String {
        self.text.lock().unwrap().clone()
    }

    /// Wait for it to end: Ok once the machine is up, or its complaint.
    pub async fn wait(mut self) -> Result<(), Error> {
        let status = self.child.wait().await.map_err(|e| Error::Cli(e.to_string()))?;
        // Its last lines are read once it's gone.
        tokio::time::sleep(Duration::from_millis(50)).await;
        if status.success() {
            return Ok(());
        }
        let out = self.output();
        let last = out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("tailscale up failed");
        Err(Error::Cli(last.trim().to_string()))
    }

    /// Give up waiting (the process is killed).
    pub async fn cancel(mut self) {
        let _ = self.child.kill().await;
    }
}

/// The shares in `tailscale drive list`'s table (a header, a line of dashes
/// as wide as each column, then a share a line).
pub fn parse_drive_list(text: &str) -> Vec<Share> {
    let mut lines = text.lines().skip_while(|l| !l.trim_start().starts_with("name"));
    let (Some(_), Some(dashes)) = (lines.next(), lines.next()) else { return vec![] };
    // Where each column starts: after a run of dashes and the spaces after it.
    let mut starts = vec![];
    let mut in_dash = false;
    for (i, c) in dashes.char_indices() {
        if c == '-' && !in_dash {
            starts.push(i);
        }
        in_dash = c == '-';
    }
    if starts.len() < 3 {
        return vec![];
    }
    lines
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let col = |a: usize, b: Option<usize>| -> Option<String> {
                let s = match b {
                    Some(b) => l.get(a..b.min(l.len()))?,
                    None => l.get(a..)?,
                };
                Some(s.trim().to_string())
            };
            Some(Share {
                name: col(starts[0], Some(starts[1]))?,
                path: col(starts[1], Some(starts[2]))?,
                as_user: col(starts[2], None).unwrap_or_default(),
            })
        })
        .collect()
}

/// The ports in `tailscale serve status --json` ({"TCP": {"443": {...}}}).
pub fn served_ports(json: &str) -> Vec<u16> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let mut ports: Vec<u16> =
        v.get("TCP").and_then(|t| t.as_object()).into_iter().flatten().filter_map(|(k, _)| k.parse().ok()).collect();
    ports.sort_unstable();
    ports
}

/// The first https link in `text`.
pub fn url_in(text: &str) -> Option<String> {
    text.split_whitespace().find(|w| w.starts_with("https://") && w.len() > "https://".len()).map(String::from)
}

/// An RFC 3339 time ('2026-10-18T04:00:00.5Z', '…+12:00') as ms since the
/// epoch; None for Go's zero time (year 1: never) or what doesn't read.
pub fn rfc3339_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse().ok() };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if y < 1970 || !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let mut rest = &s[19..];
    let mut ms = 0;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits = frac.bytes().take_while(u8::is_ascii_digit).count();
        let f = &frac[..digits];
        ms = f.chars().chain("000".chars()).take(3).collect::<String>().parse().ok()?;
        rest = &frac[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        o if o.len() == 6 && (o.starts_with('+') || o.starts_with('-')) => {
            let oh: i64 = o.get(1..3)?.parse().ok()?;
            let om: i64 = o.get(4..6)?.parse().ok()?;
            let v = (oh * 60 + om) * 60;
            if o.starts_with('-') { -v } else { v }
        }
        _ => return None,
    };
    // Days from the civil date (Howard Hinnant's).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + h * 3600 + mi * 60 + sec) - offset) * 1000 + ms)
}

/// The fixtures (`fixtures/`), for tests here and in wadd.
#[cfg(any(test, feature = "fake"))]
pub mod fixtures {
    pub const STATUS_RUNNING: &str = include_str!("../fixtures/status-running.json");
    pub const STATUS_NEEDS_LOGIN: &str = include_str!("../fixtures/status-needs-login.json");
    pub const WHOIS_PHONE: &str = include_str!("../fixtures/whois-phone.json");
    pub const WHOIS_DESK: &str = include_str!("../fixtures/whois-desk.json");
    pub const WHOIS_TAGGED: &str = include_str!("../fixtures/whois-tagged.json");
    pub const WHOIS_TAGGED_OWNER: &str = include_str!("../fixtures/whois-tagged-owner.json");
    pub const WHOIS_SHARED: &str = include_str!("../fixtures/whois-shared.json");
    pub const PREFS: &str = include_str!("../fixtures/prefs.json");
    pub const WATCH_IPN_BUS: &str = include_str!("../fixtures/watch-ipn-bus.jsonl");
    pub const UP_JSON: &str = include_str!("../fixtures/up-json.txt");
}

#[cfg(test)]
mod tests;
