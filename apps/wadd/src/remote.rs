//! wadd's remote API: what other devices on the tailnet may ask of this
//! machine (port 8087 on its tailnet addresses, bound to tailscale0, so
//! nothing from another network reaches it). Its own small router, never the
//! socket's `/v1`.
//!
//! A phone (phase 13) uses the routes after `trust`: it never opens a
//! wadspace on the screen itself, and it edits, builds and streams.
//!
//! Every request is checked first ([`gate`]): tailscaled's WhoIs says which
//! device the connection is from, and tailnet.rs how this machine sees it
//! (one of my machines, trusted, asking, denied):
//!
//! | Route | Who |
//! |---|---|
//! | `GET /v1/remote/hello` → [`RemoteHello`] | anyone on the tailnet (an untrusted device asks) |
//! | `GET /v1/remote/machine` → [`RemoteMachine`] | my machines and trusted devices |
//! | `GET /v1/remote/wadspaces` → the live documents | my machines and trusted devices |
//! | `GET /v1/remote/wadspaces/{id}` → one | my machines and trusted devices |
//! | `POST /v1/remote/wadspaces/{id}/build` → `BuildAnswer` | my machines and trusted devices |
//! | `POST /v1/remote/wadspaces/{id}/open` (`OpenRequest`) → `WadspaceRun` | my machines and trusted devices |
//! | `POST /v1/remote/wadspaces/{id}/stop` → `WadspaceRun` | my machines and trusted devices |
//! | `POST /v1/remote/wadspaces/{id}/stream` (`StreamRequest`) → `StreamShare`, once it's live (streaming it already in that mode: its share) | my machines and trusted devices |
//! | `GET /v1/remote/wadspaces/{id}/stream` → its `StreamInfo` (404: none) | my machines and trusted devices |
//! | `DELETE /v1/remote/wadspaces/{id}/stream` → `StreamInfo`, ended | my machines and trusted devices |
//! | `GET /v1/remote/peers` → my machines, as `/v1/peers` | my machines and trusted devices |
//! | `GET /v1/remote/library` → the library (keys, tags, discs, Steam) | my machines and trusted devices |
//! | `POST /v1/remote/ops` (`OpRequest`) → `OpResult`; `POST /v1/remote/undo/{id}` | my machines and trusted devices |
//! | `GET /v1/remote/runs`, `/builds`, `/jobs`, `/streams` → as the socket's | my machines and trusted devices |
//! | `GET /v1/remote/streams/{id}/share` → `StreamShare` (its players' links too) | my machines and trusted devices |
//! | `GET /v1/remote/notices` → [`RemoteNotice`]s, newest first | my machines and trusted devices |
//! | `GET /v1/remote/update`, `POST /v1/remote/update/check` → as the socket's | my machines and trusted devices |
//! | `POST /v1/remote/restart` → 202: restarts into a staged update (nothing else) | my machines and trusted devices |
//! | `GET /v1/remote/trust` → [`TrustBook`] | my machines |
//! | `POST /v1/remote/trust` ([`TrustAnswer`]) → [`TrustBook`] | my machines |
//!
//! Anything else past the gate is 403 to a device that isn't trusted (it
//! asks, unless it was denied), and 404 when there's no such route. Building,
//! opening, stopping and streaming go through the same code as the socket's
//! (streams: [`crate::api_phase7::Streams`]); a stream's share (its key and
//! password) goes only to the callers that get this far. A body is 64 KB at most.

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::extract::connect_info::Connected;
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use tokio::net::{TcpListener, TcpSocket};
use wad_model::edit::Library;
use wad_model::{PeerKind, Wadspace};
use wad_proto::{
    ApiError, BuildAnswer, ErrorCode, Event, JobInfo, Level, MachineInfo, OpRequest, OpResult, OpenRequest, PeerInfo,
    RemoteHello, RemoteMachine, RemoteNotice, RemoteTrust, StreamInfo, StreamList, StreamRequest, StreamShare,
    TrustAnswer, TrustBook, Updates, WadspaceBuild, WadspaceRun,
};

use crate::api::{AppState, Failure};
use crate::peers::{Need, TailnetPeers};
use crate::tailnet::Caller;

/// A request body's most.
pub const BODY: usize = 64 * 1024;
pub const HELLO: &str = "/v1/remote/hello";
const TRUST: &str = "/v1/remote/trust";

/// What the remote API serves from.
pub struct Remote {
    pub app: Arc<AppState>,
    pub net: Arc<TailnetPeers>,
}

