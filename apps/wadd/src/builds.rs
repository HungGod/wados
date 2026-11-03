//! Building a wadspace's image here: its set (wad_gen::sets), a job of its
//! own (`POST /v1/wadspaces/{id}/build`).
//!
//! 1. **Its packages** (packages.rs): here, downloaded, or built here.
//! 2. **Stacked** as chain images, one package each on the chain under it,
//!    named by the base and the packages so far (`packages::chain_ref`),
//!    starting from the longest chain stored here (`packages::stack_order`):
//!    taking an app out copies again only what was above it, and a set
//!    with the same packages stacks nothing.
//! 3. **Its set**: the top chain put together (`wadspaces-pkg assemble`),
//!    then its own RUN and COPY steps (`SetBuild::containerfile`). A COPY
//!    takes in only what's in your home folder (`~/…`, or a path in it),
//!    at most `[daemon] copy_max_mb` in all; on a machine, only what you own.
//!
//! What each wadspace built last is recorded (`<state_dir>/built/<id>.json`:
//! its layer and image), so `Set::state` says built, update or never: an
//! edit to its packages or commands, or a newer base, makes another key,
//! and it shows as an update (running the image it has) until it's built
//! again. Nothing builds by itself. Wadspaces with the same key share one
//! image: a build records it for all of them, and a build of a set that's
//! here already is no build at all.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use wad_gen::sets::{Set, SetBuild, set_image};
use wad_gen::{BASE_IMAGE, labels, packages};
use wad_model::{BuildState, Built, JobKind, PartState, Wadspace};
use wad_proto::{ApiError, BuildAnswer, ErrorCode, Event, WadspaceBuild};

use crate::backend::{Backend, Res};
use crate::events::Bus;
use crate::jobs::{Jobs, now_ms};
use crate::library::Docs;
use crate::packages::{Base, Got, How, Packages, Watch, build_lines, label};
use crate::tar::{Tar, add_path};

/// Builds at once (the rest wait their turn, queued).
pub const AT_ONCE: usize = 2;

/// What a wadspace built last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    /// `set:<key>` (or `base`).
    pub layer: String,
    pub image: String,
    pub base_id: String,
    pub size_mb: u64,
    pub at: i64,
}

/// The records, in memory and in `<state_dir>/built/`.
struct Records {
    dir: PathBuf,
    map: Mutex<HashMap<String, Record>>,
}

impl Records {
    fn open(state_dir: &Path) -> Self {
        let dir = state_dir.join("built");
        let mut map = HashMap::new();
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            let Some(id) =
                p.file_stem().and_then(|s| s.to_str()).filter(|_| p.extension().is_some_and(|x| x == "json"))
            else {
                continue;
            };
            match std::fs::read(&p)
                .map_err(|e| e.to_string())
                .and_then(|b| serde_json::from_slice::<Record>(&b).map_err(|e| e.to_string()))
            {
                Ok(r) => {
                    map.insert(id.to_string(), r);
                }
                Err(e) => tracing::warn!("{}: {e}", p.display()),
            }
        }
        Self { dir, map: Mutex::new(map) }
    }

    fn get(&self, id: &str) -> Option<Record> {
        self.map.lock().unwrap().get(id).cloned()
    }

    fn all(&self) -> Vec<(String, Record)> {
        self.map.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    fn set(&self, id: &str, r: Record) {
        if crate::store::safe_id(id) {
            let bytes = serde_json::to_vec_pretty(&r).unwrap_or_default();
            if let Err(e) = crate::store::write_atomic_mkdir(&self.dir.join(format!("{id}.json")), &bytes) {
                tracing::warn!("recording {id}'s build: {e}");
            }
        }
        self.map.lock().unwrap().insert(id.to_string(), r);
    }

    fn remove(&self, id: &str) -> bool {
        if crate::store::safe_id(id) {
            let _ = std::fs::remove_file(self.dir.join(format!("{id}.json")));
        }
        self.map.lock().unwrap().remove(id).is_some()
    }
}

