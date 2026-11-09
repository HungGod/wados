//! The installer stick (phase 9): `GET|POST /v1/install`.
//!
//! A stick is the host image installed on a stick, with
//! `<state>/installer/installer.toml` (root's). Without that file this is
//! off and the routes say 404. `cloudflare.toml` beside it, only on a stick
//! written with the secrets, holds Cloudflare's keys; without it the machine
//! just has no public address (a stick is built without them for tests).
//! `firebase.toml`, also only there, is the WadSpaces account's web app: each
//! machine installed gets it, and without it has no account. Neither is in
//! the image, which is public.
//!
//! An install is one Install job:
//!
//! 1. **The address** ([`wad_cloudflare`]), before the disk is touched, so a
//!    name that's taken leaves the disk as it was;
//! 2. **the job's folder** on tmpfs (0700): `request`, `passphrase` (0600,
//!    no newline), and `provision/`, what first boot puts in place: the
//!    hostname, the account (its name and the password's hash), `wadd.toml`,
//!    the tunnel's token, the TURN key, the account's web app, `setup.json`
//!    ([`write_provision`]);
//! 3. **`install-to-disk`**, a systemd unit of its own, outside wadd's
//!    sandbox: `STEP n/N` lines in its log are the job's stages.
//!
//! The tunnel creator's and DNS tokens stay on the stick. The folder goes
//! when the job ends, whatever happened. Nothing here logs a password or
//! a key, and `InstallRequest`'s `Debug` doesn't show them.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use wad_cloudflare::{Address, Cloudflare, Keys};
use wad_model::install_names::{computer_name_problem, password_problem, username_problem};
use wad_model::{JobKind, PartState};
use wad_proto::install::{ENCRYPTING, PROVISION, STAGES};
use wad_proto::{ApiError, Install, InstallDisk, InstallRequest, JobInfo};

use crate::jobs::Jobs;

pub const SCRIPT: &str = "/usr/libexec/wados/install-to-disk";
/// wadd.service's RuntimeDirectory.
pub const JOBS_DIR: &str = "/run/wados/install";
const UNIT: &str = "wados-install";

pub type OnLine<'a> = dyn FnMut(&str) + Send + 'a;

/// What an install needs from the machine.
#[async_trait]
pub trait Hands: Send + Sync + 'static {
    async fn disks(&self) -> Result<Vec<InstallDisk>, String>;
    /// The password as `/etc/shadow` keeps it.
    async fn hash(&self, password: &str) -> Result<String, String>;
    /// `install-to-disk`, each of its lines to `on_line`, until it ends.
    async fn run(&self, job: &Path, on_line: &mut OnLine<'_>) -> Result<(), String>;
}

/// `installer.toml`.
#[derive(Debug, Clone, Deserialize)]
struct File {
    installer: Stick,
}

#[derive(Debug, Clone, Deserialize)]
struct Stick {
    /// The image the machine follows.
    target: String,
    #[serde(default = "domain")]
    domain: String,
    /// Tests only: a mock of Cloudflare's API.
    #[serde(default = "api")]
    api: String,
}

fn domain() -> String {
    "wadspaces.com".into()
}

fn api() -> String {
    wad_cloudflare::API.into()
}

struct Inner {
    stick: Stick,
    keys: Option<Keys>,
    /// The stick's `firebase.toml`, as it is (checked when read).
    firebase: Option<String>,
    jobs_dir: PathBuf,
    hands: Arc<dyn Hands>,
    jobs: Arc<Jobs>,
    last: Mutex<Option<String>>,
}

/// Cheap to clone.
#[derive(Clone)]
pub struct MachineInstall(Arc<Inner>);