/// A connection to the remote API: where it's from.
#[derive(Debug, Clone, Copy)]
pub struct Conn {
    pub peer: SocketAddr,
}

impl Connected<axum::serve::IncomingStream<'_, TcpListener>> for Conn {
    fn connect_info(s: axum::serve::IncomingStream<'_, TcpListener>) -> Self {
        Conn { peer: *s.remote_addr() }
    }
}

fn forbidden(message: impl Into<String>) -> Response {
    Failure(ApiError::new(ErrorCode::Forbidden, message)).into_response()
}

/// Who's asking, before anything else: refused outright, hello only (a
/// device that isn't trusted, which asks), or as far as it's trusted. The
/// handlers get it as `Extension<Caller>`.
pub async fn gate(
    State(r): State<Arc<Remote>>,
    ConnectInfo(conn): ConnectInfo<Conn>,
    mut req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    let hello = path == HELLO;
    let need = if hello {
        Need::Hello
    } else if path.starts_with(TRUST) {
        Need::Machine
    } else {
        Need::Trusted
    };
    let caller = match r.net.identify(conn.peer, need).await {
        Ok(c) => c,
        Err(refusal) => {
            tracing::warn!(peer = %conn.peer, %path, "remote: refused ({refusal:?})");
            return forbidden(refusal.message());
        }
    };
    let machine = &r.app.machine.name;
    let allowed = hello
        || match caller.trust {
            RemoteTrust::Machine => true,
            RemoteTrust::Trusted => !path.starts_with(TRUST),
            RemoteTrust::Asked | RemoteTrust::Denied => false,
        };
    if !allowed {
        tracing::info!(device = %caller.name, %path, trust = ?caller.trust, "remote: not allowed");
        return forbidden(match caller.trust {
            RemoteTrust::Asked => format!("Trust {} on {machine} first: it's asking there", caller.name),
            RemoteTrust::Denied => format!("{machine} doesn't let {} in", caller.name),
            _ => format!("Only your machines may do that on {machine}"),
        });
    }
    tracing::debug!(device = %caller.name, %path, "remote");
    req.extensions_mut().insert(caller);
    next.run(req).await
}

pub fn router(r: Arc<Remote>) -> Router {
    Router::new()
        .route(HELLO, get(hello))
        .route("/v1/remote/machine", get(machine))
        .route("/v1/remote/wadspaces", get(wadspaces))
        .route("/v1/remote/wadspaces/{id}", get(wadspace))
        .route("/v1/remote/wadspaces/{id}/build", post(build))
        .route("/v1/remote/wadspaces/{id}/open", post(open))
        .route("/v1/remote/wadspaces/{id}/stop", post(stop))
        .route("/v1/remote/wadspaces/{id}/stream", get(stream_info).post(stream).delete(stream_stop))
        .route("/v1/remote/peers", get(peers))
        .route("/v1/remote/library", get(library))
        .route("/v1/remote/ops", post(ops))
        .route("/v1/remote/undo/{id}", post(undo))
        .route("/v1/remote/runs", get(runs))
        .route("/v1/remote/builds", get(builds))
        .route("/v1/remote/jobs", get(jobs))
        .route("/v1/remote/streams", get(streams))
        .route("/v1/remote/streams/{id}/share", get(share))
        .route("/v1/remote/notices", get(notices))
        .route("/v1/remote/update", get(update))
        .route("/v1/remote/update/check", post(update_check))
        .route("/v1/remote/restart", post(restart))
        .route(TRUST, get(trust_book).post(trust_answer))
        .fallback(|| async { Failure(ApiError::not_found("no such endpoint")) })
        .layer(axum::extract::DefaultBodyLimit::max(BODY))
        .layer(middleware::from_fn_with_state(r.clone(), gate))
        .with_state(r)
}

type Answer<T> = Result<Json<T>, Failure>;

/// This is a wadd, which machine, and how it sees the caller.
async fn hello(State(r): State<Arc<Remote>>, Extension(c): Extension<Caller>) -> Json<RemoteHello> {
    Json(RemoteHello { wadd: true, machine: r.app.machine.clone(), trust: c.trust })
}

async fn machine(State(r): State<Arc<Remote>>) -> Json<RemoteMachine> {
    Json(this_machine(&r.app.machine))
}

async fn wadspaces(State(r): State<Arc<Remote>>) -> Json<Vec<Wadspace>> {
    Json(r.app.docs.live())
}

/// One of this machine's wadspaces, not in the Trash.
fn live(r: &Remote, id: &str) -> Result<Wadspace, Failure> {
    r.app
        .docs
        .live()
        .into_iter()
        .find(|w| w.id == id)
        .ok_or_else(|| Failure(ApiError::not_found(format!("There's no wadspace {id:?} on {}", r.app.machine.name))))
}

