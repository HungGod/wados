//! What both bars (the launcher's dock and the desktop's top bar), the
//! extra displays' bars and the overlays share, made once: this session's
//! notices (the bell), wadd's jobs (the tray's ring), which windows were in
//! front most lately, and what the machine says of itself (the network…).
//! Each says when it changes, by topic.

use std::cell::{OnceCell, RefCell};
use std::rc::{Rc, Weak};

use gtk::{glib, prelude::*};
use indexmap::IndexMap;
use wad_model::JobStatus;
use wad_proto::JobInfo;

use super::notices::Notices;
use super::system::Status;
use super::tasks::Recency;
use super::{Halves, Middle};
use crate::machine::Page;
use crate::shell::Shell;
use crate::store::Change;
use crate::wm::Top;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topic {
    Notices,
    Jobs,
    Windows,
    System,
}

/// Someone told when a topic changes.
type Sub = (Topic, Box<dyn Fn()>);

pub struct Shared {
    pub notices: RefCell<Notices>,
    pub jobs: RefCell<IndexMap<String, JobInfo>>,
    pub recent: RefCell<Recency>,
    pub system: RefCell<Status>,
    /// Changes `system` (system.rs's own, so it keeps what's told it).
    set_system: RefCell<Option<super::system::Set>>,
    subs: RefCell<Vec<Sub>>,
    /// The halves (true: the bar's own, which stays on the desktop).
    halves: RefCell<Vec<(bool, Weak<Halves>)>>,
    middle: RefCell<Weak<Middle>>,
}

thread_local! {
    static SHARED: OnceCell<Rc<Shared>> = const { OnceCell::new() };
}

/// The one there is (made, and started following everything, the first time).
pub fn get(shell: &Rc<Shell>) -> Rc<Shared> {
    if let Some(s) = SHARED.with(|c| c.get().cloned()) {
        return s;
    }
    let s = Rc::new(Shared {
        notices: RefCell::default(),
        jobs: RefCell::default(),
        recent: RefCell::default(),
        system: RefCell::default(),
        set_system: RefCell::default(),
        subs: RefCell::default(),
        halves: RefCell::default(),
        middle: RefCell::default(),
    });
    SHARED.with(|c| {
        let _ = c.set(s.clone());
    });
    s.start(shell);
    s
}

pub fn now() -> i64 {
    glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0)
}

impl Shared {
    fn start(self: &Rc<Self>, shell: &Rc<Shell>) {
        let s = Rc::downgrade(shell);
        super::ui::on_closed(move || {
            if let Some(s) = s.upgrade() {
                s.keys_back();
            }
        });
        let (w, s) = (Rc::downgrade(self), shell.clone());
        shell.store.watch(move |c| {
            let Some(me) = w.upgrade() else { return };
            match c {
                Change::Notice { level, title, undo, body, icon, action, .. } => {
                    let more = super::notices::More { body: body.clone(), icon: icon.clone(), action: action.clone() };
                    me.notices.borrow_mut().push_more(*level, title, undo.clone(), more, now());
                    me.emit(Topic::Notices);
                }
                Change::Job(id) => {
                    match s.store.job(id) {
                        Some(j) => {
                            me.jobs.borrow_mut().insert(id.clone(), j);
                        }
                        None => {
                            me.jobs.borrow_mut().shift_remove(id);
                        }
                    }
                    me.emit(Topic::Jobs);
                }
                _ => {}
            }
        });
        let w = Rc::downgrade(self);
        shell.placer.on_windows(move |list: &[Top]| {
            if let Some(me) = w.upgrade() {
                me.recent.borrow_mut().update(list);
                me.emit(Topic::Windows);
            }
        });
        let w = Rc::downgrade(self);
        let set = super::system::start(Rc::new(move |st| {
            if let Some(me) = w.upgrade() {
                *me.system.borrow_mut() = st.clone();
                me.emit(Topic::System);
            }
        }));
        *self.set_system.borrow_mut() = Some(set);
        self.ctl(shell);
    }

    /// What the machine says of itself, told from elsewhere (the session's keys).
    pub fn update_system(&self, f: &dyn Fn(&mut Status)) {
        let set = self.set_system.borrow().clone();
        if let Some(set) = set {
            set(f);
        }
    }

    /// `f` each time `topic` changes.
    pub fn on(&self, topic: Topic, f: impl Fn() + 'static) {
        self.subs.borrow_mut().push((topic, Box::new(f)));
    }

