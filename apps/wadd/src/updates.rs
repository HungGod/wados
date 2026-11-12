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
//! Every [`EVERY`] (6 hours, from the last check) wadd checks by itself, on
//! a machine's wadd only (`Settings::schedule`); restarting is always the
//! person's. The base images are looked at every [`BASES_EVERY`] (a day).
//!
//! **Signing in to the registry.** The host image on ghcr is public, so
//! bootc needs no login: every check and download starts without one. A
//! private one's that's refused (401, denied) is tried again once with this
//! machine's GitHub token (the podman secret
//! `github_token`, phase 6) written as ostree's login,
//! `/run/ostree/auth.json` (0600, root's, in `/run` only: never in the
//! image and never on the disk), for ghcr.io only. A login that didn't
//! help is removed (ghcr refuses one a token can't use, even for a public
//! image).
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
//! a stage each: the published one pulled only if it was built later than
//! the copy here (its config's time, read from the registry without
//! downloading it; with the GitHub token, if there's one) and named as
//! wadspaces use it. One here that's as new, or newer (a stick carries the
//! base its builder made), is kept. A wadspace built on the base before
//! shows as an update and rebuilds when it next opens (`Builds::refresh_base`,
//! through `Hooks::bases_changed`). Looked at once a day on a machine (the
//! first a few minutes after its first start), quietly: the job, and the
//! download, only when a newer one's published.
//!
//! **Where a newer WadOS comes from** ([`UpdateSource`]). Each source names
//! an image in the machine's own family (`wados-host`, or
//! `wados-host-nvidia` on an NVIDIA machine):
//!
//! - **GitHub's** (and **pushed**, the WadOS wadspace's push to the same
//!   tag): `ghcr.io/hunggod/<family>:latest`. While bootc follows it, the
//!   check and download are bootc's (`upgrade`). While it follows something
//!   else (this machine's build), bootc's check would be of that, so wadd
//!   reads the registry itself ([`registry_version`]: the manifest's digest
//!   and the version label) and the update is a `bootc switch` back to it.
//! - **This machine's builds**: the newest `localhost/<family>` by its
//!   version label, in the machine's podman store or in a running
//!   wadspace's own (the WadOS wadspace's rootless podman, which
//!   `host/build.sh image` builds into). One in a wadspace is copied out
//!   first (`podman save` inside it, in its user namespace, so the files
//!   keep their owners; `podman load` here), then `bootc switch
//!   --transport containers-storage <name>@<image id>`. Only a build newer
//!   than what's booted is offered.
//!
//! A check downloads what it found by itself only when it's newer than
//! what's booted: going back to an older image is the person's to choose.
//! A check counts as a look whether it worked or not (the next is
//! [`EVERY`] later), and the source can't change while an update runs.
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
use wad_proto::{
    ApiError, Base, Bases, ErrorCode, Event, JobInfo, Level, NoticeAction, UpdateProgress, UpdateSource, Version,
};

use crate::backend::{ImageInfo, OnBytes, OnLine, Res};
use crate::events::Bus;
use crate::jobs::{Jobs, now_ms};
use crate::packages::GITHUB_SECRET;

/// The user name a registry sees: ghcr checks the token, not the name.
const REGISTRY_USER: &str = "wadspaces";

/// Between checks of WadOS's own image (bootc's manifest: cheap).
pub const EVERY: Duration = Duration::from_secs(6 * 3600);
/// Between refreshes of the base images: a manifest each, and bytes when
/// one moved, so it's a rarer look at the registry than the host's check.
pub const BASES_EVERY: Duration = Duration::from_secs(24 * 3600);
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
/// The image name that's a host image's.
const HOST_IMAGE: &str = "wados-host";
const NVIDIA_IMAGE: &str = "wados-host-nvidia";
/// This machine's own builds of it (`host/build.sh image`), and the NVIDIA one's.
const LOCAL_HOST_IMAGE: &str = "localhost/wados-host";

/// Which host image `host` runs: NVIDIA's (`wados-host-nvidia`), else the
/// plain one. A machine stays in its family whatever the source.
fn family(host: &Host) -> &'static str {
    let nvidia = |i: &str| i.contains(NVIDIA_IMAGE);
    if nvidia(&host.image) || host.booted.as_ref().is_some_and(|b| nvidia(&b.image)) {
        NVIDIA_IMAGE
    } else {
        HOST_IMAGE
    }
}

/// Where GitHub's CI (and the WadOS wadspace's push) publishes `host`'s image.
fn registry_target(host: &Host) -> String {
    format!("{PUBLISHED}{}:latest", family(host))
}

/// bootc follows `target` from its registry (so `bootc upgrade` updates it),
/// not this machine's build or another image.
fn follows(host: &Host, target: &str) -> bool {
    host.transport == "registry" && host.image == target
}

