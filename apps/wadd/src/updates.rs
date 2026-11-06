//! Updates (phase 8): WadOS's own image, through bootc, and the base images
//! wadspaces are built on ([`crate::api_phase8::Updates`], the Updates page).
//!
//! **WadOS's image.** bootc keeps two deployments, the booted one and the
//! one before (`rollback`), and stages a third for the next start. wadd asks
//! bootc ([`Bootc`]) and never restarts the machine: that's the person's,
//! from the session (logind). So bootc's own timer
//! (`bootc-fetch-apply-updates.timer`, which would stage *and reboot*) stays
//! off in the host image, and wadd decides when to look instead:
//!
//! - **check** (`bootc upgrade --check`): an Update job that fetches only the
//!   newer image's manifest, and says what it found (its version and
//!   digest: bootc 1.17 doesn't keep it in `bootc status`'s
//!   `cachedUpdate`, which is read too). One found that the person hasn't turned down is
//!   downloaded straight away, as the spike's check does;
//! - **apply** (`bootc upgrade`, its progress on a pipe: [`UPGRADE`]): an Update job, Downloading
//!   (bootc's progress: its bytes) then staging. Once staged, a notice with
//!   Restart (`power.restart`, the shell's);
//! - **cancel**: a staged update is undeployed (`ostree admin undeploy`),
//!   and its digest kept as turned down, so the next check doesn't fetch
//!   it again (Download on the page still does); a queued rollback is
//!   swapped back (`bootc rollback` again);
//! - **rollback** (`bootc rollback`): the one before is queued for the next
//!   start (a staged update is dropped; a check or download under way stops).
//!
//! Every 12 hours (from the last check) wadd checks by itself, on a
//! machine's wadd only (`Settings::schedule`); restarting is always the
//! person's.
//!
//! **Signing in to the registry.** The host image on ghcr is public, so
//! bootc needs no login. A private one's check that's refused (401, denied)
//! is tried again once with this machine's GitHub token (the podman secret
//! `github_token`, phase 6) written as ostree's login,
//! `/run/ostree/auth.json` (0600, root's, in `/run` only: never in the
//! image and never on the disk), for ghcr.io only.
//!
//! **How bootc runs.** wadd.service is sandboxed (ProtectSystem=strict) and
//! bootc writes the boot loader and `/sysroot`: as root, wadd has systemd
//! run it as a unit of its own (`systemd-run --wait`), outside the
//! sandbox (its output in files in `/run/wados/bootc`), stopped if its job
//! is cancelled.
//!
//! **The base images** ([`BaseImage`]): the base and the stream sidecar,
//! as wadspaces use them (`localhost/wados-base:44`), from where CI
//! publishes them (`ghcr.io/hunggod/wados-base:44`). Refresh is a Pull job,
//! a stage each: the published one pulled if it's newer (with the GitHub
//! token, if there's one) and named as wadspaces use it. A wadspace built on
//! the base before shows as an update and rebuilds when it next opens
//! (`Builds::refresh_base`, through `Hooks::bases_changed`). Every 12 hours
//! too, on a machine.
//!
//! What wadd remembers between starts (when it last looked, what was turned
//! down, when each base was refreshed): `<state_dir>/updates.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wad_model::{JobKind, PartState};
use wad_proto as proto;
use wad_proto::{ApiError, Base, Bases, ErrorCode, Event, JobInfo, Level, NoticeAction, UpdateProgress, Version};

use crate::backend::{ImageInfo, OnBytes, OnLine, Res};
use crate::events::Bus;
use crate::jobs::{Jobs, now_ms};
use crate::packages::GITHUB_SECRET;

/// The user name a registry sees: ghcr checks the token, not the name.
const REGISTRY_USER: &str = "wadspaces";

/// Between checks, and between refreshes of the base images.
pub const EVERY: Duration = Duration::from_secs(12 * 3600);
/// ostree's registry login, in `/run` only (bootc reads it before `/etc/ostree/auth.json`).
pub const AUTH_FILE: &str = "/run/ostree/auth.json";
/// Where CI publishes the images (`localhost/<name>` here).
pub const PUBLISHED: &str = "ghcr.io/hunggod/";
/// The registry the GitHub token signs in to.
const GHCR: &str = "ghcr.io";
/// Downloads and stages the update, its progress (JSON lines) on standard
/// output: bootc writes it only to a pipe, and not on fd 1 (its words go to
/// standard error instead).
pub const UPGRADE: &[&str] = &["bash", "-c", "set -o pipefail; bootc upgrade --progress-fd 3 3>&1 >&2 | cat"];
/// Drops a staged deployment (bootc has no verb for it): ostree's index 0 is the staged one.
pub const UNDEPLOY_STAGED: &[&str] = &["ostree", "admin", "undeploy", "0"];

/// The words of a failure that mean the registry wasn't reached.
const OFFLINE: &[&str] = &[
    "dial tcp",
    "no such host",
    "network is unreachable",
    "temporary failure in name resolution",
    "i/o timeout",
    "tls handshake timeout",
    "connection timed out",
    "no route to host",
];
/// ... and that it wants a login.
const REFUSED: &[&str] = &["unauthorized", "authentication required", "denied", "401", "403"];

// ------------------------------------------------------------ bootc

