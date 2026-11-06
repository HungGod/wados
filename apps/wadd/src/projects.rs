//! A wadspace's projects (`look.projects`) made real on this machine as it
//! opens ([`Projects::prepare`], from runs.rs, each a stage of the open
//! job), and cleaned up when they're taken out ([`Projects::tidy`]) or the
//! wadspace goes for good ([`Projects::forget`]).
//!
//! - **A GitHub repository** is cloned by wadd (wad-git), as the projects
//!   user (abc, 1000) when wadd is root, into
//!   `<projects_dir>/<wadspace>/<project>`, through the machine's GitHub
//!   token (the podman secret `github_token`, by an inline credential
//!   helper: never on a command line or in a file). It's labelled for
//!   containers before git fills it, and the wadspace works in it where it
//!   is (wad-gen mounts it at `~/Desktop/<name>`); the wadspace's own
//!   `wadspaces-projects --sync` fetches it and runs its setup as it starts.
//!   Why wadd clones, not the wadspace (the image can do either): the open
//!   job shows the clone and its progress, a private repository without a
//!   sign-in fails as it opens with words to act on, the clone outlives the
//!   wadspace's home and is purged with the project, and git runs once, on
//!   the machine.
//! - **A folder (or file) of this machine** is mounted as it is, read-write.
//!   Only what's inside the person's home (or a drive of theirs under
//!   `/run/media/<them>`) can be, never the home itself or anything hidden
//!   (`~/.ssh`, `~/.config`), symlinks resolved first. `make` makes a folder
//!   right in the home that isn't there.
//! - **A file (or folder) copied in** is copied once into
//!   `<state_dir>/copies/<wadspace>/<project>` (copies.rs), which the
//!   wadspace copies into its home.
//! - **Another machine's** come over Taildrive (taildrive.rs): a copy is
//!   fetched over WebDAV; a folder is mounted on this machine (rclone) at
//!   `<state_dir>/taildrive/<wadspace>/<project>` and bound in, for as long
//!   as the wadspace runs ([`Projects::release`]). Both need it online:
//!   otherwise the wadspace opens without it, and the job says why.
//!
//! A project wadd can't make here leaves the wadspace without it (it isn't
//! mounted nor listed inside), its stage says why, and so does a notice.
//!
//! Projects are kept by wadspace and project: a project's id is its
//! wadspace's alone. wadd purges only what it made: a clone (kept aside,
//! not purged, while it has work that isn't pushed), a copy, a Taildrive
//! mount (unmounted, its folder removed only once it's empty). A folder of
//! the person's is never touched.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use wad_gen::Mounts;
use wad_model::{JobKind, PartState, Project, ProjectKind, ProjectSource, Wadspace};
use wad_proto::{PeerInfo, Tailnet, TailnetState};

use crate::api_phase6::Net;
use crate::backend::{Backend, Res};
use crate::copies::{self, Copies};
use crate::jobs::{Jobs, now_ms};
use crate::packages::GITHUB_SECRET;
use crate::selinux;
use crate::store::safe_id;
use crate::taildrive::{self, Dav, DavError, Drive, Kind, Rclone};

/// GitHub, where repositories are cloned from.
pub const GITHUB: &str = "https://github.com";
/// What a mount waits for rclone.
const MOUNT_TIMEOUT: Duration = Duration::from_secs(20);

pub struct Settings {
    pub state_dir: PathBuf,
    /// Clones: `<projects_dir>/<wadspace>/<project>`.
    pub projects_dir: PathBuf,
    /// The projects user (abc's 1000): git runs as them, and what wadd makes is theirs, when wadd is root.
    pub uid: u32,
    /// wadd is root.
    pub root: bool,
    /// A machine's wadd (the system's units and mounts), not a laptop's.
    pub system: bool,
    /// The person's home: where `~/…` is.
    pub home: Option<PathBuf>,
    /// What of this machine may go into a wadspace: strictly inside these, nothing hidden.
    pub roots: Vec<PathBuf>,
    /// This machine's id: a project from another is over Taildrive.
    pub machine_id: String,
    /// Where repositories are (`<github>/<owner>/<name>.git`).
    pub github: String,
    /// Taildrive's server.
    pub taildrive: String,
    /// A uid's home and name (getpwuid): a machine's person, looked up again
    /// when it had none at first.
    pub lookup: fn(u32) -> Option<(PathBuf, String)>,
}

