//! My other machines on the tailnet (wadds of the same login), their
//! wadspaces and Copy here; and [`TailnetPeers`], the tailnet, peers and
//! remote API together as wadd's [`Net`] (the socket's routes in api_phase6.rs).
//!
//! **Notices go round.** Every notice this machine tells is posted to each of
//! my online machines (`POST /v1/remote/notices`, [`Relay`]), so an agent
//! asking a question is heard on every screen; one that came from another
//! machine isn't passed on again (`Bus::adopt` keeps its id).
//!
//! **Which nodes are my machines** ([`Peers::round`], every `poll_s` and
//! whenever tailscaled's state changes): from the tailnet's status, the
//! nodes of this machine's login that are untagged and not shared in; each
//! online one is said hello to (`GET /v1/remote/hello` on its tailnet
//! address, port 8087, [`HELLO_TIMEOUT`]); the ones that answer as a wadd are
//! my machines, kept by node stable id (`<state>/tailnet-peers.json`, 0600,
//! wiped with the trust book when the login or tailnet changes) and fed to
//! tailnet.rs (trusted without an ask, and in the devices list). An offline
//! one is kept, with when it was last seen; one that's gone from the
//! tailnet goes. A node that isn't a wadd is asked again after a while.
//!
//! **Every machine of mine follows an answer** about a device: each one made
//! here is pushed to my online machines (`POST /v1/remote/trust`), and when
//! one of mine comes online the books are swapped (`GET /v1/remote/trust`,
//! merged here; the answers it lacks, or has older, posted to it).
//!
//! **A peer's home over Taildrive:** `dav://100.100.100.100:8080/<tailnet>/<hostname>/home`
//! (GIO; `http://…` for other WebDAV clients, taildrive.rs's `home_url`):
//! `<tailnet>` is `GET /v1/tailnet`'s `tailnet` (Tailscale's
//! `CurrentTailnet.Name`, the netmap's domain), `<hostname>` the peer's
//! [`PeerInfo`] `hostname` (its MagicDNS name's first label, which is what
//! Taildrive names a machine). tailscaled serves it on this machine at
//! 100.100.100.100:8080 to anyone here; the peer must be online, and the
//! tailnet's policy must give it `drive:share` and this machine `drive:access`.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wad_model::edit::Op;
use wad_model::{JobKind, PartState, Peer, PeerKind, Wadspace};
use wad_proto::{
    Answer, ApiError, DeviceKind, ErrorCode, Event, JobInfo, MachineInfo, PeerInfo, RemoteHello, RemoteMachine,
    RemoteNotice, StreamInfo, StreamRequest, StreamShare, Tailnet as View, TrustAnswer, TrustBook,
};

use crate::api::AppState;
use crate::api_phase6::Net;
use crate::builds::Builds;
use crate::events::Bus;
use crate::jobs::{Jobs, now_ms};
use crate::library::Docs;
use crate::tailnet::{self, Caller, Class, Me, MyMachine, PersonFn, Refusal, Tailnet};

/// How long a node has to answer hello.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(2);
/// Any other call to a peer's remote API.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// Streaming one of its wadspaces: it answers once the stream is live
/// (its wadspace started, its sidecar up, the tailnet serving it).
const STREAM_TIMEOUT: Duration = Duration::from_secs(600);
/// An online machine of mine is said hello to again after this.
const RECHECK: Duration = Duration::from_secs(30);
/// A node that answered, but not as a wadd, is asked again after this.
const NOT_WADD_FOR: Duration = Duration::from_secs(300);
/// One that didn't answer at all (nothing on 8087: a phone, or a wadd not
/// started yet), after this.
const SILENT_FOR: Duration = Duration::from_secs(15);

/// One of my machines, as last seen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Known {
    /// Its node's stable id.
    node: String,
    /// Its wadd machine id.
    id: String,
    name: String,
    /// Its machine name on the tailnet (Taildrive's).
    hostname: String,
    dns_name: String,
    /// Its tailnet address (its remote API's).
    ip: String,
    kind: PeerKind,
    #[serde(default)]
    os: String,
    #[serde(default)]
    gpu: Option<String>,
    last_seen: i64,
    #[serde(skip)]
    online: bool,
    #[serde(skip)]
    checked: Option<Instant>,
}

