//! Packages on this machine (wad_gen::packages): an app's, a tool's or an
//! extension's files on their own, captured on the base. Getting what a
//! set's build needs, for each package in WadOS's order (what it needs
//! first), the first way that works:
//!
//! 1. the one CI published, when it's here already (its image id);
//! 2. downloaded from CI's registry, when `[packages] pull` is on and CI's
//!    index lists it for the base's ABI (a GitHub sign-in on the machine,
//!    the podman secret `github_token`, is used if there's one);
//! 3. the newest one here for the base's ABI (built here, or an older CI one);
//! 4. built here from its recipe (`wad_gen::packages::package_containerfile`),
//!    on the packages it needs.
//!
//! Each is named by its image id ([`packages::local_ref`]), as sets copy
//! from it. The index is fetched at most once an `index_max_age`, and kept
//! in `<state_dir>/packages/index.json` for when the network's out; with
//! no network and nothing saved, packages are built here. One fetch or
//! build of a package at a time (two builds that need rust wait for one),
//! and one package built here at a time, whichever (each takes the
//! machine's memory and processors; downloads go side by side).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use wad_gen::index::Index;
use wad_gen::{labels, packages};

use crate::backend::{Backend, ImageInfo, Res};

/// The user name a registry sees: ghcr checks the token, not the name.
const REGISTRY_USER: &str = "wadspaces";
/// The podman secret with the machine's GitHub sign-in.
pub const GITHUB_SECRET: &str = "github_token";
/// After the index couldn't be fetched, how long before trying again.
const RETRY_INDEX: Duration = Duration::from_secs(300);

pub struct Settings {
    /// Download what CI published; false: only what's here, or built here.
    pub pull: bool,
    pub index_url: String,
    pub index_max_age: Duration,
    pub state_dir: PathBuf,
    /// Free space a download leaves (the builds' own).
    pub min_free_gb: u64,
}

impl Settings {
    pub fn new(config: &wad_config::Config) -> Self {
        let p = &config.packages;
        Self {
            pull: p.pull,
            index_url: p.index_url.clone(),
            index_max_age: Duration::from_secs(p.index_max_age_min * 60),
            state_dir: config.daemon.state_dir.clone(),
            min_free_gb: config.daemon.build_min_free_gb,
        }
    }
}

/// The base a set is built on: its name, image id and ABI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base {
    pub image: String,
    pub id: String,
    pub abi: String,
}

/// How a package got here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// On this machine already.
    Here,
    /// Downloaded from CI's registry.
    Pulled,
    /// Built on this machine, now.
    Built,
}

/// A package a build can copy from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Got {
    pub id: String,
    /// `localhost/wados-pkg-<id>:<image id 12>`.
    pub image: String,
    pub how: How,
}

/// What `ensure` tells as it goes.
pub trait Watch: Send {
    /// It's on this package now.
    fn start(&mut self, _id: &str) {}
    fn line(&mut self, _line: &str) {}
    /// A download's bytes so far, and in all.
    fn bytes(&mut self, _id: &str, _done: u64, _total: u64) {}
    fn got(&mut self, _got: &Got) {}
}

/// Watching nothing.
pub struct Quiet;
impl Watch for Quiet {}

#[derive(Default)]
struct IndexState {
    got: Option<(Index, Instant, bool)>,
    failed: Option<Instant>,
}

/// A package here, as podman lists it.
#[derive(Debug, Clone)]
struct Local {
    image_id: String,
    tags: Vec<String>,
    created: f64,
}

pub struct Packages {
    backend: Arc<dyn Backend>,
    settings: Settings,
    index: tokio::sync::Mutex<IndexState>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    building_here: tokio::sync::Mutex<()>,
}

fn short(id: &str) -> &str {
    id.trim_start_matches("sha256:")
}

impl Packages {
    pub fn new(backend: Arc<dyn Backend>, settings: Settings) -> Arc<Self> {
        Arc::new(Self {
            backend,
            settings,
            index: tokio::sync::Mutex::default(),
            locks: Mutex::default(),
            building_here: tokio::sync::Mutex::default(),
        })
    }

    fn saved_index(&self) -> PathBuf {
        self.settings.state_dir.join("packages/index.json")
    }

    /// CI's index, when packages are pulled: fetched again once it's older
    /// than `index_max_age`; when that fails, the last one fetched (or saved).
    async fn index(&self) -> Option<Index> {
        if !self.settings.pull {
            return None;
        }
        let mut st = self.index.lock().await;
        let fresh =
            st.got.as_ref().is_some_and(|(_, at, fetched)| *fetched && at.elapsed() < self.settings.index_max_age);
        let resting = st.failed.is_some_and(|t| t.elapsed() < RETRY_INDEX);
        if !fresh && !resting {
            match self.download_index().await {
                Ok((index, bytes)) => {
                    if let Err(e) = crate::store::write_atomic_mkdir(&self.saved_index(), &bytes) {
                        tracing::info!("saving the packages' index: {e}");
                    }
                    st.got = Some((index, Instant::now(), true));
                    st.failed = None;
                }
                Err(e) => {
                    tracing::info!("the packages' index: {e}");
                    st.failed = Some(Instant::now());
                    if st.got.is_none() {
                        let saved = std::fs::read(self.saved_index()).ok().and_then(|b| parse_index(&b).ok());
                        st.got = saved.map(|i| (i, Instant::now(), false));
                    }
                }
            }
        }
        st.got.as_ref().map(|(i, _, _)| i.clone())
    }

