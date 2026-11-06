//! Phase 7's routes on wadd's socket: streams. They answer through the
//! [`Streams`] trait: the machine's ([`crate::streams::MachineStreams`]), or
//! sample data ([`crate::fake_phase7::Sample`]: tests, and `wadd serve
//! --sample`) for building the shell against. The types are wad-proto's
//! `streams`; the remote API's (`GET|POST|DELETE
//! /v1/remote/wadspaces/{id}/stream`) are in [`crate::remote`], through the
//! same trait.
//!
//! | Route | What |
//! |---|---|
//! | `GET /v1/streams` | the streams here, and which modes can start ([`StreamList`]) |
//! | `POST /v1/wadspaces/{id}/stream` | streams it ([`StreamRequest`]): a Stream job → [`JobInfo`] |
//! | `DELETE /v1/wadspaces/{id}/stream` | stops its stream → the [`StreamInfo`] as it ended |
//! | `GET /v1/streams/{id}/share` | its links and sign-in ([`StreamShare`]: its secrets) |
//! | `POST /v1/peers/{peer}/wadspaces/{ws}/stream` | streams a wadspace of another machine of mine to here ([`StreamRequest`]) → [`StreamShare`], once it's live |
//! | `DELETE /v1/peers/{peer}/wadspaces/{ws}/stream` | stops it there → the [`StreamInfo`] as it ended |
//!
//! Another machine's stream ("Stream from {machine}": the shell opens its
//! share's one-tap link in a WadBrowser `--app` window) is that machine's
//! (its remote API's `POST /v1/remote/wadspaces/{ws}/stream`), kept in this
//! machine's list with `peer` set; it's known by (peer, wadspace), since
//! wadspaces' ids are per machine. Its peer offline: 503.
//!
//! What each answers when it can't (the sample does the same):
//! - no such wadspace here (not in the Trash), or no stream of it: 404;
//! - streaming it already (or starting): 409, stop it first;
//! - a mode that can't start here: 409, with [`Availability`]'s reason;
//! - on the screen without `takeover`: 409 (the shell's confirm, then again with it).
//!
//! [`Availability`]: wad_proto::Availability

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use wad_model::{JobStatus, SessionDisplay, StreamMode};
use wad_proto::{ApiError, ErrorCode, JobInfo, StreamInfo, StreamList, StreamRequest, StreamShare, WadspaceRun};

use crate::api::{AppState, Failure};

/// Streams (owner: the stream lifecycle agent). A stream is known by its wadspace's id.
#[async_trait]
pub trait Streams: Send + Sync + 'static {
    /// The streams here, and which modes can start.
    async fn streams(&self) -> Result<StreamList, ApiError>;
    /// A Stream job ([`wad_proto::streams::stages`]); live when it's done
    /// (the `streams` event says so).
    async fn start(&self, ws: &str, req: StreamRequest) -> Result<JobInfo, ApiError>;
    /// Its stream stopped: it, ended. It leaves the list shortly after.
    async fn stop(&self, ws: &str) -> Result<StreamInfo, ApiError>;
    /// Its one-tap link, link and (public) sign-in.
    async fn share(&self, ws: &str) -> Result<StreamShare, ApiError>;
    /// Streams `ws` of my machine `peer` to here: its share once it's live
    /// there (it's in the list meanwhile, with `peer`).
    async fn start_peer(&self, peer: &str, ws: &str, req: StreamRequest) -> Result<StreamShare, ApiError>;
    /// Stops `peer`'s stream of `ws`: it, ended.
    async fn stop_peer(&self, peer: &str, ws: &str) -> Result<StreamInfo, ApiError>;
}

/// None here: 501, "comes in phase 7" (what the machine answered before streams.rs).
pub struct NotYet;

fn not_yet<T>(what: &str) -> Result<T, ApiError> {
    Err(ApiError::not_implemented(format!("{what} comes in phase 7")))
}