/// bootc (and ostree), as wadd runs them: a trait, so the rest is tested
/// against canned answers.
#[async_trait]
pub trait Bootc: Send + Sync + 'static {
    /// This machine boots through bootc (ostree's `/run/ostree-booted`).
    fn here(&self) -> bool;
    /// Runs `argv` (`bootc …`, `ostree …`): its standard output, each line
    /// to `on_line` as it comes; Err: why it failed, in its words. Dropping
    /// the future stops it.
    async fn run(&self, argv: &[&str], on_line: OnLine) -> Res<String>;
}

/// The machine's bootc: as root through systemd (a unit of its own, outside
/// wadd's sandbox), else as a child.
///
/// The unit's output goes to files in `out_dir` (wadd.service's
/// `/run/wados/bootc`), which wadd follows while `systemd-run --wait`
/// waits: `systemd-run --pipe` would hand the unit wadd's pipes over the
/// system bus, and dbus-broker drops a connection that does that from
/// wadd's service ("Connection reset by peer").
pub struct SystemBootc {
    through_systemd: bool,
    out_dir: PathBuf,
    seq: AtomicU64,
}

/// Where the units' output goes (wadd.service's RuntimeDirectory).
pub const OUT_DIR: &str = "/run/wados/bootc";

impl SystemBootc {
    pub fn new() -> Self {
        Self { through_systemd: nix::unistd::geteuid().is_root(), out_dir: OUT_DIR.into(), seq: AtomicU64::new(0) }
    }

    /// As a child of wadd's: its output, each line to `on_line`.
    async fn child(&self, argv: &[&str], on_line: &mut OnLine) -> Res<String> {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt};
        let (prog, args) = argv.split_first().ok_or("nothing to run")?;
        let mut child = tokio::process::Command::new(prog)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("{prog} didn't start: {e}"))?;
        let mut stderr = child.stderr.take().expect("piped");
        let err = tokio::spawn(async move {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s).await;
            s
        });
        let mut lines = tokio::io::BufReader::new(child.stdout.take().expect("piped")).lines();
        let mut out = String::new();
        while let Ok(Some(l)) = lines.next_line().await {
            on_line(&l);
            out.push_str(&l);
            out.push('\n');
        }
        let status = child.wait().await.map_err(|e| format!("{prog}: {e}"))?;
        let err = err.await.unwrap_or_default();
        if status.success() { Ok(out) } else { Err(failure(prog, &err, status.code())) }
    }

    /// As a unit of systemd's, its output in files followed meanwhile.
    async fn unit(&self, argv: &[&str], on_line: &mut OnLine) -> Res<String> {
        let prog = argv.first().ok_or("nothing to run")?;
        let unit = format!("wados-{prog}-{}-{}", std::process::id(), self.seq.fetch_add(1, Ordering::Relaxed));
        std::fs::create_dir_all(&self.out_dir).map_err(|e| format!("{}: {e}", self.out_dir.display()))?;
        let (out_file, err_file) = (self.out_dir.join(format!("{unit}.out")), self.out_dir.join(format!("{unit}.err")));
        struct Files(PathBuf, PathBuf);
        impl Drop for Files {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
                let _ = std::fs::remove_file(&self.1);
            }
        }
        let _files = Files(out_file.clone(), err_file.clone());
        let mut child = tokio::process::Command::new("systemd-run")
            .args(["--quiet", "--wait", "--collect", "--service-type=exec"])
            .arg(format!("--unit={unit}"))
            .arg("--property=Description=WadOS, outside wadd's sandbox")
            // What it prints stays root's.
            .arg("--property=UMask=0077")
            .arg(format!("--property=StandardOutput=truncate:{}", out_file.display()))
            .arg(format!("--property=StandardError=truncate:{}", err_file.display()))
            .arg("--")
            .args(argv)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("systemd-run didn't start: {e}"))?;
        let mut guard = StopUnit(Some(unit));
        let mut follow = Follow::default();
        let status = loop {
            tokio::select! {
                s = child.wait() => break s.map_err(|e| format!("systemd-run: {e}"))?,
                _ = tokio::time::sleep(Duration::from_millis(250)) => follow.read(&out_file, on_line),
            }
        };
        guard.0 = None;
        follow.read(&out_file, on_line);
        follow.end(on_line);
        let mut said = std::fs::read_to_string(&err_file).unwrap_or_default();
        if let Some(mut e) = child.stderr.take() {
            use tokio::io::AsyncReadExt;
            let _ = e.read_to_string(&mut said).await;
        }
        if status.success() { Ok(follow.all) } else { Err(failure(prog, &said, status.code())) }
    }
}

/// The podman command line, outside wadd's sandbox: ProtectSystem=strict
/// keeps it from podman's storage, so inside it only fails. Each line of its
/// output to `on_line`; all of it, at the end. One runner, so units never share a name.
pub async fn podman(args: &[String], on_line: &mut (dyn FnMut(&str) + Send)) -> Res<String> {
    let mut argv: Vec<&str> = vec!["podman"];
    argv.extend(args.iter().map(String::as_str));
    outside(&argv, on_line).await
}

/// `argv`, outside wadd's sandbox.
pub async fn outside(argv: &[&str], on_line: &mut (dyn FnMut(&str) + Send)) -> Res<String> {
    static RUNNER: std::sync::OnceLock<SystemBootc> = std::sync::OnceLock::new();
    let out = RUNNER.get_or_init(SystemBootc::new);
    // Its lines come here (the runner's callback lasts for ever, this one doesn't).
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut lines: OnLine = Box::new(move |l: &str| {
        let _ = tx.send(l.to_string());
    });
    let run = async {
        if out.through_systemd { out.unit(argv, &mut lines).await } else { out.child(argv, &mut lines).await }
    };
    tokio::pin!(run);
    loop {
        tokio::select! {
            r = &mut run => {
                while let Ok(l) = rx.try_recv() {
                    on_line(&l);
                }
                return r;
            }
            Some(l) = rx.recv() => on_line(&l),
        }
    }
}

