//! A machine in memory, for tests: podman's images and containers, a
//! registry to pull from, systemd's units. A build makes an image from its
//! Dockerfile's last stage (the labels of the image it's FROM, its LABEL
//! lines, the labels it's given); starting `wad-<id>.service` runs
//! `wad-<id>` from its unit's `Image=`, and first the units it's bound to
//! (`BindsTo=`: a streamed wadspace's sidecar), which stop with it (they're
//! `StopWhenUnneeded`) as it stops with them. A running container's
//! `PublishPort=127.0.0.1:<port>:` answers `fetch` at `http://127.0.0.1:<port>/`
//! (a sidecar's health). A container whose unit runs it in a VM
//! (`GlobalArgs=--runtime=`) can't be exec'd into, as under crun-krun. Every
//! call that changes something is in `calls`, in order.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::backend::{Backend, ExecOut, ImageInfo, MISSING, OnBytes, OnLine, RUNNING, Res};

#[derive(Debug, Clone, Default)]
pub struct FakeContainer {
    pub image: String,
    pub status: String,
    /// Polls of `desktop_up` left before its desktop is up.
    pub desktop_in: u32,
    pub pid: u32,
    /// The volumes it mounts (its unit's `Volume=<x>.volume:`): in use while it's there.
    pub volumes: Vec<String>,
    /// Stopping: polls of its status left before podman has removed it.
    pub gone_in: u32,
}

/// One build, as it was asked for.
#[derive(Debug, Clone)]
pub struct FakeBuild {
    pub tag: String,
    pub dockerfile: String,
    /// The context's entries (paths), and what each held.
    pub files: BTreeMap<String, Vec<u8>>,
    pub args: Map<String, Value>,
    pub labels: Map<String, Value>,
}

#[derive(Default)]
pub struct State {
    /// By id.
    pub images: BTreeMap<String, ImageInfo>,
    /// What can be pulled: reference → image.
    pub registry: BTreeMap<String, ImageInfo>,
    /// What `fetch` finds.
    pub documents: BTreeMap<String, Vec<u8>>,
    pub containers: BTreeMap<String, FakeContainer>,
    pub units: BTreeMap<String, String>,
    pub reloads: u32,
    pub builds: Vec<FakeBuild>,
    pub calls: Vec<String>,
    pub secrets: BTreeMap<String, String>,
    /// Builds whose Dockerfile has this fail.
    pub fail_builds: Vec<String>,
    /// No network: pulls and fetches fail.
    pub offline: bool,
    /// Podman doesn't answer.
    pub down: bool,
    /// Polls before a started wadspace's desktop is up.
    pub desktop_after: u32,
    /// Running sidecars don't answer on their ports.
    pub sidecars_silent: bool,
    /// What a running sidecar's `/api/status` answers (else `ok`).
    pub sidecar_status: Option<String>,
    /// Podman's volumes (a quadlet `.volume`'s made as a container using it starts).
    pub volumes: std::collections::BTreeSet<String>,
    /// The `<x>-volume.service` oneshots active (they stay active, RemainAfterExit).
    pub volume_units: std::collections::BTreeSet<String>,
    /// A stopped container lingers (stopping, its volumes in use) this many
    /// polls of its status before podman has removed it.
    pub slow_stops: u32,
    /// What an exec answers (code, stdout), by the start of its command
    /// (`wadspaces-session switch`); else 0 and `{"still_open": []}`.
    pub exec_answers: Vec<(String, i64, String)>,
    pub free_gb: Option<f64>,
}

pub struct Fake {
    pub state: Mutex<State>,
    seq: AtomicU64,
    /// While set, builds wait (to cancel one, or watch one run).
    pub hold: tokio::sync::Semaphore,
    holding: std::sync::atomic::AtomicBool,
}

/// A container stopped that podman hasn't removed yet ([`State::slow_stops`]).
pub const STOPPING: &str = "stopping";

