//! A WadBrowser window, as the v2 design draws it (the spike's
//! `Browser.svelte`): the tab bar, then the header bar (back, forward,
//! reload; the address, the page's title or an app's offsite pill; the
//! shield, the profile chip and ⋮), then the page. Full, Focus and App are
//! its kinds (wadbrowser_core::modes); a private window is tinted magenta.
//!
//! The drawing is `chrome.rs`'s, the menus, shield, chip and dialogs
//! `wire.rs`'s, and the engine's side of it `views.rs`'s.

// The engine's side of a window (window/views.rs).
mod views;
// The tab bar and the header bar, drawn.
mod chrome;
// What the menus, the shield, the chip and the dialogs do.
mod wire;

use adw::prelude::*;
use adw::{ApplicationWindow, TabPage, TabView, Toast, ToastOverlay};
use anyhow::Result;
use glib::clone;
use gtk::gdk::{Key, ModifierType};
use gtk::{Box as GtkBox, Entry, EventControllerKey, Label, Orientation, Revealer, SearchEntry};
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{info, warn};
use wadbrowser_core::modes::{Mode, WindowSpec};
use wadbrowser_core::signin::{self, Provider};
use wadbrowser_core::site;
use wadbrowser_core::store::{SessionState, SessionWindow};
use wadbrowser_core::{AppPaths, NEW_TAB_URI, Preferences, Store, normalize_navigation_input};
use wadbrowser_privacy::PrivacyEngine;
use wadbrowser_webkit::SessionBundle;
use webkit6::prelude::*;
use webkit6::{
    FindOptions, LoadEvent, UserContentInjectedFrames, UserContentManager, UserScript, UserScriptInjectionTime, WebView,
};

use crate::chip::ChipInfo;
use crate::shield::ShieldActions;

pub struct AppState {
    pub paths: AppPaths,
    pub prefs: Rc<RefCell<Preferences>>,
    pub store: Arc<Store>,
    pub privacy: Rc<PrivacyEngine>,
    pub normal_session: SessionBundle,
}

struct WindowCtx {
    window: ApplicationWindow,
    app: adw::Application,
    state: Rc<AppState>,
    session: SessionBundle,
    private: bool,
    tab_view: TabView,
    /// The address field's entry (while it's being edited).
    url_entry: Entry,
    toast: ToastOverlay,
    find_revealer: Revealer,
    find_entry: SearchEntry,
    tabs: Rc<chrome::TabStrip>,
    header: chrome::Header,
    /// Its kind (wadbrowser_core::modes), and the chrome that shows it.
    id: u64,
    spec: RefCell<WindowSpec>,
    chrome: ModeChrome,
    /// Where a page was last right-clicked (its view's pixels): the page menu's place.
    clicked_at: Cell<Option<(f64, f64)>>,
    /// Its first page has loaded (screenshots' hook: wire::shot).
    shot: Cell<bool>,
}