impl Known {
    fn info(&self) -> PeerInfo {
        PeerInfo {
            peer: Peer {
                id: self.id.clone(),
                name: self.name.clone(),
                hostname: self.hostname.clone(),
                kind: self.kind,
                trusted: true,
                online: self.online,
                last_seen: self.last_seen,
                os: self.os.clone(),
                gpu: self.gpu.clone(),
            },
            node_id: self.node.clone(),
            dns_name: self.dns_name.clone(),
        }
    }

    fn addr(&self, port: u16) -> Option<SocketAddr> {
        Some(SocketAddr::new(self.ip.parse().ok()?, port))
    }
}

/// `<state>/tailnet-peers.json`: my machines, for one login on one tailnet.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    user: String,
    tailnet: String,
    machines: Vec<Known>,
}

#[derive(Default)]
struct St {
    saved: Saved,
    /// What was last written.
    written: Vec<u8>,
    /// Nodes of the login's that aren't wadds of mine, as last found out.
    seen: HashMap<String, Seen>,
    /// Nodes being said hello to now.
    probing: HashSet<String>,
    published: Option<Vec<PeerInfo>>,
}

/// What a node that isn't one of my machines did when it was said hello to.
#[derive(Debug, Clone, Copy)]
enum Seen {
    /// It answered, not as a wadd.
    NotWadd(Instant),
    /// Nothing answered.
    Silent(Instant),
}

impl Seen {
    /// It's not worth asking again yet.
    fn fresh(self) -> bool {
        match self {
            Seen::NotWadd(t) => t.elapsed() < NOT_WADD_FOR,
            Seen::Silent(t) => t.elapsed() < SILENT_FOR,
        }
    }
}

pub struct Peers {
    tailnet: Arc<Tailnet>,
    /// This machine's wadd id.
    machine: String,
    port: u16,
    file: PathBuf,
    /// Calls to peers, from this machine's tailnet address.
    http: Mutex<(Option<IpAddr>, reqwest::Client)>,
    st: Mutex<St>,
    bus: Bus,
    jobs: Arc<Jobs>,
    docs: Arc<Docs>,
    builds: Arc<Builds>,
}

fn client(from: Option<IpAddr>) -> reqwest::Client {
    reqwest::Client::builder().no_proxy().local_address(from).build().expect("a client")
}

fn offline(name: &str) -> ApiError {
    ApiError::new(ErrorCode::Offline, format!("{name} is offline"))
}

fn kind_of(k: PeerKind) -> DeviceKind {
    match k {
        PeerKind::Desktop => DeviceKind::Desktop,
        PeerKind::Laptop => DeviceKind::Laptop,
        PeerKind::Server => DeviceKind::Server,
    }
}

impl Peers {
    pub fn new(
        tailnet: Arc<Tailnet>,
        machine: &str,
        state_dir: &std::path::Path,
        bus: Bus,
        jobs: Arc<Jobs>,
        docs: Arc<Docs>,
        builds: Arc<Builds>,
    ) -> Arc<Self> {
        let file = state_dir.join("tailnet-peers.json");
        let saved: Saved = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let written = serde_json::to_vec_pretty(&saved).unwrap_or_default();
        Arc::new(Self {
            port: tailnet.port(),
            tailnet,
            machine: machine.to_string(),
            file,
            http: Mutex::new((None, client(None))),
            st: Mutex::new(St { saved, written, ..Default::default() }),
            bus,
            jobs,
            docs,
            builds,
        })
    }

    fn client(&self) -> reqwest::Client {
        let ip = self.tailnet.ipv4();
        let mut h = self.http.lock().unwrap();
        if h.0 != ip {
            *h = (ip, client(ip));
        }
        h.1.clone()
    }

    /// GET from a peer's remote API. A peer that doesn't answer is offline;
    /// what it refuses is said as it said it.
    async fn get<T: DeserializeOwned>(&self, to: SocketAddr, path: &str, timeout: Duration) -> Result<T, ApiError> {
        let res = self.client().get(format!("http://{to}{path}")).timeout(timeout).send().await;
        Self::read(to, res).await
    }