/// A file being written, read line by line as it grows.
#[derive(Default)]
struct Follow {
    at: u64,
    part: String,
    all: String,
}

impl Follow {
    fn read(&mut self, path: &Path, on_line: &mut OnLine) {
        use std::io::{Read, Seek};
        let Ok(mut f) = std::fs::File::open(path) else { return };
        if f.seek(std::io::SeekFrom::Start(self.at)).is_err() {
            return;
        }
        let mut buf = String::new();
        let Ok(n) = f.read_to_string(&mut buf) else { return };
        self.at += n as u64;
        self.part.push_str(&buf);
        while let Some(i) = self.part.find('\n') {
            let line: String = self.part.drain(..=i).collect();
            let line = line.trim_end_matches('\n');
            on_line(line);
            self.all.push_str(line);
            self.all.push('\n');
        }
    }

    fn end(&mut self, on_line: &mut OnLine) {
        if !self.part.is_empty() {
            let line = std::mem::take(&mut self.part);
            on_line(&line);
            self.all.push_str(&line);
        }
    }
}

impl Default for SystemBootc {
    fn default() -> Self {
        Self::new()
    }
}

/// A unit stopped once its run is dropped before it ended (its job cancelled).
struct StopUnit(Option<String>);

impl Drop for StopUnit {
    fn drop(&mut self) {
        let Some(unit) = self.0.take() else { return };
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                let _ = tokio::process::Command::new("systemctl")
                    .args(["stop", "--no-block", &unit])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await;
            });
        }
    }
}

#[async_trait]
impl Bootc for SystemBootc {
    fn here(&self) -> bool {
        Path::new("/run/ostree-booted").exists()
    }

    async fn run(&self, argv: &[&str], mut on_line: OnLine) -> Res<String> {
        if self.through_systemd { self.unit(argv, &mut on_line).await } else { self.child(argv, &mut on_line).await }
    }
}

/// What a failed run said: its last line of errors ("error: " off), else its exit code.
pub fn failure(prog: &str, stderr: &str, code: Option<i32>) -> String {
    match stderr.lines().map(str::trim).rfind(|l| !l.is_empty()) {
        Some(l) => l.strip_prefix("error: ").unwrap_or(l).to_string(),
        None => format!("{prog} failed ({})", code.map_or("killed".into(), |c| format!("exit {c}"))),
    }
}

fn offline(e: &str) -> bool {
    let e = e.to_lowercase();
    OFFLINE.iter().any(|w| e.contains(w))
}

fn refused(e: &str) -> bool {
    let e = e.to_lowercase();
    REFUSED.iter().any(|w| e.contains(w))
}

// ------------------------------------------------------------ bootc's status

/// What `bootc status --json` says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Host {
    /// What it follows (`spec.image`): its reference and transport (`registry`, `oci`…).
    pub image: String,
    pub transport: String,
    pub booted: Option<Version>,
    pub staged: Option<Version>,
    pub rollback: Option<Version>,
    /// `bootc rollback` was run: the next start boots `rollback`.
    pub rollback_queued: bool,
    /// What the last `bootc upgrade --check` found (the booted one's `cachedUpdate`).
    pub cached: Option<Version>,
}

/// One of bootc's image statuses (`{image: {image, transport}, version, imageDigest}`).
fn version_of(i: &Value) -> Option<Version> {
    let digest = i.get("imageDigest").and_then(Value::as_str)?.to_string();
    let image = i.pointer("/image/image").and_then(Value::as_str).unwrap_or_default().to_string();
    let version = match i.get("version").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        Some(v) => v.to_string(),
        // No version label: its digest, short.
        None => digest.trim_start_matches("sha256:").chars().take(12).collect(),
    };
    Some(Version { version, image, digest: Some(digest) })
}

/// `bootc status --json` (the v1 tests' JSON, and bootc 1.17's).
pub fn parse_status(text: &str) -> Result<Host, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("bootc's status didn't read: {e}"))?;
    let entry = |k: &str| v.pointer(&format!("/status/{k}/image")).and_then(version_of);
    let s = |p: &str| v.pointer(p).and_then(Value::as_str).unwrap_or_default().to_string();
    Ok(Host {
        image: s("/spec/image/image"),
        transport: s("/spec/image/transport"),
        booted: entry("booted"),
        staged: entry("staged"),
        rollback: entry("rollback"),
        rollback_queued: v.pointer("/status/rollbackQueued").and_then(Value::as_bool).unwrap_or(false),
        cached: v.pointer("/status/booted/cachedUpdate").and_then(version_of),
    })
}

/// What `bootc upgrade --check` says it found:
///
/// ```text
/// Update available for: docker://ghcr.io/hunggod/wados-host:latest
///   Version: 44.20261105.2
///   Digest: sha256:fe84…
/// ```
///
/// (or `No changes in: …`: none).
pub fn parse_check(text: &str) -> Option<Version> {
    let mut lines = text.lines().map(str::trim);
    let image = lines.find_map(|l| l.strip_prefix("Update available for:"))?.trim();
    let image = image.strip_prefix("docker://").unwrap_or(image).to_string();
    let (mut version, mut digest) = (None, None);
    for l in lines {
        if let Some(v) = l.strip_prefix("Version:") {
            // "44.20261105.2" (bootc may add the build's time after it).
            version = v.split_whitespace().next().map(String::from);
        } else if let Some(d) = l.strip_prefix("Digest:") {
            digest = d.split_whitespace().next().map(String::from);
        }
    }
    let digest = digest?;
    let version = version.unwrap_or_else(|| digest.trim_start_matches("sha256:").chars().take(12).collect());
    Some(Version { version, image, digest: Some(digest) })
}