/// A uid's home and name, from the machine's users.
pub fn user_of(uid: u32) -> Option<(PathBuf, String)> {
    nix::unistd::User::from_uid(uid.into()).ok().flatten().map(|u| (u.dir, u.name))
}

impl Settings {
    pub fn new(config: &wad_config::Config, profile: wad_config::Profile) -> Self {
        let d = &config.daemon;
        let system = profile == wad_config::Profile::System;
        let user = nix::unistd::User::from_uid(d.projects_uid.into()).ok().flatten();
        let home =
            if system { user.as_ref().map(|u| u.dir.clone()) } else { std::env::var_os("HOME").map(PathBuf::from) };
        let name = if system {
            user.map(|u| u.name)
        } else {
            nix::unistd::User::from_uid(nix::unistd::getuid()).ok().flatten().map(|u| u.name)
        };
        let roots = Self::roots_for(home.as_deref(), name.as_deref());
        Self {
            state_dir: d.state_dir.clone(),
            projects_dir: d.projects_dir.clone(),
            uid: d.projects_uid,
            root: nix::unistd::geteuid().is_root(),
            system,
            home,
            roots,
            machine_id: config.machine_id(),
            // github.com, or the stand-in a test machine's configuration names.
            github: if config.github.web.is_empty() { GITHUB.into() } else { config.github.web.clone() },
            taildrive: taildrive::TAILDRIVE.into(),
            lookup: user_of,
        }
    }

    /// What of this machine may go into a wadspace: the home, and the person's removable drives.
    pub fn roots_for(home: Option<&Path>, name: Option<&str>) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = home.iter().map(|h| h.to_path_buf()).collect();
        roots.extend(name.map(|n| PathBuf::from("/run/media").join(n)));
        roots
    }

    /// Who owns what wadd makes for a wadspace: the projects user, when wadd is root.
    fn owner(&self) -> Option<u32> {
        self.root.then_some(self.uid)
    }
}

/// What the open has of its projects.
#[derive(Debug, Default)]
pub struct Prepared {
    /// What's here, for its units and its projects.json.
    pub mounts: Mounts,
    /// What it opens without, and why.
    pub left_out: Vec<LeftOut>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOut {
    pub project: String,
    pub name: String,
    pub why: String,
}

/// An open's job, and what it knows of the other machines.
pub struct Opening {
    pub job: String,
    away: Away,
}

/// The tailnet and my machines on it, as the open found them.
#[derive(Default)]
struct Away {
    tailnet: Option<Tailnet>,
    peers: Vec<PeerInfo>,
}

impl Away {
    fn peer(&self, id: &str) -> Option<&PeerInfo> {
        self.peers.iter().find(|p| p.peer.id == id)
    }

    fn name(&self, id: &str) -> String {
        self.peer(id).map_or_else(|| "another machine".to_string(), |p| p.peer.name.clone())
    }
}

/// A project's files on another machine: whose, and its home's URL.
struct Remote {
    name: String,
    home: String,
}

/// What a project's stage did.
enum Got {
    /// Made now (cloned, copied, mounted).
    Made(PathBuf, String),
    /// Here already: nothing to do.
    Here(PathBuf, String),
}

impl Got {
    /// Where it is, its line for the job, and how its stage ends.
    fn parts(&self) -> (&Path, &str, PartState) {
        match self {
            Got::Made(p, l) => (p, l, PartState::Done),
            Got::Here(p, l) => (p, l, PartState::Skipped),
        }
    }
}

pub struct Projects {
    s: Settings,
    backend: Arc<dyn Backend>,
    jobs: Arc<Jobs>,
    net: Arc<dyn Net>,
    git: wad_git::Git,
    drive: Arc<dyn Drive>,
    dav: Dav,
    copies: Copies,
}

impl Projects {
    pub fn new(s: Settings, backend: Arc<dyn Backend>, jobs: Arc<Jobs>, net: Arc<dyn Net>) -> Self {
        let drive = Arc::new(Rclone {
            system: s.system,
            uid: s.uid,
            cache: s.state_dir.join("taildrive").join(".cache"),
            timeout: MOUNT_TIMEOUT,
        });
        let copies = Copies::new(&s.state_dir, s.owner());
        let git = wad_git::Git::default().with_github(&s.github);
        Self { s, backend, jobs, net, git, drive, dav: Dav::new(), copies }
    }