/// A window of kind `spec` (modes.rs: Full, Focus, a web app's, a
/// sign-in's), its tabs `urls` (none: its home: an app's start page, or a
/// new tab) with the `active_tab`th selected.
pub(crate) fn build_window_as(
    app: &adw::Application,
    state: Rc<AppState>,
    spec: WindowSpec,
    urls: Vec<String>,
    active_tab: usize,
) -> ApplicationWindow {
    if let Some(display) = gtk::gdk::Display::default() {
        crate::style::install_once(&display);
    }
    let private = spec.private;
    let session = session_for(&state, &spec);

    let window = ApplicationWindow::builder()
        .application(app)
        .default_width(1200)
        .default_height(800)
        .title(spec.title(None))
        .css_classes(["wb-window"])
        .build();
    if private {
        window.add_css_class("wb-private");
    }
    own_app_id(&window, spec.app_id());

    // What the chrome's buttons do needs the window's state, made after them.
    let me: Rc<RefCell<Weak<WindowCtx>>> = Rc::default();
    let with = |f: fn(&Rc<WindowCtx>)| {
        let me = me.clone();
        move || {
            if let Some(ctx) = me.borrow().upgrade() {
                f(&ctx);
            }
        }
    };

    let tab_view = TabView::new();
    let tabs = chrome::TabStrip::new(&tab_view, Rc::new(with(|ctx| new_tab(ctx, NEW_TAB_URI))));
    let chrome = ModeChrome::new(&spec);
    let shield_actions = {
        let (a, b, c) = (me.clone(), me.clone(), me.clone());
        ShieldActions {
            set_blockers: Rc::new(move |on| {
                if let Some(ctx) = a.borrow().upgrade() {
                    wire::set_blockers(&ctx, on);
                }
            }),
            set_social: Rc::new(move |on| {
                if b.borrow().upgrade().is_some() {
                    wadbrowser_webkit::shield::set_social_blocking(on);
                }
            }),
            open_dashboard: Rc::new(move || {
                if let Some(ctx) = c.borrow().upgrade() {
                    open_page(&ctx, "wadbrowser://privacy");
                }
            }),
        }
    };
    let chip_me = me.clone();
    let header = chrome::Header::new(
        &chrome.identity,
        &chrome.page_title,
        &chrome.offsite,
        shield_actions,
        ChipInfo { name: String::new(), color: String::new(), private },
        move |b| {
            if let Some(ctx) = chip_me.borrow().upgrade() {
                wire::profile_menu(&ctx, b, true);
            }
        },
    );

    let find_box = GtkBox::new(Orientation::Horizontal, 8);
    find_box.add_css_class("find-bar");
    let find_entry = SearchEntry::new();
    find_entry.set_hexpand(true);
    find_entry.set_placeholder_text(Some("Find in page"));
    let find_close = chrome::icon_button("x", 15, "Close find");
    find_box.append(&find_entry);
    find_box.append(&find_close);
    let find_revealer = Revealer::new();
    find_revealer.set_child(Some(&find_box));
    find_revealer.set_reveal_child(false);

    let root = GtkBox::new(Orientation::Vertical, 0);
    root.append(&tabs.root);
    root.append(&header.root);
    tab_view.add_css_class("wb-page");
    root.append(&tab_view);
    root.append(&find_revealer);
    let toast = ToastOverlay::new();
    toast.set_child(Some(&root));
    window.set_content(Some(&toast));

    let ctx = Rc::new(WindowCtx {
        window: window.clone(),
        app: app.clone(),
        state: state.clone(),
        session,
        private,
        tab_view: tab_view.clone(),
        url_entry: header.address.entry.clone(),
        toast: toast.clone(),
        find_revealer: find_revealer.clone(),
        find_entry: find_entry.clone(),
        tabs,
        header,
        id: NEXT_WINDOW.fetch_add(1, Ordering::Relaxed),
        spec: RefCell::new(spec),
        chrome,
        clicked_at: Cell::new(None),
        shot: Cell::new(false),
    });
    *me.borrow_mut() = Rc::downgrade(&ctx);
    register(&ctx);

    let h = &ctx.header;
    h.back.connect_clicked(clone!(
        #[weak]
        ctx,
        move |_| {
            if let Some(view) = current_webview(&ctx)
                && view.can_go_back()
            {
                view.go_back();
            }
        }
    ));
    h.forward.connect_clicked(clone!(
        #[weak]
        ctx,
        move |_| {
            if let Some(view) = current_webview(&ctx)
                && view.can_go_forward()
            {
                view.go_forward();
            }
        }
    ));
    h.reload.connect_clicked(clone!(
        #[weak]
        ctx,
        move |_| {
            if let Some(view) = current_webview(&ctx) {
                if view.is_loading() {
                    view.stop_loading();
                } else {
                    view.reload();
                }
            }
        }
    ));
    h.menu.connect_clicked(clone!(
        #[weak]
        ctx,
        move |b| drop(wire::main_menu(&ctx, b, true))
    ));
    ctx.chrome.back_to.connect_clicked(clone!(
        #[weak]
        ctx,
        move |_| {
            let start = ctx.spec.borrow().home(NEW_TAB_URI).to_string();
            open_uri(&ctx, &start, false);
        }
    ));

    // The address: shown until it's clicked, then typed in; Enter goes, Esc doesn't.
    h.address.connect_clicked(clone!(
        #[weak]
        ctx,
        move || edit_address(&ctx)
    ));
    h.address.entry.connect_activate(clone!(
        #[weak]
        ctx,
        move |entry| {
            let text = entry.text().to_string();
            ctx.header.address.done();
            if text.trim().is_empty() {
                return;
            }
            let prefs = ctx.state.prefs.borrow().clone();
            let uri = normalize_navigation_input(&text, &prefs);
            open_uri(&ctx, &uri, false);
            if let Some(v) = current_webview(&ctx) {
                v.grab_focus();
            }
        }
    ));
    let esc = EventControllerKey::new();
    esc.connect_key_pressed(clone!(
        #[weak]
        ctx,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| {
            if key != Key::Escape {
                return glib::Propagation::Proceed;
            }
            ctx.header.address.done();
            sync_chrome(&ctx);
            if let Some(v) = current_webview(&ctx) {
                v.grab_focus();
            }
            glib::Propagation::Stop
        }
    ));
    h.address.entry.add_controller(esc);
    let leave = gtk::EventControllerFocus::new();
    leave.connect_leave(clone!(
        #[weak]
        ctx,
        move |_| {
            ctx.header.address.done();
            sync_chrome(&ctx);
        }
    ));
    h.address.entry.add_controller(leave);

    tab_view.connect_page_attached(clone!(
        #[weak]
        ctx,
        move |_, page, _| {
            if let Ok(child) = page.child().downcast::<WebView>() {
                wire_webview(&ctx, &child, page);
                attach_mode(&ctx, &child);
            }
            ctx.tabs.sync(&ctx.tab_view, ctx.spec.borrow().autohide_tabs());
            sync_controls(&ctx);
        }
    ));
    tab_view.connect_page_detached(clone!(
        #[weak]
        ctx,
        move |_, _, _| {
            ctx.tabs.sync(&ctx.tab_view, ctx.spec.borrow().autohide_tabs());
            sync_controls(&ctx);
        }
    ));
    tab_view.connect_selected_page_notify(clone!(
        #[weak]
        ctx,
        move |_| {
            ctx.header.address.done();
            sync_chrome(&ctx);
            sync_mode(&ctx);
            wire::sync_shield(&ctx);
        }
    ));
    tab_view.connect_close_page(clone!(
        #[weak]
        ctx,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |view, page| {
            view.close_page_finish(page, true);
            // The last tab: the window goes with it (Cactus's bug 13: Ctrl+W kept it).
            if view.n_pages() == 0 {
                let w = ctx.window.clone();
                glib::idle_add_local_once(move || w.close());
            }
            glib::Propagation::Stop
        }
    ));

    find_close.connect_clicked(clone!(
        #[weak]
        ctx,
        move |_| {
            ctx.find_revealer.set_reveal_child(false);
            if let Some(view) = current_webview(&ctx)
                && let Some(finder) = view.find_controller()
            {
                finder.search_finish();
            }
        }
    ));
    find_entry.connect_search_changed(clone!(
        #[weak]
        ctx,
        move |entry| {
            if let Some(view) = current_webview(&ctx)
                && let Some(finder) = view.find_controller()
            {
                let text = entry.text();
                if text.is_empty() {
                    finder.search_finish();
                } else {
                    let opts = (FindOptions::CASE_INSENSITIVE | FindOptions::WRAP_AROUND).bits();
                    finder.search(text.as_str(), opts, 128);
                }
            }
        }
    ));

    setup_actions(&ctx);
    setup_shortcuts(&ctx);
    wire::setup(&ctx);
    // Downloads are each network session's (wadbrowser_webkit::downloads,
    // toasted in their window by views.rs), not the window's.

    if urls.is_empty() {
        let home = ctx.spec.borrow().home(NEW_TAB_URI).to_string();
        let _ = create_tab(&ctx, &home, true);
    } else {
        for (i, url) in urls.iter().enumerate() {
            let _ = create_tab(&ctx, url, i == 0);
        }
        if active_tab > 0 {
            select_tab_index(&ctx.tab_view, active_tab);
        }
    }
    show_mode(&ctx);

    window.connect_close_request(clone!(
        #[strong]
        ctx,
        move |_| {
            // The session is the browser's own windows', not an app's or a
            // sign-in's; an ephemeral run keeps none.
            if ctx.spec.borrow().saved_in_session() && !wadbrowser_webkit::ephemeral_run() {
                save_session_from(&ctx);
            }
            glib::Propagation::Proceed
        }
    ));
    window.connect_destroy(clone!(
        #[strong]
        ctx,
        move |_| {
            remove_scripts(&ctx);
            unregister(ctx.id);
        }
    ));

    window
}