fn short(id: &str) -> &str {
    id.trim_start_matches("sha256:")
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            seq: AtomicU64::new(0),
            hold: tokio::sync::Semaphore::new(0),
            holding: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl Fake {
    pub fn new() -> Self {
        Self::default()
    }

    /// An image here: its name, labels and size; returns its id.
    pub fn add_image(&self, tag: &str, labels: &[(&str, &str)]) -> String {
        let id = self.new_id(tag);
        let info = ImageInfo {
            id: id.clone(),
            created: self.seq.load(Ordering::Relaxed) as f64,
            size: 100_000_000,
            labels: labels.iter().map(|(k, v)| (k.to_string(), Value::String(v.to_string()))).collect(),
            tags: vec![tag.to_string()],
            parent: String::new(),
        };
        let mut st = self.state.lock().unwrap();
        // The name moves to it.
        for i in st.images.values_mut() {
            i.tags.retain(|t| t != tag);
        }
        st.images.insert(id.clone(), info);
        id
    }

    /// An image the registry has at `reference`; returns its id.
    pub fn publish(&self, reference: &str, labels: &[(&str, &str)]) -> String {
        let id = self.new_id(reference);
        let info = ImageInfo {
            id: id.clone(),
            created: self.seq.load(Ordering::Relaxed) as f64,
            size: 50_000_000,
            labels: labels.iter().map(|(k, v)| (k.to_string(), Value::String(v.to_string()))).collect(),
            tags: vec![],
            parent: String::new(),
        };
        self.state.lock().unwrap().registry.insert(reference.to_string(), info);
        id
    }

    /// Another name for an image here.
    pub fn add_tag(&self, image: &str, tag: &str) {
        let mut st = self.state.lock().unwrap();
        let id = find(&st, image).map(|i| i.id.clone()).expect("an image");
        st.images.get_mut(&id).expect("found").tags.push(tag.to_string());
    }

    /// A container running that wadd didn't start (it was running before wadd).
    pub fn run_container(&self, name: &str, image: &str) {
        self.state.lock().unwrap().containers.insert(
            name.to_string(),
            FakeContainer { image: image.into(), status: RUNNING.into(), pid: 9000, ..Default::default() },
        );
    }

    /// Builds wait from now until `release`.
    pub fn hold_builds(&self) {
        self.holding.store(true, Ordering::SeqCst);
    }

    pub fn release_builds(&self) {
        self.holding.store(false, Ordering::SeqCst);
        self.hold.add_permits(1000);
    }

    pub fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }

    pub fn builds(&self) -> Vec<FakeBuild> {
        self.state.lock().unwrap().builds.clone()
    }

    /// The image a name (or an id) is.
    pub fn image(&self, name: &str) -> Option<ImageInfo> {
        find(&self.state.lock().unwrap(), name).cloned()
    }

    /// A container stops by itself (its window closed, say).
    pub fn exit(&self, name: &str) {
        self.state.lock().unwrap().containers.remove(name);
    }

    fn new_id(&self, what: &str) -> String {
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        let hex: String = Sha256::digest(format!("{what}#{n}").as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        format!("sha256:{hex}")
    }

    fn call(&self, s: String) {
        self.state.lock().unwrap().calls.push(s);
    }
}

fn find<'a>(st: &'a State, name: &str) -> Option<&'a ImageInfo> {
    let id = short(name);
    st.images
        .values()
        .find(|i| i.tags.iter().any(|t| t == name))
        .or_else(|| st.images.values().find(|i| id.len() >= 12 && short(&i.id).starts_with(id)))
}

/// A tar's entries (ustar and pax paths), as `tar` writes them.
pub fn untar(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut at = 0;
    let mut long: Option<String> = None;
    while at + 512 <= bytes.len() {
        let h = &bytes[at..at + 512];
        if h.iter().all(|b| *b == 0) {
            break;
        }
        let field = |a: usize, n: usize| {
            let f = &h[a..a + n];
            String::from_utf8_lossy(&f[..f.iter().position(|b| *b == 0).unwrap_or(n)]).into_owned()
        };
        let size = usize::from_str_radix(field(124, 12).trim(), 8).unwrap_or(0);
        let kind = h[156];
        let data = &bytes[at + 512..(at + 512 + size).min(bytes.len())];
        let prefix = field(345, 155);
        let name = if prefix.is_empty() { field(0, 100) } else { format!("{prefix}/{}", field(0, 100)) };
        match kind {
            b'x' => {
                let text = String::from_utf8_lossy(data);
                long = text.lines().find_map(|l| l.split_once(" path=").map(|(_, p)| p.to_string()));
            }
            _ => {
                let name = long.take().unwrap_or(name);
                if kind == b'2' {
                    out.insert(name, format!("-> {}", field(157, 100)).into_bytes());
                } else {
                    out.insert(name, data.to_vec());
                }
            }
        }
        at += 512 + size.div_ceil(512) * 512;
    }
    out
}