    /// Mounting with `drive` instead of rclone (tests).
    pub fn with_drive(self, drive: Arc<dyn Drive>) -> Self {
        Self { drive, ..self }
    }

    pub fn jobs(&self) -> &Arc<Jobs> {
        &self.jobs
    }

    fn here(&self, s: &ProjectSource) -> bool {
        s.machine_id.is_empty() || s.machine_id == self.s.machine_id
    }

    fn elsewhere(&self, p: &Project) -> bool {
        match &p.kind {
            ProjectKind::Github { .. } => false,
            ProjectKind::Mount { source, .. } | ProjectKind::Copy { source } => !self.here(source),
        }
    }

    async fn away(&self) -> Away {
        let tailnet = match self.net.tailnet().await {
            Ok(t) => Some(t),
            Err(e) => {
                tracing::info!("the tailnet: {}", e.message);
                None
            }
        };
        let peers = self.net.peers().await.unwrap_or_default();
        Away { tailnet, peers }
    }

    /// What its stage says ('Cloning HungGod/Writing', 'Mounting Thesis from Laptop over Taildrive').
    fn label(&self, away: &Away, p: &Project) -> String {
        let from = |s: &ProjectSource| {
            if self.here(s) { String::new() } else { format!(" from {} over Taildrive", away.name(&s.machine_id)) }
        };
        match &p.kind {
            ProjectKind::Github { repo, .. } => format!("Cloning {repo}"),
            ProjectKind::Mount { source, .. } => format!("Mounting {}{}", p.as_name, from(source)),
            ProjectKind::Copy { source } => format!("Copying {}{}", p.as_name, from(source)),
        }
    }

    /// The open's job (Opening {name}): a stage for each project, then its
    /// container's and its desktop's (`container`, `desktop`).
    pub async fn open_job(&self, ws: &Wadspace) -> Opening {
        let away = if ws.look.projects.iter().any(|p| self.elsewhere(p)) { self.away().await } else { Away::default() };
        let mut stages: Vec<(String, String)> =
            ws.look.projects.iter().map(|p| (format!("project:{}", p.id), self.label(&away, p))).collect();
        stages.push(("container".into(), "Starting its container".into()));
        stages.push(("desktop".into(), "Waiting for its desktop".into()));
        let job = self.jobs.create(JobKind::Launch, &format!("Opening {}", ws.name), &ws.id, &stages);
        self.jobs.running(&job);
        Opening { job, away }
    }

    /// One of the job's stages (`container`, `desktop`, a project's).
    pub fn stage(&self, job: &str, stage: &str, state: PartState) {
        self.jobs.stage(job, stage, state, None);
    }

    /// Its projects, here: each cloned, mounted or copied as it needs, a
    /// stage each (`say`: what it's doing now, for its run's message).
    pub async fn prepare(&self, ws: &Wadspace, o: &Opening, say: &(dyn Fn(&str) + Send + Sync)) -> Prepared {
        let mut out = Prepared::default();
        if !safe_id(&ws.id) {
            return out;
        }
        for p in &ws.look.projects {
            let stage = format!("project:{}", p.id);
            let label = self.label(&o.away, p);
            say(&label);
            self.jobs.stage(&o.job, &stage, PartState::Working, None);
            let got = if safe_id(&p.id) {
                self.make(ws, p, o, &stage).await
            } else {
                Err(format!("{:?} isn't a project's id", p.id))
            };
            let got = got.and_then(|g| {
                let (path, _, _) = g.parts();
                if wad_gen::mountable(&path.to_string_lossy(), &p.as_name) {
                    Ok(g)
                } else {
                    Err(format!("{} can't go into a wadspace: its name or its path has a ':' in it", p.as_name))
                }
            });
            match got {
                Ok(g) => {
                    let (path, line, state) = g.parts();
                    self.jobs.line(&o.job, line);
                    self.jobs.stage(&o.job, &stage, state, None);
                    out.mounts.insert(p.id.clone(), path.to_string_lossy().into_owned());
                }
                Err(why) => self.left_out(&mut out, &o.job, &stage, ws, p, why),
            }
        }
        out
    }

