//! Dolphin's games (`GET|POST /v1/discs`): a disc file of the person's, kept
//! at `<state>/discs/<id>/<file>`, which a wadspace with Dolphin sees
//! read-only at `/config/Games`.
//!
//! A disc stays on this machine unless "Also keep it in your GitHub packages"
//! is ticked: then it's also pushed, as a one-layer image,
//! `ghcr.io/<login>/wadspaces-disc-<id>`. The push's login is a file of
//! podman's made for it and removed after, so the token isn't on a command line.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use wad_model::games::{disc_id, disc_name, disc_package_ref, is_disc_file};
use wad_model::{DiscImage, JobKind, Op, PartState};
use wad_proto::{ApiError, DiscAdd, Event, JobInfo, Level};

use crate::jobs::{Jobs, now_ms};
use crate::library::Docs;

pub const NEEDS_GITHUB: &str = "Sign in to WadSpaces first: keeping it in your packages needs your account";
/// A first push: the token from the first sign-in can't write packages.
pub const NEEDS_PACKAGES: &str = "GitHub hasn’t said this machine may push packages: sign in to WadSpaces once more";

/// An account that may be pushed as.
#[derive(Clone)]
pub struct Account {
    pub login: String,
    pub token: String,
    pub scopes: Vec<String>,
}

impl Account {
    pub fn may_push(&self) -> bool {
        self.scopes.iter().any(|s| s == "write:packages")
    }
}

/// GitHub's packages.
#[async_trait]
pub trait Registry: Send + Sync + 'static {
    /// The account, if it's signed in.
    async fn login(&self) -> Result<Option<Account>, String>;
    /// `paths` of `context` as an image, pushed as `reference`.
    async fn push(&self, context: &Path, paths: &[String], reference: &str, who: &Account) -> Result<(), String>;
    /// `reference`'s files, into `to`.
    async fn pull(&self, reference: &str, to: &Path, who: &Account) -> Result<(), String>;
}

/// The account, able to push, or why not.
pub async fn pusher(registry: &dyn Registry) -> Result<Account, ApiError> {
    let who = registry.login().await.map_err(ApiError::internal)?;
    let who = who.ok_or_else(|| ApiError::conflict(NEEDS_GITHUB))?;
    if who.may_push() { Ok(who) } else { Err(ApiError::conflict(NEEDS_PACKAGES)) }
}

pub struct MachineDiscs {
    dir: PathBuf,
    home: Box<dyn Fn() -> Option<PathBuf> + Send + Sync>,
    registry: Arc<dyn Registry>,
    docs: Arc<Docs>,
    jobs: Arc<Jobs>,
    bus: crate::events::Bus,
}

impl MachineDiscs {
    pub fn new(
        state_dir: &Path,
        home: Box<dyn Fn() -> Option<PathBuf> + Send + Sync>,
        registry: Arc<dyn Registry>,
        docs: Arc<Docs>,
        jobs: Arc<Jobs>,
        bus: crate::events::Bus,
    ) -> Arc<Self> {
        Arc::new(Self { dir: state_dir.join("discs"), home, registry, docs, jobs, bus })
    }

    /// `path`, if it's a file in the person's home: wadd is root, and this
    /// must not read whatever anyone names.
    fn in_home(&self, path: &str) -> Result<PathBuf, ApiError> {
        let home = (self.home)().ok_or_else(|| ApiError::conflict("This machine's person has no home"))?;
        let home = home.canonicalize().map_err(|e| ApiError::conflict(format!("{}: {e}", home.display())))?;
        let full =
            Path::new(path).canonicalize().map_err(|_| ApiError::not_found(format!("There's no file {path}")))?;
        if !full.starts_with(&home) || !full.is_file() {
            return Err(ApiError::bad_request("A disc comes from a file in your home"));
        }
        Ok(full)
    }

    pub async fn add(self: &Arc<Self>, req: DiscAdd) -> Result<JobInfo, ApiError> {
        let from = self.in_home(&req.path)?;
        let file = from.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        if !is_disc_file(&file) || disc_id(&file).is_empty() {
            return Err(ApiError::bad_request("A disc is an .iso or an .rvz file"));
        }
        let account = if req.push { Some(pusher(self.registry.as_ref()).await?) } else { None };
        let reference = account.as_ref().map(|a| disc_package_ref(&a.login, &disc_id(&file)));
        let mut stages = vec![("reading".to_string(), format!("Reading {file}"))];
        if let Some(a) = &account {
            stages.push(("packing".into(), "Packing it".into()));
            stages.push(("pushing".into(), format!("Pushing it to ghcr.io/{}", a.login)));
        }
        let job = self.jobs.create(JobKind::DiscStage, &format!("Adding {file}"), &disc_id(&file), &stages);
        let (me, id) = (self.clone(), job.clone());
        let task = tokio::spawn(async move {
            me.jobs.running(&id);
            let r = me.keep(&id, &from, &file, account, reference).await;
            me.jobs.finish(&id, r);
        });
        self.jobs.attach(&job, task.abort_handle());
        self.jobs.info(&job).ok_or_else(|| ApiError::internal("the job went"))
    }

