//! WadBrowser's v2 widgets and `wadbrowser://` pages, one per run, with the
//! spike's sample data (its "populated" world), for a look and for
//! screenshots:
//!
//! ```sh
//! cargo run -p wadbrowser-ui --example gallery -- <name> [--fullscreen]
//! cargo run -p wadbrowser-ui --example gallery -- list
//! ```
//!
//! A mock header bar (the shield, the profile chip, ⋮) holds the widgets;
//! pages show in a WebView with the `cactus` scheme served as the browser
//! will: `wadbrowser://res/<file>` from `internal::resource`, the page itself
//! from its function. What a page asks (its "wadbrowser" message handler) is
//! printed. It prints `ready` once there's something to see. Popovers are
//! opened without autohide: there's no click for them to grab with.

use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use chrono::Utc;
use gtk::{gdk, gio, glib};
use wadbrowser_core::internal::{self, PageAction, Privacy, ProfileRow, Settings, SiteCount, Tile};
use wadbrowser_core::{HistoryEntry, PermissionKind, Preferences};
use wadbrowser_ui::chip::{ChipInfo, ProfileChip};
use wadbrowser_ui::data::{Mode, Profile, WebKey};
use wadbrowser_ui::dialogs::create_app::{CreateAppActions, CreateAppInfo, create_app};
use wadbrowser_ui::dialogs::new_profile::{NewProfileActions, new_profile};
use wadbrowser_ui::dialogs::permission::{PermissionInfo, permission};
use wadbrowser_ui::dialogs::signins::{add_signin, edit_signins};
use wadbrowser_ui::menu_popover::{MenuPopover, Place};
use wadbrowser_ui::menus::{
    MainActions, MainMenu, PageActions, PageMenu, ProfileActions, ProfileMenu, main_menu, page_menu, profile_menu,
};
use wadbrowser_ui::shield::{Shield, ShieldActions, ShieldInfo};
use webkit6::prelude::*;

const NAMES: &[(&str, &str)] = &[
    ("newtab", "wadbrowser://newtab: the catalog's web apps and one of yours"),
    ("newtab-menu", "wadbrowser://newtab: a tile of yours, right-clicked"),
    ("newtab-search", "wadbrowser://newtab: a search typed and sent (prints what it asks)"),
    ("newtab-plain", "wadbrowser://newtab with no apps of your own, searching with Startpage"),
    ("privacy", "wadbrowser://privacy after a few sites"),
    ("privacy-private", "wadbrowser://privacy in a private window, with a site allowed"),
    ("privacy-clicks", "wadbrowser://privacy: its toggle and Block again clicked (prints what they ask)"),
    ("settings", "wadbrowser://settings"),
    ("history", "wadbrowser://history"),
    ("history-private", "wadbrowser://history in a private window"),
    ("about", "wadbrowser://about"),
    ("error", "a page that didn't load"),
    ("blocked", "a social site, blocked"),
    ("missing", "a wadbrowser:// page that doesn't exist"),
    ("bookmarks", "wadbrowser://bookmarks (Cactus's, unlinked)"),
    ("page-menu", "right-clicking a web page"),
    ("page-menu-signins", "right-clicking a web page: Saved sign-ins"),
    ("page-menu-cactus", "right-clicking one of the browser's own pages"),
    ("main-menu", "the ⋮ menu, its Mode submenu open"),
    ("profile-menu", "the profile chip's menu"),
    ("profile-menu-private", "the profile chip's menu in a private window"),
    ("chips", "profile chips: Personal, Work, Private"),
    ("shield", "the shield on a web page"),
    ("shield-off", "the shield with the blockers off for the site"),
    ("shield-cactus", "the shield on one of the browser's own pages"),
    ("create-app", "Create app from this page"),
    ("create-app-done", "Create app: it's an app"),
    ("create-app-error", "Create app: not an address on the web"),
    ("new-profile", "New profile"),
    ("add-signin", "A new sign-in for this site"),
    ("edit-signins", "Saved sign-ins"),
    ("permission", "a site asking for the camera and microphone"),
    ("permission-notifications", "a site asking to send notifications"),
];

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = args.iter().find(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| "list".into());
    let fullscreen = args.iter().any(|a| a == "--fullscreen");
    if name == "list" || !NAMES.iter().any(|(n, _)| *n == name) {
        for (n, what) in NAMES {
            println!("{n:22} {what}");
        }
        return glib::ExitCode::SUCCESS;
    }
    let app = adw::Application::builder()
        .application_id("io.wados.Browser.Gallery")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_startup(|_| {
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
        let display = gdk::Display::default().expect("a display");
        wadbrowser_ui::style::install(&display);
        wad_gtk::theme::add_css(&display, GALLERY_CSS);
    });
    app.connect_activate(move |app| show(app, &name, fullscreen));
    app.run_with_args::<&str>(&[])
}

