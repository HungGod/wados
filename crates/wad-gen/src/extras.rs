//! A wadspace's extra folder: what it takes when it starts, never in its
//! image, so changing it never builds. wadd keeps it at
//! `<state_dir>/extra/<id>` ([`dir`]) and the unit mounts it read-only at
//! `/run/wadspaces-extra` ([`DIR`]), where the base's
//! `init-wadspaces-workspace` reads it (v1's files, in v1's shapes):
//!
//! - `webapps.json`: its web apps, `[{id, name, url}]` (their launchers;
//!   wadd puts each one's icon at `icons/<id>.png`);
//! - `layout.json`: its desktop's icons in order, and what opens at start
//!   (`wadspaces-layout`, `wadspaces-projects`);
//! - `signins.json`: the keys it mounts, what signs in with each, and the
//!   sites a web key signs in to (`wadspaces-signins`; WadBrowser reads its
//!   profiles from it);
//! - `projects.json`: its projects (`wadspaces-projects`);
//! - `wadbrowser.conf`: what links open in, with WadBrowser;
//! - its wallpaper ([`wallpaper_file`]), which wadd copies in.
//!
//! The extra folder isn't a secret's place: a key's files are in its own
//! folder, mounted beside it (`/run/wadspaces-signins/<id>`).

use serde::Serialize;
use sha2::{Digest, Sha256};
use wad_model::{AppId, KeyRef, ProjectKind, Secret, SecretKind, Wadspace, Wallpaper, catalog, look};

use crate::recipes::{self, Recipe};
use crate::{Mounts, mountable};

/// Where a wadspace sees its extra folder.
pub const DIR: &str = "/run/wadspaces-extra";
/// Where a wadspace sees each key it mounts: `<SIGNINS_ROOT>/<key id>`.
pub const SIGNINS_ROOT: &str = "/run/wadspaces-signins";
pub const WEBAPPS: &str = "webapps.json";
pub const LAYOUT: &str = "layout.json";
pub const SIGNINS: &str = "signins.json";
pub const PROJECTS: &str = "projects.json";
pub const WADBROWSER_CONF: &str = "wadbrowser.conf";
/// Web apps' icons: `icons/<web app id>.png` (wadd's).
pub const ICONS: &str = "icons";
/// What WadBrowser's own windows sign in as, in signins.json's `apps` (a web app's is its id).
pub const BROWSER_APP: &str = "@browser";

/// A wadspace's extra folder on the machine.
pub fn dir(state_dir: &str, id: &str) -> String {
    format!("{state_dir}/extra/{id}")
}

/// Its files as they are now: (name, contents), or none for one that goes
/// (no WadBrowser, no keys). wadd writes them before it starts.
pub fn files(ws: &Wadspace, keys: &[Secret], mounts: &Mounts) -> Vec<(&'static str, Option<String>)> {
    vec![
        (WEBAPPS, Some(webapps_json(ws))),
        (LAYOUT, Some(layout_json(ws))),
        (PROJECTS, Some(projects_json(ws, mounts))),
        (SIGNINS, signins_json(ws, keys)),
        (WADBROWSER_CONF, wadbrowser_conf(ws)),
    ]
}

fn pretty<T: Serialize>(v: &T) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default() + "\n"
}

fn is_false(b: &bool) -> bool {
    !*b
}

// ------------------------------------------------------------ web apps

/// A web app, as webapps.json has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebApp {
    pub id: String,
    pub name: String,
    pub url: String,
    /// The picture chosen for its icon, if one was (wadd makes `icons/<id>.png`).
    #[serde(skip)]
    pub icon_url: Option<String>,
}

/// Its web apps, with WadBrowser (each is a WadBrowser window).
pub fn webapps(ws: &Wadspace) -> Vec<WebApp> {
    if !ws.has_app(AppId::Wadbrowser) {
        return vec![];
    }
    ws.exts
        .iter()
        .filter(|e| e.app == AppId::Wadbrowser)
        .filter_map(|e| {
            let url = e.url.clone()?;
            Some(WebApp { id: e.id.clone(), name: catalog::ext_name(e), url, icon_url: e.icon_url.clone() })
        })
        .collect()
}

pub fn webapps_json(ws: &Wadspace) -> String {
    pretty(&webapps(ws))
}

// ------------------------------------------------------------ the desktop

#[derive(Serialize)]
struct Icon {
    #[serde(skip_serializing_if = "Option::is_none")]
    app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    desktop: Option<String>,
    label: String,
    #[serde(skip_serializing_if = "is_false")]
    autostart: bool,
}

#[derive(Serialize)]
struct Layout {
    version: u32,
    icons: Vec<Icon>,
}

/// The launcher an app's recipe installs (none: it isn't in the image).
pub fn launcher(app: AppId) -> Option<&'static str> {
    match recipes::app(app) {
        Recipe::Builtin { desktop } => Some(desktop),
        Recipe::Feature(f) => f.desktop,
        Recipe::Rpm(r) => r.desktop,
        Recipe::Todo(_) => None,
    }
}

