//! This machine on its owner's tailnet (Tailscale), and who may use it.
//!
//! [`Tailnet`] reads tailscaled's state (its LocalAPI, wad-tailscale) every
//! `poll_s`, every second while logging in, and at once when the IPN bus says
//! something changed; it publishes it as the `tailnet` event ([`Tailnet::view`],
//! wad-proto's `Tailnet`: one of the seven states, the devices, the asks).
//! Logging in is a Tailnet job ([`Tailnet::login`]): `tailscale up --json`,
//! its AuthURL signed in to in the browser (the `signIn` event, held), then
//! Running. Cancelling the job stops the `tailscale up` and closes the window.
//!
//! **Who's calling** the remote API (remote.rs), from Tailscale's WhoIs of
//! the connection (kept [`WHOIS_FOR`]), with [`classify`]:
//!
//! - **this machine**: not a remote caller, refused;
//! - **one of my machines**: the same Tailscale user as this machine's login,
//!   untagged (the tags are looked at first: a tagged node's user is a
//!   pseudo-user, or whoever tagged it), not shared in, and its node answers
//!   hello as a wadd (peers.rs checks that and keeps the answer by node id).
//!   Trusted without an ask;
//! - **any other device** (the phone, a laptop without WadOS, someone else's,
//!   a shared-in or tagged node): what the trust book says. Not trusted, it
//!   gets hello only and raises an ask; denied, it stays quiet.
//!
//! **The trust book** (`<state>/tailnet-devices.json`, 0600: wad-proto's
//! `TrustBook`) holds every answer (trusted, denied, untrusted), each with
//! when and which machine gave it; it's wiped when the login or the tailnet
//! changes. Every machine of mine follows an answer: peers.rs pushes each
//! one to my online machines and swaps books with one that comes online, and
//! [`Tailnet::merge`] keeps, per device, the latest (`at`, then `by`).
//!
//! **Asks** are kept in memory, at most [`MAX_ASKS`]; Later hides one for as
//! long as wadd runs.
//!
//! **Taildrive:** once it's running, the person's home is shared on the
//! tailnet as theirs (`runuser -u <person> -- tailscale drive share home
//! <home>`): my other machines reach it at
//! `dav://100.100.100.100:8080/<tailnet>/<this machine's name>/home` (the
//! view's `tailnet` and `hostname`). A tailnet whose policy doesn't allow it
//! shows `drive: notAllowed` (Settings ▸ Accounts explains `drive:share`).

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{Notify, watch};
use wad_model::{JobKind, PartState};
use wad_proto::{
    Answer, ApiError, DeviceKind, DriveState, ErrorCode, Event, JobInfo, RemoteTrust, SignInProvider, TailnetAsk,
    TailnetDevice, TailnetState, TrustAnswer, TrustBook,
};
use wad_tailscale::{Cli, LocalApi, Status, UpEvent, UpOptions, WhoIs};

use crate::events::{Bus, SignInWindow};
use crate::jobs::{Jobs, now_ms};

/// How long a login waits for the sign-in (and an admin's approval).
pub const LOGIN_WAIT: Duration = Duration::from_secs(600);
/// How long a device's WhoIs answer is kept (per address).
pub const WHOIS_FOR: Duration = Duration::from_secs(30);
/// Asks kept at once (the oldest go).
pub const MAX_ASKS: usize = 8;
/// A share that failed is tried again after this.
const DRIVE_RETRY: Duration = Duration::from_secs(300);
/// The share's name: a peer's home is `…/<tailnet>/<machine>/home`.
pub const HOME_SHARE: &str = "home";

/// A peer that offers to be an exit node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitOffer {
    pub id: String,
    pub name: String,
    pub os: String,
    pub online: bool,
    pub tagged: bool,
}

/// The tailnet's settings ([tailnet] in wadd.toml), and where its book is.
#[derive(Debug, Clone)]
pub struct Settings {
    pub enabled: bool,
    pub socket: PathBuf,
    pub cli: PathBuf,
    pub runuser: PathBuf,
    pub port: u16,
    pub listen: Vec<String>,
    pub bind_device: String,
    /// The machine's name on the tailnet, as Tailscale takes it.
    pub hostname: String,
    pub poll: Duration,
    pub login_server: String,
    pub drive: bool,
    pub state_dir: PathBuf,
}

impl Settings {
    pub fn new(config: &wad_config::Config) -> Self {
        let t = &config.tailnet;
        let name = if t.hostname.is_empty() { &config.machine.name } else { &t.hostname };
        Self {
            enabled: t.enabled,
            socket: t.socket.clone(),
            cli: t.cli.clone(),
            runuser: t.runuser.clone(),
            port: t.port,
            listen: t.listen.clone(),
            bind_device: t.bind_device.clone(),
            hostname: host_label(name),
            poll: Duration::from_secs(t.poll_s.max(1)),
            login_server: t.login_server.clone(),
            drive: t.drive,
            state_dir: config.daemon.state_dir.clone(),
        }
    }
}

/// The machine's person: whose home Taildrive shares, as them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub uid: u32,
    pub name: String,
    pub home: PathBuf,
}

/// Who the person is now (the shell's session's user, else `projects_uid`).
pub type PersonFn = Arc<dyn Fn() -> Option<Person> + Send + Sync>;

/// This machine on the tailnet, as a caller is compared with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Me {
    /// Its node's stable id.
    pub node: String,
    /// Its Tailscale user's id and login name.
    pub user: i64,
    pub login: String,
    pub tailnet: String,
}

/// What a node is to this machine, from its WhoIs (before asking whether it's a wadd).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    ThisMachine,
    /// The same user's, untagged, not shared in: one of my machines if it
    /// answers hello as a wadd, else one of the login's own devices.
    SameUser,
    /// Someone else's, shared in, or tagged.
    Other,
}

/// What `w` is to this machine. The tags first: a tagged node's user is
/// Tailscale's pseudo-user, or (Headscale) whoever tagged it.
pub fn classify(me: &Me, w: &WhoIs) -> Class {
    if w.node.stable_id == me.node {
        return Class::ThisMachine;
    }
    if !w.node.tags.is_empty() || w.node.sharer != 0 {
        return Class::Other;
    }
    if w.node.user == me.user && (w.user.login_name.is_empty() || w.user.login_name == me.login) {
        return Class::SameUser;
    }
    Class::Other
}

/// A device calling the remote API, and how this machine sees it.
#[derive(Debug, Clone)]
pub struct Caller {
    /// Its node's stable id.
    pub id: String,
    pub name: String,
    pub os: String,
    pub kind: DeviceKind,
    pub ip: IpAddr,
    pub trust: RemoteTrust,
}

impl Caller {
    /// It may use the remote API past hello.
    pub fn trusted(&self) -> bool {
        matches!(self.trust, RemoteTrust::Machine | RemoteTrust::Trusted)
    }
}

/// Why a connection to the remote API was refused outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The machine isn't on the tailnet (or tailscaled isn't answering).
    Down,
    /// Not a device on the tailnet.
    NotOnTailnet,
    /// The machine itself.
    ThisMachine,
}