#[async_trait]
impl Streams for NotYet {
    async fn streams(&self) -> Result<StreamList, ApiError> {
        not_yet("Streams")
    }
    async fn start(&self, _: &str, _: StreamRequest) -> Result<JobInfo, ApiError> {
        not_yet("Streaming a wadspace")
    }
    async fn stop(&self, _: &str) -> Result<StreamInfo, ApiError> {
        not_yet("Stopping a stream")
    }
    async fn share(&self, _: &str) -> Result<StreamShare, ApiError> {
        not_yet("Sharing a stream")
    }
    async fn start_peer(&self, _: &str, _: &str, _: StreamRequest) -> Result<StreamShare, ApiError> {
        not_yet("Streaming from another machine")
    }
    async fn stop_peer(&self, _: &str, _: &str) -> Result<StreamInfo, ApiError> {
        not_yet("Stopping another machine's stream")
    }
}

/// `POST …/open {display}` (the socket's and the remote API's): shown on
/// the screen, in a window or the background by runs.rs; `stream` goes
/// through the streams, as a private stream (`POST …/stream`, without
/// `takeover`). Its run as it is now.
pub async fn open(app: &AppState, id: &str, display: SessionDisplay) -> Result<WadspaceRun, ApiError> {
    app.focus.check_open(id, display)?;
    app.server.check_open(display)?;
    if display != SessionDisplay::Stream {
        return app.runs.open(id, display);
    }
    app.focus.check_stream(id)?;
    app.streams.start(id, StreamRequest { mode: StreamMode::Private, takeover: false }).await?;
    Ok(app.runs.state(id))
}

/// Waits for a Stream job to end: Ok when it's done (the stream is live),
/// else why not (the remote API answers its caller with the share then).
pub async fn ended(app: &AppState, job: &str) -> Result<(), ApiError> {
    let (_, mut rx) = app.bus.subscribe();
    loop {
        let info = app.jobs.info(job).ok_or_else(|| ApiError::internal("the stream's job went"))?;
        match info.job.status {
            JobStatus::Done => return Ok(()),
            JobStatus::Error => {
                let why = info.job.error.unwrap_or_else(|| "it didn't start".into());
                return Err(ApiError::conflict(why));
            }
            JobStatus::Cancelled => return Err(ApiError::new(ErrorCode::Cancelled, "Its stream was cancelled")),
            JobStatus::Queued | JobStatus::Running => {}
        }
        // Its next change, or a second (whichever comes first).
        let _ = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;
    }
}

/// The routes above, for [`crate::api::router`] (which checks the caller on each).
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/streams", get(streams))
        .route("/v1/wadspaces/{id}/stream", post(start).delete(stop))
        .route("/v1/streams/{id}/share", get(share))
        .route("/v1/peers/{id}/wadspaces/{ws}/stream", post(start_peer).delete(stop_peer))
}

type Answer<T> = Result<Json<T>, Failure>;

async fn streams(State(app): State<Arc<AppState>>) -> Answer<StreamList> {
    Ok(Json(app.streams.streams().await?))
}

async fn start(
    State(app): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<StreamRequest>,
) -> Answer<JobInfo> {
    app.focus.check_stream(&id)?;
    Ok(Json(app.streams.start(&id, req).await?))
}

async fn stop(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Answer<StreamInfo> {
    Ok(Json(app.streams.stop(&id).await?))
}

async fn share(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Answer<StreamShare> {
    Ok(Json(app.streams.share(&id).await?))
}

async fn start_peer(
    State(app): State<Arc<AppState>>,
    Path((peer, ws)): Path<(String, String)>,
    Json(req): Json<StreamRequest>,
) -> Answer<StreamShare> {
    Ok(Json(app.streams.start_peer(&peer, &ws, req).await?))
}

async fn stop_peer(State(app): State<Arc<AppState>>, Path((peer, ws)): Path<(String, String)>) -> Answer<StreamInfo> {
    Ok(Json(app.streams.stop_peer(&peer, &ws).await?))
}