async fn wadspace(State(r): State<Arc<Remote>>, Path(id): Path<String>) -> Answer<Wadspace> {
    Ok(Json(live(&r, &id)?))
}

/// Something on the machine's screen to say what a device did.
fn notice(r: &Remote, text: String) {
    tracing::info!("remote: {text}");
    r.app.bus.publish(Event::notice(Level::Info, text, None));
}

async fn build(
    State(r): State<Arc<Remote>>,
    Extension(c): Extension<Caller>,
    Path(id): Path<String>,
) -> Answer<BuildAnswer> {
    let w = live(&r, &id)?;
    let answer = r.app.builds.request(&id).await?;
    notice(&r, format!("{} is building {}", c.name, w.name));
    Ok(Json(answer))
}

async fn open(
    State(r): State<Arc<Remote>>,
    Extension(c): Extension<Caller>,
    Path(id): Path<String>,
    Json(req): Json<OpenRequest>,
) -> Answer<WadspaceRun> {
    let w = live(&r, &id)?;
    if req.file.is_some() {
        return Err(Failure(ApiError::bad_request(
            "Open with is the machine's own: its files open in its wadspaces there",
        )));
    }
    let run = crate::api_phase7::open(&r.app, &id, req.display).await?;
    notice(&r, format!("{} opened {}", c.name, w.name));
    Ok(Json(run))
}

async fn stop(
    State(r): State<Arc<Remote>>,
    Extension(c): Extension<Caller>,
    Path(id): Path<String>,
) -> Answer<WadspaceRun> {
    let w = live(&r, &id)?;
    r.app.focus.check_stop(&id)?;
    let run = r.app.runs.stop(&id).await?;
    notice(&r, format!("{} stopped {}", c.name, w.name));
    Ok(Json(run))
}

/// This machine's stream of `id` (not one it watches from another), if it has one.
async fn stream_of(r: &Remote, id: &str) -> Result<Option<StreamInfo>, Failure> {
    let list = r.app.streams.streams().await?;
    Ok(list.streams.into_iter().find(|s| s.id == id && s.peer.is_none() && s.stage != wad_model::StreamStage::Ended))
}

/// Streams a wadspace for the device that asked: its share (one-tap link,
/// link, a public one's sign-in) once it's live. Streaming it already in
/// that mode (another device asked, or this one again): that stream's.
async fn stream(
    State(r): State<Arc<Remote>>,
    Extension(c): Extension<Caller>,
    Path(id): Path<String>,
    Json(req): Json<StreamRequest>,
) -> Answer<StreamShare> {
    let w = live(&r, &id)?;
    r.app.focus.check_stream(&id)?;
    let job = match stream_of(&r, &id).await? {
        Some(s) if s.mode == req.mode => s.job,
        _ => Some(r.app.streams.start(&id, req).await?.job.id),
    };
    if let Some(job) = job.filter(|j| r.app.jobs.info(j).is_some()) {
        crate::api_phase7::ended(&r.app, &job).await?;
    }
    let share = r.app.streams.share(&id).await?;
    let how = if req.mode.is_public() { "publicly" } else { "privately, on the tailnet" };
    notice(&r, format!("{} is streaming {} {how}", c.name, w.name));
    Ok(Json(share))
}

async fn stream_info(State(r): State<Arc<Remote>>, Path(id): Path<String>) -> Answer<StreamInfo> {
    live(&r, &id)?;
    let s = stream_of(&r, &id).await?;
    Ok(Json(s.ok_or_else(|| Failure(ApiError::not_found(format!("{id:?} isn't streaming"))))?))
}

async fn stream_stop(
    State(r): State<Arc<Remote>>,
    Extension(c): Extension<Caller>,
    Path(id): Path<String>,
) -> Answer<StreamInfo> {
    let w = live(&r, &id)?;
    let ended = r.app.streams.stop(&id).await?;
    notice(&r, format!("{} stopped streaming {}", c.name, w.name));
    Ok(Json(ended))
}

// ------------------------------------------------------------ for a phone

async fn peers(State(r): State<Arc<Remote>>) -> Answer<Vec<PeerInfo>> {
    Ok(Json(r.app.net.peers().await?))
}

async fn library(State(r): State<Arc<Remote>>) -> Json<Library> {
    Json(r.app.docs.snapshot())
}

