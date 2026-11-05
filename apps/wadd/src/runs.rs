//! The wadspaces running on this machine (v1's registry.rs and launches.rs,
//! on wad_gen's quadlets): opening one, stopping it, and checking every few
//! seconds that what wadd thinks matches what podman says (reconcile).
//!
//! **Opening** (`POST /v1/wadspaces/{id}/open {display}`), one thing at a
//! time per wadspace (its lock):
//!
//! 1. its image: the one it built last, or the base (builds.rs); one never
//!    built doesn't open;
//! 2. its projects (projects.rs), each a stage of its open job (Opening
//!    {name}): GitHub's cloned, this machine's folders found, files copied
//!    in, another machine's fetched or mounted over Taildrive. One that
//!    can't be here leaves it without that project, and the job says why;
//! 3. its extra folder (`wad_gen::extras::files`, plus `display` and
//!    `screens`, and its wallpaper's picture), its keys' folders (empty
//!    until something signs in);
//! 4. its screen folder, with the socket the compositor serves it alone (screen.rs);
//! 5. its unit (`wad_gen::units::units`) in `units_dir`, systemd reloaded;
//!    then `wad-<id>.service` started;
//! 6. **ready** once its desktop is up: its container runs and its nested
//!    labwc answers on its own socket, which it makes only once it has
//!    connected to the socket wadd served it, just before its window
//!    appears ([`Backend::desktop_up`], polled).
//!
//! How it's shown (`screen`, `window`, `background`) is in its run state,
//! which the shell follows to place its window: full screen, a window,
//! minimized. wadd never places windows. Opening one that runs already
//! only changes that.
//!
//! **Reconcile**: a container that stopped by itself (its window closed:
//! its desktop halts it) goes idle; one found running that wadd didn't
//! start (wadd restarted) is taken on, as it was shown before.
//!
//! Once it's stopped, its Taildrive mounts go, and what wadd made for
//! projects it doesn't have any more (projects.rs's tidy); all of it when
//! it's gone for good.
//!
//! **Streamed** ([`Runs::open_streamed`], streams.rs's): it draws on its
//! stream sidecar instead of the screen (wad_gen's units with a
//! [`wad_gen::Sidecar`]: the wadspace bound to its sidecar, their display
//! volume), with no screen folder, socket or compositor. Opened on the
//! screen while it's streamed, the person at the machine wins: its stream
//! ends ([`StreamEnds`]), then it opens here.
//!
//! **Moving as it runs** (v1's): on a machine with a screen, a wadspace whose
//! image can move (its `io.wadspaces.display` label, "2": the base's) opens
//! on the screen with `wad_gen::units::moving_units` (both displays
//! mounted), so streaming it takes it off the screen without restarting it
//! ([`Runs::add_sidecar`], then [`Runs::switch`]: `wadspaces-session
//! switch` closes its apps as their × would and its desktop starts again on
//! the stream; terminals keep their jobs), and back again.
//!
//! **Streamed securely** ([`StreamUnit::vm`]): it runs in a VM of its own
//! (libkrun) with its stream's Selkies inside it (`wad_gen::units::secure_units`).
//! Nothing execs into a VM (`podman exec` isn't there), so it's ready once
//! Selkies' health answers on its published port, not by its desktop's
//! socket; and it never moves (it stops, and opens on the screen again).
//!
//! Not yet (later phases): devices, ~/Drives, Open-with files, runs of one
//! time, focus.

use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use async_trait::async_trait;
use tokio::task::JoinHandle;
use wad_gen::units::{self, container_name, display_name, service};
use wad_gen::{Machine, Mounts, Run, extras};
use wad_model::{PartState, Phase, RunState, SessionDisplay, Wadspace, Wallpaper};
use wad_proto::{ApiError, ErrorCode, Event, Level, WadspaceRun};

use crate::backend::{Backend, RUNNING, Res};
use crate::builds::Builds;
use crate::events::Bus;
use crate::jobs::now_ms;
use crate::library::Docs;
use crate::packages::GITHUB_SECRET;
use crate::projects::{Opening, Projects};
use crate::screen::Screen;

/// A streamed wadspace's sidecar, for its units (streams.rs makes it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamUnit {
    /// The stream image.
    pub image: String,
    pub guid: String,
    pub web_port: u16,
    pub transport: units::Transport,
    /// wadd's folder for the stream (its token table and ICE servers).
    pub dir: String,
    /// Its link's origin.
    pub origin: Option<String>,
    /// A secure stream's VM; none: the wadspace beside its sidecar.
    pub vm: Option<SecureVm>,
}

/// A secure stream's VM: its runtime and size (`wad_gen::units::Vm`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecureVm {
    /// `[streaming] secure_runtime` (`krun`), found here with KVM.
    pub runtime: String,
    pub cpus: u32,
    pub ram_mib: u32,
}

impl StreamUnit {
    fn sidecar(&self) -> units::Sidecar<'_> {
        units::Sidecar {
            image: &self.image,
            guid: &self.guid,
            web_port: self.web_port,
            transport: self.transport,
            dir: &self.dir,
            origin: self.origin.as_deref(),
        }
    }
}

/// What ends a wadspace's stream when the person at the machine opens it (streams.rs).
#[async_trait]
pub trait StreamEnds: Send + Sync + 'static {
    /// `id` was opened on the screen (or in a window, or the background)
    /// while it's streamed: its stream ends, and its wadspace stops.
    async fn opened_on_screen(&self, id: &str, display: SessionDisplay);
}

/// An image whose wadspace's desktop can move between the screen and its
/// stream as it runs (images/base: svc-de, wadspaces-session, tmux): "2".
pub const MOVES_LABEL: &str = "io.wadspaces.display";

/// There when NVIDIA's own kernel driver runs the machine's card.
const NVIDIA_DRIVER: &str = "/proc/driver/nvidia/version";
/// The machine's DRM devices (its render nodes: `renderD*`).
const DRM_CLASS: &str = "/sys/class/drm";
/// A wallpaper's picture, at most.
const MAX_WALLPAPER: u64 = 16 << 20;

pub struct Settings {
    pub state_dir: PathBuf,
    pub signins_dir: PathBuf,
    pub ready_timeout: Duration,
    /// Between readiness checks.
    pub poll: Duration,
    /// What every wadspace's units get from this machine (its pulse folder
    /// and secrets are filled in as each opens).
    pub machine: Machine,
    /// Where `~` is, for a wallpaper's picture.
    pub home: Option<PathBuf>,
    /// The uid that owns keys' folders when wadd is root (abc's, 1000).
    pub keys_uid: Option<u32>,
}

