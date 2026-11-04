//! What the window's v2 widgets do: the ⋮ menu, the profile chip and its
//! menu, the page menu (right-click), the shield, and the dialogs they open
//! (Create app, New profile, Saved sign-ins, a permission). And what they
//! show that isn't the window's: the machine's web apps and saved sign-ins.
//!
//! On the machine those are wadd's (wadbrowser_webkit::wadd, off the main
//! thread). In a wadspace there's no wadd: its web apps are
//! `/run/wadspaces-extra/webapps.json` and its keys signins.json's, and
//! both change in the Builder, on the machine.

use std::cell::{OnceCell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use serde::Deserialize;
use wadbrowser_core::internal::{self, PageAction, Tile};
use wadbrowser_core::modes::{AppSpec, Mode, WindowSpec};
use wadbrowser_core::profiles::{self, ProfileKind};
use wadbrowser_core::{APP_ID, NEW_TAB_URI, site};
use wadbrowser_webkit::wadd::{self, Failure};
use wadbrowser_webkit::{Ask, shield};
use webkit6::WebView;
use webkit6::prelude::*;

use super::{WindowCtx, all_windows, build_window_as, current_webview, devtools, open_page, show_mode, show_toast};
use crate::chip::ChipInfo;
use crate::data::{Profile, WebKey};
use crate::dialogs::create_app::{CreateAppActions, CreateAppInfo, create_app};
use crate::dialogs::new_profile::{NewProfileActions, new_profile};
use crate::dialogs::permission::{PermissionInfo, permission};
use crate::dialogs::signins::{add_signin, edit_signins};
use crate::menu_popover::{MenuPopover, Place};
use crate::menus::{MainActions, MainMenu, PageActions, PageMenu, ProfileActions, ProfileMenu};
use crate::shield::ShieldInfo;

/// Where a wadspace's web apps are (wad-gen's extras).
const WEBAPPS_FILE: &str = "/run/wadspaces-extra/webapps.json";
const WEBAPP_ICONS: &str = "/run/wadspaces-extra/icons";

/// A web app the browser knows of besides the catalog's.
#[derive(Debug, Clone, PartialEq, Eq)]
struct App {
    id: String,
    name: String,
    url: String,
    /// Its icon for a page (`cactus://res/…`, a data URI, a favicon).
    icon: Option<String>,
    /// The machine's own, made from a page (it can go); a wadspace's can't.
    mine: bool,
}

thread_local! {
    /// wadd's here: the machine (none: a wadspace).
    static MACHINE: OnceCell<bool> = const { OnceCell::new() };
    /// The web apps and saved sign-ins, once they've come.
    static APPS: RefCell<Vec<App>> = const { RefCell::new(Vec::new()) };
    static KEYS: RefCell<Vec<WebKey>> = const { RefCell::new(Vec::new()) };
    static FETCHED: OnceCell<()> = const { OnceCell::new() };
}

/// On the machine (wadd's there), not in a wadspace.
fn machine() -> bool {
    MACHINE.with(|m| *m.get_or_init(|| wadbrowser_core::wadd::Wadd::here().is_some()))
}

const IN_A_WADSPACE_APPS: &str = "Add web apps in the Builder";
const IN_A_WADSPACE_KEYS: &str = "Saved sign-ins change on the machine";

/// A window's widgets wired: the shield kept up with the engine, the chip,
/// and the web apps and keys fetched (once).
pub(super) fn setup(ctx: &Rc<WindowCtx>) {
    let w = Rc::downgrade(ctx);
    shield::connect_changed(move |view| {
        let Some(ctx) = w.upgrade() else { return };
        if view.is_none() || view == current_webview(&ctx).as_ref() {
            sync_shield(&ctx);
        }
    });
    sync_chip(ctx);
    sync_shield(ctx);
    if FETCHED.with(|f| f.set(()).is_ok()) {
        refresh_apps();
        refresh_keys();
    }
}

// ------------------------------------------------------------ the shield and the chip

/// The shield for the page shown.
pub(super) fn sync_shield(ctx: &WindowCtx) {
    let Some(view) = current_webview(ctx) else { return };
    let st = shield::shield(&view);
    ctx.header.shield.update(&ShieldInfo {
        site: st.site,
        here: st.page.min(u32::MAX as u64) as u32,
        total: st.total,
        allowed: st.allowed,
        social: st.block_social,
    });
}

/// The shield's per-site toggle: the blockers on (or off) for this page's site.
pub(super) fn set_blockers(ctx: &WindowCtx, on: bool) {
    let Some(uri) = current_webview(ctx).and_then(|v| v.uri()) else { return };
    shield::set_site_allowed(&uri, !on);
}

/// The window's profile, as the chip shows it.
fn chip_info(ctx: &WindowCtx) -> ChipInfo {
    let id = ctx.session.profile.clone();
    if ctx.private {
        return ChipInfo { name: "Private".into(), color: "#ff3d81".into(), private: true };
    }
    let (name, color) = match ProfileKind::of(&id) {
        ProfileKind::SignIn(key) => {
            let name = wadbrowser_webkit::signins().signins.iter().find(|s| s.id == key).map(|s| s.name.clone());
            (name.filter(|n| !n.is_empty()).unwrap_or(key), "#ff3d81".to_string())
        }
        ProfileKind::Ephemeral(_) => ("This run".into(), "#f5f2fa".to_string()),
        _ => profiles::list_profiles(&ctx.state.paths)
            .into_iter()
            .find(|p| p.id == id)
            .map(|p| (p.name, p.color))
            .unwrap_or_else(|| (profiles::DEFAULT_NAME.into(), profiles::DEFAULT_COLOR.into())),
    };
    ChipInfo { name, color, private: false }
}

pub(super) fn sync_chip(ctx: &WindowCtx) {
    ctx.header.chip.update(&chip_info(ctx));
}

/// The profiles the chip lists.
fn profiles(ctx: &WindowCtx) -> Vec<Profile> {
    profiles::list_profiles(&ctx.state.paths)
        .into_iter()
        .map(|p| Profile { id: p.id, name: p.name, color: p.color })
        .collect()
}

/// A window of the browser's, as `profile` (None: private), shown.
fn open_window(ctx: &WindowCtx, spec: WindowSpec) {
    build_window_as(&ctx.app, ctx.state.clone(), spec, vec![NEW_TAB_URI.into()], 0).present();
}

/// The profile chip's menu, under it.
pub(super) fn profile_menu(ctx: &Rc<WindowCtx>, chip: &gtk::Button, autohide: bool) {
    let current = ctx.session.profile.clone();
    let m = ProfileMenu { profiles: profiles(ctx), current: current.clone(), private: ctx.private };
    let w = Rc::downgrade(ctx);
    let private = ctx.private;
    let a = ProfileActions {
        open_as: Rc::new({
            let w = w.clone();
            move |id: &str| {
                let Some(ctx) = w.upgrade() else { return };
                if id == current && !private {
                    return;
                }
                open_window(&ctx, WindowSpec::browser(Mode::Full, false, Some(id.to_string())));
            }
        }),
        new_profile: Rc::new({
            let w = w.clone();
            move || {
                if let Some(ctx) = w.upgrade() {
                    new_profile_dialog(&ctx);
                }
            }
        }),
        new_private_window: Rc::new(move || {
            if let Some(ctx) = w.upgrade() {
                open_window(&ctx, WindowSpec::browser(Mode::Full, true, None));
            }
        }),
    };
    MenuPopover::show(chip, Place::Below, crate::menus::profile_menu(&m, &a), autohide);
}

// ------------------------------------------------------------ the ⋮ menu

/// What a menu item does to its window.
type OnWindow = Rc<dyn Fn(&Rc<WindowCtx>)>;

pub(super) fn main_menu(ctx: &Rc<WindowCtx>, button: &gtk::Button, autohide: bool) -> Rc<MenuPopover> {
    let spec = ctx.spec.borrow().clone();
    let m = MainMenu {
        mode: spec.mode,
        block_social: shield::social_blocking(),
        mode_off: spec.signin.as_ref().map(|_| "A sign-in window stays one".to_string()),
    };
    let w = Rc::downgrade(ctx);
    let with = |f: OnWindow| {
        let w = w.clone();
        Rc::new(move || {
            if let Some(ctx) = w.upgrade() {
                f(&ctx);
            }
        }) as crate::menus::Run
    };
    let profile = spec.profile.clone();
    let a = MainActions {
        new_tab: with(Rc::new(|ctx| super::new_tab(ctx, NEW_TAB_URI))),
        new_window: with(Rc::new(move |ctx| {
            open_window(ctx, WindowSpec::browser(Mode::Full, false, profile.clone()));
        })),
        new_private_window: with(Rc::new(|ctx| open_window(ctx, WindowSpec::browser(Mode::Full, true, None)))),
        new_profile: with(Rc::new(new_profile_dialog)),
        set_mode: Rc::new({
            let w = w.clone();
            move |mode| {
                if let Some(ctx) = w.upgrade() {
                    set_mode(&ctx, mode);
                }
            }
        }),
        set_block_social: Rc::new(shield::set_social_blocking),
        open: Rc::new(move |uri: &str| {
            if let Some(ctx) = w.upgrade() {
                open_page(&ctx, uri);
            }
        }),
    };
    MenuPopover::show(button, Place::Below, crate::menus::main_menu(&m, &a), autohide)
}

/// This window as another kind (the spike's `setMode`): App takes the page's
/// site as its home and its name; leaving App, the window's the browser's.
pub(super) fn set_mode(ctx: &Rc<WindowCtx>, mode: Mode) {
    {
        let mut spec = ctx.spec.borrow_mut();
        if spec.mode == mode || spec.signin.is_some() {
            return;
        }
        if mode == Mode::App {
            let view = current_webview(ctx);
            let uri = view.as_ref().and_then(|v| v.uri()).map(String::from).unwrap_or_default();
            let home = if site::is_cactus(&uri) || site::host_of(&uri).is_empty() {
                spec.app.as_ref().map_or(NEW_TAB_URI.to_string(), |a| a.start.clone())
            } else {
                let scheme = uri.split_once("://").map_or("https", |(s, _)| s);
                format!("{scheme}://{}", site::authority_of(&uri))
            };
            let name = app_for(&home)
                .map(|a| a.name)
                .or_else(|| view.as_ref().and_then(|v| v.title()).map(String::from).filter(|t| !t.trim().is_empty()))
                .unwrap_or_else(|| site::bare_host(&site::host_of(&home)));
            spec.app = Some(AppSpec { app_id: APP_ID.into(), name, start: home, icon: None });
        } else {
            spec.app = None;
        }
        spec.mode = mode;
    }
    show_mode(ctx);
}

// ------------------------------------------------------------ the page menu

/// Right-click on a page: its menu at `at` (the view's pixels).
pub(super) fn page_menu(ctx: &Rc<WindowCtx>, view: &WebView, at: (f64, f64), autohide: bool) -> Rc<MenuPopover> {
    let uri = view.uri().map(String::from).unwrap_or_default();
    let host = site::bare_host(&site::host_of(&uri));
    let on_machine = machine();
    let m = PageMenu {
        url: uri.clone(),
        can_back: view.can_go_back(),
        can_forward: view.can_go_forward(),
        web_keys: KEYS.with_borrow(|k| k.clone()),
        can_inspect: devtools(),
        create_app_off: (!on_machine).then(|| IN_A_WADSPACE_APPS.to_string()),
        keys_off: (!on_machine).then(|| IN_A_WADSPACE_KEYS.to_string()),
    };
    let (w, v) = (Rc::downgrade(ctx), view.downgrade());
    let on_view = |f: fn(&WebView)| {
        let v = v.clone();
        Rc::new(move || {
            if let Some(v) = v.upgrade() {
                f(&v);
            }
        }) as crate::menus::Run
    };
    let a = PageActions {
        back: on_view(|v| v.go_back()),
        forward: on_view(|v| v.go_forward()),
        reload: on_view(|v| v.reload()),
        inspect: on_view(|v| {
            if let Some(i) = v.inspector() {
                i.show();
            }
        }),
        create_app: Rc::new({
            let (w, v) = (w.clone(), v.clone());
            move || {
                if let (Some(ctx), Some(v)) = (w.upgrade(), v.upgrade()) {
                    create_app_dialog(&ctx, &v);
                }
            }
        }),
        use_key: Rc::new({
            let (w, host) = (w.clone(), host.clone());
            move |k: &WebKey| {
                let (w, host, id, name) = (w.clone(), host.clone(), k.id.clone(), k.name.clone());
                wadd::call(
                    move |d| d.save_web_key("", &host, Some(&id)),
                    move |r| {
                        let Some(ctx) = w.upgrade() else { return };
                        match r {
                            Ok(_) => show_toast(&ctx, &format!("This site signs in with {name}")),
                            Err(e) => said(&ctx, &e),
                        }
                        refresh_keys();
                    },
                );
            }
        }),
        add_key: Rc::new({
            let (w, host) = (w.clone(), host.clone());
            move || {
                let Some(ctx) = w.upgrade() else { return };
                let (w, h) = (w.clone(), host.clone());
                add_signin(
                    &ctx.window,
                    &host,
                    Rc::new(move |name: &str| {
                        let (w, h, name) = (w.clone(), h.clone(), name.to_string());
                        wadd::call(
                            move |d| d.save_web_key(&name, &h, None),
                            move |r| {
                                if let (Some(ctx), Err(e)) = (w.upgrade(), &r) {
                                    said(&ctx, e);
                                }
                                refresh_keys();
                            },
                        );
                    }),
                );
            }
        }),
        edit_keys: Rc::new({
            let (w, host) = (w.clone(), host.clone());
            move || {
                let Some(ctx) = w.upgrade() else { return };
                let keys = KEYS.with_borrow(|k| k.clone());
                let w = w.clone();
                edit_signins(
                    &ctx.window,
                    &keys,
                    &host,
                    Rc::new(move |key: &str, site: &str| {
                        let (w, key, site) = (w.clone(), key.to_string(), site.to_string());
                        wadd::call(
                            move |d| d.remove_site(&key, &site),
                            move |r| {
                                if let (Some(ctx), Err(e)) = (w.upgrade(), &r) {
                                    said(&ctx, e);
                                }
                                refresh_keys();
                            },
                        );
                    }),
                );
            }
        }),
        new_profile: Rc::new(move || {
            if let Some(ctx) = w.upgrade() {
                new_profile_dialog(&ctx);
            }
        }),
    };
    MenuPopover::show(view, Place::Point(at.0, at.1), crate::menus::page_menu(&m, &a), autohide)
}

/// wadd's answer that something didn't work, in a toast.
fn said(ctx: &WindowCtx, e: &Failure) {
    show_toast(ctx, &e.message);
}

// ------------------------------------------------------------ the dialogs

/// "Create app from this page": made by wadd (`assets.webApp`); Open it
/// opens its window (an `--app` one).
fn create_app_dialog(ctx: &Rc<WindowCtx>, view: &WebView) -> crate::dialogs::create_app::CreateAppDialog {
    let uri = view.uri().map(String::from).unwrap_or_default();
    let title = view.title().map(String::from).filter(|t| !t.trim().is_empty());
    let host = site::bare_host(&site::host_of(&uri));
    let current = if ctx.private { profiles::DEFAULT.to_string() } else { ctx.session.profile.clone() };
    let info = CreateAppInfo { title: title.unwrap_or(host), url: uri, profiles: profiles(ctx), profile: current };
    let made: Rc<RefCell<Option<String>>> = Rc::default();
    let w = Rc::downgrade(ctx);
    let m = made.clone();
    let actions = CreateAppActions {
        create: Rc::new(move |app, done| {
            let m = m.clone();
            wadd::call(
                move |d| d.create_web_app(&app.name, &app.url),
                move |r| match r {
                    Ok(id) => {
                        *m.borrow_mut() = Some(id);
                        refresh_apps();
                        done(Ok(()));
                    }
                    Err(e) => done(Err(e.message)),
                },
            );
        }),
        open: Rc::new(move |app| {
            let Some(ctx) = w.upgrade() else { return };
            let id = made.borrow().clone().unwrap_or_else(|| site::bare_host(&site::host_of(&app.url)));
            open_app(&ctx, &id, &app.name, &app.url, Some(app.profile.clone()));
        }),
    };
    create_app(&ctx.window, &info, actions)
}

/// "New profile…": made, then a window of it.
fn new_profile_dialog(ctx: &Rc<WindowCtx>) {
    let taken: Vec<String> = profiles(ctx).into_iter().map(|p| p.color).collect();
    let w = Rc::downgrade(ctx);
    new_profile(
        &ctx.window,
        &taken,
        NewProfileActions {
            create: Rc::new(move |name, color| {
                let Some(ctx) = w.upgrade() else { return };
                match profiles::create_profile(&ctx.state.paths, name, color) {
                    Ok(id) => open_window(&ctx, WindowSpec::browser(Mode::Full, false, Some(id))),
                    Err(e) => show_toast(&ctx, &format!("The profile wasn’t made: {e}")),
                }
            }),
        },
    );
}

/// A web app's window (an `--app` one): `id` its id, at `url`.
fn open_app(ctx: &WindowCtx, id: &str, name: &str, url: &str, profile: Option<String>) {
    let app = AppSpec {
        app_id: format!("{}{id}", wadbrowser_core::cli::WEBAPP_PREFIX),
        name: name.to_string(),
        start: url.to_string(),
        icon: None,
    };
    let profile = profile.filter(|p| !profiles::is_private(p));
    build_window_as(&ctx.app, ctx.state.clone(), WindowSpec::web_app(app, false, profile), vec![], 0).present();
}

/// A site asking for the camera, a location…: the dialog, answered once.
pub(super) fn ask_permission(parent: Option<&adw::ApplicationWindow>, ask: Ask) {
    let info = PermissionInfo { site: ask.host.clone(), kinds: ask.kinds.clone(), can_remember: ask.can_remember };
    let ask = Rc::new(RefCell::new(Some(ask)));
    let on: gtk::Widget = match parent {
        Some(p) => p.clone().upcast(),
        None => return,
    };
    permission(
        &on,
        &info,
        Rc::new(move |allow, remember| {
            if let Some(ask) = ask.borrow_mut().take() {
                ask.answer(allow, remember);
            }
        }),
    );
}

// ------------------------------------------------------------ what pages ask (views.rs)

/// What a `cactus://` page asked that's the chrome's: a new profile, a web
/// app's window, a web app of yours removed. False: not one of these.
pub(super) fn page_action(ctx: &Rc<WindowCtx>, action: &PageAction) -> bool {
    match action {
        PageAction::NewProfile => new_profile_dialog(ctx),
        PageAction::OpenApp { id } => match app_by_id(id) {
            Some(a) => open_app(ctx, &a.id, &a.name, &a.url, None),
            None => show_toast(ctx, "That web app isn’t here any more"),
        },
        PageAction::RemoveApp { id } => {
            if !machine() {
                show_toast(ctx, IN_A_WADSPACE_APPS);
                return true;
            }
            let (w, id) = (Rc::downgrade(ctx), id.clone());
            wadd::call(
                move |d| d.op(wad_model::Op::RemoveWebApp { id }).map(|o| o.toast),
                move |r| {
                    if let Some(ctx) = w.upgrade() {
                        match r {
                            Ok(Some(t)) => show_toast(&ctx, &t),
                            Ok(None) => {}
                            Err(e) => said(&ctx, &e),
                        }
                    }
                    refresh_apps();
                },
            );
        }
        _ => return false,
    }
    true
}

// ------------------------------------------------------------ web apps and keys

/// The catalog's web apps, as tiles (their icons the pages' own copies).
fn catalog() -> Vec<Tile> {
    wad_model::catalog::WEB_APPS
        .iter()
        .map(|s| Tile {
            id: s.id.into(),
            name: s.name.into(),
            url: s.url.into(),
            icon: wad_model::catalog::site_icon(s.id, Some(s.url)).map(|p| internal::res_url(&p)),
            mine: false,
        })
        .collect()
}

/// The new tab's tiles: the catalog's, then the machine's (or the
/// wadspace's) web apps; one you made of a catalog site shows once.
pub(super) fn newtab_tiles() -> Vec<Tile> {
    let apps = APPS.with_borrow(|a| a.clone());
    let theirs: Vec<String> = apps.iter().filter(|a| !a.mine).map(|a| a.id.clone()).collect();
    let tiles = apps.into_iter().map(|a| Tile { id: a.id, name: a.name, url: a.url, icon: a.icon, mine: a.mine });
    let mut all = internal::merge_tiles(catalog(), tiles.collect());
    for t in &mut all {
        if theirs.contains(&t.id) {
            t.mine = false;
        }
    }
    all
}

/// Settings' web apps: the machine's of yours, or the wadspace's.
pub(super) fn settings_apps() -> Vec<Tile> {
    APPS.with_borrow(|a| {
        a.iter()
            .map(|a| Tile {
                id: a.id.clone(),
                name: a.name.clone(),
                url: a.url.clone(),
                icon: a.icon.clone(),
                mine: a.mine,
            })
            .collect()
    })
}

/// A web app by id: yours, the wadspace's, or the catalog's.
fn app_by_id(id: &str) -> Option<App> {
    APPS.with_borrow(|a| a.iter().find(|a| a.id == id).cloned()).or_else(|| {
        catalog().into_iter().find(|t| t.id == id).map(|t| App {
            id: t.id,
            name: t.name,
            url: t.url,
            icon: t.icon,
            mine: false,
        })
    })
}

/// The web app (yours, the wadspace's, the catalog's) a URL's site is.
fn app_for(url: &str) -> Option<App> {
    let host = site::bare_host(&site::host_of(url));
    let mut all = APPS.with_borrow(|a| a.clone());
    all.extend(catalog().into_iter().map(|t| App { id: t.id, name: t.name, url: t.url, icon: t.icon, mine: false }));
    all.into_iter().find(|a| site::bare_host(&site::host_of(&a.url)) == host)
}

/// A web app's icon for a page: the catalog's for its site, else its site's favicon.
fn icon_for(id: &str, url: &str) -> Option<String> {
    wad_model::catalog::site_icon(id, Some(url)).map(|p| if p.starts_with('/') { internal::res_url(&p) } else { p })
}

#[derive(Deserialize)]
struct WadspaceApp {
    id: String,
    name: String,
    url: String,
}

/// A wadspace's web apps (webapps.json), their icons as data URIs.
fn read_wadspace_apps() -> Vec<App> {
    let path = std::env::var("WADBROWSER_WEBAPPS").unwrap_or_else(|_| WEBAPPS_FILE.to_string());
    let Ok(text) = std::fs::read_to_string(&path) else { return vec![] };
    let list: Vec<WadspaceApp> = match serde_json::from_str(&text) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("{path}: {e}");
            return vec![];
        }
    };
    let icons = std::path::Path::new(&path).parent().map(|d| d.join("icons")).unwrap_or(WEBAPP_ICONS.into());
    list.into_iter()
        .map(|a| {
            let png = std::fs::read(icons.join(format!("{}.png", a.id))).ok();
            let icon = png
                .map(|b| format!("data:image/png;base64,{}", glib::base64_encode(&b)))
                .or_else(|| icon_for(&a.id, &a.url));
            App { id: a.id, name: a.name, url: a.url, icon, mine: false }
        })
        .collect()
}