const GALLERY_CSS: &str = "
.gallery { background-color: var(--bg-2); }
.gallery-header { min-height: 46px; padding: 0 8px; background-color: var(--surface-2); border-bottom: 1px solid var(--line); }
.gallery-url { min-height: 32px; border-radius: 10px; background-color: var(--bg-2); border: 1px solid var(--line); color: var(--muted); font-size: 13px; }
";

/// The spike's populated world.
fn profiles() -> Vec<Profile> {
    vec![
        Profile { id: "default".into(), name: "Personal".into(), color: "#c6ff1f".into() },
        Profile { id: "work".into(), name: "Work".into(), color: "#7fd3ff".into() },
    ]
}

fn web_keys() -> Vec<WebKey> {
    let key = |id: &str, name: &str, sites: &[&str]| WebKey {
        id: id.into(),
        name: name.into(),
        sites: sites.iter().map(|s| s.to_string()).collect(),
        used_by: 1,
    };
    vec![
        key("work-web", "Work", &["claude.ai", "mail.google.com", "drive.google.com"]),
        key("hunggod", "hunggod", &["github.com"]),
        key("vanua", "Google (VanuaAcademy)", &["firebase.google.com", "cloud.google.com", "squarespace.com"]),
    ]
}

fn my_apps() -> Vec<Tile> {
    vec![Tile {
        id: "calendar".into(),
        name: "Calendar".into(),
        url: "https://calendar.google.com".into(),
        icon: wad_model::catalog::site_icon("calendar", Some("https://calendar.google.com"))
            .map(|p| internal::res_url(&p)),
        mine: true,
    }]
}

fn tiles(mine: bool) -> Vec<Tile> {
    let suggested = wad_model::catalog::WEB_APPS
        .iter()
        .map(|s| Tile {
            id: s.id.into(),
            name: s.name.into(),
            url: s.url.into(),
            icon: wad_model::catalog::site_icon(s.id, Some(s.url)).map(|p| internal::res_url(&p)),
            mine: false,
        })
        .collect();
    internal::merge_tiles(suggested, if mine { my_apps() } else { vec![] })
}

fn visit(title: &str, url: &str, secs: i64) -> HistoryEntry {
    HistoryEntry {
        id: 0,
        title: title.into(),
        url: url.into(),
        visit_count: 1,
        last_visit: Utc::now() - chrono::Duration::seconds(secs),
    }
}