    async fn download_index(&self) -> Res<(Index, Vec<u8>)> {
        let bytes = self.backend.fetch(&self.settings.index_url).await?;
        Ok((parse_index(&bytes).map_err(|e| format!("{}: {e}", self.settings.index_url))?, bytes))
    }

    /// The packages here for `abi`, by id, newest first.
    async fn here(&self, abi: &str) -> Res<HashMap<String, Vec<Local>>> {
        let mut out: HashMap<String, Vec<Local>> = HashMap::new();
        for i in self.backend.images_labelled(labels::PKG).await? {
            let (id, pkg_abi) = (label(&i, labels::PKG), label(&i, labels::PKG_ABI));
            if id.is_empty() || pkg_abi != abi {
                continue;
            }
            let list = out.entry(id.to_string()).or_default();
            let image_id = short(&i.id).to_string();
            // One image under two names is one package.
            if let Some(same) = list.iter_mut().find(|l| l.image_id == image_id) {
                same.tags.extend(i.tags.iter().cloned());
                continue;
            }
            list.push(Local { image_id, tags: i.tags.clone(), created: i.created });
        }
        for list in out.values_mut() {
            list.sort_by(|a, b| b.created.total_cmp(&a.created));
        }
        Ok(out)
    }

    fn lock_for(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks.lock().unwrap().entry(id.to_string()).or_default().clone()
    }

    /// Each of `ids`, and what they need, onto this machine for `base`:
    /// what a build copies from, in WadOS's order (needs first).
    pub async fn ensure(&self, ids: &[String], base: &Base, watch: &mut dyn Watch) -> Res<Vec<Got>> {
        let all = packages::closure(ids);
        if let Some(bad) = all.iter().find(|i| packages::source(i).is_none()) {
            return Err(format!("WadOS doesn't know how to package {bad:?}"));
        }
        let index = self.index().await;
        let mut got: Vec<Got> = vec![];
        for id in all {
            watch.start(&id);
            let lock = self.lock_for(&id);
            let _one = lock.lock().await;
            // After waiting: another build may have just got it.
            let here = self.here(&base.abi).await?.remove(&id).unwrap_or_default();
            let registry = index.as_ref().map_or(packages::REGISTRY, |i| i.registry.as_str()).to_string();
            let entry = index.as_ref().and_then(|i| i.get(&base.abi, &id)).cloned();
            let g = self.one(&id, base, entry.as_ref(), &registry, &here, &got, watch).await?;
            watch.got(&g);
            got.push(g);
        }
        Ok(got)
    }

    /// One package, the first way that works.
    #[allow(clippy::too_many_arguments)]
    async fn one(
        &self,
        id: &str,
        base: &Base,
        entry: Option<&wad_gen::index::Published>,
        registry: &str,
        here: &[Local],
        got: &[Got],
        watch: &mut dyn Watch,
    ) -> Res<Got> {
        if let Some(e) = entry {
            let want = short(&e.image_id);
            if let Some(l) = here.iter().find(|l| !want.is_empty() && l.image_id == want) {
                watch.line(&format!("» {}: on this machine", packages::name(id)));
                return self.named(id, l, How::Here).await;
            }
            if self.settings.pull {
                match self.download(id, e, registry, base, watch).await {
                    Ok(image_id) => {
                        let image = packages::local_ref(id, &image_id);
                        self.backend.tag(&image_id, &image).await?;
                        return Ok(Got { id: id.into(), image, how: How::Pulled });
                    }
                    Err(why) => watch.line(&format!("» {}: couldn't download it: {why}", packages::name(id))),
                }
            }
        }
        if let Some(l) = here.first() {
            let older = if entry.is_some() { " (not CI's latest)" } else { "" };
            watch.line(&format!("» {}: on this machine{older}", packages::name(id)));
            return self.named(id, l, How::Here).await;
        }
        self.build_here(id, base, got, watch).await
    }

    /// A package here, with its local_ref name.
    async fn named(&self, id: &str, l: &Local, how: How) -> Res<Got> {
        let image = packages::local_ref(id, &l.image_id);
        if !l.tags.contains(&image) {
            self.backend.tag(&l.image_id, &image).await?;
        }
        Ok(Got { id: id.into(), image, how })
    }