    fn left_out(&self, out: &mut Prepared, job: &str, stage: &str, ws: &Wadspace, p: &Project, why: String) {
        tracing::warn!("{}: {why} It opens without {}.", ws.id, p.as_name);
        self.jobs.line(job, &format!("✗ {why}"));
        self.jobs.stage(job, stage, PartState::Error, Some(why.clone()));
        out.left_out.push(LeftOut { project: p.id.clone(), name: p.as_name.clone(), why });
    }

    async fn make(&self, ws: &Wadspace, p: &Project, o: &Opening, stage: &str) -> Result<Got, String> {
        match &p.kind {
            ProjectKind::Github { repo, .. } => self.github(ws, p, repo, &o.job, stage).await,
            ProjectKind::Mount { source, make } => {
                let path = first(source, &p.as_name)?;
                if self.here(source) {
                    let real = self.local(&p.as_name, path, *make)?;
                    let line = format!("» {} → ~/Desktop/{}", real.display(), p.as_name);
                    Ok(Got::Made(real, line))
                } else {
                    self.mount_remote(ws, p, &o.away, source, path).await
                }
            }
            ProjectKind::Copy { source } => {
                let path = first(source, &p.as_name)?;
                let from = format!("{}:{path}", source.machine_id);
                let at = self.copies.path(&ws.id, &p.id);
                if let Some(m) = self.copies.made(&ws.id, &p.id) {
                    if m.from.is_empty() || m.from == from {
                        return Ok(Got::Here(at, format!("» {} is copied on this machine already", p.as_name)));
                    }
                    tracing::info!("{}: {} is another copy now: copied again", ws.id, p.id);
                }
                if self.here(source) {
                    let src = self.local(&p.as_name, path, false)?;
                    self.jobs.line(&o.job, &format!("» copying {}", src.display()));
                    let made = self.copies.copy_local(&ws.id, &p.id, &from, &src).await?;
                    Ok(Got::Made(at, format!("✓ {} files, {} MB", made.files, made.bytes >> 20)))
                } else {
                    let r = self.remote(&o.away, source, &p.as_name)?;
                    let url = taildrive::url_in(&r.home, path).map_err(|e| format!("{}: {e}", p.as_name))?;
                    let kind = self.dav.stat(&url).await.map_err(|e| dav_why(&p.as_name, &r.name, e))?;
                    self.jobs.line(&o.job, &format!("» copying {url}"));
                    let (jobs, job, st) = (self.jobs.clone(), o.job.clone(), stage.to_string());
                    let made = self
                        .copies
                        .fetch(&ws.id, &p.id, &from, &self.dav, &url, kind, move |done, total| {
                            jobs.bytes(&job, &st, done, total)
                        })
                        .await
                        .map_err(|e| dav_why(&p.as_name, &r.name, e))?;
                    Ok(Got::Made(at, format!("✓ {} files, {} MB, from {}", made.files, made.bytes >> 20, r.name)))
                }
            }
        }
    }

    // ------------------------------------------------------------ GitHub

    /// A GitHub project's clone here.
    pub fn clone_dir(&self, ws: &str, project: &str) -> PathBuf {
        self.s.projects_dir.join(ws).join(project)
    }