    async fn post<B: Serialize>(&self, to: SocketAddr, path: &str, body: &B) -> Result<Value, ApiError> {
        let res = self.client().post(format!("http://{to}{path}")).timeout(CALL_TIMEOUT).json(body).send().await;
        Self::read(to, res).await
    }

    /// Any call to a peer's remote API, with a body or not, waiting `timeout`.
    async fn call<B: Serialize, T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        to: SocketAddr,
        path: &str,
        body: Option<&B>,
        timeout: Duration,
    ) -> Result<T, ApiError> {
        let mut req = self.client().request(method, format!("http://{to}{path}")).timeout(timeout);
        if let Some(b) = body {
            req = req.json(b);
        }
        Self::read(to, req.send().await).await
    }

    async fn read<T: DeserializeOwned>(
        to: SocketAddr,
        res: Result<reqwest::Response, reqwest::Error>,
    ) -> Result<T, ApiError> {
        let res = res.map_err(|e| ApiError::new(ErrorCode::Offline, format!("{to} isn't answering: {e}")))?;
        let status = res.status();
        let body = res.bytes().await.map_err(|e| ApiError::new(ErrorCode::Offline, e.to_string()))?;
        if !status.is_success() {
            return Err(serde_json::from_slice::<ApiError>(&body)
                .unwrap_or_else(|_| ApiError::new(ErrorCode::Upstream, format!("{to} answered {status}"))));
        }
        serde_json::from_slice(&body).map_err(|e| ApiError::new(ErrorCode::Upstream, format!("{to}'s answer: {e}")))
    }

    /// Whether this node is one of my machines, as known now: None when it
    /// has to be found out (it was never said hello to, or it didn't answer).
    pub fn known(&self, node: &str) -> Option<bool> {
        let st = self.st.lock().unwrap();
        if st.saved.machines.iter().any(|m| m.node == node) {
            return Some(true);
        }
        matches!(st.seen.get(node), Some(s @ Seen::NotWadd(_)) if s.fresh()).then_some(false)
    }

    /// Says hello to the node now (by its stable id, from the status): whether it's a wadd of mine.
    pub async fn check_node(self: &Arc<Self>, node: &str) -> bool {
        let Some(p) = self.tailnet.status().and_then(|s| s.peer(node).cloned()) else { return false };
        let ok = self.check(&p).await;
        self.publish();
        ok
    }

    /// [`Peers::check_node`], unless it's being said hello to already (None).
    pub async fn check_node_once(self: &Arc<Self>, node: &str) -> Option<bool> {
        if !self.st.lock().unwrap().probing.insert(node.to_string()) {
            return None;
        }
        let ok = self.check_node(node).await;
        self.st.lock().unwrap().probing.remove(node);
        Some(ok)
    }

    /// Says hello to `p`: one of my machines if it answers as a wadd (kept
    /// at once, so its own calls here count as mine; then its card). One
    /// that came online swaps trust books with this one.
    async fn check(self: &Arc<Self>, p: &wad_tailscale::Peer) -> bool {
        let Some(ip) = p.ipv4() else { return false };
        let to = SocketAddr::new(ip, self.port);
        let hello = self.get::<RemoteHello>(to, "/v1/remote/hello", HELLO_TIMEOUT).await;
        match hello {
            Ok(h) if h.wadd && h.machine.id != self.machine => {
                if self.record(p, ip, &h.machine, None) {
                    let (me, node) = (self.clone(), p.id.clone());
                    tokio::spawn(async move { me.exchange(&node).await });
                }
                if let Ok(m) = self.get::<RemoteMachine>(to, "/v1/remote/machine", CALL_TIMEOUT).await {
                    self.record(p, ip, &h.machine, Some(m));
                }
                true
            }
            other => {
                let mut st = self.st.lock().unwrap();
                match st.saved.machines.iter_mut().find(|m| m.node == p.id) {
                    // Mine, but its wadd isn't answering.
                    Some(m) => {
                        m.online = false;
                        m.checked = Some(Instant::now());
                    }
                    None => {
                        let silent = matches!(&other, Err(e) if e.code == ErrorCode::Offline);
                        let seen = if silent { Seen::Silent(Instant::now()) } else { Seen::NotWadd(Instant::now()) };
                        st.seen.insert(p.id.clone(), seen);
                    }
                }
                false
            }
        }
    }

    /// Keeps `p` as one of my machines, online: whether it came online.
    fn record(&self, p: &wad_tailscale::Peer, ip: IpAddr, hello: &MachineInfo, card: Option<RemoteMachine>) -> bool {
        let mut st = self.st.lock().unwrap();
        st.seen.remove(&p.id);
        let i = match st.saved.machines.iter().position(|m| m.node == p.id) {
            Some(i) => i,
            None => {
                tracing::info!("peers: {} ({}) is one of my machines", hello.name, p.dns());
                st.saved.machines.push(Known {
                    node: p.id.clone(),
                    id: hello.id.clone(),
                    name: hello.name.clone(),
                    hostname: String::new(),
                    dns_name: String::new(),
                    ip: String::new(),
                    kind: PeerKind::Desktop,
                    os: String::new(),
                    gpu: None,
                    last_seen: 0,
                    online: false,
                    checked: None,
                });
                st.saved.machines.len() - 1
            }
        };
        let m = &mut st.saved.machines[i];
        let came = !m.online;
        m.id = hello.id.clone();
        m.name = hello.name.clone();
        m.hostname = p.machine_name();
        m.dns_name = p.dns().to_string();
        m.ip = ip.to_string();
        m.online = true;
        m.checked = Some(Instant::now());
        m.last_seen = now_ms();
        if let Some(c) = card {
            m.name = c.name;
            m.kind = c.kind;
            m.os = c.os;
            m.gpu = c.gpu;
        }
        // Until its card comes (or if it had none): what Tailscale says it runs.
        if m.os.is_empty() {
            m.os = p.os.clone();
        }
        came
    }

    /// My machines, by name (`GET /v1/peers`).
    pub fn list(&self) -> Vec<PeerInfo> {
        let st = self.st.lock().unwrap();
        let mut v: Vec<PeerInfo> = st.saved.machines.iter().map(Known::info).collect();
        v.sort_by(|a, b| a.peer.name.to_lowercase().cmp(&b.peer.name.to_lowercase()).then(a.peer.id.cmp(&b.peer.id)));
        v
    }

    /// Saves the list if it changed, and tells tailnet.rs and the watchers.
    fn publish(&self) {
        let peers = self.list();
        let (mine, changed) = {
            let mut st = self.st.lock().unwrap();
            let bytes = serde_json::to_vec_pretty(&st.saved).unwrap_or_default();
            if bytes != st.written {
                if let Err(e) = tailnet::save_private(&self.file, &st.saved) {
                    tracing::warn!("peers: {}: {e}", self.file.display());
                }
                st.written = bytes;
            }
            let mine: Vec<MyMachine> = st
                .saved
                .machines
                .iter()
                .map(|m| MyMachine {
                    node: m.node.clone(),
                    name: m.name.clone(),
                    os: m.os.clone(),
                    kind: kind_of(m.kind),
                    online: m.online,
                })
                .collect();
            let changed = st.published.as_ref() != Some(&peers);
            if changed {
                st.published = Some(peers.clone());
            }
            (mine, changed)
        };
        self.tailnet.set_machines(mine);
        if changed {
            self.bus.publish(Event::Peers { peers });
        }
    }

    /// Another login, or another tailnet: no machine of the last one's is mine.
    fn check_owner(&self, me: &Me) {
        let mut st = self.st.lock().unwrap();
        if st.saved.user == me.login && st.saved.tailnet == me.tailnet {
            return;
        }
        st.saved = Saved { user: me.login.clone(), tailnet: me.tailnet.clone(), machines: vec![] };
        st.seen.clear();
    }

    /// Logged out: no machines.
    pub fn forget(&self) {
        {
            let mut st = self.st.lock().unwrap();
            st.saved.machines.clear();
            st.seen.clear();
        }
        self.publish();
    }

    /// Looks at the tailnet again: who's online, who's a wadd of mine; the
    /// books swapped with each machine of mine that came online.
    pub async fn round(self: &Arc<Self>) {
        let status = self.tailnet.status().filter(|s| s.running());
        let (Some(status), Some(me)) = (status, self.tailnet.me()) else {
            for m in &mut self.st.lock().unwrap().saved.machines {
                m.online = false;
            }
            self.publish();
            return;
        };
        self.check_owner(&me);
        let own: Vec<&wad_tailscale::Peer> = status
            .peer_list()
            .into_iter()
            .filter(|p| p.tags.is_empty() && !p.sharee_node && p.user_id == me.user)
            .collect();
        let to_check = {
            let mut st = self.st.lock().unwrap();
            // Gone from the tailnet (or not the login's own any more): not mine.
            st.saved.machines.retain(|m| own.iter().any(|p| p.id == m.node));
            for m in &mut st.saved.machines {
                let p = own.iter().find(|p| p.id == m.node).expect("kept");
                if !p.online {
                    m.online = false;
                    if let Some(t) = p.last_seen_ms() {
                        m.last_seen = m.last_seen.max(t);
                    }
                }
            }
            let to_check: Vec<wad_tailscale::Peer> = own
                .iter()
                .filter(|p| p.online)
                .filter(|p| match st.saved.machines.iter().find(|m| m.node == p.id) {
                    Some(m) => !m.online || m.checked.is_none_or(|t| t.elapsed() >= RECHECK),
                    None => st.seen.get(&p.id).is_none_or(|s| !s.fresh()) && !st.probing.contains(&p.id),
                })
                .map(|p| (*p).clone())
                .collect::<Vec<_>>();
            to_check
        };
        futures_util::future::join_all(to_check.iter().map(|p| self.check(p))).await;
        self.publish();
    }

    /// Rounds until wadd stops: every `poll_s`, and when tailscaled's state changes.
    pub async fn run(self: Arc<Self>) {
        let mut changes = self.tailnet.changes();
        loop {
            self.round().await;
            tokio::select! {
                _ = tokio::time::sleep(self.tailnet.poll()) => {}
                r = changes.changed() => if r.is_err() { return },
            }
        }
    }

    fn machine(&self, id: &str) -> Result<Known, ApiError> {
        let st = self.st.lock().unwrap();
        st.saved
            .machines
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or_else(|| ApiError::not_found(format!("There's no machine {id:?} of yours on the tailnet")))
    }

    /// An online machine of mine, and its remote API.
    fn reachable(&self, id: &str) -> Result<(Known, SocketAddr), ApiError> {
        let m = self.machine(id)?;
        match m.addr(self.port) {
            Some(a) if m.online => Ok((m, a)),
            _ => Err(offline(&m.name)),
        }
    }

    /// A machine of mine didn't answer: it's offline until it does.
    fn unanswered(&self, node: &str, e: &ApiError) {
        if e.code != ErrorCode::Offline {
            return;
        }
        if let Some(m) = self.st.lock().unwrap().saved.machines.iter_mut().find(|m| m.node == node) {
            m.online = false;
        }
        self.publish();
    }

    /// A peer's live wadspaces, from its remote API.
    pub async fn wadspaces(&self, id: &str) -> Result<Vec<Wadspace>, ApiError> {
        let (m, to) = self.reachable(id)?;
        let r = self.get(to, "/v1/remote/wadspaces", CALL_TIMEOUT).await;
        if let Err(e) = &r {
            self.unanswered(&m.node, e);
        }
        r.map_err(|e| if e.code == ErrorCode::Offline { offline(&m.name) } else { e })
    }

    /// Copy here: a CopyWadspace job that takes the peer's document and
    /// applies `space.import` (a new id, this machine's, never built here;
    /// its projects as they were, so its folders stay on the peer).
    pub async fn copy(&self, id: &str, ws: &str) -> Result<JobInfo, ApiError> {
        let (m, to) = self.reachable(id)?;
        let doc: Wadspace = match self.get(to, &format!("/v1/remote/wadspaces/{ws}"), CALL_TIMEOUT).await {
            Ok(d) => d,
            Err(e) => {
                self.unanswered(&m.node, &e);
                return Err(if e.code == ErrorCode::Offline { offline(&m.name) } else { e });
            }
        };
        let stages = [
            ("fetch".to_string(), format!("Getting {} from {}", doc.name, m.name)),
            ("import".to_string(), "Making it this machine's".to_string()),
        ];
        let title = format!("Copying {} to this machine", doc.name);
        let job = self.jobs.create(JobKind::CopyWadspace, &title, ws, &stages);
        let (jobs, docs, builds, from, j) =
            (self.jobs.clone(), self.docs.clone(), self.builds.clone(), m.id.clone(), job.clone());
        // Which machine it comes from: the subject is only the wadspace's id,
        // and ids are per machine (the shell's cards are `<peer>:<id>`).
        self.jobs.update(&job, true, |x| {
            x.params.insert("peer".into(), Value::String(from.clone()));
        });
        let task = tokio::spawn(async move {
            jobs.running(&j);
            jobs.stage(&j, "fetch", PartState::Done, None);
            jobs.stage(&j, "import", PartState::Working, None);
            let r = tokio::task::spawn_blocking(move || docs.apply(Op::Import { doc: Box::new(doc), from }))
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| e.message));
            let r = r.map(|out| {
                builds.edited(&out.changed);
                if let Some(created) = out.created {
                    jobs.update(&j, true, |x| {
                        x.params.insert("created".into(), Value::String(created));
                    });
                }
            });
            if r.is_ok() {
                jobs.stage(&j, "import", PartState::Done, None);
            }
            jobs.finish(&j, r);
        });
        self.jobs.attach(&job, task.abort_handle());
        self.jobs.info(&job).ok_or_else(|| ApiError::internal("the job went"))
    }

    /// A call about one of its wadspaces' stream: it said why not as it said
    /// it, or it's offline.
    async fn stream_call<T: DeserializeOwned>(
        &self,
        id: &str,
        method: reqwest::Method,
        ws: &str,
        body: Option<&StreamRequest>,
        timeout: Duration,
    ) -> Result<T, ApiError> {
        let (m, to) = self.reachable(id)?;
        let path = format!("/v1/remote/wadspaces/{ws}/stream");
        let r = self.call(method, to, &path, body, timeout).await;
        if let Err(e) = &r {
            self.unanswered(&m.node, e);
        }
        r.map_err(|e| if e.code == ErrorCode::Offline { offline(&m.name) } else { e })
    }

    /// Pushes an answer made here to each of my machines online now.
    pub fn push(self: &Arc<Self>, answer: TrustAnswer) {
        let to: Vec<(String, SocketAddr)> = {
            let st = self.st.lock().unwrap();
            st.saved
                .machines
                .iter()
                .filter(|m| m.online)
                .filter_map(|m| Some((m.name.clone(), m.addr(self.port)?)))
                .collect()
        };
        for (name, addr) in to {
            let (me, a) = (self.clone(), answer.clone());
            tokio::spawn(async move {
                if let Err(e) = me.post(addr, "/v1/remote/trust", &a).await {
                    tracing::warn!("peers: telling {name} about {}: {e}", a.name);
                }
            });
        }
    }

    /// One of my machines came online: its answers merged here, and the
    /// ones it lacks (or has older) sent to it.
    pub async fn exchange(&self, node: &str) {
        let (name, to) = {
            let st = self.st.lock().unwrap();
            let Some(m) = st.saved.machines.iter().find(|m| m.node == node) else { return };
            let Some(a) = m.addr(self.port) else { return };
            (m.name.clone(), a)
        };
        let theirs: TrustBook = match self.get(to, "/v1/remote/trust", CALL_TIMEOUT).await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("peers: {name}'s trust book: {e}");
                return;
            }
        };
        let ours = self.tailnet.book();
        if theirs.user == ours.user && theirs.tailnet == ours.tailnet {
            self.tailnet.merge(theirs.devices.clone());
        }
        let ours = self.tailnet.book();
        let mut sent = 0;
        for a in ours.devices {
            let behind = match theirs.devices.iter().find(|d| d.device == a.device) {
                Some(d) => tailnet::newer(&a, d),
                None => true,
            };
            if behind {
                match self.post(to, "/v1/remote/trust", &a).await {
                    Ok(_) => sent += 1,
                    Err(e) => tracing::warn!("peers: telling {name} about {}: {e}", a.name),
                }
            }
        }
        tracing::info!("peers: swapped trust books with {name} ({sent} answers sent)");
    }
}

