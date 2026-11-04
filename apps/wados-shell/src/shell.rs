//! The shell's verbs, for every surface (the spike's `shell/controller.ts`
//! `getShell()`): what wadd knows ([`Store`]), asking it for things
//! ([`Link`]), the windows ([`Placer`]), the page (launcher or desktop), toasts,
//! and `wadsctl` commands of each module's own.
//!
//! Surfaces hold an `Rc<Shell>` and use only what's here; a module that
//! needs more adds a verb of its own on top of these.

use std::cell::{OnceCell, RefCell};
use std::rc::{Rc, Weak};

use gtk::{gdk, glib, prelude::*};
use wad_model::SessionDisplay;
use wad_model::edit::Op;
use wad_proto::{Event, Level, OpResult};

use crate::builder::Builder;
use crate::machine::{Machine, Page};
use crate::placer::Placer;
use crate::store::Store;
use crate::wadd::Link;

type CtlHandler = Box<dyn Fn(&[&str]) -> String>;

pub struct Shell {
    pub app: gtk::Application,
    pub store: Rc<Store>,
    pub link: Rc<Link>,
    pub placer: Rc<Placer>,
    pub primary: gdk::Monitor,
    pub others: Vec<gdk::Monitor>,
    machine: OnceCell<Weak<Machine>>,
    builder: OnceCell<Weak<Builder>>,
    ctl: RefCell<Vec<(String, CtlHandler)>>,
}

impl Shell {
    pub fn new(
        app: &gtk::Application,
        store: Rc<Store>,
        link: Rc<Link>,
        placer: Rc<Placer>,
        primary: gdk::Monitor,
        others: Vec<gdk::Monitor>,
    ) -> Rc<Self> {
        Rc::new(Self {
            app: app.clone(),
            store,
            link,
            placer,
            primary,
            others,
            machine: OnceCell::new(),
            builder: OnceCell::new(),
            ctl: RefCell::default(),
        })
    }

    pub(crate) fn set_machine(&self, m: &Rc<Machine>) {
        let _ = self.machine.set(Rc::downgrade(m));
    }

    pub(crate) fn set_builder(&self, b: &Rc<Builder>) {
        let _ = self.builder.set(Rc::downgrade(b));
    }

    /// The Builder (its window may be closed).
    pub fn builder(&self) -> Option<Rc<Builder>> {
        self.builder.get().and_then(Weak::upgrade)
    }

    fn machine(&self) -> Option<Rc<Machine>> {
        self.machine.get().and_then(Weak::upgrade)
    }

    // ------------------------------------------------------------ the page

    pub fn page(&self) -> Page {
        self.machine().map(|m| m.page()).unwrap_or(Page::Launcher)
    }

    /// To the launcher or the desktop (the scroll between them).
    pub fn go(&self, page: Page) {
        if let Some(m) = self.machine() {
            m.go(page);
        }
    }

