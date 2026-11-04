use crate::style::APP_CSS;
use adw::prelude::*;
use adw::{ApplicationWindow, HeaderBar, TabBar, TabPage, TabView, Toast, ToastOverlay, WindowTitle};
use anyhow::Result;
use glib::clone;
use gtk::gdk::{Key, ModifierType};
use gtk::{Box as GtkBox, Button, Entry, EventControllerKey, Label, Orientation, ProgressBar, Revealer, SearchEntry};
use gtk::{gio, glib};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tracing::{info, warn};
use wadbrowser_core::internal::{
    about_html, bookmarks_html, downloads_html, error_html, history_html, is_internal, map_about, newtab_html,
    privacy_html, settings_html,
};
use wadbrowser_core::store::{SessionState, SessionWindow};
use wadbrowser_core::{APP_NAME, AppPaths, HOME_URI, NEW_TAB_URI, Preferences, Store, normalize_navigation_input};
use wadbrowser_privacy::PrivacyEngine;
use wadbrowser_webkit::{
    SessionBundle, apply_privacy_headers, create_web_view, file_uri_for_path, filename_from_url, inject_cosmetic_css,
    load_html, navigate, path_from_destination, unique_download_path, wire_privacy,
};
use webkit6::prelude::*;
use webkit6::{Download, FindOptions, LoadEvent, WebView};

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
    url_entry: Entry,
    progress: ProgressBar,
    toast: ToastOverlay,
    bookmark_bar: GtkBox,
    find_revealer: Revealer,
    find_entry: SearchEntry,
    title_widget: WindowTitle,
    back_btn: Button,
    forward_btn: Button,
    reload_btn: Button,
}