    async fn github(&self, ws: &Wadspace, p: &Project, repo: &str, job: &str, stage: &str) -> Result<Got, String> {
        let dest = self.clone_dir(&ws.id, &p.id);
        let url = format!("{}/{repo}.git", self.s.github.trim_end_matches('/'));
        if dest.exists() {
            match origin(&dest) {
                Some(o) if o != url => {
                    // Its id was another repository's, whose clone was kept: aside, not over it.
                    let aside = self.keep_aside(&ws.id, &p.id)?;
                    tracing::info!("{}: {} was {o}'s clone: kept at {}", ws.id, p.id, aside.display());
                }
                _ => {
                    relabel(&dest).await;
                    let line =
                        format!("» {} is on this machine already: the wadspace fetches it as it starts", p.as_name);
                    return Ok(Got::Here(dest, line));
                }
            }
        }
        self.owned_dir(&ws.id)?;
        let token = self.backend.secret_value(GITHUB_SECRET).await.ok().flatten().filter(|t| !t.trim().is_empty());
        self.jobs.line(job, &format!("» git clone https://github.com/{repo} ~/Desktop/{}", p.as_name));
        let (jobs, j, st) = (self.jobs.clone(), job.to_string(), stage.to_string());
        let on_line = move |text: &str, part: Option<f64>| match part {
            // Its progress, redrawn: the stage's, and only its last line.
            Some(f) => {
                jobs.progress(&j, &st, f);
                if f >= 1.0 {
                    jobs.line(&j, text);
                }
            }
            None => jobs.line(&j, text),
        };
        let r = self
            .git
            .clone_repo(&url, None, &dest, token.as_deref(), on_line, Some(self.s.uid), |part| {
                // The projects user's alone (wad-git hands it to them), and
                // labelled for containers: what git puts in it takes the label.
                let _ = std::fs::set_permissions(part, std::fs::Permissions::from_mode(0o700));
                if let Err(e) = selinux::label(part, selinux::CONTAINER_FILE) {
                    tracing::warn!("{e}");
                }
            })
            .await;
        match r {
            Ok(()) => Ok(Got::Made(dest, format!("✓ cloned {repo}"))),
            Err(wad_git::Error::NoGit) => Err("git isn't on this machine".into()),
            Err(e) => Err(clone_failed(repo, token.is_some(), &e.to_string())),
        }
    }

