//! Storage (phase 8, Settings ▸ Storage): the disk wadd keeps images on,
//! and Clean Up ([`crate::api_phase8::Storage`]).
//!
//! **The disk**: its size and what's free (statvfs, on `/var` on a machine:
//! podman's storage, wadd's state and people's homes are all there), then
//! what podman says it keeps (`podman system df -v`), counted as:
//!
//! - **base images**: the base and the stream sidecar (their names here,
//!   `updates::Settings::bases`), all of each;
//! - **wadspaces**: each wadspace's image here (its set's: what it adds to
//!   the base; one shared by two counts once in all) and what it keeps (its
//!   `/config` volume, `wad-<id>-config`);
//! - **other**: everything else used (the system, people's files, packages
//!   and chains kept for builds, leftovers).
//!
//! Each of this machine's wadspaces (not in the Trash) with its bytes and
//! whether it's built, biggest first. It's measured at most every 30
//! seconds (podman counts the volumes' files), and told (the `storage`
//! event) when wadd starts, after a cleanup, when the bases change
//! ([`MachineStorage::poke`]) and every 10 minutes if it changed.
//!
//! **Clean Up** is a Cleanup job ([`proto::storage::CLEANUP`]) that removes
//! what belongs to nothing:
//!
//! 1. *images*: set images no wadspace here has (in the Trash too: it can
//!    come back), then the images nothing names or stands on (dangling), and
//!    the chains no set here would restack from ([`Inner::old_chains`]: a
//!    tagged image isn't dangling, so podman's prune leaves it);
//! 2. *containers*: stopped containers that aren't a wadspace's;
//! 3. *caches*: what unfinished builds left (buildah's containers, and the
//!    images under them).
//!
//! Never a named image anything uses (the bases, a wadspace's set, the
//! packages and chains a set here still stacks from), a volume (a
//! wadspace's home, its keys, its streams), or any file of the person's.
//! What it freed (the free space it made, else what podman says it removed)
//! is a notice.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;
use wad_model::{JobKind, PartState};
use wad_podman::Df;
use wad_proto as proto;
use wad_proto::{ApiError, Event, JobInfo, Level, WadspaceStorage};

use crate::backend::{ImageInfo, Res};
use crate::builds::Builds;
use crate::events::Bus;
use crate::jobs::Jobs;
use crate::library::Docs;

/// How long a measure is handed out before it's measured again.
const FRESH: Duration = Duration::from_secs(30);
const GB: f64 = 1e9;

/// What wadd measures and cleans the disk with: a trait, so it's tested against a fake.
#[async_trait]
pub trait Disk: Send + Sync + 'static {
    /// The filesystem at `path`: its size and what's free, in bytes.
    fn space(&self, path: &Path) -> Res<(u64, u64)>;
    /// `podman system df -v`.
    async fn df(&self) -> Res<Df>;
    /// Every image, with its names and labels.
    async fn images(&self) -> Res<Vec<ImageInfo>>;
    /// Removes an image, all its names with it (an error while it's used).
    async fn remove_image(&self, id: &str) -> Res<bool>;
    /// Dangling images (`external`: and builds' leftovers): what went, (id, bytes).
    async fn prune_images(&self, external: bool) -> Res<Vec<(String, u64)>>;
    /// Stopped containers but those labelled `keep_label`: what went.
    async fn prune_containers(&self, keep_label: &str) -> Res<Vec<(String, u64)>>;
}