fn same(a: Option<&Version>, b: Option<&Version>) -> bool {
    matches!((a, b), (Some(a), Some(b)) if a.digest == b.digest)
}

/// One line of bootc's progress (`--progress-fd`, JSON lines): (task, done, total)
/// in bytes for a download, in steps otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Bytes { done: u64, total: u64 },
    Steps { task: String, done: u64, total: u64 },
}

pub fn progress_of(line: &str) -> Option<Progress> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    match v.get("type").and_then(Value::as_str)? {
        "ProgressBytes" => Some(Progress::Bytes { done: n("bytes"), total: n("bytesTotal") }),
        "ProgressSteps" => Some(Progress::Steps {
            task: v.get("task").and_then(Value::as_str).unwrap_or_default().to_string(),
            done: n("steps"),
            total: n("stepsTotal"),
        }),
        _ => None,
    }
}

/// containers-auth.json for ghcr.io, with the GitHub token.
pub fn auth_json(token: &str) -> String {
    let auth = base64::engine::general_purpose::STANDARD.encode(format!("{REGISTRY_USER}:{token}"));
    format!("{{\"auths\":{{\"{GHCR}\":{{\"auth\":\"{auth}\"}}}}}}\n")
}

/// `text` written to `path`, root's only (0600), whole or not at all.
pub fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

// ------------------------------------------------------------ the base images

/// What base images are pulled with: a trait, so refreshing is tested
/// against a fake.
#[async_trait]
pub trait Images: Send + Sync + 'static {
    async fn info(&self, image: &str) -> Res<Option<ImageInfo>>;
    /// Downloads `reference` if the registry has a newer one than here
    /// (`auth`: X-Registry-Auth); its id. Dropping the future stops it.
    async fn pull_newer(&self, reference: &str, auth: Option<String>, on_bytes: OnBytes) -> Res<String>;
    async fn tag(&self, image: &str, target: &str) -> Res<()>;
    /// The GitHub token (the podman secret), if there's one.
    async fn token(&self) -> Option<String>;
}

/// The machine's podman.
pub struct PodmanImages(pub wad_podman::Podman);

#[async_trait]
impl Images for PodmanImages {
    async fn info(&self, image: &str) -> Res<Option<ImageInfo>> {
        self.0.image_info(image).await.map_err(|e| e.to_string())
    }

    async fn pull_newer(&self, reference: &str, auth: Option<String>, mut on_bytes: OnBytes) -> Res<String> {
        self.0.pull_newer(reference, auth.as_deref(), &mut on_bytes).await.map_err(|e| e.to_string())
    }

    async fn tag(&self, image: &str, target: &str) -> Res<()> {
        self.0.tag(image, target).await.map_err(|e| e.to_string())
    }

    async fn token(&self) -> Option<String> {
        self.0.secret_value(GITHUB_SECRET).await.ok().flatten().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
    }
}

/// A base image: what it's called, its name here, and where CI publishes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseImage {
    pub name: String,
    /// As wadspaces use it (`localhost/wados-base:44`).
    pub local: String,
    /// `ghcr.io/hunggod/wados-base:44`.
    pub published: String,
}

impl BaseImage {
    pub fn new(name: &str, local: &str) -> Self {
        Self { name: name.into(), local: local.into(), published: published(local) }
    }
}

/// Where CI publishes an image named `localhost/<name>` here (one from a registry already: itself).
pub fn published(local: &str) -> String {
    match local.strip_prefix("localhost/") {
        Some(rest) => format!("{PUBLISHED}{rest}"),
        None => local.to_string(),
    }
}

// ------------------------------------------------------------ the service

/// What the updates are told of, beyond their own state.
pub struct Settings {
    pub state_dir: PathBuf,
    pub auth_file: PathBuf,
    /// Between checks (and refreshes).
    pub every: Duration,
    /// The first look after wadd starts, when one's due (the machine settles first).
    pub first: Duration,
    /// How often the schedule looks at what's due.
    pub tick: Duration,
    /// Check and refresh by themselves (a machine's wadd; not a laptop's).
    pub schedule: bool,
    pub bases: Vec<BaseImage>,
}

impl Settings {
    pub fn new(config: &wad_config::Config, profile: wad_config::Profile) -> Self {
        Self {
            state_dir: config.daemon.state_dir.clone(),
            auth_file: AUTH_FILE.into(),
            every: EVERY,
            first: Duration::from_secs(300),
            tick: Duration::from_secs(600),
            schedule: profile == wad_config::Profile::System,
            bases: vec![
                BaseImage::new("Fedora 44 base", wad_gen::BASE_IMAGE),
                BaseImage::new("Stream sidecar", &config.streaming.image),
            ],
        }
    }
}

/// What happens beside: the bases changed (wadspaces built on the old one
/// show as updates; the disk changed).
#[derive(Default)]
pub struct Hooks {
    pub bases_changed: Option<Box<dyn Fn() + Send + Sync>>,
}

