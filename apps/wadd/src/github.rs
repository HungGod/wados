//! GitHub on this machine ([`Accounts`] for api_phase6's routes): wadd's
//! sign-in, its token, whose it is, and the repositories it can use.
//!
//! - **The token** is the podman secret `github_token`. Runs give it to the
//!   wadspaces (git's credential helper reads it there) and packages sign in
//!   to ghcr.io with it; it never leaves wadd and podman otherwise, and is
//!   never in a log, a job or an event.
//! - **Signing in** (`POST /v1/github/signin`) is a Github job, the OAuth
//!   device flow (wad_github::device) in three stages:
//!   1. "Opening GitHub's sign-in in the browser": GitHub gives a device
//!      code for the app's scopes (and the asked ones, and the ones the
//!      token has already), and the job's params get its `url` and `code`;
//!   2. "Signing in to WadSpaces in the browser" (held): `signIn` with the
//!      device page and the code (the shell opens it in WadBrowser) while
//!      wadd polls for the token; `signInDone` when that's over, however
//!      it ends (cancelling the job stops the polling and closes it);
//!   3. "Saving the sign-in": the token replaces the secret, GitHub says
//!      whose it is (`GET /user` and its `X-OAuth-Scopes`), and the
//!      `github` event tells everyone.
//!
//!   One at a time: the same ask while one runs is that job (its window
//!   asked for again); another ask replaces it.
//! - **Whose** (`GET /v1/github`): asked of GitHub once per token every
//!   [`WHO_FOR`], and kept in `<state>/github.json` (the token's hash and
//!   the account, never the token), so it's known offline and after a
//!   restart. A token GitHub rejects is no account (null).
//! - **Repos** (`GET /v1/github/repos`): every page, kept for [`REPOS_FOR`]
//!   under a hash of the token, so a new token sees fresh ones (v1's).
//! - **Signing out** (`DELETE /v1/github`) deletes the secret. The token
//!   stays valid on GitHub until it's revoked there (Settings ▸
//!   Applications): revoking needs the app's client secret, which WadOS
//!   doesn't have.
//!
//! The OAuth app (its client ID and the first sign-in's scopes) is
//! `/usr/lib/wados/github.toml`; wad-config's `[github]` says where it is,
//! and GitHub's addresses (tests point them at a mock).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::task::JoinHandle;
use wad_github::{AppConfig, Clock, DeviceCode, Github, RealClock, Token};
use wad_model::{JobKind, PartState};
use wad_proto::{ApiError, ErrorCode, Event, GithubAccount, JobInfo, Repo, SignInProvider, SignInRequest};

use crate::api_phase6::Accounts;
use crate::backend::Backend;
use crate::events::Bus;
use crate::jobs::{Jobs, finished};
use crate::packages::GITHUB_SECRET;

/// The repo list is asked for again after this.
pub const REPOS_FOR: Duration = Duration::from_secs(60);
/// Whose the token is, asked again after this.
pub const WHO_FOR: Duration = Duration::from_secs(300);
/// Each call to GitHub, at most.
const CALL_TIMEOUT: Duration = Duration::from_secs(20);
const WHO_FILE: &str = "github.json";
const SIGNED_OUT: &str = "Sign in to WadSpaces first";

/// The job's stages: (id, label).
pub const STAGES: [(&str, &str); 3] =
    [("open", "Opening GitHub's sign-in in the browser"), ("signin", ""), ("save", "Saving the sign-in")];

/// Where the app and GitHub are, and wadd's state.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The OAuth app's file (`/usr/lib/wados/github.toml`).
    pub app: PathBuf,
    pub web: String,
    pub api: String,
    pub state_dir: PathBuf,
}

impl Settings {
    pub fn new(config: &wad_config::Config) -> Self {
        let g = &config.github;
        Self { app: g.app.clone(), web: g.web.clone(), api: g.api.clone(), state_dir: config.daemon.state_dir.clone() }
    }
}