pub fn build_window(
    app: &adw::Application,
    state: Rc<AppState>,
    private: bool,
    initial_urls: Option<Vec<String>>,
    active_tab: usize,
) -> ApplicationWindow {
    let session = if private { SessionBundle::ephemeral() } else { state.normal_session.clone() };

    let window = ApplicationWindow::builder()
        .application(app)
        .default_width(1200)
        .default_height(800)
        .title(if private { format!("{APP_NAME} (Private)") } else { APP_NAME.into() })
        .build();

    let toast = ToastOverlay::new();
    let root = GtkBox::new(Orientation::Vertical, 0);

    let title_widget = WindowTitle::new(APP_NAME, if private { "Private" } else { "Browser" });
    title_widget.add_css_class("window-title");

    let header = HeaderBar::new();

    let back_btn = icon_btn("go-previous-symbolic", "Back");
    let forward_btn = icon_btn("go-next-symbolic", "Forward");
    let reload_btn = icon_btn("view-refresh-symbolic", "Reload");
    let home_btn = icon_btn("go-home-symbolic", "Home");
    let new_tab_btn = icon_btn("tab-new-symbolic", "New tab");
    let bookmark_btn = icon_btn("bookmark-new-symbolic", "Bookmark this page");
    let menu_btn = icon_btn("open-menu-symbolic", "Menu");

    let url_entry = Entry::new();
    url_entry.add_css_class("url-entry");
    url_entry.set_hexpand(true);
    url_entry.set_placeholder_text(Some("Search or enter address"));
    url_entry.set_input_purpose(gtk::InputPurpose::Url);

    let title_box = GtkBox::new(Orientation::Horizontal, 8);
    if private {
        let badge = Label::new(Some("PRIVATE"));
        badge.add_css_class("private-badge");
        title_box.append(&badge);
    }
    title_box.append(&url_entry);
    title_box.set_hexpand(true);

    header.pack_start(&back_btn);
    header.pack_start(&forward_btn);
    header.pack_start(&reload_btn);
    header.pack_start(&home_btn);
    header.pack_start(&new_tab_btn);
    header.set_title_widget(Some(&title_box));
    header.pack_end(&menu_btn);
    header.pack_end(&bookmark_btn);

    let progress = ProgressBar::new();
    progress.add_css_class("progress");
    progress.set_fraction(0.0);
    progress.set_visible(false);

    let tab_view = TabView::new();
    let tab_bar = TabBar::new();
    tab_bar.set_view(Some(&tab_view));
    tab_bar.set_autohide(false);

    let bookmark_bar = GtkBox::new(Orientation::Horizontal, 4);
    bookmark_bar.add_css_class("bookmark-bar");
    let bookmark_revealer = Revealer::new();
    bookmark_revealer.set_child(Some(&bookmark_bar));
    bookmark_revealer.set_reveal_child(state.prefs.borrow().show_bookmark_bar && !private);

    let find_box = GtkBox::new(Orientation::Horizontal, 8);
    find_box.add_css_class("find-bar");
    let find_entry = SearchEntry::new();
    find_entry.set_hexpand(true);
    find_entry.set_placeholder_text(Some("Find in page"));
    let find_close = icon_btn("window-close-symbolic", "Close find");
    find_box.append(&find_entry);
    find_box.append(&find_close);
    let find_revealer = Revealer::new();
    find_revealer.set_child(Some(&find_box));
    find_revealer.set_reveal_child(false);

    root.append(&header);
    root.append(&progress);
    root.append(&tab_bar);
    root.append(&bookmark_revealer);
    root.append(&tab_view);
    root.append(&find_revealer);

    toast.set_child(Some(&root));
    window.set_content(Some(&toast));

    let provider = gtk::CssProvider::new();
    provider.load_from_string(APP_CSS);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }

    let ctx = Rc::new(WindowCtx {
        window: window.clone(),
        app: app.clone(),
        state: state.clone(),
        session,
        private,
        tab_view: tab_view.clone(),
        url_entry: url_entry.clone(),
        progress: progress.clone(),
        toast: toast.clone(),
        bookmark_bar: bookmark_bar.clone(),
        find_revealer: find_revealer.clone(),
        find_entry: find_entry.clone(),
        title_widget: title_widget.clone(),
        back_btn: back_btn.clone(),
        forward_btn: forward_btn.clone(),
        reload_btn: reload_btn.clone(),
    });

    refresh_bookmark_bar(&ctx);

    back_btn.connect_clicked(clone!(
        #[strong]
        ctx,
        move |_| {
            if let Some(view) = current_webview(&ctx)
                && view.can_go_back()
            {
                view.go_back();
            }
        }
    ));
    forward_btn.connect_clicked(clone!(
        #[strong]
        ctx,
        move |_| {
            if let Some(view) = current_webview(&ctx)
                && view.can_go_forward()
            {
                view.go_forward();
            }
        }
    ));
    reload_btn.connect_clicked(clone!(
        #[strong]
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
    home_btn.connect_clicked(clone!(
        #[strong]
        ctx,
        move |_| {
            let home = ctx.state.prefs.borrow().homepage.clone();
            open_uri(&ctx, &home, false);
        }
    ));
    new_tab_btn.connect_clicked(clone!(
        #[strong]
        ctx,
        move |_| {
            let _ = create_tab(&ctx, NEW_TAB_URI, true);
        }
    ));
    bookmark_btn.connect_clicked(clone!(
        #[strong]
        ctx,
        move |_| {
            bookmark_current(&ctx);
        }
    ));

    url_entry.connect_activate(clone!(
        #[strong]
        ctx,
        move |entry| {
            let text = entry.text().to_string();
            let prefs = ctx.state.prefs.borrow().clone();
            let uri = normalize_navigation_input(&text, &prefs);
            open_uri(&ctx, &uri, false);
        }
    ));

    tab_view.connect_page_attached(clone!(
        #[strong]
        ctx,
        move |_, page, _| {
            if let Ok(child) = page.child().downcast::<WebView>() {
                wire_webview(&ctx, &child, page);
            }
        }
    ));

    tab_view.connect_selected_page_notify(clone!(
        #[strong]
        ctx,
        move |_| {
            sync_chrome(&ctx);
        }
    ));

    tab_view.connect_close_page(clone!(
        #[strong]
        ctx,
        move |view, page| {
            if view.n_pages() <= 1 {
                let _ = create_tab(&ctx, NEW_TAB_URI, true);
            }
            view.close_page_finish(page, true);
            glib::Propagation::Stop
        }
    ));

    find_close.connect_clicked(clone!(
        #[strong]
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
        #[strong]
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

    setup_menu(&ctx, &menu_btn);
    setup_shortcuts(&ctx);

    // One download handler per network session
    ctx.session.network.connect_download_started(clone!(
        #[strong]
        ctx,
        move |_, download| {
            handle_download(&ctx, download);
        }
    ));

    if let Some(urls) = initial_urls {
        if urls.is_empty() {
            let _ = create_tab(&ctx, NEW_TAB_URI, true);
        } else {
            for (i, url) in urls.iter().enumerate() {
                let _ = create_tab(&ctx, url, i == 0);
            }
            if active_tab > 0 {
                select_tab_index(&ctx.tab_view, active_tab);
            }
        }
    } else {
        let _ = create_tab(&ctx, NEW_TAB_URI, true);
    }

    window.connect_close_request(clone!(
        #[strong]
        ctx,
        move |_| {
            if !ctx.private {
                save_session_from(&ctx);
            }
            glib::Propagation::Proceed
        }
    ));

    window
}

fn icon_btn(icon: &str, tooltip: &str) -> Button {
    let btn = Button::from_icon_name(icon);
    btn.set_tooltip_text(Some(tooltip));
    btn
}

fn create_tab(ctx: &Rc<WindowCtx>, uri: &str, select: bool) -> Result<TabPage> {
    let prefs = ctx.state.prefs.borrow().clone();
    let view = create_web_view(&ctx.session, &prefs);
    apply_privacy_headers(&view, &prefs);
    wire_privacy(&view, ctx.state.privacy.clone());

    view.connect_create(clone!(
        #[strong]
        ctx,
        move |_, _| {
            let prefs = ctx.state.prefs.borrow().clone();
            let view = create_web_view(&ctx.session, &prefs);
            apply_privacy_headers(&view, &prefs);
            wire_privacy(&view, ctx.state.privacy.clone());
            let page = ctx.tab_view.append(&view);
            page.set_title("New Tab");
            ctx.tab_view.set_selected_page(&page);
            Some(view.upcast::<gtk::Widget>())
        }
    ));

    let page = ctx.tab_view.append(&view);
    page.set_title("New Tab");
    if select {
        ctx.tab_view.set_selected_page(&page);
    }
    open_in_view(ctx, &view, uri);
    Ok(page)
}

fn wire_webview(ctx: &Rc<WindowCtx>, view: &WebView, page: &TabPage) {
    view.connect_notify_local(
        Some("title"),
        clone!(
            #[strong]
            page,
            #[strong]
            ctx,
            move |view, _| {
                let title = view.title().unwrap_or_else(|| "New Tab".into());
                page.set_title(&title);
                if is_selected(&ctx, &page) {
                    ctx.title_widget.set_title(&title);
                    sync_chrome(&ctx);
                }
            }
        ),
    );

    view.connect_notify_local(
        Some("uri"),
        clone!(
            #[strong]
            page,
            #[strong]
            ctx,
            move |_, _| {
                if is_selected(&ctx, &page) {
                    sync_chrome(&ctx);
                }
            }
        ),
    );

    view.connect_load_changed(clone!(
        #[strong]
        ctx,
        #[strong]
        page,
        move |view, event| {
            match event {
                LoadEvent::Started | LoadEvent::Redirected => {
                    if is_selected(&ctx, &page) {
                        ctx.progress.set_visible(true);
                        ctx.progress.set_fraction(0.15);
                        ctx.reload_btn.set_icon_name("process-stop-symbolic");
                    }
                }
                LoadEvent::Committed => {
                    if let Some(uri) = view.uri() {
                        let css = ctx.state.privacy.cosmetic_styles(&uri);
                        inject_cosmetic_css(view, &css);
                    }
                    if is_selected(&ctx, &page) {
                        ctx.progress.set_fraction(0.7);
                    }
                }
                LoadEvent::Finished => {
                    if is_selected(&ctx, &page) {
                        ctx.progress.set_fraction(1.0);
                        ctx.progress.set_visible(false);
                        ctx.reload_btn.set_icon_name("view-refresh-symbolic");
                        sync_chrome(&ctx);
                    }
                    if !ctx.private {
                        let title = view.title().unwrap_or_default().to_string();
                        let uri = view.uri().unwrap_or_default().to_string();
                        if let Err(err) = ctx.state.store.record_history(&title, &uri) {
                            warn!("history: {err}");
                        }
                        wadbrowser_core::metrics::record_page_load(&ctx.state.store);
                    }
                }
                _ => {}
            }
        }
    ));

    view.connect_load_failed(clone!(
        #[strong]
        ctx,
        move |view, _event, uri, error| {
            let _ = &ctx;
            let html = error_html("Unable to load page", &error.to_string(), uri);
            load_html(view, &html, "cactus://error");
            true
        }
    ));

    view.connect_context_menu(clone!(
        #[strong]
        ctx,
        move |_view, _menu, hit| {
            if hit.context_is_link()
                && let Some(link) = hit.link_uri()
            {
                let link = link.to_string();
                // Open linked URLs from the chrome menu via toast hint; keep default menu.
                let _ = (&ctx, link);
            }
            false
        }
    ));
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
    let mut uri = uri.to_string();
    if let Some(mapped) = map_about(&uri) {
        uri = mapped.to_string();
    }

    if is_internal(&uri) {
        let html = render_internal(ctx, &uri);
        load_html(view, &html, &uri);
        ctx.url_entry.set_text(&uri);
        return;
    }

    navigate(view, &uri);
    ctx.url_entry.set_text(&uri);
}

fn render_internal(ctx: &Rc<WindowCtx>, uri: &str) -> String {
    let prefs = ctx.state.prefs.borrow().clone();
    match uri {
        "cactus://newtab" | "cactus://home" => {
            let bookmarks = ctx.state.store.list_bookmark_bar().unwrap_or_default();
            let history = ctx.state.store.list_history(12).unwrap_or_default();
            let stats = ctx.state.privacy.stats();
            newtab_html(&bookmarks, &history, stats.blocked_requests, stats.cosmetic_injections)
        }
        "cactus://privacy" | "cactus://shield" => {
            let stats = ctx.state.privacy.stats();
            privacy_html(
                &prefs,
                stats.blocked_requests,
                stats.allowed_requests,
                stats.cosmetic_injections,
                ctx.state.privacy.engine_loaded(),
                &ctx.state.privacy.allowlist().list(),
            )
        }
        "cactus://settings" | "cactus://preferences" => settings_html(&prefs),
        "cactus://history" => {
            let entries = ctx.state.store.list_history(200).unwrap_or_default();
            history_html(&entries)
        }
        "cactus://bookmarks" => {
            let entries = ctx.state.store.list_bookmarks().unwrap_or_default();
            bookmarks_html(&entries)
        }
        "cactus://downloads" => {
            let entries = ctx.state.store.list_downloads(100).unwrap_or_default();
            downloads_html(&entries)
        }
        "cactus://about" | "cactus://landing" | "cactus://project" => {
            about_html(&wadbrowser_webkit::webkit_version_string())
        }
        _ => error_html("Unknown internal page", "This cactus:// page is not implemented.", uri),
    }
}

fn current_webview(ctx: &WindowCtx) -> Option<WebView> {
    ctx.tab_view.selected_page().and_then(|p| p.child().downcast::<WebView>().ok())
}

fn sync_chrome(ctx: &WindowCtx) {
    let Some(view) = current_webview(ctx) else {
        return;
    };
    let uri = view.uri().unwrap_or_default();
    if !ctx.url_entry.has_focus() {
        ctx.url_entry.set_text(&uri);
    }
    let title = view.title().unwrap_or_else(|| APP_NAME.into());
    ctx.title_widget.set_title(&title);
    ctx.back_btn.set_sensitive(view.can_go_back());
    ctx.forward_btn.set_sensitive(view.can_go_forward());
    if view.is_loading() {
        ctx.reload_btn.set_icon_name("process-stop-symbolic");
    } else {
        ctx.reload_btn.set_icon_name("view-refresh-symbolic");
    }
}

fn bookmark_current(ctx: &Rc<WindowCtx>) {
    if ctx.private {
        show_toast(ctx, "Bookmarks are disabled in private windows");
        return;
    }
    let Some(view) = current_webview(ctx) else {
        return;
    };
    let uri = view.uri().unwrap_or_default().to_string();
    let title = view.title().unwrap_or_else(|| uri.clone().into()).to_string();
    if uri.starts_with("cactus://") || uri.is_empty() {
        show_toast(ctx, "Can't bookmark this page");
        return;
    }
    match ctx.state.store.add_bookmark(&title, &uri, "Bar") {
        Ok(_) => {
            wadbrowser_core::metrics::record_bookmark_added(&ctx.state.store);
            show_toast(ctx, "Bookmarked");
            refresh_bookmark_bar(ctx);
        }
        Err(err) => show_toast(ctx, &format!("Bookmark failed: {err}")),
    }
}

fn refresh_bookmark_bar(ctx: &Rc<WindowCtx>) {
    while let Some(child) = ctx.bookmark_bar.first_child() {
        ctx.bookmark_bar.remove(&child);
    }
    if ctx.private {
        return;
    }
    let Ok(bookmarks) = ctx.state.store.list_bookmark_bar() else {
        return;
    };
    for b in bookmarks {
        let btn = Button::with_label(&b.title);
        btn.set_tooltip_text(Some(&b.url));
        let url = b.url.clone();
        btn.connect_clicked(clone!(
            #[strong]
            ctx,
            move |_| {
                open_uri(&ctx, &url, false);
            }
        ));
        ctx.bookmark_bar.append(&btn);
    }
}

fn handle_download(ctx: &Rc<WindowCtx>, download: &Download) {
    let dest_dir = ctx.state.paths.downloads_dir();
    let _ = std::fs::create_dir_all(&dest_dir);
    let url = download.request().and_then(|r| r.uri()).unwrap_or_default().to_string();
    let suggested = filename_from_url(&url);
    let path = unique_download_path(&dest_dir, &suggested);
    download.set_destination(&file_uri_for_path(&path));

    let fname = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| suggested.clone());

    let record_id = if !ctx.private {
        ctx.state.store.add_download(&url, &fname, &path, "started").ok().map(|r| r.id)
    } else {
        None
    };
    show_toast(ctx, &format!("Downloading {fname}"));

    download.connect_finished(clone!(
        #[strong]
        ctx,
        move |d| {
            let dest = d.destination().unwrap_or_default().to_string();
            let path = path_from_destination(&dest);
            if let Some(id) = record_id {
                let _ = ctx.state.store.update_download_status(id, "finished");
            } else {
                let _ = ctx.state.store.update_download_status_by_path(&path.to_string_lossy(), "finished");
            }
            wadbrowser_core::metrics::record_download_completed(&ctx.state.store);
            show_toast(&ctx, &format!("Download complete: {}", path.display()));
        }
    ));
    download.connect_failed(clone!(
        #[strong]
        ctx,
        move |d, err| {
            let dest = d.destination().unwrap_or_default().to_string();
            let path = path_from_destination(&dest);
            if let Some(id) = record_id {
                let _ = ctx.state.store.update_download_status(id, "failed");
            } else {
                let _ = ctx.state.store.update_download_status_by_path(&path.to_string_lossy(), "failed");
            }
            show_toast(&ctx, &format!("Download failed: {err}"));
        }
    ));
}

fn show_toast(ctx: &WindowCtx, msg: &str) {
    ctx.toast.add_toast(Toast::new(msg));
}

fn setup_menu(ctx: &Rc<WindowCtx>, menu_btn: &Button) {
    let pop = gtk::PopoverMenu::from_model(None::<&gio::MenuModel>);
    let menu = gio::Menu::new();
    menu.append(Some("New Tab"), Some("win.new-tab"));
    menu.append(Some("New Private Window"), Some("win.new-private"));
    menu.append(Some("Bookmarks"), Some("win.bookmarks"));
    menu.append(Some("History"), Some("win.history"));
    menu.append(Some("Downloads"), Some("win.downloads"));
    menu.append(Some("Privacy Dashboard"), Some("win.privacy"));
    menu.append(Some("Find in Page"), Some("win.find"));
    menu.append(Some("Settings"), Some("win.settings"));
    menu.append(Some("Toggle Ad Blocking"), Some("win.toggle-blocking"));
    menu.append(Some("Clear Cookies & Cache"), Some("win.clear-data"));
    menu.append(Some("About"), Some("win.about"));
    pop.set_menu_model(Some(&menu));
    pop.set_parent(menu_btn);

    menu_btn.connect_clicked(move |_| {
        pop.popup();
    });

    let win = &ctx.window;
    add_action(
        win,
        "new-tab",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                let _ = create_tab(&ctx, NEW_TAB_URI, true);
            }
        ),
    );
    add_action(
        win,
        "new-private",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                let w = build_window(&ctx.app, ctx.state.clone(), true, Some(vec![NEW_TAB_URI.into()]), 0);
                w.present();
            }
        ),
    );
    add_action(
        win,
        "bookmarks",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                open_uri(&ctx, "cactus://bookmarks", false);
            }
        ),
    );
    add_action(
        win,
        "history",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                open_uri(&ctx, "cactus://history", false);
            }
        ),
    );
    add_action(
        win,
        "downloads",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                open_uri(&ctx, "cactus://downloads", false);
            }
        ),
    );
    add_action(
        win,
        "privacy",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                open_uri(&ctx, "cactus://privacy", false);
            }
        ),
    );
    add_action(
        win,
        "find",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                ctx.find_revealer.set_reveal_child(true);
                ctx.find_entry.grab_focus();
            }
        ),
    );
    add_action(
        win,
        "settings",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                crate::settings::open_settings_dialog(&ctx.window, ctx.state.paths.clone(), ctx.state.prefs.clone(), {
                    let privacy = ctx.state.privacy.clone();
                    let bookmark_revealer_paths = ctx.state.paths.clone();
                    move |prefs| {
                        privacy.set_enabled(prefs.block_ads_trackers);
                        privacy.set_cosmetic_enabled(prefs.block_cosmetic_filters);
                        let _ = bookmark_revealer_paths;
                    }
                });
            }
        ),
    );
    add_action(
        win,
        "about",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                open_uri(&ctx, "cactus://about", false);
            }
        ),
    );
    add_action(
        win,
        "toggle-blocking",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                let enabled = !ctx.state.privacy.is_enabled();
                ctx.state.privacy.set_enabled(enabled);
                ctx.state.prefs.borrow_mut().block_ads_trackers = enabled;
                let _ = ctx.state.prefs.borrow().save(&ctx.state.paths);
                show_toast(&ctx, if enabled { "Ad & tracker blocking on" } else { "Blocking off" });
            }
        ),
    );
    add_action(
        win,
        "clear-data",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                ctx.session.clear_cookies_and_site_data();
                ctx.session.clear_http_cache();
                let _ = ctx.state.store.clear_history();
                show_toast(&ctx, "Cleared cookies, cache, and history");
            }
        ),
    );
    add_action(
        win,
        "zoom-in",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                if let Some(v) = current_webview(&ctx) {
                    v.set_zoom_level(v.zoom_level() + 0.1);
                }
            }
        ),
    );
    add_action(
        win,
        "zoom-out",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                if let Some(v) = current_webview(&ctx) {
                    v.set_zoom_level((v.zoom_level() - 0.1).max(0.3));
                }
            }
        ),
    );
    add_action(
        win,
        "zoom-reset",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                if let Some(v) = current_webview(&ctx) {
                    v.set_zoom_level(1.0);
                }
            }
        ),
    );
    add_action(
        win,
        "focus-url",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                ctx.url_entry.grab_focus();
                ctx.url_entry.select_region(0, -1);
            }
        ),
    );
    add_action(
        win,
        "close-tab",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                if let Some(page) = ctx.tab_view.selected_page() {
                    ctx.tab_view.close_page(&page);
                }
            }
        ),
    );
    add_action(
        win,
        "reload",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                if let Some(v) = current_webview(&ctx) {
                    v.reload();
                }
            }
        ),
    );
    add_action(
        win,
        "home",
        clone!(
            #[strong]
            ctx,
            move |_, _| {
                open_uri(&ctx, HOME_URI, false);
            }
        ),
    );
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
    controller.connect_key_pressed(clone!(
        #[strong]
        ctx,
        move |_, key, _code, mods| {
            let ctrl = mods.contains(ModifierType::CONTROL_MASK);
            let shift = mods.contains(ModifierType::SHIFT_MASK);
            let alt = mods.contains(ModifierType::ALT_MASK);

            if ctrl && !shift && key == Key::t {
                let _ = create_tab(&ctx, NEW_TAB_URI, true);
                return glib::Propagation::Stop;
            }
            if ctrl && !shift && key == Key::w {
                if let Some(page) = ctx.tab_view.selected_page() {
                    ctx.tab_view.close_page(&page);
                }
                return glib::Propagation::Stop;
            }
            if ctrl && !shift && key == Key::l {
                ctx.url_entry.grab_focus();
                ctx.url_entry.select_region(0, -1);
                return glib::Propagation::Stop;
            }
            if ctrl && !shift && key == Key::r {
                if let Some(v) = current_webview(&ctx) {
                    v.reload();
                }
                return glib::Propagation::Stop;
            }
            if ctrl && shift && key == Key::n {
                let w = build_window(&ctx.app, ctx.state.clone(), true, Some(vec![NEW_TAB_URI.into()]), 0);
                w.present();
                return glib::Propagation::Stop;
            }
            if ctrl && !shift && key == Key::n {
                let w = build_window(&ctx.app, ctx.state.clone(), false, Some(vec![NEW_TAB_URI.into()]), 0);
                w.present();
                return glib::Propagation::Stop;
            }
            if ctrl && !shift && key == Key::d {
                bookmark_current(&ctx);
                return glib::Propagation::Stop;
            }
            if ctrl && !shift && key == Key::h {
                open_uri(&ctx, "cactus://history", false);
                return glib::Propagation::Stop;
            }
            if ctrl && !shift && key == Key::j {
                open_uri(&ctx, "cactus://downloads", false);
                return glib::Propagation::Stop;
            }
            if ctrl && shift && key == Key::p {
                open_uri(&ctx, "cactus://privacy", false);
                return glib::Propagation::Stop;
            }
            if ctrl && !shift && key == Key::f {
                ctx.find_revealer.set_reveal_child(true);
                ctx.find_entry.grab_focus();
                return glib::Propagation::Stop;
            }
            if ctrl && (key == Key::plus || key == Key::equal) {
                if let Some(v) = current_webview(&ctx) {
                    v.set_zoom_level(v.zoom_level() + 0.1);
                }
                return glib::Propagation::Stop;
            }
            if ctrl && key == Key::minus {
                if let Some(v) = current_webview(&ctx) {
                    v.set_zoom_level((v.zoom_level() - 0.1).max(0.3));
                }
                return glib::Propagation::Stop;
            }
            if ctrl && key == Key::_0 {
                if let Some(v) = current_webview(&ctx) {
                    v.set_zoom_level(1.0);
                }
                return glib::Propagation::Stop;
            }
            if ctrl && key == Key::Tab {
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