impl Settings {
    /// The machine as wadd's config describes it.
    pub fn new(config: &wad_config::Config, profile: wad_config::Profile) -> Self {
        let d = &config.daemon;
        let root = nix::unistd::geteuid().is_root();
        let machine = Machine {
            state_dir: d.state_dir.to_string_lossy().into_owned(),
            signins_dir: d.signins_dir.to_string_lossy().into_owned(),
            tz: time_zone(),
            secrets: vec![],
            // Rootless podman: you're abc (1000) inside.
            rootless_uid: (!root).then_some(d.projects_uid),
            // Root inside a machine's wadspace isn't root on it.
            userns_base: (root && profile == wad_config::Profile::System).then_some(units::USERNS_BASE),
            devices: if Path::new("/dev/dri").exists() { vec!["/dev/dri".into()] } else { vec![] },
            device_rules: vec![],
            screen_dir: config.display.enabled.then(|| d.screen_dir.to_string_lossy().into_owned()),
            pulse_dir: None,
            screens: 1,
            software_render: Path::new(NVIDIA_DRIVER).exists() || no_3d(Path::new(DRM_CLASS)),
        };
        let home = match profile {
            wad_config::Profile::User => std::env::var_os("HOME").map(PathBuf::from),
            wad_config::Profile::System => {
                nix::unistd::User::from_uid(d.projects_uid.into()).ok().flatten().map(|u| u.dir)
            }
        };
        Self {
            state_dir: d.state_dir.clone(),
            signins_dir: d.signins_dir.clone(),
            ready_timeout: Duration::from_secs(d.ready_timeout_s.max(1)),
            poll: Duration::from_secs(1),
            machine,
            home,
            keys_uid: root.then_some(d.projects_uid),
        }
    }
}

/// A GPU whose render nodes can't draw in 3D: a VM's virtio GPU without
/// virgl (VIRTIO_GPU_F_VIRGL, its first feature bit, off). Mesa has no
/// driver for its render node, and a wadspace's labwc (wlroots, on the
/// machine's Wayland) tries GLES2 and Vulkan on it, then gives up rather than
/// draw with pixman. False when there's no render node at all: wlroots
/// draws in software by itself then.
fn no_3d(drm: &Path) -> bool {
    let nodes: Vec<PathBuf> = std::fs::read_dir(drm)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("renderD"))
        .map(|e| e.path())
        .collect();
    !nodes.is_empty() && nodes.iter().all(|n| virtio_gpu_without_virgl(&n.join("device")))
}

/// `device` (a render node's), or a virtio device under it (its PCI
/// device's `virtio<n>`), is virtio_gpu with its virgl bit off.
fn virtio_gpu_without_virgl(device: &Path) -> bool {
    let driver = |d: &Path| std::fs::read_link(d.join("driver")).ok().and_then(|l| l.file_name().map(|n| n.to_owned()));
    let mut candidates = vec![device.to_path_buf()];
    candidates.extend(
        std::fs::read_dir(device)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("virtio"))
            .map(|e| e.path()),
    );
    candidates.iter().any(|d| {
        driver(d).is_some_and(|n| n == "virtio_gpu")
            && std::fs::read_to_string(d.join("features")).is_ok_and(|f| f.trim_start().starts_with('0'))
    })
}

/// The machine's time zone (/etc/localtime's link), empty when it's UTC or unknown.
fn time_zone() -> String {
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| p.to_string_lossy().split_once("zoneinfo/").map(|(_, z)| z.to_string()))
        .filter(|z| !z.is_empty() && !z.contains(char::is_control))
        .unwrap_or_default()
}

pub struct Runs {
    backend: Arc<dyn Backend>,
    docs: Arc<Docs>,
    bus: Bus,
    screen: Arc<Screen>,
    builds: Arc<Builds>,
    settings: Settings,
    states: Mutex<HashMap<String, WadspaceRun>>,
    /// How each was shown, kept across wadd's restarts (`runs.json`).
    shown: Mutex<BTreeMap<String, SessionDisplay>>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    tasks: Mutex<HashMap<String, JoinHandle<()>>>,
    /// Its projects, made here as it opens ([`Runs::attach`]); none: it opens without them.
    projects: OnceLock<Arc<Projects>>,
    /// What ends its stream when it's opened on the screen ([`Runs::attach_streams`]).
    streams: OnceLock<Weak<dyn StreamEnds>>,
    /// The wadspaces running with moving units (started so here): they can
    /// move between the screen and a stream as they run.
    moving: Mutex<std::collections::HashSet<String>>,
    /// The wadspaces running in a secure stream's VM (started so here, or
    /// taken on by streams.rs after a restart): never exec'd into.
    secure: Mutex<std::collections::HashSet<String>>,
    me: Weak<Self>,
}

fn busy(p: Phase) -> bool {
    matches!(p, Phase::Starting | Phase::Waiting | Phase::Stopping)
}

fn conflict(e: impl Into<String>) -> ApiError {
    ApiError::new(ErrorCode::Conflict, e)
}

impl Runs {
    pub fn new(
        backend: Arc<dyn Backend>,
        docs: Arc<Docs>,
        bus: Bus,
        screen: Arc<Screen>,
        builds: Arc<Builds>,
        settings: Settings,
    ) -> Arc<Self> {
        let shown = std::fs::read(settings.state_dir.join("runs.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Arc::new_cyclic(|me| Self {
            backend,
            docs,
            bus,
            screen,
            builds,
            settings,
            states: Mutex::default(),
            shown: Mutex::new(shown),
            locks: Mutex::default(),
            tasks: Mutex::default(),
            projects: OnceLock::new(),
            streams: OnceLock::new(),
            moving: Mutex::default(),
            secure: Mutex::default(),
            me: me.clone(),
        })
    }

    /// Wadspaces open with their projects from now on.
    pub fn attach(&self, projects: Arc<Projects>) {
        let _ = self.projects.set(projects);
    }

    /// Opening a streamed wadspace on the screen ends its stream from now on.
    pub fn attach_streams(&self, streams: Weak<dyn StreamEnds>) {
        let _ = self.streams.set(streams);
    }

    fn lock_for(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks.lock().unwrap().entry(id.into()).or_default().clone()
    }