/// A token's hash: what the caches and github.json know it by.
fn key(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Whose a token is, as GitHub last said (github.json, without `at`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Who {
    /// The token's hash.
    token: String,
    account: GithubAccount,
    /// When GitHub was last asked (none: read from the file).
    #[serde(skip)]
    at: Option<Instant>,
}

/// The sign-in going on.
struct SigningIn {
    job: String,
    scopes: Vec<String>,
    /// Its window (url, code), once GitHub has given the code.
    window: Option<(String, String)>,
    task: Option<JoinHandle<()>>,
}

/// GitHub as the API has it. Cheap to clone.
#[derive(Clone)]
pub struct GithubService(Arc<Inner>);

struct Inner {
    gh: Github,
    settings: Settings,
    backend: Arc<dyn Backend>,
    jobs: Arc<Jobs>,
    bus: Bus,
    clock: Arc<dyn Clock>,
    who: Mutex<Option<Who>>,
    /// (the token's hash, when, its repos)
    repos: Mutex<Option<(String, Instant, Vec<Repo>)>>,
    signing_in: Mutex<Option<SigningIn>>,
}

impl GithubService {
    /// The machine's GitHub; its present (whose the token is, or none) is
    /// told to the watchers once it's known.
    pub fn new(backend: Arc<dyn Backend>, jobs: Arc<Jobs>, bus: Bus, settings: Settings) -> Self {
        Self::with_clock(backend, jobs, bus, settings, Arc::new(RealClock::default()))
    }

    /// With the device flow's waits on `clock`.
    pub fn with_clock(
        backend: Arc<dyn Backend>,
        jobs: Arc<Jobs>,
        bus: Bus,
        settings: Settings,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(CALL_TIMEOUT)
            .connect_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        let gh = Github::with_bases(http, &settings.web, &settings.api);
        let me = Arc::new(Inner {
            gh,
            settings,
            backend,
            jobs,
            bus,
            clock,
            who: Mutex::default(),
            repos: Mutex::default(),
            signing_in: Mutex::default(),
        });
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            let m = me.clone();
            rt.spawn(async move {
                if let Err(e) = m.account().await {
                    tracing::info!("whose GitHub token this is isn't known yet: {}", e.message);
                }
            });
        }
        Self(me)
    }
}

impl Inner {
    /// The token, if there's one.
    async fn token(&self) -> Result<Option<String>, ApiError> {
        match self.backend.secret_value(GITHUB_SECRET).await {
            Ok(t) => Ok(t.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())),
            Err(e) => {
                Err(ApiError::new(ErrorCode::Upstream, format!("Podman didn't say whether there's a token: {e}")))
            }
        }
    }

    fn who_file(&self) -> PathBuf {
        self.settings.state_dir.join(WHO_FILE)
    }

    /// What's known of token `k`'s account: in memory, else github.json's.
    fn known(&self, k: &str) -> Option<Who> {
        let mut who = self.who.lock().unwrap();
        if let Some(w) = who.as_ref().filter(|w| w.token == k) {
            return Some(w.clone());
        }
        let text = std::fs::read(self.who_file()).ok()?;
        let w: Who = serde_json::from_slice(&text).ok().filter(|w: &Who| w.token == k)?;
        *who = Some(w.clone());
        Some(w)
    }

    /// GitHub said token `k` is `account`'s: kept, here and in github.json.
    fn remember(&self, k: &str, account: &GithubAccount) {
        let w = Who { token: k.to_string(), account: account.clone(), at: Some(Instant::now()) };
        match serde_json::to_vec_pretty(&w) {
            Ok(bytes) => {
                if let Err(e) = crate::store::write_atomic_mkdir(&self.who_file(), &bytes) {
                    tracing::warn!("{}: {e}", self.who_file().display());
                }
            }
            Err(e) => tracing::warn!("{WHO_FILE}: {e}"),
        }
        *self.who.lock().unwrap() = Some(w);
    }

    /// No account any more: nothing kept of the old one.
    fn forget(&self) {
        *self.who.lock().unwrap() = None;
        *self.repos.lock().unwrap() = None;
        match std::fs::remove_file(self.who_file()) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("{}: {e}", self.who_file().display()),
        }
    }

    /// The `github` event, if it isn't what watchers have already.
    fn tell(&self, account: Option<GithubAccount>) {
        let now = Event::Github { account };
        if self.bus.latest("github").as_ref() != Some(&now) {
            self.bus.publish(now);
        }
    }

    /// Whose the token is: known (asked within WHO_FOR), else GitHub's
    /// answer; what was known when GitHub can't be reached.
    async fn account(&self) -> Result<Option<GithubAccount>, ApiError> {
        let Some(token) = self.token().await? else {
            if self.who.lock().unwrap().is_some() || self.who_file().exists() {
                self.forget();
            }
            self.tell(None);
            return Ok(None);
        };
        let k = key(&token);
        let known = self.known(&k);
        if let Some(w) = &known
            && w.at.is_some_and(|at| at.elapsed() < WHO_FOR)
        {
            return Ok(Some(w.account.clone()));
        }
        match self.gh.user(&Token::new(token)).await {
            Ok(a) => {
                self.remember(&k, &a);
                self.tell(Some(a.clone()));
                Ok(Some(a))
            }
            Err(wad_github::Error::BadToken) => {
                tracing::info!("GitHub rejected the machine's token: signed out");
                self.forget();
                self.tell(None);
                Ok(None)
            }
            Err(e) => match known {
                Some(mut w) => {
                    tracing::info!("GitHub didn't say whose the token is ({e}): {} as last known", w.account.login);
                    // Not asked again for a while.
                    w.at = Some(Instant::now());
                    *self.who.lock().unwrap() = Some(w.clone());
                    self.tell(Some(w.account.clone()));
                    Ok(Some(w.account))
                }
                None => Err(e.into()),
            },
        }
    }

    /// One stage held (waiting on the person) or not.
    fn hold(&self, job: &str, stage: &str, held: bool) {
        self.jobs.update(job, true, |j| j.stages.iter_mut().filter(|s| s.id == stage).for_each(|s| s.held = held));
    }

    /// The sign-in's work (its job's): the device code, the token, the secret.
    async fn sign_in(&self, job: &str, app: &AppConfig, scopes: &[String]) -> Result<(), String> {
        let jobs = &self.jobs;
        jobs.stage(job, "open", PartState::Working, None);
        jobs.line(job, &format!("Asking GitHub for a code that may do: {}", scopes.join(", ")));
        let code = self.gh.device_code(&app.client_id, scopes).await.map_err(|e| e.to_string())?;
        jobs.line(
            job,
            &format!(
                "Your code is {}: type it at {} if it isn't filled in there (it lasts {} minutes)",
                code.user_code,
                code.verification_uri,
                code.expires_in / 60
            ),
        );
        self.opened(job, &code);
        jobs.stage(job, "open", PartState::Done, None);

        jobs.stage(job, "signin", PartState::Working, None);
        self.hold(job, "signin", true);
        let granted = {
            let _window = self.bus.sign_in(SignInProvider::Github, &code.verification_uri, Some(&code.user_code));
            self.gh.poll(&app.client_id, &code, &*self.clock).await
        };
        self.hold(job, "signin", false);
        let granted = granted.map_err(|e| e.to_string())?;
        jobs.stage(job, "signin", PartState::Done, None);

        jobs.stage(job, "save", PartState::Working, None);
        self.backend
            .create_secret(GITHUB_SECRET, granted.token.expose().as_bytes())
            .await
            .map_err(|e| format!("GitHub gave a token, but it wasn't kept: {e}"))?;
        *self.repos.lock().unwrap() = None;
        jobs.line(job, &format!("The token is kept (the podman secret {GITHUB_SECRET})"));
        // Whose it is: asked again on a bad connection (the token's kept
        // already, so giving up here would show signed out while it isn't).
        let mut tries = 0;
        let mut account = loop {
            match self.gh.user(&granted.token).await {
                Ok(a) => break a,
                Err(e) if tries < 3 => {
                    tries += 1;
                    tracing::warn!("GitHub's user, try {tries}: {e}");
                    tokio::time::sleep(Duration::from_secs(2 * tries)).await;
                }
                Err(e) => return Err(format!("The token is kept, but GitHub didn't say whose it is: {e}")),
            }
        };
        if account.scopes.is_empty() {
            account.scopes = granted.scopes;
        }
        self.remember(&key(granted.token.expose()), &account);
        self.tell(Some(account.clone()));
        jobs.line(job, &format!("Signed in to GitHub as {} ({})", account.login, account.scopes.join(", ")));
        jobs.update(job, true, |j| {
            j.params.insert("login".into(), Value::String(account.login.clone()));
        });
        Ok(())
    }

    /// GitHub gave the code: the job says where, and the same ask again shows it.
    fn opened(&self, job: &str, code: &DeviceCode) {
        self.jobs.update(job, true, |j| {
            j.params.insert("url".into(), Value::String(code.verification_uri.clone()));
            j.params.insert("code".into(), Value::String(code.user_code.clone()));
        });
        if let Some(s) = self.signing_in.lock().unwrap().as_mut().filter(|s| s.job == job) {
            s.window = Some((code.verification_uri.clone(), code.user_code.clone()));
        }
    }
}