/// Kept in `<state_dir>/updates.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Saved {
    checked_at: Option<i64>,
    /// The digest of an update the person said not to apply.
    declined: Option<String>,
    /// When the bases were last refreshed (all of them).
    bases_at: Option<i64>,
    /// Each base's last pull, by its name here.
    refreshed: BTreeMap<String, i64>,
}

#[derive(Default)]
struct St {
    host: Option<Host>,
    saved: Saved,
    error: Option<String>,
    offline: bool,
    progress: Option<UpdateProgress>,
    job: Option<String>,
    checking: bool,
    bases_job: Option<String>,
    /// What the last check found.
    found: Option<Version>,
}

/// Cheap to clone: one service, shared.
#[derive(Clone)]
pub struct HostUpdates(Arc<Inner>);

struct Inner {
    st: Mutex<St>,
    bootc: Arc<dyn Bootc>,
    images: Arc<dyn Images>,
    jobs: Arc<Jobs>,
    bus: Bus,
    settings: Settings,
    hooks: Hooks,
    /// One bootc at a time (a check, a download, a rollback).
    lock: tokio::sync::Mutex<()>,
}

fn busy(what: &str, job: &str) -> ApiError {
    ApiError::conflict(format!("{what} is under way already ({job})"))
}

fn labelled(stages: &[(&str, &str)]) -> Vec<(String, String)> {
    stages.iter().map(|(s, l)| (s.to_string(), l.to_string())).collect()
}

/// Clears what a job held, however it ends (cancelled too).
struct Gone<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Drop for Gone<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}

impl HostUpdates {
    pub fn new(
        settings: Settings,
        bootc: Arc<dyn Bootc>,
        images: Arc<dyn Images>,
        jobs: Arc<Jobs>,
        bus: Bus,
        hooks: Hooks,
    ) -> Self {
        let saved = std::fs::read(settings.state_dir.join("updates.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let st = St { saved, ..St::default() };
        Self(Arc::new(Inner {
            st: Mutex::new(st),
            bootc,
            images,
            jobs,
            bus,
            settings,
            hooks,
            lock: tokio::sync::Mutex::new(()),
        }))
    }

    /// The schedule, for as long as wadd runs: bootc asked at once, then a
    /// check and a refresh of the bases when each is due (every 12 hours).
    pub fn start(&self) -> Vec<tokio::task::JoinHandle<()>> {
        let me = self.clone();
        vec![tokio::spawn(async move { me.run().await })]
    }

    async fn run(self) {
        if self.0.bootc.here() {
            if let Err(e) = self.0.refresh().await {
                tracing::warn!("updates: {e}");
            }
        } else {
            self.0.publish();
        }
        if !self.0.settings.schedule {
            return;
        }
        tokio::time::sleep(self.0.settings.first).await;
        loop {
            self.due().await;
            tokio::time::sleep(self.0.settings.tick).await;
        }
    }

    /// A check and a refresh, if they're due.
    pub async fn due(&self) {
        let (check, bases) = {
            let st = self.0.st.lock().unwrap();
            let every = self.0.settings.every.as_millis() as i64;
            let now = now_ms();
            let due = |at: Option<i64>| at.is_none_or(|t| now - t >= every);
            (due(st.saved.checked_at) && st.job.is_none(), due(st.saved.bases_at) && st.bases_job.is_none())
        };
        if check && self.0.bootc.here() {
            tracing::info!("updates: checking, as every 12 hours");
            if let Err(e) = self.check_job() {
                tracing::info!("updates: {}", e.message);
            }
        }
        if bases {
            tracing::info!("updates: refreshing the base images, as every 12 hours");
            if let Err(e) = self.refresh_job() {
                tracing::info!("updates: {}", e.message);
            }
        }
    }

    fn check_job(&self) -> Result<JobInfo, ApiError> {
        let me = &self.0;
        me.can_update()?;
        let job = {
            let mut st = me.st.lock().unwrap();
            if let Some(j) = &st.job {
                return Err(busy("An update", j));
            }
            let job =
                me.jobs.create(JobKind::Update, "Checking for updates", "wados", &labelled(proto::updates::CHECK));
            st.job = Some(job.clone());
            st.checking = true;
            job
        };
        me.publish();
        let (inner, id) = (me.clone(), job.clone());
        let task = tokio::spawn(async move {
            let gone = {
                let me = inner.clone();
                Gone(Some(move || {
                    let mut st = me.st.lock().unwrap();
                    st.checking = false;
                    st.job = None;
                    drop(st);
                    me.publish();
                }))
            };
            inner.jobs.running(&id);
            inner.jobs.stage(&id, "check", PartState::Working, None);
            let r = inner.check(&id).await;
            match &r {
                Ok(_) => inner.jobs.stage(&id, "check", PartState::Done, None),
                Err(e) => inner.jobs.stage(&id, "check", PartState::Error, Some(e.clone())),
            }
            drop(gone);
            let fetch = matches!(r, Ok(true));
            inner.jobs.finish(&id, r.map(|_| ()));
            // Found, and not turned down: downloaded now (the spike's check).
            if fetch && let Err(e) = HostUpdates(inner.clone()).apply_job() {
                tracing::info!("updates: {}", e.message);
            }
        });
        me.jobs.attach(&job, task.abort_handle());
        me.info(&job)
    }

    fn apply_job(&self) -> Result<JobInfo, ApiError> {
        let me = &self.0;
        me.can_update()?;
        let job = {
            let mut st = me.st.lock().unwrap();
            if let Some(j) = &st.job {
                return Err(busy("An update", j));
            }
            let next = me.view(&st).available.ok_or_else(|| ApiError::conflict("Nothing to apply: it's up to date"))?;
            let title = format!("Updating WadSpaces to {}", next.version);
            let job = me.jobs.create(JobKind::Update, &title, "wados", &labelled(proto::updates::APPLY));
            st.job = Some(job.clone());
            job
        };
        me.publish();
        let (inner, id) = (me.clone(), job.clone());
        let task = tokio::spawn(async move {
            let gone = {
                let me = inner.clone();
                Gone(Some(move || {
                    let mut st = me.st.lock().unwrap();
                    st.progress = None;
                    st.job = None;
                    drop(st);
                    me.publish();
                }))
            };
            inner.jobs.running(&id);
            let r = inner.apply(&id).await;
            drop(gone);
            inner.jobs.finish(&id, r.clone().map(|_| ()));
            if let Ok(v) = r {
                inner.notice(
                    Level::Ok,
                    format!("WadSpaces {} is ready", v.version),
                    "Restart to start it.",
                    "download",
                    Some(NoticeAction::new("Restart", "power.restart", &[])),
                    true,
                );
            }
        });
        me.jobs.attach(&job, task.abort_handle());
        me.info(&job)
    }

    fn refresh_job(&self) -> Result<JobInfo, ApiError> {
        let me = &self.0;
        let stages: Vec<(String, String)> =
            me.settings.bases.iter().enumerate().map(|(i, b)| (format!("base-{i}"), b.name.clone())).collect();
        let job = {
            let mut st = me.st.lock().unwrap();
            if let Some(j) = &st.bases_job {
                return Err(busy("A refresh", j));
            }
            let job = me.jobs.create(JobKind::Pull, "Refreshing the base images", "bases", &stages);
            st.bases_job = Some(job.clone());
            // Now, or by the schedule: the next is 12 hours from now.
            st.saved.bases_at = Some(now_ms());
            me.save(&st.saved);
            job
        };
        let (inner, id) = (me.clone(), job.clone());
        let task = tokio::spawn(async move {
            let gone = {
                let me = inner.clone();
                Gone(Some(move || me.st.lock().unwrap().bases_job = None))
            };
            inner.jobs.running(&id);
            let r = inner.refresh_bases(&id).await;
            drop(gone);
            inner.jobs.finish(&id, r);
            if let Some(f) = &inner.hooks.bases_changed {
                f();
            }
        });
        me.jobs.attach(&job, task.abort_handle());
        me.info(&job)
    }
}

impl Inner {
    fn info(&self, job: &str) -> Result<JobInfo, ApiError> {
        self.jobs.info(job).ok_or_else(|| ApiError::internal("the job went"))
    }

    fn save(&self, saved: &Saved) {
        let path = self.settings.state_dir.join("updates.json");
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(&self.settings.state_dir)?;
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(saved).unwrap_or_default())?;
            std::fs::rename(&tmp, &path)
        };
        if let Err(e) = write() {
            tracing::warn!("updates: {}: {e}", path.display());
        }
    }

