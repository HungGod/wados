//! Phase 6's sample data, for building the shell and the apps against before
//! the real tailnet, peers and GitHub are made (`Parts::sample`: wadd's tests,
//! and `wadd serve --sample`). It answers every route of
//! [`crate::api_phase6`] and keeps what it's told for as long as wadd runs:
//!
//! - **The tailnet:** running on `tail1234.ts.net`; three devices (Desk and
//!   Laptop, my machines; a trusted Pixel 9) and one ask (a Galaxy Tab S9).
//!   Trust, untrust, deny and Later change it; logout leaves it needing a
//!   login, and login is a Tailnet job that signs in "in the browser"
//!   (`signIn`, then `signInDone`) and comes back running.
//! - **Peers:** Desk (online, with two wadspaces) and Laptop (offline). Copy
//!   here is a CopyWadspace job that applies `space.import` to this library.
//! - **GitHub:** signed in as hunggod, with three repositories. Sign-in is a
//!   Github job with a device code, like the real one; sign-out forgets it.
//!
//! Each stage of its jobs takes `step`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use wad_model::edit::Op;
use wad_model::{AppId, AppIn, JobKind, KeyRef, PartState, Peer, PeerKind, Project, ProjectKind, ProjectSource};
use wad_model::{SecretKind, Wadspace};
use wad_proto::github::FIRST_SCOPES;
use wad_proto::{
    ApiError, DeviceKind, ErrorCode, Event, GithubAccount, JobInfo, PeerInfo, Repo, SignInProvider, SignInRequest,
    Tailnet, TailnetAsk, TailnetDevice, TailnetState,
};

use crate::api_phase6::{Accounts, Net};
use crate::builds::Builds;
use crate::events::Bus;
use crate::jobs::{Jobs, now_ms};
use crate::library::Docs;

const TAILNET: &str = "tail1234.ts.net";
const HOUR: i64 = 3_600_000;

/// Cheap to clone: one sample, shared (the API holds it as [`Net`] and as [`Accounts`]).
#[derive(Clone)]
pub struct Sample(Arc<Inner>);

struct Inner {
    st: Mutex<St>,
    hostname: String,
    jobs: Arc<Jobs>,
    bus: Bus,
    docs: Arc<Docs>,
    builds: Arc<Builds>,
    step: Duration,
}

struct St {
    tailnet: Tailnet,
    peers: Vec<PeerInfo>,
    github: Option<GithubAccount>,
}

fn device(id: &str, name: &str, os: &str, kind: DeviceKind, online: bool, mine: bool) -> TailnetDevice {
    TailnetDevice { id: id.into(), name: name.into(), os: os.into(), kind, online, trusted: true, mine }
}

fn running(hostname: &str, now: i64) -> Tailnet {
    Tailnet {
        state: TailnetState::Running,
        tailnet: TAILNET.into(),
        hostname: hostname.into(),
        ip: "100.101.102.103".into(),
        login_url: None,
        devices: vec![
            device("nDesk1", "Desk", "linux", DeviceKind::Desktop, true, true),
            device("nLaptop1", "Laptop", "linux", DeviceKind::Laptop, false, true),
            device("nPhone1", "Pixel 9", "android", DeviceKind::Phone, true, false),
        ],
        asks: vec![TailnetAsk {
            id: "nTab1".into(),
            device: "Galaxy Tab S9".into(),
            os: "android".into(),
            kind: DeviceKind::Tablet,
            ip: "100.88.20.7".into(),
            at: now - 60_000,
        }],
    }
}

fn peer(id: &str, name: &str, kind: PeerKind, online: bool, last_seen: i64, gpu: Option<&str>) -> PeerInfo {
    let hostname = name.to_lowercase();
    PeerInfo {
        peer: Peer {
            id: id.into(),
            name: name.into(),
            hostname: hostname.clone(),
            kind,
            trusted: true,
            online,
            last_seen,
            os: "WadOS 44.20261021.0 · Fedora 44".into(),
            gpu: gpu.map(String::from),
        },
        node_id: format!("n{name}1"),
        dns_name: format!("{hostname}.{TAILNET}"),
    }
}

fn peers(now: i64) -> Vec<PeerInfo> {
    vec![
        peer("m-desk", "Desk", PeerKind::Desktop, true, now, Some("AMD Radeon 780M")),
        peer("m-laptop", "Laptop", PeerKind::Laptop, false, now - 51 * HOUR, None),
    ]
}

