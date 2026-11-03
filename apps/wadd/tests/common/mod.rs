//! A machine for wadd's tests: the fake podman and systemd, fake screen
//! hands, a compositor's socket (a real one, in a temporary folder), and
//! wadd's parts on them, as `Server` puts them together.
#![allow(dead_code)]

use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use wad_gen::{BASE_IMAGE, labels};
use wad_model::{AppId, Op, Section};
use wad_proto::{Event, MachineInfo, Profile};
use wadd::builds::{Builds, Home};
use wadd::events::Bus;
use wadd::fake::{Fake, FakeHands};
use wadd::jobs::Jobs;
use wadd::library::Docs;
use wadd::packages::Packages;
use wadd::runs::Runs;
use wadd::screen::Screen;
use wadd::store::Store;

pub const ABI: &str = "fedora44.1";
pub const INDEX: &str = "file:///index.json";

pub struct Rig {
    pub dir: tempfile::TempDir,
    pub fake: Arc<Fake>,
    pub hands: Arc<FakeHands>,
    pub bus: Bus,
    pub docs: Arc<Docs>,
    pub jobs: Arc<Jobs>,
    pub builds: Arc<Builds>,
    pub screen: Arc<Screen>,
    pub runs: Arc<Runs>,
    pub home: PathBuf,
    /// The compositor's socket, while it's held.
    pub compositor: PathBuf,
    pub listener: Option<UnixListener>,
}

pub struct Options {
    pub pull: bool,
    pub base: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { pull: false, base: true }
    }
}

pub fn rig() -> Rig {
    rig_with(Options::default())
}

pub fn rig_with(o: Options) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let home = dir.path().join("home");
    let runtime = dir.path().join("rt");
    for d in [&home, &runtime] {
        std::fs::create_dir_all(d).unwrap();
    }
    let fake = Arc::new(Fake::new());
    if o.base {
        fake.add_image(BASE_IMAGE, &[(labels::ABI, ABI), (labels::EXTRAS, "1")]);
    }
    let hands = Arc::new(FakeHands::default());
    let bus =
        Bus::new(MachineInfo { id: "m-test".into(), name: "Test".into(), version: "0".into(), profile: Profile::User });
    let docs = Arc::new(Docs::open(Store::new(&state).unwrap(), bus.clone(), "m-test").unwrap());
    let jobs = Jobs::new(bus.clone(), &state);
    let packages = Packages::new(
        fake.clone(),
        wadd::packages::Settings {
            pull: o.pull,
            index_url: INDEX.into(),
            index_max_age: Duration::from_secs(3600),
            state_dir: state.clone(),
            min_free_gb: 1,
        },
    );
    let builds = Builds::new(
        fake.clone(),
        packages,
        jobs.clone(),
        docs.clone(),
        bus.clone(),
        &state,
        wadd::builds::Settings { min_free_gb: 1, copy_max_bytes: 1 << 20 },
        Some(Home { dir: home.clone(), owner: None }),
    );
    let screen = Screen::new(
        hands.clone(),
        fake.clone(),
        bus.clone(),
        wadd::screen::Settings {
            enabled: true,
            root: dir.path().join("screen"),
            runtime_dir: runtime.clone(),
            socket: None,
            bind_pulse: false,
        },
    );
    let mut config = wad_config::Config::defaults(wad_config::Profile::User);
    config.daemon.state_dir = state.clone();
    config.daemon.signins_dir = state.join("keys");
    config.daemon.screen_dir = dir.path().join("screen");
    config.daemon.ready_timeout_s = 5;
    let settings = wadd::runs::Settings {
        poll: Duration::from_millis(10),
        home: Some(home.clone()),
        ..wadd::runs::Settings::new(&config, wad_config::Profile::User)
    };
    let runs = Runs::new(fake.clone(), docs.clone(), bus.clone(), screen.clone(), builds.clone(), settings);
    let compositor = runtime.join("wayland-0");
    let listener = Some(UnixListener::bind(&compositor).unwrap());
    Rig { dir, fake, hands, bus, docs, jobs, builds, screen, runs, home, compositor, listener }
}

impl Rig {
    /// A new wadspace with these apps (and add-ons, `app:tool`): its id.
    pub fn wadspace(&self, name: &str, apps: &[AppId], addons: &[&str]) -> String {
        let id = self
            .docs
            .apply(Op::Create { name: Some(name.into()), category: None, machine: None })
            .unwrap()
            .created
            .unwrap();
        for a in apps {
            self.docs.apply(Op::AddApp { ws: id.clone(), app: *a }).unwrap();
        }
        for item in addons {
            self.edit(Op::Add { ws: id.clone(), section: Section::Addons, item: item.to_string(), from: None });
        }
        self.builds.publish(&id);
        id
    }

    /// An edit, as the API makes it.
    pub fn edit(&self, op: Op) -> wad_proto::OpResult {
        let out = self.docs.apply(op).unwrap();
        self.builds.edited(&out.changed);
        out
    }

    /// A package here, as images/build.sh or an earlier build left it: its image id.
    pub fn package_here(&self, id: &str) -> String {
        let image = self.fake.add_image(
            &format!("localhost/wados-pkg-{id}:latest"),
            &[(labels::PKG, id), (labels::PKG_ABI, ABI), (labels::PKG_BUILT_HERE, "true")],
        );
        self.fake.add_tag(&image, &wad_gen::packages::local_ref(id, &image));
        image
    }

    /// Waits for a job to end; its status.
    pub async fn finished(&self, job: &str) -> wad_model::JobStatus {
        for _ in 0..500 {
            let j = self.jobs.info(job).expect("the job");
            if wadd::jobs::finished(j.job.status) {
                return j.job.status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("{job} didn't end: {:?}", self.jobs.info(job));
    }

    /// The compositor restarts: a new socket at the same path.
    pub fn restart_compositor(&mut self) {
        self.listener = None;
        std::fs::remove_file(&self.compositor).unwrap();
        self.listener = Some(UnixListener::bind(&self.compositor).unwrap());
    }

    pub fn screen_dir(&self, id: &str) -> PathBuf {
        self.dir.path().join("screen").join(id)
    }

    pub fn state_dir(&self) -> PathBuf {
        self.dir.path().join("state")
    }
}

/// Every event heard so far.
pub fn drain(rx: &mut tokio::sync::broadcast::Receiver<Event>) -> Vec<Event> {
    let mut out = vec![];
    while let Ok(e) = rx.try_recv() {
        out.push(e);
    }
    out
}

pub fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}