    fn can_update(&self) -> Result<(), ApiError> {
        if self.bootc.here() { Ok(()) } else { Err(ApiError::conflict("This machine doesn't update through bootc")) }
    }

    /// The Updates as they are now.
    fn view(&self, st: &St) -> proto::Updates {
        let host = st.host.clone().unwrap_or_default();
        let booted = host.booted.clone().unwrap_or_else(|| Version { image: host.image.clone(), ..Version::default() });
        let staged = if host.rollback_queued { host.rollback.clone() } else { host.staged.clone() };
        let available = st
            .found
            .clone()
            .or_else(|| host.cached.clone())
            .filter(|c| !same(Some(c), Some(&booted)) && !same(Some(c), staged.as_ref()) && !host.rollback_queued);
        let error = match (&st.host, self.bootc.here()) {
            (_, false) => Some("This machine doesn't update through bootc".into()),
            _ => st.error.clone(),
        };
        proto::Updates {
            online: self.bootc.here() && host.transport == "registry" && !st.offline,
            checking: st.checking,
            booted,
            available,
            staged,
            rollback: host.rollback,
            error,
            checked_at: st.saved.checked_at,
            progress: st.progress,
            job: st.job.clone(),
        }
    }

    fn now(&self) -> proto::Updates {
        self.view(&self.st.lock().unwrap())
    }

    fn publish(&self) -> proto::Updates {
        let u = self.now();
        self.bus.publish(Event::Updates(Box::new(u.clone())));
        u
    }

    fn notice(&self, level: Level, title: String, body: &str, icon: &str, action: Option<NoticeAction>, toast: bool) {
        self.bus.publish(Event::Notice {
            level,
            title,
            undo: None,
            body: Some(body.into()),
            icon: Some(icon.into()),
            action,
            toast,
        });
    }

    /// bootc, quietly: its output; each line to the job's log when there's one.
    async fn bootc(&self, job: Option<&str>, argv: &[&str]) -> Res<String> {
        let (jobs, id) = (self.jobs.clone(), job.map(String::from));
        if let Some(id) = &id {
            jobs.line(id, &format!("$ {}", argv.join(" ")));
        }
        let on_line: OnLine = Box::new(move |l| {
            if let Some(id) = &id
                && progress_of(l).is_none()
                && !l.trim().is_empty()
            {
                jobs.line(id, l);
            }
        });
        self.bootc.run(argv, on_line).await
    }

