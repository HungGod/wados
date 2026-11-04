//! The engine's side of a window (wadbrowser-webkit): starting it, what it
//! asks of windows (a popup's tab, a page closing itself, a tab moving into
//! a key's session, a permission prompt, a `cactus://` page's message), the
//! `cactus://` pages, download toasts, and tab sleeping.
//!
//! A child of `window`, so it reaches the window's state; `window.rs`'s
//! `create_tab`, `wire_webview`, `open_in_view`, `render_internal` and
//! `handle_download` call into it.

use super::{WindowCtx, current_webview, show_toast};
use adw::prelude::*;
use adw::{TabPage, TabView};
use chrono::Utc;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;
use tracing::info;
use wadbrowser_core::internal::{self, PageAction, SiteCount};
use wadbrowser_core::profiles::list_profiles;
use wadbrowser_webkit::{
    Ask, DownloadEvent, Hooks, Page, PageRequest, SessionBundle, create_web_view, downloads, filters, navigate,
    profile_for_uri, scheme, shield, sleep, state, testlog,
};
use webkit6::WebView;
use webkit6::prelude::*;

thread_local! {
    static STARTED: Cell<bool> = const { Cell::new(false) };
    /// Each view and the window it's in (a tab moved to another window follows).
    static VIEWS: RefCell<Vec<(glib::WeakRef<WebView>, Weak<WindowCtx>)>> = const { RefCell::new(Vec::new()) };
    /// A view's window-level signal handlers, to take off when it moves to another window.
    static WIRED: RefCell<Vec<(glib::WeakRef<WebView>, Vec<glib::SignalHandlerId>)>> = const { RefCell::new(Vec::new()) };
    /// The windows already set up for sleeping tabs.
    static WINDOWS: RefCell<Vec<Weak<WindowCtx>>> = const { RefCell::new(Vec::new()) };
    /// A popup and its opener (its tab is selected again when the popup closes).
    static OPENERS: RefCell<Vec<(glib::WeakRef<WebView>, glib::WeakRef<WebView>)>> = const { RefCell::new(Vec::new()) };
}

/// The window `view` is in.
pub(super) fn window_of(view: &WebView) -> Option<Rc<WindowCtx>> {
    VIEWS.with_borrow(|all| all.iter().find(|(v, _)| v.upgrade().as_ref() == Some(view)).and_then(|(_, w)| w.upgrade()))
}

fn any_window() -> Option<Rc<WindowCtx>> {
    VIEWS
        .with_borrow(|all| all.iter().find_map(|(_, w)| w.upgrade()))
        .or_else(|| WINDOWS.with_borrow(|all| all.iter().find_map(Weak::upgrade)))
}

fn page_of(ctx: &WindowCtx, view: &WebView) -> Option<TabPage> {
    let pages = ctx.tab_view.pages();
    (0..pages.n_items())
        .filter_map(|i| pages.item(i).and_downcast::<TabPage>())
        .find(|p| p.child().downcast_ref::<WebView>() == Some(view))
}

fn views_of(tab_view: &TabView) -> Vec<WebView> {
    let pages = tab_view.pages();
    (0..pages.n_items())
        .filter_map(|i| pages.item(i).and_downcast::<TabPage>())
        .filter_map(|p| p.child().downcast::<WebView>().ok())
        .collect()
}

/// Starts the engine with the app's state (once), and this window's side of it.
pub(super) fn ensure(ctx: &Rc<WindowCtx>) {
    if !STARTED.replace(true) {
        let s = &ctx.state;
        wadbrowser_webkit::init(s.paths.clone(), s.prefs.clone(), s.store.clone(), s.privacy.clone());
        wadbrowser_webkit::set_hooks(Hooks {
            open_popup: Box::new(open_popup),
            close_view: Box::new(close_view),
            move_view: Box::new(|view, session, uri| {
                if let Some(ctx) = window_of(view) {
                    move_view(&ctx, view, session, uri);
                }
            }),
            page_action: Box::new(page_action),
            ask_permission: Box::new(ask_permission),
        });
        wadbrowser_webkit::set_page_source(page_source);
        downloads::connect(download_toast);
        glib::timeout_add_seconds_local(60, || {
            sleep_tabs();
            glib::ControlFlow::Continue
        });
    }
    let seen = WINDOWS.with_borrow(|all| all.iter().any(|w| w.upgrade().is_some_and(|w| Rc::ptr_eq(&w, ctx))));
    if seen {
        return;
    }
    WINDOWS.with_borrow_mut(|all| {
        all.retain(|w| w.upgrade().is_some());
        all.push(Rc::downgrade(ctx));
    });
    // A tab shown wakes (if it slept), and counts as looked at.
    let weak = Rc::downgrade(ctx);
    ctx.tab_view.connect_selected_page_notify(move |_| {
        if let Some(ctx) = weak.upgrade()
            && let Some(v) = current_webview(&ctx)
        {
            sleep::wake(&v);
        }
    });
    if testlog::on() {
        ctx.tab_view.connect_n_pages_notify(|tv| testlog::line("tabs", &[&tv.n_pages().to_string()]));
    }
    // A private window's session goes with it.
    let profile = ctx.session.profile.clone();
    let private = ctx.session.private;
    ctx.window.connect_destroy(move |_| {
        if private {
            wadbrowser_webkit::forget(&profile);
        }
    });
}

