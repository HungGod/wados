//! The content filters WebKit blocks with: the lists' (EasyList and
//! EasyPrivacy) and the social blocker's, compiled once by WebKit's
//! `UserContentFilterStore` into `<cache>/filters/` under an id that's the
//! lists' hash (`wadbrowser_privacy::filter_identifier`). A start with the
//! same lists only loads what was compiled; the lists are read, and on a cache
//! miss turned into WebKit's rules, off the main thread (WebKit compiles on
//! its own).
//!
//! Each view's UserContentManager has the filters its page's site should: none
//! where the shield's toggle allowed the site, the social one only with "Block
//! all social media" on. They're put right as each top-level page starts and
//! commits, and again when a toggle flips.

use crate::engine::{Engine, engine};
use crate::testlog;
use crate::view::{self, ViewState};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};
use tracing::{info, warn};
use wadbrowser_core::AppPaths;
use wadbrowser_privacy::{Ready, content, filter_identifier};
use webkit6::{UserContentFilter, UserContentFilterStore, WebView, gio, glib};

/// How long a page waits for the filters at the start (a cached one loads in
/// well under this; the first compile of EasyList doesn't, and pages then go
/// ahead without it).
const WAIT: Duration = Duration::from_secs(2);

pub(crate) struct Filters {
    store: UserContentFilterStore,
    lists: RefCell<Option<UserContentFilter>>,
    social: RefCell<Option<UserContentFilter>>,
    /// The lists' filter is in, or won't be: loads needn't wait.
    settled: Cell<bool>,
    waiting: RefCell<Vec<Box<dyn FnOnce()>>>,
}

/// Which filters a view has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Applied {
    pub lists: bool,
    pub social: bool,
}

impl Filters {
    pub(crate) fn new(paths: &AppPaths) -> Self {
        let dir = paths.filter_cache_dir();
        let _ = std::fs::create_dir_all(&dir);
        Self {
            store: UserContentFilterStore::new(&dir.to_string_lossy()),
            lists: RefCell::default(),
            social: RefCell::default(),
            settled: Cell::new(false),
            waiting: RefCell::default(),
        }
    }

    /// Whether the lists' filter is in (or won't be).
    pub(crate) fn settled(&self) -> bool {
        self.settled.get()
    }

    fn settle(&self) {
        if self.settled.replace(true) {
            return;
        }
        let waiting = std::mem::take(&mut *self.waiting.borrow_mut());
        for f in waiting {
            f();
        }
    }
}

type Once = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;

/// Runs `f` once the lists' filter is in (or after [`WAIT`], whichever comes first).
pub fn when_ready(f: impl FnOnce() + 'static) {
    let Some(e) = engine() else { return f() };
    if e.filters.settled() || !e.privacy.is_enabled() {
        return f();
    }
    let once: Once = Rc::new(RefCell::new(Some(Box::new(f))));
    let a = once.clone();
    e.filters.waiting.borrow_mut().push(Box::new(move || {
        if let Some(f) = a.take() {
            f()
        }
    }));
    glib::timeout_add_local_once(WAIT, move || {
        if let Some(f) = once.take() {
            f()
        }
    });
}

/// Starts loading the filters: the social one now, the lists' once they're read.
pub(crate) fn start(e: &Rc<Engine>) {
    load_social(e);
    glib::timeout_add_local(Duration::from_millis(50), || {
        let Some(e) = engine() else { return glib::ControlFlow::Break };
        match e.privacy.poll() {
            Some(ready) => {
                load_lists(&e, ready);
                glib::ControlFlow::Break
            }
            None if e.privacy.ready().is_some() => glib::ControlFlow::Break,
            None => glib::ControlFlow::Continue,
        }
    });
}

fn load_lists(e: &Rc<Engine>, ready: Ready) {
    let started = Instant::now();
    let id = ready.identifier.clone();
    e.filters.store.load(&ready.identifier.clone(), None::<&gio::Cancellable>, move |res| match res {
        Ok(filter) => {
            let ms = started.elapsed().as_millis();
            info!("content filter {id}: loaded from the cache in {ms} ms");
            testlog::line("filter-loaded", &[&id, &ms.to_string()]);
            installed(Some(filter), &id);
        }
        Err(_) => compile_lists(ready),
    });
}