/// What a remote API call needs of its caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Nothing: hello.
    Hello,
    /// One of my machines, or a trusted device.
    Trusted,
    /// One of my machines (the trust book's routes).
    Machine,
}

/// The tailnet, my machines and the remote API: wadd's [`Net`].
pub struct TailnetPeers {
    pub tailnet: Arc<Tailnet>,
    pub peers: Arc<Peers>,
}

impl TailnetPeers {
    pub fn new(
        config: &wad_config::Config,
        machine: &MachineInfo,
        bus: Bus,
        jobs: Arc<Jobs>,
        docs: Arc<Docs>,
        builds: Arc<Builds>,
        person: PersonFn,
    ) -> Arc<Self> {
        let tailnet = Tailnet::new(tailnet::Settings::new(config), &machine.id, bus.clone(), jobs.clone(), person);
        let peers = Peers::new(tailnet.clone(), &machine.id, &config.daemon.state_dir, bus, jobs, docs, builds);
        // The present, for the first watchers.
        peers.publish();
        Arc::new(Self { tailnet, peers })
    }

    /// Its loops (tailscaled read and watched, my machines looked for, the
    /// remote API served), until they're aborted.
    pub fn start(self: &Arc<Self>, app: Arc<AppState>) -> Vec<tokio::task::JoinHandle<()>> {
        if !self.tailnet.enabled() {
            return vec![];
        }
        vec![
            tokio::spawn(self.tailnet.clone().run()),
            tokio::spawn(self.tailnet.clone().watch_bus()),
            tokio::spawn(self.peers.clone().run()),
            tokio::spawn(crate::remote::serve(Arc::new(crate::remote::Remote { app, net: self.clone() }))),
        ]
    }