/// The repository of a tag (`localhost/wados-host` of `localhost/wados-host:44.1`).
fn repo_of(tag: &str) -> &str {
    match tag.rsplit_once(':') {
        Some((r, t)) if !t.contains('/') => r,
        _ => tag,
    }
}

/// The newest of this machine's builds of `family` (`wados-host`, or the
/// NVIDIA one: never another image whose name starts the same, such as
/// `wados-host-builder`), by its `org.opencontainers.image.version` (else
/// its tag): the version, its tag and image id, and the wadspace whose own
/// podman has it (none: the machine's store, which wins a tie).
fn newest_local(family: &str, images: &[(Option<String>, ImageInfo)]) -> Option<(Version, Option<String>)> {
    let repo = format!("localhost/{family}");
    let mut best: Option<(Version, Option<String>)> = None;
    for (from, i) in images {
        let Some(tag) = i.tags.iter().find(|t| repo_of(t) == repo) else { continue };
        let version =
            i.labels.get("org.opencontainers.image.version").and_then(Value::as_str).map(String::from).or_else(|| {
                i.tags
                    .iter()
                    .filter(|t| repo_of(t) == repo)
                    .find_map(|t| t.strip_prefix(&format!("{repo}:")).filter(|v| *v != "latest").map(String::from))
            });
        let Some(version) = version else { continue };
        let v = Version { version, image: tag.clone(), digest: Some(i.id.trim_start_matches("sha256:").to_string()) };
        let better = match &best {
            None => true,
            Some((b, b_from)) => match compare_versions(&v.version, &b.version) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Equal => b_from.is_some() && from.is_none(),
                std::cmp::Ordering::Less => false,
            },
        };
        if better {
            best = Some((v, from.clone()));
        }
    }
    best
}

/// CI's versions (`44.20261105.2`): ones that can be told newer or older.
fn ci_version(v: &str) -> bool {
    v.split('.').count() >= 2 && v.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// `a` is newer than `b`, by their versions; when either isn't CI's kind
/// (a digest for a version), it can't be told, so it counts as newer.
fn newer(a: &Version, b: &Version) -> bool {
    if ci_version(&a.version) && ci_version(&b.version) {
        compare_versions(&a.version, &b.version).is_gt()
    } else {
        true
    }
}

/// The host image with NVIDIA's driver beside `booted` (same registry and
/// tag), if `booted` is the plain one.
pub fn nvidia_image(booted: &str) -> Option<String> {
    if booted.contains(NVIDIA_IMAGE) || !booted.contains(HOST_IMAGE) {
        return None;
    }
    Some(booted.replacen(HOST_IMAGE, NVIDIA_IMAGE, 1))
}

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
    "dns error",
    "error sending request",
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

/// Whether two are the same one: the same digest, else the same version
/// (a local image has an image id for its digest, not the registry's).
fn same_version(a: Option<&Version>, b: Option<&Version>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.digest == b.digest || (!a.version.is_empty() && a.version == b.version),
        _ => false,
    }
}