/// Whose home a COPY takes from, and (on a machine, as root) whose files only.
#[derive(Debug, Clone)]
pub struct Home {
    pub dir: PathBuf,
    pub owner: Option<u32>,
}

pub struct Settings {
    pub min_free_gb: u64,
    pub copy_max_bytes: u64,
}

pub struct Builds {
    backend: Arc<dyn Backend>,
    packages: Arc<Packages>,
    jobs: Arc<Jobs>,
    docs: Arc<Docs>,
    bus: Bus,
    records: Records,
    settings: Settings,
    home: Mutex<Option<Home>>,
    /// The base as last seen: (it, its size in MB).
    base: Mutex<Option<(Base, u64)>>,
    slots: tokio::sync::Semaphore,
    /// One request at a time between "is it building?" and its job made, so
    /// two asks for the same set make one build.
    asking: tokio::sync::Mutex<()>,
    published: Mutex<HashMap<String, WadspaceBuild>>,
    me: Weak<Self>,
}

fn conflict(e: impl Into<String>) -> ApiError {
    ApiError::new(ErrorCode::Conflict, e)
}

fn internal(e: impl Into<String>) -> ApiError {
    ApiError::new(ErrorCode::Internal, e)
}

impl Builds {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        backend: Arc<dyn Backend>,
        packages: Arc<Packages>,
        jobs: Arc<Jobs>,
        docs: Arc<Docs>,
        bus: Bus,
        state_dir: &Path,
        settings: Settings,
        home: Option<Home>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            backend,
            packages,
            jobs,
            docs,
            bus,
            records: Records::open(state_dir),
            settings,
            home: Mutex::new(home),
            base: Mutex::default(),
            slots: tokio::sync::Semaphore::new(AT_ONCE),
            asking: tokio::sync::Mutex::new(()),
            published: Mutex::default(),
            me: me.clone(),
        })
    }

    pub fn set_home(&self, home: Home) {
        *self.home.lock().unwrap() = Some(home);
    }

    // ------------------------------------------------------------ the base

    /// Looks at the base again; true if it changed (a newer one, or it came or went).
    pub async fn refresh_base(&self) -> bool {
        let now = match self.backend.image_info(BASE_IMAGE).await {
            Ok(Some(i)) => Some((
                Base { image: BASE_IMAGE.into(), id: i.id.clone(), abi: label(&i, labels::ABI).to_string() },
                i.size / 1_000_000,
            )),
            Ok(None) => None,
            // Podman isn't answering: what was seen stands.
            Err(_) => return false,
        };
        let mut base = self.base.lock().unwrap();
        let changed = *base != now;
        *base = now;
        changed
    }

    pub fn base(&self) -> Option<Base> {
        self.base.lock().unwrap().as_ref().map(|(b, _)| b.clone())
    }

    // ------------------------------------------------------------ states

    /// A wadspace's image here, as it is now.
    pub fn state_of(&self, ws: &Wadspace) -> WadspaceBuild {
        let base = self.base.lock().unwrap().clone();
        let set = Set::of(ws, base.as_ref().map_or("", |(b, _)| b.id.as_str()));
        let rec = self.records.get(&ws.id);
        let built = set.state(rec.as_ref().map(|r| r.layer.as_str()), base.is_some());
        let (image, size_mb) = match (&set.key, built) {
            (None, Built::Built) => (Some(BASE_IMAGE.to_string()), base.as_ref().map_or(0, |(_, s)| *s)),
            (Some(_), Built::Built | Built::Update) => match &rec {
                Some(r) => (Some(r.image.clone()), r.size_mb),
                None => (None, 0),
            },
            _ => (None, 0),
        };
        WadspaceBuild {
            id: ws.id.clone(),
            state: BuildState { built, size_mb },
            layer: set.layer(),
            image,
            job: set.key.as_deref().and_then(|k| self.jobs.running_with("key", k)),
            unpackaged: set.unpackaged,
        }
    }

    /// The image a wadspace runs when it's opened (none: it isn't built).
    pub fn image_for(&self, ws: &Wadspace) -> Option<String> {
        self.state_of(ws).image
    }

    /// Every wadspace's state here.
    pub fn states(&self) -> Vec<WadspaceBuild> {
        self.docs.live().iter().map(|w| self.state_of(w)).collect()
    }

    /// Watchers hear a wadspace's state, if it changed (gone: forgotten).
    pub fn publish(&self, id: &str) {
        let Some(ws) = self.docs.wadspace(id).filter(|w| !w.is_trashed()) else {
            self.published.lock().unwrap().remove(id);
            self.bus.forget(&format!("build:{id}"));
            return;
        };
        let st = self.state_of(&ws);
        let mut published = self.published.lock().unwrap();
        if published.get(id) == Some(&st) {
            return;
        }
        published.insert(id.to_string(), st.clone());
        drop(published);
        self.bus.publish(Event::Build(Box::new(st)));
    }

    pub fn publish_all(&self) {
        let live: Vec<String> = self.docs.live().into_iter().map(|w| w.id).collect();
        let gone: Vec<String> =
            self.published.lock().unwrap().keys().filter(|id| !live.contains(id)).cloned().collect();
        for id in live.iter().chain(gone.iter()) {
            self.publish(id);
        }
    }

    /// After an edit: what it changed, told again (a wadspace gone for good loses its record).
    pub fn edited(&self, changed: &[String]) {
        for id in changed {
            if self.docs.wadspace(id).is_none() {
                self.records.remove(id);
            }
            self.publish(id);
        }
    }

    /// The images the records name, checked: one that's gone is forgotten.
    pub async fn check_records(&self) {
        for (id, r) in self.records.all() {
            if let Ok(None) = self.backend.image_info(&r.image).await {
                tracing::info!("{id}: its image {} isn't here any more", r.image);
                self.records.remove(&id);
                self.publish(&id);
            }
        }
    }

    // ------------------------------------------------------------ building

    /// Builds a wadspace's set, unless it's built here already (then there's
    /// no job), or building (that build is the answer).
    pub async fn request(&self, id: &str) -> Result<BuildAnswer, ApiError> {
        let ws = self.docs.wadspace(id).ok_or_else(|| ApiError::not_found(format!("no wadspace {id:?}")))?;
        if ws.is_trashed() {
            return Err(conflict(format!("{} is in the Trash: restore it first", ws.name)));
        }
        self.refresh_base().await;
        let base = self.base_for_sets().await?;
        let set = Set::of(&ws, &base.id);
        let Some(key) = set.key.clone() else {
            // The base as it is: nothing to build.
            self.publish(id);
            return Ok(BuildAnswer { state: self.state_of(&ws), job: None });
        };
        // What can't be a step, and copies that can't be taken, are the edit's to fix.
        let probe = SetBuild { key: &key, from: BASE_IMAGE, base_id: &base.id, packages: &[], commands: &set.commands };
        let file = probe.containerfile().map_err(|e| ApiError::bad_request(e.to_string()))?;
        for s in &file.stage {
            self.resolve_copy(&s.from).map_err(ApiError::bad_request)?;
        }
        let _asking = self.asking.lock().await;
        if let Some(job) = self.jobs.running_with("key", &key) {
            return Ok(BuildAnswer { state: self.state_of(&ws), job: self.jobs.info(&job) });
        }
        let image = set_image(&key);
        if let Some(i) = self.backend.image_info(&image).await.map_err(internal)?
            && label(&i, labels::FROM) == base.id
        {
            tracing::info!("{id}: its set {image} is here already");
            self.record(&ws.id, &key, &image, &base.id, i.size / 1_000_000);
            return Ok(BuildAnswer { state: self.state_of(&ws), job: None });
        }
        let mut stages: Vec<(String, String)> =
            set.packages.iter().map(|p| (format!("pkg:{p}"), packages::name(p))).collect();
        if !set.packages.is_empty() {
            stages.push(("stack".into(), "Stacking its packages".into()));
        }
        stages
            .push(("set".into(), if set.commands.is_empty() { "Putting it together" } else { "Its own steps" }.into()));
        let job = self.jobs.create(JobKind::Build, &format!("Building {}", ws.name), id, &stages);
        self.jobs.update(&job, true, |j| {
            j.params.insert("key".into(), Value::String(key.clone()));
            j.params.insert("image".into(), Value::String(image.clone()));
        });
        let state = self.state_of(&ws);
        let me = self.me.upgrade().ok_or_else(|| internal("wadd is stopping"))?;
        let job_id = job.clone();
        let task = tokio::spawn(async move { me.run(job_id, ws.id, ws.name, set, base).await });
        self.jobs.attach(&job, task.abort_handle());
        self.publish(id);
        Ok(BuildAnswer { state, job: self.jobs.info(&job) })
    }

    /// A job's cancelled: what it was building is told again.
    pub fn cancelled(&self, subject: Option<&str>) {
        if let Some(id) = subject {
            self.publish(id);
        }
    }

    /// The base, if sets can be built on it.
    async fn base_for_sets(&self) -> Result<Base, ApiError> {
        let info = self.backend.image_info(BASE_IMAGE).await.map_err(|e| ApiError::new(ErrorCode::Offline, e))?;
        let Some(info) = info else {
            return Err(conflict(format!("{BASE_IMAGE} isn't on this machine: images/build.sh builds it")));
        };
        if !info.labels.contains_key(labels::EXTRAS) || label(&info, labels::ABI).is_empty() {
            return Err(conflict(format!("{BASE_IMAGE} is from before sets: build it again (images/build.sh)")));
        }
        Ok(Base { image: BASE_IMAGE.into(), id: info.id.clone(), abi: label(&info, labels::ABI).to_string() })
    }

    /// Records `image` as what `id` built, and as every wadspace's whose set it is now.
    fn record(&self, id: &str, key: &str, image: &str, base_id: &str, size_mb: u64) {
        let rec =
            Record { layer: format!("set:{key}"), image: image.into(), base_id: base_id.into(), size_mb, at: now_ms() };
        self.records.set(id, rec.clone());
        for w in self.docs.live() {
            if w.id != id && Set::of(&w, base_id).key.as_deref() == Some(key) {
                self.records.set(&w.id, rec.clone());
                self.publish(&w.id);
            }
        }
        self.publish(id);
    }

    async fn run(self: Arc<Self>, job: String, ws_id: String, name: String, set: Set, base: Base) {
        let _turn = self.slots.acquire().await;
        self.jobs.running(&job);
        self.publish(&ws_id);
        let started = std::time::Instant::now();
        let r = self.go(&job, &name, &set, &base).await;
        match r {
            Ok((image, size_mb)) => {
                self.jobs.line(&job, &format!("✓ built {image} in {:.0} s", started.elapsed().as_secs_f64()));
                self.jobs.finish(&job, Ok(()));
                // After the job's end: what they're told has no job in it.
                self.record(&ws_id, set.key.as_deref().unwrap_or_default(), &image, &base.id, size_mb);
                tracing::info!("{ws_id}: built {image} in {:.1} s", started.elapsed().as_secs_f64());
            }
            Err(e) => {
                tracing::warn!("{ws_id}: its build failed: {e}");
                self.jobs.finish(&job, Err(e));
            }
        }
        self.publish(&ws_id);
    }

    /// The build: its image and size when it's done.
    async fn go(&self, job: &str, name: &str, set: &Set, base: &Base) -> Res<(String, u64)> {
        let key = set.key.as_deref().ok_or("nothing to build")?;
        if let Some(free) = self.backend.free_gb().await
            && free < self.settings.min_free_gb as f64
        {
            return Err(format!("only {free:.1} GB free; building needs at least {} GB", self.settings.min_free_gb));
        }
        let n = set.packages.len();
        self.jobs.line(
            job,
            &format!(
                "» {name}: {n} package{}{}",
                if n == 1 { "" } else { "s" },
                if n > 0 { format!(": {}", set.packages.join(", ")) } else { String::new() }
            ),
        );
        if !set.unpackaged.is_empty() {
            self.jobs
                .line(job, &format!("» not packaged for Fedora yet, so built without: {}", set.unpackaged.join(", ")));
        }
        let mut watch = JobWatch { jobs: self.jobs.clone(), job: job.to_string(), current: String::new() };
        let got = self.packages.ensure(&set.packages, base, &mut watch).await?;
        let pairs = self.stack(job, base, &got).await?;

        self.jobs.stage(job, "set", PartState::Working, None);
        let from = if pairs.is_empty() { BASE_IMAGE.to_string() } else { packages::chain_ref(&base.id, &pairs) };
        let file = SetBuild { key, from: &from, base_id: &base.id, packages: &pairs, commands: &set.commands }
            .containerfile()
            .map_err(|e| e.to_string())?;
        let mut tar = Tar::new();
        tar.file("Dockerfile", 0o644, file.containerfile.as_bytes())?;
        for s in &file.stage {
            let path = self.resolve_copy(&s.from)?;
            let owner = self.home.lock().unwrap().as_ref().and_then(|h| h.owner);
            let allowed = move |m: &std::fs::Metadata| {
                use std::os::unix::fs::MetadataExt;
                owner.is_none_or(|o| m.uid() == o)
            };
            add_path(&mut tar, &path, &s.to, self.settings.copy_max_bytes, &allowed)
                .map_err(|e| format!("COPY {}: {e}", s.from))?;
            self.jobs.line(job, &format!("» taking in {} ({} KB so far)", s.from, tar.bytes >> 10));
        }
        let image = set_image(key);
        self.jobs.line(
            job,
            &format!(
                "» {} → {image}",
                if pairs.is_empty() { "its own steps" } else { "putting its packages together, then its own steps" }
            ),
        );
        let mut args = Map::new();
        args.insert("BASE_IMAGE".into(), Value::String(from.clone()));
        let (jobs, j) = (self.jobs.clone(), job.to_string());
        build_lines(self.backend.as_ref(), tar.finish(), &image, args, Map::new(), move |l| {
            jobs.line(&j, l);
            if let Some((a, b)) = step(l) {
                jobs.progress(&j, "set", f64::from(a.saturating_sub(1)) / f64::from(b.max(1)));
            }
        })
        .await?;
        let info =
            self.backend.image_info(&image).await?.ok_or_else(|| format!("{image} isn't here after its build"))?;
        self.jobs.stage(job, "set", PartState::Done, None);
        Ok((image, info.size / 1_000_000))
    }

    /// Its packages stacked on the base: (id, image) bottom first, each
    /// chain image built unless it's here.
    async fn stack(&self, job: &str, base: &Base, got: &[Got]) -> Res<Vec<(String, String)>> {
        if got.is_empty() {
            return Ok(vec![]);
        }
        self.jobs.stage(job, "stack", PartState::Working, None);
        let stored = self.stored_chains(&base.id, got).await?;
        let ids: Vec<String> = got.iter().map(|g| g.id.clone()).collect();
        let pairs: Vec<(String, String)> = packages::stack_order(&ids, &stored)
            .into_iter()
            .filter_map(|p| got.iter().find(|g| g.id == p).map(|g| (g.id.clone(), g.image.clone())))
            .collect();
        let mut have = 0;
        for i in (0..pairs.len()).rev() {
            if self.backend.image_info(&packages::chain_ref(&base.id, &pairs[..=i])).await?.is_some() {
                have = i + 1;
                break;
            }
        }
        if have > 0 {
            self.jobs.line(job, &format!("» {have} of them stacked already, by an earlier build"));
        }
        let (mut from, mut from_id) = if have == 0 {
            (BASE_IMAGE.to_string(), base.id.clone())
        } else {
            let top = packages::chain_ref(&base.id, &pairs[..have]);
            let id = self.backend.image_info(&top).await?.map(|i| i.id).ok_or_else(|| format!("{top} went away"))?;
            (top, id)
        };
        for i in have..pairs.len() {
            let chain = &pairs[..=i];
            let tag = packages::chain_ref(&base.id, chain);
            let (pid, pref) = &pairs[i];
            self.jobs.line(job, &format!("» stacking {}", packages::name(pid)));
            let mut tar = Tar::new();
            tar.file("Dockerfile", 0o644, packages::chain_step(&from, &from_id, pref, chain).as_bytes())?;
            let (jobs, j) = (self.jobs.clone(), job.to_string());
            build_lines(self.backend.as_ref(), tar.finish(), &tag, Map::new(), Map::new(), move |l| jobs.line(&j, l))
                .await?;
            from_id = self
                .backend
                .image_info(&tag)
                .await?
                .map(|i| i.id)
                .ok_or_else(|| format!("{tag} isn't here after its build"))?;
            from = tag;
            self.jobs.progress(job, "stack", (i + 1 - have) as f64 / (pairs.len() - have) as f64);
        }
        self.jobs.stage(job, "stack", PartState::Done, None);
        Ok(pairs)
    }

    /// Chains stored on this base made of the packages as `got` has them:
    /// each one's packages, bottom first.
    async fn stored_chains(&self, base_id: &str, got: &[Got]) -> Res<Vec<Vec<String>>> {
        let mut out = vec![];
        for i in self.backend.images_labelled(labels::CHAIN).await? {
            // A set inherits its top chain's label.
            if i.labels.contains_key(labels::LAYER) {
                continue;
            }
            let pairs: Option<Vec<(String, String)>> = label(&i, labels::CHAIN)
                .split(',')
                .map(|p| {
                    let (id, tag) = p.split_once('@')?;
                    let r = packages::local_ref(id, tag);
                    got.iter().any(|g| g.id == id && g.image == r).then(|| (id.to_string(), r))
                })
                .collect();
            let Some(pairs) = pairs.filter(|p| !p.is_empty()) else { continue };
            if i.tags.contains(&packages::chain_ref(base_id, &pairs)) {
                out.push(pairs.into_iter().map(|(id, _)| id).collect());
            }
        }
        Ok(out)
    }

    /// Where a COPY's source is: in your home folder (`~/…`, a path in it,
    /// or one relative to it), links followed only as far as it.
    pub fn resolve_copy(&self, from: &str) -> Res<PathBuf> {
        let home = self.home.lock().unwrap().clone().ok_or("there's no home folder to copy from")?;
        let path = match from {
            "~" => home.dir.clone(),
            f if f.starts_with("~/") => home.dir.join(&f[2..]),
            f if f.starts_with('/') => PathBuf::from(f),
            f => home.dir.join(f),
        };
        let real = path.canonicalize().map_err(|e| format!("{from}: {e}"))?;
        let home_real = home.dir.canonicalize().map_err(|e| format!("{}: {e}", home.dir.display()))?;
        if !real.starts_with(&home_real) {
            return Err(format!(
                "{from} isn't in your home folder ({}): a build takes only what's there",
                home.dir.display()
            ));
        }
        Ok(real)
    }
}