/// A page's address and HTML, if `name` is a page.
fn page(name: &str) -> Option<(&'static str, String)> {
    let prefs = Preferences::default();
    let sites = vec![
        SiteCount { site: "x.com".into(), url: "https://x.com".into(), pages: 1, blocked: 7 },
        SiteCount { site: "github.com".into(), url: "https://github.com".into(), pages: 1, blocked: 0 },
        SiteCount { site: "docs.rs".into(), url: "https://docs.rs".into(), pages: 1, blocked: 0 },
    ];
    Some(match name {
        "newtab" | "newtab-menu" | "newtab-search" => {
            ("wadbrowser://newtab", internal::newtab_page(&prefs, &tiles(true)))
        }
        "newtab-plain" => {
            let prefs = Preferences { search_engine: wadbrowser_core::SearchEngine::Startpage, ..prefs };
            ("wadbrowser://newtab", internal::newtab_page(&prefs, &tiles(false)))
        }
        "privacy" => (
            "wadbrowser://privacy",
            internal::privacy_page(&Privacy {
                blocked: 1291,
                blocked_here: 7,
                block_ads: true,
                sites,
                ..Privacy::default()
            }),
        ),
        "privacy-private" | "privacy-clicks" => (
            "wadbrowser://privacy",
            internal::privacy_page(&Privacy {
                blocked: 1291,
                private: true,
                block_social: true,
                block_ads: true,
                allowed: vec!["old.reddit.com".into()],
                ..Privacy::default()
            }),
        ),
        "settings" => (
            "wadbrowser://settings",
            internal::settings_page(
                &prefs,
                &Settings {
                    block_social: false,
                    profiles: profiles()
                        .into_iter()
                        .map(|p| ProfileRow { id: p.id, name: p.name, color: p.color })
                        .collect(),
                    apps: my_apps(),
                    webkit_version: webkit_version(),
                },
            ),
        ),
        "history" => (
            "wadbrowser://history",
            internal::history_page(
                &[
                    visit("Docs.rs", "https://docs.rs", 5),
                    visit("X", "https://x.com", 12),
                    visit("GitHub", "https://github.com", 20),
                    visit("gtk4 - Rust", "https://docs.rs/gtk4/latest/gtk4/", 60 * 50),
                    visit("Inbox · Gmail", "https://mail.google.com/mail/u/0/#inbox", 60 * 60 * 5),
                    visit("Claude", "https://claude.ai/new", 60 * 60 * 30),
                ],
                false,
                Utc::now(),
            ),
        ),
        "history-private" => ("wadbrowser://history", internal::history_page(&[], true, Utc::now())),
        "about" => ("wadbrowser://about", internal::about_html(&webkit_version())),
        "error" => (
            "wadbrowser://error",
            internal::error_html(
                "This page didn’t load",
                "Couldn’t find the server: check the address, or your connection.",
                "https://nowhere.invalid/some/page",
            ),
        ),
        "blocked" => ("wadbrowser://blocked?site=x.com", internal::blocked_html("x.com")),
        "missing" => ("wadbrowser://nowhere", internal::missing_html("wadbrowser://nowhere")),
        "bookmarks" => ("wadbrowser://bookmarks", internal::bookmarks_html(&[])),
        _ => return None,
    })
}

fn webkit_version() -> String {
    format!(
        "{}.{}.{}",
        webkit6::functions::major_version(),
        webkit6::functions::minor_version(),
        webkit6::functions::micro_version()
    )
}

fn ready() {
    println!("ready");
}

fn ready_soon() {
    glib::timeout_add_local_once(Duration::from_millis(900), ready);
}

struct Chrome {
    window: adw::ApplicationWindow,
    content: gtk::Overlay,
    shield: Rc<Shield>,
    chip: ProfileChip,
    menu: gtk::Button,
}

/// The window: a mock header bar (back, forward, reload, the address, the
/// shield, the chip, ⋮) over the content.
fn chrome(app: &adw::Application, private: bool) -> Chrome {
    let window = adw::ApplicationWindow::builder().application(app).default_width(1280).default_height(800).build();
    window.add_css_class("gallery");
    let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    let header = gtk::Box::builder().spacing(8).css_classes(["gallery-header"]).build();
    let nav = gtk::Box::builder().spacing(2).build();
    for (icon, on) in [("arrow-left", true), ("arrow-right", false), ("refresh-cw", true)] {
        let b = gtk::Button::builder()
            .child(&wad_gtk::Icon::new(icon, 16))
            .css_classes(["wb-hb"])
            .valign(gtk::Align::Center)
            .sensitive(on)
            .build();
        nav.append(&b);
    }
    header.append(&nav);
    let url = gtk::Box::builder().hexpand(true).valign(gtk::Align::Center).css_classes(["gallery-url"]).build();
    let inner = gtk::Box::builder().spacing(7).halign(gtk::Align::Center).hexpand(true).build();
    inner.append(&wad_gtk::Icon::new("lock", 13));
    inner.append(&gtk::Label::new(Some("github.com")));
    url.append(&inner);
    url.set_margin_start(130);
    url.set_margin_end(130);
    header.append(&url);
    let shield = Shield::new(ShieldActions {
        set_blockers: Rc::new(|on| println!("set_blockers {on}")),
        set_social: Rc::new(|on| println!("set_social {on}")),
        open_dashboard: Rc::new(|| println!("open_dashboard")),
    });
    shield.update(&ShieldInfo { site: Some("github.com".into()), here: 0, total: 1284, allowed: false, social: false });
    header.append(shield.button());
    let chip = ProfileChip::new(&ChipInfo { name: "Personal".into(), color: "#c6ff1f".into(), private }, |_| {});
    header.append(chip.button());
    let menu = gtk::Button::builder()
        .child(&wad_gtk::Icon::new("ellipsis-vertical", 16))
        .css_classes(["wb-hb"])
        .valign(gtk::Align::Center)
        .tooltip_text("Main menu")
        .build();
    header.append(&menu);
    col.append(&header);
    let content = gtk::Overlay::builder().hexpand(true).vexpand(true).build();
    content.set_child(Some(&gtk::Box::builder().css_classes(["gallery"]).build()));
    col.append(&content);
    window.set_content(Some(&col));
    Chrome { window, content, shield, chip, menu }
}