fn hunggod() -> GithubAccount {
    GithubAccount { login: "hunggod".into(), name: "Hung".into(), scopes: FIRST_SCOPES.map(String::from).to_vec() }
}

/// A peer's live wadspaces, as its remote API would send them.
pub fn peer_wadspaces(peer: &str, now: i64) -> Vec<Wadspace> {
    let mut art = wad_model::ops::blank("pixel-art", "Pixel Art", peer, now - 30 * 24 * HOUR, 1);
    art.apps = vec![AppIn::new(AppId::Libresprite), AppIn::new(AppId::Krita)];
    let source = ProjectSource { machine_id: peer.into(), paths: vec!["~/Pictures/Sprites".into()] };
    art.look.projects = vec![Project {
        id: "sprites".into(),
        as_name: "Sprites".into(),
        kind: ProjectKind::Mount { source, make: false },
    }];
    art.look.layout.icons = ["app:libresprite", "app:krita", "project:sprites"].map(String::from).to_vec();
    art.tag = Some("studio".into());
    let mut notes = wad_model::ops::blank("notes", "Notes", peer, now - 9 * 24 * HOUR, 2);
    notes.apps = vec![AppIn::new(AppId::Obsidian)];
    notes.look.layout.icons = vec!["app:obsidian".into()];
    // A key only the peer has: a copy here signs in with nothing.
    notes.secrets.by_slot.insert(
        "app:obsidian".into(),
        KeyRef { id: "desk-vault".into(), name: "Desk vault".into(), kind: SecretKind::Obsidian },
    );
    vec![art, notes]
}

fn repos(now: i64) -> Vec<Repo> {
    let repo = |name: &str, private: bool, hours: i64, description: &str| Repo {
        full_name: format!("HungGod/{name}"),
        name: name.into(),
        private,
        url: format!("https://github.com/HungGod/{name}.git"),
        default_branch: "main".into(),
        pushed_at: Some(now - hours * HOUR),
        description: description.into(),
    };
    vec![
        repo("WadSpaces", true, 2, "Workspaces as containers"),
        repo("VanuaAcademy", true, 26, "The academy's site"),
        repo("IntelligenceQuest", false, 24 * 12, "A game about learning"),
    ]
}

fn not_found(what: &str, id: &str) -> ApiError {
    ApiError::not_found(format!("There's no {what} {id:?}"))
}

impl Sample {
    /// The sample, its present told to the watchers.
    pub fn new(
        hostname: &str,
        jobs: Arc<Jobs>,
        bus: Bus,
        docs: Arc<Docs>,
        builds: Arc<Builds>,
        step: Duration,
    ) -> Self {
        let now = now_ms();
        let st = St { tailnet: running(hostname, now), peers: peers(now), github: Some(hunggod()) };
        let me = Inner { st: Mutex::new(st), hostname: hostname.into(), jobs, bus, docs, builds, step };
        me.tell_tailnet();
        me.tell_peers();
        me.tell_github();
        Self(Arc::new(me))
    }
}

impl Inner {
    fn tell_tailnet(&self) -> Tailnet {
        let t = self.st.lock().unwrap().tailnet.clone();
        self.bus.publish(Event::Tailnet(t.clone()));
        t
    }

    fn tell_peers(&self) {
        let peers = self.st.lock().unwrap().peers.clone();
        self.bus.publish(Event::Peers { peers });
    }

    fn tell_github(&self) {
        let account = self.st.lock().unwrap().github.clone();
        self.bus.publish(Event::Github { account });
    }

    /// Changes the tailnet (an error: nothing changed), then tells it.
    fn tailnet_with(&self, f: impl FnOnce(&mut Tailnet) -> Result<(), ApiError>) -> Result<Tailnet, ApiError> {
        f(&mut self.st.lock().unwrap().tailnet)?;
        Ok(self.tell_tailnet())
    }

    fn set_state(&self, state: TailnetState, login_url: Option<&str>) {
        {
            let t = &mut self.st.lock().unwrap().tailnet;
            t.state = state;
            t.login_url = login_url.map(String::from);
        }
        self.tell_tailnet();
    }