fn create_tab(ctx: &Rc<WindowCtx>, uri: &str, select: bool) -> Result<TabPage> {
    views::ensure(ctx);
    // Its profile's session: the window's, or a key's for its site.
    let session = views::session_for_uri(ctx, uri);
    let view = views::new_view(ctx, &session);
    let page = ctx.tab_view.append(&view);
    page.set_title("New tab");
    if select {
        ctx.tab_view.set_selected_page(&page);
    }
    open_in_view(ctx, &view, uri);
    Ok(page)
}

/// A new tab at `uri`, shown (a new tab's own search box takes the keys).
fn new_tab(ctx: &Rc<WindowCtx>, uri: &str) {
    let _ = create_tab(ctx, uri, true);
}

/// One of the browser's pages: in an app's window (on its site), a tab of
/// its own, so the app stays; else in this tab.
fn open_page(ctx: &Rc<WindowCtx>, uri: &str) {
    let spec = ctx.spec.borrow().clone();
    let here = current_webview(ctx).and_then(|v| v.uri()).map(String::from).unwrap_or_default();
    let new_tab = spec.mode == Mode::App && !spec.offsite(&here);
    open_uri(ctx, uri, new_tab);
}

/// Developer tools (Inspect), with `WADBROWSER_DEVTOOLS=1`; off otherwise.
fn devtools() -> bool {
    std::env::var_os("WADBROWSER_DEVTOOLS").is_some_and(|v| !v.is_empty() && v != "0")
}