/// The session a tab at `uri` opens in: the window's, or a key's for its site.
pub(super) fn session_for_uri(ctx: &WindowCtx, uri: &str) -> SessionBundle {
    match profile_for_uri(&ctx.session.profile, uri) {
        Some(target) => SessionBundle::for_profile(&ctx.state.paths, &target),
        None => ctx.session.clone(),
    }
}

/// A new tab's view, in `session`.
pub(super) fn new_view(ctx: &WindowCtx, session: &SessionBundle) -> WebView {
    let prefs = ctx.state.prefs.borrow().clone();
    let view = create_web_view(session, &prefs);
    testlog::line("view", &[&session.profile]);
    view
}

/// `uri` in `view`, as the person asked: WadBrowser's own pages from the
/// scheme; a key's site in the key's session (a new view in the tab's
/// place); anything else once the content filter's in (or soon after).
pub(super) fn open(ctx: &Rc<WindowCtx>, view: &WebView, uri: &str) {
    let uri = internal::map_about(uri).map(str::to_string).unwrap_or_else(|| uri.to_string());
    if !ctx.url_entry.has_focus() || uri.starts_with("cactus://") {
        ctx.url_entry.set_text(&uri);
    }
    if internal::is_internal(&uri) {
        navigate(view, &uri);
        return;
    }
    if let Some(st) = state(view)
        && let Some(target) = profile_for_uri(&st.profile, &uri)
    {
        let session = SessionBundle::for_profile(&ctx.state.paths, &target);
        testlog::line("moved", &[&st.profile, &target, &uri]);
        move_view(ctx, view, &session, &uri);
        return;
    }
    let v = view.clone();
    filters::when_ready(move || navigate(&v, &uri));
}

/// A view joining window `ctx` (a new tab, a popup, a tab dragged in): its
/// window-level signals, the old window's taken off.
pub(super) fn wired(ctx: &Rc<WindowCtx>, view: &WebView, handlers: Vec<glib::SignalHandlerId>) {
    ensure(ctx);
    let old = WIRED.with_borrow_mut(|all| {
        all.retain(|(v, _)| v.upgrade().is_some());
        let i = all.iter().position(|(v, _)| v.upgrade().as_ref() == Some(view));
        let old = i.map(|i| all.remove(i).1);
        all.push((view.downgrade(), handlers));
        old
    });
    for id in old.into_iter().flatten() {
        view.disconnect(id);
    }
    VIEWS.with_borrow_mut(|all| {
        all.retain(|(v, w)| v.upgrade().is_some_and(|v| &v != view) && w.upgrade().is_some());
        all.push((view.downgrade(), Rc::downgrade(ctx)));
    });
}

// ------------------------------------------------------------ the hooks

/// A popup: a tab right after its opener's, selected.
fn open_popup(opener: &WebView, popup: &WebView) {
    let Some(ctx) = window_of(opener).or_else(any_window) else { return };
    let at = page_of(&ctx, opener).map(|p| ctx.tab_view.page_position(&p) + 1).unwrap_or(ctx.tab_view.n_pages());
    OPENERS.with_borrow_mut(|all| {
        all.retain(|(p, o)| p.upgrade().is_some() && o.upgrade().is_some());
        all.push((popup.downgrade(), opener.downgrade()));
    });
    let page = ctx.tab_view.insert(popup, at);
    page.set_title(&popup.title().map(String::from).unwrap_or_else(|| "New Tab".into()));
    ctx.tab_view.set_selected_page(&page);
    testlog::line("tab-open", &[&state(popup).map(|s| s.profile.clone()).unwrap_or_default(), "popup"]);
}

