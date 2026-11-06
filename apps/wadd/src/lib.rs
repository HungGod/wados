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
//! Phase 6: projects as a wadspace opens (projects.rs: GitHub clones,
//! this machine's folders, copies, another machine's over Taildrive).
//!
//! Phase 7: streams (streams.rs, their routes in api_phase7.rs, sample
//! data in fake_phase7.rs; TURN in turn.rs).
//!
//! Phase 8: the system apps' routes (api_phase8.rs: the network,
//! Bluetooth, updates, storage; 501 until they're made, sample data in
//! fake_phase8.rs), and deleting a wadspace out of the Trash for good.
//! WadOS's own updates through bootc and the base images' refresh
//! (updates.rs), and the disk with its Clean Up (storage.rs).
//!
//! Later phases: Open-with files, runs of one time,
//! keys' sign-in flows, devices, the network, install, server mode and focus.

pub mod access;
pub mod agents;
pub mod api;
pub mod api_phase6;
pub mod api_phase7;
pub mod api_phase8;
pub mod api_phase9;
pub mod backend;
pub mod bluetooth;
pub mod builds;
pub mod clip;
pub mod console;
pub mod copies;
pub mod discs;
pub mod events;
pub mod fake;
pub mod fake_phase6;
pub mod fake_phase7;
pub mod fake_phase8;
pub mod focus;
pub mod github;
pub mod install;
pub mod jobs;
pub mod keys;
pub mod library;
pub mod logbuf;
pub mod mounts;
pub mod network;
pub mod packages;
pub mod peers;
pub mod projects;
pub mod proxy;
pub mod remote;
pub mod runs;
pub mod screen;
pub mod selinux;
pub mod server;
pub mod setup;
pub mod steam;
pub mod storage;
pub mod store;
pub mod streams;
pub mod systemd;
pub mod taildrive;
pub mod tailnet;
pub mod tar;
pub mod tunnel;
pub mod turn;
pub mod updates;

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

/// How long wadd waits, once told to stop, for requests under way (systemd's
/// TimeoutStopSec is 10 s): open event streams are closed after it.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

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
    /// The tailnet, peers, GitHub ([`fake_phase6`]), streams
    /// ([`fake_phase7`]) and the system apps' ([`fake_phase8`]) as sample data, each stage of its jobs taking this
    /// long; none: the machine's.
    pub sample: Option<Duration>,
}

impl Parts {
    pub fn real(config: &Config, profile: Profile) -> Self {
        Self {
            backend: Arc::new(backend::Real::new(config, profile)),
            hands: Arc::new(screen::HostHands),
            poll: Duration::from_secs(1),
            sample: None,
        }
    }
}

/// Phase 8's: the system apps' backends.
struct System {
    network: Arc<dyn api_phase8::Network>,
    bluetooth: Arc<dyn api_phase8::Bluetooth>,
    updates: Arc<dyn api_phase8::Updates>,
    storage: Arc<dyn api_phase8::Storage>,
}

impl System {
    fn not_yet() -> Self {
        let n = Arc::new(api_phase8::NotYet);
        Self { network: n.clone(), bluetooth: n.clone(), updates: n.clone(), storage: n }
    }
}