fn wire_webview(ctx: &Rc<WindowCtx>, view: &WebView, page: &TabPage) {
    if let Some(s) = WebViewExt::settings(view) {
        s.set_enable_developer_extras(devtools());
    }
    // Favicons for the tabs (each profile's data manager keeps them).
    if let Some(m) = view.network_session().and_then(|n| n.website_data_manager()) {
        m.set_favicons_enabled(true);
    }
    // Weak: the window holds its tabs, and a tab mustn't hold its window.
    let mut ids = vec![];
    ids.push(view.connect_notify_local(
        Some("title"),
        clone!(
            #[weak]
            page,
            #[weak]
            ctx,
            move |view, _| {
                let title = view.title().map(String::from).filter(|t| !t.trim().is_empty());
                let title = title.unwrap_or_else(|| tab_title_for(&view.uri().unwrap_or_default()));
                page.set_title(&title);
                if is_selected(&ctx, &page) {
                    sync_chrome(&ctx);
                }
            }
        ),
    ));
    ids.push(view.connect_notify_local(
        Some("uri"),
        clone!(
            #[weak]
            page,
            #[weak]
            ctx,
            move |_, _| {
                if is_selected(&ctx, &page) {
                    sync_chrome(&ctx);
                    wire::sync_shield(&ctx);
                }
            }
        ),
    ));
    ids.push(view.connect_notify_local(
        Some("is-loading"),
        clone!(
            #[weak]
            page,
            #[weak]
            ctx,
            move |view, _| {
                page.set_loading(view.is_loading());
                if is_selected(&ctx, &page) {
                    sync_loading(&ctx, view);
                }
            }
        ),
    ));
    ids.push(view.connect_notify_local(
        Some("estimated-load-progress"),
        clone!(
            #[weak]
            page,
            #[weak]
            ctx,
            move |view, _| {
                if is_selected(&ctx, &page) {
                    sync_loading(&ctx, view);
                }
            }
        ),
    ));
    ids.push(view.connect_load_changed(clone!(
        #[weak]
        ctx,
        #[weak]
        page,
        move |view, event| {
            if event != LoadEvent::Finished {
                return;
            }
            if is_selected(&ctx, &page) {
                sync_chrome(&ctx);
                // Screenshots' hook: once, after the window's first page.
                if !ctx.shot.replace(true) {
                    let w = Rc::downgrade(&ctx);
                    glib::timeout_add_local_once(std::time::Duration::from_millis(900), move || {
                        if let Some(ctx) = w.upgrade() {
                            wire::shot(&ctx);
                        }
                    });
                }
            }
            let keeps = !ctx.private && wadbrowser_webkit::state(view).is_some_and(|s| !s.private);
            if keeps {
                let title = view.title().unwrap_or_default().to_string();
                // Never a fragment or a token in history: a stream's one-tap
                // link carries its key there (`/ws-<guid>/#token=…`).
                let uri = wadbrowser_core::internal::shown_uri(&view.uri().unwrap_or_default());
                if let Err(err) = ctx.state.store.record_history(&title, &uri) {
                    warn!("history: {err}");
                }
                wadbrowser_core::metrics::record_page_load(&ctx.state.store);
            }
        }
    )));
    // Right-click: the page menu, but on a link, a picture, a video, words
    // picked out or a field, WebKit's own (open in a new tab, copy, paste…).
    let right = gtk::GestureClick::builder().button(3).build();
    right.set_propagation_phase(gtk::PropagationPhase::Capture);
    right.connect_pressed(clone!(
        #[weak]
        ctx,
        move |_, _, x, y| ctx.clicked_at.set(Some((x, y)))
    ));
    view.add_controller(right);
    ids.push(view.connect_context_menu(clone!(
        #[weak]
        ctx,
        #[upgrade_or]
        false,
        move |view, _menu, hit| {
            let webkits = hit.context_is_link()
                || hit.context_is_image()
                || hit.context_is_media()
                || hit.context_is_selection()
                || hit.context_is_editable()
                || hit.context_is_scrollbar();
            if webkits {
                return false;
            }
            let at = ctx.clicked_at.take().unwrap_or_else(|| (view.width() as f64 / 2.0, view.height() as f64 / 3.0));
            wire::page_menu(&ctx, view, at, true);
            true
        }
    )));
    // Error pages, popups, the filters and the rest are the engine's
    // (wadbrowser-webkit's view.rs), wired as the view was made.
    views::wired(ctx, view, ids);
}

/// A tab's title before its page has one.
fn tab_title_for(uri: &str) -> String {
    match uri.strip_prefix("wadbrowser://").map(|p| p.split(['/', '?', '#']).next().unwrap_or("")) {
        Some("newtab") | Some("home") | Some("") => "New tab".into(),
        Some("privacy") => "Privacy".into(),
        Some("settings") => "Settings".into(),
        Some("history") => "History".into(),
        Some(other) => other.to_string(),
        None => site::bare_host(&site::host_of(uri)),
    }
}

fn is_selected(ctx: &WindowCtx, page: &TabPage) -> bool {
    ctx.tab_view.selected_page().as_ref().map(|p| p == page).unwrap_or(false)
}

fn open_uri(ctx: &Rc<WindowCtx>, uri: &str, new_tab: bool) {
    if new_tab {
        let _ = create_tab(ctx, uri, true);
        return;
    }
    if let Some(view) = current_webview(ctx) {
        open_in_view(ctx, &view, uri);
    } else {
        let _ = create_tab(ctx, uri, true);
    }
}

fn open_in_view(ctx: &Rc<WindowCtx>, view: &WebView, uri: &str) {
    views::open(ctx, view, uri);
}

/// A `wadbrowser://` page's HTML (the scheme's callback: views.rs's `page_source`).
#[allow(dead_code)]
fn render_internal(ctx: &Rc<WindowCtx>, uri: &str) -> String {
    views::render(ctx, uri)
}

/// GDK's numbers for a mouse's side buttons.
const MOUSE_BACK: u32 = 8;
const MOUSE_FORWARD: u32 = 9;

fn current_webview(ctx: &WindowCtx) -> Option<WebView> {
    ctx.tab_view.selected_page().and_then(|p| p.child().downcast::<WebView>().ok())
}

/// The address being typed: what's there (nothing on a new tab), selected.
fn edit_address(ctx: &Rc<WindowCtx>) {
    if !ctx.spec.borrow().has_urlbar() {
        return;
    }
    let uri = current_webview(ctx).and_then(|v| v.uri()).map(String::from).unwrap_or_default();
    let text = if is_newtab(&uri) { "" } else { uri.as_str() };
    ctx.header.address.edit(text);
}

fn is_newtab(uri: &str) -> bool {
    uri.is_empty()
        || uri.starts_with("about:blank")
        || uri.starts_with(NEW_TAB_URI)
        || uri.starts_with("wadbrowser://home")
}

/// The header bar for the page shown: its address, back and forward.
fn sync_chrome(ctx: &WindowCtx) {
    let Some(view) = current_webview(ctx) else {
        return;
    };
    let uri = view.uri().map(String::from).unwrap_or_default();
    if !ctx.header.address.editing() {
        let engine = wadbrowser_core::internal::engine_name(&ctx.state.prefs.borrow().search_engine).to_string();
        ctx.header.address.show(&uri, is_newtab(&uri), &format!("Search {engine} or type an address"));
    }
    ctx.header.back.set_sensitive(view.can_go_back());
    ctx.header.forward.set_sensitive(view.can_go_forward());
    sync_loading(ctx, &view);
}