    async fn keep(
        &self,
        job: &str,
        from: &Path,
        file: &str,
        account: Option<Account>,
        reference: Option<String>,
    ) -> Result<(), String> {
        let id = disc_id(file);
        self.jobs.stage(job, "reading", PartState::Working, None);
        let dir = self.dir.join(&id);
        let say = |e: std::io::Error| format!("{}: {e}", dir.display());
        std::fs::create_dir_all(&dir).map_err(say)?;
        let (to, part) = (dir.join(file), dir.join(format!("{file}.part")));
        tokio::fs::copy(from, &part).await.map_err(say)?;
        std::fs::rename(&part, &to).map_err(say)?;
        let bytes = std::fs::metadata(&to).map_err(say)?.len();
        self.jobs.stage(job, "reading", PartState::Done, None);
        if let (Some(who), Some(r)) = (&account, &reference) {
            self.jobs.stage(job, "packing", PartState::Done, None);
            self.jobs.stage(job, "pushing", PartState::Working, None);
            self.registry.push(&dir, &[file.to_string()], r, who).await?;
            self.jobs.stage(job, "pushing", PartState::Done, None);
        }
        let disc = DiscImage {
            id,
            name: disc_name(file),
            file: file.to_string(),
            mb: bytes / 1_000_000,
            reference: reference.clone().unwrap_or_default(),
            machine: self.docs.machine().to_string(),
            at: now_ms(),
        };
        let name = disc.name.clone();
        self.docs.apply(Op::KeepDisc { disc }).map_err(|e| e.message)?;
        let (title, body) = match reference {
            Some(_) => (format!("{name} is in your packages"), "It’s a game for Dolphin now, on any of your machines."),
            None => (format!("{name} is on this machine"), "It’s a game for Dolphin now."),
        };
        self.bus.publish(Event::Notice {
            level: Level::Ok,
            title,
            undo: None,
            body: Some(body.into()),
            icon: Some("disc-3".into()),
            action: None,
            toast: true,
        });
        Ok(())
    }
}

/// The machine's: the GitHub account, and podman on the command line. The
/// login is a file of podman's, 0600, in wadd's state folder and removed after:
/// never on a command line, in a build's context, or anywhere a wadspace sees.
pub struct Ghcr {
    pub accounts: Arc<dyn crate::api_phase6::Accounts>,
    pub backend: Arc<dyn crate::backend::Backend>,
    pub state_dir: PathBuf,
}

struct AuthFile(PathBuf);

impl AuthFile {
    fn new(dir: &Path, who: &Account) -> Result<Self, String> {
        use base64::Engine;
        let pair = base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", who.login, who.token));
        let path = dir.join(format!(".ghcr-auth-{}.json", std::process::id()));
        let body = format!(r#"{{"auths":{{"ghcr.io":{{"auth":"{pair}"}}}}}}"#);
        crate::streams::write_private(&path, body.as_bytes(), None).map_err(|e| e.to_string())?;
        Ok(Self(path))
    }
}

impl Drop for AuthFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn podman(args: &[String]) -> Result<String, String> {
    crate::updates::podman(args, &mut |_| {}).await.map(|o| o.trim().to_string())
}

/// `FROM scratch`, with `paths` kept where they are.
pub fn recipe(paths: &[String]) -> String {
    let mut text = String::from("FROM scratch\n");
    for p in paths {
        let quoted = serde_json::to_string(p).unwrap_or_default();
        let to = serde_json::to_string(&format!("/{p}")).unwrap_or_default();
        text.push_str(&format!("COPY [{quoted}, {to}]\n"));
    }
    text
}

#[async_trait]
impl Registry for Ghcr {
    async fn login(&self) -> Result<Option<Account>, String> {
        let Some(who) = self.accounts.github().await.map_err(|e| e.message)? else { return Ok(None) };
        let token = self.backend.secret_value("github_token").await?.map(|t| t.trim().to_string());
        Ok(token.filter(|t| !t.is_empty()).map(|token| Account { login: who.login, token, scopes: who.scopes }))
    }

    async fn push(&self, context: &Path, paths: &[String], reference: &str, who: &Account) -> Result<(), String> {
        let auth = AuthFile::new(&self.state_dir, who)?;
        let recipe_file = self.state_dir.join(format!(".ghcr-recipe-{}", std::process::id()));
        std::fs::write(&recipe_file, recipe(paths)).map_err(|e| e.to_string())?;
        let f = |s: &str| s.to_string();
        let built = podman(&[
            f("build"),
            f("-q"),
            f("-t"),
            f(reference),
            f("-f"),
            recipe_file.display().to_string(),
            context.display().to_string(),
        ])
        .await;
        let _ = std::fs::remove_file(&recipe_file);
        built?;
        podman(&[f("push"), f("--authfile"), auth.0.display().to_string(), f(reference)]).await.map(drop)
    }

    async fn pull(&self, reference: &str, to: &Path, who: &Account) -> Result<(), String> {
        let auth = AuthFile::new(&self.state_dir, who)?;
        let f = |s: &str| s.to_string();
        podman(&[f("pull"), f("-q"), f("--authfile"), auth.0.display().to_string(), f(reference)]).await?;
        let id = podman(&[f("create"), f(reference), f("/nothing")]).await?;
        let copied = podman(&[f("cp"), format!("{id}:/."), to.display().to_string()]).await;
        let _ = podman(&[f("rm"), f("-f"), id]).await;
        copied.map(drop)
    }
}