    /// A wadspace's run as it is now (idle if wadd knows nothing of it).
    pub fn state(&self, id: &str) -> WadspaceRun {
        self.states.lock().unwrap().get(id).cloned().unwrap_or_else(|| WadspaceRun {
            id: id.into(),
            state: RunState { since: now_ms(), ..Default::default() },
            display: None,
        })
    }

    /// Every wadspace's here, this machine's in order, then any others still running.
    pub fn states(&self) -> Vec<WadspaceRun> {
        let mut ids: Vec<String> = self.docs.live().into_iter().map(|w| w.id).collect();
        let mut extra: Vec<String> = self.states.lock().unwrap().keys().filter(|k| !ids.contains(k)).cloned().collect();
        extra.sort();
        ids.extend(extra);
        ids.iter().map(|id| self.state(id)).collect()
    }

    /// Changes a run: a new phase clears the error (unless it's one) and,
    /// out of the busy ones, the message. Watchers hear it if it changed.
    fn update(&self, id: &str, f: impl FnOnce(&mut WadspaceRun)) {
        let changed = {
            let mut states = self.states.lock().unwrap();
            let st = states.entry(id.to_string()).or_insert_with(|| WadspaceRun {
                id: id.into(),
                state: RunState { since: now_ms(), ..Default::default() },
                display: None,
            });
            let before = st.clone();
            f(st);
            if st.state.phase != before.state.phase {
                st.state.since = now_ms();
                if st.state.phase != Phase::Error {
                    st.state.error = None;
                }
                if !busy(st.state.phase) {
                    st.state.message = None;
                }
                // Its open job, while it opens (and when it failed: why).
                if matches!(st.state.phase, Phase::Ready | Phase::Idle) {
                    st.state.job = None;
                }
            }
            (*st != before).then(|| st.clone())
        };
        if let Some(st) = changed {
            if matches!(st.state.phase, Phase::Idle | Phase::Error) {
                self.moving.lock().unwrap().remove(id);
                self.secure.lock().unwrap().remove(id);
            }
            self.keep_shown(id, st.display);
            self.bus.publish(Event::Run(Box::new(st)));
            self.screen.poke();
        }
    }

    fn keep_shown(&self, id: &str, display: Option<SessionDisplay>) {
        let mut shown = self.shown.lock().unwrap();
        let before = shown.get(id).copied();
        match display {
            Some(d) => shown.insert(id.into(), d),
            None => shown.remove(id),
        };
        if shown.get(id).copied() != before {
            let bytes = serde_json::to_vec_pretty(&*shown).unwrap_or_default();
            if let Err(e) = crate::store::write_atomic_mkdir(&self.settings.state_dir.join("runs.json"), &bytes) {
                tracing::debug!("runs.json: {e}");
            }
        }
    }

    /// Every wadspace's state, for a new start of wadd.
    pub fn publish_all(&self) {
        for w in self.docs.live() {
            let st = self.state(&w.id);
            self.states.lock().unwrap().entry(w.id.clone()).or_insert_with(|| st.clone());
            self.bus.publish(Event::Run(Box::new(st)));
        }
    }

    /// What's on the screen: (id, its desktop is up).
    pub fn on_screen(&self) -> Vec<(String, bool)> {
        self.states
            .lock()
            .unwrap()
            .values()
            .filter(|s| matches!(s.state.phase, Phase::Starting | Phase::Waiting | Phase::Ready))
            .filter(|s| s.display.is_none_or(|d| d != SessionDisplay::Stream))
            .map(|s| (s.id.clone(), s.state.phase == Phase::Ready))
            .collect()
    }

    // ------------------------------------------------------------ open

    /// Opens a wadspace, shown as `display`: it starts in the background
    /// (its run state says how it's going). One that's up already is only
    /// shown another way.
    pub fn open(&self, id: &str, display: SessionDisplay) -> Result<WadspaceRun, ApiError> {
        if display == SessionDisplay::Stream {
            return Err(ApiError::bad_request(
                "a stream starts with POST /v1/wadspaces/{id}/stream (streams.rs): open it on the screen, in a window or in the background",
            ));
        }
        let ws = self.docs.wadspace(id).ok_or_else(|| ApiError::not_found(format!("no wadspace {id:?}")))?;
        if ws.is_trashed() {
            return Err(conflict(format!("{} is in the Trash: restore it first", ws.name)));
        }
        let st = self.state(id);
        match st.state.phase {
            Phase::Starting | Phase::Waiting | Phase::Ready if st.display == Some(SessionDisplay::Stream) => {
                return self.off_its_stream(id, display);
            }
            Phase::Starting | Phase::Waiting | Phase::Ready => {
                self.update(id, |s| s.display = Some(display));
                return Ok(self.state(id));
            }
            Phase::Stopping => return Err(conflict(format!("{} is stopping", ws.name))),
            Phase::Idle | Phase::Error => {}
        }
        let image = self
            .builds
            .image_for(&ws)
            .ok_or_else(|| conflict(format!("{} isn't built on this machine yet: build it first", ws.name)))?;
        if self.screen.compositor().is_none() {
            return Err(ApiError::new(
                ErrorCode::Offline,
                "there's no screen to show it on: the shell hasn't said where its compositor is, and none was found",
            ));
        }
        self.start_up(id, display, image, None)
    }

    /// The person at the machine opened a streamed wadspace: they win. Its
    /// stream ends (its wadspace stops), then it opens as they asked.
    fn off_its_stream(&self, id: &str, display: SessionDisplay) -> Result<WadspaceRun, ApiError> {
        let me = self.me.upgrade().ok_or_else(|| ApiError::internal("wadd is stopping"))?;
        let streams = self.streams.get().and_then(Weak::upgrade);
        tracing::info!("{id}: opened at the machine, so its stream ends");
        self.update(id, |s| s.state.message = Some("taking it off its stream".into()));
        let i = id.to_string();
        tokio::spawn(async move {
            if let Some(s) = streams {
                s.opened_on_screen(&i, display).await;
            }
            // Still streamed (no stream to end, or it didn't move back): it stops here.
            let st = me.state(&i);
            if st.display == Some(SessionDisplay::Stream)
                && matches!(st.state.phase, Phase::Starting | Phase::Waiting | Phase::Ready | Phase::Stopping)
            {
                let _ = me.stop(&i).await;
            }
            if let Err(e) = me.open(&i, display) {
                tracing::warn!("{i}: it didn't open after its stream: {}", e.message);
                me.bus.publish(Event::notice(Level::Warn, e.message, None));
            }
        });
        Ok(self.state(id))
    }