impl Refusal {
    pub fn message(self) -> &'static str {
        match self {
            Refusal::Down => "This machine isn't on the tailnet right now.",
            Refusal::NotOnTailnet => "Only devices on the tailnet can use this.",
            Refusal::ThisMachine => "That's this machine.",
        }
    }
}

/// One of my machines, as the devices list shows it (peers.rs says which).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyMachine {
    pub node: String,
    pub name: String,
    pub os: String,
    pub kind: DeviceKind,
    pub online: bool,
}

/// What a device is, as far as its OS and model tell.
pub fn device_kind(os: &str, model: &str, tagged: bool) -> DeviceKind {
    let os = os.to_lowercase();
    let m = model.to_lowercase();
    if m.contains("ipad") || m.contains("tablet") || m.contains("galaxy tab") {
        DeviceKind::Tablet
    } else if os == "android" || os == "ios" {
        DeviceKind::Phone
    } else if m.contains("macbook") || m.contains("laptop") {
        DeviceKind::Laptop
    } else if ["imac", "mac mini", "macmini", "mac pro", "mac studio"].iter().any(|d| m.contains(d)) {
        DeviceKind::Desktop
    } else if tagged {
        DeviceKind::Server
    } else {
        DeviceKind::Other
    }
}

/// `a` is newer than `b` (the latest `at` wins; a tie, the larger `by`, so
/// every machine picks the same one).
pub fn newer(a: &TrustAnswer, b: &TrustAnswer) -> bool {
    (a.at, &a.by) > (b.at, &b.by)
}

/// Merges `answers` into `book`, per device the newest; whether it changed.
pub fn merge_into(book: &mut TrustBook, answers: impl IntoIterator<Item = TrustAnswer>) -> bool {
    let mut changed = false;
    for a in answers {
        match book.devices.iter_mut().find(|d| d.device == a.device) {
            Some(d) if newer(&a, d) => {
                *d = a;
                changed = true;
            }
            Some(_) => {}
            None => {
                book.devices.push(a);
                changed = true;
            }
        }
    }
    changed
}

/// A name Tailscale takes for the machine: letters, digits and dashes.
pub fn host_label(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_end_matches('-').chars().take(63).collect();
    if out.is_empty() { "wadspaces".into() } else { out.trim_end_matches('-').to_string() }
}

#[derive(Default)]
struct Inner {
    status: Option<Status>,
    me: Option<Me>,
    /// The last read of tailscaled failed.
    unreachable: bool,
    /// A login is under way: its link, once there is one.
    joining: Option<Option<String>>,
    book: TrustBook,
    asks: Vec<TailnetAsk>,
    /// Asks put off with Later (this session).
    later: HashSet<String>,
    whois: HashMap<IpAddr, (Instant, Option<WhoIs>)>,
    /// Devices' models, from their WhoIs (the status hasn't them).
    models: HashMap<String, String>,
    machines: Vec<MyMachine>,
    drive: DriveState,
    drive_error: Option<String>,
    drive_tried: Option<Instant>,
    drive_busy: bool,
    was_running: bool,
    /// What the last status said that peers.rs looks at (its state, the nodes and who's online).
    sig: u64,
    published: Option<wad_proto::Tailnet>,
}

pub struct Tailnet {
    s: Settings,
    api: LocalApi,
    cli: Cli,
    /// This wadd's machine id (an answer's `by`).
    machine: String,
    file: PathBuf,
    bus: Bus,
    jobs: Arc<Jobs>,
    person: PersonFn,
    inner: Mutex<Inner>,
    addrs: watch::Sender<Vec<SocketAddr>>,
    /// Bumped whenever a read of tailscaled changed something.
    changes: watch::Sender<u64>,
    wake: Notify,
}

fn offline(message: &str) -> ApiError {
    ApiError::new(ErrorCode::Offline, message)
}

impl Tailnet {
    /// The tailnet, its present told to the watchers (off until tailscaled is read).
    pub fn new(s: Settings, machine: &str, bus: Bus, jobs: Arc<Jobs>, person: PersonFn) -> Arc<Self> {
        let file = s.state_dir.join("tailnet-devices.json");
        let book = read_book(&file);
        let t = Arc::new(Self {
            api: LocalApi::new(&s.socket),
            cli: Cli::new(&s.cli, &s.socket).with_runuser(&s.runuser),
            machine: machine.to_string(),
            file,
            bus,
            jobs,
            person,
            inner: Mutex::new(Inner { book, ..Default::default() }),
            addrs: watch::channel(vec![]).0,
            changes: watch::channel(0).0,
            wake: Notify::new(),
            s,
        });
        t.publish();
        t
    }

    pub fn enabled(&self) -> bool {
        self.s.enabled
    }

    pub fn port(&self) -> u16 {
        self.s.port
    }

    pub fn poll(&self) -> Duration {
        self.s.poll
    }

    /// The interface the remote API is bound to ("": none).
    pub fn bind_device(&self) -> &str {
        &self.s.bind_device
    }

    /// Where the remote API should listen now (it rebinds when this changes).
    pub fn addrs(&self) -> watch::Receiver<Vec<SocketAddr>> {
        self.addrs.subscribe()
    }

    /// Bumped whenever tailscaled's state changed (peers.rs looks again).
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Read tailscaled again soon.
    pub fn poke(&self) {
        self.wake.notify_one();
    }

    /// Reads tailscaled until wadd stops: every `poll_s`, every second while
    /// logging in, and at once when poked (the IPN bus, a change here).
    pub async fn run(self: Arc<Self>) {
        if !self.enabled() {
            return;
        }
        loop {
            self.refresh().await;
            let every = if self.inner.lock().unwrap().joining.is_some() { Duration::from_secs(1) } else { self.s.poll };
            tokio::select! {
                _ = tokio::time::sleep(every) => {}
                _ = self.wake.notified() => {}
            }
        }
    }

