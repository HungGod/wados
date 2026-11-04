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
use wadbrowser_core::modes::{self, Context, Mode, Plan, WindowSpec};
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
        if is_first && let Some(restore) = req.restore {
            state.prefs.borrow_mut().restore_session = restore;
        }
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
            let spec = WindowSpec::browser(Mode::Full, false, None);
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
    let prefs = Preferences::load(&paths)?;
    prefs.validate()?;

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
    info!(instance, ephemeral, data = %paths.data_dir.display(), "WadBrowser started");
    Ok(Rc::new(AppState { paths, prefs: Rc::new(RefCell::new(prefs)), store, privacy, normal_session }))
}

/// Opens what `req` asks for, in a window already open where that fits
/// (`modes::plan`). An error is what to tell the launch.
fn carry_out(app: &adw::Application, state: &Rc<AppState>, req: &Request, first: bool) -> Result<(), String> {
    let open = window::open_windows(app);
    // What links open in: read when it's asked, so a change counts at once.
    let default_mode = if req.default {
        let user = instance::user_config_dir().unwrap_or_else(|| state.paths.config_dir.clone());
        Conf::load(&conf::files(&user)).default_mode
    } else {
        Mode::Full
    };
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
            let (urls, active) = restore_urls(state).unwrap_or_else(|| (vec![NEW_TAB_URI.into()], 0));
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
