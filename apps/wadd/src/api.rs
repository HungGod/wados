//! wadd's API, `/v1` on its Unix socket: who may call it is checked on every
//! request (access.rs), and every failure is an [`ApiError`] with the HTTP
//! status its code says.
//!
//! | Route | What |
//! |---|---|
//! | `GET /v1/health` | `ok` |
//! | `GET /v1/machine` | this machine ([`MachineInfo`]) |
//! | `GET /v1/library` | its wadspaces, tags, keys, web apps (wad_model's `Library`) |
//! | `GET /v1/wadspaces` | its wadspaces not in the Trash, in order |
//! | `GET /v1/wadspaces/{id}` | one wadspace's document |
//! | `POST /v1/ops` | one edit ([`OpRequest`]) → [`OpResult`] |
//! | `POST /v1/undo/{id}` | an edit's Undo → [`OpResult`] |
//! | `POST /v1/wadspaces/{id}/build` | its image built here (a job) → [`BuildAnswer`] |
//! | `POST /v1/wadspaces/{id}/open` | opened on the screen, in a window or in the background ([`OpenRequest`]) → [`WadspaceRun`] |
//! | `POST /v1/wadspaces/{id}/stop` | stopped → [`WadspaceRun`] |
//! | `GET /v1/builds` | each wadspace's image here ([`WadspaceBuild`]) |
//! | `GET /v1/runs` | each wadspace's run here ([`WadspaceRun`]) |
//! | `GET /v1/jobs` | the jobs, newest first ([`JobInfo`]) |
//! | `GET /v1/jobs/{id}?since=` | one, with its log from line `since` ([`JobLog`]) |
//! | `POST /v1/jobs/{id}/cancel` | stops it → [`JobInfo`] |
//! | `GET /v1/screen` | the compositor wadd serves wadspaces on ([`ScreenInfo`]) |
//! | `PUT /v1/screen/session` | the shell's compositor, as it starts ([`ScreenSession`]) → [`ScreenInfo`] |
//! | `GET /v1/events` | what changes, as server-sent events (the present first) |
//! | `GET /v1/logs?n=` | wadd's last log lines, secrets taken out |
//!
//! Phase 6's (the tailnet, peers, GitHub) are in [`crate::api_phase6`],
//! phase 7's (streams) in [`crate::api_phase7`].

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::connect_info::Connected;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::sse::{self, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use futures_util::stream::{self, Stream, StreamExt};
use tokio::net::UnixListener;
use tokio_stream::wrappers::BroadcastStream;
use wad_model::Wadspace;
use wad_model::edit::Library;
use wad_proto::{
    ApiError, BuildAnswer, ErrorCode, Event, JobInfo, JobLog, LogLine, MachineInfo, OpRequest, OpResult, OpenRequest,
    ScreenInfo, ScreenSession, WadspaceBuild, WadspaceRun,
};

use crate::access::{Peer, Policy};
use crate::api_phase6::{Accounts, Net};
use crate::api_phase7::Streams;
use crate::builds::Builds;
use crate::events::Bus;
use crate::jobs::Jobs;
use crate::library::Docs;
use crate::logbuf::LogBuffer;
use crate::runs::Runs;
use crate::screen::Screen;

pub struct AppState {
    pub machine: MachineInfo,
    pub profile: wad_config::Profile,
    pub policy: Policy,
    pub bus: Bus,
    pub docs: Arc<Docs>,
    pub logs: LogBuffer,
    pub jobs: Arc<Jobs>,
    pub builds: Arc<Builds>,
    pub runs: Arc<Runs>,
    pub screen: Arc<Screen>,
    /// The tailnet and peers ([`crate::api_phase6`]).
    pub net: Arc<dyn Net>,
    /// GitHub ([`crate::api_phase6`]).
    pub accounts: Arc<dyn Accounts>,
    /// The web keys, for the machine's own browser ([`crate::keys`]).
    pub keys: crate::keys::MachineKeys,
    /// Streams ([`crate::api_phase7`]).
    pub streams: Arc<dyn Streams>,
}

/// An ApiError as an HTTP response.
pub struct Failure(pub ApiError);

impl From<ApiError> for Failure {
    fn from(e: ApiError) -> Self {
        Self(e)
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let status = match self.0.code {
            ErrorCode::BadRequest => StatusCode::BAD_REQUEST,
            ErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
            ErrorCode::Forbidden => StatusCode::FORBIDDEN,
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
            ErrorCode::Conflict | ErrorCode::Cancelled => StatusCode::CONFLICT,
            ErrorCode::Offline => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Upstream => StatusCode::BAD_GATEWAY,
            ErrorCode::Timeout => StatusCode::GATEWAY_TIMEOUT,
            ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::NotImplemented => StatusCode::NOT_IMPLEMENTED,
        };
        (status, Json(self.0)).into_response()
    }
}

impl Connected<axum::serve::IncomingStream<'_, UnixListener>> for Peer {
    fn connect_info(stream: axum::serve::IncomingStream<'_, UnixListener>) -> Self {
        match stream.io().peer_cred() {
            Ok(c) => Peer { uid: c.uid(), gid: c.gid(), pid: c.pid() },
            Err(_) => Peer::UNKNOWN,
        }
    }
}

