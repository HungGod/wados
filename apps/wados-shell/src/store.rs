//! What the shell knows of the machine: what wadd says, kept current from
//! its event stream (wadd.rs). The shell's surfaces read it and watch it;
//! they never change it themselves (every change goes to wadd, which tells
//! everyone, this store included).
//!
//! Phase 6's: the tailnet (its devices and asks), my other machines
//! (peers), the sign-ins waiting in the browser (the GitHub account is
//! Settings' to show: here it's only told as a change), and
//! each peer's wadspaces as it last sent them (asked for by `peers.rs`;
//! they come from it, over the tailnet, not from events).
//!
//! A wadspace's id is only unique on its machine (every machine has its
//! starter, `wadspaces`; a copy keeps its source's id), so a peer's are kept
//! here under a key of their machine's: `<peer id>:<its id>` ([`peer_key`]).
//! A wadspace's id never has a `:` (`wad_model::check::id`), so a key is
//! never one of this machine's ids, and the two are never taken for each
//! other. Calls to wadd take the id back off ([`split_key`]).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use indexmap::IndexMap;
use wad_model::StreamStage;
use wad_model::edit::Library;
use wad_model::{Phase, SessionDisplay, Wadspace};
use wad_proto::{
    Event, JobInfo, Level, MachineInfo, PeerInfo, ScreenInfo, SignInProvider, StreamInfo, StreamsAvailability, Tailnet,
    WadspaceBuild, WadspaceRun,
};

/// What changed, for watchers.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// Focus started or ended.
    Focus,
    /// An agent or a sandbox changed.
    Agents,
    /// Connected to wadd, or lost it (it's followed again by itself).
    Connected(bool),
    Machine,
    /// The whole library (tags, keys, or everything at once).
    Library,
    /// One wadspace's document (made, changed, or gone).
    Wadspace(String),
    Run(String),
    Build(String),
    Job(String),
    Screen,
    /// Something to tell the person.
    Notice {
        level: Level,
        title: String,
        undo: Option<String>,
        /// Phase 8's: a line under it, its icon, a button of its own, and
        /// whether it's a toast too (else only in the bell's Notifications).
        body: Option<String>,
        icon: Option<String>,
        action: Option<wad_proto::NoticeAction>,
        toast: bool,
    },
    /// The tailnet: its state, devices or asks.
    Tailnet,
    /// My other machines (the list, or one's online).
    Peers,
    /// The GitHub account (signed in or out).
    Github,
    /// A sign-in in the browser began (or loads elsewhere), or is done.
    SignIn(SignInProvider),
    /// A peer's wadspaces came (or couldn't).
    PeerSpaces(String),
    /// The streams, or which modes can start.
    Streams,
    /// WadOS's own update (staged, checking…).
    Updates,
    /// The network (Wi-Fi, the wire, airplane mode), as wadd last said.
    Network,
    /// Bluetooth and its devices, as wadd last said.
    Bluetooth,
}

/// The key a peer's wadspace is kept under: `<peer id>:<its id there>`.
pub fn peer_key(peer: &str, ws: &str) -> String {
    format!("{peer}:{ws}")
}

/// A peer's wadspace's key, as its machine and its id there (none: one of this machine's ids).
pub fn split_key(key: &str) -> Option<(&str, &str)> {
    key.split_once(':').filter(|(p, w)| !p.is_empty() && !w.is_empty())
}

/// A stream's key: its wadspace's (this machine's id, or a peer's key, [`peer_key`]).
pub fn stream_key(s: &StreamInfo) -> String {
    match &s.peer {
        Some(p) => peer_key(p, &s.id),
        None => s.id.clone(),
    }
}

/// A sign-in waiting in the browser (wadd's `signIn`): where, and GitHub's code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignIn {
    pub url: String,
    pub code: Option<String>,
}

