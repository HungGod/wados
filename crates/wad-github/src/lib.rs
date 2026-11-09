//! GitHub for WadOS machines: the OAuth device flow that signs wadd in
//! ([`device`]: no client secret, so the client ID is public), and the REST
//! calls wadd makes with the token it gets. A token is a [`Token`], which
//! never prints and never serializes.
//!
//! - [`Github::user`]: whose a token is, and what it may do (its
//!   `X-OAuth-Scopes`).
//! - [`Github::repos`]: the repositories the account can use, every page.
//! - [`Github::create_repo`]: a new one, with a first commit so it can be
//!   cloned straight away.
//!
//! Both of GitHub's addresses can be changed ([`Github::with_bases`]): its
//! sign-in (`https://github.com`) and its API (`https://api.github.com`).
//! Tests point them at a mock; nothing here is ever tested against GitHub.

use std::fmt;
use std::path::Path;

use serde::Deserialize;
use wad_proto::github::{FIRST_SCOPES, GithubAccount, Repo};
use wad_proto::{ApiError, ErrorCode};

pub mod device;

pub use device::{Clock, DeviceCode, Granted, Poll, RealClock};

/// GitHub's sign-in: the device flow's pages and endpoints.
pub const WEB: &str = "https://github.com";
/// GitHub's REST API.
pub const API: &str = "https://api.github.com";
const USER_AGENT: &str = concat!("WadOS/", env!("CARGO_PKG_VERSION"));

/// The OAuth app wadd signs in with (`/usr/lib/wados/github.toml`): its
/// client ID (public: the device flow has no secret), and what the first
/// sign-in asks a token to be able to do.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AppConfig {
    pub client_id: String,
    /// What the first sign-in asks for (none said: [`FIRST_SCOPES`]).
    #[serde(default = "first_scopes")]
    pub scopes: Vec<String>,
}

fn first_scopes() -> Vec<String> {
    FIRST_SCOPES.map(String::from).to_vec()
}

impl AppConfig {
    pub const PATH: &str = "/usr/lib/wados/github.toml";

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let app: Self = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if app.client_id.trim().is_empty() {
            return Err(format!("{}: no client_id", path.display()));
        }
        Ok(app)
    }
}

/// A GitHub access token. Debug is redacted; use [`Token::expose`] where
/// the value really has to go somewhere.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(…)")
    }
}

/// What went wrong, in words for the person (a job's error says this).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("GitHub rejected the token (it may have been revoked): sign in to WadSpaces again")]
    BadToken,
    #[error("Couldn't reach GitHub: {0}")]
    Network(#[source] reqwest::Error),
    /// GitHub refused what was asked (a repo by that name exists, ...).
    #[error("{0}")]
    Invalid(String),
    #[error("GitHub answered {status}: {message}")]
    Upstream { status: u16, message: String },
    // ---------------------------------------------------- the device flow's ends
    /// `access_denied`: Cancel on GitHub's page.
    #[error("The sign-in was cancelled on GitHub, so WadOS isn't signed in")]
    Denied,
    /// `expired_token`, or the code's time ran out here.
    #[error("The code expired before the sign-in was finished on GitHub: sign in again")]
    Expired,
    /// `incorrect_device_code`.
    #[error("GitHub didn't know the sign-in's code: sign in again")]
    IncorrectCode,
    /// `device_flow_disabled`: the OAuth app hasn't the device flow on.
    #[error(
        "WadOS's GitHub app doesn't have \"Enable Device Flow\" ticked, so it can't sign in. Its owner ticks it in \
         the app's settings on GitHub (Settings ▸ Developer settings ▸ OAuth Apps)"
    )]
    DeviceFlowDisabled,
    /// `incorrect_client_credentials`, or no such app: the client ID is wrong.
    #[error("GitHub doesn't know WadOS's client ID (its GitHub app's, in /usr/lib/wados/github.toml)")]
    UnknownApp,
    /// Another OAuth error.
    #[error("GitHub refused the sign-in: {description} ({error})")]
    OAuth { error: String, description: String },
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        match e.status() {
            Some(s) => Error::Upstream { status: s.as_u16(), message: e.to_string() },
            None => Error::Network(e),
        }
    }
}

impl Error {
    /// Worth trying again in a moment (the network, GitHub's own trouble).
    pub fn passing(&self) -> bool {
        match self {
            Error::Network(_) => true,
            Error::Upstream { status, .. } => *status >= 500 || *status == 429,
            _ => false,
        }
    }
}

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        let code = match e {
            Error::BadToken => ErrorCode::Unauthorized,
            Error::Invalid(_) => ErrorCode::BadRequest,
            Error::Network(_) => ErrorCode::Offline,
            Error::Denied | Error::Expired => ErrorCode::Cancelled,
            Error::IncorrectCode => ErrorCode::Conflict,
            Error::Upstream { .. } | Error::DeviceFlowDisabled | Error::UnknownApp | Error::OAuth { .. } => {
                ErrorCode::Upstream
            }
        };
        ApiError::new(code, e.to_string())
    }
}

/// GitHub's sign-in and API, over `http`.
#[derive(Debug, Clone)]
pub struct Github {
    http: reqwest::Client,
    web: String,
    api: String,
}