async fn check_peer(
    State(app): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    req: Request,
    next: Next,
) -> Response {
    if !app.policy.allows(&peer) {
        tracing::warn!(uid = peer.uid, pid = ?peer.pid, path = %req.uri().path(), "refused a caller");
        return Failure(ApiError::new(ErrorCode::Forbidden, "not allowed to use wadd")).into_response();
    }
    next.run(req).await
}

pub fn router(app: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/health", get(|| async { "ok" }))
        .route("/v1/machine", get(machine))
        .route("/v1/library", get(library))
        .route("/v1/wadspaces", get(wadspaces))
        .route("/v1/wadspaces/{id}", get(wadspace))
        .route("/v1/wadspaces/{id}/build", post(build))
        .route("/v1/wadspaces/{id}/open", post(open))
        .route("/v1/wadspaces/{id}/stop", post(stop))
        .route("/v1/builds", get(builds))
        .route("/v1/runs", get(runs))
        .route("/v1/jobs", get(jobs))
        .route("/v1/jobs/{id}", get(job))
        .route("/v1/jobs/{id}/cancel", post(cancel))
        .route("/v1/screen", get(screen))
        .route("/v1/screen/session", put(screen_session))
        .route("/v1/ops", post(ops))
        .route("/v1/undo/{id}", post(undo))
        .route("/v1/events", get(events))
        .route("/v1/logs", get(logs))
        .merge(crate::api_phase6::routes())
        .merge(crate::api_phase7::routes())
        .layer(middleware::from_fn_with_state(app.clone(), check_peer))
        .with_state(app)
}

async fn machine(State(app): State<Arc<AppState>>) -> Json<MachineInfo> {
    Json(app.machine.clone())
}

async fn library(State(app): State<Arc<AppState>>) -> Json<Library> {
    Json(app.docs.snapshot())
}

async fn wadspaces(State(app): State<Arc<AppState>>) -> Json<Vec<Wadspace>> {
    Json(app.docs.live())
}

