//! wadd, the WadSpaces machine daemon: it keeps the machine's wadspaces
//! (wad_model's library, edited only through it), builds their images, runs
//! them, and serves `/v1` on a Unix socket to the callers its policy allows.
//!
//! Phase 1: the library and its edits, events, logs; building a wadspace's
//! image (builds.rs: its set, from packages here, CI's or built here) as a
//! job (jobs.rs); and running it on the screen (runs.rs: wad_gen's quadlets,
//! a socket of its own on the compositor, screen.rs), through podman and
//! systemd behind a trait (backend.rs; fake.rs for tests).
//!
//! Later phases: streams and the proxy, the tailnet and remote API,
//! projects (clones, mounts, copies), Open-with files, runs of one time,
//! keys' sign-in flows, devices, the network, updates, install, server mode,
//! focus, and the tidy-up of images nothing needs (v1's tidy.rs).

pub mod access;
pub mod api;
pub mod backend;
pub mod builds;
pub mod clip;
pub mod events;
pub mod fake;
pub mod jobs;
pub mod library;
pub mod logbuf;
pub mod mounts;
pub mod packages;
pub mod runs;
pub mod screen;
pub mod selinux;
pub mod store;
pub mod systemd;
pub mod tar;

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UnixListener;
use wad_config::{Config, Profile};
use wad_proto::MachineInfo;

use crate::access::{Peer, Policy};
use crate::api::AppState;
use crate::backend::Backend;
use crate::events::Bus;
use crate::library::Docs;
use crate::logbuf::LogBuffer;
use crate::store::Store;

/// Where the API listens.
pub enum Listen {
    /// The socket systemd handed over (wadd.socket).
    Activated(std::os::unix::net::UnixListener),
    /// A path to bind (and remove on exit).
    Path(PathBuf),
}

/// What wadd works the machine with: podman and systemd, and the screen's
/// hands. The machine's own, or fakes (tests).
pub struct Parts {
    pub backend: Arc<dyn Backend>,
    pub hands: Arc<dyn screen::Hands>,
    /// Between readiness checks.
    pub poll: Duration,
}

impl Parts {
    pub fn real(config: &Config, profile: Profile) -> Self {
        Self {
            backend: Arc::new(backend::Real::new(config, profile)),
            hands: Arc::new(screen::HostHands),
            poll: Duration::from_secs(1),
        }
    }
}

pub struct Server {
    listener: UnixListener,
    /// The path we bound, to clean up; None when systemd owns the socket.
    bound: Option<PathBuf>,
    reconcile_every: Duration,
    pub state: Arc<AppState>,
}

impl Server {
    pub fn new(profile: Profile, config: Config, listen: Listen, logs: LogBuffer) -> std::io::Result<Self> {
        let parts = Parts::real(&config, profile);
        Self::with_parts(profile, config, listen, logs, parts)
    }