/// The labels a Dockerfile's last stage gives (`LABEL k="v" …` lines), and
/// the image it's FROM (`${BASE_IMAGE}` from its ARG or the build's args).
fn last_stage(file: &str, args: &Map<String, Value>) -> (Option<String>, Vec<(String, String)>) {
    let mut arg_default = None;
    let mut from = None;
    let mut labels = vec![];
    for line in file.lines() {
        if let Some(a) = line.strip_prefix("ARG BASE_IMAGE=") {
            arg_default = Some(a.to_string());
        } else if let Some(f) = line.strip_prefix("FROM ") {
            let image = f.split_whitespace().next().unwrap_or_default().to_string();
            let image = if image == "${BASE_IMAGE}" {
                args.get("BASE_IMAGE")
                    .and_then(Value::as_str)
                    .map(String::from)
                    .or(arg_default.clone())
                    .unwrap_or_default()
            } else {
                image
            };
            from = (image != "scratch").then_some(image);
            labels.clear();
        } else if let Some(rest) = line.strip_prefix("LABEL ") {
            // k="v" pairs, the values JSON strings.
            let mut s = rest;
            while let Some((k, after)) = s.split_once('=') {
                let k = k.trim().to_string();
                let mut de = serde_json::Deserializer::from_str(after).into_iter::<String>();
                let Some(Ok(v)) = de.next() else { break };
                labels.push((k, v));
                s = &after[de.byte_offset()..];
            }
        }
    }
    (from, labels)
}

#[async_trait]
impl Backend for Fake {
    async fn available(&self) -> bool {
        !self.state.lock().unwrap().down
    }

    async fn image_info(&self, image: &str) -> Res<Option<ImageInfo>> {
        let st = self.state.lock().unwrap();
        if st.down {
            return Err("podman isn't answering".into());
        }
        Ok(find(&st, image).cloned())
    }

    async fn images_labelled(&self, label: &str) -> Res<Vec<ImageInfo>> {
        let st = self.state.lock().unwrap();
        Ok(st.images.values().filter(|i| i.labels.contains_key(label)).cloned().collect())
    }

    async fn build(
        &self,
        context: Vec<u8>,
        tag: &str,
        args: Map<String, Value>,
        labels: Map<String, Value>,
        mut on_line: OnLine,
    ) -> Res<String> {
        let files = untar(&context);
        let dockerfile =
            String::from_utf8_lossy(files.get("Dockerfile").map(Vec::as_slice).unwrap_or_default()).into_owned();
        self.call(format!("build {tag}"));
        self.state.lock().unwrap().builds.push(FakeBuild {
            tag: tag.to_string(),
            dockerfile: dockerfile.clone(),
            files,
            args: args.clone(),
            labels: labels.clone(),
        });
        on_line(&format!("STEP 1/2: building {tag}"));
        if self.holding.load(Ordering::SeqCst) {
            let _ = self.hold.acquire().await;
        }
        if let Some(f) = self.state.lock().unwrap().fail_builds.iter().find(|f| dockerfile.contains(f.as_str())) {
            return Err(format!("the build failed ({f})"));
        }
        on_line("STEP 2/2: done");
        let (from, own) = last_stage(&dockerfile, &args);
        let id = self.new_id(&format!("{tag}{dockerfile}"));
        let mut st = self.state.lock().unwrap();
        let mut all: Map<String, Value> = match &from {
            Some(f) => find(&st, f).ok_or_else(|| format!("{f} isn't here"))?.labels.clone(),
            None => Map::new(),
        };
        all.extend(own.into_iter().map(|(k, v)| (k, Value::String(v))));
        all.extend(labels);
        // The name moves to the new image.
        for i in st.images.values_mut() {
            i.tags.retain(|t| t != tag);
        }
        let created = self.seq.load(Ordering::Relaxed) as f64;
        st.images.insert(
            id.clone(),
            ImageInfo {
                id: id.clone(),
                created,
                size: 200_000_000,
                labels: all,
                tags: vec![tag.into()],
                parent: String::new(),
            },
        );
        Ok(id)
    }

    async fn pull(&self, reference: &str, _auth: Option<String>, mut on_bytes: OnBytes) -> Res<String> {
        self.call(format!("pull {reference}"));
        let mut st = self.state.lock().unwrap();
        if st.offline {
            return Err("no network".into());
        }
        let mut info = st.registry.get(reference).cloned().ok_or_else(|| format!("{reference}: not found"))?;
        on_bytes(info.size / 2, info.size);
        on_bytes(info.size, info.size);
        info.tags = vec![reference.to_string()];
        let id = info.id.clone();
        st.images.entry(id.clone()).or_insert(info);
        Ok(id)
    }