/// Versions as CI names them (`44.20261105.2`: the day, then the build that
/// day), compared part by part, numbers as numbers.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |v: &str| -> Vec<String> { v.split(&['.', '-', '+'][..]).map(String::from).collect() };
    let (a, b) = (parts(a), parts(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).map(String::as_str).unwrap_or(""), b.get(i).map(String::as_str).unwrap_or(""));
        let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            _ => x.cmp(y),
        };
        if ord != std::cmp::Ordering::Equal {
            return ord;
        }
    }
    std::cmp::Ordering::Equal
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
    /// Every image here, as podman lists them (a WadOS wadspace's builds are
    /// `localhost/wados-host*`).
    async fn all(&self) -> Res<Vec<ImageInfo>>;
    /// The host images built inside the wadspaces running here (a WadOS
    /// wadspace's own podman: rootless, in its /config): (its container, the
    /// image as that podman lists it).
    async fn built_in_wadspaces(&self) -> Res<Vec<(String, ImageInfo)>>;
    /// One of those copied into the machine's store, where bootc reads it:
    /// `podman save` inside the wadspace (its own user namespace, so the
    /// files keep their owners), `podman load` here. Its id stays the same.
    async fn import(&self, container: &str, id: &str, on_line: OnLine) -> Res<()>;
    /// What a registry has at `reference` (its version label and manifest
    /// digest), without downloading it: for a source bootc doesn't follow.
    async fn remote(&self, reference: &str, token: Option<String>) -> Res<Version>;
    /// When the registry's image at `reference` was built (Unix seconds,
    /// its config's `created`), without downloading it: a base's refresh
    /// downloads only one newer than the copy here.
    async fn published_created(&self, reference: &str, token: Option<String>) -> Res<f64>;
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

    async fn all(&self) -> Res<Vec<ImageInfo>> {
        self.0.images_all().await.map_err(|e| e.to_string())
    }

    async fn built_in_wadspaces(&self) -> Res<Vec<(String, ImageInfo)>> {
        // Running ones only (a stopped wadspace's podman can't be asked).
        let running = self.0.containers_labelled(wad_gen::labels::WADSPACE).await.map_err(|e| e.to_string())?;
        let mut out = vec![];
        for c in running {
            let Some(id) = c.labels.get(wad_gen::labels::WADSPACE).and_then(Value::as_str) else { continue };
            // The wadspace itself, not its stream's sidecar.
            if c.name != wad_gen::units::container_name(id) {
                continue;
            }
            let list = format!(
                "command -v podman > /dev/null || exit 0; exec podman images --format json --filter 'reference={}*'",
                LOCAL_HOST_IMAGE
            );
            let r = self.0.exec(&c.name, "abc", &["sh", "-c", &list], &[("HOME", "/config")]).await;
            match r {
                Ok(o) if o.code == 0 => {
                    let text = String::from_utf8_lossy(&o.stdout);
                    out.extend(parse_podman_images(&text).into_iter().map(|i| (c.name.clone(), i)));
                }
                Ok(o) => tracing::info!("updates: {}'s podman: {}", c.name, o.stderr.trim()),
                Err(e) => tracing::info!("updates: {}'s podman: {e}", c.name),
            }
        }
        Ok(out)
    }

    async fn import(&self, container: &str, id: &str, mut on_line: OnLine) -> Res<()> {
        if !container.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            || !id.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(format!("not an image to copy: {container} {id}"));
        }
        let script = r#"set -euo pipefail
podman exec --user abc -e HOME=/config "$1" podman save --format docker-archive "$2" | podman load"#;
        outside(&["bash", "-c", script, "import", container, id], &mut |l| on_line(l)).await.map(drop)
    }

    async fn remote(&self, reference: &str, token: Option<String>) -> Res<Version> {
        registry_version(reference, token.as_deref()).await
    }

    async fn published_created(&self, reference: &str, token: Option<String>) -> Res<f64> {
        registry_created(reference, token.as_deref()).await
    }
}

/// `podman images --format json`, as the CLI prints it: what each image is.
fn parse_podman_images(text: &str) -> Vec<ImageInfo> {
    let list: Vec<Value> = serde_json::from_str(text.trim()).unwrap_or_default();
    list.iter()
        .filter_map(|i| {
            let id = i.get("Id").and_then(Value::as_str)?.trim_start_matches("sha256:").to_string();
            let tags = i
                .get("Names")
                .and_then(Value::as_array)
                .map(|n| n.iter().filter_map(Value::as_str).map(String::from).collect())
                .unwrap_or_default();
            Some(ImageInfo {
                id,
                created: i.get("Created").and_then(Value::as_f64).unwrap_or(0.0),
                size: i.get("Size").and_then(Value::as_u64).unwrap_or(0),
                labels: i.get("Labels").and_then(Value::as_object).cloned().unwrap_or_default(),
                tags,
                parent: String::new(),
            })
        })
        .collect()
}

/// The platform's name in an image index (`amd64`, `arm64`).
fn oci_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    }
}

/// Manifests and indexes, as registries answer for them.
const MANIFEST_TYPES: &str = "application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json, \
     application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json";

/// What a registry has at `reference`: the version its config's
/// `org.opencontainers.image.version` says, and its manifest's digest (this
/// platform's, for a list), as bootc reports a deployment's. The registry's
/// own token (OCI distribution's Bearer challenge): anonymous, or with
/// `token` (the GitHub token, for ghcr) when it's given.
pub async fn registry_version(reference: &str, token: Option<&str>) -> Res<Version> {
    let (digest, config) = registry_config(reference, token).await?;
    let version = config
        .pointer("/config/Labels/org.opencontainers.image.version")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| digest.trim_start_matches("sha256:").chars().take(12).collect());
    Ok(Version { version, image: reference.to_string(), digest: Some(digest) })
}

/// When the registry's image at `reference` was built: its config's
/// `created` (RFC 3339), in Unix seconds.
pub async fn registry_created(reference: &str, token: Option<&str>) -> Res<f64> {
    let (_, config) = registry_config(reference, token).await?;
    let created = config.get("created").and_then(Value::as_str).ok_or_else(|| format!("{reference}: no build time"))?;
    created_secs(created).ok_or_else(|| format!("{reference}: its build time {created:?} isn't one"))
}