    /// Watches tailscaled's IPN bus: each change (a state, a login link) is read at once.
    pub async fn watch_bus(self: Arc<Self>) {
        if !self.enabled() {
            return;
        }
        loop {
            match self.api.watch_ipn_bus(wad_tailscale::WATCH_INITIAL_STATE).await {
                Ok(mut bus) => {
                    while let Some(n) = bus.next().await {
                        match n {
                            Ok(n) if n.state.is_some() || n.browse_to_url.is_some() => self.poke(),
                            Ok(_) => {}
                            Err(e) => {
                                tracing::debug!("tailnet: the IPN bus: {e}");
                                break;
                            }
                        }
                    }
                }
                Err(e) => tracing::debug!("tailnet: the IPN bus: {e}"),
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    /// tailscaled's state read again (and published if it changed).
    pub async fn refresh(self: &Arc<Self>) {
        if !self.enabled() {
            return;
        }
        let read = self.api.status().await;
        let (share, moved) = {
            let mut i = self.inner.lock().unwrap();
            let sig = read.as_ref().map(signature).unwrap_or(0);
            let moved = std::mem::replace(&mut i.sig, sig) != sig;
            match read {
                Ok(s) => {
                    i.unreachable = false;
                    i.me = me_of(&s);
                    self.check_owner(&mut i);
                    i.status = Some(s);
                }
                Err(e) => {
                    if !i.unreachable {
                        tracing::warn!("tailnet: {e}");
                    }
                    i.unreachable = true;
                    i.status = None;
                    i.me = None;
                }
            }
            let running = i.me.is_some();
            if !running {
                i.drive = DriveState::Off;
                i.drive_error = None;
            }
            let came_up = running && !i.was_running;
            i.was_running = running;
            let due = i.drive_tried.is_none_or(|t| t.elapsed() >= DRIVE_RETRY);
            let share = running && self.s.drive && !i.drive_busy && (came_up || (i.drive != DriveState::Shared && due));
            if share {
                i.drive_busy = true;
            }
            (share, moved)
        };
        if share {
            tokio::spawn(self.clone().share_home());
        }
        let addrs = self.listen_addrs();
        self.addrs.send_if_modified(|now| {
            if *now == addrs {
                return false;
            }
            *now = addrs;
            true
        });
        self.publish();
        if moved {
            self.changes.send_modify(|n| *n += 1);
        }
    }

    /// The trust is for one login on one tailnet: logged in as someone else,
    /// or on another tailnet, the machine starts again with no answers.
    fn check_owner(&self, i: &mut Inner) {
        let Some(me) = i.me.clone() else { return };
        let b = &mut i.book;
        if b.user == me.login && b.tailnet == me.tailnet {
            return;
        }
        if !b.user.is_empty() {
            tracing::warn!("tailnet: the machine's Tailscale login or tailnet changed; the trust book starts again");
        }
        *b = TrustBook { user: me.login, tailnet: me.tailnet, devices: vec![] };
        i.asks.clear();
        i.later.clear();
        i.whois.clear();
        save_book(&self.file, &i.book);
    }

    fn listen_addrs(&self) -> Vec<SocketAddr> {
        if !self.s.listen.is_empty() {
            return self
                .s
                .listen
                .iter()
                .filter_map(|a| a.parse().inspect_err(|e| tracing::warn!("[tailnet] listen {a:?}: {e}")).ok())
                .collect();
        }
        let i = self.inner.lock().unwrap();
        match &i.status {
            Some(s) if s.running() => s.ips().into_iter().map(|ip| SocketAddr::new(ip, self.s.port)).collect(),
            _ => vec![],
        }
    }

    /// The status last read, while tailscaled answers.
    pub fn status(&self) -> Option<Status> {
        self.inner.lock().unwrap().status.clone()
    }

    /// This machine on the tailnet, while it's running.
    pub fn me(&self) -> Option<Me> {
        self.inner.lock().unwrap().me.clone()
    }

    /// The machine's own tailnet address (IPv4), while it's running.
    pub fn ipv4(&self) -> Option<IpAddr> {
        let i = self.inner.lock().unwrap();
        i.status.as_ref().filter(|s| s.running()).and_then(Status::ipv4)
    }

    fn state(&self, i: &Inner) -> TailnetState {
        let Some(s) = i.status.as_ref().filter(|_| self.enabled() && !i.unreachable) else {
            return TailnetState::Off;
        };
        let joining = i.joining.is_some();
        match s.backend_state.as_str() {
            "Running" => TailnetState::Running,
            "NeedsMachineAuth" => TailnetState::NeedsApproval,
            _ if joining => TailnetState::LoggingIn,
            "Stopped" => TailnetState::Stopped,
            "Starting" | "NoState" => TailnetState::Starting,
            _ => TailnetState::NeedsLogin,
        }
    }

    /// `GET /v1/tailnet` (the `tailnet` event).
    pub fn view(&self) -> wad_proto::Tailnet {
        let i = self.inner.lock().unwrap();
        self.view_of(&i)
    }

    fn view_of(&self, i: &Inner) -> wad_proto::Tailnet {
        let state = self.state(i);
        let mut v = wad_proto::Tailnet { state, hostname: self.s.hostname.clone(), ..Default::default() };
        if state == TailnetState::Off {
            return v;
        }
        let s = i.status.as_ref().expect("a status when it isn't off");
        if state != TailnetState::Running {
            v.login_url = i.joining.clone().flatten().or_else(|| Some(s.auth_url.clone()).filter(|u| !u.is_empty()));
        }
        if let Some(me) = s.me.as_ref() {
            let name = me.machine_name();
            if !name.is_empty() {
                v.hostname = name;
            }
        }
        v.tailnet = s.tailnet_name().to_string();
        v.ip = s.ipv4().map(|ip| ip.to_string()).unwrap_or_default();
        v.drive = i.drive;
        v.drive_error = i.drive_error.clone();
        let answer = |id: &str| i.book.devices.iter().find(|d| d.device == id).map(|d| d.answer);
        for m in &i.machines {
            v.devices.push(TailnetDevice {
                id: m.node.clone(),
                name: m.name.clone(),
                os: m.os.clone(),
                kind: m.kind,
                online: m.online,
                trusted: true,
                mine: true,
            });
        }
        if let Some(me) = i.me.as_ref() {
            // The login's own devices that aren't wadds of mine (a phone, a laptop).
            for p in s.peer_list() {
                let own = p.tags.is_empty() && !p.sharee_node && p.user_id == me.user;
                if !own || v.devices.iter().any(|d| d.id == p.id) {
                    continue;
                }
                let model = i.models.get(&p.id).map(String::as_str).unwrap_or_default();
                v.devices.push(TailnetDevice {
                    id: p.id.clone(),
                    name: p.host_name.clone(),
                    os: p.os.clone(),
                    kind: device_kind(&p.os, model, false),
                    online: p.online,
                    trusted: answer(&p.id) == Some(Answer::Trusted),
                    mine: false,
                });
            }
        }
        // And every device the book has an answer for.
        for a in &i.book.devices {
            if v.devices.iter().any(|d| d.id == a.device) {
                continue;
            }
            let p = s.peer(&a.device);
            let model = i.models.get(&a.device).map(String::as_str).unwrap_or_default();
            v.devices.push(TailnetDevice {
                id: a.device.clone(),
                name: p.map(|p| p.host_name.clone()).unwrap_or_else(|| a.name.clone()),
                os: a.os.clone(),
                kind: device_kind(&a.os, model, p.is_some_and(|p| !p.tags.is_empty())),
                online: p.is_some_and(|p| p.online),
                trusted: a.answer == Answer::Trusted,
                mine: false,
            });
        }
        v.asks = i.asks.iter().filter(|a| !i.later.contains(&a.id)).cloned().collect();
        v
    }

    /// Tells the watchers, if it changed: whether it did.
    fn publish(&self) -> bool {
        let v = {
            let mut i = self.inner.lock().unwrap();
            let v = self.view_of(&i);
            if i.published.as_ref() == Some(&v) {
                return false;
            }
            i.published = Some(v.clone());
            v
        };
        self.bus.publish(Event::Tailnet(v));
        true
    }

    /// My machines, from peers.rs: trusted without an ask, and in the devices list.
    pub fn set_machines(&self, machines: Vec<MyMachine>) {
        {
            let mut i = self.inner.lock().unwrap();
            if i.machines == machines {
                return;
            }
            let ids: HashSet<&str> = machines.iter().map(|m| m.node.as_str()).collect();
            i.asks.retain(|a| !ids.contains(a.id.as_str()));
            i.machines = machines;
        }
        self.publish();
    }

    // ------------------------------------------------------------ logging in

    /// Log in: a Tailnet job (`tailscale up`, then its AuthURL signed in to
    /// in the browser, then Running). One at a time: a second asks for the first.
    pub async fn login(self: &Arc<Self>) -> Result<JobInfo, ApiError> {
        if !self.enabled() {
            return Err(offline("Tailscale is turned off on this machine."));
        }
        if let Some(id) = self.jobs.running_with("tailnet", "login") {
            return self.jobs.info(&id).ok_or_else(|| ApiError::internal("the login went"));
        }
        self.refresh().await;
        match self.status() {
            None => return Err(offline("Tailscale isn't running on this machine.")),
            Some(s) if s.running() => return Err(ApiError::conflict("This machine is on the tailnet already.")),
            Some(_) => {}
        }
        let signin = SignInProvider::Tailscale;
        let stages = [
            ("up".to_string(), "Starting Tailscale".to_string()),
            ("signin".to_string(), signin.held_label()),
            ("running".to_string(), "Joining the tailnet".to_string()),
        ];
        let id = self.jobs.create(JobKind::Tailnet, "Logging in to Tailscale", "tailnet", &stages);
        self.jobs.update(&id, true, |j| {
            j.params.insert("tailnet".into(), "login".into());
        });
        let (me, job) = (self.clone(), id.clone());
        let task = tokio::spawn(async move {
            me.jobs.running(&job);
            let r = me.log_in(&job).await;
            me.jobs.finish(&job, r);
            me.poke();
        });
        self.jobs.attach(&id, task.abort_handle());
        self.jobs.info(&id).ok_or_else(|| ApiError::internal("the login went"))
    }

    /// The sign-in's done (its window closed): on to joining.
    fn signed_in(&self, job: &str) {
        self.hold(job, "signin", false);
        self.jobs.stage(job, "signin", PartState::Done, None);
        self.jobs.stage(job, "running", PartState::Working, None);
    }

    fn hold(&self, job: &str, stage: &str, held: bool) {
        self.jobs.update(job, true, |j| j.stages.iter_mut().filter(|s| s.id == stage).for_each(|s| s.held = held));
    }

    async fn log_in(self: &Arc<Self>, job: &str) -> Result<(), String> {
        let jobs = &self.jobs;
        jobs.stage(job, "up", PartState::Working, None);
        let _joining = Joining::start(self.clone());
        let login_server = match self.s.login_server.as_str() {
            "" => self.api.prefs().await.ok().map(|p| p.control_url).filter(|u| !u.is_empty()),
            s => Some(s.to_string()),
        };
        let opts = UpOptions { hostname: Some(self.s.hostname.clone()), login_server, timeout: LOGIN_WAIT };
        tracing::info!("tailnet: logging in as {}", self.s.hostname);
        let mut login = self.cli.up(&opts).map_err(|e| e.to_string())?;
        let mut window: Option<(String, SignInWindow)> = None;
        // A sign-in window was shown.
        let mut shown = false;
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut ended = false;
        loop {
            let url = tokio::select! {
                e = login.next() => match e {
                    Some(UpEvent::AuthUrl(u)) => Some(u),
                    Some(UpEvent::State(s)) if s == "Running" => break,
                    Some(UpEvent::State(s)) => {
                        // Signed in, and the tailnet's admin approves the machine now.
                        if s == "NeedsMachineAuth" && window.take().is_some() {
                            self.signed_in(job);
                        }
                        self.poke();
                        None
                    }
                    Some(UpEvent::Error(e)) => return Err(e),
                    None => {
                        ended = true;
                        break;
                    }
                },
                _ = tick.tick() => {
                    self.refresh().await;
                    match self.status() {
                        Some(s) if s.running() => break,
                        Some(s) if s.backend_state == "NeedsLogin" && !s.auth_url.is_empty() => Some(s.auth_url),
                        _ => None,
                    }
                }
            };
            let Some(url) = url else { continue };
            if window.as_ref().is_some_and(|(u, _)| *u == url) {
                continue;
            }
            if !shown {
                shown = true;
                jobs.stage(job, "up", PartState::Done, None);
                jobs.stage(job, "signin", PartState::Working, None);
                self.hold(job, "signin", true);
            }
            self.inner.lock().unwrap().joining = Some(Some(url.clone()));
            self.publish();
            // A new link: the old window closes first.
            drop(window.take());
            window = Some((url.clone(), self.bus.sign_in(SignInProvider::Tailscale, &url, None)));
        }
        if window.take().is_some() {
            self.signed_in(job);
        }
        if ended {
            login.wait().await.map_err(|e| e.to_string())?;
        }
        jobs.stage(job, "up", PartState::Done, None);
        if !shown {
            // Its key was still good: no sign-in.
            jobs.stage(job, "signin", PartState::Skipped, None);
        }
        jobs.stage(job, "running", PartState::Working, None);
        for _ in 0..60 {
            self.refresh().await;
            if self.status().is_some_and(|s| s.running()) {
                jobs.stage(job, "running", PartState::Done, None);
                tracing::info!("tailnet: logged in");
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Err("Tailscale said it was up, but it isn't running".into())
    }

    /// Log out (a login under way is cancelled): the machine leaves the tailnet.
    pub async fn logout(self: &Arc<Self>) -> Result<wad_proto::Tailnet, ApiError> {
        if !self.enabled() {
            return Err(offline("Tailscale is turned off on this machine."));
        }
        if let Some(id) = self.jobs.running_with("tailnet", "login") {
            let _ = self.jobs.cancel(&id);
        }
        let r = self.cli.logout().await;
        {
            let mut i = self.inner.lock().unwrap();
            i.asks.clear();
            i.later.clear();
            i.whois.clear();
            i.machines.clear();
        }
        self.refresh().await;
        r.map_err(|e| ApiError::new(ErrorCode::Upstream, e.to_string()))?;
        tracing::info!("tailnet: logged out");
        Ok(self.view())
    }

    // ------------------------------------------------------------ the exit node

    /// While it's running: the exit node in use (its node id, online or not)
    /// and the peers that offer to be one (Tailscale's `ExitNodeOption`).
    pub fn exit_nodes(&self) -> (Option<String>, Vec<ExitOffer>) {
        let i = self.inner.lock().unwrap();
        let Some(s) = i.status.as_ref().filter(|s| self.enabled() && !i.unreachable && s.running()) else {
            return (None, vec![]);
        };
        let using = s
            .exit_node_status
            .as_ref()
            .map(|e| e.id.clone())
            .filter(|id| !id.is_empty())
            .or_else(|| s.peer_list().into_iter().find(|p| p.exit_node).map(|p| p.id.clone()));
        let offers = s
            .peer_list()
            .into_iter()
            .filter(|p| p.exit_node_option)
            .map(|p| ExitOffer {
                id: p.id.clone(),
                name: p.host_name.clone(),
                os: p.os.clone(),
                online: p.online,
                tagged: !p.tags.is_empty(),
            })
            .collect();
        (using, offers)
    }

    /// This machine's internet traffic out through the peer `node` (one
    /// that offers to be an exit node, online), or straight out (none).
    pub async fn set_exit_node(self: &Arc<Self>, node: Option<&str>) -> Result<(), ApiError> {
        let ip = {
            let i = self.inner.lock().unwrap();
            let Some(s) = i.status.as_ref().filter(|s| self.enabled() && !i.unreachable && s.running()) else {
                return Err(ApiError::conflict("Join Tailscale first"));
            };
            match node {
                None => None,
                Some(id) => {
                    let p = s
                        .peer_list()
                        .into_iter()
                        .find(|p| p.id == id && p.exit_node_option && p.online)
                        .ok_or_else(|| ApiError::not_found(format!("{id:?} can't be an exit node now")))?;
                    Some(p.ipv4().map(|a| a.to_string()).unwrap_or_else(|| p.machine_name()))
                }
            }
        };
        let r = self.cli.set_exit_node(ip.as_deref()).await;
        self.refresh().await;
        r.map_err(|e| ApiError::new(ErrorCode::Upstream, e.to_string()))?;
        tracing::info!("tailnet: the exit node is {}", node.unwrap_or("none"));
        Ok(())
    }

    // ---------------------------------------------------------------- trust

    /// What the book says of a device.
    pub fn answer_of(&self, device: &str) -> Option<Answer> {
        self.inner.lock().unwrap().book.devices.iter().find(|d| d.device == device).map(|d| d.answer)
    }

    /// The answers about devices (`GET /v1/remote/trust`).
    pub fn book(&self) -> TrustBook {
        self.inner.lock().unwrap().book.clone()
    }

    /// Answers about devices from another machine of mine: per device, the
    /// latest is kept. Whether anything changed.
    pub fn merge(&self, answers: Vec<TrustAnswer>) -> bool {
        let changed = {
            let mut i = self.inner.lock().unwrap();
            let i = &mut *i;
            let mine: HashSet<&str> = i.machines.iter().map(|m| m.node.as_str()).collect();
            let answers: Vec<TrustAnswer> = answers.into_iter().filter(|a| !mine.contains(a.device.as_str())).collect();
            if !merge_into(&mut i.book, answers) {
                return false;
            }
            settle_asks(i);
            save_book(&self.file, &i.book);
            true
        };
        if changed {
            self.publish();
        }
        changed
    }

    /// Records this machine's answer about `device` (now, by this machine):
    /// the answer, to push to my other machines.
    fn answer(&self, i: &mut Inner, device: &str, name: String, os: String, answer: Answer) -> TrustAnswer {
        let at = now_ms().max(i.book.devices.iter().find(|d| d.device == device).map_or(0, |d| d.at + 1));
        let a = TrustAnswer { device: device.into(), name, os, answer, at, by: self.machine.clone() };
        merge_into(&mut i.book, [a.clone()]);
        settle_asks(i);
        save_book(&self.file, &i.book);
        a
    }

    /// What's known of a device: its name and OS (an ask, the tailnet, the book).
    fn known_device(i: &Inner, id: &str) -> Option<(String, String)> {
        if let Some(a) = i.asks.iter().find(|a| a.id == id) {
            return Some((a.device.clone(), a.os.clone()));
        }
        if let Some(p) = i.status.as_ref().and_then(|s| s.peer(id)) {
            return Some((p.host_name.clone(), p.os.clone()));
        }
        i.book.devices.iter().find(|d| d.device == id).map(|d| (d.name.clone(), d.os.clone()))
    }

    fn is_mine(i: &Inner, id: &str) -> Option<String> {
        i.machines.iter().find(|m| m.node == id).map(|m| m.name.clone())
    }

    /// Trusts a device (one on the tailnet, or one asking): the tailnet now, and the answer.
    pub fn trust(&self, device: &str) -> Result<(wad_proto::Tailnet, Option<TrustAnswer>), ApiError> {
        let a = {
            let mut i = self.inner.lock().unwrap();
            if Self::is_mine(&i, device).is_some() {
                None
            } else {
                let (name, os) = Self::known_device(&i, device)
                    .ok_or_else(|| ApiError::not_found(format!("There's no device {device:?} on the tailnet")))?;
                let already = i.book.devices.iter().any(|d| d.device == device && d.answer == Answer::Trusted);
                (!already).then(|| self.answer(&mut i, device, name, os, Answer::Trusted))
            }
        };
        if let Some(a) = &a {
            tracing::info!("tailnet: trusted {} ({device})", a.name);
        }
        self.publish();
        Ok((self.view(), a))
    }

    /// Stops trusting a device: it's one this machine doesn't know again (its next call asks).
    pub fn untrust(&self, device: &str) -> Result<(wad_proto::Tailnet, Option<TrustAnswer>), ApiError> {
        let a = {
            let mut i = self.inner.lock().unwrap();
            if let Some(name) = Self::is_mine(&i, device) {
                return Err(ApiError::conflict(format!("{name} is one of your machines: it's always trusted")));
            }
            let (name, os) = Self::known_device(&i, device)
                .ok_or_else(|| ApiError::not_found(format!("There's no device {device:?} on the tailnet")))?;
            let answered = i.book.devices.iter().any(|d| d.device == device && d.answer != Answer::Untrusted);
            answered.then(|| self.answer(&mut i, device, name, os, Answer::Untrusted))
        };
        if let Some(a) = &a {
            tracing::info!("tailnet: no longer trusts {} ({device})", a.name);
        }
        self.publish();
        Ok((self.view(), a))
    }

    /// Denies an ask: the device stays quiet (hello only, no new ask).
    pub fn deny(&self, ask: &str) -> Result<(wad_proto::Tailnet, Option<TrustAnswer>), ApiError> {
        let a = {
            let mut i = self.inner.lock().unwrap();
            let Some(asked) = i.asks.iter().find(|a| a.id == ask && !i.later.contains(&a.id)).cloned() else {
                return Err(ApiError::not_found(format!("{ask:?} isn't asking to be trusted")));
            };
            self.answer(&mut i, ask, asked.device, asked.os, Answer::Denied)
        };
        tracing::info!("tailnet: denied {} ({ask})", a.name);
        self.publish();
        Ok((self.view(), Some(a)))
    }

    /// Puts an ask off for this session.
    pub fn later(&self, ask: &str) -> Result<wad_proto::Tailnet, ApiError> {
        {
            let mut i = self.inner.lock().unwrap();
            if !i.asks.iter().any(|a| a.id == ask && !i.later.contains(&a.id)) {
                return Err(ApiError::not_found(format!("{ask:?} isn't asking to be trusted")));
            }
            i.later.insert(ask.to_string());
        }
        self.publish();
        Ok(self.view())
    }

    // -------------------------------------------------------------- callers

    /// Which device a connection from `addr` is (its WhoIs, kept [`WHOIS_FOR`]
    /// per address); None when it isn't one on the tailnet.
    pub async fn whois(&self, addr: SocketAddr) -> Result<Option<WhoIs>, Refusal> {
        // A loopback port is tailscaled's (userspace mode), one connection at
        // a time: never kept.
        let keep = !addr.ip().is_loopback() || self.s.listen.is_empty();
        if keep {
            let mut i = self.inner.lock().unwrap();
            i.whois.retain(|_, (at, _)| at.elapsed() < WHOIS_FOR);
            if let Some((_, w)) = i.whois.get(&addr.ip()) {
                return Ok(w.clone());
            }
        }
        let w = self.api.whois(addr).await.map_err(|e| {
            tracing::warn!("tailnet: whois {addr}: {e}");
            Refusal::Down
        })?;
        let mut i = self.inner.lock().unwrap();
        if let Some(w) = &w
            && !w.node.hostinfo.device_model.is_empty()
        {
            i.models.insert(w.node.stable_id.clone(), w.node.hostinfo.device_model.clone());
        }
        if keep {
            i.whois.insert(addr.ip(), (Instant::now(), w.clone()));
        }
        Ok(w)
    }

    /// How this machine sees `w` calling from `ip`: one of my machines
    /// (`machine`), or what the book says. Not trusted, it asks (`ask`: now;
    /// else it's still being found out whether it's a machine of mine).
    pub fn caller(&self, w: &WhoIs, ip: IpAddr, machine: bool, ask: bool) -> Caller {
        let id = w.node.stable_id.clone();
        let tagged = !w.node.tags.is_empty();
        let mut c = Caller {
            name: w.device_name(),
            os: w.node.hostinfo.os.clone(),
            kind: device_kind(&w.node.hostinfo.os, &w.node.hostinfo.device_model, tagged),
            ip,
            trust: RemoteTrust::Machine,
            id,
        };
        if machine {
            return c;
        }
        let mut i = self.inner.lock().unwrap();
        c.trust = match i.book.devices.iter().find(|d| d.device == c.id).map(|d| d.answer) {
            Some(Answer::Trusted) => RemoteTrust::Trusted,
            Some(Answer::Denied) => RemoteTrust::Denied,
            Some(Answer::Untrusted) | None => RemoteTrust::Asked,
        };
        if c.trust != RemoteTrust::Asked || !ask || i.later.contains(&c.id) {
            return c;
        }
        if let Some(a) = i.asks.iter_mut().find(|a| a.id == c.id) {
            a.ip = ip.to_string();
            return c;
        }
        tracing::info!("tailnet: {} ({}) asks to be trusted", c.name, c.id);
        i.asks.push(TailnetAsk {
            id: c.id.clone(),
            device: c.name.clone(),
            os: c.os.clone(),
            kind: c.kind,
            ip: ip.to_string(),
            at: now_ms(),
        });
        while i.asks.len() > MAX_ASKS {
            i.asks.remove(0);
        }
        drop(i);
        self.publish();
        c
    }

    // ------------------------------------------------------------- Taildrive

    /// Shares the person's home on the tailnet, as them (`drive share home`).
    async fn share_home(self: Arc<Self>) {
        let (state, error) = self.try_share().await;
        {
            let mut i = self.inner.lock().unwrap();
            i.drive_busy = false;
            i.drive_tried = Some(Instant::now());
            if i.me.is_none() {
                return;
            }
            i.drive = state;
            i.drive_error = error;
        }
        self.publish();
    }

    async fn try_share(&self) -> (DriveState, Option<String>) {
        let Some(p) = (self.person)() else {
            return (DriveState::Failed, Some("There's no person on this machine to share the home of".into()));
        };
        let euid = nix::unistd::geteuid();
        let cli = if p.uid == euid.as_raw() {
            self.cli.clone()
        } else if euid.is_root() {
            self.cli.as_user(&p.name)
        } else {
            return (DriveState::Failed, Some(format!("wadd isn't root: it can't share {}'s home", p.name)));
        };
        let home = p.home.to_string_lossy().to_string();
        match cli.drive_list().await {
            Ok(shares) if shares.iter().any(|s| s.name == HOME_SHARE && s.path == home) => {
                return (DriveState::Shared, None);
            }
            Err(e) if e.drive_not_allowed() => return not_allowed(&e),
            _ => {}
        }
        match cli.drive_share(HOME_SHARE, &p.home).await {
            Ok(()) => {
                tracing::info!("tailnet: sharing {home} on the tailnet as {}", p.name);
                (DriveState::Shared, None)
            }
            Err(e) if e.drive_not_allowed() => not_allowed(&e),
            Err(e) => {
                tracing::warn!("tailnet: sharing {home}: {e}");
                (DriveState::Failed, Some(e.to_string()))
            }
        }
    }
}

fn not_allowed(e: &wad_tailscale::Error) -> (DriveState, Option<String>) {
    tracing::warn!("tailnet: Taildrive isn't allowed here: {e}");
    (DriveState::NotAllowed, Some(e.to_string()))
}

/// An answered device asks no more (until it's untrusted).
fn settle_asks(i: &mut Inner) {
    let book = &i.book;
    i.asks.retain(|a| !book.devices.iter().any(|d| d.device == a.id && d.answer != Answer::Untrusted));
}

/// What a status says that my machines depend on: its state, its
/// addresses, and each node and whether it's online.
fn signature(s: &Status) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.backend_state.hash(&mut h);
    s.tailscale_ips.hash(&mut h);
    s.tailnet_name().hash(&mut h);
    for p in s.peer_list() {
        (&p.id, p.online, &p.tailscale_ips, p.user_id, &p.tags).hash(&mut h);
    }
    h.finish()
}

/// This machine, from a status that's running.
fn me_of(s: &Status) -> Option<Me> {
    if !s.running() {
        return None;
    }
    let me = s.me.as_ref()?;
    Some(Me {
        node: me.id.clone(),
        user: me.user_id,
        login: s.me_user().map(|u| u.login_name.clone()).unwrap_or_default(),
        tailnet: s.tailnet_name().to_string(),
    })
}

/// A login under way, for as long as its job runs (dropped: cancelled, or done).
struct Joining(Arc<Tailnet>);

impl Joining {
    fn start(t: Arc<Tailnet>) -> Self {
        t.inner.lock().unwrap().joining = Some(None);
        t.publish();
        Self(t)
    }
}

impl Drop for Joining {
    fn drop(&mut self) {
        self.0.inner.lock().unwrap().joining = None;
        self.0.publish();
        self.0.poke();
    }
}

fn read_book(file: &Path) -> TrustBook {
    match std::fs::read(file) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
            tracing::warn!("{}: {e}", file.display());
            TrustBook::default()
        }),
        Err(_) => TrustBook::default(),
    }
}

/// Saves `value` to `file` (0600, whole or not at all).
pub fn save_private(file: &Path, value: &impl serde::Serialize) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = file.with_extension("json.tmp");
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(&serde_json::to_vec_pretty(value).expect("json"))?;
    f.sync_all()?;
    std::fs::rename(&tmp, file)
}

