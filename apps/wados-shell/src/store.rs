//! What the shell knows of the machine: what wadd says, kept current from
//! its event stream (wadd.rs). The shell's surfaces read it and watch it;
//! they never change it themselves (every change goes to wadd, which tells
//! everyone, this store included).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use indexmap::IndexMap;
use wad_model::edit::Library;
use wad_model::{Phase, SessionDisplay, Wadspace};
use wad_proto::{Event, JobInfo, Level, MachineInfo, ScreenInfo, WadspaceBuild, WadspaceRun};

/// What changed, for watchers.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// Connected to wadd, or lost it (it's followed again by itself).
    Connected(bool),
    Machine,
    /// The whole library (tags, keys, or everything at once).
    Library,
    /// One wadspace's document (made, changed, or gone).
    Wadspace(String),
    Run(String),
    Build(String),
    Job(String),
    Screen,
    /// Something to tell the person.
    Notice {
        level: Level,
        title: String,
        undo: Option<String>,
    },
}

type Watcher = Box<dyn Fn(&Change)>;

#[derive(Default)]
pub struct Store {
    connected: Cell<bool>,
    machine: RefCell<Option<MachineInfo>>,
    library: RefCell<Library>,
    runs: RefCell<IndexMap<String, WadspaceRun>>,
    builds: RefCell<IndexMap<String, WadspaceBuild>>,
    jobs: RefCell<IndexMap<String, JobInfo>>,
    screen: RefCell<Option<ScreenInfo>>,
    watchers: RefCell<Vec<Watcher>>,
}

impl Store {
    pub fn new() -> Rc<Self> {
        Rc::default()
    }

    /// Calls `f` on every change from now on.
    pub fn watch(&self, f: impl Fn(&Change) + 'static) {
        self.watchers.borrow_mut().push(Box::new(f));
    }

    fn tell(&self, c: Change) {
        // Watchers may read the store (never borrow_mut it), so it's free here.
        for w in self.watchers.borrow().iter() {
            w(&c);
        }
    }

    pub fn set_connected(&self, on: bool) {
        if self.connected.replace(on) != on {
            self.tell(Change::Connected(on));
        }
    }

    /// One of wadd's events.
    pub fn apply(&self, e: Event) {
        let change = match e {
            Event::Machine(m) => {
                *self.machine.borrow_mut() = Some(m);
                Change::Machine
            }
            Event::Library(l) => {
                *self.library.borrow_mut() = *l;
                Change::Library
            }
            Event::Wadspace(w) => {
                let id = w.id.clone();
                let mut lib = self.library.borrow_mut();
                match lib.wadspaces.iter_mut().find(|x| x.id == id) {
                    Some(x) => *x = *w,
                    None => lib.wadspaces.push(*w),
                }
                Change::Wadspace(id)
            }
            Event::WadspaceGone { id } => {
                self.library.borrow_mut().wadspaces.retain(|x| x.id != id);
                self.runs.borrow_mut().shift_remove(&id);
                self.builds.borrow_mut().shift_remove(&id);
                Change::Wadspace(id)
            }
            Event::Run(r) => {
                let id = r.id.clone();
                self.runs.borrow_mut().insert(id.clone(), *r);
                Change::Run(id)
            }
            Event::Build(b) => {
                let id = b.id.clone();
                self.builds.borrow_mut().insert(id.clone(), *b);
                Change::Build(id)
            }
            Event::Job { job } => {
                let id = job.job.id.clone();
                self.jobs.borrow_mut().insert(id.clone(), *job);
                Change::Job(id)
            }
            Event::JobLines { .. } => return,
            Event::JobGone { id } => {
                self.jobs.borrow_mut().shift_remove(&id);
                Change::Job(id)
            }
            Event::Screen(s) => {
                *self.screen.borrow_mut() = Some(s);
                Change::Screen
            }
            Event::Notice { level, title, undo } => Change::Notice { level, title, undo },
            // Phase 6's: the tailnet, peers, GitHub and the sign-in driver (the UI's, still to come).
            Event::Tailnet(_)
            | Event::Peers { .. }
            | Event::Github { .. }
            | Event::SignIn { .. }
            | Event::SignInDone { .. } => return,
        };
        self.tell(change);
    }

    pub fn connected(&self) -> bool {
        self.connected.get()
    }

    pub fn machine(&self) -> Option<MachineInfo> {
        self.machine.borrow().clone()
    }

    pub fn machine_id(&self) -> String {
        self.machine.borrow().as_ref().map(|m| m.id.clone()).unwrap_or_default()
    }

    pub fn library(&self) -> std::cell::Ref<'_, Library> {
        self.library.borrow()
    }

    pub fn wadspace(&self, id: &str) -> Option<Wadspace> {
        self.library.borrow().wadspace(id).cloned()
    }

    /// This machine's wadspaces not in the Trash, in order.
    pub fn live(&self) -> Vec<Wadspace> {
        let m = self.machine_id();
        self.library.borrow().on(&m).into_iter().cloned().collect()
    }

    pub fn run(&self, id: &str) -> Option<WadspaceRun> {
        self.runs.borrow().get(id).cloned()
    }

    pub fn build(&self, id: &str) -> Option<WadspaceBuild> {
        self.builds.borrow().get(id).cloned()
    }

    pub fn job(&self, id: &str) -> Option<JobInfo> {
        self.jobs.borrow().get(id).cloned()
    }

    pub fn screen(&self) -> Option<ScreenInfo> {
        self.screen.borrow().clone()
    }

    /// It's running (or on its way up) on this machine.
    pub fn is_running(&self, id: &str) -> bool {
        self.run(id).is_some_and(|r| matches!(r.state.phase, Phase::Starting | Phase::Waiting | Phase::Ready))
    }

    /// How it's shown, while it runs.
    pub fn display(&self, id: &str) -> Option<SessionDisplay> {
        self.run(id)
            .filter(|r| matches!(r.state.phase, Phase::Starting | Phase::Waiting | Phase::Ready))
            .and_then(|r| r.display)
    }

    /// The running wadspaces, in the library's order.
    pub fn running(&self) -> Vec<Wadspace> {
        self.live().into_iter().filter(|w| self.is_running(&w.id)).collect()
    }

    /// Its tile's badges.
    pub fn tile_state(&self, id: &str) -> wad_gtk::TileState {
        let run = self.run(id);
        let build = self.build(id);
        let busy =
            build.as_ref().and_then(|b| b.job.as_deref()).and_then(|j| self.job(j)).map(|j| j.job.progress).or_else(
                || run.as_ref().filter(|r| matches!(r.state.phase, Phase::Starting | Phase::Waiting)).map(|_| 0.0),
            );
        wad_gtk::TileState {
            running: self.is_running(id),
            streaming: None,
            unbuilt: build.as_ref().is_some_and(|b| b.state.needs_build()),
            failed: run.is_some_and(|r| r.state.phase == Phase::Error),
            busy,
        }
    }
}