    /// A job of `stages`, run by `work` on its own task (cancelling it drops the task).
    fn job<F>(
        self: &Arc<Self>,
        kind: JobKind,
        title: &str,
        subject: &str,
        stages: &[(&str, String)],
        work: impl FnOnce(Arc<Self>, String) -> F,
    ) -> Result<JobInfo, ApiError>
    where
        F: Future<Output = Result<(), String>> + Send + 'static,
    {
        let stages: Vec<(String, String)> = stages.iter().map(|(id, label)| (id.to_string(), label.clone())).collect();
        let id = self.jobs.create(kind, title, subject, &stages);
        let fut = work(self.clone(), id.clone());
        let (jobs, job) = (self.jobs.clone(), id.clone());
        let task = tokio::spawn(async move {
            jobs.running(&job);
            let r = fut.await;
            jobs.finish(&job, r);
        });
        self.jobs.attach(&id, task.abort_handle());
        self.jobs.info(&id).ok_or_else(|| ApiError::internal("the job went"))
    }

    /// One stage: working (held: waiting on the person), a step, done.
    async fn stage(&self, job: &str, stage: &str, held: bool, steps: u32) {
        self.jobs.stage(job, stage, PartState::Working, None);
        if held {
            self.jobs.update(job, true, |j| j.stages.iter_mut().filter(|s| s.id == stage).for_each(|s| s.held = true));
        }
        tokio::time::sleep(self.step * steps).await;
        self.jobs.update(job, true, |j| j.stages.iter_mut().filter(|s| s.id == stage).for_each(|s| s.held = false));
        self.jobs.stage(job, stage, PartState::Done, None);
    }
}

#[async_trait]
impl Net for Sample {
    async fn tailnet(&self) -> Result<Tailnet, ApiError> {
        Ok(self.0.st.lock().unwrap().tailnet.clone())
    }

    async fn login(&self) -> Result<JobInfo, ApiError> {
        let signin = SignInProvider::Tailscale;
        let stages = [
            ("up", "Starting Tailscale".to_string()),
            ("signin", signin.held_label()),
            ("running", "Joining the tailnet".to_string()),
        ];
        self.0.job(JobKind::Tailnet, "Logging in to Tailscale", TAILNET, &stages, |me, job| async move {
            me.set_state(TailnetState::Starting, None);
            me.stage(&job, "up", false, 1).await;
            let url = "https://login.tailscale.com/a/sample";
            me.set_state(TailnetState::LoggingIn, Some(url));
            {
                let _window = me.bus.sign_in(signin, url, None);
                me.stage(&job, "signin", true, 3).await;
            }
            me.stage(&job, "running", false, 1).await;
            let now = now_ms();
            {
                let mut st = me.st.lock().unwrap();
                st.tailnet = running(&me.hostname, now);
                st.peers = peers(now);
            }
            me.tell_tailnet();
            me.tell_peers();
            Ok(())
        })
    }

    async fn logout(&self) -> Result<Tailnet, ApiError> {
        let me = &self.0;
        let t = me.tailnet_with(|t| {
            *t = Tailnet { state: TailnetState::NeedsLogin, hostname: t.hostname.clone(), ..Tailnet::default() };
            Ok(())
        })?;
        me.st.lock().unwrap().peers.clear();
        me.tell_peers();
        Ok(t)
    }

    async fn trust(&self, device: &str) -> Result<Tailnet, ApiError> {
        self.0.tailnet_with(|t| {
            if let Some(i) = t.asks.iter().position(|a| a.id == device) {
                let a = t.asks.remove(i);
                t.devices.retain(|d| d.id != device);
                t.devices.push(TailnetDevice {
                    id: a.id,
                    name: a.device,
                    os: a.os,
                    kind: a.kind,
                    online: true,
                    trusted: true,
                    mine: false,
                });
                return Ok(());
            }
            let d = t.devices.iter_mut().find(|d| d.id == device).ok_or_else(|| not_found("device", device))?;
            d.trusted = true;
            Ok(())
        })
    }

    async fn untrust(&self, device: &str) -> Result<Tailnet, ApiError> {
        self.0.tailnet_with(|t| {
            let d = t.devices.iter_mut().find(|d| d.id == device).ok_or_else(|| not_found("device", device))?;
            if d.mine {
                return Err(ApiError::conflict(format!("{} is one of your machines: it's always trusted", d.name)));
            }
            d.trusted = false;
            Ok(())
        })
    }

    async fn deny(&self, ask: &str) -> Result<Tailnet, ApiError> {
        self.0.tailnet_with(|t| {
            let i = t.asks.iter().position(|a| a.id == ask).ok_or_else(|| not_found("ask", ask))?;
            let a = t.asks.remove(i);
            t.devices.retain(|d| d.id != ask);
            t.devices.push(TailnetDevice {
                id: a.id,
                name: a.device,
                os: a.os,
                kind: a.kind,
                online: true,
                trusted: false,
                mine: false,
            });
            Ok(())
        })
    }