impl MachineInstall {
    /// This machine's, if it's a stick.
    pub fn open(state_dir: &Path, jobs_dir: &Path, hands: Arc<dyn Hands>, jobs: Arc<Jobs>) -> Option<Self> {
        let dir = state_dir.join("installer");
        let text = std::fs::read_to_string(dir.join("installer.toml")).ok()?;
        let stick = match toml::from_str::<File>(&text) {
            Ok(f) => f.installer,
            Err(e) => {
                tracing::error!("the stick's installer.toml: {}", e.message());
                return None;
            }
        };
        let keys = match std::fs::read_to_string(dir.join("cloudflare.toml")) {
            Ok(t) => match Keys::parse(&t) {
                Ok(k) => Some(k),
                Err(e) => {
                    tracing::warn!("{e}: this machine won't have a public address");
                    None
                }
            },
            Err(_) => None,
        };
        let firebase_file = dir.join("firebase.toml");
        let firebase = crate::account::FirebaseApp::read(&firebase_file)
            .and_then(|_| std::fs::read_to_string(&firebase_file).ok());
        Some(Self(Arc::new(Inner {
            stick,
            keys,
            firebase,
            jobs_dir: jobs_dir.to_path_buf(),
            hands,
            jobs,
            last: Mutex::new(None),
        })))
    }

    /// `wadd serve --sample`'s: a stick whose install only pretends.
    pub fn sample(jobs_dir: &Path, step: Duration, jobs: Arc<Jobs>) -> Self {
        let stick = Stick { target: "ghcr.io/hunggod/wados-host:latest".into(), domain: domain(), api: api() };
        Self(Arc::new(Inner {
            stick,
            keys: None,
            firebase: None,
            jobs_dir: jobs_dir.to_path_buf(),
            hands: Arc::new(SampleHands { step }),
            jobs,
            last: Mutex::new(None),
        }))
    }

    fn stages(encrypted: bool, public: bool) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        if public {
            out.push((PROVISION.0.into(), PROVISION.1.into()));
        }
        for (id, label) in STAGES {
            let label = if id == "disk" && encrypted { ENCRYPTING } else { label };
            out.push((id.into(), label.into()));
        }
        out
    }
}

/// `STEP 3/7: …`: which of the script's steps, from 0.
fn step_of(line: &str) -> Option<usize> {
    let rest = line.strip_prefix("STEP ")?;
    let (n, _) = rest.split_once('/')?;
    n.parse::<usize>().ok()?.checked_sub(1)
}

fn write_file(path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(mode).open(path)?;
    f.write_all(bytes)
}