/// `more` added to `scopes`, each once, in order.
fn add_scopes(scopes: &mut Vec<String>, more: impl IntoIterator<Item = String>) {
    for s in more {
        let s = s.trim().to_string();
        if !s.is_empty() && !scopes.contains(&s) {
            scopes.push(s);
        }
    }
}

#[async_trait]
impl Accounts for GithubService {
    async fn github(&self) -> Result<Option<GithubAccount>, ApiError> {
        self.0.account().await
    }

    async fn github_signin(&self, req: SignInRequest) -> Result<JobInfo, ApiError> {
        let me = &self.0;
        let app =
            AppConfig::load(&me.settings.app).map_err(|e| ApiError::internal(format!("WadOS's GitHub app: {e}")))?;
        let mut scopes = app.scopes.clone();
        add_scopes(&mut scopes, req.scopes);
        // What the token may do already, it keeps.
        if let Some(w) = me.who.lock().unwrap().as_ref() {
            add_scopes(&mut scopes, w.account.scopes.clone());
        }
        let mut current = me.signing_in.lock().unwrap();
        let mut old = None;
        if let Some(s) = current.as_mut()
            && me.jobs.info(&s.job).is_some_and(|j| !finished(j.job.status))
        {
            if s.scopes == scopes {
                // The same ask again: that sign-in, its window shown again.
                if let Some((url, code)) = &s.window {
                    me.bus.publish(Event::SignIn {
                        provider: SignInProvider::Github,
                        url: url.clone(),
                        code: Some(code.clone()),
                    });
                }
                return me.jobs.info(&s.job).ok_or_else(|| ApiError::internal("the job went"));
            }
            tracing::info!("a sign-in to GitHub for other scopes: {} is cancelled", s.job);
            let _ = me.jobs.cancel(&s.job);
            old = s.task.take();
        }
        let signin = SignInProvider::Github.held_label();
        let stages: Vec<(String, String)> = STAGES
            .iter()
            .map(|(id, label)| (id.to_string(), if *id == "signin" { signin.clone() } else { label.to_string() }))
            .collect();
        let id = me.jobs.create(JobKind::Github, "Signing in to WadSpaces", "github", &stages);
        me.jobs.update(&id, true, |j| {
            j.params.insert("scopes".into(), Value::from(scopes.clone()));
        });
        *current = Some(SigningIn { job: id.clone(), scopes: scopes.clone(), window: None, task: None });
        let (inner, job) = (me.clone(), id.clone());
        let task = tokio::spawn(async move {
            // The one it replaces is gone first, its window with it.
            if let Some(t) = old {
                t.abort();
                let _ = t.await;
            }
            inner.jobs.running(&job);
            let r = inner.sign_in(&job, &app, &scopes).await;
            if let Err(e) = &r {
                tracing::info!("the sign-in to GitHub didn't finish: {e}");
            }
            inner.jobs.finish(&job, r);
        });
        me.jobs.attach(&id, task.abort_handle());
        if let Some(s) = current.as_mut() {
            s.task = Some(task);
        }
        drop(current);
        me.jobs.info(&id).ok_or_else(|| ApiError::internal("the job went"))
    }

