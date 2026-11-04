//! Phase 6's routes on wadd's socket: the tailnet, peers and GitHub. Each
//! group answers through a trait, so the shell and the apps can be built
//! against sample data ([`crate::fake_phase6::Sample`]: tests, and `wadd
//! serve --sample`) while the real ones are made. Until they are, the
//! machine's answer 501 ([`NotYet`]). The types are wad-proto's `tailnet`,
//! `peers` and `github`; the remote API (port 8087) isn't here.
//!
//! | Route | What | Whose |
//! |---|---|---|
//! | `GET /v1/tailnet` | this machine on the tailnet ([`Tailnet`]) | tailnet & peers |
//! | `POST /v1/tailnet/login` | logs in: a Tailnet job → [`JobInfo`] | tailnet & peers |
//! | `POST /v1/tailnet/logout` | logs out → [`Tailnet`] | tailnet & peers |
//! | `PUT /v1/tailnet/devices/{id}` | trusts a device (its node id) → [`Tailnet`] | tailnet & peers |
//! | `DELETE /v1/tailnet/devices/{id}` | stops trusting it → [`Tailnet`] | tailnet & peers |
//! | `POST /v1/tailnet/asks/{id}/deny` | denies an ask → [`Tailnet`] | tailnet & peers |
//! | `POST /v1/tailnet/asks/{id}/later` | puts it off this session → [`Tailnet`] | tailnet & peers |
//! | `GET /v1/peers` | my machines ([`PeerInfo`]) | tailnet & peers |
//! | `GET /v1/peers/{id}/wadspaces` | a peer's live documents (online only) | tailnet & peers |
//! | `POST /v1/peers/{id}/wadspaces/{ws}/copy` | a CopyWadspace job (`space.import`) → [`JobInfo`] | tailnet & peers |
//! | `GET /v1/github` | the account, or null ([`GithubAccount`]) | accounts |
//! | `POST /v1/github/signin` | signs in ([`SignInRequest`], optional): a Github job → [`JobInfo`] | accounts |
//! | `DELETE /v1/github` | signs out → null | accounts |
//! | `GET /v1/github/repos` | its repositories ([`Repo`]) | accounts |
//!
//! Whose: the phase 6 agent that replaces [`NotYet`] for that group (the
//! tailnet & peers agent: [`Net`]; the accounts agent: [`Accounts`]).
//! Projects (clones, mounts, copies) have no routes of their own here.

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use wad_model::Wadspace;
use wad_proto::{ApiError, GithubAccount, JobInfo, PeerInfo, Repo, SignInRequest, Tailnet};

use crate::api::{AppState, Failure};

/// The tailnet and peers (owner: the tailnet & peers agent).
#[async_trait]
pub trait Net: Send + Sync + 'static {
    async fn tailnet(&self) -> Result<Tailnet, ApiError>;
    /// A Tailnet job: `tailscale up`, its AuthURL signed in to in the browser, Running.
    async fn login(&self) -> Result<JobInfo, ApiError>;
    async fn logout(&self) -> Result<Tailnet, ApiError>;
    /// `device`: a node stable id (a device on the tailnet, or one asking).
    async fn trust(&self, device: &str) -> Result<Tailnet, ApiError>;
    async fn untrust(&self, device: &str) -> Result<Tailnet, ApiError>;
    async fn deny(&self, ask: &str) -> Result<Tailnet, ApiError>;
    async fn later(&self, ask: &str) -> Result<Tailnet, ApiError>;
    async fn peers(&self) -> Result<Vec<PeerInfo>, ApiError>;
    /// `peer`: its wadd machine id.
    async fn peer_wadspaces(&self, peer: &str) -> Result<Vec<Wadspace>, ApiError>;
    /// A CopyWadspace job: the document fetched, and `space.import` applied here.
    async fn copy_from_peer(&self, peer: &str, ws: &str) -> Result<JobInfo, ApiError>;
}

/// GitHub (owner: the accounts agent).
#[async_trait]
pub trait Accounts: Send + Sync + 'static {
    async fn github(&self) -> Result<Option<GithubAccount>, ApiError>;
    /// A Github job: the device flow, in the browser, its token kept as the podman secret `github_token`.
    async fn github_signin(&self, req: SignInRequest) -> Result<JobInfo, ApiError>;
    async fn github_signout(&self) -> Result<(), ApiError>;
    async fn github_repos(&self) -> Result<Vec<Repo>, ApiError>;
}

/// The machine's, until they're made: 501, "comes in phase 6".
pub struct NotYet;

fn not_yet<T>(what: &str) -> Result<T, ApiError> {
    Err(ApiError::not_implemented(format!("{what} comes in phase 6")))
}