    /// bootc's status asked again, and told.
    async fn refresh(&self) -> Res<proto::Updates> {
        let text = self.bootc(None, &["bootc", "status", "--json"]).await?;
        let host = parse_status(&text)?;
        self.st.lock().unwrap().host = Some(host);
        Ok(self.publish())
    }

    /// The registry's login, from the GitHub token, for an image on ghcr.io:
    /// true if it's there now.
    async fn sign_in(&self) -> bool {
        let image = self.st.lock().unwrap().host.as_ref().map(|h| h.image.clone()).unwrap_or_default();
        if !image.starts_with(&format!("{GHCR}/")) {
            return false;
        }
        let Some(token) = self.images.token().await else { return false };
        match write_private(&self.settings.auth_file, &auth_json(&token)) {
            Ok(()) => {
                tracing::info!("updates: signed in to {GHCR} with this machine's GitHub token");
                true
            }
            Err(e) => {
                tracing::warn!("updates: {}: {e}", self.settings.auth_file.display());
                false
            }
        }
    }

    /// bootc `argv`, tried again once signed in when the registry refused it.
    async fn with_login(&self, job: &str, argv: &[&str]) -> Res<String> {
        match self.bootc(Some(job), argv).await {
            Err(e) if refused(&e) && self.sign_in().await => {
                self.jobs.line(job, "» the registry wants a login: with this machine's GitHub token");
                self.bootc(Some(job), argv).await
            }
            r => r,
        }
    }

    /// What a failure leaves said (offline: the registry wasn't reached).
    fn failed(&self, e: &str) {
        let mut st = self.st.lock().unwrap();
        st.offline = offline(e);
        st.error = Some(e.to_string());
    }

    /// The check: true if it found one to download (not turned down).
    async fn check(&self, job: &str) -> Res<bool> {
        let _one = self.lock.lock().await;
        // What it follows (whose registry it signs in to), if it isn't known yet.
        if self.st.lock().unwrap().host.is_none() {
            self.refresh().await?;
        }
        let r = self.with_login(job, &["bootc", "upgrade", "--check"]).await;
        {
            let mut st = self.st.lock().unwrap();
            st.saved.checked_at = Some(now_ms());
            self.save(&st.saved);
        }
        let out = match r {
            Ok(out) => out,
            Err(e) => {
                self.failed(&e);
                self.publish();
                return Err(e);
            }
        };
        {
            let mut st = self.st.lock().unwrap();
            st.offline = false;
            st.error = None;
            st.found = parse_check(&out);
        }
        let u = self.refresh().await?;
        let declined = self.st.lock().unwrap().saved.declined.clone();
        Ok(u.available.is_some_and(|a| a.digest != declined))
    }

    /// The download and staging: what's staged.
    async fn apply(self: &Arc<Self>, job: &str) -> Res<Version> {
        let _one = self.lock.lock().await;
        self.jobs.stage(job, "download", PartState::Working, None);
        let (jobs, id, me) = (self.jobs.clone(), job.to_string(), self.clone());
        let staging = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let on = {
            let staging = staging.clone();
            move |l: &str| match progress_of(l) {
                Some(Progress::Bytes { done, total }) if total > 0 => {
                    jobs.bytes(&id, "download", done, total);
                    me.set_progress(Some(UpdateProgress { done_bytes: done, total_bytes: total }));
                }
                Some(Progress::Steps { task, done, total }) if task != "pulling" => {
                    if !staging.swap(true, Ordering::Relaxed) {
                        jobs.stage(&id, "download", PartState::Done, None);
                        jobs.stage(&id, "stage", PartState::Working, None);
                        me.set_progress(None);
                    }
                    if total > 0 {
                        jobs.progress(&id, "stage", done as f64 / total as f64);
                    }
                }
                _ => {}
            }
        };
        let r = {
            let (jobs, id) = (self.jobs.clone(), job.to_string());
            let on_line: OnLine = Box::new(move |l| {
                on(l);
                if progress_of(l).is_none() && !l.trim().is_empty() {
                    jobs.line(&id, l);
                }
            });
            self.jobs.line(job, "$ bootc upgrade");
            let argv = UPGRADE;
            match self.bootc.run(argv, on_line).await {
                Err(e) if refused(&e) && self.sign_in().await => {
                    self.jobs.line(job, "» the registry wants a login: with this machine's GitHub token");
                    self.bootc(Some(job), argv).await
                }
                r => r,
            }
        };
        if let Err(e) = r {
            self.failed(&e);
            self.publish();
            return Err(e);
        }
        if !staging.load(Ordering::Relaxed) {
            self.jobs.stage(job, "download", PartState::Done, None);
            self.jobs.stage(job, "stage", PartState::Working, None);
        }
        {
            let mut st = self.st.lock().unwrap();
            st.error = None;
            st.offline = false;
            st.progress = None;
            st.saved.declined = None;
            self.save(&st.saved);
        }
        let u = self.refresh().await?;
        self.jobs.stage(job, "stage", PartState::Done, None);
        // Nothing staged: bootc found nothing newer after all.
        u.staged.ok_or_else(|| "bootc staged nothing: this machine is up to date".to_string())
    }

    fn set_progress(&self, p: Option<UpdateProgress>) {
        self.st.lock().unwrap().progress = p;
        self.publish();
    }