    /// Who a connection to the remote API from `addr` is, as far as `need`
    /// asks. A node of the login's own not known yet to be one of my machines
    /// is said hello to first, unless that can't change the answer (it's
    /// trusted anyway), or it's saying hello: then that's done in the
    /// background, and it asks only once it's found not to be a wadd. (Hello
    /// never waits on another machine, so two wadds saying hello to each
    /// other don't wait on each other.)
    pub async fn identify(&self, addr: SocketAddr, need: Need) -> Result<Caller, Refusal> {
        let me = self.tailnet.me().ok_or(Refusal::Down)?;
        let w = self.tailnet.whois(addr).await?.ok_or(Refusal::NotOnTailnet)?;
        let ip = addr.ip();
        let machine = match tailnet::classify(&me, &w) {
            Class::ThisMachine => return Err(Refusal::ThisMachine),
            Class::Other => false,
            Class::SameUser => match self.peers.known(&w.node.stable_id) {
                Some(m) => m,
                None if need == Need::Trusted && self.tailnet.answer_of(&w.node.stable_id) == Some(Answer::Trusted) => {
                    false
                }
                None if need != Need::Hello => self.peers.check_node(&w.node.stable_id).await,
                None => {
                    let (peers, tailnet, w2) = (self.peers.clone(), self.tailnet.clone(), w.clone());
                    tokio::spawn(async move {
                        if peers.check_node_once(&w2.node.stable_id).await == Some(false) {
                            tailnet.caller(&w2, ip, false, true);
                        }
                    });
                    return Ok(self.tailnet.caller(&w, ip, false, false));
                }
            },
        };
        Ok(self.tailnet.caller(&w, ip, machine, true))
    }