/// The loading line under the header bar.
fn sync_loading(ctx: &WindowCtx, view: &WebView) {
    let line = &ctx.header.loading;
    if view.is_loading() {
        line.set_visible(true);
        line.set_fraction(view.estimated_load_progress().max(0.3));
    } else {
        line.set_visible(false);
        line.set_fraction(0.0);
    }
}

fn show_toast(ctx: &WindowCtx, msg: &str) {
    ctx.toast.add_toast(Toast::new(msg));
}

/// The window's actions (`win.…`), for its keyboard and its menus.
fn setup_actions(ctx: &Rc<WindowCtx>) {
    let win = &ctx.window;
    let page = |name: &'static str, uri: &'static str| {
        let ctx = Rc::downgrade(ctx);
        add_action(win, name, move |_, _| {
            if let Some(ctx) = ctx.upgrade() {
                open_page(&ctx, uri);
            }
        });
    };
    page("history", "wadbrowser://history");
    page("privacy", "wadbrowser://privacy");
    page("settings", "wadbrowser://settings");
    page("downloads", "wadbrowser://downloads");
    page("about", "wadbrowser://about");
    let zoom = |name: &'static str, f: fn(&WebView)| {
        let ctx = Rc::downgrade(ctx);
        add_action(win, name, move |_, _| {
            if let Some(ctx) = ctx.upgrade()
                && let Some(v) = current_webview(&ctx)
            {
                f(&v);
            }
        });
    };
    zoom("zoom-in", wadbrowser_webkit::zoom::zoom_in);
    zoom("zoom-out", wadbrowser_webkit::zoom::zoom_out);
    zoom("zoom-reset", wadbrowser_webkit::zoom::zoom_reset);
}

fn add_action<F>(win: &ApplicationWindow, name: &str, f: F)
where
    F: Fn(&gio::SimpleAction, Option<&glib::Variant>) + 'static,
{
    let action = gio::SimpleAction::new(name, None);
    action.connect_activate(f);
    win.add_action(&action);
}