    async fn tag(&self, image: &str, target: &str) -> Res<()> {
        self.call(format!("tag {} {target}", &short(image)[..short(image).len().min(12)]));
        let mut st = self.state.lock().unwrap();
        let id = find(&st, image).map(|i| i.id.clone()).ok_or_else(|| format!("no image {image}"))?;
        for i in st.images.values_mut() {
            i.tags.retain(|t| t != target);
        }
        let i = st.images.get_mut(&id).expect("found");
        i.tags.push(target.to_string());
        Ok(())
    }

    async fn remove_image(&self, image: &str) -> Res<bool> {
        self.call(format!("untag {image}"));
        let mut st = self.state.lock().unwrap();
        let Some(id) = find(&st, image).map(|i| i.id.clone()) else { return Ok(false) };
        let i = st.images.get_mut(&id).expect("found");
        i.tags.retain(|t| t != image);
        if i.tags.is_empty() {
            st.images.remove(&id);
        }
        Ok(true)
    }

    async fn free_gb(&self) -> Option<f64> {
        self.state.lock().unwrap().free_gb
    }

    async fn secret_names(&self) -> Res<Vec<String>> {
        Ok(self.state.lock().unwrap().secrets.keys().cloned().collect())
    }

    async fn secret_value(&self, name: &str) -> Res<Option<String>> {
        Ok(self.state.lock().unwrap().secrets.get(name).cloned())
    }

    async fn create_secret(&self, name: &str, value: &[u8]) -> Res<()> {
        // Its name only: a value never goes in the calls.
        self.call(format!("secret create {name}"));
        let mut st = self.state.lock().unwrap();
        if st.down {
            return Err("podman isn't answering".into());
        }
        let value = String::from_utf8(value.to_vec()).map_err(|_| "a secret here is text".to_string())?;
        st.secrets.insert(name.to_string(), value);
        Ok(())
    }

    async fn delete_secret(&self, name: &str) -> Res<bool> {
        self.call(format!("secret delete {name}"));
        let mut st = self.state.lock().unwrap();
        if st.down {
            return Err("podman isn't answering".into());
        }
        Ok(st.secrets.remove(name).is_some())
    }

    async fn fetch(&self, url: &str) -> Res<Vec<u8>> {
        // A container's published port (not a call: it's polled).
        if let Some(rest) = url.strip_prefix("http://127.0.0.1:") {
            let port = rest.split('/').next().unwrap_or_default();
            let st = self.state.lock().unwrap();
            let publish = format!("PublishPort=127.0.0.1:{port}:");
            let up = !st.sidecars_silent
                && st.containers.iter().any(|(name, c)| {
                    c.status == RUNNING
                        && st
                            .units
                            .get(&format!("{name}.container"))
                            .is_some_and(|u| u.lines().any(|l| l.starts_with(&publish)))
                });
            // Selkies' `/api/status` says what it's set to answer.
            let answer = match (url.ends_with("/api/status"), &st.sidecar_status) {
                (true, Some(status)) => status.clone().into_bytes(),
                _ => b"ok".to_vec(),
            };
            return if up { Ok(answer) } else { Err(format!("{url}: connection refused")) };
        }
        self.call(format!("fetch {url}"));
        let st = self.state.lock().unwrap();
        if st.offline {
            return Err("no network".into());
        }
        st.documents.get(url).cloned().ok_or_else(|| format!("{url}: HTTP 404"))
    }

    async fn container_status(&self, name: &str) -> Res<String> {
        let mut st = self.state.lock().unwrap();
        if st.down {
            return Err("podman isn't answering".into());
        }
        // A stopping one goes after a few looks.
        if let Some(c) = st.containers.get_mut(name)
            && c.status == STOPPING
        {
            c.gone_in = c.gone_in.saturating_sub(1);
            if c.gone_in == 0 {
                st.containers.remove(name);
            }
        }
        Ok(st.containers.get(name).map_or(MISSING.to_string(), |c| c.status.clone()))
    }

    async fn container_pid(&self, name: &str) -> Res<Option<u32>> {
        Ok(self.state.lock().unwrap().containers.get(name).map(|c| c.pid))
    }

    async fn container_logs(&self, name: &str, _lines: usize) -> Res<Option<String>> {
        Ok(self.state.lock().unwrap().containers.contains_key(name).then(|| "[svc-de] drawing\n".to_string()))
    }