fn show(app: &adw::Application, name: &str, fullscreen: bool) {
    let private = name.ends_with("-private") && !name.starts_with("privacy") && !name.starts_with("history");
    let c = chrome(app, private);
    if fullscreen {
        c.window.fullscreen();
    }
    c.window.present();

    if let Some((uri, html)) = page(name) {
        show_page(&c, uri, html, script(name));
        return;
    }
    let name = name.to_string();
    // Once it's laid out (popovers point at where things are).
    glib::timeout_add_local_once(Duration::from_millis(300), move || widget(&c, &name));
}

fn widget(c: &Chrome, name: &str) {
    let page_menu_at = |url: &str| {
        let m = PageMenu { url: url.into(), can_back: true, web_keys: web_keys(), ..PageMenu::default() };
        let a = PageActions { create_app: Rc::new(|| println!("create_app")), ..PageActions::default() };
        MenuPopover::show(&c.content, Place::Point(640.0, 404.0), page_menu(&m, &a), false)
    };
    match name {
        "page-menu" => {
            page_menu_at("https://github.com/HungGod");
        }
        "page-menu-signins" => {
            let m = page_menu_at("https://github.com/HungGod");
            glib::timeout_add_local_once(Duration::from_millis(200), move || {
                m.open("page-signins");
            });
        }
        "page-menu-cactus" => {
            page_menu_at("wadbrowser://privacy");
        }
        "main-menu" => {
            let a = MainActions { set_mode: Rc::new(|m| println!("set_mode {m:?}")), ..MainActions::default() };
            let m = MenuPopover::show(
                &c.menu,
                Place::Below,
                main_menu(&MainMenu { mode: Mode::Full, block_social: false, mode_off: None }, &a),
                false,
            );
            glib::timeout_add_local_once(Duration::from_millis(200), move || {
                m.open("menu-mode");
            });
        }
        "profile-menu" | "profile-menu-private" => {
            let m = ProfileMenu { profiles: profiles(), current: "default".into(), private: name.ends_with("private") };
            MenuPopover::show(c.chip.button(), Place::Below, profile_menu(&m, &ProfileActions::default()), false);
        }
        "chips" => {
            let row = gtk::Box::builder().spacing(12).halign(gtk::Align::Center).valign(gtk::Align::Center).build();
            for (name, color, private) in [
                ("Personal", "#c6ff1f", false),
                ("Work", "#7fd3ff", false),
                ("School", "#ffb36b", false),
                ("", "", true),
            ] {
                let chip = ProfileChip::new(&ChipInfo { name: name.into(), color: color.into(), private }, |_| {});
                row.append(chip.button());
            }
            c.content.add_overlay(&row);
        }
        "shield" | "shield-off" | "shield-cactus" => {
            let info = match name {
                "shield-off" => ShieldInfo {
                    site: Some("old.reddit.com".into()),
                    here: 0,
                    total: 1291,
                    allowed: true,
                    social: true,
                },
                "shield-cactus" => ShieldInfo { site: None, here: 0, total: 1291, allowed: false, social: false },
                _ => {
                    ShieldInfo { site: Some("github.com".into()), here: 0, total: 1284, allowed: false, social: false }
                }
            };
            c.shield.update(&info);
            c.shield.popover().set_autohide(false);
            c.shield.popup();
        }
        "create-app" | "create-app-done" | "create-app-error" => {
            let info = CreateAppInfo {
                title: "GitHub".into(),
                url: if name == "create-app-error" {
                    "wadbrowser://newtab".into()
                } else {
                    "https://github.com".into()
                },
                profiles: profiles(),
                profile: "default".into(),
            };
            let d = create_app(
                &c.window,
                &info,
                CreateAppActions {
                    create: Rc::new(|app, done| {
                        println!("create {app:?}");
                        done(Ok(()));
                    }),
                    open: Rc::new(|app| println!("open {app:?}")),
                },
            );
            if name != "create-app" {
                d.submit();
            }
        }
        "new-profile" => {
            new_profile(
                &c.window,
                &["#c6ff1f".into(), "#7fd3ff".into()],
                NewProfileActions { create: Rc::new(|n, col| println!("new profile {n} {col}")) },
            );
        }
        "add-signin" => {
            add_signin(&c.window, "github.com", Rc::new(|n| println!("save {n}")));
        }
        "edit-signins" => {
            edit_signins(&c.window, &web_keys(), "github.com", Rc::new(|k, s| println!("remove {s} from {k}")));
        }
        "permission" | "permission-notifications" => {
            let info = PermissionInfo {
                site: "meet.example.com".into(),
                kinds: if name == "permission" {
                    vec![PermissionKind::Camera, PermissionKind::Microphone]
                } else {
                    vec![PermissionKind::Notifications]
                },
                can_remember: true,
            };
            permission(&c.window, &info, Rc::new(|allow, remember| println!("allow {allow} remember {remember}")));
        }
        _ => {}
    }
    ready_soon();
}