/// What first boot puts in place (`host/usr/libexec/wados/provision`).
pub fn write_provision(
    dir: &Path,
    req: &InstallRequest,
    hash: &str,
    address: Option<&Address>,
    keys: Option<&Keys>,
    firebase: Option<&str>,
) -> std::io::Result<()> {
    write_file(&dir.join("hostname"), format!("{}\n", req.name).as_bytes(), 0o644)?;
    write_file(&dir.join("account"), format!("{}\n{hash}\n", req.username).as_bytes(), 0o600)?;
    let mut wadd = format!("[machine]\nid = \"{}\"\n", req.name);
    if let Some(a) = address {
        wadd.push_str(&format!("\n[streaming]\nhost = \"{}\"\n", a.host));
        write_file(&dir.join("cloudflared-token"), a.token.as_bytes(), 0o600)?;
    }
    write_file(&dir.join("wadd.toml"), wadd.as_bytes(), 0o644)?;
    if let Some(turn) = keys.and_then(Keys::turn_file) {
        write_file(&dir.join("turn.toml"), turn.as_bytes(), 0o600)?;
    }
    if let Some(f) = firebase {
        write_file(&dir.join("firebase.toml"), f.as_bytes(), 0o600)?;
    }
    write_file(&dir.join("setup.json"), br#"{"step":"welcome"}"#, 0o600)
}

impl Inner {
    fn check(&self, req: &InstallRequest) -> Result<(), ApiError> {
        let problem = computer_name_problem(&req.name)
            .or_else(|| username_problem(&req.username))
            .or_else(|| password_problem(&req.password))
            .or_else(|| req.disk_password.as_deref().and_then(password_problem));
        match problem {
            Some(p) => Err(ApiError::bad_request(p)),
            None => Ok(()),
        }
    }

    async fn address(&self, name: &str, job: &str) -> Result<Option<Address>, String> {
        let Some(keys) = &self.keys else {
            return Ok(None);
        };
        let (stage, _) = PROVISION;
        self.jobs.stage(job, stage, PartState::Working, None);
        let cf = Cloudflare::new(&self.stick.api, &self.stick.domain, keys);
        match cf.provision(name).await {
            Ok(a) => {
                self.jobs.line(job, &format!("» {}", a.host));
                self.jobs.stage(job, stage, PartState::Done, None);
                Ok(Some(a))
            }
            Err(e) => {
                self.jobs.stage(job, stage, PartState::Error, Some(e.to_string()));
                Err(e.to_string())
            }
        }
    }

    fn folder(
        &self,
        job: &str,
        req: &InstallRequest,
        hash: &str,
        address: Option<&Address>,
    ) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&self.jobs_dir)?;
        let dir = self.jobs_dir.join(job);
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        let provision = dir.join("provision");
        std::fs::DirBuilder::new().mode(0o700).create(&provision)?;
        write_provision(&provision, req, hash, address, self.keys.as_ref(), self.firebase.as_deref())?;
        let request = format!(
            "DISK={}\nSIZE={}\nSERIAL={}\nTARGET={}\nENCRYPT={}\n",
            req.by_id,
            req.size,
            req.serial,
            self.stick.target,
            u8::from(req.disk_password.is_some())
        );
        write_file(&dir.join("request"), request.as_bytes(), 0o600)?;
        if let Some(p) = &req.disk_password {
            write_file(&dir.join("passphrase"), p.as_bytes(), 0o600)?;
        }
        Ok(dir)
    }

    async fn install(&self, job: &str, req: InstallRequest) -> Result<(), String> {
        let address = self.address(&req.name, job).await?;
        let hash = self.hands.hash(&req.password).await?;
        let dir = self.folder(job, &req, &hash, address.as_ref()).map_err(|e| format!("the job's folder: {e}"))?;
        drop(req);
        struct Gone(PathBuf);
        impl Drop for Gone {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _gone = Gone(dir.clone());
        let mut at: Option<usize> = None;
        let mut on_line = |line: &str| match step_of(line) {
            Some(n) if n < STAGES.len() => {
                if let Some(before) = at {
                    self.jobs.stage(job, STAGES[before].0, PartState::Done, None);
                }
                self.jobs.stage(job, STAGES[n].0, PartState::Working, None);
                at = Some(n);
            }
            _ => self.jobs.line(job, line),
        };
        let ran = self.hands.run(&dir, &mut on_line).await;
        let result = std::fs::read_to_string(dir.join("result")).unwrap_or_default();
        let result = result.trim();
        match (ran, result.strip_prefix("failed: ")) {
            (_, Some(why)) => Err(why.to_string()),
            (Ok(()), None) if result == "ok" => {
                if let Some(n) = at {
                    self.jobs.stage(job, STAGES[n].0, PartState::Done, None);
                }
                Ok(())
            }
            (Err(e), None) => Err(e),
            (Ok(()), None) => Err("the install ended without saying how it went".into()),
        }
    }
}

#[async_trait]
impl crate::api_phase9::Installer for MachineInstall {
    async fn status(&self) -> Result<Install, ApiError> {
        let me = &self.0;
        let disks = me.hands.disks().await.map_err(ApiError::internal)?;
        Ok(Install { disks, public: me.keys.is_some(), job: me.last.lock().unwrap().clone() })
    }