    pub fn with_parts(
        profile: Profile,
        config: Config,
        listen: Listen,
        logs: LogBuffer,
        parts: Parts,
    ) -> std::io::Result<Self> {
        let machine = MachineInfo {
            id: config.machine_id(),
            name: config.machine.name.clone(),
            version: env!("CARGO_PKG_VERSION").into(),
            profile: match profile {
                Profile::System => wad_proto::Profile::System,
                Profile::User => wad_proto::Profile::User,
            },
        };
        let d = &config.daemon;
        let bus = Bus::new(machine.clone());
        let docs = Arc::new(Docs::open(Store::new(&d.state_dir)?, bus.clone(), &machine.id)?);
        let policy = Policy {
            own_uid: nix::unistd::getuid().as_raw(),
            users: d.allow_users.clone(),
            groups: d.allow_groups.clone(),
            directory: Box::new(access::System),
        };
        let (listener, bound) = match listen {
            Listen::Activated(l) => {
                l.set_nonblocking(true)?;
                (UnixListener::from_std(l)?, None)
            }
            Listen::Path(p) => (bind(&p, profile)?, Some(p)),
        };
        let root = nix::unistd::geteuid().is_root();
        let jobs = jobs::Jobs::new(bus.clone(), &d.state_dir);
        let packages = packages::Packages::new(parts.backend.clone(), packages::Settings::new(&config));
        let run_settings = runs::Settings { poll: parts.poll, ..runs::Settings::new(&config, profile) };
        let home = run_settings.home.clone().map(|dir| builds::Home {
            dir,
            // As root, a COPY takes only what's the user's.
            owner: root.then_some(d.projects_uid),
        });
        let builds = builds::Builds::new(
            parts.backend.clone(),
            packages,
            jobs.clone(),
            docs.clone(),
            bus.clone(),
            &d.state_dir,
            builds::Settings { min_free_gb: d.build_min_free_gb, copy_max_bytes: d.copy_max_mb << 20 },
            home,
        );
        let screen = screen::Screen::new(
            parts.hands,
            parts.backend.clone(),
            bus.clone(),
            screen::Settings {
                enabled: config.display.enabled,
                root: d.screen_dir.clone(),
                runtime_dir: config.display.runtime_dir.clone(),
                socket: config.display.socket.clone(),
                // Only a machine's wadd (root) can bind into PID 1's mounts.
                bind_pulse: root && profile == Profile::System,
            },
        );
        if config.display.enabled {
            std::fs::create_dir_all(&d.screen_dir)?;
        }
        let runs =
            runs::Runs::new(parts.backend, docs.clone(), bus.clone(), screen.clone(), builds.clone(), run_settings);
        let state = Arc::new(AppState { machine, profile, policy, bus, docs, logs, jobs, builds, runs, screen });
        Ok(Self { listener, bound, reconcile_every: Duration::from_secs(d.reconcile_s.max(1)), state })
    }

    /// Serves until SIGTERM or Ctrl+C.
    pub async fn serve(self) -> std::io::Result<()> {
        let shutdown = async {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM");
            tokio::select! {
                _ = term.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            tracing::info!("stopping");
        };
        self.serve_until(shutdown).await
    }

    /// Serves until `shutdown` resolves.
    pub async fn serve_until(self, shutdown: impl Future<Output = ()> + Send + 'static) -> std::io::Result<()> {
        let app = self.state.clone();
        let loops = [
            tokio::spawn(app.jobs.clone().run()),
            tokio::spawn(watch_machine(app.clone(), self.reconcile_every)),
            tokio::spawn(watch_screen(app.clone())),
        ];
        systemd::notify("READY=1");
        tracing::info!(machine = %self.state.machine.id, "serving /v1");
        let router = api::router(self.state.clone()).into_make_service_with_connect_info::<Peer>();
        let res = axum::serve(self.listener, router).with_graceful_shutdown(shutdown).await;
        for l in loops {
            l.abort();
        }
        if let Some(p) = &self.bound {
            let _ = std::fs::remove_file(p);
        }
        res
    }
}

/// The base, the images built, and the containers, against what wadd
/// thinks, every `every`: the present first (each wadspace's image and run).
async fn watch_machine(app: Arc<AppState>, every: Duration) {
    app.builds.refresh_base().await;
    app.builds.publish_all();
    app.runs.publish_all();
    loop {
        app.runs.reconcile().await;
        if app.builds.refresh_base().await {
            tracing::info!("the base changed: {:?}", app.builds.base().map(|b| b.id));
        }
        app.builds.check_records().await;
        app.builds.publish_all();
        tokio::time::sleep(every).await;
    }
}

/// The screen: sockets served to what's on it, every few seconds or when
/// something changes (a wadspace starting or stopping, the shell's session).
async fn watch_screen(app: Arc<AppState>) {
    loop {
        app.screen.tick(app.runs.on_screen()).await;
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(3)) => {}
            _ = app.screen.poked() => {}
        }
    }
}

/// Binds the API's socket: the machine's people may use a machine's (its
/// group, 0660, as wadd.socket makes it), only you a laptop's (0600).
fn bind(path: &std::path::Path, profile: Profile) -> std::io::Result<UnixListener> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let l = UnixListener::bind(path)?;
    let mode = if profile == Profile::System { 0o660 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(l)
}