    /// `<projects_dir>/<ws>`: the projects user's (0700), in a projects
    /// folder they can pass through (0711, the machine's tmpfiles make it).
    fn owned_dir(&self, ws: &str) -> Res<PathBuf> {
        let top = &self.s.projects_dir;
        if !top.exists() {
            std::fs::create_dir_all(top).map_err(|e| format!("{}: {e}", top.display()))?;
            std::fs::set_permissions(top, std::fs::Permissions::from_mode(0o711))
                .map_err(|e| format!("{}: {e}", top.display()))?;
        }
        let dir = top.join(ws);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        if let Some(uid) = self.s.owner() {
            nix::unistd::chown(&dir, Some(uid.into()), Some(uid.into()))
                .map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        Ok(dir)
    }

    /// A clone moved out of a project's way: `<projects_dir>/<ws>/.kept-<project>-<ms>`.
    fn keep_aside(&self, ws: &str, project: &str) -> Res<PathBuf> {
        let from = self.clone_dir(ws, project);
        let to = self.s.projects_dir.join(ws).join(format!(".kept-{project}-{}", now_ms()));
        std::fs::rename(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
        Ok(to)
    }

    // ------------------------------------------------------------ this machine

    /// A path of this machine ('~/Thesis', or whole) that may go into a
    /// wadspace: there, inside the roots, nothing hidden; symlinks resolved.
    /// A path of this machine's (`~/…` or absolute) that may go into a
    /// wadspace: in the person's home, not hidden (Open with's file).
    pub fn local_path(&self, name: &str, path: &str) -> Result<PathBuf, String> {
        self.local(name, path, false)
    }

    /// The person's home and what may go in, as they are now: a machine's
    /// person is looked up again when there was none as wadd started (made
    /// after it: the installer, a test's useradd).
    fn person(&self) -> (Option<PathBuf>, Vec<PathBuf>) {
        if self.s.home.is_some() || !self.s.system {
            return (self.s.home.clone(), self.s.roots.clone());
        }
        match (self.s.lookup)(self.s.uid) {
            Some((home, name)) => (Some(home.clone()), Settings::roots_for(Some(&home), Some(&name))),
            None => (None, self.s.roots.clone()),
        }
    }

    fn local(&self, name: &str, path: &str, make: bool) -> Result<PathBuf, String> {
        let (home, roots) = self.person();
        let home = home.as_ref();
        let full = match path.strip_prefix('~') {
            Some("") => home.cloned().ok_or("this machine's person has no home")?,
            Some(rest) if rest.starts_with('/') => {
                home.ok_or("this machine's person has no home")?.join(rest.trim_start_matches('/'))
            }
            Some(_) => return Err(format!("{name}: {path} isn't a path on this machine")),
            None if path.starts_with('/') => PathBuf::from(path),
            None => return Err(format!("{name}: {path} isn't a path on this machine")),
        };
        // `make`: a folder right in the home, made empty (the person's) when it isn't there.
        let one = path.strip_prefix("~/").filter(|r| !r.is_empty() && !r.contains('/') && !r.starts_with('.'));
        if make && one.is_some() && std::fs::symlink_metadata(&full).is_err() {
            let (at, owner) = (full.clone(), self.s.owner());
            let mkdir = move || {
                std::fs::create_dir(&at).map_err(|e| format!("{}: {e}", at.display()))?;
                if let Some(uid) = owner {
                    nix::unistd::chown(&at, Some(uid.into()), Some(uid.into()))
                        .map_err(|e| format!("{}: {e}", at.display()))?;
                }
                Ok(())
            };
            // wadd.service sees the home read-only (ProtectHome): made where the machine sees it.
            if self.s.system && self.s.root {
                crate::mounts::inside(1, mkdir)?;
            } else {
                mkdir()?;
            }
        }
        let real = std::fs::canonicalize(&full)
            .map_err(|_| format!("{name} isn't on this machine any more ({path} isn't there)"))?;
        if allowed(&roots, &real) {
            Ok(real)
        } else {
            Err(format!(
                "{name}: only what's in your home (not the home itself, nor hidden folders) can go into a wadspace, and {path} isn't"
            ))
        }
    }
}

fn allowed(roots: &[PathBuf], real: &Path) -> bool {
    roots.iter().any(|r| {
        let r = std::fs::canonicalize(r).unwrap_or_else(|_| r.clone());
        real.strip_prefix(&r).is_ok_and(|rest| {
            let parts: Vec<_> = rest.components().collect();
            !parts.is_empty()
                && parts
                    .iter()
                    .all(|c| matches!(c, std::path::Component::Normal(n) if !n.to_string_lossy().starts_with('.')))
        })
    })
}

impl Projects {
    // ------------------------------------------------------------ other machines

    /// Where a project's machine's home is over Taildrive, or why it can't be reached now.
    fn remote(&self, away: &Away, s: &ProjectSource, name: &str) -> Result<Remote, String> {
        let running = away.tailnet.as_ref().filter(|t| t.state == TailnetState::Running);
        let Some(t) = running else { return Err("Join your tailnet first: files come over Taildrive".into()) };
        let Some(peer) = away.peer(&s.machine_id) else {
            return Err(format!("{name} is on a machine this one can't find among yours on the tailnet"));
        };
        if !peer.peer.online {
            return Err(format!(
                "{name} is on {}, which is offline. Folders come over Taildrive, so both machines need to be online.",
                peer.peer.name
            ));
        }
        let home = taildrive::home_url(&self.s.taildrive, &t.tailnet, &peer.dns_name, &peer.peer.hostname);
        Ok(Remote { name: peer.peer.name.clone(), home })
    }

    /// Where a wadspace's Taildrive mounts are: `<state_dir>/taildrive/<ws>`.
    fn mounts_dir(&self, ws: &str) -> PathBuf {
        self.s.state_dir.join("taildrive").join(ws)
    }

    async fn mount_remote(
        &self,
        ws: &Wadspace,
        p: &Project,
        away: &Away,
        s: &ProjectSource,
        path: &str,
    ) -> Result<Got, String> {
        let r = self.remote(away, s, &p.as_name)?;
        let url = taildrive::url_in(&r.home, path).map_err(|e| format!("{}: {e}", p.as_name))?;
        match self.dav.stat(&url).await {
            Ok(Kind::Folder) => {}
            Ok(Kind::File(_)) => {
                return Err(format!(
                    "{} is a file on {}: another machine's file is copied in, not mounted",
                    p.as_name, r.name
                ));
            }
            Err(e) => return Err(dav_why(&p.as_name, &r.name, e)),
        }
        let top = self.s.state_dir.join("taildrive");
        for d in [top.clone(), self.mounts_dir(&ws.id)] {
            std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
            std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("{}: {e}", d.display()))?;
        }
        let at = self.mounts_dir(&ws.id).join(&p.id);
        let unit = taildrive::unit_name(&ws.id, &p.id);
        // One left from before (wadd stopped while it ran): gone first.
        if let Err(e) = self.drive.unmount(&unit, &at).await {
            return Err(format!("{}: an earlier mount of it is still there: {e}", p.as_name));
        }
        std::fs::create_dir_all(&at).map_err(|e| format!("{}: {e}", at.display()))?;
        self.drive.mount(&unit, &url, &at).await.map_err(|e| format!("{} from {}: {e}", p.as_name, r.name))?;
        Ok(Got::Made(at, format!("» {url} → ~/Desktop/{}", p.as_name)))
    }

    /// Its Taildrive mounts undone (it stopped): each folder removed once it's empty.
    pub async fn release(&self, ws: &str) {
        if !safe_id(ws) {
            return;
        }
        let dir = self.mounts_dir(ws);
        for project in copies::names(&dir) {
            let at = dir.join(&project);
            match self.drive.unmount(&taildrive::unit_name(ws, &project), &at).await {
                // Never anything but an empty folder: what's under a mount is the other machine's.
                Ok(()) => {
                    let _ = std::fs::remove_dir(&at);
                }
                Err(e) => tracing::warn!("{ws}: {project} over Taildrive is still mounted at {}: {e}", at.display()),
            }
        }
        let _ = std::fs::remove_dir(&dir);
    }

    // ------------------------------------------------------------ cleaning up

    /// What wadd made for projects it doesn't have any more, gone (it isn't
    /// running): copies, clones (kept aside while they have work that isn't
    /// pushed), Taildrive mounts.
    pub async fn tidy(&self, ws: &Wadspace) {
        if !safe_id(&ws.id) {
            return;
        }
        self.release(&ws.id).await;
        let has = |id: &str, github: bool| {
            ws.look.projects.iter().any(|p| {
                p.id == id
                    && match &p.kind {
                        ProjectKind::Github { .. } => github,
                        ProjectKind::Copy { .. } => !github,
                        ProjectKind::Mount { .. } => false,
                    }
            })
        };
        for project in copies::names(&self.s.projects_dir.join(&ws.id)) {
            if !has(&project, true) {
                self.purge_clone(&ws.id, &project).await;
            }
        }
        for project in self.copies.ids(&ws.id) {
            if !has(&project, false) {
                match self.copies.remove(&ws.id, &project) {
                    Ok(()) => tracing::info!("{}: {project}'s copy removed (it isn't in it any more)", ws.id),
                    Err(e) => tracing::warn!("{}: {project}'s copy: {e}", ws.id),
                }
            }
        }
    }

    async fn purge_clone(&self, ws: &str, project: &str) {
        let dir = self.clone_dir(ws, project);
        if wad_git::is_repo(&dir) {
            let st = self.git.status(&dir, Some(self.s.uid)).await;
            if !st.as_ref().is_some_and(wad_git::GitStatus::all_pushed) {
                match self.keep_aside(ws, project) {
                    Ok(to) => {
                        tracing::warn!("{ws}: {project}'s clone has work that isn't pushed: kept at {}", to.display())
                    }
                    Err(e) => tracing::warn!("{ws}: {project}'s clone: {e}"),
                }
                return;
            }
        }
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => tracing::info!("{ws}: {project}'s clone removed (it isn't in it any more)"),
            Err(e) => tracing::warn!("{ws}: {project}'s clone: {}: {e}", dir.display()),
        }
    }

