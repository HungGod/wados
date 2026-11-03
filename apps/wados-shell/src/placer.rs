//! Where each wadspace's window goes, from how wadd says it's shown (its
//! run's display): **screen** is full screen on the primary display (the
//! launcher and desktop are under it), **window** an ordinary window,
//! **background** minimized (it keeps running). wadd starts the container;
//! the shell places its window (docs/decisions/0002). A window is a
//! wadspace's when its app id is `io.wadspaces.<id>` (0001's labwc patch).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use wad_model::SessionDisplay;

use crate::store::Store;
use crate::wm::{Cmd, Top, Which, Wm};

pub struct Placer {
    wm: Option<Rc<Wm>>,
    store: Rc<Store>,
    /// The primary display's connector ("eDP-1", "HEADLESS-1").
    primary: RefCell<String>,
    windows: RefCell<Vec<Top>>,
    /// What each window was last told (by its toplevel id), so it's told once.
    applied: RefCell<HashMap<u32, SessionDisplay>>,
}

impl Placer {
    pub fn new(wm: Option<Rc<Wm>>, store: Rc<Store>, primary: &str) -> Rc<Self> {
        Rc::new(Self {
            wm,
            store,
            primary: RefCell::new(primary.to_string()),
            windows: RefCell::default(),
            applied: RefCell::default(),
        })
    }

    /// Every window on the machine, as foreign-toplevel last said.
    pub fn windows(&self) -> Vec<Top> {
        self.windows.borrow().clone()
    }

    /// A wadspace's windows (one per display its labwc draws on).
    pub fn windows_of(&self, ws: &str) -> Vec<Top> {
        self.windows.borrow().iter().filter(|t| t.wadspace() == Some(ws)).cloned().collect()
    }

    pub fn wm(&self) -> Option<&Rc<Wm>> {
        self.wm.as_ref()
    }

    pub fn set_primary(&self, connector: &str) {
        *self.primary.borrow_mut() = connector.to_string();
        self.applied.borrow_mut().clear();
        self.place_all();
    }

    /// foreign-toplevel's list changed.
    pub fn windows_changed(&self, list: Vec<Top>) {
        let ids: Vec<u32> = list.iter().map(|t| t.id).collect();
        self.applied.borrow_mut().retain(|id, _| ids.contains(id));
        *self.windows.borrow_mut() = list;
        self.place_all();
    }

    /// A wadspace's run changed (it may be shown another way now).
    pub fn run_changed(&self, ws: &str) {
        let tops = self.windows_of(ws);
        {
            let mut applied = self.applied.borrow_mut();
            for t in &tops {
                applied.remove(&t.id);
            }
        }
        for t in &tops {
            self.place(t);
        }
    }

    /// Puts `ws` in front again (as it's shown: full screen or its window).
    pub fn raise(&self, ws: &str) {
        let Some(wm) = &self.wm else { return };
        for t in self.windows_of(ws) {
            wm.send(Cmd::Activate(Which(format!("#{}", t.id))));
        }
    }

    fn place_all(&self) {
        for t in self.windows() {
            self.place(&t);
        }
    }

    fn place(&self, t: &Top) {
        let (Some(wm), Some(ws)) = (&self.wm, t.wadspace()) else { return };
        let Some(want) = self.store.display(ws) else { return };
        if self.applied.borrow().get(&t.id) == Some(&want) {
            return;
        }
        let which = || Which(format!("#{}", t.id));
        match want {
            SessionDisplay::Screen => {
                wm.send(Cmd::Fullscreen(which(), Some(self.primary.borrow().clone())));
                wm.send(Cmd::Activate(which()));
            }
            SessionDisplay::Window => {
                wm.send(Cmd::Unfullscreen(which()));
                wm.send(Cmd::Activate(which()));
            }
            SessionDisplay::Background => wm.send(Cmd::Minimize(which())),
            // A stream's desktop isn't on this screen.
            SessionDisplay::Stream => {}
        }
        tracing::debug!("{ws}: its window #{} is {want:?}", t.id);
        self.applied.borrow_mut().insert(t.id, want);
    }
}