    /// Who's calling with the machine's owner's account sign-in (the
    /// phone's, its ID token checked): trusted as one of the owner's own
    /// devices, without asking, unless the machine denied it; one of my
    /// machines stays one.
    pub async fn identify_owner(&self, addr: SocketAddr) -> Result<Caller, Refusal> {
        let me = self.tailnet.me().ok_or(Refusal::Down)?;
        let w = self.tailnet.whois(addr).await?.ok_or(Refusal::NotOnTailnet)?;
        if matches!(tailnet::classify(&me, &w), Class::ThisMachine) {
            return Err(Refusal::ThisMachine);
        }
        let machine = self.peers.known(&w.node.stable_id) == Some(true);
        let mut c = self.tailnet.caller(&w, addr.ip(), machine, false);
        if c.trust == wad_proto::RemoteTrust::Asked {
            c.trust = wad_proto::RemoteTrust::Trusted;
        }
        Ok(c)
    }

    fn answered(&self, (view, answer): (View, Option<TrustAnswer>)) -> View {
        if let Some(a) = answer {
            self.peers.push(a);
        }
        view
    }
}

/// Notices told here, on to my other machines: each one posted to the online
/// ones, in the background (a machine that's slow or asleep doesn't hold the
/// shell up), with its origin so it's shown as this machine's.
impl crate::events::Relay for Peers {
    fn notice(&self, n: &RemoteNotice) {
        let me = self.machine.clone();
        // Who to tell, now: the online ones only.
        let to: Vec<SocketAddr> = self
            .st
            .lock()
            .unwrap()
            .saved
            .machines
            .iter()
            .filter(|m| m.online && m.id != me)
            .filter_map(|m| m.addr(self.port))
            .collect();
        if to.is_empty() {
            return;
        }
        let mut posted = n.clone();
        // The machine it was told on: this one's name, as the others know it.
        posted.machine = self.st.lock().unwrap().saved.machines.iter().find(|m| m.id == me).map(|m| m.name.clone());
        let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
        for a in to {
            let (client, body) = (self.client(), posted.clone());
            rt.spawn(async move {
                let url = format!("http://{a}/v1/remote/notices");
                let sent = client.post(url).timeout(std::time::Duration::from_secs(5)).json(&body).send().await;
                if let Err(e) = sent {
                    tracing::debug!("a notice didn't reach {a}: {e}");
                }
            });
        }
    }
}