    /// Opens a wadspace on its stream sidecar (streams.rs's Stream job): it
    /// starts in the background, as `open` does; one that's up already is
    /// stopped first by the caller (a conflict here).
    pub fn open_streamed(&self, id: &str, stream: StreamUnit) -> Result<WadspaceRun, ApiError> {
        let ws = self.docs.wadspace(id).ok_or_else(|| ApiError::not_found(format!("no wadspace {id:?}")))?;
        if ws.is_trashed() {
            return Err(conflict(format!("{} is in the Trash: restore it first", ws.name)));
        }
        match self.state(id).state.phase {
            Phase::Starting | Phase::Waiting | Phase::Ready => {
                return Err(conflict(format!("{} is open already", ws.name)));
            }
            Phase::Stopping => return Err(conflict(format!("{} is stopping", ws.name))),
            Phase::Idle | Phase::Error => {}
        }
        let image = self
            .builds
            .image_for(&ws)
            .ok_or_else(|| conflict(format!("{} isn't built on this machine yet: build it first", ws.name)))?;
        self.start_up(id, SessionDisplay::Stream, image, Some(stream))
    }

    /// Starting it in the background, shown as `display` (on its stream: `stream`).
    fn start_up(
        &self,
        id: &str,
        display: SessionDisplay,
        image: String,
        stream: Option<StreamUnit>,
    ) -> Result<WadspaceRun, ApiError> {
        let me = self.me.upgrade().ok_or_else(|| ApiError::internal("wadd is stopping"))?;
        // Starting, and its bring-up known, as one step: the Starting event
        // wakes reconcile (on another thread), which must find the task, not
        // a wadspace starting with no container yet that it'd take for stopped.
        let mut tasks = self.tasks.lock().unwrap();
        self.update(id, |s| {
            s.state.phase = Phase::Starting;
            s.state.message = Some("getting it ready".into());
            s.state.error = None;
            s.state.job = None;
            s.display = Some(display);
        });
        let (i, img) = (id.to_string(), image);
        let task = tokio::spawn(async move { me.bring_up(i, img, stream).await });
        tasks.insert(id.into(), task);
        drop(tasks);
        Ok(self.state(id))
    }

    /// Waits for a wadspace's bring-up, if one is going on (tests, trials).
    pub async fn settled(&self, id: &str) {
        let task = self.tasks.lock().unwrap().remove(id);
        if let Some(t) = task {
            let _ = t.await;
        }
    }

    async fn bring_up(self: Arc<Self>, id: String, image: String, stream: Option<StreamUnit>) {
        let lock = self.lock_for(&id);
        let _held = lock.lock().await;
        if let Err(e) = self.bring_up_locked(&id, &image, stream.as_ref()).await {
            tracing::warn!("{id}: it didn't open: {e}");
            if let Some(p) = self.projects.get() {
                if let Some(job) = self.state(&id).state.job {
                    p.jobs().finish(&job, Err(e.clone()));
                }
                // Not running: its Taildrive mounts go.
                if self.backend.container_status(&container_name(&id)).await.is_ok_and(|s| s != RUNNING) {
                    p.release(&id).await;
                }
            }
            self.update(&id, |s| {
                s.state.phase = Phase::Error;
                s.state.error = Some(e);
            });
        }
    }

    /// Its projects, here (and what it no longer has, gone): what its unit
    /// mounts, and its open job (a wadspace with no projects has none).
    async fn prepare_projects(&self, ws: &Wadspace) -> (Mounts, Option<Opening>) {
        let Some(p) = self.projects.get() else {
            if !ws.look.projects.is_empty() {
                let names: Vec<&str> = ws.look.projects.iter().map(|p| p.as_name.as_str()).collect();
                tracing::info!("{}: projects ({}) aren't made here: it opens without them", ws.id, names.join(", "));
            }
            return (Mounts::new(), None);
        };
        p.tidy(ws).await;
        if ws.look.projects.is_empty() {
            return (Mounts::new(), None);
        }
        let opening = p.open_job(ws).await;
        self.update(&ws.id, |s| s.state.job = Some(opening.job.clone()));
        let id = ws.id.clone();
        let say = |m: &str| self.update(&id, |s| s.state.message = Some(m.to_string()));
        let prepared = p.prepare(ws, &opening, &say).await;
        for l in &prepared.left_out {
            let title = if l.why.contains(&l.name) { l.why.clone() } else { format!("{}: {}", l.name, l.why) };
            self.bus.publish(Event::notice(Level::Warn, title, None));
        }
        (prepared.mounts, Some(opening))
    }