    /// A wadspace gone for good: everything wadd made for it.
    pub async fn forget(&self, ws: &str) {
        if !safe_id(ws) {
            return;
        }
        self.release(ws).await;
        let clones = self.s.projects_dir.join(ws);
        if clones.exists() && std::fs::remove_dir_all(&clones).is_ok() {
            tracing::info!("{ws}: its clones removed");
        }
        self.copies.remove_all(ws);
    }
}

/// A project's first path (one is all keepFiles makes).
fn first<'a>(s: &'a ProjectSource, name: &str) -> Result<&'a str, String> {
    s.paths.first().map(String::as_str).ok_or_else(|| format!("{name} says nothing about where it is"))
}

/// Why a clone failed, in words to act on. GitHub answers a private
/// repository it won't show as one that isn't there.
pub fn clone_failed(repo: &str, signed_in: bool, e: &str) -> String {
    let l = e.to_ascii_lowercase();
    let unseen = [
        "could not read username",
        "terminal prompts disabled",
        "authentication failed",
        "repository not found",
        "not found",
        "does not appear to be a git repository",
        "could not read from remote repository",
        "error: 401",
        "error: 403",
    ]
    .iter()
    .any(|w| l.contains(w));
    match (unseen, signed_in) {
        (true, false) => "It's private: sign in to GitHub first".into(),
        (true, true) => format!("{repo} isn't on GitHub, or your GitHub account can't see it"),
        (false, _) => format!("Couldn't clone {repo}: {e}"),
    }
}

