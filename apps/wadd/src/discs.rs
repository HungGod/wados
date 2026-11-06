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

pub const NEEDS_GITHUB: &str = "Link GitHub first: keeping it in your packages needs your account";

/// Pushing to the account's packages.
#[async_trait]
pub trait Registry: Send + Sync + 'static {
    /// The account's login and token, if it's signed in.
    async fn login(&self) -> Result<Option<(String, String)>, String>;
    async fn push(&self, file: &Path, reference: &str, login: &str, token: &str) -> Result<(), String>;
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
        let account = if req.push {
            let got = self.registry.login().await.map_err(ApiError::internal)?;
            Some(got.ok_or_else(|| ApiError::conflict(NEEDS_GITHUB))?)
        } else {
            None
        };
        let reference = account.as_ref().map(|(login, _)| disc_package_ref(login, &disc_id(&file)));
        let mut stages = vec![("reading".to_string(), format!("Reading {file}"))];
        if let Some((login, _)) = &account {
            stages.push(("packing".into(), "Packing it".into()));
            stages.push(("pushing".into(), format!("Pushing it to ghcr.io/{login}")));
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
        account: Option<(String, String)>,
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
        if let (Some((login, token)), Some(r)) = (&account, &reference) {
            self.jobs.stage(job, "packing", PartState::Done, None);
            self.jobs.stage(job, "pushing", PartState::Working, None);
            self.registry.push(&to, r, login, token).await?;
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

/// The machine's: the GitHub account, and podman on the command line.
pub struct Ghcr {
    pub accounts: Arc<dyn crate::api_phase6::Accounts>,
    pub backend: Arc<dyn crate::backend::Backend>,
}

#[async_trait]
impl Registry for Ghcr {
    async fn login(&self) -> Result<Option<(String, String)>, String> {
        let Some(who) = self.accounts.github().await.map_err(|e| e.message)? else { return Ok(None) };
        let token = self.backend.secret_value("github_token").await?;
        Ok(token.map(|t| (who.login, t.trim().to_string())).filter(|(_, t)| !t.is_empty()))
    }

    async fn push(&self, file: &Path, reference: &str, login: &str, token: &str) -> Result<(), String> {
        use base64::Engine;
        let dir = file.parent().ok_or("the disc has no folder")?;
        let name = file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let recipe = dir.join("Containerfile.push");
        std::fs::write(&recipe, format!("FROM scratch\nCOPY [\"{name}\", \"/{name}\"]\n"))
            .map_err(|e| e.to_string())?;
        // Never in the disc's folder: wadspaces see it, and it's the build's context.
        let state = dir.parent().and_then(Path::parent).ok_or("the discs have no folder")?;
        let auth = state.join(".disc-push-auth.json");
        let pair = base64::engine::general_purpose::STANDARD.encode(format!("{login}:{token}"));
        let body = format!(r#"{{"auths":{{"ghcr.io":{{"auth":"{pair}"}}}}}}"#);
        crate::streams::write_private(&auth, body.as_bytes(), None).map_err(|e| e.to_string())?;
        let podman = |args: Vec<String>| async move {
            let out = tokio::process::Command::new("podman").args(&args).output().await.map_err(|e| e.to_string())?;
            if out.status.success() {
                Ok(())
            } else {
                Err(format!("podman: {}", String::from_utf8_lossy(&out.stderr).trim()))
            }
        };
        let f = |s: &str| s.to_string();
        let built = podman(vec![
            f("build"),
            f("-q"),
            f("-t"),
            f(reference),
            f("-f"),
            recipe.display().to_string(),
            dir.display().to_string(),
        ])
        .await;
        let pushed = match built {
            Ok(()) => podman(vec![f("push"), f("--authfile"), auth.display().to_string(), f(reference)]).await,
            Err(e) => Err(e),
        };
        let _ = std::fs::remove_file(&auth);
        let _ = std::fs::remove_file(&recipe);
        pushed
    }
}