/// One of my machines' wadspaces, as it last sent them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PeerSpaces {
    /// Its live wadspaces, in its order, each under its key ([`peer_key`]).
    pub list: Vec<Wadspace>,
    /// Being asked for now.
    pub loading: bool,
    /// The last ask didn't work: why, in wadd's words.
    pub error: Option<String>,
    /// When they last came (ms; 0 never).
    pub at: i64,
}

type Watcher = Box<dyn Fn(&Change)>;

#[derive(Default)]
pub struct Store {
    connected: Cell<bool>,
    machine: RefCell<Option<MachineInfo>>,
    library: RefCell<Library>,
    runs: RefCell<IndexMap<String, WadspaceRun>>,
    builds: RefCell<IndexMap<String, WadspaceBuild>>,
    jobs: RefCell<IndexMap<String, JobInfo>>,
    screen: RefCell<Option<ScreenInfo>>,
    focus: RefCell<Option<wad_model::Focus>>,
    agents: RefCell<wad_proto::Agents>,
    tailnet: RefCell<Option<Tailnet>>,
    peers: RefCell<Vec<PeerInfo>>,
    signins: RefCell<IndexMap<SignInProvider, SignIn>>,
    peer_spaces: RefCell<IndexMap<String, PeerSpaces>>,
    /// Copies this shell started: each job's wadspace's key.
    copies: RefCell<IndexMap<String, String>>,
    streams: RefCell<Vec<StreamInfo>>,
    /// Which modes can start (none: wadd hasn't said).
    stream_modes: RefCell<Option<StreamsAvailability>>,
    /// How many `streams` events have come (a list asked for is only taken if none came meanwhile).
    streams_said: Cell<u64>,
    /// WadOS's own update, as wadd last said (the tray's Updates tile).
    updates: RefCell<Option<wad_proto::Updates>>,
    /// The network and Bluetooth, as wadd last said (the tray's flyouts).
    network: RefCell<Option<wad_proto::Network>>,
    bluetooth: RefCell<Option<wad_proto::Bluetooth>>,
    watchers: RefCell<Vec<Watcher>>,
}

impl Store {
    pub fn new() -> Rc<Self> {
        Rc::default()
    }

    /// Calls `f` on every change from now on.
    pub fn watch(&self, f: impl Fn(&Change) + 'static) {
        self.watchers.borrow_mut().push(Box::new(f));
    }

    fn tell(&self, c: Change) {
        // Watchers may read the store (never borrow_mut it), so it's free here.
        for w in self.watchers.borrow().iter() {
            w(&c);
        }
    }

    pub fn set_connected(&self, on: bool) {
        if self.connected.replace(on) != on {
            self.tell(Change::Connected(on));
        }
    }