/// The page closed itself: its tab goes (its opener's tab is shown again).
fn close_view(view: &WebView) {
    let Some(ctx) = window_of(view) else { return };
    let Some(page) = page_of(&ctx, view) else { return };
    let opener = OPENERS
        .with_borrow(|all| all.iter().find(|(p, _)| p.upgrade().as_ref() == Some(view)).and_then(|(_, o)| o.upgrade()));
    let selected = ctx.tab_view.selected_page().as_ref() == Some(&page);
    ctx.tab_view.close_page(&page);
    if selected
        && let Some(o) = opener
        && let Some(op) = page_of(&ctx, &o)
    {
        ctx.tab_view.set_selected_page(&op);
    }
    testlog::line("tab-closed", &[&state(view).map(|s| s.profile.clone()).unwrap_or_default()]);
}

/// A tab into another session (a key's): a new view in its place, loading `uri`.
fn move_view(ctx: &Rc<WindowCtx>, view: &WebView, session: &SessionBundle, uri: &str) {
    let fresh = new_view(ctx, session);
    let old = page_of(ctx, view);
    let at = old.as_ref().map(|p| ctx.tab_view.page_position(p) + 1).unwrap_or(ctx.tab_view.n_pages());
    let page = ctx.tab_view.insert(&fresh, at);
    page.set_title(&old.as_ref().map(|p| p.title().to_string()).unwrap_or_else(|| "New Tab".into()));
    if old.as_ref().is_none_or(|p| ctx.tab_view.selected_page().as_ref() == Some(p)) {
        ctx.tab_view.set_selected_page(&page);
    }
    if let Some(p) = old {
        ctx.tab_view.close_page(&p);
    }
    info!(profile = %session.profile, %uri, "a tab moved into a key's profile");
    let uri = uri.to_string();
    filters::when_ready(move || navigate(&fresh, &uri));
}

/// What a `cactus://` page asked for.
fn page_action(view: &WebView, json: &str) {
    let uri = view.uri().map(String::from).unwrap_or_default();
    let Some(action) = PageAction::from_page(&uri, json) else {
        tracing::debug!(%uri, json, "a page message that isn't one of ours");
        return;
    };
    info!(?action, %uri, "a page asks");
    match action {
        PageAction::Go { to } => {
            let Some(ctx) = window_of(view) else { return };
            let prefs = ctx.state.prefs.borrow().clone();
            let target = wadbrowser_core::normalize_navigation_input(&to, &prefs);
            open(&ctx, view, &target);
        }
        PageAction::BlockSocial { on } => shield::set_social_blocking(on),
        PageAction::Blockers { site, on } => {
            shield::set_site_allowed(&site, !on);
            // The blocked page's "let it through": back to the page it stopped.
            if !on
                && uri.starts_with("cactus://blocked")
                && let Some(back) = scheme::query_value(&uri, "uri")
                && let Some(ctx) = window_of(view)
            {
                open(&ctx, view, &back);
            } else if uri.starts_with("cactus://privacy") {
                view.reload();
            }
        }
        PageAction::SearchEngine { id } => {
            let Some(ctx) = window_of(view) else { return };
            if let Some(engine) = wadbrowser_core::search::engine_from_id(&id) {
                ctx.state.prefs.borrow_mut().search_engine = engine;
                let _ = ctx.state.prefs.borrow().save(&ctx.state.paths);
            }
        }
        // The chrome's (wire.rs): the New profile dialog, a web app's window, one of yours removed.
        PageAction::NewProfile | PageAction::OpenApp { .. } | PageAction::RemoveApp { .. } => {
            if let Some(ctx) = window_of(view) {
                super::wire::page_action(&ctx, &action);
            }
        }
    }
}

/// A site asks for the camera, the microphone, notifications…: the
/// chrome's dialog (wire.rs).
fn ask_permission(view: &WebView, ask: Ask) {
    let parent = window_of(view).map(|c| c.window.clone());
    super::wire::ask_permission(parent.as_ref(), ask);
}

fn download_toast(view: Option<&WebView>, ev: &DownloadEvent) {
    let Some(ctx) = view.and_then(window_of).or_else(any_window) else { return };
    let msg = match ev {
        DownloadEvent::Started { name, .. } => format!("Downloading {name}"),
        DownloadEvent::Finished { name, .. } => format!("Downloaded {name}"),
        DownloadEvent::Failed { cancelled: true, .. } => return,
        DownloadEvent::Failed { name, error, .. } => format!("{name} didn't download: {error}"),
    };
    show_toast(&ctx, &msg);
}