    async fn start(&self, req: InstallRequest) -> Result<JobInfo, ApiError> {
        let me = &self.0;
        me.check(&req)?;
        let disks = me.hands.disks().await.map_err(ApiError::internal)?;
        let found = disks.iter().any(|d| d.by_id == req.by_id && d.size == req.size && d.serial == req.serial);
        if !found {
            return Err(ApiError::conflict("That disk isn't here any more: choose it again"));
        }
        let job = {
            let mut last = me.last.lock().unwrap();
            if let Some(id) = &*last {
                let running = me.jobs.info(id).is_some_and(|j| !crate::jobs::finished(j.job.status));
                if running {
                    return Err(ApiError::conflict("An install is under way already"));
                }
            }
            let stages = MachineInstall::stages(req.disk_password.is_some(), me.keys.is_some());
            let id = me.jobs.create(JobKind::Install, "Installing WadSpaces", &req.name, &stages);
            *last = Some(id.clone());
            id
        };
        let (inner, id) = (me.clone(), job.clone());
        let task = tokio::spawn(async move {
            inner.jobs.running(&id);
            let r = inner.install(&id, req).await;
            inner.jobs.finish(&id, r);
        });
        me.jobs.attach(&job, task.abort_handle());
        me.jobs.info(&job).ok_or_else(|| ApiError::internal("the job went"))
    }
}

/// `lsblk --json` as the disks an install could use: not the stick (what's
/// mounted at the system's places), not a disk with no lasting name.
pub fn parse_disks(lsblk: &Value, by_id: &dyn Fn(&str) -> Option<String>) -> Vec<InstallDisk> {
    let mut out = Vec::new();
    let text = |d: &Value, k: &str| d[k].as_str().map(str::trim).unwrap_or("").to_string();
    for d in lsblk["blockdevices"].as_array().into_iter().flatten() {
        let name = text(d, "name");
        let size = d["size"].as_u64().unwrap_or(0);
        if d["type"].as_str() != Some("disk") || size == 0 || name.starts_with("zram") {
            continue;
        }
        if mounted_system(d) {
            continue;
        }
        let Some(link) = by_id(&name) else {
            continue;
        };
        out.push(InstallDisk {
            by_id: link,
            model: Some(text(d, "model")).filter(|m| !m.is_empty()).unwrap_or_else(|| name.clone()),
            serial: text(d, "serial"),
            removable: d["rm"].as_bool().unwrap_or(false) || d["tran"].as_str() == Some("usb"),
            holds: holds(d),
            size,
            name,
        });
    }
    out
}

fn mounts(d: &Value) -> Vec<String> {
    let mut all: Vec<String> =
        d["mountpoints"].as_array().into_iter().flatten().filter_map(|m| m.as_str().map(String::from)).collect();
    for c in d["children"].as_array().into_iter().flatten() {
        all.extend(mounts(c));
    }
    all
}

fn mounted_system(d: &Value) -> bool {
    mounts(d)
        .iter()
        .any(|m| matches!(m.as_str(), "/" | "/sysroot" | "/boot" | "/var" | "/etc") || m.starts_with("/boot/"))
}

/// What's on a disk, in a word.
fn holds(d: &Value) -> Option<String> {
    let parts: Vec<&Value> = d["children"].as_array().into_iter().flatten().collect();
    if parts.is_empty() {
        return d["fstype"].as_str().map(String::from);
    }
    let named: Vec<&str> = parts.iter().filter_map(|p| p["label"].as_str().or(p["fstype"].as_str())).collect();
    Some(if named.is_empty() { format!("{} partitions", parts.len()) } else { named.join(", ") })
}

/// The machine's own.
pub struct SystemHands {
    pub script: PathBuf,
}

/// A disk's `/dev/disk/by-id` link: the bus's, rather than `wwn-`, `eui.` or a namespace's `_1`.
fn by_id(name: &str) -> Option<String> {
    let dir = Path::new("/dev/disk/by-id");
    let mut links: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| {
            std::fs::canonicalize(e.path()).is_ok_and(|p| p.file_name().is_some_and(|n| n.to_string_lossy() == name))
        })
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    links.sort_by_key(|l| (l.starts_with("wwn-") || l.contains("eui.") || l.ends_with("_1"), l.len()));
    links.first().map(|l| dir.join(l).display().to_string())
}

struct StopUnit(bool);

