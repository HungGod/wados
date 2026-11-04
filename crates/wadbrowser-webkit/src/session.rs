//! A WebKit network session per browser profile (wadbrowser-core's
//! `profiles` says what a profile is): its cookies in a SQLite file in its
//! data folder, kept across restarts (Cactus never set the cookie
//! manager's persistent storage, so they were lost); a private window's,
//! an `ephemeral-…` profile's and an ephemeral run's in memory. Each
//! session is made once and shared by every view in that profile; each
//! has one download handler.

use anyhow::Result;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use wadbrowser_core::AppPaths;
use wadbrowser_core::profiles::{self, SignIns, Storage};
use webkit6::{CookiePersistentStorage, NetworkSession, WebsiteDataManager, WebsiteDataTypes, gio, glib};

thread_local! {
    static SESSIONS: RefCell<HashMap<String, NetworkSession>> = RefCell::new(HashMap::new());
    static EPHEMERAL_RUN: Cell<bool> = const { Cell::new(false) };
    static SIGNINS: RefCell<Option<Rc<SignIns>>> = const { RefCell::new(None) };
    static NEXT_PRIVATE: Cell<u64> = const { Cell::new(1) };
}

/// A run that keeps nothing: every profile is ephemeral but a key's
/// (`wadbrowser_core::profiles::resolve`). Set it before the first session
/// is made (the app does, from its command line); it's off by default.
pub fn set_ephemeral_run(on: bool) {
    EPHEMERAL_RUN.set(on);
}

pub fn ephemeral_run() -> bool {
    EPHEMERAL_RUN.get()
}

/// The keys: this wadspace's (signins.json, read once; `WADBROWSER_SIGNINS`
/// names another file), or on the machine wadd's, once they've come
/// ([`set_machine_keys`]; none until then).
pub fn signins() -> Rc<SignIns> {
    SIGNINS.with_borrow_mut(|s| s.get_or_insert_with(|| Rc::new(SignIns::here())).clone())
}

/// The machine's keys, from wadd (`HOOK(keys-on-machine)`: the UI fetches
/// them off the main thread). Ignored in a wadspace (its signins.json's
/// are its keys). True if they changed: pages on a key's site then go into
/// it (`crate::view::rehome`).
pub fn set_machine_keys(keys: SignIns) -> bool {
    if SignIns::file_here() {
        return false;
    }
    let changed = SIGNINS.with_borrow_mut(|s| {
        if s.as_deref() == Some(&keys) {
            return false;
        }
        *s = Some(Rc::new(keys));
        true
    });
    if changed {
        // A key's session made before its folder was known keeps nothing: the next is its folder's.
        SESSIONS.with_borrow_mut(|all| {
            all.retain(|id, s| !(id.starts_with(profiles::SIGNIN) && s.is_ephemeral()));
        });
        tracing::info!(keys = signins().signins.len(), "the machine's keys");
    }
    changed
}

/// Profile `id`'s session, made the first time it's asked for.
pub fn network_session(paths: &AppPaths, id: &str) -> NetworkSession {
    if let Some(s) = SESSIONS.with_borrow(|all| all.get(id).cloned()) {
        return s;
    }
    let s = make(paths, id);
    SESSIONS.with_borrow_mut(|all| all.insert(id.to_string(), s.clone()));
    s
}

/// Lets an ephemeral profile's session go (its last window closed): what it
/// kept in memory goes with it. A profile kept on disk stays.
pub fn forget(id: &str) {
    SESSIONS.with_borrow_mut(|all| {
        if all.get(id).is_some_and(|s| s.is_ephemeral()) {
            all.remove(id);
        }
    });
}

fn make(paths: &AppPaths, id: &str) -> NetworkSession {
    let storage = profiles::resolve(id, paths, &signins().signins, ephemeral_run());
    let session = match &storage {
        Storage::Ephemeral => NetworkSession::new_ephemeral(),
        Storage::Disk { data, cache } => {
            for d in [data, cache] {
                if let Err(e) = std::fs::create_dir_all(d) {
                    tracing::warn!("profile {id}: {}: {e}", d.display());
                }
            }
            let s = NetworkSession::new(Some(&data.to_string_lossy()), Some(&cache.to_string_lossy()));
            // Kept across restarts.
            if let (Some(cookies), Some(file)) = (s.cookie_manager(), storage.cookies()) {
                cookies.set_persistent_storage(&file.to_string_lossy(), CookiePersistentStorage::Sqlite);
            }
            s
        }
    };
    session.set_itp_enabled(true);
    crate::downloads::watch(&session, id);
    tracing::info!(profile = id, kept = !matches!(storage, Storage::Ephemeral), "a network session");
    session
}

/// A browsing profile's network session, as a window or a view uses it.
#[derive(Clone)]
pub struct SessionBundle {
    pub network: NetworkSession,
    /// Its profile's id ("default", "work", "signin-<key>", "private-3"…).
    pub profile: String,
    /// A private window's (it keeps nothing, and its tabs never go into a key).
    pub private: bool,
}

impl SessionBundle {
    /// The browser's own windows' profile: the key whose apps list
    /// `@browser` (in a wadspace), else `default`.
    pub fn persistent(paths: &AppPaths) -> Result<Self> {
        Ok(Self::for_profile(paths, &signins().browser_profile()))
    }

    /// A new private window's: a session of its own, in memory.
    pub fn ephemeral() -> Self {
        let n = NEXT_PRIVATE.get();
        NEXT_PRIVATE.set(n + 1);
        let profile = profiles::private_profile(n);
        let network = NetworkSession::new_ephemeral();
        network.set_itp_enabled(true);
        crate::downloads::watch(&network, &profile);
        SESSIONS.with_borrow_mut(|all| all.insert(profile.clone(), network.clone()));
        Self { network, profile, private: true }
    }

    /// Profile `id`'s.
    pub fn for_profile(paths: &AppPaths, id: &str) -> Self {
        Self { network: network_session(paths, id), profile: id.to_string(), private: profiles::is_private(id) }
    }

    /// A window's (`WindowSpec`'s `profile` and `private`): a private one's
    /// own, else the profile's, else the browser's.
    pub fn for_window(paths: &AppPaths, profile: Option<&str>, private: bool) -> Result<Self> {
        match (private, profile) {
            (true, _) => Ok(Self::ephemeral()),
            (false, Some(id)) => Ok(Self::for_profile(paths, id)),
            (false, None) => Self::persistent(paths),
        }
    }

    /// Whether it keeps nothing (private, ephemeral, or an ephemeral run's).
    pub fn keeps_nothing(&self) -> bool {
        self.network.is_ephemeral()
    }

    pub fn clear_http_cache(&self) {
        if let Some(manager) = self.network.website_data_manager() {
            clear(&manager, WebsiteDataTypes::MEMORY_CACHE | WebsiteDataTypes::DISK_CACHE);
        }
    }

    pub fn clear_cookies_and_site_data(&self) {
        if let Some(manager) = self.network.website_data_manager() {
            clear(&manager, WebsiteDataTypes::ALL);
        }
    }
}

fn clear(manager: &WebsiteDataManager, what: WebsiteDataTypes) {
    manager.clear(what, glib::TimeSpan(0), None::<&gio::Cancellable>, |r| {
        if let Err(e) = r {
            tracing::warn!("clearing site data: {e}");
        }
    });
}