pub struct Server {
    listener: UnixListener,
    /// The path we bound, to clean up; None when systemd owns the socket.
    bound: Option<PathBuf>,
    reconcile_every: Duration,
    pub state: Arc<AppState>,
    /// The tailnet, peers and remote API (none with the sample): their loops start with `serve`.
    tailnet: Option<Arc<peers::TailnetPeers>>,
    /// The streaming proxy's public listener, its routes and where (none
    /// with the sample, or streaming off): it starts with `serve`.
    proxy: Option<(Arc<proxy::RouteTable>, Vec<String>)>,
    /// The machine's streams (none with the sample): adopted, then checked, from `serve`.
    streams: Option<Arc<streams::MachineStreams>>,
    /// The machine's updates and disk (none with the sample): their schedules start with `serve`.
    host: Option<(updates::HostUpdates, storage::MachineStorage)>,
    /// The machine's network and Bluetooth (none with the sample, or `[radios]` off): followed from `serve`.
    radios: Option<(Arc<network::MachineNetwork>, Arc<bluetooth::MachineBluetooth>)>,
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
        // A machine the installer made is offered WadOS by Set up; a stick has no wadspaces.
        let installer_made = setup::read(&d.state_dir) != wad_proto::SetupStep::Done;
        let stick = d.state_dir.join("installer/installer.toml").exists();
        let starter = !installer_made && !stick;
        let docs = Arc::new(Docs::open_with(Store::new(&d.state_dir)?, bus.clone(), &machine.id, starter)?);
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
            Listen::Path(p) => (bind(&p, profile, &d.allow_groups)?, Some(p)),
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
        // The tailnet, my other machines and the remote API (the sample's instead, with it).
        let tailnet = parts.sample.is_none().then(|| {
            let person = person_of(screen.clone(), profile, d.projects_uid);
            peers::TailnetPeers::new(&config, &machine, bus.clone(), jobs.clone(), docs.clone(), builds.clone(), person)
        });
        let (net, accounts): (Arc<dyn api_phase6::Net>, Arc<dyn api_phase6::Accounts>) = match parts.sample {
            Some(step) => {
                let hostname = machine.name.to_lowercase();
                let s =
                    fake_phase6::Sample::new(&hostname, jobs.clone(), bus.clone(), docs.clone(), builds.clone(), step);
                (Arc::new(s.clone()), Arc::new(s))
            }
            None => (
                tailnet.clone().expect("made without the sample"),
                Arc::new(github::GithubService::new(
                    parts.backend.clone(),
                    jobs.clone(),
                    bus.clone(),
                    github::Settings::new(&config),
                )),
            ),
        };
        let keys = keys::MachineKeys::new(&config);
        let projects = projects::Projects::new(
            projects::Settings::new(&config, profile),
            parts.backend.clone(),
            jobs.clone(),
            net.clone(),
        );
        let runs = runs::Runs::new(
            parts.backend.clone(),
            docs.clone(),
            bus.clone(),
            screen.clone(),
            builds.clone(),
            run_settings,
        );
        runs.attach(Arc::new(projects));
        // The streaming proxy's routes, which the streams fill (proxy.rs).
        let routes = proxy::RouteTable::new();
        let public_proxy = (config.streaming.enabled && parts.sample.is_none())
            .then(|| (routes.clone(), config.streaming.listen.clone()));
        let mut machine_streams = None;
        let streams: Arc<dyn api_phase7::Streams> = match parts.sample {
            Some(step) => {
                let hostname = machine.name.to_lowercase();
                Arc::new(fake_phase7::Sample::new(
                    &hostname,
                    jobs.clone(),
                    bus.clone(),
                    docs.clone(),
                    runs.clone(),
                    step,
                ))
            }
            None => {
                let t = tailnet.clone().expect("made without the sample");
                let s = streams::MachineStreams::new(
                    streams::Settings { poll: parts.poll, ..streams::Settings::new(&config) },
                    streams::Parts {
                        backend: parts.backend.clone(),
                        docs: docs.clone(),
                        bus: bus.clone(),
                        jobs: jobs.clone(),
                        runs: runs.clone(),
                        builds: builds.clone(),
                        routes: Arc::new(routes.clone()),
                        private: Arc::new(streams::TailscaleServe::of(&config, t.tailnet.clone())),
                        tunnel: Arc::new(tunnel::Tunnel::new(&config.streaming)),
                        peers: Some(t.peers.clone()),
                    },
                );
                let ends: std::sync::Weak<dyn runs::StreamEnds> = Arc::downgrade(&s) as _;
                runs.attach_streams(ends);
                machine_streams = Some(s.clone());
                s
            }
        };
        let mut host = None;
        let mut radios_on = None;
        let system = match parts.sample {
            Some(step) => {
                let s = fake_phase8::Sample::new(
                    &machine.name.to_lowercase(),
                    jobs.clone(),
                    bus.clone(),
                    docs.clone(),
                    builds.clone(),
                    net.clone(),
                    step,
                );
                System {
                    network: Arc::new(s.clone()),
                    bluetooth: Arc::new(s.clone()),
                    updates: Arc::new(s.clone()),
                    storage: Arc::new(s),
                }
            }
            None => {
                let mut s = System::not_yet();
                let (u, st) = host_system(&config, profile, &jobs, &bus, &docs, &builds);
                (s.updates, s.storage) = (Arc::new(u.clone()), Arc::new(st.clone()));
                host = Some((u, st));
                if config.radios.enabled {
                    let (n, b) = radios(&config, &machine, &bus, tailnet.as_ref().map(|t| t.tailnet.clone()), &net);
                    (s.network, s.bluetooth) = (Arc::new(n.clone()), Arc::new(b.clone()));
                    radios_on = Some((n, b));
                }
                s
            }
        };
        let install: Arc<dyn api_phase9::Installer> = match parts.sample {
            Some(step) => Arc::new(install::MachineInstall::sample(&d.state_dir.join("install"), step, jobs.clone())),
            None => {
                let hands = Arc::new(install::SystemHands { script: install::SCRIPT.into() });
                match install::MachineInstall::open(&d.state_dir, install::JOBS_DIR.as_ref(), hands, jobs.clone()) {
                    Some(i) => Arc::new(i),
                    None => Arc::new(api_phase9::NotAStick),
                }
            }
        };
        let setup = setup::SetupState::new(&d.state_dir, docs.clone(), system.updates.clone());
        let focus_dir = match profile {
            Profile::System => PathBuf::from(focus::RUN_DIR),
            Profile::User => d.state_dir.clone(),
        };
        let focus = focus::MachineFocus::new(&focus_dir, docs.clone(), runs.clone(), bus.clone());
        let (server_dir, seat): (PathBuf, Arc<dyn server::Hands>) = match profile {
            Profile::System => (server::RUN_DIR.into(), Arc::new(server::SystemHands { uid: d.projects_uid })),
            Profile::User => (d.state_dir.clone(), Arc::new(server::NoSeat)),
        };
        let registry: Arc<dyn discs::Registry> = Arc::new(discs::Ghcr {
            accounts: accounts.clone(),
            backend: parts.backend.clone(),
            state_dir: d.state_dir.clone(),
        });
        let (who, uid) = (screen.clone(), d.projects_uid);
        let home = person_of(who, profile, uid);
        let steam = steam::MachineSteam::new(
            &d.signins_dir,
            Arc::new(steam::RealSpaces { docs: docs.clone(), builds: builds.clone(), runs: runs.clone() }),
            registry.clone(),
            docs.clone(),
            jobs.clone(),
            steam::SIGN_IN_FOR,
        );
        let discs = discs::MachineDiscs::new(
            &d.state_dir,
            Box::new(move || home().map(|p| p.home)),
            registry,
            docs.clone(),
            jobs.clone(),
            bus.clone(),
        );
        let agents_dir = agents::sockets_dir(&d.state_dir, profile == Profile::System);
        let agents = agents::MachineAgents::new(
            &d.state_dir,
            &agents_dir,
            Arc::new(agents::Exec),
            Arc::new(agents::RealHands { docs: docs.clone(), builds: builds.clone(), runs: runs.clone() }),
            docs.clone(),
            bus.clone(),
        );
        let server = server::ServerMode::new(&server_dir, runs.clone(), focus.clone(), seat, bus.clone());
        let state = Arc::new(AppState {
            machine,
            profile,
            policy,
            bus,
            docs,
            logs,
            jobs,
            builds,
            runs,
            screen,
            net,
            accounts,
            keys,
            streams,
            network: system.network,
            bluetooth: system.bluetooth,
            updates: system.updates,
            storage: system.storage,
            install,
            setup,
            focus,
            server,
            discs,
            steam,
            agents,
        });
        Ok(Self {
            listener,
            bound,
            reconcile_every: Duration::from_secs(d.reconcile_s.max(1)),
            state,
            tailnet,
            proxy: public_proxy,
            streams: machine_streams,
            host,
            radios: radios_on,
        })
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
        let mut loops = vec![
            tokio::spawn(app.jobs.clone().run()),
            tokio::spawn(watch_machine(app.clone(), self.reconcile_every)),
            tokio::spawn(watch_screen(app.clone())),
            tokio::spawn(app.focus.clone().run()),
            tokio::spawn(app.agents.clone().run()),
        ];
        if let Some(t) = &self.tailnet {
            loops.extend(t.start(app.clone()));
        }
        if let Some((routes, listen)) = &self.proxy {
            loops.push(tokio::spawn(proxy::serve(routes.clone(), listen.clone())));
        }
        if let Some((n, b)) = &self.radios {
            loops.push(tokio::spawn(n.clone().run()));
            loops.push(tokio::spawn(b.clone().run()));
        }
        if let Some((u, st)) = &self.host {
            loops.extend(u.start());
            loops.extend(st.start());
        }
        if let Some(s) = &self.streams {
            loops.push(tokio::spawn(s.clone().run()));
        }
        systemd::notify("READY=1");
        tracing::info!(machine = %self.state.machine.id, "serving /v1");
        let router = api::router(self.state.clone()).into_make_service_with_connect_info::<Peer>();
        // Graceful, but not forever: an event stream (the shell's, Settings',
        // a peer's) never ends by itself, and axum waits for every open
        // connection. A few seconds for requests under way, then they close.
        let (stopping, mut stopped) = tokio::sync::watch::channel(false);
        let signal = async move {
            shutdown.await;
            let _ = stopping.send(true);
        };
        let serve =
            std::future::IntoFuture::into_future(axum::serve(self.listener, router).with_graceful_shutdown(signal));
        let res = tokio::select! {
            r = serve => r,
            _ = async {
                let _ = stopped.wait_for(|s| *s).await;
                tokio::time::sleep(SHUTDOWN_GRACE).await;
            } => {
                tracing::info!("stopping: closing the connections still open after {SHUTDOWN_GRACE:?}");
                Ok(())
            }
        };
        for l in loops {
            l.abort();
        }
        if let Some(p) = &self.bound {
            let _ = std::fs::remove_file(p);
        }
        res
    }
}

