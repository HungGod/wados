//! The installer stick and first boot (phase 9): routes in
//! [`wad_proto::install`]'s table.

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use wad_proto::{ApiError, Install, InstallRequest, JobInfo, Setup, SetupDesign};

use crate::api::{AppState, Failure};

#[async_trait]
pub trait Installer: Send + Sync + 'static {
    async fn status(&self) -> Result<Install, ApiError>;
    /// An Install job.
    async fn start(&self, req: InstallRequest) -> Result<JobInfo, ApiError>;
}

/// Anything but a stick.
pub struct NotAStick;

const NOT_A_STICK: &str = "This isn't an installer stick";

#[async_trait]
impl Installer for NotAStick {
    async fn status(&self) -> Result<Install, ApiError> {
        Err(ApiError::not_found(NOT_A_STICK))
    }
    async fn start(&self, _: InstallRequest) -> Result<JobInfo, ApiError> {
        Err(ApiError::not_found(NOT_A_STICK))
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/install", get(install).post(start))
        .route("/v1/setup", get(setup).put(put_setup))
        .route("/v1/setup/design", post(design))
}

type Answer<T> = Result<Json<T>, Failure>;
type App = State<Arc<AppState>>;

async fn install(State(app): App) -> Answer<Install> {
    Ok(Json(app.install.status().await?))
}

async fn start(State(app): App, Json(req): Json<InstallRequest>) -> Answer<JobInfo> {
    Ok(Json(app.install.start(req).await?))
}

async fn setup(State(app): App) -> Answer<Setup> {
    Ok(Json(app.setup.get()))
}

async fn put_setup(State(app): App, Json(req): Json<Setup>) -> Answer<Setup> {
    Ok(Json(app.setup.put(req.step)?))
}

async fn design(State(app): App, Json(req): Json<SetupDesign>) -> Answer<Setup> {
    Ok(Json(app.setup.design(req).await?))
}