    async fn github_signout(&self) -> Result<(), ApiError> {
        let me = &self.0;
        me.backend
            .delete_secret(GITHUB_SECRET)
            .await
            .map_err(|e| ApiError::new(ErrorCode::Upstream, format!("The token wasn't deleted: {e}")))?;
        me.forget();
        tracing::info!("signed out of GitHub");
        me.bus.publish(Event::Github { account: None });
        Ok(())
    }

    async fn github_repos(&self) -> Result<Vec<Repo>, ApiError> {
        let me = &self.0;
        let token = me.token().await?.ok_or_else(|| ApiError::new(ErrorCode::Unauthorized, SIGNED_OUT))?;
        let k = key(&token);
        if let Some((_, _, repos)) =
            me.repos.lock().unwrap().as_ref().filter(|(h, at, _)| *h == k && at.elapsed() < REPOS_FOR)
        {
            return Ok(repos.clone());
        }
        let repos = me.gh.repos(&Token::new(token)).await?;
        *me.repos.lock().unwrap() = Some((k, Instant::now(), repos.clone()));
        Ok(repos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_are_each_asked_for_once() {
        let mut s = vec!["repo".to_string(), "workflow".to_string()];
        add_scopes(&mut s, ["write:packages", "repo", " ", "workflow"].map(String::from));
        assert_eq!(s, ["repo", "workflow", "write:packages"]);
        assert_eq!(key("gho_x").len(), 64);
        assert_ne!(key("gho_x"), key("gho_y"));
    }
}