    async fn desktop_up(&self, name: &str) -> Res<bool> {
        let mut st = self.state.lock().unwrap();
        in_vm(&st, name)?;
        let c = st.containers.get_mut(name).ok_or_else(|| format!("no container {name}"))?;
        if c.desktop_in == 0 {
            return Ok(true);
        }
        c.desktop_in -= 1;
        Ok(false)
    }

    async fn write_units(&self, files: &[(String, String)], remove: &[String]) -> Res<bool> {
        let mut st = self.state.lock().unwrap();
        let mut changed = false;
        for (n, t) in files {
            if st.units.get(n) != Some(t) {
                st.units.insert(n.clone(), t.clone());
                changed = true;
            }
        }
        for n in remove {
            if !files.iter().any(|(f, _)| f == n) && st.units.remove(n).is_some() {
                changed = true;
            }
        }
        if changed {
            st.reloads += 1;
            st.calls.push("reload".into());
        }
        Ok(changed)
    }

    async fn remove_volume(&self, name: &str) -> Res<bool> {
        self.call(format!("volume rm {name}"));
        let mut st = self.state.lock().unwrap();
        if let Some((c, _)) = st.containers.iter().find(|(_, c)| c.volumes.iter().any(|v| v == name)) {
            return Err(format!("volume {name} is being used by the following container(s): {c}"));
        }
        Ok(st.volumes.remove(name))
    }

    async fn volume_names(&self) -> Res<Vec<String>> {
        Ok(self.state.lock().unwrap().volumes.iter().cloned().collect())
    }

    async fn reset_failed(&self, unit: &str) -> Res<()> {
        self.call(format!("reset-failed {unit}"));
        Ok(())
    }

    async fn exec(&self, name: &str, user: &str, cmd: &[&str]) -> Res<ExecOut> {
        let line = cmd.join(" ");
        self.call(format!("exec {name} {user} {line}"));
        let st = self.state.lock().unwrap();
        if st.containers.get(name).is_none_or(|c| c.status != RUNNING) {
            return Err(format!("{name} isn't running"));
        }
        in_vm(&st, name)?;
        let (code, stdout) = st
            .exec_answers
            .iter()
            .find(|(start, _, _)| line.starts_with(start.as_str()))
            .map(|(_, c, o)| (*c, o.clone()))
            .unwrap_or((0, "{\"still_open\": []}".into()));
        Ok(ExecOut { code, stdout: stdout.into_bytes(), stderr: String::new() })
    }

    async fn start(&self, unit: &str) -> Res<()> {
        self.call(format!("start {unit}"));
        let mut st = self.state.lock().unwrap();
        let name = unit.strip_suffix(".service").ok_or("not a service")?;
        start_unit(&mut st, name, 0)
    }

    async fn stop(&self, unit: &str) -> Res<()> {
        self.call(format!("stop {unit}"));
        let name = unit.strip_suffix(".service").unwrap_or(unit);
        let mut st = self.state.lock().unwrap();
        if let Some(v) = name.strip_suffix("-volume")
            && st.volume_units.remove(v)
        {
            return Ok(());
        }
        stop_unit(&mut st, name);
        Ok(())
    }
}

/// A container running in a VM (its unit's `GlobalArgs=--runtime=`: a
/// secure stream's, libkrun) has no `podman exec`, as crun-krun hasn't.
fn in_vm(st: &State, name: &str) -> Res<()> {
    let vm = st
        .units
        .get(&format!("{name}.container"))
        .is_some_and(|u| u.lines().any(|l| l.starts_with("GlobalArgs=--runtime=")));
    if vm { Err(format!("{name}: OCI runtime error: the handler does not support exec")) } else { Ok(()) }
}

/// The units `name`'s is bound to (`BindsTo=<it>.service`).
fn bound_to(st: &State, name: &str) -> Vec<String> {
    let Some(text) = st.units.get(&format!("{name}.container")) else { return vec![] };
    text.lines().filter_map(|l| l.strip_prefix("BindsTo=")?.strip_suffix(".service")).map(String::from).collect()
}