#[async_trait]
impl Net for TailnetPeers {
    async fn tailnet(&self) -> Result<View, ApiError> {
        Ok(self.tailnet.view())
    }

    async fn login(&self) -> Result<JobInfo, ApiError> {
        self.tailnet.login().await
    }

    async fn logout(&self) -> Result<View, ApiError> {
        let v = self.tailnet.logout().await;
        self.peers.forget();
        v
    }

    async fn trust(&self, device: &str) -> Result<View, ApiError> {
        Ok(self.answered(self.tailnet.trust(device)?))
    }

    async fn untrust(&self, device: &str) -> Result<View, ApiError> {
        Ok(self.answered(self.tailnet.untrust(device)?))
    }

    async fn deny(&self, ask: &str) -> Result<View, ApiError> {
        Ok(self.answered(self.tailnet.deny(ask)?))
    }

    async fn later(&self, ask: &str) -> Result<View, ApiError> {
        self.tailnet.later(ask)
    }

    async fn peers(&self) -> Result<Vec<PeerInfo>, ApiError> {
        Ok(self.peers.list())
    }

    async fn peer_wadspaces(&self, peer: &str) -> Result<Vec<Wadspace>, ApiError> {
        self.peers.wadspaces(peer).await
    }

    async fn copy_from_peer(&self, peer: &str, ws: &str) -> Result<JobInfo, ApiError> {
        self.peers.copy(peer, ws).await
    }
}

/// My machines' streams of their wadspaces (streams.rs's, "Stream from {machine}").
#[async_trait]
impl crate::streams::PeerStreams for Peers {
    async fn start(&self, peer: &str, ws: &str, req: StreamRequest) -> Result<StreamShare, ApiError> {
        if !crate::store::safe_id(ws) {
            return Err(ApiError::bad_request(format!("not a wadspace's id: {ws:?}")));
        }
        self.stream_call(peer, reqwest::Method::POST, ws, Some(&req), STREAM_TIMEOUT).await
    }

    async fn stop(&self, peer: &str, ws: &str) -> Result<StreamInfo, ApiError> {
        if !crate::store::safe_id(ws) {
            return Err(ApiError::not_found(format!("not a wadspace's id: {ws:?}")));
        }
        self.stream_call(peer, reqwest::Method::DELETE, ws, None, CALL_TIMEOUT).await
    }

    async fn info(&self, peer: &str, ws: &str) -> Result<Option<StreamInfo>, ApiError> {
        if !crate::store::safe_id(ws) {
            return Ok(None);
        }
        match self.stream_call(peer, reqwest::Method::GET, ws, None, CALL_TIMEOUT).await {
            Ok(i) => Ok(Some(i)),
            Err(e) if e.code == ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}