/// An edit from the phone's Builder, as the machine's own Builder's.
async fn ops(State(r): State<Arc<Remote>>, Json(req): Json<OpRequest>) -> Answer<OpResult> {
    Ok(Json(crate::api::apply_op(&r.app, req.op).await?))
}

async fn undo(State(r): State<Arc<Remote>>, Path(id): Path<String>) -> Answer<OpResult> {
    Ok(Json(crate::api::apply_undo(&r.app, id).await?))
}

async fn runs(State(r): State<Arc<Remote>>) -> Json<Vec<WadspaceRun>> {
    Json(r.app.runs.states())
}

async fn builds(State(r): State<Arc<Remote>>) -> Json<Vec<WadspaceBuild>> {
    Json(r.app.builds.states())
}

async fn jobs(State(r): State<Arc<Remote>>) -> Json<Vec<JobInfo>> {
    Json(r.app.jobs.list())
}

async fn streams(State(r): State<Arc<Remote>>) -> Answer<StreamList> {
    Ok(Json(r.app.streams.streams().await?))
}

async fn share(State(r): State<Arc<Remote>>, Path(id): Path<String>) -> Answer<StreamShare> {
    live(&r, &id)?;
    Ok(Json(r.app.streams.share(&id).await?))
}

async fn notices(State(r): State<Arc<Remote>>) -> Json<Vec<RemoteNotice>> {
    Json(r.app.bus.recent_notices())
}

async fn update(State(r): State<Arc<Remote>>) -> Answer<Updates> {
    Ok(Json(r.app.updates.status().await?))
}

async fn update_check(State(r): State<Arc<Remote>>) -> Answer<JobInfo> {
    Ok(Json(r.app.updates.check().await?))
}

/// Restarts into a staged update, and only that: on a machine's own wadd
/// (never a laptop's), a moment after it answers.
async fn restart(State(r): State<Arc<Remote>>, Extension(c): Extension<Caller>) -> Result<StatusCode, Failure> {
    let staged = r.app.updates.status().await?.staged;
    let Some(v) = staged else {
        return Err(Failure(ApiError::conflict("There's no update to restart into")));
    };
    if r.app.profile != wad_config::Profile::System {
        return Err(Failure(ApiError::conflict("Only a machine restarts to update")));
    }
    notice(&r, format!("{} is restarting {} into WadSpaces {}", c.name, r.app.machine.name, v.version));
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let _ = tokio::process::Command::new("systemctl").arg("reboot").status().await;
    });
    Ok(StatusCode::ACCEPTED)
}

async fn trust_book(State(r): State<Arc<Remote>>) -> Json<TrustBook> {
    Json(r.net.tailnet.book())
}

/// One of my machines' answers about a device: kept if it's the latest.
async fn trust_answer(
    State(r): State<Arc<Remote>>,
    Extension(c): Extension<Caller>,
    Json(a): Json<TrustAnswer>,
) -> Answer<TrustBook> {
    if a.device.is_empty() || a.by.is_empty() {
        return Err(Failure(ApiError::bad_request("An answer needs its device and who gave it")));
    }
    if r.net.tailnet.merge(vec![a.clone()]) {
        tracing::info!("remote: {} says {} is {:?}", c.name, a.name, a.answer);
    }
    Ok(Json(r.net.tailnet.book()))
}

// ------------------------------------------------------------ this machine

/// What a peer card shows of this machine.
pub fn this_machine(m: &MachineInfo) -> RemoteMachine {
    static FACTS: OnceLock<(String, Option<String>, PeerKind)> = OnceLock::new();
    let (os, gpu, kind) = FACTS
        .get_or_init(|| {
            let os = std::fs::read_to_string("/etc/os-release").map(|t| os_name(&t)).unwrap_or_default();
            (os, gpu(std::path::Path::new("/sys/class/drm")), chassis_kind())
        })
        .clone();
    RemoteMachine { id: m.id.clone(), name: m.name.clone(), os, gpu, kind }
}