/// Starts `name`'s container, after the units it's bound to (as systemd does).
fn start_unit(st: &mut State, name: &str, depth: u32) -> Res<()> {
    for b in bound_to(st, name) {
        if depth < 4 && st.containers.get(&b).is_none_or(|c| c.status != RUNNING) {
            start_unit(st, &b, depth + 1)?;
        }
    }
    let unit = format!("{name}.service");
    let text = st.units.get(&format!("{name}.container")).ok_or_else(|| format!("{unit}: not found"))?;
    let image = text.lines().find_map(|l| l.strip_prefix("Image=")).unwrap_or_default().to_string();
    if find(st, &image).is_none() {
        return Err(format!("{unit}: failed (its image {image} isn't here)"));
    }
    // Its quadlet volumes: each made (its oneshot active) as it starts.
    let mut volumes = vec![];
    for unit in text.lines().filter_map(|l| l.strip_prefix("Volume=")?.split(':').next()?.strip_suffix(".volume")) {
        let named = st
            .units
            .get(&format!("{unit}.volume"))
            .and_then(|v| v.lines().find_map(|l| l.strip_prefix("VolumeName=")).map(String::from));
        volumes.push(named.unwrap_or_else(|| format!("systemd-{unit}")));
        st.volume_units.insert(unit.to_string());
    }
    st.volumes.extend(volumes.iter().cloned());
    let desktop_in = st.desktop_after;
    let pid = 4000 + st.containers.len() as u32;
    let c = FakeContainer { image, status: RUNNING.into(), desktop_in, pid, volumes, gone_in: 0 };
    st.containers.insert(name.to_string(), c);
    Ok(())
}

/// Stops `name`'s container, what's bound to it, and what it's bound to that
/// nothing running needs now (`StopWhenUnneeded`).
fn stop_unit(st: &mut State, name: &str) {
    if st.containers.get(name).is_none_or(|c| c.status != RUNNING) {
        return;
    }
    // Gone at once, or (slow) once its status has been asked a few times.
    match st.slow_stops {
        0 => {
            st.containers.remove(name);
        }
        n => {
            let c = st.containers.get_mut(name).expect("there");
            (c.status, c.gone_in) = (STOPPING.into(), n);
        }
    }
    let running = |st: &State| -> Vec<String> {
        st.containers.iter().filter(|(_, c)| c.status == RUNNING).map(|(n, _)| n.clone()).collect()
    };
    let needing: Vec<String> = running(st).into_iter().filter(|c| bound_to(st, c).iter().any(|b| b == name)).collect();
    for c in needing {
        stop_unit(st, &c);
    }
    for b in bound_to(st, name) {
        let unneeded = st.units.get(&format!("{b}.container")).is_some_and(|t| t.contains("StopWhenUnneeded=yes"));
        let needed = running(st).iter().any(|c| bound_to(st, c).contains(&b));
        if unneeded && !needed {
            stop_unit(st, &b);
        }
    }
}

/// Screen access in memory: what was served, taken away and bridged.
#[derive(Default)]
pub struct FakeHands {
    /// Each socket made: (app id, its path, the compositor's inode).
    pub listens: Mutex<Vec<(String, std::path::PathBuf, u64)>>,
    pub removed: Mutex<Vec<std::path::PathBuf>>,
    pub bridges: Mutex<Vec<String>>,
    /// The compositor refuses sockets.
    pub refuse: std::sync::atomic::AtomicBool,
}

struct FakeBridge;

impl crate::screen::Running for FakeBridge {
    fn alive(&mut self) -> bool {
        true
    }
}

impl crate::screen::Hands for FakeHands {
    fn prepare(&self, dir: &std::path::Path) -> Res<()> {
        std::fs::create_dir_all(dir.join(wad_gen::units::PULSE)).map_err(|e| e.to_string())
    }

    fn listen(
        &self,
        comp: &crate::screen::Compositor,
        socket: &std::path::Path,
        app_id: &str,
        _instance: &str,
    ) -> Res<crate::screen::Guard> {
        if self.refuse.load(Ordering::SeqCst) {
            return Err("the compositor refused the socket".into());
        }
        self.listens.lock().unwrap().push((app_id.to_string(), socket.to_path_buf(), comp.inode));
        Ok(Box::new(()))
    }

    fn remove(&self, socket: &std::path::Path) {
        self.removed.lock().unwrap().push(socket.to_path_buf());
    }

    fn bind_pulse(&self, _src: &std::path::Path, _dst: &std::path::Path) -> Res<()> {
        Ok(())
    }

    fn unbind_pulse(&self, _dst: &std::path::Path) {}

    fn bridge(&self, _comp: &crate::screen::Compositor, id: &str, _pid: u32) -> Res<Box<dyn crate::screen::Running>> {
        self.bridges.lock().unwrap().push(id.to_string());
        Ok(Box::new(FakeBridge))
    }
}