    async fn refresh_bases(&self, job: &str) -> Result<(), String> {
        let token = self.images.token().await;
        let mut failed = vec![];
        for (i, b) in self.settings.bases.iter().enumerate() {
            let stage = format!("base-{i}");
            self.jobs.stage(job, &stage, PartState::Working, None);
            self.jobs.line(job, &format!("» {}: {}", b.name, b.published));
            let auth = token.as_ref().filter(|_| b.published.starts_with(&format!("{GHCR}/")));
            let auth = auth.map(|t| wad_podman::registry_auth(REGISTRY_USER, t, GHCR));
            let (jobs, id, st) = (self.jobs.clone(), job.to_string(), stage.clone());
            let on: OnBytes = Box::new(move |d, t| jobs.bytes(&id, &st, d, t));
            let r = match self.images.pull_newer(&b.published, auth, on).await {
                Ok(id) if b.local != b.published => self.images.tag(&id, &b.local).await,
                Ok(_) => Ok(()),
                Err(e) => Err(e),
            };
            match r {
                Ok(()) => {
                    let mut st = self.st.lock().unwrap();
                    st.saved.refreshed.insert(b.local.clone(), now_ms());
                    self.save(&st.saved);
                    drop(st);
                    self.jobs.stage(job, &stage, PartState::Done, None);
                }
                Err(e) => {
                    self.jobs.line(job, &format!("✗ {}: {e}", b.name));
                    self.jobs.stage(job, &stage, PartState::Error, Some(e.clone()));
                    failed.push(format!("{}: {e}", b.name));
                }
            }
        }
        if failed.is_empty() { Ok(()) } else { Err(failed.join("; ")) }
    }

    async fn bases(&self) -> Bases {
        let (saved, schedule) = (self.st.lock().unwrap().saved.clone(), self.settings.schedule);
        let mut bases = vec![];
        for b in &self.settings.bases {
            let info = self.images.info(&b.local).await.ok().flatten();
            let pulled = saved.refreshed.get(&b.local).copied();
            bases.push(Base {
                name: b.name.clone(),
                image: b.published.clone(),
                size: info.as_ref().map(|i| i.size),
                refreshed_at: info
                    .as_ref()
                    .and_then(|i| pulled.or((i.created > 0.0).then_some((i.created * 1000.0) as i64))),
                missing: info.is_none(),
            });
        }
        let every = self.settings.every.as_millis() as i64;
        Bases {
            bases,
            next_refresh_at: schedule
                .then(|| saved.bases_at.map_or(now_ms() + self.settings.first.as_millis() as i64, |t| t + every)),
        }
    }
}

#[async_trait]
impl crate::api_phase8::Updates for HostUpdates {
    async fn status(&self) -> Result<proto::Updates, ApiError> {
        let me = &self.0;
        let asked = me.st.lock().unwrap().host.is_some();
        if !asked && me.bootc.here() {
            me.refresh().await.map_err(|e| ApiError::new(ErrorCode::Internal, e))?;
        }
        Ok(me.now())
    }

    async fn check(&self) -> Result<JobInfo, ApiError> {
        self.check_job()
    }

    async fn apply(&self) -> Result<JobInfo, ApiError> {
        self.apply_job()
    }

    async fn cancel(&self) -> Result<proto::Updates, ApiError> {
        let me = &self.0;
        me.can_update()?;
        let u = me.now();
        let was = u.staged.clone().ok_or_else(|| ApiError::conflict("Nothing is staged"))?;
        if let Some(j) = &u.job {
            return Err(busy("An update", j));
        }
        let queued = me.st.lock().unwrap().host.as_ref().is_some_and(|h| h.rollback_queued);
        {
            let _one = me.lock.lock().await;
            let argv: &[&str] = if queued { &["bootc", "rollback"] } else { UNDEPLOY_STAGED };
            me.bootc(None, argv).await.map_err(|e| ApiError::new(ErrorCode::Internal, e))?;
            if !queued {
                let mut st = me.st.lock().unwrap();
                st.saved.declined = was.digest.clone();
                me.save(&st.saved);
            }
        }
        let u = me.refresh().await.map_err(|e| ApiError::new(ErrorCode::Internal, e))?;
        let title = format!("WadSpaces {} won't be applied", was.version);
        me.notice(Level::Info, title, &format!("This machine stays on {}.", u.booted.version), "x", None, false);
        Ok(u)
    }

    async fn rollback(&self) -> Result<proto::Updates, ApiError> {
        let me = &self.0;
        me.can_update()?;
        let u = me.now();
        let back = match &u.rollback {
            Some(r) if !same(u.staged.as_ref(), Some(r)) => r.clone(),
            Some(_) => return Err(ApiError::conflict("It's staged already")),
            None => return Err(ApiError::conflict("There's nothing to go back to")),
        };
        // A check or download under way would stage over it: stopped.
        if let Some(j) = &u.job {
            let _ = me.jobs.cancel(j);
        }
        {
            let _one = me.lock.lock().await;
            me.bootc(None, &["bootc", "rollback"]).await.map_err(|e| ApiError::new(ErrorCode::Internal, e))?;
            me.st.lock().unwrap().error = None;
        }
        let u = me.refresh().await.map_err(|e| ApiError::new(ErrorCode::Internal, e))?;
        me.notice(
            Level::Info,
            format!("WadSpaces {} is staged", back.version),
            "Restart to go back to it.",
            "rotate-ccw",
            Some(NoticeAction::new("Restart", "power.restart", &[])),
            true,
        );
        Ok(u)
    }

    async fn bases(&self) -> Result<Bases, ApiError> {
        Ok(self.0.bases().await)
    }

    async fn refresh_bases(&self) -> Result<JobInfo, ApiError> {
        self.refresh_job()
    }
}

#[cfg(test)]
mod tests;
