use crate::window::{AppState, build_window, restore_urls};
use anyhow::Result;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tracing::{info, warn};
use wadbrowser_core::{
    APP_ID, AppPaths, LaunchArgs, NEW_TAB_URI, Preferences, Store, help_text, normalize_navigation_input,
};
use wadbrowser_privacy::PrivacyEngine;
use wadbrowser_webkit::SessionBundle;

pub fn run(args: LaunchArgs) -> Result<()> {
    if args.urls.iter().any(|u| u == "__help__") {
        print!("{}", help_text());
        return Ok(());
    }

    if args.verbose {
        // Caller may already have configured env_logger; bump if possible.
        info!("verbose mode enabled");
    }

    let paths = AppPaths::discover()?;
    let mut prefs = Preferences::load(&paths)?;
    if let Some(restore) = args.restore {
        prefs.restore_session = restore;
    }
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

    let state = Rc::new(AppState { paths, prefs: Rc::new(RefCell::new(prefs)), store, privacy, normal_session });

    let app = adw::Application::builder().application_id(APP_ID).build();

    // Dark only.
    app.connect_startup(|_| {
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    });

    let private = args.private;
    let cli_urls = args.urls.clone();

    app.connect_activate(move |app| {
        if let Some(win) = app.active_window() {
            win.present();
            return;
        }

        let prefs_snapshot = state.prefs.borrow().clone();
        let (urls, active) = if !cli_urls.is_empty() {
            let normalized: Vec<String> =
                cli_urls.iter().map(|u| normalize_navigation_input(u, &prefs_snapshot)).collect();
            (normalized, 0usize)
        } else if private {
            (vec![NEW_TAB_URI.into()], 0)
        } else {
            restore_urls(&state).unwrap_or_else(|| (vec![NEW_TAB_URI.into()], 0))
        };

        info!("starting {} window with {} tab(s)", if private { "private" } else { "normal" }, urls.len());
        let window = build_window(app, state.clone(), private, Some(urls), active);
        window.present();
    });

    let status = app.run();
    if status != glib::ExitCode::SUCCESS {
        anyhow::bail!("application exited with {status:?}");
    }
    Ok(())
}