/// Background tabs not looked at for a while sleep (the settings' minutes).
fn sleep_tabs() {
    let windows: Vec<Rc<WindowCtx>> = WINDOWS.with_borrow(|all| all.iter().filter_map(Weak::upgrade).collect());
    for ctx in windows {
        let minutes = ctx.state.prefs.borrow().sleep_tabs_after_minutes;
        let shown = current_webview(&ctx);
        if let Some(v) = &shown {
            sleep::touch(v);
        }
        if minutes == 0 {
            continue;
        }
        for v in views_of(&ctx.tab_view) {
            if Some(&v) != shown.as_ref() && sleep::sleepy(&v, Duration::from_secs(minutes * 60)) {
                sleep::sleep(&v);
            }
        }
    }
}

// ------------------------------------------------------------ the pages

/// `cactus://` pages, for the scheme: drawn for the view's window.
fn page_source(req: &PageRequest) -> Option<Page> {
    let ctx = req.view.and_then(window_of).or_else(any_window)?;
    Some(Page::html(render(&ctx, req.uri)))
}

/// A `cactus://` page's HTML (window.rs's `render_internal`).
pub(super) fn render(ctx: &Rc<WindowCtx>, uri: &str) -> String {
    let prefs = ctx.state.prefs.borrow().clone();
    let page = uri.strip_prefix("cactus://").unwrap_or(uri);
    let name = page.split(['?', '#', '/']).next().unwrap_or_default();
    let q = |k: &str| scheme::query_value(uri, k).unwrap_or_default();
    let private = ctx.private;
    match name {
        "newtab" | "home" => {
            // The catalog's web apps and the machine's (or the wadspace's): wire.rs's.
            internal::newtab_page(&prefs, &super::wire::newtab_tiles())
        }
        "privacy" | "shield" => internal::privacy_page(&privacy(ctx)),
        "settings" | "preferences" => {
            let profiles = list_profiles(&ctx.state.paths)
                .into_iter()
                .map(|p| internal::ProfileRow { id: p.id, name: p.name, color: p.color })
                .collect();
            internal::settings_page(
                &prefs,
                &internal::Settings {
                    block_social: prefs.block_social,
                    profiles,
                    apps: super::wire::settings_apps(),
                    webkit_version: wadbrowser_webkit::webkit_version_string(),
                },
            )
        }
        "history" => {
            let entries = if private { vec![] } else { ctx.state.store.list_history(200).unwrap_or_default() };
            internal::history_page(&entries, private, Utc::now())
        }
        "bookmarks" => internal::bookmarks_html(&ctx.state.store.list_bookmarks().unwrap_or_default()),
        "downloads" => internal::downloads_html(&ctx.state.store.list_downloads(100).unwrap_or_default()),
        "about" | "landing" | "project" => internal::about_html(&wadbrowser_webkit::webkit_version_string()),
        "error" => internal::error_html(&q("title"), &q("message"), &q("uri")),
        "blocked" => internal::blocked_html(&q("site")),
        _ => internal::missing_html(uri),
    }
}

/// What `cactus://privacy` shows for this window.
fn privacy(ctx: &WindowCtx) -> internal::Privacy {
    let prefs = ctx.state.prefs.borrow();
    let mut sites: Vec<SiteCount> = vec![];
    let mut here = 0;
    for v in views_of(&ctx.tab_view) {
        let uri = v.uri().map(String::from).unwrap_or_default();
        let n = shield::page_blocked(&v);
        here += n;
        let Some(site) = wadbrowser_privacy::site_key(&uri).filter(|_| uri.starts_with("http")) else { continue };
        match sites.iter_mut().find(|s| s.site == site) {
            Some(s) => {
                s.pages += 1;
                s.blocked += n;
            }
            None => sites.push(SiteCount { site, url: uri, pages: 1, blocked: n }),
        }
    }
    sites.sort_by_key(|s| std::cmp::Reverse(s.blocked));
    internal::Privacy {
        blocked: shield::total_blocked(),
        blocked_here: here,
        private: ctx.private,
        block_social: prefs.block_social,
        block_ads: prefs.block_ads_trackers,
        sites: if ctx.private { vec![] } else { sites },
        allowed: ctx.state.privacy.allowlist().list(),
    }
}