/// The web apps again: wadd's (the machine's), or the wadspace's file's;
/// open new tabs and settings pages show them when they've come.
fn refresh_apps() {
    let done = |apps: Vec<App>| {
        let changed = APPS.with_borrow_mut(|a| {
            let changed = *a != apps;
            *a = apps;
            changed
        });
        if changed {
            reload_pages(&["cactus://newtab", "cactus://home", "cactus://settings"]);
        }
    };
    if machine() {
        wadd::call(
            |d| d.web_apps(),
            move |r| match r {
                Ok(list) => done(
                    list.into_iter()
                        .map(|a| App { icon: icon_for(&a.id, &a.url), id: a.id, name: a.name, url: a.url, mine: true })
                        .collect(),
                ),
                Err(e) if e.is_unreachable() => {}
                Err(e) => tracing::warn!("wadd's web apps: {e}"),
            },
        );
    } else {
        let (tx, rx) = async_channel::bounded(1);
        let spawned = std::thread::Builder::new().name("wadbrowser-webapps".into()).spawn(move || {
            let _ = tx.send_blocking(read_wadspace_apps());
        });
        if spawned.is_ok() {
            glib::spawn_future_local(async move {
                if let Ok(apps) = rx.recv().await {
                    done(apps);
                }
            });
        }
    }
}

