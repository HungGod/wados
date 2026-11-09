//! The account's routes on wadd's socket (wad-proto's `account`): the
//! machine's link to its owner's WadSpaces account, and the projects'
//! secret files. The remote API serves the secret files to the owner's phone
//! and machines too (remote.rs), through the same functions here.
//!
//! | Route | What |
//! |---|---|
//! | `GET /v1/account` | the link ([`AccountLink`]) |
//! | `POST /v1/account/link` | links it with the GitHub sign-in → [`AccountLink`] |
//! | `DELETE /v1/account` | unlinks it → [`AccountLink`] |
//! | `GET /v1/secret-files` | what's kept, and what's asking ([`SecretFiles`]) |
//! | `POST /v1/secret-files` | keeps a file ([`SecretFileKeep`]) → [`SecretFile`] |
//! | `DELETE /v1/secret-files/{name}` | no longer kept → 204 |
//! | `POST /v1/secret-files/asks/{id}` | answers an ask ([`SecretFileAnswer`]) → [`SecretFiles`] |

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use wad_proto::{AccountLink, ApiError, SecretFile, SecretFileAnswer, SecretFileKeep, SecretFiles};

use crate::api::{AppState, Failure};

type Answer<T> = Result<Json<T>, Failure>;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/account", get(account).delete(unlink))
        .route("/v1/account/link", post(link))
        .route("/v1/secret-files", get(secret_files).post(keep))
        .route("/v1/secret-files/{name}", axum::routing::delete(remove))
        .route("/v1/secret-files/asks/{id}", post(answer))
}

async fn account(State(app): State<Arc<AppState>>) -> Json<AccountLink> {
    Json(app.account.view())
}

async fn link(State(app): State<Arc<AppState>>) -> Answer<AccountLink> {
    Ok(Json(app.account.link().await?))
}

async fn unlink(State(app): State<Arc<AppState>>) -> Answer<AccountLink> {
    Ok(Json(app.account.unlink().await?))
}

async fn secret_files(State(app): State<Arc<AppState>>) -> Json<SecretFiles> {
    Json(app.secret_files.status())
}

async fn keep(State(app): State<Arc<AppState>>, Json(k): Json<SecretFileKeep>) -> Answer<SecretFile> {
    Ok(Json(keep_file(&app, k).await?))
}

async fn remove(State(app): State<Arc<AppState>>, Path(name): Path<String>) -> Result<StatusCode, Failure> {
    remove_file(&app, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn answer(
    State(app): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(a): Json<SecretFileAnswer>,
) -> Answer<SecretFiles> {
    Ok(Json(app.secret_files.answer(&id, a.keep).await?))
}

/// A file kept (the socket's, and the remote API's).
pub async fn keep_file(app: &AppState, k: SecretFileKeep) -> Result<SecretFile, ApiError> {
    // 64 KiB in base64, and a little.
    if k.value.len() > crate::secret_files::MAX_BYTES / 3 * 4 + 8 {
        return Err(ApiError::bad_request("A secret file can be up to 64 KB"));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(k.value.trim())
        .map_err(|_| ApiError::bad_request("The file's value isn't base64"))?;
    app.secret_files.keep(&k.repo, &k.path, &bytes).await
}

pub async fn remove_file(app: &AppState, name: &str) -> Result<(), ApiError> {
    if !crate::secret_files::is_name(name) {
        return Err(ApiError::not_found("There's no such secret file"));
    }
    app.secret_files.remove(name).await
}