/// The lists into WebKit's rules on a thread, then WebKit compiles and keeps them.
fn compile_lists(ready: Ready) {
    let started = Instant::now();
    let (tx, rx) = async_channel::bounded(1);
    let lists = ready.lists.clone();
    let spawned = std::thread::Builder::new().name("wadbrowser-filter".into()).spawn(move || {
        let _ = tx.send_blocking(content::lists_rules(&lists).map_err(|e| format!("{e:#}")));
    });
    if let Err(e) = spawned {
        warn!("can't convert the filter lists: {e}");
        return installed(None, &ready.identifier);
    }
    glib::spawn_future_local(async move {
        let Ok(converted) = rx.recv().await else { return installed(None, &ready.identifier) };
        let (json, n) = match converted {
            Ok(v) => v,
            Err(err) => {
                warn!("the filter lists didn't convert: {err}");
                return installed(None, &ready.identifier);
            }
        };
        let converting = started.elapsed();
        let Some(e) = engine() else { return };
        let id = ready.identifier.clone();
        let bytes = glib::Bytes::from_owned(json.into_bytes());
        e.filters.store.save(&id.clone(), &bytes, None::<&gio::Cancellable>, move |res| match res {
            Ok(filter) => {
                let ms = started.elapsed().as_millis();
                info!(
                    "content filter {id}: compiled {n} rules in {ms} ms ({} ms converting, {} ms WebKit's)",
                    converting.as_millis(),
                    ms - converting.as_millis()
                );
                testlog::line("filter-compiled", &[&id, &ms.to_string(), &n.to_string()]);
                installed(Some(filter), &id);
                forget_others(&id);
            }
            Err(err) => {
                warn!("WebKit didn't compile the content filter: {err}");
                installed(None, &id);
            }
        });
    });
}

fn installed(filter: Option<UserContentFilter>, id: &str) {
    let Some(e) = engine() else { return };
    let had = filter.is_some();
    *e.filters.lists.borrow_mut() = filter;
    e.filters.settle();
    if had {
        reapply_all();
    } else {
        testlog::line("filter-failed", &[id]);
    }
}

/// The lists' filter compiled before this one (lists that changed since)
/// goes: the store's id of the last is kept beside it.
fn forget_others(current: &str) {
    let Some(e) = engine() else { return };
    let file = e.paths.filter_cache_dir().join("current-filter");
    let before = std::fs::read_to_string(&file).unwrap_or_default();
    let before = before.trim();
    if !before.is_empty() && before != current && before.starts_with(content::LISTS_PREFIX) {
        e.filters.store.remove(before, None::<&gio::Cancellable>, |_| {});
    }
    if let Err(err) = std::fs::write(&file, current) {
        warn!("{}: {err}", file.display());
    }
}

fn load_social(e: &Rc<Engine>) {
    let id = content::social_identifier();
    let store = e.filters.store.clone();
    e.filters.store.load(&id.clone(), None::<&gio::Cancellable>, move |res| match res {
        Ok(f) => social_in(f),
        Err(_) => {
            let bytes = glib::Bytes::from_owned(content::social_rules().into_bytes());
            store.save(&id, &bytes, None::<&gio::Cancellable>, |res| match res {
                Ok(f) => social_in(f),
                Err(err) => warn!("WebKit didn't compile the social blocker: {err}"),
            });
        }
    });
}

fn social_in(f: UserContentFilter) {
    let Some(e) = engine() else { return };
    *e.filters.social.borrow_mut() = Some(f);
    reapply_all();
}

/// What `uri`'s page should have.
fn wanted(e: &Engine, uri: &str) -> Applied {
    let web = uri.starts_with("http://") || uri.starts_with("https://");
    let allowed = e.privacy.allowlist().contains_url(uri);
    let on = web && !allowed && e.privacy.is_enabled();
    Applied { lists: on, social: web && !allowed && e.prefs.borrow().block_social }
}

/// Puts `view`'s filters right for `uri` (its top-level page, or the one it's going to).
pub(crate) fn apply(view: &WebView, st: &ViewState, uri: &str) {
    let Some(e) = engine() else { return };
    let want = wanted(&e, uri);
    let have = st.applied.get();
    let ucm = &st.ucm;
    let lists = e.filters.lists.borrow();
    let social = e.filters.social.borrow();
    // What's in the UCM is what was added while the filter was there.
    let now = Applied { lists: want.lists && lists.is_some(), social: want.social && social.is_some() };
    if now == have {
        return;
    }
    if let Some(f) = lists.as_ref()
        && have.lists != now.lists
    {
        if now.lists { ucm.add_filter(f) } else { ucm.remove_filter(f) }
    }
    if let Some(f) = social.as_ref()
        && have.social != now.social
    {
        if now.social { ucm.add_filter(f) } else { ucm.remove_filter(f) }
    }
    st.applied.set(now);
    let _ = view;
}

/// Every view's filters again (a filter came in, a toggle flipped).
pub(crate) fn reapply_all() {
    for v in view::live_views() {
        if let Some(st) = view::state(&v) {
            let uri = st.page_uri(&v);
            // A filter that changed (new lists) isn't the one the UCM has: take every one out first.
            if st.applied.get() != Applied::default() {
                st.ucm.remove_all_filters();
                st.applied.set(Applied::default());
            }
            apply(&v, &st, &uri);
        }
    }
}

/// The lists' filter's id for these lists (for tests and the Privacy page).
pub fn identifier_for(lists: &[String]) -> String {
    filter_identifier(lists)
}