    async fn bring_up_locked(&self, id: &str, image: &str, stream: Option<&StreamUnit>) -> Res<()> {
        let ws = self.docs.wadspace(id).ok_or("it's gone")?;
        let name = container_name(id);
        let status = self.backend.container_status(&name).await?;
        let mut job: Option<Opening> = None;
        if status != RUNNING {
            let (mounts, opening) = self.prepare_projects(&ws).await;
            job = opening;
            let stage = |stage: &str, state: PartState| {
                if let (Some(p), Some(o)) = (self.projects.get(), job.as_ref()) {
                    p.stage(&o.job, stage, state);
                }
            };
            stage("container", PartState::Working);
            let wallpaper = self.write_extras(&ws, &mounts, stream.is_some())?;
            self.make_key_folders(&ws)?;
            let mut machine = self.settings.machine.clone();
            // On the screen: its screen folder (its socket, sound). Streamed, its sidecar's display.
            if stream.is_none() {
                let screen = self.screen.clone();
                let i = id.to_string();
                tokio::task::spawn_blocking(move || screen.prepare(&i)).await.map_err(|e| e.to_string())??;
                machine.pulse_dir = self.screen.pulse_dir().map(|p| p.to_string_lossy().into_owned());
            }
            machine.secrets = self
                .backend
                .secret_names()
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|n| n == GITHUB_SECRET)
                .collect();
            let run = Run { image, mounts: &mounts, wallpaper: wallpaper.as_ref() };
            let sidecar = stream.map(StreamUnit::sidecar);
            let vm = stream.and_then(|s| s.vm.as_ref());
            let moves = stream.is_none() && machine.screen_dir.is_some() && self.image_moves(image).await;
            let files = match (sidecar.as_ref(), vm) {
                (Some(s), Some(vm)) => {
                    // The stream image's files, made again from the image (one
                    // left from before would be an old image's).
                    self.drop_secure_volume(id).await;
                    let vm = units::Vm { runtime: &vm.runtime, cpus: vm.cpus, ram_mib: vm.ram_mib };
                    units::secure_units(&ws, &run, &machine, s, &vm).map_err(|e| e.to_string())?
                }
                _ if moves => units::moving_units(&ws, &run, &machine, None),
                _ => units::units(&ws, &run, &machine, sidecar.as_ref()),
            };
            // No sidecar's units from a stream before (a moving one keeps its
            // volume), nor a secure stream's image volume.
            let display = display_name(id);
            let secure_volume = format!("{}.volume", units::secure_volume(id));
            let gone = match (stream, moves) {
                (Some(_), _) if vm.is_some() => vec![format!("{display}.container"), format!("{display}.volume")],
                (Some(_), _) => vec![secure_volume],
                (None, true) => vec![format!("{display}.container"), secure_volume],
                (None, false) => vec![format!("{display}.container"), format!("{display}.volume"), secure_volume],
            };
            self.backend.write_units(&files, &gone).await?;
            {
                let mut secure = self.secure.lock().unwrap();
                match vm.is_some() {
                    true => secure.insert(id.to_string()),
                    false => secure.remove(id),
                };
            }
            {
                let mut moving = self.moving.lock().unwrap();
                match moves {
                    true => moving.insert(id.to_string()),
                    false => moving.remove(id),
                };
            }
            self.update(id, |s| s.state.message = Some("starting its container".into()));
            tracing::info!("{id}: starting {} on {image}", service(id));
            self.backend.start(&service(id)).await?;
            stage("container", PartState::Done);
        }
        let jobs = self.projects.get().map(|p| p.jobs().clone());
        let open_job = job.as_ref().map(|o| o.job.clone());
        if let (Some(jobs), Some(j)) = (&jobs, &open_job) {
            jobs.stage(j, "desktop", PartState::Working, None);
        }
        self.update(id, |s| {
            s.state.phase = Phase::Waiting;
            s.state.message = Some("waiting for its desktop".into());
        });
        match stream.filter(|s| s.vm.is_some()) {
            // In its VM: Selkies, inside it beside its desktop, answering.
            Some(s) => self.wait_served(&ws_name(&self.docs, id), &name, s).await?,
            None => self.wait_desktop(&ws_name(&self.docs, id), &name).await?,
        }
        if let (Some(jobs), Some(j)) = (&jobs, &open_job) {
            jobs.finish(j, Ok(()));
        }
        self.update(id, |s| s.state.phase = Phase::Ready);
        tracing::info!("{id}: ready");
        Ok(())
    }

    /// Until its desktop is up, or it stops, or it's taken too long.
    async fn wait_desktop(&self, title: &str, container: &str) -> Res<()> {
        let deadline = tokio::time::Instant::now() + self.settings.ready_timeout;
        loop {
            match self.backend.desktop_up(container).await {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(e) => tracing::debug!("{container}: {e}"),
            }
            if self.backend.container_status(container).await.is_ok_and(|s| s != RUNNING) {
                let logs = self.backend.container_logs(container, 15).await.ok().flatten().unwrap_or_default();
                let last: Vec<&str> = logs.lines().rev().take(3).collect();
                let why = last.into_iter().rev().collect::<Vec<_>>().join(" / ");
                return Err(format!(
                    "{title} stopped before its desktop was up{}",
                    if why.is_empty() { String::new() } else { format!(": {why}") }
                ));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("{title}'s desktop wasn't up within {} s", self.settings.ready_timeout.as_secs()));
            }
            tokio::time::sleep(self.settings.poll).await;
        }
    }

    /// Until a secure stream's Selkies answers on its published port (it
    /// serves once the VM, its desktop's s6 and Selkies are up), or its VM
    /// stops, or it's taken too long. Never `podman exec`: a VM hasn't it.
    async fn wait_served(&self, title: &str, container: &str, s: &StreamUnit) -> Res<()> {
        let url = format!("http://127.0.0.1:{}{}/api/health", s.web_port, units::stream_path(&s.guid));
        let deadline = tokio::time::Instant::now() + self.settings.ready_timeout;
        loop {
            if self.backend.fetch(&url).await.is_ok() {
                return Ok(());
            }
            if self.backend.container_status(container).await.is_ok_and(|s| s != RUNNING) {
                let logs = self.backend.container_logs(container, 15).await.ok().flatten().unwrap_or_default();
                let last: Vec<&str> = logs.lines().rev().take(3).collect();
                let why = last.into_iter().rev().collect::<Vec<_>>().join(" / ");
                return Err(format!(
                    "{title}'s VM stopped before its stream was up{}",
                    if why.is_empty() { String::new() } else { format!(": {why}") }
                ));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "{title}'s stream wasn't up in its VM within {} s",
                    self.settings.ready_timeout.as_secs()
                ));
            }
            tokio::time::sleep(self.settings.poll).await;
        }
    }

    /// A secure stream's image volume, gone: its unit (a oneshot that stays
    /// active) first, or podman would make an empty volume in its place the
    /// next time the wadspace starts.
    pub async fn drop_secure_volume(&self, id: &str) {
        let volume = units::secure_volume(id);
        let unit = format!("{volume}.volume");
        // Once its wadspace has gone (it's the volume's only user), deterministically.
        let users = [container_name(id)];
        let wait = Duration::from_secs(120);
        let r = crate::backend::drop_volume(
            &*self.backend,
            &volume,
            &unit,
            std::slice::from_ref(&unit),
            &users,
            self.settings.poll,
            wait,
        )
        .await;
        if let Err(e) = r {
            tracing::warn!("{id}: its secure stream's image volume is left: {e}");
        }
    }

    /// Whether `id` opens on the screen to move as it runs: a machine with a
    /// screen, and its image's desktop moves (what bring-up checks).
    async fn opened_moving(&self, id: &str) -> bool {
        let Some(ws) = self.docs.wadspace(id) else { return false };
        let Some(image) = self.builds.image_for(&ws) else { return false };
        self.settings.machine.screen_dir.is_some() && self.image_moves(&image).await
    }

    /// A wadspace that moved to its stream as it ran, found running after
    /// wadd restarted (streams.rs's adopt): on its stream, up, and it moves
    /// (its stream ending puts it back on the screen as it runs).
    pub fn taken_on_moved(&self, id: &str) {
        self.moving.lock().unwrap().insert(id.to_string());
        self.update(id, |s| {
            s.state.phase = Phase::Ready;
            s.state.message = None;
            s.display = Some(SessionDisplay::Stream);
        });
    }

    /// A secure stream's wadspace found running after wadd restarted
    /// (streams.rs's adopt): in its VM, on its stream, up.
    pub fn taken_on_secure(&self, id: &str) {
        self.secure.lock().unwrap().insert(id.to_string());
        self.update(id, |s| {
            s.state.phase = Phase::Ready;
            s.state.message = None;
            s.display = Some(SessionDisplay::Stream);
        });
    }

    /// Its extra folder as it is now; the wallpaper the unit should use
    /// instead of its look's, if its picture isn't here.
    fn write_extras(&self, ws: &Wadspace, mounts: &Mounts, streamed: bool) -> Res<Option<Wallpaper>> {
        let dir = PathBuf::from(extras::dir(&self.settings.state_dir.to_string_lossy(), &ws.id));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        let keys = self.docs.snapshot().keys;
        let mut files: Vec<(String, Option<String>)> =
            extras::files(ws, &keys, mounts).into_iter().map(|(n, t)| (n.to_string(), t)).collect();
        // Where its desktop draws (svc-de), and on how many screens (a stream is one).
        let (display, screens) = match streamed {
            true => ("stream", 1),
            false => ("screen", self.settings.machine.screens.max(1)),
        };
        files.push(("display".into(), Some(format!("{display}\n"))));
        files.push(("screens".into(), Some(format!("{screens}\n"))));
        for (name, text) in files {
            let path = dir.join(&name);
            match text {
                Some(t) => put(&path, t.as_bytes())?,
                None => remove(&path)?,
            }
        }
        // Its wallpaper's picture, copied in (none of another kind stays).
        let want = extras::wallpaper_file(&ws.look.wallpaper);
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("wallpaper.") && Some(&n) != want.as_ref() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        let (Some(file), Wallpaper::Image { value }) = (want, &ws.look.wallpaper) else { return Ok(None) };
        match self.wallpaper_source(value) {
            Some(src) => {
                let bytes = std::fs::read(&src).map_err(|e| format!("{}: {e}", src.display()))?;
                put(&dir.join(&file), &bytes)?;
                Ok(None)
            }
            None => {
                tracing::info!("{}: its wallpaper {value} isn't on this machine; it opens on the base's", ws.id);
                Ok(Some(Wallpaper::Empty))
            }
        }
    }

    /// A wallpaper's picture on this machine: one of WadSpaces' own
    /// (`/wallpapers/art/<name>.svg`, from its assets), a path, or one in your home.
    fn wallpaper_source(&self, value: &str) -> Option<PathBuf> {
        if value.split('/').any(|part| part == "..") {
            return None;
        }
        let p = if let Some(rest) = value.strip_prefix("/wallpapers/") {
            assets_dirs().into_iter().map(|d| d.join("wallpapers").join(rest)).find(|p| p.is_file())?
        } else {
            match value.strip_prefix("~/") {
                Some(rest) => self.settings.home.as_ref()?.join(rest),
                None => PathBuf::from(value),
            }
        };
        let m = std::fs::metadata(&p).ok()?;
        (p.is_absolute() && m.is_file() && m.len() <= MAX_WALLPAPER).then_some(p)
    }

    /// Its keys' folders (its unit mounts each): made empty if they're new.
    fn make_key_folders(&self, ws: &Wadspace) -> Res<()> {
        for s in extras::signins(ws) {
            if !crate::store::safe_id(&s.key.id) {
                return Err(format!("not a key's id: {:?}", s.key.id));
            }
            let dir = self.settings.signins_dir.join(&s.key.id);
            if dir.exists() {
                continue;
            }
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("{}: {e}", dir.display()))?;
            if let Some(uid) = self.settings.keys_uid {
                nix::unistd::chown(&dir, Some(uid.into()), Some(uid.into()))
                    .map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            tracing::info!("{}: {}'s folder made, empty until something signs in", ws.id, s.key.name);
        }
        Ok(())
    }

    // ------------------------------------------------------------ moving as it runs

    /// Its image's desktop can move as it runs.
    async fn image_moves(&self, image: &str) -> bool {
        let info = self.backend.image_info(image).await.ok().flatten();
        info.is_some_and(|i| i.labels.get(MOVES_LABEL).and_then(|v| v.as_str()) == Some("2"))
    }

    /// It's up on the screen with moving units: it can move to a stream as it runs.
    pub fn moves(&self, id: &str) -> bool {
        let st = self.state(id);
        st.state.phase == Phase::Ready && self.moving.lock().unwrap().contains(id)
    }

    /// A moving wadspace's sidecar, started beside it (it doesn't stop with
    /// the wadspace's desktop, nor the wadspace with it).
    pub async fn add_sidecar(&self, id: &str, stream: &StreamUnit) -> Result<(), ApiError> {
        let ws = self.docs.wadspace(id).ok_or_else(|| ApiError::not_found(format!("no wadspace {id:?}")))?;
        let mounts = Mounts::new();
        let run = Run { image: "", mounts: &mounts, wallpaper: None };
        let display = display_name(id);
        let files: Vec<(String, String)> =
            units::moving_units(&ws, &run, &self.settings.machine, Some(&stream.sidecar()))
                .into_iter()
                .filter(|(n, _)| n.starts_with(&format!("{display}.")))
                .collect();
        self.backend.write_units(&files, &[]).await.map_err(ApiError::internal)?;
        self.backend.start(&format!("{display}.service")).await.map_err(ApiError::internal)
    }

    /// Moves a running wadspace's desktop to `to` (its stream, or the
    /// screen, a window or the background) without restarting it: its
    /// display file says where, then `wadspaces-session switch` closes its
    /// apps as their × would and ends its labwc, which svc-de starts again
    /// there. Apps that didn't close in time: it stays where it was, they're
    /// named, and what did close opens again.
    pub async fn switch(&self, id: &str, to: SessionDisplay) -> Result<(), ApiError> {
        let ws = self.docs.wadspace(id).ok_or_else(|| ApiError::not_found(format!("no wadspace {id:?}")))?;
        let lock = self.lock_for(id);
        let _held = lock.lock().await;
        let was = self.state(id);
        let streamed = to == SessionDisplay::Stream;
        if was.state.phase != Phase::Ready || !self.moving.lock().unwrap().contains(id) {
            return Err(conflict(format!("{} can't move as it runs", ws.name)));
        }
        let say = |e: String| ApiError::internal(format!("moving {}: {e}", ws.name));
        self.write_display(id, streamed).map_err(say)?;
        let message = if streamed { "moving to its stream" } else { "moving to the screen" };
        // Back on the screen: served there first (its desktop waits for the socket).
        self.update(id, |s| {
            s.state.message = Some(message.into());
            if !streamed {
                s.display = Some(to);
            }
        });
        let name = container_name(id);
        let out = self.backend.exec(&name, "abc", &["wadspaces-session", "switch", "--timeout", "20"]).await;
        // What didn't close in time (then nothing moved).
        let still_open = |stdout: &[u8]| -> Vec<String> {
            let answer: serde_json::Value = serde_json::from_slice(stdout).unwrap_or_default();
            answer["still_open"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|w| {
                    let title = w["title"].as_str().filter(|t| !t.is_empty());
                    title.or(w["app_id"].as_str()).or(w.as_str()).map(String::from)
                })
                .collect()
        };
        let failed = match &out {
            Ok(o) if o.code == 0 && still_open(&o.stdout).is_empty() => None,
            Ok(o) if o.code == 0 || o.code == 3 => {
                let open = still_open(&o.stdout);
                let (them, it) = (open.join(", "), if open.len() == 1 { "it" } else { "them" });
                Some(ApiError::new(
                    ErrorCode::Conflict,
                    format!("{them} didn't close (unsaved work?): close {it} in {}, then try again", ws.name),
                ))
            }
            Ok(o) => Some(say(format!("exit {}: {}", o.code, o.stderr.trim()))),
            Err(e) => Some(say(e.clone())),
        };
        if let Some(e) = failed {
            // Called off: where it was, its apps open again.
            let _ = self.write_display(id, was.display == Some(SessionDisplay::Stream));
            self.update(id, |s| s.display = was.display);
            if let Err(err) = self.backend.exec(&name, "abc", &["wadspaces-session", "restore"]).await {
                tracing::warn!("{id}: opening its apps again: {err}");
            }
            return Err(e);
        }
        tracing::info!("{id}: moved to {}", if streamed { "its stream" } else { "the screen" });
        self.update(id, |s| {
            s.display = Some(to);
            s.state.phase = Phase::Waiting;
            s.state.message = Some("waiting for its desktop".into());
        });
        if let Err(e) = self.wait_desktop(&ws.name, &name).await {
            self.update(id, |s| {
                s.state.phase = Phase::Error;
                s.state.error = Some(e.clone());
            });
            return Err(ApiError::internal(e));
        }
        self.update(id, |s| s.state.phase = Phase::Ready);
        Ok(())
    }

    /// Its display file (and screens): where its desktop draws from its next start of labwc.
    fn write_display(&self, id: &str, streamed: bool) -> Res<()> {
        let dir = PathBuf::from(extras::dir(&self.settings.state_dir.to_string_lossy(), id));
        let (display, screens) = match streamed {
            true => ("stream", 1),
            false => ("screen", self.settings.machine.screens.max(1)),
        };
        put(&dir.join(units::DISPLAY_FILE), format!("{display}\n").as_bytes())?;
        put(&dir.join("screens"), format!("{screens}\n").as_bytes())
    }

    // ------------------------------------------------------------ stop

    pub async fn stop(&self, id: &str) -> Result<WadspaceRun, ApiError> {
        let known = self.docs.wadspace(id).is_some() || self.states.lock().unwrap().contains_key(id);
        if !known {
            return Err(ApiError::not_found(format!("no wadspace {id:?}")));
        }
        if let Some(t) = self.tasks.lock().unwrap().remove(id) {
            t.abort();
        }
        // Its open job, if it was opening.
        if let (Some(p), Some(job)) = (self.projects.get(), self.state(id).state.job) {
            let _ = p.jobs().cancel(&job);
        }
        let lock = self.lock_for(id);
        let _held = lock.lock().await;
        let running = self.backend.container_status(&container_name(id)).await.map_or(true, |s| s == RUNNING);
        if !running && matches!(self.state(id).state.phase, Phase::Idle | Phase::Error) {
            // Nothing to stop.
            self.stopped(id).await;
            self.screen.take(id);
            self.update(id, |s| {
                s.state.phase = Phase::Idle;
                s.display = None;
            });
            return Ok(self.state(id));
        }
        self.update(id, |s| {
            s.state.phase = Phase::Stopping;
            s.state.message = Some("stopping".into());
        });
        let mut stopped = self.backend.stop(&service(id)).await;
        // Its unit was never made (stopped before it started): nothing runs.
        if stopped.is_err() && self.backend.container_status(&container_name(id)).await.is_ok_and(|s| s != RUNNING) {
            stopped = Ok(());
        }
        self.screen.take(id);
        match stopped {
            Ok(()) => {
                self.stopped(id).await;
                tracing::info!("{id}: stopped");
                self.update(id, |s| {
                    s.state.phase = Phase::Idle;
                    s.display = None;
                });
                Ok(self.state(id))
            }
            Err(e) => {
                self.update(id, |s| {
                    s.state.phase = Phase::Error;
                    s.state.error = Some(e.clone());
                });
                Err(ApiError::internal(e))
            }
        }
    }

    /// It stopped: its Taildrive mounts go, and what it no longer has.
    async fn stopped(&self, id: &str) {
        self.secure.lock().unwrap().remove(id);
        let Some(p) = self.projects.get() else { return };
        match self.docs.wadspace(id) {
            Some(ws) => p.tidy(&ws).await,
            None => p.release(id).await,
        }
    }

    /// After an edit: a wadspace put in the Trash stops; one gone for good
    /// is stopped, and what wadd kept for it goes (its unit, extra folder,
    /// screen folder, state, what it had of its projects). One that isn't
    /// running loses what wadd made for projects it doesn't have any more.
    pub async fn edited(&self, changed: &[String]) {
        for id in changed {
            if let Some(ws) = self.docs.wadspace(id) {
                if ws.is_trashed() && self.state(id).state.phase != Phase::Idle {
                    tracing::info!("{id}: in the Trash, so it stops");
                    let _ = self.stop(id).await;
                } else if self.state(id).state.phase == Phase::Idle
                    && let Some(p) = self.projects.get()
                {
                    let lock = self.lock_for(id);
                    if let Ok(_held) = lock.try_lock() {
                        p.tidy(&ws).await;
                    }
                }
                continue;
            }
            if self.state(id).state.phase != Phase::Idle {
                let _ = self.stop(id).await;
            }
            let _ = self.backend.write_units(&[], &[format!("{}.container", container_name(id))]).await;
            if crate::store::safe_id(id) {
                let _ = std::fs::remove_dir_all(extras::dir(&self.settings.state_dir.to_string_lossy(), id));
            }
            if let Some(p) = self.projects.get() {
                p.forget(id).await;
            }
            self.screen.forget(id);
            self.states.lock().unwrap().remove(id);
            self.keep_shown(id, None);
            self.bus.forget(&format!("run:{id}"));
        }
    }

    // ------------------------------------------------------------ reconcile

    /// What podman says now, against what wadd thinks: containers that
    /// stopped by themselves, and ones running that wadd didn't start.
    pub async fn reconcile(&self) {
        if !self.backend.available().await {
            return;
        }
        let mut ids: Vec<String> = self.docs.live().into_iter().map(|w| w.id).collect();
        ids.extend(self.states.lock().unwrap().keys().filter(|k| !ids.contains(k)).cloned().collect::<Vec<_>>());
        for id in ids {
            let lock = self.lock_for(&id);
            let Ok(_held) = lock.try_lock() else { continue }; // busy: it knows
            if self.tasks.lock().unwrap().get(&id).is_some_and(|t| !t.is_finished()) {
                continue;
            }
            let name = container_name(&id);
            let Ok(status) = self.backend.container_status(&name).await else { continue };
            let phase = self.state(&id).state.phase;
            if status == RUNNING {
                if matches!(phase, Phase::Idle | Phase::Waiting) {
                    // Running, and wadd didn't start it (or it restarted): as it was shown.
                    // In a secure stream's VM: nothing to exec, and streams.rs knows it's up.
                    let secure = self.secure.lock().unwrap().contains(&id);
                    let up = secure || self.backend.desktop_up(&name).await.unwrap_or(false);
                    let shown = self.shown.lock().unwrap().get(&id).copied().unwrap_or(SessionDisplay::Screen);
                    if phase == Phase::Idle {
                        tracing::info!("{id}: running already; taken on");
                        // On the screen, it was opened to move as it runs if its image
                        // moves (as at open). On a stream, only streams.rs knows:
                        // `taken_on_moved`, when its stream had moved there.
                        if shown != SessionDisplay::Stream && self.opened_moving(&id).await {
                            self.moving.lock().unwrap().insert(id.clone());
                        }
                    }
                    self.update(&id, |s| {
                        s.display = s.display.or(Some(shown));
                        if up {
                            s.state.phase = Phase::Ready;
                        } else {
                            s.state.phase = Phase::Waiting;
                            s.state.message = Some("waiting for its desktop".into());
                        }
                    });
                }
            } else if matches!(phase, Phase::Ready | Phase::Waiting) {
                // It stopped by itself (its window closed: its desktop halts it).
                // Not one still Starting: its bring-up hasn't made its container
                // yet (open() marks it before the task takes its lock), and ends
                // it ready, in an error, or stopped.
                tracing::info!("{id}: its container stopped ({status})");
                self.update(&id, |s| {
                    s.state.phase = Phase::Idle;
                    s.display = None;
                });
                self.stopped(&id).await;
            }
        }
    }
}