fn setup_shortcuts(ctx: &Rc<WindowCtx>) {
    let controller = EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    controller.connect_key_pressed(clone!(
        #[weak]
        ctx,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _code, mods| {
            let ctrl = mods.contains(ModifierType::CONTROL_MASK);
            let shift = mods.contains(ModifierType::SHIFT_MASK);
            let alt = mods.contains(ModifierType::ALT_MASK);
            let signing_in = ctx.spec.borrow().signin.is_some();

            if ctrl && !shift && key == Key::t && !signing_in {
                new_tab(&ctx, NEW_TAB_URI);
                return glib::Propagation::Stop;
            }
            if ctrl && !shift && key == Key::w {
                // The last tab's close takes the window (connect_close_page).
                if let Some(page) = ctx.tab_view.selected_page() {
                    ctx.tab_view.close_page(&page);
                }
                return glib::Propagation::Stop;
            }
            if ctrl && !shift && key == Key::l && ctx.spec.borrow().has_urlbar() {
                edit_address(&ctx);
                return glib::Propagation::Stop;
            }
            if ctrl && !shift && key == Key::r {
                if let Some(v) = current_webview(&ctx) {
                    v.reload();
                }
                return glib::Propagation::Stop;
            }
            // (With Shift held the key is N.)
            if ctrl && key.to_lower() == Key::n {
                // A window of this one's kind (an app's: a Full one); Shift: private.
                let spec = ctx.spec.borrow().sibling(shift);
                let w = build_window_as(&ctx.app, ctx.state.clone(), spec, vec![NEW_TAB_URI.into()], 0);
                w.present();
                return glib::Propagation::Stop;
            }
            if ctrl && !shift && key == Key::h {
                open_page(&ctx, "wadbrowser://history");
                return glib::Propagation::Stop;
            }
            if ctrl && !shift && key == Key::j {
                open_page(&ctx, "wadbrowser://downloads");
                return glib::Propagation::Stop;
            }
            if ctrl && shift && key.to_lower() == Key::p {
                open_page(&ctx, "wadbrowser://privacy");
                return glib::Propagation::Stop;
            }
            if ctrl && !shift && key == Key::f {
                ctx.find_revealer.set_reveal_child(true);
                ctx.find_entry.grab_focus();
                return glib::Propagation::Stop;
            }
            // Zoom is per site (wadbrowser_webkit::zoom).
            if ctrl && (key == Key::plus || key == Key::equal) {
                if let Some(v) = current_webview(&ctx) {
                    wadbrowser_webkit::zoom::zoom_in(&v);
                }
                return glib::Propagation::Stop;
            }
            if ctrl && key == Key::minus {
                if let Some(v) = current_webview(&ctx) {
                    wadbrowser_webkit::zoom::zoom_out(&v);
                }
                return glib::Propagation::Stop;
            }
            if ctrl && key == Key::_0 {
                if let Some(v) = current_webview(&ctx) {
                    wadbrowser_webkit::zoom::zoom_reset(&v);
                }
                return glib::Propagation::Stop;
            }
            if ctrl && (key == Key::Tab || key == Key::ISO_Left_Tab) {
                if shift {
                    let _ = ctx.tab_view.select_previous_page();
                } else {
                    let _ = ctx.tab_view.select_next_page();
                }
                return glib::Propagation::Stop;
            }
            if alt && key == Key::Left {
                if let Some(v) = current_webview(&ctx)
                    && v.can_go_back()
                {
                    v.go_back();
                }
                return glib::Propagation::Stop;
            }
            if alt && key == Key::Right {
                if let Some(v) = current_webview(&ctx)
                    && v.can_go_forward()
                {
                    v.go_forward();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    ));
    ctx.window.add_controller(controller);

    // The mouse's back and forward buttons, wherever it is in the window
    // (taken before the page, which does nothing with them).
    let side = gtk::GestureClick::builder().button(0).propagation_phase(gtk::PropagationPhase::Capture).build();
    side.connect_pressed(clone!(
        #[weak]
        ctx,
        move |g, _, _, _| {
            let button = g.current_button();
            if !matches!(button, MOUSE_BACK | MOUSE_FORWARD) {
                return;
            }
            g.set_state(gtk::EventSequenceState::Claimed);
            let Some(v) = current_webview(&ctx) else { return };
            if button == MOUSE_BACK && v.can_go_back() {
                v.go_back();
            } else if button == MOUSE_FORWARD && v.can_go_forward() {
                v.go_forward();
            }
        }
    ));
    ctx.window.add_controller(side);
}

fn save_session_from(ctx: &WindowCtx) {
    let model = ctx.tab_view.pages();
    let n = model.n_items();
    let mut tabs = Vec::new();
    let mut active = 0usize;
    let selected = ctx.tab_view.selected_page();

    for i in 0..n {
        if let Some(obj) = model.item(i)
            && let Ok(page) = obj.downcast::<TabPage>()
        {
            if selected.as_ref() == Some(&page) {
                active = i as usize;
            }
            if let Ok(view) = page.child().downcast::<WebView>() {
                let uri = view.uri().unwrap_or_else(|| NEW_TAB_URI.into()).to_string();
                tabs.push(uri);
            }
        }
    }

    let session = SessionState { windows: vec![SessionWindow { private: false, active_tab: active, tabs }] };
    if let Err(err) = Store::save_session(&ctx.state.paths, &session) {
        warn!("session save: {err}");
    } else {
        info!("session saved");
    }
}

pub fn restore_urls(state: &AppState) -> Option<(Vec<String>, usize)> {
    if !state.prefs.borrow().restore_session {
        return None;
    }
    let session = Store::load_session(&state.paths).ok().flatten()?;
    let win = session.windows.into_iter().find(|w| !w.private)?;
    if win.tabs.is_empty() {
        None
    } else {
        let active = win.active_tab.min(win.tabs.len().saturating_sub(1));
        Some((win.tabs, active))
    }
}

/// Select tab by index after restore.
pub fn select_tab_index(window_tabs: &TabView, index: usize) {
    let model = window_tabs.pages();
    if let Some(obj) = model.item(index as u32)
        && let Ok(page) = obj.downcast::<TabPage>()
    {
        window_tabs.set_selected_page(&page);
    }
}

// ------------------------------------------------- window kinds (modes.rs)
// What a window's kind adds to it, and the browser's windows as app.rs
// routes launches to them. The logic is wadbrowser_core::modes's; this only
// draws it, so it moves with the chrome.

thread_local! {
    /// The browser's windows (GTK is on this thread only).
    static WINDOWS: RefCell<Vec<Weak<WindowCtx>>> = const { RefCell::new(Vec::new()) };
}
static NEXT_WINDOW: AtomicU64 = AtomicU64::new(1);

fn register(ctx: &Rc<WindowCtx>) {
    WINDOWS.with(|w| w.borrow_mut().push(Rc::downgrade(ctx)));
}

fn unregister(id: u64) {
    WINDOWS.with(|w| w.borrow_mut().retain(|c| c.upgrade().is_some_and(|c| c.id != id)));
}

fn find(id: u64) -> Option<Rc<WindowCtx>> {
    WINDOWS.with(|w| w.borrow().iter().filter_map(Weak::upgrade).find(|c| c.id == id))
}

/// Every open window (for what changes them all: the apps, the keys).
fn all_windows() -> Vec<Rc<WindowCtx>> {
    WINDOWS.with(|w| w.borrow().iter().filter_map(Weak::upgrade).collect())
}

/// The browser's open windows and what each is, the most recently active
/// first (GtkApplication keeps them in that order).
pub(crate) fn open_windows(app: &adw::Application) -> Vec<(u64, WindowSpec)> {
    let all = all_windows();
    app.windows()
        .iter()
        .filter_map(|w| all.iter().find(|c| c.window.upcast_ref::<gtk::Window>() == w))
        .map(|c| (c.id, c.spec.borrow().clone()))
        .collect()
}

/// New tabs at `urls` in window `id` (the first one shown).
pub(crate) fn add_tabs(id: u64, urls: &[String]) {
    let Some(ctx) = find(id) else { return };
    for (i, url) in urls.iter().enumerate() {
        let _ = create_tab(&ctx, url, i == 0);
    }
}

/// Window `id` up, in front (GTK takes the launch's activation token).
pub(crate) fn present(id: u64) {
    if let Some(ctx) = find(id) {
        ctx.window.present();
    }
}

pub(crate) fn close(id: u64) {
    if let Some(ctx) = find(id) {
        ctx.window.close();
    }
}

/// A sign-in window again: at `url`, with `code` (its device page's script
/// made again with it). Its kind as it is now.
pub(crate) fn reuse_signin(id: u64, url: &str, code: Option<String>) -> Option<WindowSpec> {
    let ctx = find(id)?;
    {
        let mut spec = ctx.spec.borrow_mut();
        if let Some(s) = spec.signin.as_mut() {
            s.code = code;
        }
        if let Some(a) = spec.app.as_mut() {
            a.start = url.into();
        }
    }
    remove_scripts(&ctx);
    for view in views(&ctx) {
        add_signin_script(&ctx, &view);
    }
    wire::sync_code(&ctx);
    ctx.chrome.kept.borrow_mut().clear();
    open_uri(&ctx, url, false);
    let spec = ctx.spec.borrow().clone();
    Some(spec)
}

fn views(ctx: &WindowCtx) -> Vec<WebView> {
    let pages = ctx.tab_view.pages();
    (0..pages.n_items())
        .filter_map(|i| pages.item(i).and_downcast::<TabPage>())
        .filter_map(|p| p.child().downcast::<WebView>().ok())
        .collect()
}

/// The network session a window's pages use: a private window's own; else
/// its profile's (`spec.profile`), a web app's key's (signins.json), or the
/// browser's own (its `@browser` key's, else `default`). An ephemeral run's
/// keep nothing (wadbrowser_webkit::set_ephemeral_run).
fn session_for(state: &AppState, spec: &WindowSpec) -> SessionBundle {
    let default = || wadbrowser_core::profiles::DEFAULT.to_string();
    let profile = spec.profile.clone().or_else(|| match (&spec.app, &spec.signin) {
        // A sign-in's: the default profile, so what it signs in to stays
        // signed in in the browser.
        (_, Some(_)) => Some(default()),
        // A web app's: its key's here (signins.json), else the default
        // profile, so one sign-in covers every web app.
        (Some(a), None) => Some(
            a.app_id
                .strip_prefix(wadbrowser_core::cli::WEBAPP_PREFIX)
                .and_then(|id| wadbrowser_webkit::signins().app_profile(id))
                .unwrap_or_else(default),
        ),
        // The browser's own: its `@browser` key's, else default (persistent).
        (None, None) => None,
    });
    SessionBundle::for_window(&state.paths, profile.as_deref(), spec.private)
        .unwrap_or_else(|_| state.normal_session.clone())
}

/// The window's Wayland app_id: a web app's own (`wadspaces-webapp-<id>`),
/// else WadBrowser's, whatever the GApplication's (an instance's) is. GTK
/// sets the application's as the window is realized and mapped; this goes
/// after each, before the first frame, so the compositor sees it on map
/// (set on realize alone, it was lost: apps/wados-shell's Builder).
fn own_app_id(window: &ApplicationWindow, app_id: &str) {
    let id = app_id.to_owned();
    let set = move |w: &ApplicationWindow| match w.surface().and_downcast::<gdk4_wayland::WaylandToplevel>() {
        Some(t) => t.set_application_id(&id),
        None => tracing::debug!("not a Wayland toplevel: the window keeps GTK's app_id"),
    };
    window.connect_realize(set.clone());
    window.connect_map(set);
}

/// The chrome a window's kind adds: an app's picture and name (at the
/// header bar's start), Focus's page title, and "Offsite · host" with
/// "Back to <name>".
struct ModeChrome {
    identity: GtkBox,
    page_title: Label,
    offsite: GtkBox,
    offsite_label: Label,
    back_to: gtk::Button,
    /// The user scripts this window put in its views' content managers.
    scripts: RefCell<Vec<(UserContentManager, UserScript)>>,
    /// Where a sign-in window was last kept (signin::Saved).
    kept: RefCell<String>,
}

impl ModeChrome {
    fn new(_spec: &WindowSpec) -> ModeChrome {
        let identity = GtkBox::builder().spacing(9).css_classes(["wb-identity"]).valign(gtk::Align::Center).build();
        let page_title = Label::builder()
            .css_classes(["wb-page-title"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .hexpand(true)
            .build();
        let offsite = GtkBox::builder().spacing(10).halign(gtk::Align::Center).hexpand(true).build();
        let pill = GtkBox::builder().spacing(6).css_classes(["wb-offsite"]).valign(gtk::Align::Center).build();
        pill.append(&wad_gtk::Icon::new("external-link", 13));
        let offsite_label = Label::new(None);
        pill.append(&offsite_label);
        let back_to = gtk::Button::builder().css_classes(["wb-back-to"]).valign(gtk::Align::Center).build();
        offsite.append(&pill);
        offsite.append(&back_to);
        ModeChrome {
            identity,
            page_title,
            offsite,
            offsite_label,
            back_to,
            scripts: RefCell::default(),
            kept: RefCell::default(),
        }
    }

    /// Drawn for `spec`: an app's identity, what shows in the middle.
    fn show(&self, spec: &WindowSpec) {
        while let Some(c) = self.identity.first_child() {
            self.identity.remove(&c);
        }
        if let Some(a) = &spec.app {
            let pic: gtk::Widget = match a.icon.as_deref().filter(|p| std::path::Path::new(p).is_file()) {
                Some(icon) => {
                    let img = gtk::Image::from_file(icon);
                    img.set_pixel_size(22);
                    img.upcast()
                }
                None => {
                    let host = site::bare_host(&site::host_of(&a.start));
                    let id = a.app_id.strip_prefix(wadbrowser_core::cli::WEBAPP_PREFIX).unwrap_or(&host).to_string();
                    wad_gtk::app_icon::site(&id, Some(&a.name), Some(&a.start), 22)
                }
            };
            pic.set_valign(gtk::Align::Center);
            self.identity.append(&pic);
            self.identity.append(&Label::builder().label(&a.name).css_classes(["wb-identity-name"]).build());
            self.back_to.set_label(&format!("Back to {}", a.name));
        }
        self.identity.set_visible(spec.app.is_some());
        self.page_title.set_visible(spec.mode == Mode::Focus);
    }
}

/// The window drawn for its kind: what's in the header bar, its tint, the
/// tab bar (an app's hides with one tab), the window's own buttons.
fn show_mode(ctx: &WindowCtx) {
    let spec = ctx.spec.borrow().clone();
    ctx.chrome.show(&spec);
    ctx.header.address.root.set_visible(spec.has_urlbar());
    for (m, class) in [(Mode::Full, "wb-mode-full"), (Mode::Focus, "wb-mode-focus"), (Mode::App, "wb-mode-app")] {
        if spec.mode == m {
            ctx.window.add_css_class(class);
        } else {
            ctx.window.remove_css_class(class);
        }
    }
    ctx.tabs.sync(&ctx.tab_view, spec.autohide_tabs());
    sync_controls(ctx);
    sync_mode(ctx);
    sync_chrome(ctx);
}

/// The window's buttons (minimize, maximize, close) on the top row: the
/// tab bar's, or the header bar's when the tab bar's hidden.
fn sync_controls(ctx: &WindowCtx) {
    let tabs = ctx.tabs.root.get_visible();
    ctx.tabs.controls.set_visible(tabs);
    ctx.header.controls.set_visible(!tabs);
}

/// A page joining the window: its kind's scripts, and its chrome kept up
/// with where it is.
fn attach_mode(ctx: &Rc<WindowCtx>, view: &WebView) {
    add_signin_script(ctx, view);
    for prop in ["uri", "title"] {
        view.connect_notify_local(
            Some(prop),
            clone!(
                #[weak]
                ctx,
                move |view, _| {
                    if current_webview(&ctx).as_ref() == Some(view) {
                        sync_mode(&ctx);
                    }
                }
            ),
        );
    }
}

/// The window's title, Focus's page title, offsite, and where a sign-in is.
fn sync_mode(ctx: &WindowCtx) {
    let view = current_webview(ctx);
    let uri = view.as_ref().and_then(|v| v.uri()).map(|u| u.to_string()).unwrap_or_default();
    let title = view.as_ref().and_then(|v| v.title()).map(|t| t.to_string());
    let spec = ctx.spec.borrow();
    ctx.window.set_title(Some(&spec.title(title.as_deref())));
    if spec.mode == Mode::Focus {
        let shown = title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| tab_title_for(&uri));
        ctx.chrome.page_title.set_label(&shown);
    }
    let offsite = spec.offsite(&uri);
    if offsite {
        ctx.chrome.offsite_label.set_label(&format!("Offsite · {}", site::bare_host(&site::host_of(&uri))));
    }
    ctx.chrome.offsite.set_visible(offsite);
    // A sign-in window: where it is, for --show once it's closed.
    if let Some(s) = &spec.signin
        && !uri.is_empty()
        && !uri.starts_with("about:")
        && *ctx.chrome.kept.borrow() != uri
    {
        let saved = signin::Saved { url: uri.clone(), code: s.code.clone() };
        if let Err(e) = saved.store(&ctx.state.paths, s.provider) {
            warn!("keeping the {} sign-in: {e}", s.provider.id());
        }
        *ctx.chrome.kept.borrow_mut() = uri;
    }
}

/// GitHub's sign-in window: the script that types its device code in, in
/// this page's own content manager (each view has its own). It's kept, to
/// be taken out again when the code changes (reuse_signin) or the window
/// goes, and it only acts on the device page of the sign-in's host.
fn add_signin_script(ctx: &WindowCtx, view: &WebView) {
    let spec = ctx.spec.borrow();
    let (Some(s), Some(a)) = (&spec.signin, &spec.app) else { return };
    let (Provider::Github, Some(code)) = (s.provider, s.code.as_deref()) else { return };
    let Some(ucm) = view.user_content_manager() else { return };
    let js = signin::device_code_script(&site::authority_of(&a.start), code);
    let script = UserScript::new(&js, UserContentInjectedFrames::TopFrame, UserScriptInjectionTime::End, &[], &[]);
    ucm.add_script(&script);
    ctx.chrome.scripts.borrow_mut().push((ucm, script));
}

fn remove_scripts(ctx: &WindowCtx) {
    for (ucm, script) in ctx.chrome.scripts.borrow_mut().drain(..) {
        ucm.remove_script(&script);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tab_is_named_before_its_page_says() {
        assert_eq!(tab_title_for("wadbrowser://newtab"), "New tab");
        assert_eq!(tab_title_for("wadbrowser://privacy?x=1"), "Privacy");
        assert_eq!(tab_title_for("wadbrowser://settings"), "Settings");
        assert_eq!(tab_title_for("https://www.GitHub.com/x"), "github.com");
    }

    #[test]
    fn the_new_tab_is_the_placeholder() {
        for u in ["", "about:blank", "wadbrowser://newtab", "wadbrowser://home"] {
            assert!(is_newtab(u), "{u}");
        }
        assert!(!is_newtab("wadbrowser://settings"));
        assert!(!is_newtab("https://example.com/"));
    }

    #[test]
    fn devtools_are_off_unless_asked_for() {
        // The test process doesn't set it.
        if std::env::var_os("WADBROWSER_DEVTOOLS").is_none() {
            assert!(!devtools());
        }
    }
}