    /// One of wadd's events.
    pub fn apply(&self, e: Event) {
        let change = match e {
            Event::Machine(m) => {
                *self.machine.borrow_mut() = Some(m);
                Change::Machine
            }
            Event::Library(l) => {
                *self.library.borrow_mut() = *l;
                Change::Library
            }
            Event::Wadspace(w) => {
                let id = w.id.clone();
                let mut lib = self.library.borrow_mut();
                match lib.wadspaces.iter_mut().find(|x| x.id == id) {
                    Some(x) => *x = *w,
                    None => lib.wadspaces.push(*w),
                }
                Change::Wadspace(id)
            }
            Event::WadspaceGone { id } => {
                self.library.borrow_mut().wadspaces.retain(|x| x.id != id);
                self.runs.borrow_mut().shift_remove(&id);
                self.builds.borrow_mut().shift_remove(&id);
                Change::Wadspace(id)
            }
            Event::Run(r) => {
                let id = r.id.clone();
                self.runs.borrow_mut().insert(id.clone(), *r);
                Change::Run(id)
            }
            Event::Build(b) => {
                let id = b.id.clone();
                self.builds.borrow_mut().insert(id.clone(), *b);
                Change::Build(id)
            }
            Event::Job { job } => {
                let id = job.job.id.clone();
                self.jobs.borrow_mut().insert(id.clone(), *job);
                Change::Job(id)
            }
            Event::JobLines { .. } | Event::Server { .. } => return,
            Event::JobGone { id } => {
                self.jobs.borrow_mut().shift_remove(&id);
                Change::Job(id)
            }
            Event::Screen(s) => {
                *self.screen.borrow_mut() = Some(s);
                Change::Screen
            }
            Event::Notice { level, title, undo, body, icon, action, toast } => {
                Change::Notice { level, title, undo, body, icon, action, toast }
            }
            Event::Tailnet(t) => {
                *self.tailnet.borrow_mut() = Some(t);
                Change::Tailnet
            }
            Event::Peers { peers } => {
                // A machine that's gone keeps nothing here.
                self.peer_spaces.borrow_mut().retain(|id, _| peers.iter().any(|p| &p.peer.id == id));
                *self.peers.borrow_mut() = peers;
                Change::Peers
            }
            // The shell shows no account: Settings does.
            Event::Github { .. } => Change::Github,
            Event::Agents(a) => {
                *self.agents.borrow_mut() = *a;
                Change::Agents
            }
            Event::Focus { focus } => {
                *self.focus.borrow_mut() = focus;
                Change::Focus
            }
            Event::SignIn { provider, url, code } => {
                self.signins.borrow_mut().insert(provider, SignIn { url, code });
                Change::SignIn(provider)
            }
            Event::SignInDone { provider } => {
                self.signins.borrow_mut().shift_remove(&provider);
                Change::SignIn(provider)
            }
            Event::Updates(u) => {
                *self.updates.borrow_mut() = Some(*u);
                Change::Updates
            }
            Event::Streams { streams, available } => {
                *self.streams.borrow_mut() = streams;
                *self.stream_modes.borrow_mut() = Some(available);
                self.streams_said.set(self.streams_said.get() + 1);
                Change::Streams
            }
            // Phase 8's system state: the shell doesn't keep it (yet).
            Event::Network(n) => {
                *self.network.borrow_mut() = Some(*n);
                Change::Network
            }
            Event::Bluetooth(b) => {
                *self.bluetooth.borrow_mut() = Some(*b);
                Change::Bluetooth
            }
            // Pairing's prompts are Settings' (pairing starts there).
            // The account and its secret files are Settings' too.
            Event::BtPrompt { .. } | Event::Storage(_) | Event::Account(_) | Event::SecretFiles(_) => {
                return;
            }
        };
        self.tell(change);
    }

    pub fn connected(&self) -> bool {
        self.connected.get()
    }

    pub fn machine(&self) -> Option<MachineInfo> {
        self.machine.borrow().clone()
    }

    pub fn machine_id(&self) -> String {
        self.machine.borrow().as_ref().map(|m| m.id.clone()).unwrap_or_default()
    }