    pub fn emit(&self, topic: Topic) {
        for (t, f) in self.subs.borrow().iter() {
            if *t == topic {
                f();
            }
        }
    }

    pub fn add_halves(&self, fixed: bool, h: &Rc<Halves>) {
        self.halves.borrow_mut().push((fixed, Rc::downgrade(h)));
    }

    pub fn set_middle(&self, m: &Rc<Middle>) {
        *self.middle.borrow_mut() = Rc::downgrade(m);
    }

    /// The halves on the screen: the bar's on the desktop, the launcher's dock on the launcher.
    pub fn halves_shown(&self, shell: &Shell) -> Option<Rc<Halves>> {
        let want_bar = shell.page() == Page::Desktop;
        self.halves.borrow().iter().find(|(fixed, _)| *fixed == want_bar).and_then(|(_, h)| h.upgrade())
    }

    /// The jobs the tray shows (not boot's or the tailnet's own), running ones.
    pub fn running_jobs(&self) -> Vec<JobInfo> {
        self.jobs.borrow().values().filter(|j| j.job.visible && j.job.status == JobStatus::Running).cloned().collect()
    }

    /// `wadsctl bar …`: what the bar shows, and opening its flyouts and menus
    /// (for tests and screenshots).
    fn ctl(self: &Rc<Self>, shell: &Rc<Shell>) {
        let (w, s) = (Rc::downgrade(self), shell.clone());
        shell.on_ctl("bar", move |args| {
            let Some(me) = w.upgrade() else { return "error gone".into() };
            let halves = me.halves_shown(&s);
            let middle = me.middle.borrow().upgrade();
            match args {
                [] => {
                    let running: Vec<String> = s.store.running().into_iter().map(|w| w.id).collect();
                    let tasks = middle.map(|m| m.describe()).unwrap_or_default();
                    format!(
                        "ok page={:?} running=[{}] {} unread={} jobs={} known=[{}]",
                        s.page(),
                        running.join(","),
                        tasks,
                        me.notices.borrow().unread(),
                        me.running_jobs().len(),
                        me.jobs
                            .borrow()
                            .values()
                            .map(|j| format!(
                                "{}:{:?}{}",
                                j.job.id,
                                j.job.status,
                                if j.job.visible { "" } else { "(hidden)" }
                            ))
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                }
                ["sizes"] => match halves {
                    Some(h) => {
                        let m = |w: &gtk::Widget| {
                            let (min, nat, _, _) = w.measure(gtk::Orientation::Horizontal, -1);
                            let mut kids = vec![];
                            let mut c = w.first_child();
                            while let Some(k) = c {
                                let (a, b, _, _) = k.measure(gtk::Orientation::Horizontal, -1);
                                kids.push(format!("{}:{a}/{b}/{}", k.type_().name(), k.width()));
                                c = k.next_sibling();
                            }
                            format!("{min}/{nat}/{} [{}]", w.width(), kids.join(" "))
                        };
                        let mid = middle.map(|x| m(&x.widget().first_child().unwrap())).unwrap_or_default();
                        format!("ok left={} right={} running={mid}", m(&h.left()), m(&h.right()))
                    }
                    None => "error no halves".into(),
                },
                ["flyout"] | ["flyout", "none"] => {
                    if let Some(h) = halves {
                        h.close_all();
                    }
                    "ok".into()
                }
                ["flyout", name] => match halves.map(|h| h.open(name)) {
                    Some(true) => "ok".into(),
                    _ => format!("error no flyout {name}"),
                },
                ["menu", id] => match middle.map(|m| m.menu(id)) {
                    Some(true) => "ok".into(),
                    _ => format!("error {id} isn't in the bar"),
                },
                ["peek", app] => match middle.map(|m| m.peek(app)) {
                    Some(true) => "ok".into(),
                    _ => format!("error no windows of {app}"),
                },
                ["click", what] => match middle.map(|m| m.click(what)) {
                    Some(true) => "ok".into(),
                    _ => format!("error {what} isn't in the bar"),
                },
                ["where", what] => match middle.and_then(|m| m.where_(what)) {
                    Some((x, y)) => format!("ok {x:.0} {y:.0}"),
                    None => format!("error {what} isn't in the bar"),
                },
                _ => "error bar [flyout <name>|flyout none|menu <wadspace|#window>|peek <app>|click <wadspace|app|#window>|where <wadspace|app|#window>|sizes]".into(),
            }
        });
    }
}