impl Drop for StopUnit {
    fn drop(&mut self) {
        if self.0 {
            let _ = std::process::Command::new("systemctl").args(["stop", UNIT]).status();
        }
    }
}

#[async_trait]
impl Hands for SystemHands {
    async fn disks(&self) -> Result<Vec<InstallDisk>, String> {
        let out = tokio::process::Command::new("lsblk")
            .args(["--json", "-b", "-o", "NAME,SIZE,MODEL,SERIAL,TRAN,TYPE,RM,MOUNTPOINTS,FSTYPE,LABEL"])
            .output()
            .await
            .map_err(|e| format!("lsblk: {e}"))?;
        let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("lsblk: {e}"))?;
        Ok(parse_disks(&v, &by_id))
    }

    async fn hash(&self, password: &str) -> Result<String, String> {
        use tokio::io::AsyncWriteExt;
        let mut child = tokio::process::Command::new("openssl")
            .args(["passwd", "-6", "-stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("openssl: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("openssl: no stdin")?;
        stdin.write_all(password.as_bytes()).await.map_err(|e| format!("openssl: {e}"))?;
        drop(stdin);
        let out = child.wait_with_output().await.map_err(|e| format!("openssl: {e}"))?;
        let hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() && hash.starts_with('$') { Ok(hash) } else { Err("couldn't hash the password".into()) }
    }

    async fn run(&self, job: &Path, on_line: &mut OnLine<'_>) -> Result<(), String> {
        let mut child = tokio::process::Command::new("systemd-run")
            .args(["--quiet", "--wait", "--collect", "--service-type=exec"])
            .arg(format!("--unit={UNIT}"))
            .arg("--property=Description=Installing WadOS (wadd)")
            .arg("--")
            .arg(&self.script)
            .arg(job)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("systemd-run didn't start: {e}"))?;
        let mut stop = StopUnit(true);
        let mut read = 0;
        let log = job.join("log");
        let status = loop {
            tokio::select! {
                s = child.wait() => break s.map_err(|e| format!("systemd-run: {e}"))?,
                _ = tokio::time::sleep(Duration::from_millis(500)) => read = follow(&log, read, on_line),
            }
        };
        stop.0 = false;
        follow(&log, read, on_line);
        if status.success() { Ok(()) } else { Err("the install stopped".into()) }
    }
}

/// The whole lines added to `log` since `read`; where it's read to now.
fn follow(log: &Path, read: usize, on_line: &mut OnLine<'_>) -> usize {
    let Ok(bytes) = std::fs::read(log) else {
        return read;
    };
    let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else {
        return read;
    };
    if end < read {
        return read;
    }
    for line in String::from_utf8_lossy(&bytes[read..=end]).lines() {
        on_line(line);
    }
    end + 1
}

/// `wadd serve --sample`: a stick with two disks, whose install only pretends.
pub struct SampleHands {
    pub step: Duration,
}

#[async_trait]
impl Hands for SampleHands {
    async fn disks(&self) -> Result<Vec<InstallDisk>, String> {
        let d = |name: &str, model: &str, gb: u64, removable, holds: Option<&str>| InstallDisk {
            by_id: format!("/dev/disk/by-id/sample-{name}"),
            name: name.into(),
            model: model.into(),
            serial: format!("S{name}"),
            size: gb * 1_000_000_000,
            removable,
            holds: holds.map(String::from),
        };
        Ok(vec![
            d("nvme0n1", "Samsung SSD 980", 512, false, Some("Windows")),
            d("sda", "SanDisk Ultra", 64, true, None),
        ])
    }

    async fn hash(&self, _: &str) -> Result<String, String> {
        Ok("$6$sample$sample".into())
    }

    async fn run(&self, job: &Path, on_line: &mut OnLine<'_>) -> Result<(), String> {
        for (i, (_, label)) in STAGES.iter().enumerate() {
            on_line(&format!("STEP {}/{}: {label}", i + 1, STAGES.len()));
            tokio::time::sleep(self.step).await;
        }
        std::fs::write(job.join("result"), "ok\n").map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests;