/// `STEP 3/7: RUN …` → (3, 7).
fn step(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("STEP ")?;
    let (frac, _) = rest.split_once(':')?;
    let (a, b) = frac.split_once('/')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// A build's packages as they come, on its job's stages.
struct JobWatch {
    jobs: Arc<Jobs>,
    job: String,
    current: String,
}

impl Watch for JobWatch {
    fn start(&mut self, id: &str) {
        self.current = format!("pkg:{id}");
        self.jobs.stage(&self.job, &self.current, PartState::Working, None);
    }
    fn line(&mut self, line: &str) {
        self.jobs.line(&self.job, line);
        if let Some((a, b)) = step(line) {
            self.jobs.progress(&self.job, &self.current, f64::from(a.saturating_sub(1)) / f64::from(b.max(1)));
        }
    }
    fn bytes(&mut self, id: &str, done: u64, total: u64) {
        self.jobs.bytes(&self.job, &format!("pkg:{id}"), done, total);
    }
    fn got(&mut self, got: &Got) {
        let how = match got.how {
            How::Here => "here",
            How::Pulled => "downloaded",
            How::Built => "built here",
        };
        self.jobs.line(&self.job, &format!("» {}: {how} ({})", packages::name(&got.id), got.image));
        self.jobs.stage(&self.job, &format!("pkg:{}", got.id), PartState::Done, None);
    }
}