/// The machine's network (NetworkManager) and Bluetooth (BlueZ), on
/// `[radios] bus` (the system's by default), and airplane mode over both.
fn radios(
    config: &Config,
    machine: &MachineInfo,
    bus: &Bus,
    tailnet: Option<Arc<tailnet::Tailnet>>,
    net: &Arc<dyn api_phase6::Net>,
) -> (Arc<network::MachineNetwork>, Arc<bluetooth::MachineBluetooth>) {
    let r = &config.radios;
    let every = Duration::from_secs(r.poll_s.max(1));
    let airplane = network::Airplane::new(&config.daemon.state_dir);
    let hostname = machine.name.to_lowercase();
    let b = bluetooth::MachineBluetooth::new(&r.bus, &hostname, bus.clone(), every, airplane.clone());
    let n = network::MachineNetwork::new(&r.bus, airplane, b.clone(), tailnet, net.clone(), bus.clone(), every);
    (n, b)
}

/// The machine's updates (bootc, the base images) and its disk: podman's
/// at `podman_socket`. A refresh of the bases makes wadspaces built on the
/// old base show as updates, and the disk is measured again.
fn host_system(
    config: &Config,
    profile: Profile,
    jobs: &Arc<jobs::Jobs>,
    bus: &Bus,
    docs: &Arc<Docs>,
    builds: &Arc<builds::Builds>,
) -> (updates::HostUpdates, storage::MachineStorage) {
    let podman = || wad_podman::Podman::new(&config.daemon.podman_socket);
    let st = storage::MachineStorage::new(
        storage::Settings::new(config, profile),
        Arc::new(storage::PodmanDisk(podman())),
        docs.clone(),
        builds.clone(),
        jobs.clone(),
        bus.clone(),
    );
    let (b, s) = (builds.clone(), st.clone());
    let hooks = updates::Hooks {
        bases_changed: Some(Box::new(move || {
            let (b, s) = (b.clone(), s.clone());
            tokio::spawn(async move {
                b.refresh_base().await;
                b.publish_all();
                s.poke();
            });
        })),
    };
    let u = updates::HostUpdates::new(
        updates::Settings::new(config, profile),
        Arc::new(updates::SystemBootc::new()),
        Arc::new(updates::PodmanImages(podman())),
        jobs.clone(),
        bus.clone(),
        hooks,
    );
    (u, st)
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

/// The machine's person (whose home Taildrive shares, as them): the shell's
/// session's user, else `projects_uid`; on a laptop, you.
fn person_of(screen: Arc<screen::Screen>, profile: Profile, projects_uid: u32) -> tailnet::PersonFn {
    let fallback = match profile {
        Profile::System => projects_uid,
        Profile::User => nix::unistd::getuid().as_raw(),
    };
    Arc::new(move || {
        let uid = screen.info().uid.filter(|u| *u != 0).unwrap_or(fallback);
        let u = nix::unistd::User::from_uid(uid.into()).ok().flatten()?;
        Some(tailnet::Person { uid, name: u.name, home: u.dir })
    })
}

/// The screen: sockets served to what's on it, every few seconds or when
/// something changes (a wadspace starting or stopping, the shell's session).
async fn watch_screen(app: Arc<AppState>) {
    loop {
        let lib = app.docs.snapshot();
        app.screen.seal(lib.wadspaces.iter().filter(|w| w.sealed).map(|w| w.id.clone()).collect());
        app.screen.tick(app.runs.on_screen()).await;
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(3)) => {}
            _ = app.screen.poked() => {}
        }
    }
}