    /// A flyout or menu that had the keys closed: on the desktop page with
    /// no window in front, the desktop has them back (labwc gives them to
    /// the topmost window, or no one). A moment later, so a window the
    /// close's click picked shows as in front first.
    pub fn keys_back(self: &Rc<Self>) {
        let me = Rc::downgrade(self);
        glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
            let Some(me) = me.upgrade() else { return };
            let in_front = me.placer.windows().iter().any(|t| t.activated && !t.minimized);
            if me.page() == Page::Desktop
                && !in_front
                && let Some(m) = me.machine()
            {
                m.desktop_keys();
            }
        });
    }

    /// The primary display's size, in logical pixels.
    pub fn size(&self) -> (i32, i32) {
        let g = self.primary.geometry();
        (g.width(), g.height())
    }

    // ------------------------------------------------------------ wadspaces

    /// Opens it (full screen, in a window, or in the background), building
    /// it first if it has to be; one that's running is shown that way.
    pub fn open(self: &Rc<Self>, id: &str, display: SessionDisplay) {
        let needs_build = self.store.build(id).is_some_and(|b| b.state.needs_build() && b.image.is_none());
        let me = self.clone();
        let id = id.to_string();
        if needs_build {
            self.toast(Level::Info, "Building it first: it opens when it's built", None);
            self.link.build(&id.clone(), move |_| {
                // It opens when its build is done: watch for it.
                me.open_when_built(&id, display);
            });
            return;
        }
        let again = id.clone();
        self.link.open(&id, display, move |_| me.placer.run_changed(&again));
    }

    fn open_when_built(self: &Rc<Self>, id: &str, display: SessionDisplay) {
        let me = Rc::downgrade(self);
        let id = id.to_string();
        let done = Rc::new(std::cell::Cell::new(false));
        self.store.watch(move |c| {
            if done.get() {
                return;
            }
            let (Some(me), crate::store::Change::Build(b)) = (me.upgrade(), c) else { return };
            if *b == id && me.store.build(&id).is_some_and(|b| b.image.is_some() && b.job.is_none()) {
                done.set(true);
                me.open(&id, display);
            }
        });
    }

    /// Its group, in the desktop's order: the desktop's loose wadspaces (work
    /// before gaming), or its tag folder's. Another machine's is alone.
    pub fn group_of(&self, id: &str) -> Vec<String> {
        let Some(x) = self.store.wadspace(id) else { return vec![] };
        let mut spaces = self.store.live();
        if !spaces.iter().any(|y| y.id == x.id) {
            return vec![x.id];
        }
        spaces.sort_by_key(|y| y.category_or_default() == wad_model::Category::Gaming);
        let tags = self.store.library().tags.clone();
        let tag = wad_model::tags::tag_of(&tags, &x).to_string();
        spaces.into_iter().filter(|y| wad_model::tags::tag_of(&tags, y) == tag).map(|y| y.id).collect()
    }

    /// Opened in the Builder with the rest of its group, it selected (a
    /// wadspace double-clicked on the desktop, its menu's Edit).
    pub fn edit(&self, id: &str) {
        if let Some(b) = self.builder() {
            b.open_group(&self.group_of(id), id);
        }
    }

    /// A new wadspace: empty glass on the desktop, in the Builder, its title ready to type.
    pub fn new_wadspace(&self) {
        if let Some(b) = self.builder() {
            b.new_wadspace();
        }
    }

    /// Brings a running wadspace up (its window to the front, as it's shown).
    pub fn show(self: &Rc<Self>, id: &str) {
        match self.store.display(id) {
            Some(SessionDisplay::Background) | None => self.open(id, SessionDisplay::Screen),
            Some(_) => self.placer.raise(id),
        }
    }

    pub fn stop(&self, id: &str) {
        self.link.stop(id);
    }

    pub fn build(&self, id: &str) {
        self.link.build(id, |_| {});
    }

    /// One edit; `done` with what it did (a refusal is toasted).
    pub fn op(&self, op: Op, done: impl FnOnce(OpResult) + 'static) {
        self.link.op(op, done);
    }

    pub fn undo(&self, id: &str) {
        self.link.undo(id);
    }

    // ------------------------------------------------------------ toasts

    /// Something to tell the person (the toasts show it, as wadd's notices).
    pub fn toast(&self, level: Level, title: &str, undo: Option<String>) {
        self.store.apply(Event::Notice { level, title: title.into(), undo });
    }

    // ------------------------------------------------------------ wadsctl

    /// `wadsctl <word> …` comes to `f` (with what follows the word).
    pub fn on_ctl(&self, word: &str, f: impl Fn(&[&str]) -> String + 'static) {
        self.ctl.borrow_mut().push((word.to_string(), Box::new(f)));
    }

    /// A module's own command, if `words` is one.
    pub fn ctl(&self, words: &[&str]) -> Option<String> {
        let (first, rest) = words.split_first()?;
        let handlers = self.ctl.borrow();
        handlers.iter().find(|(w, _)| w == first).map(|(_, f)| f(rest))
    }
}