async fn wadspace(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<Wadspace>, Failure> {
    app.docs.wadspace(&id).map(Json).ok_or_else(|| Failure(ApiError::not_found(format!("no wadspace {id:?}"))))
}

async fn ops(State(app): State<Arc<AppState>>, Json(req): Json<OpRequest>) -> Result<Json<OpResult>, Failure> {
    let app2 = app.clone();
    let out = tokio::task::spawn_blocking(move || app2.docs.apply(req.op))
        .await
        .map_err(|e| Failure(ApiError::internal(e.to_string())))??;
    edited(&app, &out.changed).await;
    Ok(Json(out))
}

async fn undo(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<OpResult>, Failure> {
    let app2 = app.clone();
    let out = tokio::task::spawn_blocking(move || app2.docs.undo(&id))
        .await
        .map_err(|e| Failure(ApiError::internal(e.to_string())))??;
    edited(&app, &out.changed).await;
    Ok(Json(out))
}

/// What an edit changed, told to what follows the wadspaces: an image out
/// of date shows as an update (nothing builds by itself); one gone for good
/// stops, and what wadd kept for it goes.
async fn edited(app: &AppState, changed: &[String]) {
    app.builds.edited(changed);
    app.runs.edited(changed).await;
}

async fn build(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<BuildAnswer>, Failure> {
    Ok(Json(app.builds.request(&id).await?))
}

async fn open(
    State(app): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<OpenRequest>,
) -> Result<Json<WadspaceRun>, Failure> {
    Ok(Json(app.runs.open(&id, req.display)?))
}

async fn stop(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<WadspaceRun>, Failure> {
    Ok(Json(app.runs.stop(&id).await?))
}

async fn builds(State(app): State<Arc<AppState>>) -> Json<Vec<WadspaceBuild>> {
    Json(app.builds.states())
}

async fn runs(State(app): State<Arc<AppState>>) -> Json<Vec<WadspaceRun>> {
    Json(app.runs.states())
}

async fn jobs(State(app): State<Arc<AppState>>) -> Json<Vec<JobInfo>> {
    Json(app.jobs.list())
}

#[derive(serde::Deserialize)]
struct SinceQuery {
    #[serde(default)]
    since: u64,
}

async fn job(
    State(app): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SinceQuery>,
) -> Result<Json<JobLog>, Failure> {
    Ok(Json(app.jobs.get(&id, q.since)?))
}

async fn cancel(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<JobInfo>, Failure> {
    let info = app.jobs.cancel(&id)?;
    app.builds.cancelled(info.job.subject.as_deref());
    Ok(Json(info))
}

async fn screen(State(app): State<Arc<AppState>>) -> Json<ScreenInfo> {
    Json(app.screen.info())
}

async fn screen_session(
    State(app): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    Json(req): Json<ScreenSession>,
) -> Result<Json<ScreenInfo>, Failure> {
    let info = app.screen.register(&req.wayland_socket, req.uid, peer.uid)?;
    // A machine's COPY steps take from that session's user's home.
    if app.profile == wad_config::Profile::System
        && let Ok(Some(u)) = nix::unistd::User::from_uid(req.uid.into())
    {
        app.builds.set_home(crate::builds::Home { dir: u.dir, owner: Some(req.uid) });
    }
    Ok(Json(info))
}

/// The present (the machine and the other latest events, then the library),
/// then each change as it happens.
async fn events(State(app): State<Arc<AppState>>) -> Sse<impl Stream<Item = Result<sse::Event, Infallible>>> {
    let (mut now, rx) = app.bus.subscribe();
    now.push(Event::Library(Box::new(app.docs.snapshot())));
    let present = stream::iter(now);
    let changes = BroadcastStream::new(rx).filter_map(|e| async move {
        match e {
            Ok(e) => Some(e),
            Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
                tracing::warn!("an events watcher fell {n} behind");
                None
            }
        }
    });
    let all = present.chain(changes).map(|e| {
        let data = serde_json::to_string(&e).unwrap_or_else(|_| "{}".into());
        Ok(sse::Event::default().data(data))
    });
    Sse::new(all).keep_alive(KeepAlive::default())
}

#[derive(serde::Deserialize)]
struct LogsQuery {
    #[serde(default = "default_lines")]
    n: usize,
}

fn default_lines() -> usize {
    200
}

async fn logs(State(app): State<Arc<AppState>>, Query(q): Query<LogsQuery>) -> Json<Vec<LogLine>> {
    Json(app.logs.tail(q.n))
}