/// What the machine runs, from os-release ('WadOS 44.20261021.0 · Fedora Linux 44').
pub fn os_name(os_release: &str) -> String {
    let get = |k: &str| {
        os_release
            .lines()
            .find_map(|l| l.strip_prefix(k)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').to_string())
    };
    let base = match (get("NAME"), get("VERSION_ID")) {
        (Some(n), Some(v)) => format!("{n} {v}"),
        _ => get("PRETTY_NAME").unwrap_or_else(|| "Linux".into()),
    };
    match get("IMAGE_VERSION").or_else(|| get("OSTREE_VERSION")) {
        Some(v) => format!("WadOS {v} · {base}"),
        None => base,
    }
}

/// The machine's graphics, by its cards' PCI vendors (a discrete one first).
pub fn gpu(drm: &std::path::Path) -> Option<String> {
    let mut found = vec![];
    for e in std::fs::read_dir(drm).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let vendor = std::fs::read_to_string(e.path().join("device/vendor")).unwrap_or_default();
        let gpu = match vendor.trim() {
            "0x10de" => Some((0, "NVIDIA")),
            "0x1002" => Some((1, "AMD Radeon")),
            "0x8086" => Some((2, "Intel graphics")),
            _ => None,
        };
        found.extend(gpu);
    }
    found.sort();
    found.first().map(|(_, g)| g.to_string())
}

/// A laptop, a server or a desktop, by the firmware's chassis type.
fn chassis_kind() -> PeerKind {
    let t = std::fs::read_to_string("/sys/class/dmi/id/chassis_type").unwrap_or_default();
    match t.trim().parse::<u32>().unwrap_or(0) {
        8 | 9 | 10 | 11 | 14 | 30 | 31 | 32 => PeerKind::Laptop,
        17 | 23 | 25 | 28 | 29 => PeerKind::Server,
        _ => PeerKind::Desktop,
    }
}

// ------------------------------------------------------------ serving

/// A listener on `addr`, bound to `device` when there is one.
fn bind(addr: SocketAddr, device: &str) -> std::io::Result<TcpListener> {
    let s = if addr.is_ipv4() { TcpSocket::new_v4()? } else { TcpSocket::new_v6()? };
    s.set_reuseaddr(true)?;
    if !device.is_empty() {
        s.bind_device(Some(device.as_bytes()))?;
    }
    s.bind(addr)?;
    s.listen(128)
}

/// The listeners serving now: stopped when it's dropped.
struct Serving(Vec<tokio::task::JoinHandle<()>>);

impl Drop for Serving {
    fn drop(&mut self) {
        for s in &self.0 {
            s.abort();
        }
    }
}

/// Serves the remote API on the tailnet's addresses until it's aborted,
/// listening again whenever they change (logged in, out, a new address);
/// addresses that can't be bound yet are tried again shortly.
pub async fn serve(r: Arc<Remote>) {
    let mut addrs = r.net.tailnet.addrs();
    let device = r.net.tailnet.bind_device().to_string();
    loop {
        let want = addrs.borrow_and_update().clone();
        // Its listeners stop with it (aborted, or when the addresses change).
        let mut serving = Serving(vec![]);
        let mut missed = false;
        for a in &want {
            match bind(*a, &device) {
                Ok(l) => {
                    tracing::info!("remote API on {a}");
                    let svc = router(r.clone()).into_make_service_with_connect_info::<Conn>();
                    serving.0.push(tokio::spawn(async move {
                        if let Err(e) = axum::serve(l, svc).await {
                            tracing::warn!("remote API: {e}");
                        }
                    }));
                }
                Err(e) => {
                    tracing::warn!("remote API: {a}: {e}");
                    missed = true;
                }
            }
        }
        let retry = async {
            if missed { tokio::time::sleep(Duration::from_secs(5)).await } else { std::future::pending().await }
        };
        let gone = tokio::select! {
            c = addrs.changed() => c.is_err(),
            _ = retry => false,
        };
        drop(serving);
        if gone {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_machine_runs() {
        let wados = "NAME=\"Fedora Linux\"\nVERSION_ID=44\nPRETTY_NAME=\"Fedora Linux 44 (Forty Four)\"\nIMAGE_VERSION=\"44.20261021.0\"\n";
        assert_eq!(os_name(wados), "WadOS 44.20261021.0 · Fedora Linux 44");
        assert_eq!(os_name("NAME=Fedora Linux\nVERSION_ID=43\n"), "Fedora Linux 43");
        assert_eq!(os_name("PRETTY_NAME=\"Arch Linux\"\n"), "Arch Linux");
    }

    #[test]
    fn its_graphics_a_discrete_card_first() {
        let d = tempfile::tempdir().unwrap();
        for (card, vendor) in [("card0", "0x8086"), ("card1", "0x1002"), ("card1-eDP-1", "0x10de")] {
            std::fs::create_dir_all(d.path().join(card).join("device")).unwrap();
            std::fs::write(d.path().join(card).join("device/vendor"), format!("{vendor}\n")).unwrap();
        }
        assert_eq!(gpu(d.path()).as_deref(), Some("AMD Radeon"));
        assert_eq!(gpu(&d.path().join("none")), None);
    }
}