fn icon(ws: &Wadspace, key: &str) -> Option<Icon> {
    let (kind, id) = key.split_once(':')?;
    let autostart = ws.look.autostart.iter().any(|a| a == key);
    let app = |app: String, desktop: String, label: String| Icon {
        app: Some(app),
        project: None,
        desktop: Some(desktop),
        label,
        autostart,
    };
    match kind {
        "app" => {
            let a = AppId::parse(id).filter(|a| ws.has_app(*a))?;
            Some(app(a.to_string(), launcher(a)?.to_string(), catalog::app_name(a).to_string()))
        }
        "web" if ws.has_app(AppId::Wadbrowser) => {
            let e = ws.ext(AppId::Wadbrowser, id).filter(|e| e.url.is_some())?;
            Some(app(format!("webapp-{id}"), format!("wadspaces-webapp-{id}.desktop"), catalog::ext_name(e)))
        }
        "project" => {
            let p = ws.look.projects.iter().find(|p| p.id == id)?;
            Some(Icon { app: None, project: Some(p.id.clone()), desktop: None, label: p.as_name.clone(), autostart })
        }
        // A game's or a named variant's: started by its app, a later phase's.
        _ => None,
    }
}

/// layout.json: its desktop's icons in its look's order (then anything it
/// has that the order doesn't name, as a new wadspace's are), each app's
/// with the launcher its recipe installs, and which open at start. An app
/// not in its image (not packaged yet) has no icon.
pub fn layout_json(ws: &Wadspace) -> String {
    let mut keys: Vec<String> = ws.look.layout.icons.clone();
    for k in look::default_icons(&ws.apps, &ws.exts, &ws.look.projects) {
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    let icons = keys.iter().filter_map(|k| icon(ws, k)).collect();
    pretty(&Layout { version: 1, icons })
}

/// wadbrowser.conf, with WadBrowser: links open in it, full or Focus (its
/// add-on: no address bar).
pub fn wadbrowser_conf(ws: &Wadspace) -> Option<String> {
    if !ws.has_app(AppId::Wadbrowser) {
        return None;
    }
    let focus = ws.ext(AppId::Wadbrowser, "focus").is_some_and(|e| e.url.is_none());
    let mode = if focus { "focus" } else { "full" };
    Some(format!(
        "# WadBrowser's settings for this wadspace, from its design (a\n\
         # user's own go in ~/.config/wadbrowser/wadbrowser.conf).\n\
         #\n\
         # What links open in: full (tabs and an address bar) or focus (no address bar).\n\
         default = {mode}\n"
    ))
}

/// The file its wallpaper is in, in the extra folder: a picture's, by its
/// kind (none for a colour, which the unit passes as it is, or none).
pub fn wallpaper_file(wp: &Wallpaper) -> Option<String> {
    let Wallpaper::Image { value } = wp else { return None };
    let ext = value.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let ext = match ext.as_str() {
        "jpeg" => "jpg",
        e @ ("png" | "jpg" | "webp" | "svg" | "gif") => e,
        _ => "png",
    };
    Some(format!("wallpaper.{ext}"))
}

// ------------------------------------------------------------ keys

/// A key it mounts, and what in it signs in with it: apps ('vscode'), and
/// for a web key, WadBrowser's windows ([`BROWSER_APP`]) and web apps (their ids).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignIn {
    pub key: KeyRef,
    pub apps: Vec<String>,
}

/// The keys it uses, once each, in order: its slots' (for what's in it),
/// then WadBrowser's groups'.
pub fn signins(ws: &Wadspace) -> Vec<SignIn> {
    let mut out: Vec<SignIn> = vec![];
    let mut add = |key: &KeyRef, app: String| match out.iter_mut().find(|s| s.key.id == key.id) {
        Some(s) if !s.apps.contains(&app) => s.apps.push(app),
        Some(_) => {}
        None => out.push(SignIn { key: key.clone(), apps: vec![app] }),
    };
    for (slot, key) in &ws.secrets.by_slot {
        let app = match slot.split_once(':') {
            Some(("app", a)) => AppId::parse(a).filter(|a| ws.has_app(*a)),
            Some(("ext", e)) => ws.exts.iter().find(|x| x.id == e && ws.has_app(x.app)).map(|x| x.app),
            _ => None,
        };
        if let Some(app) = app {
            add(key, app.to_string());
        }
    }
    if ws.has_app(AppId::Wadbrowser) {
        for g in &ws.secrets.groups {
            for m in &g.members {
                let app = match m.strip_prefix("web:") {
                    None if m == "browser" => Some(BROWSER_APP.to_string()),
                    Some(w) if ws.ext(AppId::Wadbrowser, w).is_some_and(|e| e.url.is_some()) => Some(w.to_string()),
                    _ => None,
                };
                if let Some(app) = app {
                    add(&g.key, app);
                }
            }
        }
    }
    out
}