/// The saved sign-ins again: wadd's web keys, or this wadspace's (signins.json).
fn refresh_keys() {
    if machine() {
        wadd::call(
            |d| d.web_keys(),
            |r| match r {
                Ok(keys) => KEYS.with_borrow_mut(|k| {
                    *k = keys
                        .into_iter()
                        .map(|s| WebKey { id: s.id, name: s.name, sites: s.sites, used_by: s.used_by.len() as u32 })
                        .collect();
                }),
                Err(e) if e.is_unreachable() => {}
                Err(e) => tracing::warn!("wadd's keys: {e}"),
            },
        );
    } else {
        let keys = wadbrowser_webkit::signins()
            .signins
            .iter()
            .filter(|s| s.web)
            .map(|s| WebKey {
                id: s.id.clone(),
                name: if s.name.is_empty() { s.id.clone() } else { s.name.clone() },
                sites: s.sites.clone(),
                used_by: 0,
            })
            .collect();
        KEYS.with_borrow_mut(|k| *k = keys);
    }
}

/// Pages at these addresses, in every window, drawn again.
fn reload_pages(prefixes: &[&str]) {
    for ctx in all_windows() {
        for v in super::views(&ctx) {
            let uri = v.uri().map(String::from).unwrap_or_default();
            if prefixes.iter().any(|p| uri.starts_with(p)) {
                v.reload();
            }
        }
    }
}

