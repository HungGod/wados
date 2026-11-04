//! Phase 7's routes on wadd's socket: streams. They answer through the
//! [`Streams`] trait, so the shell can be built against sample data
//! ([`crate::fake_phase7::Sample`]: tests, and `wadd serve --sample`) while
//! the real ones are made. Until they are, the machine's answer 501
//! ([`NotYet`]). The types are wad-proto's `streams`; the remote API's two
//! (`POST|DELETE /v1/remote/wadspaces/{id}/stream`) are in
//! [`crate::remote`], through the same trait.
//!
//! | Route | What |
//! |---|---|
//! | `GET /v1/streams` | the streams here, and which modes can start ([`StreamList`]) |
//! | `POST /v1/wadspaces/{id}/stream` | streams it ([`StreamRequest`]): a Stream job → [`JobInfo`] |
//! | `DELETE /v1/wadspaces/{id}/stream` | stops its stream → the [`StreamInfo`] as it ended |
//! | `GET /v1/streams/{id}/share` | its links and sign-in ([`StreamShare`]: its secrets) |
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
use wad_model::JobStatus;
use wad_proto::{ApiError, ErrorCode, JobInfo, StreamInfo, StreamList, StreamRequest, StreamShare};

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
}

/// The machine's, until they're made: 501, "comes in phase 7".
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
    Ok(Json(app.streams.start(&id, req).await?))
}

async fn stop(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Answer<StreamInfo> {
    Ok(Json(app.streams.stop(&id).await?))
}

async fn share(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Answer<StreamShare> {
    Ok(Json(app.streams.share(&id).await?))
}