fn save_book(file: &Path, b: &TrustBook) {
    if let Err(e) = save_private(file, b) {
        tracing::warn!("tailnet: {}: {e}", file.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_proto::{MachineInfo, Profile};
    use wad_tailscale::fixtures::*;

    fn whois(json: &str) -> WhoIs {
        serde_json::from_str(json).unwrap()
    }

    fn me() -> Me {
        Me {
            node: "nSurf1CNTRL".into(),
            user: 2001,
            login: "owner@example.com".into(),
            tailnet: "owner@example.com".into(),
        }
    }

    fn tailnet(dir: &Path) -> (Arc<Tailnet>, Bus) {
        let bus = Bus::new(MachineInfo {
            id: "m-surf".into(),
            name: "Surf".into(),
            version: "0".into(),
            profile: Profile::User,
        });
        let mut config = wad_config::Config::defaults(wad_config::Profile::User);
        config.daemon.state_dir = dir.to_path_buf();
        config.tailnet.enabled = true;
        config.tailnet.socket = dir.join("nobody.sock");
        let jobs = Jobs::new(bus.clone(), dir);
        let t = Tailnet::new(Settings::new(&config), "m-surf", bus.clone(), jobs, Arc::new(|| None));
        // As a running status would have it.
        let status: Status = serde_json::from_str(STATUS_RUNNING).unwrap();
        {
            let mut i = t.inner.lock().unwrap();
            i.me = me_of(&status);
            i.status = Some(status);
            t.check_owner(&mut i);
        }
        (t, bus)
    }

    fn answer(device: &str, answer: Answer, at: i64, by: &str) -> TrustAnswer {
        TrustAnswer { device: device.into(), name: device.into(), os: "android".into(), answer, at, by: by.into() }
    }

    #[test]
    fn who_a_caller_is() {
        let me = me();
        assert_eq!(me_of(&serde_json::from_str(STATUS_RUNNING).unwrap()), Some(me.clone()));
        // The same login's, untagged, not shared: mine if it's a wadd.
        assert_eq!(classify(&me, &whois(WHOIS_DESK)), Class::SameUser);
        assert_eq!(classify(&me, &whois(WHOIS_PHONE)), Class::SameUser);
        // Tagged: Tailscale's pseudo-user, or (Headscale) the owner's own id; either way not mine.
        assert_eq!(classify(&me, &whois(WHOIS_TAGGED)), Class::Other);
        assert_eq!(classify(&me, &whois(WHOIS_TAGGED_OWNER)), Class::Other);
        // Someone else's, shared in.
        assert_eq!(classify(&me, &whois(WHOIS_SHARED)), Class::Other);
        let mut shared_mine = whois(WHOIS_DESK);
        shared_mine.node.sharer = 3001;
        assert_eq!(classify(&me, &shared_mine), Class::Other, "shared in, even as the same user's");
        let mut this = whois(WHOIS_DESK);
        this.node.stable_id = me.node.clone();
        assert_eq!(classify(&me, &this), Class::ThisMachine);
        assert!(me_of(&serde_json::from_str(STATUS_NEEDS_LOGIN).unwrap()).is_none());
    }

    #[test]
    fn what_kind_of_device() {
        assert_eq!(device_kind("android", "Google Pixel 9", false), DeviceKind::Phone);
        assert_eq!(device_kind("iOS", "iPad13,1", false), DeviceKind::Tablet);
        assert_eq!(device_kind("android", "Samsung Galaxy Tab S9", false), DeviceKind::Tablet);
        assert_eq!(device_kind("macOS", "MacBookPro18,3", false), DeviceKind::Laptop);
        assert_eq!(device_kind("linux", "", true), DeviceKind::Server);
        assert_eq!(device_kind("windows", "", false), DeviceKind::Other);
    }

    #[test]
    fn my_machines_never_ask_and_other_devices_do() {
        let dir = tempfile::tempdir().unwrap();
        let (t, _) = tailnet(dir.path());
        let ip: IpAddr = "100.64.0.5".parse().unwrap();
        let desk = t.caller(&whois(WHOIS_DESK), ip, true, true);
        assert_eq!(desk.trust, RemoteTrust::Machine);
        assert!(desk.trusted() && t.view().asks.is_empty());
        // The phone isn't a wadd: it asks, once.
        let phone = t.caller(&whois(WHOIS_PHONE), "100.64.0.7".parse().unwrap(), false, true);
        assert_eq!((phone.trust, phone.kind), (RemoteTrust::Asked, DeviceKind::Phone));
        t.caller(&whois(WHOIS_PHONE), "100.64.0.7".parse().unwrap(), false, true);
        let asks = t.view().asks;
        assert_eq!(
            asks.iter().map(|a| (a.id.as_str(), a.device.as_str())).collect::<Vec<_>>(),
            [("nPhone1CNTRL", "Pixel 9")]
        );
        // Not asking yet: it's still being found out whether it's a machine of mine.
        t.caller(&whois(WHOIS_TAGGED), "100.64.0.9".parse().unwrap(), false, false);
        assert_eq!(t.view().asks.len(), 1);
        // Trusted: no ask, and it's a trusted device of the login's.
        let (v, a) = t.trust("nPhone1CNTRL").unwrap();
        assert_eq!(a.as_ref().map(|a| (a.answer, a.by.as_str())), Some((Answer::Trusted, "m-surf")));
        assert!(v.asks.is_empty());
        assert!(v.devices.iter().any(|d| d.id == "nPhone1CNTRL" && d.trusted && !d.mine));
        assert_eq!(
            t.caller(&whois(WHOIS_PHONE), "100.64.0.7".parse().unwrap(), false, true).trust,
            RemoteTrust::Trusted
        );
        // Trusted again: nothing to tell the others.
        assert!(t.trust("nPhone1CNTRL").unwrap().1.is_none());
        // Untrusted: a device it doesn't know again, so its next call asks.
        let (_, a) = t.untrust("nPhone1CNTRL").unwrap();
        assert_eq!(a.map(|a| a.answer), Some(Answer::Untrusted));
        assert_eq!(t.caller(&whois(WHOIS_PHONE), "100.64.0.7".parse().unwrap(), false, true).trust, RemoteTrust::Asked);
        assert_eq!(t.view().asks.len(), 1);
        assert_eq!(t.trust("nope").unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn a_denied_device_stays_quiet_and_later_hides_an_ask() {
        let dir = tempfile::tempdir().unwrap();
        let (t, _) = tailnet(dir.path());
        let shared = whois(WHOIS_SHARED);
        let ip: IpAddr = "100.64.0.11".parse().unwrap();
        t.caller(&shared, ip, false, true);
        let (v, a) = t.deny("nFriend1CNTRL").unwrap();
        assert_eq!(a.map(|a| a.answer), Some(Answer::Denied));
        assert!(v.asks.is_empty());
        assert!(v.devices.iter().any(|d| d.id == "nFriend1CNTRL" && !d.trusted), "{:?}", v.devices);
        assert_eq!(t.caller(&shared, ip, false, true).trust, RemoteTrust::Denied);
        assert!(t.view().asks.is_empty(), "denied: no new ask");
        assert_eq!(t.deny("nFriend1CNTRL").unwrap_err().code, ErrorCode::NotFound);

        // Later: hidden for the session, and it doesn't ask again meanwhile.
        t.caller(&whois(WHOIS_PHONE), "100.64.0.7".parse().unwrap(), false, true);
        let v = t.later("nPhone1CNTRL").unwrap();
        assert!(v.asks.is_empty());
        t.caller(&whois(WHOIS_PHONE), "100.64.0.7".parse().unwrap(), false, true);
        assert!(t.view().asks.is_empty());
        assert_eq!(t.later("nPhone1CNTRL").unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(t.deny("nPhone1CNTRL").unwrap_err().code, ErrorCode::NotFound);
        // It can still be trusted from the devices list.
        assert!(t.trust("nPhone1CNTRL").unwrap().1.is_some());
    }

    #[test]
    fn at_most_eight_asks_the_oldest_go() {
        let dir = tempfile::tempdir().unwrap();
        let (t, _) = tailnet(dir.path());
        for n in 0..10 {
            let mut w = whois(WHOIS_SHARED);
            w.node.stable_id = format!("n{n}");
            t.caller(&w, format!("100.64.1.{n}").parse().unwrap(), false, true);
        }
        let asks = t.view().asks;
        assert_eq!(asks.len(), MAX_ASKS);
        assert_eq!((asks[0].id.as_str(), asks[7].id.as_str()), ("n2", "n9"));
    }

    #[test]
    fn my_machines_cant_be_untrusted_and_their_asks_go() {
        let dir = tempfile::tempdir().unwrap();
        let (t, _) = tailnet(dir.path());
        t.caller(&whois(WHOIS_DESK), "100.64.0.5".parse().unwrap(), false, true);
        assert_eq!(t.view().asks.len(), 1);
        let desk = MyMachine {
            node: "nDesk1CNTRL".into(),
            name: "Desk".into(),
            os: "WadOS".into(),
            kind: DeviceKind::Desktop,
            online: true,
        };
        t.set_machines(vec![desk]);
        let v = t.view();
        assert!(v.asks.is_empty(), "found to be one of mine: its ask goes");
        assert!(v.devices.iter().any(|d| d.id == "nDesk1CNTRL" && d.mine && d.trusted && d.name == "Desk"));
        assert_eq!(t.untrust("nDesk1CNTRL").unwrap_err().code, ErrorCode::Conflict);
        assert!(t.trust("nDesk1CNTRL").unwrap().1.is_none());
        // The phone is the login's own, unanswered.
        assert!(v.devices.iter().any(|d| d.id == "nPhone1CNTRL" && !d.mine && !d.trusted));
    }

    #[test]
    fn books_merge_by_the_latest_answer() {
        let mut book = TrustBook { user: "u".into(), tailnet: "t".into(), devices: vec![] };
        assert!(merge_into(&mut book, [answer("nPhone", Answer::Trusted, 10, "m-a")]));
        // Older: ignored. Newer: wins, and untrusted is an answer like any other.
        assert!(!merge_into(&mut book, [answer("nPhone", Answer::Denied, 5, "m-b")]));
        assert!(merge_into(&mut book, [answer("nPhone", Answer::Untrusted, 20, "m-b")]));
        assert_eq!(book.devices[0].answer, Answer::Untrusted);
        // A tie: the larger machine id, so both sides pick the same.
        assert!(merge_into(&mut book, [answer("nPhone", Answer::Trusted, 20, "m-c")]));
        assert!(!merge_into(&mut book, [answer("nPhone", Answer::Denied, 20, "m-a")]));
        assert_eq!((book.devices[0].answer, book.devices[0].by.as_str()), (Answer::Trusted, "m-c"));
        // The same answer again changes nothing.
        assert!(!merge_into(&mut book, [answer("nPhone", Answer::Trusted, 20, "m-c")]));
        assert!(merge_into(&mut book, [answer("nTab", Answer::Denied, 1, "m-a")]));
        assert_eq!(book.devices.len(), 2);
    }

    #[test]
    fn answers_from_my_machines_settle_asks_and_are_kept_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (t, bus) = tailnet(dir.path());
        t.caller(&whois(WHOIS_PHONE), "100.64.0.7".parse().unwrap(), false, true);
        let (_, mut rx) = bus.subscribe();
        assert!(t.merge(vec![answer("nPhone1CNTRL", Answer::Trusted, now_ms(), "m-desk")]));
        assert!(t.view().asks.is_empty());
        assert!(matches!(rx.try_recv(), Ok(Event::Tailnet(v)) if v.asks.is_empty()));
        assert!(!t.merge(vec![answer("nPhone1CNTRL", Answer::Denied, 1, "m-desk")]), "older");
        let file = dir.path().join("tailnet-devices.json");
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        let saved: TrustBook = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!((saved.user.as_str(), saved.devices.len()), ("owner@example.com", 1));
        // My own answer about it is later than the one merged.
        let (_, a) = t.untrust_for_test("nPhone1CNTRL");
        assert!(a.at > saved.devices[0].at);
    }

    #[test]
    fn the_book_starts_again_for_another_login() {
        let dir = tempfile::tempdir().unwrap();
        let (t, _) = tailnet(dir.path());
        t.merge(vec![answer("nPhone1CNTRL", Answer::Trusted, 5, "m-desk")]);
        assert_eq!(t.book().devices.len(), 1);
        // Same login after a restart: kept.
        drop(t);
        let (t, _) = tailnet(dir.path());
        assert_eq!(t.book().devices.len(), 1);
        {
            let mut i = t.inner.lock().unwrap();
            i.me = Some(Me { login: "someone@else.com".into(), ..me() });
            t.check_owner(&mut i);
        }
        let b = t.book();
        assert_eq!((b.user.as_str(), b.devices.len()), ("someone@else.com", 0));
        {
            let mut i = t.inner.lock().unwrap();
            i.me = Some(Me { tailnet: "other.ts.net".into(), login: "someone@else.com".into(), ..me() });
            t.check_owner(&mut i);
        }
        assert_eq!(t.book().tailnet, "other.ts.net");
    }

    #[test]
    fn names_tailscale_takes() {
        assert_eq!(host_label("Surface Book 2"), "surface-book-2");
        assert_eq!(host_label("  wad's  machine!! "), "wad-s-machine");
        assert_eq!(host_label("!!!"), "wadspaces");
        assert_eq!(host_label(&"x".repeat(80)).len(), 63);
    }

    impl Tailnet {
        fn untrust_for_test(&self, device: &str) -> (wad_proto::Tailnet, TrustAnswer) {
            let (v, a) = self.untrust(device).unwrap();
            (v, a.expect("an answer"))
        }
    }
}