/// statvfs: (size, free to anyone), in bytes.
pub fn statvfs(path: &Path) -> Res<(u64, u64)> {
    let s = nix::sys::statvfs::statvfs(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let frag = s.fragment_size() as u64;
    Ok((s.blocks() as u64 * frag, s.blocks_available() as u64 * frag))
}

/// The machine's podman.
pub struct PodmanDisk(pub wad_podman::Podman);

#[async_trait]
impl Disk for PodmanDisk {
    fn space(&self, path: &Path) -> Res<(u64, u64)> {
        statvfs(path)
    }

    async fn df(&self) -> Res<Df> {
        self.0.system_df().await.map_err(|e| e.to_string())
    }

    async fn images(&self) -> Res<Vec<ImageInfo>> {
        self.0.images_all().await.map_err(|e| e.to_string())
    }

    async fn remove_image(&self, id: &str) -> Res<bool> {
        self.0.remove_image_id(id).await.map_err(|e| e.to_string())
    }

    async fn prune_images(&self, external: bool) -> Res<Vec<(String, u64)>> {
        self.0.prune_images(external).await.map_err(|e| e.to_string())
    }

    async fn prune_containers(&self, keep_label: &str) -> Res<Vec<(String, u64)>> {
        self.0.prune_containers(keep_label).await.map_err(|e| e.to_string())
    }
}

pub struct Settings {
    /// The filesystem measured (`/var` on a machine; your home on a laptop).
    pub disk: PathBuf,
    /// The base images' names here.
    pub bases: Vec<String>,
}

impl Settings {
    pub fn new(config: &wad_config::Config, profile: wad_config::Profile) -> Self {
        let disk = match profile {
            wad_config::Profile::System => PathBuf::from("/var"),
            wad_config::Profile::User => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/".into()),
        };
        let bases = crate::updates::Settings::new(config, profile).bases.into_iter().map(|b| b.local).collect();
        Self { disk, bases }
    }
}

/// Cheap to clone: one service, shared.
#[derive(Clone)]
pub struct MachineStorage(Arc<Inner>);

struct Inner {
    disk: Arc<dyn Disk>,
    docs: Arc<Docs>,
    builds: Arc<Builds>,
    jobs: Arc<Jobs>,
    bus: Bus,
    settings: Settings,
    last: Mutex<Option<(Instant, proto::Storage)>>,
    /// What was told last.
    told: Mutex<Option<proto::Storage>>,
    job: Mutex<Option<String>>,
}

fn label<'a>(i: &'a ImageInfo, k: &str) -> &'a str {
    i.labels.get(k).and_then(Value::as_str).unwrap_or_default()
}

/// An image's names, as podman lists them (`localhost/x:1`, sometimes `x:1`).
fn named(i: &ImageInfo, name: &str) -> bool {
    i.tags.iter().any(|t| t == name || format!("localhost/{t}") == name)
}

/// A set image (`io.wadspaces.layer` is `set:<key>`).
fn is_set(i: &ImageInfo) -> bool {
    label(i, wad_gen::labels::LAYER).starts_with("set:")
}

/// The packages a set stacks, in order (`io.wadspaces.pkgs`: `id@image-id,…`).
fn pkgs_of(label: &str) -> Vec<&str> {
    label.split(',').filter_map(|p| p.split('@').next()).filter(|p| !p.is_empty()).collect()
}

/// The `io.wadspaces.chain` label's packages, in order, if `i` is a chain
/// (not a set: a set inherits its top chain's label).
fn chain_pkgs(i: &ImageInfo) -> Option<Vec<&str>> {
    let chain = label(i, wad_gen::labels::CHAIN);
    if chain.is_empty() || is_set(i) {
        return None;
    }
    Some(pkgs_of(chain))
}

/// The disk as Storage tells it, from what was measured.
pub fn measure(
    (total, free): (u64, u64),
    df: &Df,
    images: &[ImageInfo],
    bases: &[String],
    // Each wadspace: its id, its image here (if any) and whether it's built.
    wadspaces: &[(String, Option<String>, wad_model::Built)],
) -> proto::Storage {
    let find = |name: &str| images.iter().find(|i| named(i, name) || i.id == name);
    let mut base_ids = HashSet::new();
    let mut bases_bytes = 0;
    for b in bases {
        if let Some(i) = find(b)
            && base_ids.insert(i.id.clone())
        {
            bases_bytes += i.size;
        }
    }
    let base_size = bases.first().and_then(|b| find(b)).map_or(0, |i| i.size);
    let volume = |id: &str| {
        let name = format!("wad-{id}-config");
        df.volumes.iter().find(|v| v.name == name).map_or(0, |v| v.size)
    };
    let mut counted = HashSet::new();
    let mut wadspaces_bytes = 0;
    let mut per: Vec<WadspaceStorage> = wadspaces
        .iter()
        .map(|(id, image, built)| {
            let img = image.as_deref().and_then(find).filter(|i| !base_ids.contains(&i.id));
            let adds = img.map_or(0, |i| i.size.saturating_sub(base_size));
            if let Some(i) = img
                && counted.insert(i.id.clone())
            {
                wadspaces_bytes += adds;
            }
            let keeps = volume(id);
            wadspaces_bytes += keeps;
            WadspaceStorage { id: id.clone(), bytes: adds + keeps, built: *built }
        })
        .collect();
    per.sort_by_key(|w| std::cmp::Reverse(w.bytes));
    let used = total.saturating_sub(free);
    // What's measured can't be more than what's used (a guess at the rest, then).
    let bases_bytes = bases_bytes.min(used);
    let wadspaces_bytes = wadspaces_bytes.min(used - bases_bytes);
    proto::Storage {
        total,
        free,
        bases: bases_bytes,
        wadspaces: wadspaces_bytes,
        other: used - bases_bytes - wadspaces_bytes,
        per_wadspace: per,
    }
}