/// An RFC 3339 time (`2026-11-11T03:41:59.028043743Z`) in Unix seconds.
fn created_secs(s: &str) -> Option<f64> {
    let t = chrono::DateTime::parse_from_rfc3339(s).ok()?;
    Some(t.timestamp() as f64 + f64::from(t.timestamp_subsec_nanos()) / 1e9)
}

/// The registry's manifest digest for `reference` (this platform's) and its config.
async fn registry_config(reference: &str, token: Option<&str>) -> Res<(String, Value)> {
    let (host, rest) = reference.split_once('/').ok_or_else(|| format!("{reference}: no registry in it"))?;
    let (repo, tag) = match rest.rsplit_once(':') {
        Some((r, t)) if !t.contains('/') => (r, t),
        _ => (rest, "latest"),
    };
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("couldn't reach {host}: {e}"))?;
    let base = format!("https://{host}/v2/{repo}");
    let mut bearer: Option<String> = None;
    // A GET, with the registry's token once it asks for one.
    async fn get(
        http: &reqwest::Client,
        url: &str,
        accept: &str,
        bearer: &mut Option<String>,
        login: Option<&str>,
    ) -> Res<reqwest::Response> {
        for _ in 0..2 {
            let mut req = http.get(url).header(reqwest::header::ACCEPT, accept);
            if let Some(b) = bearer.as_deref() {
                req = req.bearer_auth(b);
            }
            let res = req.send().await.map_err(|e| format!("couldn't reach {url}: {e}"))?;
            if res.status() != reqwest::StatusCode::UNAUTHORIZED || bearer.is_some() {
                if !res.status().is_success() {
                    return Err(format!("{url}: {}", res.status()));
                }
                return Ok(res);
            }
            // `Bearer realm="…",service="…",scope="…"`: its token, then again.
            let challenge = res
                .headers()
                .get(reqwest::header::WWW_AUTHENTICATE)
                .and_then(|h| h.to_str().ok())
                .unwrap_or_default()
                .to_string();
            let field = |k: &str| {
                challenge.split(',').find_map(|p| {
                    let (key, v) = p.trim().trim_start_matches("Bearer ").split_once('=')?;
                    (key.trim() == k).then(|| v.trim().trim_matches('"').to_string())
                })
            };
            let realm = field("realm").ok_or_else(|| format!("{url}: 401 Unauthorized"))?;
            let mut q = vec![];
            for k in ["service", "scope"] {
                if let Some(v) = field(k) {
                    q.push((k, v));
                }
            }
            let mut t = http.get(&realm).query(&q);
            if let Some(l) = login {
                t = t.basic_auth(REGISTRY_USER, Some(l));
            }
            let t = t.send().await.map_err(|e| format!("couldn't reach {realm}: {e}"))?;
            if !t.status().is_success() {
                return Err(format!("{realm}: {} (unauthorized)", t.status()));
            }
            let v: Value = t.json().await.map_err(|e| format!("{realm}: {e}"))?;
            let got = v.get("token").or_else(|| v.get("access_token")).and_then(Value::as_str);
            *bearer = Some(got.ok_or_else(|| format!("{realm}: no token"))?.to_string());
        }
        Err(format!("{url}: 401 Unauthorized"))
    }
    let digest_of = |res: &reqwest::Response| {
        res.headers().get("docker-content-digest").and_then(|h| h.to_str().ok()).map(String::from)
    };
    let res = get(&http, &format!("{base}/manifests/{tag}"), MANIFEST_TYPES, &mut bearer, token).await?;
    let mut digest = digest_of(&res);
    let mut manifest: Value = res.json().await.map_err(|e| format!("{reference}'s manifest: {e}"))?;
    // A list: this platform's.
    if let Some(list) = manifest.get("manifests").and_then(Value::as_array) {
        let mine = list
            .iter()
            .find(|m| m.pointer("/platform/architecture").and_then(Value::as_str) == Some(oci_arch()))
            .and_then(|m| m.get("digest").and_then(Value::as_str))
            .ok_or_else(|| format!("{reference} has no image for {}", oci_arch()))?
            .to_string();
        let res = get(&http, &format!("{base}/manifests/{mine}"), MANIFEST_TYPES, &mut bearer, token).await?;
        digest = Some(digest_of(&res).unwrap_or(mine));
        manifest = res.json().await.map_err(|e| format!("{reference}'s manifest: {e}"))?;
    }
    let digest = digest.ok_or_else(|| format!("{reference}: the registry didn't say its digest"))?;
    let config = manifest
        .pointer("/config/digest")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{reference}'s manifest has no config"))?;
    let res = get(&http, &format!("{base}/blobs/{config}"), "*/*", &mut bearer, token).await?;
    let config: Value = res.json().await.map_err(|e| format!("{reference}'s config: {e}"))?;
    Ok((digest, config))
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
    /// Between refreshes of the base images (a superset of `every`'s checks).
    pub bases_every: Duration,
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
            bases_every: BASES_EVERY,
            first: Duration::from_secs(300),
            // Well under EVERY, so a due check runs close to when it's due.
            tick: Duration::from_secs(60),
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
    /// Where a newer WadOS is looked for ([`UpdateSource`]; GitHub's until chosen).
    source: UpdateSource,
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
    /// The newest image this machine built (a WadOS wadspace's), as last
    /// looked for: what "This machine's builds" would offer.
    local: Option<Version>,
    /// The running wadspace whose own podman has `local` (none: the
    /// machine's store has it): it's copied out before bootc switches.
    local_in: Option<String>,
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
    /// check, and a refresh of the bases, when each is due.
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
            let bases_every = self.0.settings.bases_every.as_millis() as i64;
            let now = now_ms();
            let due = |at: Option<i64>, every: i64| at.is_none_or(|t| now - t >= every);
            (
                due(st.saved.checked_at, every) && st.job.is_none(),
                due(st.saved.bases_at, bases_every) && st.bases_job.is_none(),
            )
        };
        if check && self.0.bootc.here() {
            tracing::info!("updates: checking, as every {} h", self.0.settings.every.as_secs() / 3600);
            if let Err(e) = self.check_job() {
                tracing::info!("updates: {}", e.message);
            }
        }
        if bases {
            // Looked at quietly: a refresh (a job in Activity, and gigabytes)
            // only when one's newer where it's published, or missing here.
            // Up to date, or unreadable (offline): nothing to show, and the
            // next look is a day from now.
            {
                let mut st = self.0.st.lock().unwrap();
                st.saved.bases_at = Some(now_ms());
                self.0.save(&st.saved);
            }
            match self.0.any_base_newer().await {
                Ok(true) => {
                    tracing::info!("updates: a newer base image is published: refreshing");
                    if let Err(e) = self.refresh_job() {
                        tracing::info!("updates: {}", e.message);
                    }
                }
                Ok(false) => tracing::info!("updates: the base images here are up to date"),
                Err(e) => tracing::info!("updates: the base images weren't looked at: {e}"),
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

    /// Moves to the image with NVIDIA's driver: staged for the next start,
    /// as an update is, and later updates come from it.
    fn nvidia_job(&self, image: String) -> Result<JobInfo, ApiError> {
        let me = &self.0;
        me.can_update()?;
        let job = {
            let mut st = me.st.lock().unwrap();
            if let Some(j) = &st.job {
                return Err(busy("An update", j));
            }
            let job = me.jobs.create(
                JobKind::Update,
                "Installing NVIDIA's graphics image",
                "wados",
                &labelled(proto::updates::APPLY),
            );
            st.job = Some(job.clone());
            job
        };
        me.publish();
        let (inner, id) = (me.clone(), job.clone());
        let task = tokio::spawn(async move {
            let gone = {
                let me = inner.clone();
                Gone(Some(move || {
                    me.st.lock().unwrap().job = None;
                    me.publish();
                }))
            };
            inner.jobs.running(&id);
            let r = inner.switch(&id, &image).await;
            drop(gone);
            inner.jobs.finish(&id, r.clone());
            if r.is_ok() {
                inner.notice(
                    Level::Ok,
                    "NVIDIA's graphics image is ready".to_string(),
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
            // Now, or by the schedule: the next is a day from now.
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
        // What was found for the chosen source. GitHub's also has bootc's own
        // cached update, while bootc follows GitHub's image (one it cached
        // for this machine's build would be nothing). This machine's builds
        // are found by their label, and only a newer one is offered: an
        // older build is never an update.
        let source = st.saved.source;
        let found = match source {
            UpdateSource::Github if follows(&host, &registry_target(&host)) => {
                st.found.clone().or_else(|| host.cached.clone())
            }
            UpdateSource::Github | UpdateSource::Pushed => st.found.clone(),
            UpdateSource::Local => st.local.clone().filter(|l| newer(l, &booted)),
        };
        let available = found.filter(|c| {
            !same_version(Some(c), Some(&booted)) && !same_version(Some(c), staged.as_ref()) && !host.rollback_queued
        });
        let error = match (&st.host, self.bootc.here()) {
            (_, false) => Some("This machine doesn't update through bootc".into()),
            _ => st.error.clone(),
        };
        proto::Updates {
            // GitHub's (and the pushed) image is looked for on its registry,
            // whatever bootc follows now; this machine's own builds need no
            // registry, so that source is online when there's one.
            online: (self.bootc.here()
                && !st.offline
                && (host.transport == "registry" || source != UpdateSource::Local))
                || (source == UpdateSource::Local && st.local.is_some()),
            checking: st.checking,
            booted,
            available,
            staged,
            rollback: host.rollback,
            error,
            checked_at: st.saved.checked_at,
            progress: st.progress,
            job: st.job.clone(),
            source: st.saved.source,
            local: st.local.clone(),
            // What a look at what this machine pushed found (the same image
            // family as GitHub's, from the wadspace's tag).
            pushed: (st.saved.source == UpdateSource::Pushed).then(|| st.found.clone()).flatten(),
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

    /// The registry's login gone: each try starts without one. A public image
    /// needs none, and ghcr refuses a login it can't use (a GitHub token
    /// without read:packages) even for a public one, so one left from an
    /// image that was private once would refuse every check after.
    fn sign_out(&self) {
        if let Err(e) = std::fs::remove_file(&self.settings.auth_file)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("updates: {}: {e}", self.settings.auth_file.display());
        }
    }

    /// After a try signed in: a login that didn't help isn't kept.
    fn signed_in_answer(&self, r: Res<String>) -> Res<String> {
        if matches!(&r, Err(e) if refused(e)) {
            self.sign_out();
        }
        r
    }

    /// bootc `argv`, without a login first, then signed in if the registry refused it.
    async fn with_login(&self, job: &str, argv: &[&str]) -> Res<String> {
        self.sign_out();
        match self.bootc(Some(job), argv).await {
            Err(e) if refused(&e) && self.sign_in().await => {
                self.jobs.line(job, "» the registry wants a login: with this machine's GitHub token");
                let r = self.bootc(Some(job), argv).await;
                self.signed_in_answer(r)
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

    /// The check: true if it found one to download by itself (newer than
    /// what's booted, and not turned down).
    async fn check(&self, job: &str) -> Res<bool> {
        let _one = self.lock.lock().await;
        // What it follows (whose registry it signs in to), if it isn't known yet.
        if self.st.lock().unwrap().host.is_none() {
            self.refresh().await?;
        }
        // Looked now, whatever comes of it: a check that fails waits for the
        // next one as one that worked does (not every tick).
        let (source, host) = {
            let mut st = self.st.lock().unwrap();
            st.saved.checked_at = Some(now_ms());
            self.save(&st.saved);
            (st.saved.source, st.host.clone().unwrap_or_default())
        };
        // The image this machine built (a WadOS wadspace's) is looked for
        // whichever source is chosen, so the page can offer it.
        self.look_local(Some(job)).await;
        // GitHub's (or the pushed) image while bootc follows another (this
        // machine's build): bootc's check would ask about that one, so the
        // registry is asked itself.
        let target = registry_target(&host);
        if source != UpdateSource::Github || !follows(&host, &target) {
            return self.check_source(job, source, &target).await;
        }
        let r = self.with_login(job, &["bootc", "upgrade", "--check"]).await;
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
        Ok(u.available.is_some_and(|a| a.digest != declined && newer(&a, &u.booted)))
    }

    /// The newest of this machine's own builds, looked for again (the
    /// machine's store, and the wadspaces' own podman), kept for the page.
    async fn look_local(&self, job: Option<&str>) {
        let family = family(&self.st.lock().unwrap().host.clone().unwrap_or_default());
        let mut images: Vec<(Option<String>, ImageInfo)> = match self.images.all().await {
            Ok(all) => all.into_iter().map(|i| (None, i)).collect(),
            Err(e) => {
                tracing::info!("updates: the machine's images: {e}");
                vec![]
            }
        };
        match self.images.built_in_wadspaces().await {
            Ok(built) => images.extend(built.into_iter().map(|(c, i)| (Some(c), i))),
            Err(e) => tracing::info!("updates: the wadspaces' images: {e}"),
        }
        let (local, from) = newest_local(family, &images).map_or((None, None), |(v, c)| (Some(v), c));
        if let (Some(job), Some(v)) = (job, &local) {
            let where_ = from.as_deref().map_or("this machine's store".to_string(), |c| format!("{c}'s podman"));
            self.jobs.line(job, &format!("» this machine's newest build: {} ({where_})", v.version));
        }
        let mut st = self.st.lock().unwrap();
        st.local = local;
        st.local_in = from;
    }

    /// The check, but by wadd itself: this machine's own build, or GitHub's
    /// (or the pushed) image when bootc follows something else, read from
    /// its registry. Nothing's downloaded: a local image is here, and a
    /// registry's is `bootc switch`ed to when it's applied.
    async fn check_source(&self, job: &str, source: UpdateSource, target: &str) -> Res<bool> {
        let (found, error) = match source {
            UpdateSource::Local => (self.st.lock().unwrap().local.clone(), None),
            UpdateSource::Github | UpdateSource::Pushed => {
                self.jobs.line(job, &format!("» {target}, from its registry"));
                let token = if target.starts_with(&format!("{GHCR}/")) { self.images.token().await } else { None };
                // Without a login first (a public image needs none), then with one if refused.
                let r = match self.images.remote(target, None).await {
                    Err(e) if refused(&e) && token.is_some() => self.images.remote(target, token).await,
                    r => r,
                };
                match r {
                    Ok(v) => {
                        self.jobs.line(job, &format!("» it has {}", v.version));
                        (Some(v), None)
                    }
                    Err(e) => (None, Some(e)),
                }
            }
        };
        if let Some(e) = error {
            self.failed(&e);
            self.publish();
            return Err(e);
        }
        // The one it's on, or one it's already got ready, isn't a new one.
        let (booted, staged) = {
            let host = self.st.lock().unwrap().host.clone().unwrap_or_default();
            (host.booted, host.staged)
        };
        let found =
            found.filter(|f| !same_version(Some(f), booted.as_ref()) && !same_version(Some(f), staged.as_ref()));
        {
            let mut st = self.st.lock().unwrap();
            st.offline = false;
            st.error = None;
            st.found = found;
        }
        let u = self.refresh().await?;
        let declined = self.st.lock().unwrap().saved.declined.clone();
        // Downloaded by itself only when it's newer: going back from this
        // machine's build to an older one of GitHub's is the person's to say.
        Ok(u.available.is_some_and(|a| a.digest != declined && newer(&a, &u.booted)))
    }

    /// Its source, as chosen: saved, and told the page. Not while an update
    /// is under way (it was started for the source it had).
    pub fn set_source(&self, source: UpdateSource) -> Result<proto::Updates, ApiError> {
        {
            let mut st = self.st.lock().unwrap();
            if let Some(j) = &st.job {
                return Err(busy("An update", j));
            }
            st.saved.source = source;
            // What was found is for the source that found it.
            st.found = None;
            self.save(&st.saved);
        }
        Ok(self.publish())
    }

    /// `bootc switch` to an image on a registry: its download and staging
    /// (it says no progress).
    async fn switch(self: &Arc<Self>, job: &str, image: &str) -> Res<()> {
        let _one = self.lock.lock().await;
        self.switch_locked(job, image, false).await
    }

    /// `switch`, with the lock already held (apply has it). `local`: the
    /// image is in the machine's podman store (this machine's build), not
    /// on a registry.
    async fn switch_locked(self: &Arc<Self>, job: &str, image: &str, local: bool) -> Res<()> {
        self.jobs.stage(job, "download", PartState::Working, None);
        let mut argv: Vec<&str> = vec!["bootc", "switch"];
        if local {
            argv.extend(["--transport", "containers-storage"]);
        }
        argv.push(image);
        let argv = argv.as_slice();
        let lines: OnLine = {
            let (jobs, id) = (self.jobs.clone(), job.to_string());
            Box::new(move |l| {
                if !l.trim().is_empty() {
                    jobs.line(&id, l);
                }
            })
        };
        self.jobs.line(job, &format!("$ bootc switch {image}"));
        self.sign_out();
        let r = match self.bootc.run(argv, lines).await {
            Err(e) if refused(&e) && self.sign_in().await => {
                self.jobs.line(job, "» the registry wants a login: with this machine's GitHub token");
                let r = self.bootc(Some(job), argv).await;
                self.signed_in_answer(r)
            }
            r => r,
        };
        if let Err(e) = r {
            self.failed(&e);
            self.publish();
            return Err(e);
        }
        self.jobs.stage(job, "download", PartState::Done, None);
        self.jobs.stage(job, "stage", PartState::Done, None);
        let _ = self.refresh().await;
        Ok(())
    }

    /// What bootc staged, once a switch is done.
    async fn staged(&self, job: &str) -> Res<Version> {
        let u = self.refresh().await?;
        self.jobs.stage(job, "stage", PartState::Done, None);
        u.staged.ok_or_else(|| "bootc staged nothing: this machine is up to date".to_string())
    }

    /// The download and staging: what's staged.
    async fn apply(self: &Arc<Self>, job: &str) -> Res<Version> {
        let _one = self.lock.lock().await;
        // What's to be applied, all read at once (one lock of `st`: a second
        // taken while the first is held would never come).
        let (source, host, next, local_in) = {
            let st = self.st.lock().unwrap();
            (st.saved.source, st.host.clone().unwrap_or_default(), self.view(&st).available, st.local_in.clone())
        };
        // This machine's own build: `bootc switch` to it in the machine's
        // podman store (copied there first from the wadspace that built
        // it), nothing downloaded.
        if source == UpdateSource::Local {
            let next = next.ok_or("Nothing to apply: this machine has built nothing newer")?;
            let id = next.digest.clone().ok_or("this machine's build has no image id")?;
            let id = id.trim_start_matches("sha256:").to_string();
            self.jobs.line(job, &format!("» from this machine's builds: {}", next.version));
            if let Some(container) = local_in {
                self.jobs.stage(job, "download", PartState::Working, None);
                self.jobs.line(job, &format!("» copying it out of {container}'s podman"));
                let (jobs, id_) = (self.jobs.clone(), job.to_string());
                let on: OnLine = Box::new(move |l| {
                    if !l.trim().is_empty() {
                        jobs.line(&id_, l);
                    }
                });
                if let Err(e) = self.images.import(&container, &id, on).await {
                    let e = format!("couldn't copy it out of {container}: {e}");
                    self.failed(&e);
                    self.publish();
                    return Err(e);
                }
                // In the machine's store now: a later apply needs no copy.
                self.st.lock().unwrap().local_in = None;
            }
            // `name@<image id>` (containers-storage's own form): the id pins
            // it, so a build made after the check can't be a surprise.
            let image = format!("{}@{id}", repo_of(&next.image));
            self.switch_locked(job, &image, true).await?;
            return self.staged(job).await;
        }
        // GitHub's (or the pushed) image, while bootc follows another (this
        // machine's build, or an image of another name): switched back to.
        let target = registry_target(&host);
        if !follows(&host, &target) {
            self.jobs.line(job, &format!("» back to {target}"));
            self.switch_locked(job, &target, false).await?;
            return self.staged(job).await;
        }
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
            self.sign_out();
            match self.bootc.run(argv, on_line).await {
                Err(e) if refused(&e) && self.sign_in().await => {
                    self.jobs.line(job, "» the registry wants a login: with this machine's GitHub token");
                    let r = self.bootc(Some(job), argv).await;
                    self.signed_in_answer(r)
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
            // Only a newer one: the copy here as new (CI's, pulled before; or
            // one built here, which a stick carries) is neither downloaded
            // again nor replaced by an older one.
            match self.base_newer(b, token.clone()).await {
                Ok(false) => {
                    self.jobs.line(job, &format!("✓ {}: the one here is as new", b.name));
                    let mut st = self.st.lock().unwrap();
                    st.saved.refreshed.insert(b.local.clone(), now_ms());
                    self.save(&st.saved);
                    drop(st);
                    self.jobs.stage(job, &stage, PartState::Done, None);
                    continue;
                }
                Ok(true) => {}
                // Not read (the registry, or the way to it): pulled as before.
                Err(e) => self.jobs.line(job, &format!("  {}: its build time wasn't read ({e})", b.name)),
            }
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

    /// Whether the published `b` is newer than the copy here (built later,
    /// by its config's time): true when there's none here.
    async fn base_newer(&self, b: &BaseImage, token: Option<String>) -> Res<bool> {
        let Some(here) = self.images.info(&b.local).await? else { return Ok(true) };
        let token = token.filter(|_| b.published.starts_with(&format!("{GHCR}/")));
        let published = self.images.published_created(&b.published, token).await?;
        // A second's leeway: the same image, read two ways.
        Ok(published > here.created + 1.0)
    }

    /// Whether any base is newer where it's published (or missing here).
    /// Errors: what couldn't be read (offline, say).
    async fn any_base_newer(&self) -> Res<bool> {
        let token = self.images.token().await;
        for b in &self.settings.bases {
            if self.base_newer(b, token.clone()).await? {
                return Ok(true);
            }
        }
        Ok(false)
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
        // A source that isn't GitHub's needs this machine's own builds known
        // (a check looks again; the page can be opened before one).
        let wanted = me.st.lock().unwrap().saved.source != UpdateSource::Github;
        if wanted && me.st.lock().unwrap().local.is_none() {
            me.look_local(None).await;
        }
        Ok(me.now())
    }

    async fn check(&self) -> Result<JobInfo, ApiError> {
        self.check_job()
    }

    async fn apply(&self) -> Result<JobInfo, ApiError> {
        self.apply_job()
    }

    async fn use_nvidia(&self) -> Result<JobInfo, ApiError> {
        let booted = self.status().await?.booted.image;
        let image = nvidia_image(&booted)
            .ok_or_else(|| ApiError::conflict(format!("{booted} has no NVIDIA image beside it")))?;
        self.nvidia_job(image)
    }

    async fn source(&self, source: UpdateSource) -> Result<proto::Updates, ApiError> {
        // A machine whose image bootc doesn't follow has only the two the
        // wadspace makes (that's the point of them: a laptop's build).
        let me = &self.0;
        if source != UpdateSource::Github {
            me.can_update()?;
        }
        me.set_source(source)
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