impl Github {
    pub fn new(http: reqwest::Client) -> Self {
        Self::with_bases(http, WEB, API)
    }

    /// Against other servers (a mock): `web` for the sign-in, `api` for the rest.
    pub fn with_bases(http: reqwest::Client, web: &str, api: &str) -> Self {
        Self { http, web: web.trim_end_matches('/').into(), api: api.trim_end_matches('/').into() }
    }

    /// The account a token belongs to, and what the token may do.
    pub async fn user(&self, token: &Token) -> Result<GithubAccount, Error> {
        #[derive(Deserialize)]
        struct User {
            login: String,
            name: Option<String>,
        }
        let res = self.api_call(self.http.get(format!("{}/user", self.api)), token).await?;
        let scopes =
            res.headers().get("x-oauth-scopes").and_then(|v| v.to_str().ok()).map(split_scopes).unwrap_or_default();
        let u: User = res.json().await.map_err(Error::Network)?;
        let name = u.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| u.login.clone());
        Ok(GithubAccount { login: u.login, name, scopes })
    }

    async fn api_call(&self, req: reqwest::RequestBuilder, token: &Token) -> Result<reqwest::Response, Error> {
        let res = req
            .header("accept", "application/vnd.github+json")
            .header("x-github-api-version", "2022-11-28")
            .header("user-agent", USER_AGENT)
            .bearer_auth(token.expose())
            .send()
            .await
            .map_err(Error::Network)?;
        let status = res.status().as_u16();
        match status {
            200..=299 => Ok(res),
            401 => Err(Error::BadToken),
            _ => {
                let body: serde_json::Value = res.json().await.unwrap_or(serde_json::Value::Null);
                let message = message_of(&body, status);
                Err(if status == 422 { Error::Invalid(message) } else { Error::Upstream { status, message } })
            }
        }
    }

    /// Every repo the account can use (every page), most recently pushed first.
    pub async fn repos(&self, token: &Token) -> Result<Vec<Repo>, Error> {
        let mut out = vec![];
        let mut url = Some(format!("{}/user/repos?{REPOS_QUERY}", self.api));
        while let Some(u) = url {
            let res = self.api_call(self.http.get(&u), token).await?;
            url = next_link(&res);
            let page: serde_json::Value = res.json().await.map_err(Error::Network)?;
            out.extend(page.as_array().into_iter().flatten().filter(|r| r.is_object()).map(repo_of));
        }
        Ok(out)
    }

    /// A new repo, private unless asked, with a first commit (auto_init).
    pub async fn create_repo(
        &self,
        token: &Token,
        name: &str,
        private: bool,
        description: &str,
    ) -> Result<Repo, Error> {
        let body = serde_json::json!({"name": name, "private": private, "description": description, "auto_init": true});
        let res = self.api_call(self.http.post(format!("{}/user/repos", self.api)).json(&body), token).await?;
        let r: serde_json::Value = res.json().await.map_err(Error::Network)?;
        Ok(repo_of(&r))
    }
}

/// "repo, workflow" (`X-OAuth-Scopes`) or "repo,workflow" (a token's `scope`).
pub fn split_scopes(s: &str) -> Vec<String> {
    s.split([',', ' ']).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

/// What the repo list asks for: what `gh repo list` shows, plus
/// collaborations and organisations, most recently pushed first.
const REPOS_QUERY: &str = "per_page=100&sort=pushed&affiliation=owner,collaborator,organization_member";

/// A repo as wad-proto has it.
fn repo_of(r: &serde_json::Value) -> Repo {
    let s = |k: &str| r.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    Repo {
        full_name: s("full_name"),
        name: s("name"),
        private: r.get("private").and_then(|v| v.as_bool()).unwrap_or(false),
        url: s("clone_url"),
        default_branch: s("default_branch"),
        pushed_at: r
            .get("pushed_at")
            .and_then(|v| v.as_str())
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.timestamp_millis()),
        description: s("description"),
    }
}

/// What GitHub said went wrong: its field errors when there are any ("name
/// already exists on this account"), else its message.
fn message_of(body: &serde_json::Value, status: u16) -> String {
    let fields: Vec<String> = body
        .get("errors")
        .and_then(|e| e.as_array())
        .into_iter()
        .flatten()
        .filter_map(|e| {
            e.get("message").and_then(|m| m.as_str()).map(String::from).or_else(|| {
                let f = |k: &str| e.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
                e.is_object().then(|| format!("{} {}", f("field"), f("code")))
            })
        })
        .collect();
    if !fields.is_empty() {
        return fields.join("; ");
    }
    body.get("message").and_then(|m| m.as_str()).map(String::from).unwrap_or_else(|| format!("HTTP {status}"))
}

/// The `rel="next"` URL of a Link header.
fn next_link(res: &reqwest::Response) -> Option<String> {
    let link = res.headers().get("link")?.to_str().ok()?;
    link.split(',').find_map(|part| {
        let (url, rel) = part.split_once(';')?;
        rel.contains("rel=\"next\"").then(|| url.trim().trim_start_matches('<').trim_end_matches('>').to_string())
    })
}

#[cfg(test)]
mod tests;