/// Why another machine's files didn't come.
fn dav_why(name: &str, peer: &str, e: DavError) -> String {
    match e {
        DavError::NotFound => format!("{name} isn't on {peer} any more"),
        DavError::Refused(_) => {
            format!(
                "{peer} didn't let this machine in over Taildrive: your tailnet's policy needs drive:share and drive:access"
            )
        }
        DavError::Unreachable(e) => format!("{peer} didn't answer over Taildrive ({e})"),
        DavError::Failed(e) => format!("{name} from {peer}: {e}"),
    }
}

/// A clone's `origin` (its `.git/config`'s).
fn origin(repo: &Path) -> Option<String> {
    let text = std::fs::read_to_string(repo.join(".git/config")).ok()?;
    let mut in_origin = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_origin = line == "[remote \"origin\"]";
        } else if in_origin
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == "url"
        {
            return Some(v.trim().to_string());
        }
    }
    None
}

/// A file's SELinux label.
fn label_of(path: &Path) -> Option<String> {
    let p = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    let mut buf = vec![0u8; 512];
    // SAFETY: lgetxattr with valid C strings and a buffer of the given length.
    let n = unsafe { libc::lgetxattr(p.as_ptr(), c"security.selinux".as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        return None;
    }
    buf.truncate(n as usize);
    String::from_utf8(buf).ok().map(|s| s.trim_end_matches('\0').to_string())
}

/// A clone labelled for containers again, if something relabelled it
/// (a restorecon of /var): the whole tree, as v1's relabel.
async fn relabel(path: &Path) {
    if !selinux::enabled() || label_of(path).is_some_and(|l| l.contains(":container_file_t:")) {
        return;
    }
    match tokio::process::Command::new("chcon").args(["-R", "-t", "container_file_t", "--"]).arg(path).output().await {
        Ok(o) if o.status.success() => tracing::info!("{}: labelled for containers again", path.display()),
        Ok(o) => tracing::warn!("labelling {}: {}", path.display(), String::from_utf8_lossy(&o.stderr).trim()),
        Err(e) => tracing::warn!("labelling {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn why_a_clone_failed() {
        let no_user = "git clone https://github.com/o/r.git failed: fatal: could not read Username for 'https://github.com': terminal prompts disabled";
        assert_eq!(clone_failed("o/r", false, no_user), "It's private: sign in to GitHub first");
        let missing = "git clone failed: remote: Repository not found.";
        assert_eq!(clone_failed("o/r", true, missing), "o/r isn't on GitHub, or your GitHub account can't see it");
        let net = "git clone failed: fatal: unable to access 'https://github.com/o/r.git/': Could not resolve host: github.com";
        assert!(clone_failed("o/r", true, net).starts_with("Couldn't clone o/r: "));
    }

    #[test]
    fn a_clones_origin() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".git")).unwrap();
        std::fs::write(
            d.path().join(".git/config"),
            "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = https://github.com/o/r.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n",
        )
        .unwrap();
        assert_eq!(origin(d.path()).as_deref(), Some("https://github.com/o/r.git"));
        assert_eq!(origin(&d.path().join("nope")), None);
    }
}
