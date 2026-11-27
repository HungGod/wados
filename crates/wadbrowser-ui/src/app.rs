//! The application: one WadBrowser per user (or per `--instance`), which
//! takes every launch's command line.
//!
//! GApplication's uniqueness is the hand-off: the first `wadbrowser` owns the
//! application id on the session bus (`io.wados.Browser`, or the instance's:
//! `wadbrowser_core::instance`); a later one sends its arguments to it over
//! D-Bus (`HANDLES_COMMAND_LINE`) and exits with what it answered (0, or 1
//! with why on its stderr), without ever opening a display. The browser's
//! state (its store, its blocker, its network session) is only made in the
//! first, as it starts up. Each command line goes through
//! `wadbrowser_core::modes::plan`, which says which window it opens in.
//!
//! Its settings are wadbrowser.conf's (`wadbrowser_core::conf`): read as it
//! starts, and again whenever either file changes ([`follow_conf`]), so a
//! change in the Builder or in Settings ▸ Browser counts in the windows
//! already open. The first window opens at its home, not the last session's
//! tabs (those only with `--restore`).

use crate::window::{self, AppState, restore_urls};
use anyhow::Result;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use tracing::{info, warn};
use wadbrowser_core::conf::{self, Conf};
use wadbrowser_core::modes::{self, Context, Plan, WindowSpec};
use wadbrowser_core::signin::{self, Provider};
use wadbrowser_core::{
    APP_VERSION, CACTUS_COMMIT, NEW_TAB_URI, Preferences, Request, Store, cli, help_text, instance,
    normalize_navigation_input,
};
use wadbrowser_privacy::PrivacyEngine;
use wadbrowser_webkit::SessionBundle;

/// Runs the browser for `req` (this launch's own arguments, already
/// checked): as the browser, or by handing them to the one running. Returns
/// the exit status.
pub fn run(req: &Request) -> Result<i32> {
    let app = adw::Application::builder()
        .application_id(instance::app_id(req.instance.as_deref()))
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    // Made in the browser only (startup), never in a launch that hands over.
    let state: Rc<RefCell<Option<Rc<AppState>>>> = Rc::default();
    let first = Rc::new(Cell::new(true));

    let (inst, ephemeral) = (req.instance.clone(), req.ephemeral);
    let st = state.clone();
    app.connect_startup(move |_| {
        // Dark only.
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
        match start(inst.as_deref(), ephemeral) {
            Ok(s) => *st.borrow_mut() = Some(s),
            Err(e) => tracing::error!("WadBrowser can't start: {e:#}"),
        }
    });

    let st = state.clone();
    app.connect_command_line(move |app, cmdline| {
        let Some(state) = st.borrow().clone() else {
            cmdline.printerr_literal("wadbrowser: the browser couldn't start (see its log)\n");
            return glib::ExitCode::FAILURE;
        };
        let args = cmdline.arguments().into_iter().skip(1).map(|a| a.to_string_lossy().into_owned());
        let cwd = cmdline.cwd().unwrap_or_else(|| PathBuf::from("/"));
        let req = match cli::parse(args, &cwd) {
            Ok(r) => r,
            Err(e) => {
                cmdline.printerr_literal(&format!("wadbrowser: {e}\n{}\n", cli::USAGE));
                return glib::ExitCode::new(2);
            }
        };
        if req.help {
            cmdline.print_literal(&help_text());
            return glib::ExitCode::SUCCESS;
        }
        if req.version {
            cmdline.print_literal(&format!("wadbrowser {APP_VERSION} (Cactus {CACTUS_COMMIT})\n"));
            return glib::ExitCode::SUCCESS;
        }
        let is_first = first.replace(false);
        match carry_out(app, &state, &req, is_first) {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(why) => {
                cmdline.printerr_literal(&format!("wadbrowser: {why}\n"));
                glib::ExitCode::FAILURE
            }
        }
    });

    // Activated on its own (D-Bus Activate): the last window up, or a new one.
    let st = state;
    app.connect_activate(move |app| {
        let Some(state) = st.borrow().clone() else { return };
        if let Some((id, _)) = window::open_windows(app).first() {
            window::present(*id);
        } else {
            let spec = WindowSpec::browser(state.conf.borrow().default_mode, false, None);
            window::build_window_as(app, state, spec, vec![], 0).present();
        }
    });

    Ok(i32::from(app.run()))
}

/// The browser's state: its folders (the instance's), settings, store,
/// blocker and network session. An ephemeral run (`--ephemeral`) keeps
/// nothing: it's settled before the first session is made.
fn start(instance: Option<&str>, ephemeral: bool) -> Result<Rc<AppState>> {
    wadbrowser_webkit::set_ephemeral_run(ephemeral);
    let paths = instance::paths(instance)?;
    let mut prefs = Preferences::load(&paths)?;
    prefs.validate()?;
    let conf = read_conf(&paths);
    prefs.take_conf(&conf.prefs);

    let store = Arc::new(Store::open(&paths)?);
    let privacy = PrivacyEngine::new(prefs.block_ads_trackers);
    privacy.set_cosmetic_enabled(prefs.block_cosmetic_filters);
    if let Ok(list) = store.allowlist_all() {
        privacy.allowlist().replace_all(list);
    }
    if let Err(err) = privacy.load(&paths) {
        warn!("privacy engine load failed: {err}");
    }

    let normal_session = SessionBundle::persistent(&paths)?;
    info!(instance, ephemeral, data = %paths.data_dir.display(), ?conf, "WadBrowser started");
    let state = Rc::new(AppState {
        paths,
        prefs: Rc::new(RefCell::new(prefs)),
        store,
        privacy,
        normal_session,
        conf: RefCell::new(conf),
    });
    follow_conf(&state);
    Ok(state)
}

