//! The browser engine's state, one per process on the main thread: the
//! folders, settings, store and blocker the views use, the content filters,
//! and what the UI does when a page asks for something a window does
//! ([`Hooks`]: a popup's tab, a page closing itself, a tab moving into a key's
//! session, a permission prompt, a `cactus://` page's message).

use crate::session::SessionBundle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use wadbrowser_core::zoom::SiteZoom;
use wadbrowser_core::{AppPaths, Preferences, Store};
use wadbrowser_privacy::PrivacyEngine;
use webkit6::{WebView, glib};

pub struct Engine {
    pub paths: AppPaths,
    pub prefs: Rc<RefCell<Preferences>>,
    pub store: Arc<Store>,
    pub privacy: Rc<PrivacyEngine>,
    pub(crate) filters: crate::filters::Filters,
    pub(crate) zoom: RefCell<SiteZoom>,
    /// Blocked requests not yet added to the store's total.
    pub(crate) unsaved_blocks: Cell<i64>,
}

/// A hook on a view.
pub type ViewHook = Box<dyn Fn(&WebView)>;
/// A hook on a view and another (a popup and its opener).
pub type ViewsHook = Box<dyn Fn(&WebView, &WebView)>;
/// A hook on a view moving into a session, at an address.
pub type MoveHook = Box<dyn Fn(&WebView, &SessionBundle, &str)>;
/// A hook on a view and a message.
pub type MessageHook = Box<dyn Fn(&WebView, &str)>;
/// A hook on a view's permission question.
pub type AskHook = Box<dyn Fn(&WebView, crate::permissions::Ask)>;

/// What the UI does for a page (each is called on a later turn of the main
/// loop, never inside WebKit's signal).
pub struct Hooks {
    /// A page opened `popup` (window.open, a link to a new window): show it as
    /// a tab beside `opener`'s, selected. It keeps `window.opener`.
    pub open_popup: ViewsHook,
    /// The page closed itself (`window.close()`): close its tab.
    pub close_view: ViewHook,
    /// A top-level page on a key's site, in a tab that isn't in the key's
    /// profile: put a new view in `session` (the key's) in the tab's place,
    /// loading `uri` (`create_web_view` then `navigate`).
    pub move_view: MoveHook,
    /// A `cactus://` page posted `json` to its "cactus" message handler
    /// (wadbrowser-core's `internal::PageAction`).
    pub page_action: MessageHook,
    /// A site asks for the camera, the microphone, notifications…: ask the
    /// person, then `ask.answer(allow, remember)`. Dropping it unanswered denies.
    pub ask_permission: AskHook,
}

thread_local! {
    static ENGINE: RefCell<Option<Rc<Engine>>> = const { RefCell::new(None) };
    static HOOKS: RefCell<Option<Rc<Hooks>>> = const { RefCell::new(None) };
}

/// Starts the engine (once; a later call gives the first's): registers
/// `cactus://`, starts reading the filter lists off the main thread, then
/// loads (or the first time, compiles) the content filters.
pub fn init(
    paths: AppPaths,
    prefs: Rc<RefCell<Preferences>>,
    store: Arc<Store>,
    privacy: Rc<PrivacyEngine>,
) -> Rc<Engine> {
    if let Some(e) = engine() {
        return e;
    }
    crate::scheme::register();
    if let Ok(list) = store.allowlist_all() {
        privacy.allowlist().replace_all(list);
    }
    let _ = privacy.load(&paths);
    let filters = crate::filters::Filters::new(&paths);
    let zoom = RefCell::new(SiteZoom::open(&paths));
    let e = Rc::new(Engine { paths, prefs, store, privacy, filters, zoom, unsaved_blocks: Cell::new(0) });
    ENGINE.set(Some(e.clone()));
    crate::filters::start(&e);
    // The total blocked, saved every few seconds rather than per request.
    glib::timeout_add_local(Duration::from_secs(3), || {
        if let Some(e) = engine() {
            e.save_blocks();
        }
        glib::ControlFlow::Continue
    });
    e
}

/// The engine, once it's started.
pub fn engine() -> Option<Rc<Engine>> {
    ENGINE.with_borrow(|e| e.clone())
}

/// What the UI does for pages (set once, before the first view).
pub fn set_hooks(hooks: Hooks) {
    HOOKS.set(Some(Rc::new(hooks)));
}

pub(crate) fn hooks() -> Option<Rc<Hooks>> {
    HOOKS.with_borrow(|h| h.clone())
}

/// Runs `f` with the hooks on the next turn of the main loop.
pub(crate) fn later(f: impl FnOnce(&Hooks) + 'static) {
    glib::idle_add_local_once(move || {
        if let Some(h) = hooks() {
            f(&h);
        }
    });
}

impl Engine {
    pub(crate) fn save_blocks(&self) {
        let n = self.unsaved_blocks.replace(0);
        if n > 0 {
            wadbrowser_core::metrics::record_blocks(&self.store, n);
        }
    }

    /// The browser's whole total: saved, and not yet.
    pub fn total_blocked(&self) -> u64 {
        let saved = self.store.get_stat(wadbrowser_core::metrics::STAT_BLOCKS).unwrap_or(0);
        (saved + self.unsaved_blocks.get()).max(0) as u64
    }
}