fn ws_name(docs: &Docs, id: &str) -> String {
    docs.wadspace(id).map_or_else(|| id.to_string(), |w| w.name)
}

/// Writes a file whole, readable inside.
fn put(path: &Path, bytes: &[u8]) -> Res<()> {
    if std::fs::read(path).ok().as_deref() == Some(bytes) {
        return Ok(());
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, bytes)
        .and_then(|_| std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)))
        .and_then(|_| std::fs::rename(&tmp, path))
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn remove(path: &Path) -> Res<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Where WadSpaces' own pictures are: installed on the machine, or the repo's
/// on a laptop's build.
fn assets_dirs() -> Vec<PathBuf> {
    vec![PathBuf::from("/usr/share/wados/assets"), PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"))]
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;
    use std::path::Path;

    /// A render node on a PCI virtio GPU, as Linux 7.2's sysfs has it:
    /// renderD128/device is the PCI device (virtio-pci), virtio1 under it is virtio_gpu.
    fn virtio(root: &Path, features: &str) {
        let pci = root.join("devices/pci0000:00/0000:00:02.0");
        let virtio = pci.join("virtio1");
        std::fs::create_dir_all(&virtio).unwrap();
        std::fs::create_dir_all(root.join("drivers/virtio-pci")).unwrap();
        std::fs::create_dir_all(root.join("drivers/virtio_gpu")).unwrap();
        symlink(root.join("drivers/virtio-pci"), pci.join("driver")).unwrap();
        symlink(root.join("drivers/virtio_gpu"), virtio.join("driver")).unwrap();
        std::fs::write(virtio.join("features"), format!("{features}\n")).unwrap();
        let node = root.join("class/drm/renderD128");
        std::fs::create_dir_all(&node).unwrap();
        symlink(&pci, node.join("device")).unwrap();
    }

    #[test]
    fn a_virtio_gpu_without_virgl_draws_in_software() {
        let d = tempfile::tempdir().unwrap();
        virtio(d.path(), "0100000000000000000000000000110010000000100000000000000000000000");
        assert!(super::no_3d(&d.path().join("class/drm")));
    }

    #[test]
    fn a_virtio_gpu_with_virgl_or_another_gpu_draws_in_3d() {
        let d = tempfile::tempdir().unwrap();
        virtio(d.path(), "1100000000000000000000000000110010000000100000000000000000000000");
        assert!(!super::no_3d(&d.path().join("class/drm")));

        // An Intel GPU: its render node's device is the PCI device, driven by i915.
        let d = tempfile::tempdir().unwrap();
        let pci = d.path().join("devices/pci0000:00/0000:00:02.0");
        std::fs::create_dir_all(&pci).unwrap();
        std::fs::create_dir_all(d.path().join("drivers/i915")).unwrap();
        symlink(d.path().join("drivers/i915"), pci.join("driver")).unwrap();
        let node = d.path().join("class/drm/renderD128");
        std::fs::create_dir_all(&node).unwrap();
        symlink(&pci, node.join("device")).unwrap();
        assert!(!super::no_3d(&d.path().join("class/drm")));

        // No render node at all (a firmware framebuffer): wlroots chooses pixman itself.
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("class/drm/card0")).unwrap();
        assert!(!super::no_3d(&d.path().join("class/drm")));
        assert!(!super::no_3d(&d.path().join("missing")));
    }
}