/// wadbrowser.conf's files: the wadspace's, then the user's (whatever the instance).
fn conf_files(paths: &wadbrowser_core::AppPaths) -> Vec<PathBuf> {
    let user = instance::user_config_dir().unwrap_or_else(|| paths.config_dir.clone());
    conf::files(&user)
}

fn read_conf(paths: &wadbrowser_core::AppPaths) -> Conf {
    Conf::load(&conf_files(paths))
}

thread_local! {
    /// The files' monitors (dropped, they stop).
    static MONITORS: RefCell<Vec<gio::FileMonitor>> = const { RefCell::new(Vec::new()) };
}

/// Each of wadbrowser.conf's files watched (one that isn't there yet too: a
/// user's is made when they first change a setting). A change is read a
/// moment later, once the writer's done, and put into the browser.
fn follow_conf(state: &Rc<AppState>) {
    let pending = Rc::new(Cell::new(false));
    for f in conf_files(&state.paths) {
        let file = gio::File::for_path(&f);
        let monitor = match file.monitor_file(gio::FileMonitorFlags::WATCH_MOVES, None::<&gio::Cancellable>) {
            Ok(m) => m,
            Err(e) => {
                warn!("{}: can't watch it: {e}", f.display());
                continue;
            }
        };
        let (w, pending) = (Rc::downgrade(state), pending.clone());
        monitor.connect_changed(move |_, _, _, _| {
            if pending.replace(true) {
                return;
            }
            let (w, pending) = (w.clone(), pending.clone());
            glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
                pending.set(false);
                if let Some(state) = w.upgrade() {
                    apply_conf(&state, read_conf(&state.paths));
                }
            });
        });
        MONITORS.with_borrow_mut(|m| m.push(monitor));
    }
}

/// New settings, into the browser: the blocker, the search engine and the
/// blocked sites (the engine's filters and the pages they change), and Full
/// or Focus in the browser's own windows.
fn apply_conf(state: &AppState, conf: Conf) {
    if *state.conf.borrow() == conf {
        return;
    }
    info!(?conf, "wadbrowser.conf changed");
    let before = state.conf.replace(conf.clone());
    let blocked_before = state.prefs.borrow().blocked_sites.clone();
    state.prefs.borrow_mut().take_conf(&conf.prefs);
    state.privacy.set_enabled(conf.prefs.ad_blocker);
    wadbrowser_webkit::shield::settings_changed(&blocked_before);
    if before.default_mode != conf.default_mode {
        window::set_default_mode(conf.default_mode);
    }
    window::sync_all();
}

/// Opens what `req` asks for, in a window already open where that fits
/// (`modes::plan`). An error is what to tell the launch.
fn carry_out(app: &adw::Application, state: &Rc<AppState>, req: &Request, first: bool) -> Result<(), String> {
    let open = window::open_windows(app);
    // Full or Focus: wadbrowser.conf's, for every window of the browser's own.
    let default_mode = state.conf.borrow().default_mode;
    let saved = |p: Provider| signin::Saved::load(&state.paths, p);
    let cx = Context {
        open: open.iter().map(|(id, spec)| modes::Open { id: *id, spec }).collect(),
        default_mode,
        first,
        saved_signin: &saved,
    };
    let plan = modes::plan(req, &cx);
    info!(?plan, "launch");
    let prefs = state.prefs.borrow().clone();
    let page = |u: &str| normalize_navigation_input(u, &prefs);

    match plan {
        Plan::Tabs { window, urls } => {
            let urls: Vec<String> = urls.iter().map(|u| page(u)).collect();
            window::add_tabs(window, &urls);
            window::present(window);
        }
        Plan::New { mut spec, urls } => {
            if let Some(a) = &mut spec.app {
                a.start = page(&a.start);
            }
            let urls: Vec<String> = urls.iter().map(|u| page(u)).collect();
            remember_signin(state, &spec, urls.first());
            window::build_window_as(app, state.clone(), spec, urls, 0).present();
        }
        Plan::Restore { spec } => {
            // Its home, unless --restore asks for the last session's tabs.
            let restored = if req.restore == Some(true) { restore_urls(state) } else { None };
            let home = spec.home(NEW_TAB_URI).to_string();
            let (urls, active) = restored.unwrap_or_else(|| (vec![home], 0));
            info!("starting a window with {} tab(s)", urls.len());
            window::build_window_as(app, state.clone(), spec, urls, active).present();
        }
        Plan::SignIn { window, url, code } => {
            let url = page(&url);
            if let Some(spec) = window::reuse_signin(window, &url, code) {
                remember_signin(state, &spec, Some(&url));
            }
            window::present(window);
        }
        Plan::Close { windows } => {
            for w in windows {
                window::close(w);
            }
            if let Some(s) = &req.signin {
                signin::Saved::forget(&state.paths, s.provider);
            }
        }
        Plan::Nothing { why } => return Err(why),
    }
    Ok(())
}

/// A sign-in window's address and code, for `--show` after it's closed.
fn remember_signin(state: &AppState, spec: &WindowSpec, url: Option<&String>) {
    let (Some(s), Some(url)) = (&spec.signin, url) else { return };
    let saved = signin::Saved { url: url.clone(), code: s.code.clone() };
    if let Err(e) = saved.store(&state.paths, s.provider) {
        warn!("keeping the {} sign-in: {e}", s.provider.id());
    }
}