/// Binds the API's socket: the machine's people may use a machine's (0660,
/// its group the first of `groups` the machine has, as wadd.socket makes
/// it), only you a laptop's (0600). Its folder must let them reach it.
fn bind(path: &std::path::Path, profile: Profile, groups: &[String]) -> std::io::Result<UnixListener> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let l = UnixListener::bind(path)?;
    let system = profile == Profile::System;
    if system && nix::unistd::geteuid().is_root() {
        let lookup = |name: &str| nix::unistd::Group::from_name(name).ok().flatten().map(|g| g.gid.as_raw());
        match socket_group(groups, lookup) {
            Some(gid) => std::os::unix::fs::chown(path, None, Some(gid))?,
            None => {
                tracing::warn!("none of the groups {groups:?} is on this machine: only root may use {}", path.display())
            }
        }
    }
    let mode = if system { 0o660 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(l)
}

/// The API socket's group: the first of `groups` (wadd's allowed ones) that `lookup` finds.
fn socket_group(groups: &[String], lookup: impl Fn(&str) -> Option<u32>) -> Option<u32> {
    groups.iter().find_map(|g| lookup(g))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_socket_is_the_first_allowed_group_the_machine_has() {
        let groups = vec!["wados".to_string(), "wheel".to_string()];
        let both = |g: &str| match g {
            "wados" => Some(990),
            "wheel" => Some(10),
            _ => None,
        };
        assert_eq!(super::socket_group(&groups, both), Some(990));
        let wheel_only = |g: &str| (g == "wheel").then_some(10);
        assert_eq!(super::socket_group(&groups, wheel_only), Some(10));
        assert_eq!(super::socket_group(&groups, |_| None), None);
        assert_eq!(super::socket_group(&[], both), None);
    }
}