    pub fn library(&self) -> std::cell::Ref<'_, Library> {
        self.library.borrow()
    }

    pub fn wadspace(&self, id: &str) -> Option<Wadspace> {
        self.library.borrow().wadspace(id).cloned()
    }

    /// This machine's wadspaces not in the Trash, in order.
    pub fn live(&self) -> Vec<Wadspace> {
        let m = self.machine_id();
        self.library.borrow().on(&m).into_iter().cloned().collect()
    }

    pub fn run(&self, id: &str) -> Option<WadspaceRun> {
        self.runs.borrow().get(id).cloned()
    }

    pub fn build(&self, id: &str) -> Option<WadspaceBuild> {
        self.builds.borrow().get(id).cloned()
    }

    pub fn job(&self, id: &str) -> Option<JobInfo> {
        self.jobs.borrow().get(id).cloned()
    }

    pub fn agents(&self) -> wad_proto::Agents {
        self.agents.borrow().clone()
    }

    pub fn focus(&self) -> Option<wad_model::Focus> {
        self.focus.borrow().clone()
    }

    pub fn screen(&self) -> Option<ScreenInfo> {
        self.screen.borrow().clone()
    }

    // ------------------------------------------------------------ phase 7

    /// The streams (not ended), here and from my other machines, newest last.
    pub fn streams(&self) -> Vec<StreamInfo> {
        self.streams.borrow().iter().filter(|s| s.stage != StreamStage::Ended).cloned().collect()
    }

    /// The stream of a wadspace (this machine's by its id, a peer's by its key), if it streams.
    pub fn stream(&self, key: &str) -> Option<StreamInfo> {
        self.streams.borrow().iter().find(|s| s.stage != StreamStage::Ended && stream_key(s) == key).cloned()
    }

    /// Which modes can start here (none: wadd hasn't said).
    pub fn stream_modes(&self) -> Option<StreamsAvailability> {
        self.stream_modes.borrow().clone()
    }

    /// How many `streams` events have come so far.
    pub fn streams_said(&self) -> u64 {
        self.streams_said.get()
    }

    /// The streams as `GET /v1/streams` answered (asked for when `asked` of
    /// them had come): taken unless an event said them since (it's newer).
    pub fn set_streams(&self, list: wad_proto::StreamList, asked: u64) -> bool {
        if self.streams_said.get() != asked {
            return false;
        }
        self.apply(list.into());
        true
    }

    // ------------------------------------------------------------ phase 6

    /// WadOS's own update, if wadd has said.
    /// The network, once wadd's said (none on a wadd without it).
    pub fn network(&self) -> Option<wad_proto::Network> {
        self.network.borrow().clone()
    }

    pub fn bluetooth(&self) -> Option<wad_proto::Bluetooth> {
        self.bluetooth.borrow().clone()
    }

    pub fn updates(&self) -> Option<wad_proto::Updates> {
        self.updates.borrow().clone()
    }

    /// The tailnet, once wadd has said (none: not yet).
    pub fn tailnet(&self) -> Option<Tailnet> {
        self.tailnet.borrow().clone()
    }

    /// My other machines.
    pub fn peers(&self) -> Vec<PeerInfo> {
        self.peers.borrow().clone()
    }

    pub fn peer(&self, id: &str) -> Option<PeerInfo> {
        self.peers.borrow().iter().find(|p| p.peer.id == id).cloned()
    }

    /// A machine's name: this one's, a peer's, or none.
    pub fn machine_name(&self, id: &str) -> Option<String> {
        if let Some(m) = self.machine.borrow().as_ref().filter(|m| m.id == id) {
            return Some(m.name.clone());
        }
        self.peers.borrow().iter().find(|p| p.peer.id == id).map(|p| p.peer.name.clone())
    }

    /// The sign-in waiting in the browser for `provider`, if one is.
    pub fn sign_in(&self, provider: SignInProvider) -> Option<SignIn> {
        self.signins.borrow().get(&provider).cloned()
    }

    /// A peer's wadspaces, as it last sent them (none: never asked).
    pub fn peer_spaces(&self, peer: &str) -> Option<PeerSpaces> {
        self.peer_spaces.borrow().get(peer).cloned()
    }

    /// Asking `peer` for its wadspaces (they're kept as they were meanwhile).
    pub fn peer_spaces_asked(&self, peer: &str) {
        self.peer_spaces.borrow_mut().entry(peer.to_string()).or_default().loading = true;
        self.tell(Change::PeerSpaces(peer.to_string()));
    }

    /// `peer`'s wadspaces came (or why they didn't), at `now` (ms).
    pub fn set_peer_spaces(&self, peer: &str, got: Result<Vec<Wadspace>, String>, now: i64) {
        {
            let mut all = self.peer_spaces.borrow_mut();
            let e = all.entry(peer.to_string()).or_default();
            e.loading = false;
            match got {
                Ok(mut list) => {
                    list.retain(|x| !x.is_trashed());
                    list.sort_by_key(|x| x.order);
                    for x in &mut list {
                        x.id = peer_key(peer, &x.id);
                    }
                    *e = PeerSpaces { list, loading: false, error: None, at: now };
                }
                Err(why) => e.error = Some(why),
            }
        }
        self.tell(Change::PeerSpaces(peer.to_string()));
    }

    /// One of a peer's wadspaces, by its key ([`peer_key`]), and whose it is.
    pub fn peer_wadspace(&self, key: &str) -> Option<(String, Wadspace)> {
        let (peer, _) = split_key(key)?;
        let all = self.peer_spaces.borrow();
        all.get(peer)?.list.iter().find(|x| x.id == key).map(|x| (peer.to_string(), x.clone()))
    }

    /// A copy this shell started: its job copies the peer's wadspace `key`.
    pub fn note_copy(&self, job: &str, key: &str) {
        self.copies.borrow_mut().insert(job.to_string(), key.to_string());
        self.tell(Change::Job(job.to_string()));
    }

    /// The key of the peer's wadspace a CopyWadspace job copies: the one
    /// this shell asked for; else the machine its `peer` says (if wadd says);
    /// else the one peer that has a wadspace by that id (none when several do).
    pub fn copy_key(&self, j: &JobInfo) -> Option<String> {
        if j.job.kind != wad_model::JobKind::CopyWadspace {
            return None;
        }
        if let Some(k) = self.copies.borrow().get(&j.job.id) {
            return Some(k.clone());
        }
        let ws = j.job.subject.as_deref()?;
        if let Some(peer) = j.job.params.get("peer").and_then(|p| p.as_str()) {
            return Some(peer_key(peer, ws));
        }
        let all = self.peer_spaces.borrow();
        let mut has = all.iter().filter(|(p, s)| s.list.iter().any(|x| x.id == peer_key(p, ws)));
        match (has.next(), has.next()) {
            (Some((p, _)), None) => Some(peer_key(p, ws)),
            _ => None,
        }
    }

    /// How far a copy here of the peer's wadspace `key` has got, while one runs.
    pub fn copying(&self, key: &str) -> Option<f64> {
        split_key(key)?;
        self.jobs
            .borrow()
            .values()
            .find(|j| {
                matches!(j.job.status, wad_model::JobStatus::Queued | wad_model::JobStatus::Running)
                    && self.copy_key(j).as_deref() == Some(key)
            })
            .map(|j| j.job.progress)
    }

    /// A wadspace by this machine's id, or a peer's by its key: never one for the other.
    pub fn any_wadspace(&self, id: &str) -> Option<Wadspace> {
        match split_key(id) {
            Some(_) => self.peer_wadspace(id).map(|(_, x)| x),
            None => self.wadspace(id),
        }
    }

    /// It's running (or on its way up) on this machine.
    pub fn is_running(&self, id: &str) -> bool {
        self.run(id).is_some_and(|r| matches!(r.state.phase, Phase::Starting | Phase::Waiting | Phase::Ready))
    }

    /// How it's shown, while it runs.
    pub fn display(&self, id: &str) -> Option<SessionDisplay> {
        self.run(id)
            .filter(|r| matches!(r.state.phase, Phase::Starting | Phase::Waiting | Phase::Ready))
            .and_then(|r| r.display)
    }

    /// The running wadspaces, in the library's order.
    pub fn running(&self) -> Vec<Wadspace> {
        self.live().into_iter().filter(|w| self.is_running(&w.id)).collect()
    }

    /// Its tile's badges.
    pub fn tile_state(&self, id: &str) -> wad_gtk::TileState {
        let run = self.run(id);
        let build = self.build(id);
        let busy =
            build.as_ref().and_then(|b| b.job.as_deref()).and_then(|j| self.job(j)).map(|j| j.job.progress).or_else(
                || run.as_ref().filter(|r| matches!(r.state.phase, Phase::Starting | Phase::Waiting)).map(|_| 0.0),
            );
        let streaming = self.stream(id).map(|s| match s.live {
            true => wad_gtk::Streaming::Live(s.viewers),
            false => wad_gtk::Streaming::Starting,
        });
        wad_gtk::TileState {
            running: self.is_running(id),
            streaming,
            unbuilt: build.as_ref().is_some_and(|b| b.state.needs_build()),
            failed: run.is_some_and(|r| r.state.phase == Phase::Error),
            busy,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn job(id: &str, subject: &str, status: &str, progress: f64, peer: Option<&str>) -> Event {
        let mut params = serde_json::Map::new();
        if let Some(p) = peer {
            params.insert("peer".into(), json!(p));
        }
        serde_json::from_value(json!({"kind": "job", "job": {
            "id": id, "kind": "copyWadspace", "status": status, "title": "Copying WadOS to this machine",
            "subject": subject, "progress": progress, "startedAt": 1, "source": "machine", "visible": true,
            "lineCount": 0, "params": params
        }}))
        .unwrap()
    }

    fn ws(id: &str, name: &str, machine: &str) -> Wadspace {
        wad_model::ops::blank(id, name, machine, 0, 1)
    }

    /// This machine (m-surface) and Desk both have their starter, `wadspaces`.
    fn two_starters() -> Store {
        let s = Store::default();
        s.apply(Event::Machine(MachineInfo {
            id: "m-surface".into(),
            name: "Surface".into(),
            version: "0".into(),
            profile: wad_proto::Profile::System,
        }));
        let mut lib = Library::default();
        lib.wadspaces.push(ws("wadspaces", "WadOS", "m-surface"));
        s.apply(Event::Library(Box::new(lib)));
        let desk = vec![ws("wadspaces", "WadOS", "m-desk"), ws("pixel-art", "Pixel Art", "m-desk")];
        s.set_peer_spaces("m-desk", Ok(desk), 1);
        s.set_peer_spaces("m-laptop", Ok(vec![ws("pixel-art", "Pixel Art", "m-laptop")]), 1);
        s
    }

    #[test]
    fn a_peers_wadspace_is_never_taken_for_this_machines_of_the_same_id() {
        let s = two_starters();
        // This machine's, by its id; Desk's, by its key: each its own.
        assert_eq!(s.wadspace("wadspaces").map(|x| x.machine).as_deref(), Some("m-surface"));
        assert_eq!(s.any_wadspace("wadspaces").map(|x| x.machine).as_deref(), Some("m-surface"));
        let desks = s.any_wadspace("m-desk:wadspaces").unwrap();
        assert_eq!((desks.id.as_str(), desks.machine.as_str()), ("m-desk:wadspaces", "m-desk"));
        assert_eq!(s.peer_wadspace("m-desk:wadspaces").map(|(p, _)| p).as_deref(), Some("m-desk"));
        // A bare id is never a peer's, and a key never this machine's.
        assert!(s.peer_wadspace("wadspaces").is_none());
        assert!(s.wadspace("m-desk:wadspaces").is_none());
        assert!(s.any_wadspace("m-laptop:wadspaces").is_none());
        // The live list is this machine's alone.
        assert_eq!(s.live().len(), 1);
        assert_eq!(split_key("m-desk:wadspaces"), Some(("m-desk", "wadspaces")));
        assert_eq!(split_key("wadspaces"), None);
    }

    #[test]
    fn a_copy_shows_on_the_card_it_copies_and_no_other() {
        let s = two_starters();
        // Only Desk has a `wadspaces` among my machines: a copy of one is Desk's,
        // and this machine's starter of the same id never shows it.
        s.apply(job("j1", "wadspaces", "running", 0.25, None));
        assert_eq!(s.copying("m-desk:wadspaces"), Some(0.25));
        assert_eq!(s.copying("wadspaces"), None);
        // Desk and Laptop both have `pixel-art`: which one's isn't known…
        s.apply(job("j2", "pixel-art", "running", 0.5, None));
        assert_eq!(s.copy_key(&s.job("j2").unwrap()), None);
        assert_eq!((s.copying("m-desk:pixel-art"), s.copying("m-laptop:pixel-art")), (None, None));
        // …until this shell says it asked for Laptop's,
        s.note_copy("j2", "m-laptop:pixel-art");
        assert_eq!((s.copying("m-desk:pixel-art"), s.copying("m-laptop:pixel-art")), (None, Some(0.5)));
        // or wadd names the machine.
        s.apply(job("j3", "pixel-art", "running", 0.75, Some("m-desk")));
        assert_eq!(s.copying("m-desk:pixel-art"), Some(0.75));
        // Done: nothing shows.
        s.apply(job("j1", "wadspaces", "done", 1.0, None));
        assert_eq!(s.copying("m-desk:wadspaces"), None);
    }

    fn stream(id: &str, peer: Option<&str>, live: bool, stage: StreamStage) -> StreamInfo {
        StreamInfo {
            id: id.into(),
            mode: wad_model::StreamMode::Private,
            transport: wad_model::Transport::Websocket,
            stage,
            live,
            viewers: 3,
            started: 1,
            last_active: 1,
            link: "https://x.invalid/ws-1/".into(),
            turn_down: false,
            warnings: vec![],
            job: None,
            peer: peer.map(String::from),
        }
    }

    #[test]
    fn a_peers_stream_is_never_taken_for_this_machines_of_the_same_id() {
        let s = two_starters();
        s.apply(Event::Streams {
            streams: vec![
                stream("wadspaces", Some("m-desk"), false, StreamStage::Sidecar),
                stream("pixel-art", None, true, StreamStage::Ended),
            ],
            available: StreamsAvailability::of(true, false, false),
        });
        assert!(s.stream("wadspaces").is_none(), "Desk's isn't this machine's");
        assert_eq!(s.stream("m-desk:wadspaces").map(|x| x.live), Some(false));
        assert_eq!(s.tile_state("wadspaces").streaming, None);
        // One that ended isn't streaming.
        assert!(s.stream("pixel-art").is_none());
        assert_eq!(s.streams().len(), 1);
        assert!(!s.stream_modes().unwrap().public.available);
        s.apply(Event::Streams {
            streams: vec![stream("wadspaces", None, true, StreamStage::Live)],
            available: StreamsAvailability::of(true, true, false),
        });
        assert_eq!(s.tile_state("wadspaces").streaming, Some(wad_gtk::Streaming::Live(3)));
        assert_eq!(stream_key(&stream("notes", Some("m-desk"), true, StreamStage::Live)), "m-desk:notes");
    }

    #[test]
    fn a_list_asked_for_is_taken_unless_an_event_said_it_since() {
        let s = Store::default();
        let list = |public: bool| wad_proto::StreamList {
            streams: vec![],
            available: StreamsAvailability::of(true, public, false),
        };
        // Connected (after a restart): nothing said yet, the list answers it.
        assert!(s.stream_modes().is_none());
        let asked = s.streams_said();
        assert!(s.set_streams(list(true), asked));
        assert!(s.stream_modes().unwrap().public.available);
        // An event came while another list was on its way: the event's newer.
        let asked = s.streams_said();
        s.apply(Event::Streams { streams: vec![], available: StreamsAvailability::of(true, false, false) });
        assert!(!s.set_streams(list(true), asked));
        assert!(!s.stream_modes().unwrap().public.available);
    }

    #[test]
    fn sign_ins_come_and_go() {
        let s = Store::default();
        s.apply(Event::SignIn { provider: SignInProvider::Github, url: "u".into(), code: Some("C".into()) });
        assert_eq!(s.sign_in(SignInProvider::Github).and_then(|x| x.code).as_deref(), Some("C"));
        s.apply(Event::SignInDone { provider: SignInProvider::Github });
        assert!(s.sign_in(SignInProvider::Github).is_none());
    }
}