/// What a page's run does once it's loaded (as a person would), if anything.
fn script(name: &str) -> Option<&'static str> {
    Some(match name {
        // Right-click the tile of yours.
        "newtab-menu" => {
            "const t = document.querySelector('.tile.mine'); const r = t.getBoundingClientRect();
             t.dispatchEvent(new MouseEvent('contextmenu', {bubbles: true, cancelable: true, clientX: r.x + r.width / 2, clientY: r.y + r.height / 2}));"
        }
        "newtab-search" => {
            "const f = document.querySelector('form.search'); f.querySelector('input').value = 'rust gtk4';
             f.requestSubmit();"
        }
        "privacy-clicks" => {
            "document.querySelector('[data-do=block-social]').click();
             document.querySelector('[data-do=blockers]').click();"
        }
        _ => return None,
    })
}

/// A page in a WebView, its `cactus` scheme served as the browser will.
fn show_page(c: &Chrome, uri: &'static str, html: String, script: Option<&'static str>) {
    let context = webkit6::WebContext::default().expect("WebKit's context");
    if let Some(security) = context.security_manager() {
        // Pages, their stylesheet and their fonts are all wadbrowser://, on different hosts.
        security.register_uri_scheme_as_cors_enabled("wadbrowser");
        security.register_uri_scheme_as_secure("wadbrowser");
    }
    let html = Rc::new(html);
    context.register_uri_scheme("wadbrowser", move |req| {
        let uri = req.uri().map(|u| u.to_string()).unwrap_or_default();
        let (bytes, mime): (glib::Bytes, &str) = match internal::resource(&uri) {
            Some((b, m)) => (glib::Bytes::from_static(b), m),
            None if uri.starts_with(internal::RES) => {
                let mut e = glib::Error::new(gio::IOErrorEnum::NotFound, "no such file");
                req.finish_error(&mut e);
                return;
            }
            None => (glib::Bytes::from_owned(html.as_bytes().to_vec()), "text/html"),
        };
        let len = bytes.len() as i64;
        let stream = gio::MemoryInputStream::from_bytes(&bytes);
        req.finish(&stream, len, Some(mime));
    });
    let content = webkit6::UserContentManager::new();
    content.register_script_message_handler("wadbrowser", None);
    let settings = webkit6::Settings::new();
    // The pages' console, on the gallery's stdout.
    settings.set_enable_write_console_messages_to_stdout(true);
    let view = webkit6::WebView::builder()
        .network_session(&webkit6::NetworkSession::new_ephemeral())
        .user_content_manager(&content)
        .settings(&settings)
        .hexpand(true)
        .vexpand(true)
        .build();
    let v = view.downgrade();
    content.connect_script_message_received(Some("wadbrowser"), move |_, value| {
        let page = v.upgrade().and_then(|v| v.uri()).unwrap_or_default();
        println!("page asks: {:?}", PageAction::from_page(&page, &value.to_str()));
    });
    view.set_background_color(&gdk::RGBA::new(0.055, 0.035, 0.098, 1.0));
    view.connect_load_changed(move |view, event| {
        if event != webkit6::LoadEvent::Finished {
            return;
        }
        if let Some(js) = script {
            view.evaluate_javascript(js, None, None, None::<&gio::Cancellable>, |r| {
                if let Err(e) = r {
                    eprintln!("script: {e}");
                }
            });
        }
        glib::timeout_add_local_once(Duration::from_millis(700), ready);
    });
    view.connect_load_failed(|_, _, uri, e| {
        eprintln!("{uri}: {e}");
        false
    });
    c.content.set_child(Some(&view));
    view.load_uri(uri);
}
