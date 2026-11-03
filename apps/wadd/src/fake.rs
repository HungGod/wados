//! A machine in memory, for tests: podman's images and containers, a
//! registry to pull from, systemd's units. A build makes an image from its
//! Dockerfile's last stage (the labels of the image it's FROM, its LABEL
//! lines, the labels it's given); starting `wad-<id>.service` runs
//! `wad-<id>` from its unit's `Image=`. Every call that changes something
//! is in `calls`, in order.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::backend::{Backend, ImageInfo, MISSING, OnBytes, OnLine, RUNNING, Res};

#[derive(Debug, Clone, Default)]
pub struct FakeContainer {
    pub image: String,
    pub status: String,
    /// Polls of `desktop_up` left before its desktop is up.
    pub desktop_in: u32,
    pub pid: u32,
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
    pub free_gb: Option<f64>,
}

pub struct Fake {
    pub state: Mutex<State>,
    seq: AtomicU64,
    /// While set, builds wait (to cancel one, or watch one run).
    pub hold: tokio::sync::Semaphore,
    holding: std::sync::atomic::AtomicBool,
}

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
            FakeContainer { image: image.into(), status: RUNNING.into(), desktop_in: 0, pid: 9000 },
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

    async fn fetch(&self, url: &str) -> Res<Vec<u8>> {
        self.call(format!("fetch {url}"));
        let st = self.state.lock().unwrap();
        if st.offline {
            return Err("no network".into());
        }
        st.documents.get(url).cloned().ok_or_else(|| format!("{url}: HTTP 404"))
    }

    async fn container_status(&self, name: &str) -> Res<String> {
        let st = self.state.lock().unwrap();
        if st.down {
            return Err("podman isn't answering".into());
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

    async fn start(&self, unit: &str) -> Res<()> {
        self.call(format!("start {unit}"));
        let mut st = self.state.lock().unwrap();
        let name = unit.strip_suffix(".service").ok_or("not a service")?;
        let text = st.units.get(&format!("{name}.container")).ok_or_else(|| format!("{unit}: not found"))?;
        let image = text.lines().find_map(|l| l.strip_prefix("Image=")).unwrap_or_default().to_string();
        if find(&st, &image).is_none() {
            return Err(format!("{unit}: failed (its image {image} isn't here)"));
        }
        let desktop_in = st.desktop_after;
        let pid = 4000 + st.containers.len() as u32;
        st.containers.insert(name.to_string(), FakeContainer { image, status: RUNNING.into(), desktop_in, pid });
        Ok(())
    }

    async fn stop(&self, unit: &str) -> Res<()> {
        self.call(format!("stop {unit}"));
        let name = unit.strip_suffix(".service").unwrap_or(unit);
        self.state.lock().unwrap().containers.remove(name);
        Ok(())
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