    /// Downloads CI's package; returns its image id.
    async fn download(
        &self,
        id: &str,
        e: &wad_gen::index::Published,
        registry: &str,
        base: &Base,
        watch: &mut dyn Watch,
    ) -> Res<String> {
        if let Some(free) = self.backend.free_gb().await
            && free - e.size as f64 / 1e9 < self.settings.min_free_gb as f64
        {
            return Err(format!("only {free:.1} GB free"));
        }
        let reference = if e.digest.starts_with("sha256:") {
            format!("{registry}/{}{id}@{}", packages::PREFIX, e.digest)
        } else {
            format!("{registry}/{}{id}:{}", packages::PREFIX, base.abi)
        };
        let server = reference.split('/').next().unwrap_or_default().to_string();
        let token = match self.backend.secret_value(GITHUB_SECRET).await {
            Ok(t) => t.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()),
            Err(_) => None,
        };
        let auth = token.map(|t| wad_podman::registry_auth(REGISTRY_USER, &t, &server));
        watch.line(&format!("» {}: downloading {:.1} MB", packages::name(id), e.size as f64 / 1e6));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let pull = self.backend.pull(
            &reference,
            auth,
            Box::new(move |d, t| {
                let _ = tx.send((d, t));
            }),
        );
        tokio::pin!(pull);
        let res = loop {
            tokio::select! {
                r = &mut pull => break r,
                Some((done, total)) = rx.recv() => watch.bytes(id, done, total),
            }
        };
        while let Ok((done, total)) = rx.try_recv() {
            watch.bytes(id, done, total);
        }
        let image_id = short(&res?).to_string();
        // It's the package it says it is.
        let info = self.backend.image_info(&image_id).await?.ok_or("it isn't here after its download")?;
        if label(&info, labels::PKG) != id {
            return Err(format!("{reference} isn't the {id} package"));
        }
        Ok(image_id)
    }

    /// Builds the package here from its recipe, on the packages it needs.
    async fn build_here(&self, id: &str, base: &Base, got: &[Got], watch: &mut dyn Watch) -> Res<Got> {
        let _one = match self.building_here.try_lock() {
            Ok(held) => held,
            Err(_) => {
                watch.line(&format!("» {}: waiting for another package's build here", packages::name(id)));
                self.building_here.lock().await
            }
        };
        let needs: Vec<(String, String)> = packages::closure(&packages::needs(id))
            .into_iter()
            .filter_map(|n| got.iter().find(|g| g.id == n).map(|g| (n, g.image.clone())))
            .collect();
        let file = packages::package_containerfile(id, &base.image, &needs, false).map_err(|e| e.to_string())?;
        let mut tar = crate::tar::Tar::new();
        tar.file("Dockerfile", 0o644, file.as_bytes())?;
        let building = format!("localhost/{}{id}:building", packages::PREFIX);
        let mut labels = Map::new();
        labels.insert(labels::PKG.into(), id.into());
        labels.insert(labels::PKG_ABI.into(), base.abi.clone().into());
        labels.insert(labels::PKG_BUILT_HERE.into(), "true".into());
        let mut args = Map::new();
        args.insert("BASE_IMAGE".into(), Value::String(base.image.clone()));
        watch.line(&format!("» {}: building it here", packages::name(id)));
        let built = build_lines(self.backend.as_ref(), tar.finish(), &building, args, labels, |l| watch.line(l))
            .await
            .map_err(|e| format!("building the {id} package: {e}"))?;
        let image_id = short(&built).to_string();
        let image = packages::local_ref(id, &image_id);
        self.backend.tag(&image_id, &image).await?;
        // images/build.sh's name for it too.
        self.backend.tag(&image_id, &format!("localhost/{}{id}:latest", packages::PREFIX)).await?;
        self.backend.remove_image(&building).await?;
        Ok(Got { id: id.into(), image, how: How::Built })
    }
}

fn parse_index(bytes: &[u8]) -> Result<Index, String> {
    let index: Index = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if index.version != wad_gen::index::VERSION {
        return Err(format!("an index of version {}, not one WadOS reads", index.version));
    }
    Ok(index)
}

pub fn label<'a>(i: &'a ImageInfo, k: &str) -> &'a str {
    i.labels.get(k).and_then(Value::as_str).unwrap_or_default()
}

/// A build whose lines go to `sink` as they come; its image id.
pub async fn build_lines(
    backend: &dyn Backend,
    context: Vec<u8>,
    tag: &str,
    args: Map<String, Value>,
    labels: Map<String, Value>,
    mut sink: impl FnMut(&str),
) -> Res<String> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let build = backend.build(
        context,
        tag,
        args,
        labels,
        Box::new(move |l| {
            let _ = tx.send(l.to_string());
        }),
    );
    tokio::pin!(build);
    let res = loop {
        tokio::select! {
            r = &mut build => break r,
            Some(l) = rx.recv() => sink(&l),
        }
    };
    while let Ok(l) = rx.try_recv() {
        sink(&l);
    }
    res
}