#[async_trait]
impl Net for NotYet {
    async fn tailnet(&self) -> Result<Tailnet, ApiError> {
        not_yet("The tailnet")
    }
    async fn login(&self) -> Result<JobInfo, ApiError> {
        not_yet("Logging in to the tailnet")
    }
    async fn logout(&self) -> Result<Tailnet, ApiError> {
        not_yet("Logging out of the tailnet")
    }
    async fn trust(&self, _: &str) -> Result<Tailnet, ApiError> {
        not_yet("Trusting a device")
    }
    async fn untrust(&self, _: &str) -> Result<Tailnet, ApiError> {
        not_yet("Untrusting a device")
    }
    async fn deny(&self, _: &str) -> Result<Tailnet, ApiError> {
        not_yet("Answering a device")
    }
    async fn later(&self, _: &str) -> Result<Tailnet, ApiError> {
        not_yet("Answering a device")
    }
    async fn peers(&self) -> Result<Vec<PeerInfo>, ApiError> {
        not_yet("Your other machines")
    }
    async fn peer_wadspaces(&self, _: &str) -> Result<Vec<Wadspace>, ApiError> {
        not_yet("Another machine's wadspaces")
    }
    async fn copy_from_peer(&self, _: &str, _: &str) -> Result<JobInfo, ApiError> {
        not_yet("Copying a wadspace from another machine")
    }
}

#[async_trait]
impl Accounts for NotYet {
    async fn github(&self) -> Result<Option<GithubAccount>, ApiError> {
        not_yet("GitHub")
    }
    async fn github_signin(&self, _: SignInRequest) -> Result<JobInfo, ApiError> {
        not_yet("Signing in to GitHub")
    }
    async fn github_signout(&self) -> Result<(), ApiError> {
        not_yet("Signing out of GitHub")
    }
    async fn github_repos(&self) -> Result<Vec<Repo>, ApiError> {
        not_yet("Your repositories")
    }
}

/// The routes above, for [`crate::api::router`] (which checks the caller on each).
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // ------------------------------------------------ the tailnet & peers agent's
        .route("/v1/tailnet", get(tailnet))
        .route("/v1/tailnet/login", post(login))
        .route("/v1/tailnet/logout", post(logout))
        .route("/v1/tailnet/devices/{id}", put(trust).delete(untrust))
        .route("/v1/tailnet/asks/{id}/deny", post(deny))
        .route("/v1/tailnet/asks/{id}/later", post(later))
        .route("/v1/peers", get(peers))
        .route("/v1/peers/{id}/wadspaces", get(peer_wadspaces))
        .route("/v1/peers/{id}/wadspaces/{ws}/copy", post(copy_from_peer))
        // ------------------------------------------------ the accounts agent's
        .route("/v1/github", get(github).delete(github_signout))
        .route("/v1/github/signin", post(github_signin))
        .route("/v1/github/repos", get(github_repos))
}

type Answer<T> = Result<Json<T>, Failure>;

async fn tailnet(State(app): State<Arc<AppState>>) -> Answer<Tailnet> {
    Ok(Json(app.net.tailnet().await?))
}

async fn login(State(app): State<Arc<AppState>>) -> Answer<JobInfo> {
    Ok(Json(app.net.login().await?))
}

async fn logout(State(app): State<Arc<AppState>>) -> Answer<Tailnet> {
    Ok(Json(app.net.logout().await?))
}

async fn trust(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Answer<Tailnet> {
    Ok(Json(app.net.trust(&id).await?))
}

async fn untrust(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Answer<Tailnet> {
    Ok(Json(app.net.untrust(&id).await?))
}

async fn deny(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Answer<Tailnet> {
    Ok(Json(app.net.deny(&id).await?))
}

async fn later(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Answer<Tailnet> {
    Ok(Json(app.net.later(&id).await?))
}

async fn peers(State(app): State<Arc<AppState>>) -> Answer<Vec<PeerInfo>> {
    Ok(Json(app.net.peers().await?))
}

async fn peer_wadspaces(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Answer<Vec<Wadspace>> {
    Ok(Json(app.net.peer_wadspaces(&id).await?))
}

async fn copy_from_peer(State(app): State<Arc<AppState>>, Path((id, ws)): Path<(String, String)>) -> Answer<JobInfo> {
    Ok(Json(app.net.copy_from_peer(&id, &ws).await?))
}

async fn github(State(app): State<Arc<AppState>>) -> Answer<Option<GithubAccount>> {
    Ok(Json(app.accounts.github().await?))
}

async fn github_signin(State(app): State<Arc<AppState>>, body: Option<Json<SignInRequest>>) -> Answer<JobInfo> {
    let req = body.map(|Json(b)| b).unwrap_or_default();
    Ok(Json(app.accounts.github_signin(req).await?))
}

async fn github_signout(State(app): State<Arc<AppState>>) -> Answer<Option<GithubAccount>> {
    app.accounts.github_signout().await?;
    Ok(Json(None))
}

async fn github_repos(State(app): State<Arc<AppState>>) -> Answer<Vec<Repo>> {
    Ok(Json(app.accounts.github_repos().await?))
}