/// What a cleanup's notice says: its title, body and icon.
pub fn freed_words(freed: u64, removed: usize) -> (String, &'static str, &'static str) {
    if freed == 0 && removed == 0 {
        ("Nothing to clean up".into(), "There are no leftovers on this machine.", "check")
    } else {
        (
            format!("Freed {:.1} GB", freed as f64 / GB),
            "Old build layers, stopped containers and caches are gone.",
            "sparkles",
        )
    }
}

impl MachineStorage {
    pub fn new(
        settings: Settings,
        disk: Arc<dyn Disk>,
        docs: Arc<Docs>,
        builds: Arc<Builds>,
        jobs: Arc<Jobs>,
        bus: Bus,
    ) -> Self {
        Self(Arc::new(Inner {
            disk,
            docs,
            builds,
            jobs,
            bus,
            settings,
            last: Mutex::default(),
            told: Mutex::default(),
            job: Mutex::default(),
        }))
    }

    /// Told when wadd has settled, then every 10 minutes if it changed.
    pub fn start(&self) -> Vec<tokio::task::JoinHandle<()>> {
        let me = self.clone();
        vec![tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            loop {
                me.0.tell(false).await;
                tokio::time::sleep(Duration::from_secs(600)).await;
            }
        })]
    }

    /// Something changed on the disk (the bases, a build): measured again and told.
    pub fn poke(&self) {
        let me = self.clone();
        tokio::spawn(async move { me.0.tell(true).await });
    }
}

impl Inner {
    async fn measure(&self) -> Result<proto::Storage, ApiError> {
        let space = self.disk.space(&self.settings.disk).map_err(ApiError::internal)?;
        let offline = |e: String| ApiError::new(wad_proto::ErrorCode::Offline, e);
        let df = self.disk.df().await.map_err(offline)?;
        let images = self.disk.images().await.map_err(offline)?;
        let ws: Vec<_> = self
            .docs
            .live()
            .iter()
            .map(|w| {
                let s = self.builds.state_of(w);
                (w.id.clone(), s.image, s.state.built)
            })
            .collect();
        Ok(measure(space, &df, &images, &self.settings.bases, &ws))
    }

    async fn storage(&self, fresh: bool) -> Result<proto::Storage, ApiError> {
        if !fresh
            && let Some((at, s)) = &*self.last.lock().unwrap()
            && at.elapsed() < FRESH
        {
            return Ok(s.clone());
        }
        let s = self.measure().await?;
        *self.last.lock().unwrap() = Some((Instant::now(), s.clone()));
        Ok(s)
    }

    /// Measured (again, `fresh`) and told, if it changed.
    async fn tell(&self, fresh: bool) {
        match self.storage(fresh).await {
            Ok(s) => {
                let mut told = self.told.lock().unwrap();
                if told.as_ref() != Some(&s) {
                    *told = Some(s.clone());
                    drop(told);
                    self.bus.publish(Event::Storage(Box::new(s)));
                }
            }
            Err(e) => tracing::debug!("storage: {}", e.message),
        }
    }

    /// The images a wadspace of this machine's has (in the Trash too): its set's.
    fn kept(&self) -> HashSet<String> {
        let lib = self.docs.snapshot();
        lib.wadspaces
            .iter()
            .filter(|w| w.machine == self.docs.machine())
            .filter_map(|w| self.builds.state_of(w).image)
            .collect()
    }

    /// The chains a build would still restack from, removed: every chain
    /// image that isn't a prefix of some set that's here built on the same
    /// base. A build that doesn't find a chain just makes it again (one
    /// layer per package). An image another is built on is left (podman
    /// refuses it): the chains still on the way go first.
    async fn old_chains(&self, images: &[ImageInfo]) -> Vec<(String, u64)> {
        // A set is built from a chain on its own base (sets.rs's FROM: the
        // chain's image id). A chain on another one can't be restacked.
        let mut want = HashSet::new();
        for i in images.iter().filter(|i| is_set(i)) {
            let base = label(i, wad_gen::labels::FROM);
            let pkgs = pkgs_of(label(i, wad_gen::labels::PKGS));
            for n in 1..=pkgs.len() {
                want.insert((base.to_string(), pkgs[..n].join(",")));
            }
        }
        // Top of a chain first: the images under it are on its way.
        let mut chains: Vec<&ImageInfo> = images.iter().filter(|i| chain_pkgs(i).is_some()).collect();
        chains.sort_by_key(|i| std::cmp::Reverse(i.parent.is_empty()));
        let mut gone = vec![];
        for i in chains {
            let Some(pkgs) = chain_pkgs(i) else { continue };
            let base = label(i, wad_gen::labels::FROM);
            if want.contains(&(base.to_string(), pkgs.join(","))) {
                continue;
            }
            match self.disk.remove_image(&i.id).await {
                Ok(true) => gone.push((i.id.clone(), i.size)),
                Ok(false) => {}
                // Another image is built on it: it's still on the way.
                Err(_) => {}
            }
        }
        gone
    }