/// Where an app keeps its sign-in, under the home folder, when its key
/// doesn't say (v1's): folders end in `/`. A web key is a WadBrowser
/// profile: no paths.
pub fn default_paths(kind: SecretKind) -> &'static [&'static str] {
    match kind {
        SecretKind::Web => &[],
        // Accounts and extensions' sign-ins (state.vscdb, readable with
        // --password-store=basic), settings, keybindings and snippets.
        SecretKind::Vscode => &[
            ".config/Code/User/globalStorage/",
            ".config/Code/User/settings.json",
            ".config/Code/User/keybindings.json",
            ".config/Code/User/snippets/",
        ],
        SecretKind::Obsidian => &[".config/obsidian/Local Storage/"],
        // Claude Code's CLI, its settings beside its login.
        SecretKind::VscodeClaudeCode => &[".claude/", ".claude.json"],
        SecretKind::Steam => &[".steam/", ".local/share/Steam/config/", ".local/share/Steam/userdata/"],
    }
}

#[derive(Serialize)]
struct SignInEntry {
    id: String,
    name: String,
    web: bool,
    apps: Vec<String>,
    dir: String,
    paths: Vec<String>,
    /// A web key's sites: a page on one of them, in any of WadBrowser's
    /// windows, opens in this key's profile.
    sites: Vec<String>,
}

/// signins.json's version: 2 added each key's `sites` (version 1's
/// readers, `wadspaces-signins` and WadBrowser, read it as they did).
pub const SIGNINS_VERSION: u32 = 2;

/// signins.json: each key it mounts, where, its paths (the key's own,
/// from `keys`, this machine's; else its kind's) and, for a web key, the
/// sites it signs in to (its record's). None without keys.
pub fn signins_json(ws: &Wadspace, keys: &[Secret]) -> Option<String> {
    let list = signins(ws);
    if list.is_empty() {
        return None;
    }
    let signins: Vec<SignInEntry> = list
        .into_iter()
        .map(|s| {
            let record = keys.iter().find(|k| k.id == s.key.id);
            let web = s.key.kind == SecretKind::Web;
            let paths = match record.map(|r| &r.paths).filter(|p| !p.is_empty() && !web) {
                Some(p) => p.clone(),
                None => default_paths(s.key.kind).iter().map(|p| p.to_string()).collect(),
            };
            let sites = record.filter(|_| web).map(|r| r.sites.clone()).unwrap_or_default();
            SignInEntry {
                dir: format!("{SIGNINS_ROOT}/{}", s.key.id),
                name: record.map_or_else(|| s.key.name.clone(), |r| r.name.clone()),
                id: s.key.id,
                web,
                apps: s.apps,
                paths,
                sites,
            }
        })
        .collect();
    Some(pretty(&serde_json::json!({ "version": SIGNINS_VERSION, "signins": signins })))
}

// ------------------------------------------------------------ projects

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectEntry<'a> {
    id: &'a str,
    kind: &'static str,
    mount: &'a str,
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    setup: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    setup_hash: Option<String>,
}

/// A project's one-time setup, as projects.json names it (it runs once per hash).
pub fn setup_hash(setup: &str) -> String {
    Sha256::digest(setup.as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// projects.json (version 2): each project at `~/Desktop/<its name>`, listed
/// only when `mounts` has it here. A GitHub one is wadd's clone, mounted
/// there (the wadspace fetches it and runs its setup); a copy is copied in
/// from wadd's copy of its files (`/run/wadspaces-copies/<id>`); a mount is
/// the machine's own folder or file (or another machine's, over Taildrive).
pub fn projects_json(ws: &Wadspace, mounts: &Mounts) -> String {
    let projects: Vec<ProjectEntry> = ws
        .look
        .projects
        .iter()
        .filter(|p| mounts.get(&p.id).is_some_and(|path| mountable(path, &p.as_name)))
        .map(|p| {
            let e = ProjectEntry {
                id: &p.id,
                kind: "copy",
                mount: &p.as_name,
                name: &p.as_name,
                repo: None,
                setup: None,
                setup_hash: None,
            };
            match &p.kind {
                ProjectKind::Github { repo, setup } => {
                    let setup = setup.as_deref().unwrap_or("");
                    ProjectEntry {
                        kind: "github",
                        repo: Some(repo),
                        setup: Some(setup),
                        setup_hash: Some(setup_hash(setup)),
                        ..e
                    }
                }
                ProjectKind::Copy { .. } => e,
                ProjectKind::Mount { .. } => ProjectEntry { kind: "mount", ..e },
            }
        })
        .collect();
    pretty(&serde_json::json!({ "version": 2, "projects": projects }))
}