    async fn later(&self, ask: &str) -> Result<Tailnet, ApiError> {
        self.0.tailnet_with(|t| {
            let i = t.asks.iter().position(|a| a.id == ask).ok_or_else(|| not_found("ask", ask))?;
            t.asks.remove(i);
            Ok(())
        })
    }

    async fn peers(&self) -> Result<Vec<PeerInfo>, ApiError> {
        Ok(self.0.st.lock().unwrap().peers.clone())
    }

    async fn peer_wadspaces(&self, peer: &str) -> Result<Vec<Wadspace>, ApiError> {
        let p = self.0.st.lock().unwrap().peers.iter().find(|p| p.peer.id == peer).cloned();
        let p = p.ok_or_else(|| not_found("machine", peer))?;
        if !p.peer.online {
            return Err(ApiError::new(ErrorCode::Offline, format!("{} is offline", p.peer.name)));
        }
        Ok(peer_wadspaces(peer, now_ms()))
    }

    async fn copy_from_peer(&self, peer: &str, ws: &str) -> Result<JobInfo, ApiError> {
        let from = self.0.st.lock().unwrap().peers.iter().find(|p| p.peer.id == peer).map(|p| p.peer.name.clone());
        let doc = self
            .peer_wadspaces(peer)
            .await?
            .into_iter()
            .find(|w| w.id == ws)
            .ok_or_else(|| not_found("wadspace", ws))?;
        let from = from.unwrap_or_default();
        let stages = [
            ("fetch", format!("Getting {} from {from}", doc.name)),
            ("import", "Making it this machine's".to_string()),
        ];
        let title = format!("Copying {} from {from}", doc.name);
        let peer = peer.to_string();
        let info = self.0.job(JobKind::CopyWadspace, &title, ws, &stages, |me, job| async move {
            me.stage(&job, "fetch", false, 2).await;
            me.jobs.stage(&job, "import", PartState::Working, None);
            let out = me.docs.apply(Op::Import { doc: Box::new(doc), from: peer }).map_err(|e| e.message)?;
            me.builds.edited(&out.changed);
            if let Some(id) = out.created {
                me.jobs.update(&job, true, |j| {
                    j.params.insert("created".into(), Value::String(id));
                });
            }
            Ok(())
        })?;
        Ok(info)
    }
}

#[async_trait]
impl Accounts for Sample {
    async fn github(&self) -> Result<Option<GithubAccount>, ApiError> {
        Ok(self.0.st.lock().unwrap().github.clone())
    }

    async fn github_signin(&self, req: SignInRequest) -> Result<JobInfo, ApiError> {
        let signin = SignInProvider::Github;
        let stages = [
            ("code", "Asking GitHub for a code".to_string()),
            ("signin", signin.held_label()),
            ("token", "Keeping the token".to_string()),
        ];
        self.0.job(JobKind::Github, "Signing in to GitHub", "github", &stages, |me, job| async move {
            me.stage(&job, "code", false, 1).await;
            {
                let _window = me.bus.sign_in(signin, "https://github.com/login/device", Some("WADS-0601"));
                me.stage(&job, "signin", true, 3).await;
            }
            me.stage(&job, "token", false, 1).await;
            {
                let mut st = me.st.lock().unwrap();
                let mut account = st.github.take().unwrap_or_else(hunggod);
                for s in FIRST_SCOPES.iter().map(|s| s.to_string()).chain(req.scopes) {
                    if !account.scopes.contains(&s) {
                        account.scopes.push(s);
                    }
                }
                st.github = Some(account);
            }
            me.tell_github();
            Ok(())
        })
    }

    async fn github_signout(&self) -> Result<(), ApiError> {
        self.0.st.lock().unwrap().github = None;
        self.0.tell_github();
        Ok(())
    }

    async fn github_repos(&self) -> Result<Vec<Repo>, ApiError> {
        if self.0.st.lock().unwrap().github.is_none() {
            return Err(ApiError::new(ErrorCode::Unauthorized, "Sign in to GitHub first"));
        }
        Ok(repos(now_ms()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peers_sample_wadspaces_check_out() {
        for w in peer_wadspaces("m-desk", 1_000_000_000) {
            wad_model::check::wadspace(&w).unwrap_or_else(|e| panic!("{}: {e}", w.id));
        }
    }
}