    /// The cleanup's stages: what went, in bytes podman says, and how many things.
    async fn clean(&self, job: &str) -> Result<(u64, usize), String> {
        let mut reported = 0;
        let mut removed = 0;
        let mut add = |list: Vec<(String, u64)>, jobs: &Jobs, what: &str| {
            for (id, size) in &list {
                jobs.line(job, &format!("» removed {what} {}", id.trim_start_matches("sha256:")));
                reported += size;
            }
            removed += list.len();
        };
        // 1. Sets no wadspace has, then what nothing names.
        self.jobs.stage(job, "images", PartState::Working, None);
        let kept = self.kept();
        let images = self.disk.images().await?;
        let mut sets = vec![];
        for i in images.iter().filter(|i| is_set(i) && !kept.iter().any(|k| named(i, k) || &i.id == k)) {
            match self.disk.remove_image(&i.id).await {
                Ok(true) => sets.push((i.id.clone(), i.size)),
                Ok(false) => {}
                // In use (a container runs it): it stays.
                Err(e) => self.jobs.line(job, &format!("» kept {}: {e}", i.tags.first().unwrap_or(&i.id))),
            }
        }
        add(sets, &self.jobs, "the set");
        add(self.disk.prune_images(false).await?, &self.jobs, "the image");
        // The chains on the way to the sets that are here. podman's prune
        // doesn't take a tagged image, so they'd stay for good otherwise.
        add(self.old_chains(&images).await, &self.jobs, "the chain");
        self.jobs.stage(job, "images", PartState::Done, None);
        // 2. Stopped containers that aren't a wadspace's.
        self.jobs.stage(job, "containers", PartState::Working, None);
        add(self.disk.prune_containers(wad_gen::labels::WADSPACE).await?, &self.jobs, "the container");
        self.jobs.stage(job, "containers", PartState::Done, None);
        // 3. What unfinished builds left.
        self.jobs.stage(job, "caches", PartState::Working, None);
        add(self.disk.prune_images(true).await?, &self.jobs, "the build's");
        self.jobs.stage(job, "caches", PartState::Done, None);
        Ok((reported, removed))
    }
}

#[async_trait]
impl crate::api_phase8::Storage for MachineStorage {
    async fn storage(&self) -> Result<proto::Storage, ApiError> {
        self.0.storage(false).await
    }

    async fn cleanup(&self) -> Result<JobInfo, ApiError> {
        let me = &self.0;
        let job = {
            let mut j = me.job.lock().unwrap();
            if let Some(id) = &*j {
                return Err(ApiError::conflict(format!("A cleanup is under way already ({id})")));
            }
            let stages: Vec<(String, String)> =
                proto::storage::CLEANUP.iter().map(|(s, l)| (s.to_string(), l.to_string())).collect();
            let id = me.jobs.create(JobKind::Cleanup, "Cleaning up", "storage", &stages);
            *j = Some(id.clone());
            id
        };
        let (inner, id) = (me.clone(), job.clone());
        let task = tokio::spawn(async move {
            struct Gone(Arc<Inner>);
            impl Drop for Gone {
                fn drop(&mut self) {
                    *self.0.job.lock().unwrap() = None;
                }
            }
            let gone = Gone(inner.clone());
            inner.jobs.running(&id);
            let before = inner.disk.space(&inner.settings.disk).map(|(_, f)| f).unwrap_or(0);
            let r = inner.clean(&id).await;
            let after = inner.disk.space(&inner.settings.disk).map(|(_, f)| f).unwrap_or(0);
            drop(gone);
            match r {
                Ok((reported, removed)) => {
                    let made = after.saturating_sub(before);
                    let freed = if made > 0 { made } else { reported };
                    inner.jobs.finish(&id, Ok(()));
                    inner.tell(true).await;
                    let (title, body, icon) = freed_words(freed, removed);
                    let level = if removed == 0 { Level::Info } else { Level::Ok };
                    inner.bus.publish(Event::Notice {
                        level,
                        title,
                        undo: None,
                        body: Some(body.into()),
                        icon: Some(icon.into()),
                        action: None,
                        toast: true,
                    });
                }
                Err(e) => inner.jobs.finish(&id, Err(e)),
            }
        });
        me.jobs.attach(&job, task.abort_handle());
        me.jobs.info(&job).ok_or_else(|| ApiError::internal("the job went"))
    }
}

#[cfg(test)]
mod tests;