// ------------------------------------------------------------ screenshots

/// Screenshots' hook (`WADBROWSER_SHOT`): once a window's first page has
/// loaded, what it names opened as if clicked, and kept open (there's no
/// click to close it on): `main-menu`, `main-menu-mode`, `profile-menu`,
/// `page-menu`, `page-menu-signins`, `shield`, `create-app`, `new-profile`,
/// `add-signin`, `edit-signins`.
pub(super) fn shot(ctx: &Rc<WindowCtx>) {
    let Ok(what) = std::env::var("WADBROWSER_SHOT") else { return };
    let Some(view) = current_webview(ctx) else { return };
    let host = site::bare_host(&site::host_of(&view.uri().unwrap_or_default()));
    match what.as_str() {
        "main-menu" => drop(main_menu(ctx, &ctx.header.menu, false)),
        "main-menu-mode" => {
            let m = main_menu(ctx, &ctx.header.menu, false);
            glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                let _ = m.open("menu-mode");
            });
        }
        "profile-menu" => profile_menu(ctx, ctx.header.chip.button(), false),
        "page-menu" => drop(page_menu(ctx, &view, (640.0, 250.0), false)),
        "page-menu-signins" => {
            let m = page_menu(ctx, &view, (640.0, 250.0), false);
            glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                let _ = m.open("page-signins");
            });
        }
        "shield" => {
            ctx.header.shield.popover().set_autohide(false);
            ctx.header.shield.popup();
        }
        "create-app" => drop(create_app_dialog(ctx, &view)),
        "new-profile" => new_profile_dialog(ctx),
        "add-signin" => drop(add_signin(&ctx.window, &host, Rc::new(|_: &str| {}))),
        "edit-signins" => {
            drop(edit_signins(&ctx.window, &KEYS.with_borrow(|k| k.clone()), &host, Rc::new(|_: &str, _: &str| {})))
        }
        other => tracing::warn!("WADBROWSER_SHOT={other}: nothing by that name"),
    }
}
