// The catalog: the same apps WadSpaces always served (wad-core's
// catalog.json, copied unmodified into data/), with v2's words where they
// changed: WadBrowser is a Cactus fork now, and the images are Fedora 44,
// not Debian. There's no package manager: these are all the apps there are.
// Also what opens what (Open with), the web apps people pick often, and
// Steam's store as the mock accounts have it.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::types::{AppId, Ext, SecretKind};

const CATALOG_JSON: &str = include_str!("../data/catalog.json");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSecret {
    pub kind: SecretKind,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Install {
    pub kind: String,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogApp {
    pub id: AppId,
    pub name: String,
    pub summary: String,
    pub description: String,
    pub category: String,
    pub icon: String,
    #[serde(default)]
    pub secret: Option<CatalogSecret>,
    pub install: Install,
    #[serde(default)]
    pub opens_folders: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogExtension {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub secret: Option<CatalogSecret>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogTool {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Deserialize)]
struct Raw {
    apps: Vec<CatalogApp>,
    extensions: Vec<CatalogExtension>,
    tools: Vec<CatalogTool>,
}

struct Catalog {
    apps: Vec<CatalogApp>,
    extensions: Vec<CatalogExtension>,
    tools: Vec<CatalogTool>,
}

/// Add-ons the catalog doesn't offer any more: Notion, Figma and Docs.rs for WadBrowser; Go for VS Code and the command line; Git LFS.
pub const RETIRED_ADDONS: [(AppId, &str); 6] = [
    (AppId::Wadbrowser, "notion"),
    (AppId::Wadbrowser, "figma"),
    (AppId::Wadbrowser, "docs-rs"),
    (AppId::Vscode, "golang.go"),
    (AppId::Terminal, "go"),
    (AppId::Terminal, "git-lfs"),
];

fn retired(app: AppId, id: &str) -> bool {
    RETIRED_ADDONS.iter().any(|&(a, i)| a == app && i == id)
}

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    // It's compiled in, and a test reads it: it parses.
    let raw: Raw = serde_json::from_str(CATALOG_JSON).expect("data/catalog.json is the catalog");
    let fedora = |kind: &str, source: &str| Install { kind: kind.to_string(), source: Some(source.to_string()) };
    let apps = raw
        .apps
        .into_iter()
        .map(|mut a| {
            match a.id {
                AppId::Wadbrowser => {
                    a.summary = "The browser in every wadspace, and every web app’s window".to_string();
                    a.description = "WadSpaces’ browser, a fork of Cactus (GTK4, WebKitGTK 6): tabs, web apps in windows of their own, Focus without an address bar, ad and tracker blocking, and a plugin that blocks all social media. Right-click a page to make it an app, add a saved sign-in, or start a new profile.".to_string();
                    a.install = fedora("builtin", "Comes with the Fedora 44 base image");
                }
                AppId::Lmms | AppId::Openshot => a.install = fedora("feature", "Fedora’s package"),
                AppId::Obs => a.install = fedora("feature", "Fedora’s package (RPM Fusion)"),
                AppId::Steam => a.install = fedora("feature", "RPM Fusion’s steam, 32-bit libraries"),
                AppId::Terminal => a.install = fedora("builtin", "Comes with the Fedora 44 base image"),
                _ => {}
            }
            a
        })
        .collect();
    let extensions = raw.extensions.into_iter().filter(|e| !retired(AppId::Vscode, &e.id)).collect();
    // v1's other command-line downloads (its images' features beyond the catalog's tools: claude-code.sh, hplip.sh), as tools here too, and Ori (OpenRouter's agent).
    let more = [
        ("claude-code", "Claude Code", "Anthropic’s claude command, from npm (adds Node.js)"),
        ("ori", "Ori", "OpenRouter’s command-line coding agent: Claude Code, Codex and more, through OpenRouter"),
        (
            "podman",
            "Podman",
            "podman, buildah and skopeo: build and push images from a wadspace (the WadOS one, the host image)",
        ),
        ("hplip", "HP printer tools", "hplip (hp-setup, hp-info) and the CUPS client commands"),
    ];
    let tools = raw
        .tools
        .into_iter()
        .filter(|t| !retired(AppId::Terminal, &t.id))
        .map(|mut t| {
            if t.id == "gcloud" {
                t.description = "gcloud, gsutil and bq from Google’s dnf repo".to_string();
            }
            t
        })
        .chain(more.map(|(id, name, description)| CatalogTool {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
        }))
        .collect();
    Catalog { apps, extensions, tools }
});

/// Every app, in the catalog's order.
pub fn apps() -> &'static [CatalogApp] {
    &CATALOG.apps
}

/// VS Code's extensions.
pub fn extensions() -> &'static [CatalogExtension] {
    &CATALOG.extensions
}

/// The command line's tools.
pub fn tools() -> &'static [CatalogTool] {
    &CATALOG.tools
}

/// An app's entry (the catalog has every app).
pub fn app(id: AppId) -> Option<&'static CatalogApp> {
    CATALOG.apps.iter().find(|a| a.id == id)
}

pub fn app_name(id: AppId) -> &'static str {
    app(id).map_or(id.as_str(), |a| a.name.as_str())
}

pub fn app_icon(id: AppId) -> &'static str {
    app(id).map_or("/icons/terminal.png", |a| a.icon.as_str())
}

pub fn extension(id: &str) -> Option<&'static CatalogExtension> {
    CATALOG.extensions.iter().find(|e| e.id == id)
}

pub fn tool(id: &str) -> Option<&'static CatalogTool> {
    CATALOG.tools.iter().find(|t| t.id == id)
}

// ------------------------------------------------------------ web apps

/// A web app people pick often (the Builder's suggestions; any address works).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Site {
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
    /// Its own icon, fetched from it once into public/icons/web/ (an SVG where the site has one).
    icon: &'static str,
}

pub const WEB_APPS: [Site; 10] = [
    Site { id: "github", name: "GitHub", url: "https://github.com", icon: "github.svg" },
    Site { id: "claude", name: "Claude", url: "https://claude.ai", icon: "claude.svg" },
    Site { id: "gmail", name: "Gmail", url: "https://mail.google.com", icon: "gmail.png" },
    Site {
        id: "google-firebase",
        name: "Google FireBase",
        url: "https://firebase.google.com",
        icon: "google-firebase.png",
    },
    Site { id: "google-cloud", name: "Google Cloud", url: "https://cloud.google.com", icon: "google-cloud.png" },
    Site { id: "google-drive", name: "Google Drive", url: "https://drive.google.com", icon: "google-drive.png" },
    Site { id: "squarespace", name: "Squarespace", url: "https://squarespace.com", icon: "squarespace.png" },
    Site { id: "youtube", name: "YouTube", url: "https://youtube.com", icon: "youtube.png" },
    Site { id: "calendar", name: "Calendar", url: "https://calendar.google.com", icon: "calendar.png" },
    Site { id: "chatgpt", name: "ChatGPT", url: "https://chatgpt.com", icon: "chatgpt.svg" },
];

pub fn site(id: &str) -> Option<&'static Site> {
    WEB_APPS.iter().find(|s| s.id == id)
}

/// A web address's host, lowercased, with its port if it has one ('github.com', 'localhost:8080'); none if it isn't one. 'https://' is assumed.
pub fn url_host(url: &str) -> Option<String> {
    let s = url.trim();
    let lower = s.to_ascii_lowercase();
    let rest = if lower.starts_with("https://") {
        &s[8..]
    } else if lower.starts_with("http://") {
        &s[7..]
    } else {
        s
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let host_port = authority.rsplit('@').next().unwrap_or(authority).to_ascii_lowercase();
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (host_port.as_str(), None),
    };
    let label_ok = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    if host.is_empty() || host.len() > 253 || !host.split('.').all(label_ok) {
        return None;
    }
    if let Some(p) = port
        && (p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    Some(host_port)
}

/// A web address's site: its host without 'www.' ('reddit.com').
pub fn host_of(url: &str) -> Option<String> {
    url_host(url).map(|h| h.strip_prefix("www.").map(str::to_string).unwrap_or(h))
}

/// A web app's icon: its site's (one of the catalog's, by id or address), else the favicon of the site it points at; none if it points nowhere.
pub fn site_icon(id: &str, url: Option<&str>) -> Option<String> {
    let host = url.and_then(host_of);
    let by_host = host.as_deref().and_then(|h| WEB_APPS.iter().find(|s| host_of(s.url).as_deref() == Some(h)));
    if let Some(s) = site(id).or(by_host) {
        return Some(format!("/icons/web/{}", s.icon));
    }
    host.map(|h| format!("https://{h}/favicon.ico"))
}

/// WadBrowser's Focus: tabs with no address bar.
pub fn focus() -> Ext {
    Ext::new(AppId::Wadbrowser, "focus").named("Focus")
}

/// Social media the browser's plugin blocks.
pub const SOCIAL: [&str; 10] = [
    "x.com",
    "twitter.com",
    "facebook.com",
    "instagram.com",
    "tiktok.com",
    "reddit.com",
    "threads.net",
    "linkedin.com",
    "snapchat.com",
    "bsky.app",
];

// ------------------------------------------------------------ add-ons

/// Apps whose variants are games.
pub const GAME_APPS: [AppId; 2] = [AppId::Dolphin, AppId::Steam];

/// A named variant of its app (a game, a profile), not an add-on from a catalog: any app's but VS Code's, the terminal's and the browser's.
pub fn is_named_app(app: AppId) -> bool {
    !matches!(app, AppId::Vscode | AppId::Terminal | AppId::Wadbrowser)
}

pub fn is_named(e: &Ext) -> bool {
    is_named_app(e.app)
}

/// An add-on's name.
pub fn ext_name(e: &Ext) -> String {
    if let Some(n) = &e.name {
        return n.clone();
    }
    match e.app {
        AppId::Vscode => extension(&e.id).map_or_else(|| e.id.clone(), |x| x.name.clone()),
        AppId::Terminal => tool(&e.id).map_or_else(|| e.id.clone(), |x| x.name.clone()),
        _ => e.id.clone(),
    }
}

/// What an add-on is ("VS Code extension", "Terminal tool", "Web app", "Game").
pub fn ext_kind(e: &Ext) -> &'static str {
    match e.app {
        AppId::Vscode => "VS Code extension",
        AppId::Terminal => "Terminal tool",
        _ if e.id == "focus" && e.app == AppId::Wadbrowser => "Browser mode",
        app if is_named_app(app) => {
            if GAME_APPS.contains(&app) {
                "Game"
            } else {
                "Variant"
            }
        }
        _ => "Web app",
    }
}

/// An add-on's secret, if it keeps one (Claude Code).
pub fn ext_secret(e: &Ext) -> Option<&'static CatalogSecret> {
    if e.app == AppId::Vscode { extension(&e.id).and_then(|x| x.secret.as_ref()) } else { None }
}

// ------------------------------------------------------------ what opens what

/// What each app opens: MIME types (`image/*` is any image) and file extensions (`.tmx`), in the order Open with offers them.
pub const OPENS: [(AppId, &[&str]); 10] = [
    (AppId::Krita, &["image/*", ".kra", ".psd", ".ora"]),
    (AppId::Libresprite, &["image/png", "image/gif", ".aseprite", ".ase"]),
    (AppId::Audacity, &["audio/*", ".wav", ".mp3", ".flac", ".ogg", ".aup3"]),
    (AppId::Lmms, &["audio/*", ".mmp", ".mmpz", ".mid", ".midi"]),
    (AppId::Openshot, &["video/*", ".osp"]),
    (
        AppId::Vscode,
        &[
            "text/*",
            "application/json",
            ".md",
            ".ts",
            ".js",
            ".py",
            ".rs",
            ".go",
            ".toml",
            ".yaml",
            ".yml",
            ".json",
            ".svg",
        ],
    ),
    (AppId::Obsidian, &["text/markdown", ".md"]),
    (AppId::Tiled, &[".tmx", ".tsx", ".tmj", ".tsj"]),
    (AppId::Dolphin, &[".iso", ".rvz", ".gcm", ".wbfs", ".ciso", ".wad"]),
    (AppId::Wadbrowser, &["application/pdf", "text/html", ".pdf", ".html", ".htm"]),
];

const MIME_BY_EXT: [(&str, &str); 19] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("svg", "image/svg+xml"),
    ("wav", "audio/wav"),
    ("mp3", "audio/mpeg"),
    ("flac", "audio/flac"),
    ("ogg", "audio/ogg"),
    ("mp4", "video/mp4"),
    ("mkv", "video/x-matroska"),
    ("webm", "video/webm"),
    ("mov", "video/quicktime"),
    ("md", "text/markdown"),
    ("txt", "text/plain"),
    ("pdf", "application/pdf"),
    ("json", "application/json"),
    ("html", "text/html"),
];

/// A file's extension, with its dot, lowercased ('.png'; '' if it has none).
fn ext_of(name: &str) -> String {
    name.rfind('.').map(|i| name[i..].to_lowercase()).unwrap_or_default()
}

/// A file's type: its own MIME, else from its extension (other machines' files have none).
pub fn mime_of(name: &str, mime: Option<&str>, folder: bool) -> String {
    if folder {
        return "inode/directory".to_string();
    }
    if let Some(m) = mime.filter(|m| !m.is_empty()) {
        return m.to_string();
    }
    let ext = ext_of(name);
    let ext = ext.strip_prefix('.').unwrap_or("");
    MIME_BY_EXT.iter().find(|(e, _)| *e == ext).map_or("application/octet-stream", |&(_, m)| m).to_string()
}

/// The apps that open a file, best first (a folder: the ones that open folders).
pub fn apps_for(name: &str, mime: Option<&str>, folder: bool) -> Vec<AppId> {
    if folder {
        return apps().iter().filter(|a| a.opens_folders).map(|a| a.id).collect();
    }
    let mime = mime_of(name, mime, false);
    let ext = ext_of(name);
    let fits = |p: &str| {
        if p.starts_with('.') {
            p == ext
        } else if let Some(prefix) = p.strip_suffix('*') {
            mime.starts_with(prefix)
        } else {
            p == mime
        }
    };
    OPENS.iter().filter(|(_, ps)| ps.iter().any(|p| fits(p))).map(|(id, _)| *id).collect()
}

// ------------------------------------------------------------ Steam's store

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    /// Its named variant's id ('stardew-valley').
    pub id: String,
    pub name: String,
    pub appid: u32,
    pub size_mb: u64,
    /// Its cover, as a picture's name.
    pub cover: String,
    /// Its expansions the account owns: Steam downloads them with it, and it starts with them (Isaac's Repentance).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dlc: Vec<String>,
}

static STEAM_GAMES: LazyLock<Vec<Game>> = LazyLock::new(|| {
    let g = |id: &str, name: &str, appid: u32, size_mb: u64, cover: &str, dlc: &[&str]| Game {
        id: id.to_string(),
        name: name.to_string(),
        appid,
        size_mb,
        cover: cover.to_string(),
        dlc: dlc.iter().map(|d| d.to_string()).collect(),
    };
    vec![
        g("stardew-valley", "Stardew Valley", 413_150, 520, "Aurora", &[]),
        g("overcooked-2", "Overcooked! 2", 728_880, 3400, "Acid Dusk", &[]),
        g("hades", "Hades", 1_145_360, 15_000, "Ember", &[]),
        g("celeste", "Celeste", 504_230, 1200, "Glacier", &[]),
        g("rocket-league", "Rocket League", 252_950, 25_000, "Neon Tide", &[]),
        g("hollow-knight", "Hollow Knight", 367_520, 9000, "Midnight", &[]),
        g("portal-2", "Portal 2", 620, 12_000, "Nebula", &[]),
        g("terraria", "Terraria", 105_600, 400, "Haze", &[]),
        g("binding-of-isaac-rebirth", "The Binding of Isaac: Rebirth", 250_900, 1100, "Ember", &["Repentance"]),
    ]
});

/// Steam games the mock accounts own.
pub fn steam_games() -> &'static [Game] {
    &STEAM_GAMES
}

pub fn steam_game(id: &str) -> Option<&'static Game> {
    STEAM_GAMES.iter().find(|g| g.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_has_every_app() {
        assert_eq!(apps().iter().map(|a| a.id).collect::<Vec<_>>(), AppId::ALL);
        assert_eq!(app_name(AppId::Vscode), "VS Code");
        assert_eq!(app_name(AppId::Terminal), "Command line");
        assert!(app(AppId::Wadbrowser).is_some_and(|a| a.description.contains("Cactus")));
        assert_eq!(
            app(AppId::Steam).and_then(|a| a.install.source.as_deref()),
            Some("RPM Fusion’s steam, 32-bit libraries")
        );
    }

    #[test]
    fn has_v1s_other_downloads_as_tools_and_not_what_it_dropped() {
        let tools: Vec<_> = tools().iter().map(|t| t.id.as_str()).collect();
        for id in ["rust", "claude-code", "ori", "podman", "hplip"] {
            assert!(tools.contains(&id), "{id}");
        }
        assert!(!tools.contains(&"go") && !tools.contains(&"git-lfs"));
        assert!(extension("golang.go").is_none());
        for id in ["notion", "figma", "docs-rs"] {
            assert!(site(id).is_none());
        }
        assert_eq!(
            tool("gcloud").map(|t| t.description.as_str()),
            Some("gcloud, gsutil and bq from Google’s dnf repo")
        );
    }

    #[test]
    fn knows_what_opens_what() {
        assert_eq!(apps_for("cover.png", Some("image/png"), false), [AppId::Krita, AppId::Libresprite]);
        assert_eq!(apps_for("take 2.flac", None, false), [AppId::Audacity, AppId::Lmms]);
        assert_eq!(apps_for("level.tmx", None, false), [AppId::Tiled]);
        assert_eq!(apps_for("Thesis", None, true), [AppId::Vscode]);
        assert_eq!(mime_of("notes.TXT", None, false), "text/plain");
    }

    #[test]
    fn web_addresses() {
        assert_eq!(url_host("https://www.GitHub.com/x?y").as_deref(), Some("www.github.com"));
        assert_eq!(host_of("www.reddit.com").as_deref(), Some("reddit.com"));
        assert_eq!(url_host("localhost:8080").as_deref(), Some("localhost:8080"));
        assert_eq!(url_host("not a site"), None);
        assert_eq!(url_host(""), None);
        assert_eq!(url_host("https://"), None);
        assert_eq!(site_icon("github", None).as_deref(), Some("/icons/web/github.svg"));
        assert_eq!(site_icon("mine", Some("https://mail.google.com/u/0")).as_deref(), Some("/icons/web/gmail.png"));
        assert_eq!(site_icon("mine", Some("example.org")).as_deref(), Some("https://example.org/favicon.ico"));
        assert_eq!(site_icon("mine", None), None);
    }

    #[test]
    fn add_ons_say_what_they_are() {
        assert_eq!(ext_name(&Ext::new(AppId::Vscode, "anthropic.claude-code")), "Claude Code");
        assert_eq!(ext_kind(&Ext::new(AppId::Terminal, "rust")), "Terminal tool");
        assert_eq!(ext_kind(&focus()), "Browser mode");
        assert_eq!(ext_kind(&Ext::new(AppId::Steam, "hades")), "Game");
        assert_eq!(ext_kind(&Ext::new(AppId::Krita, "inks")), "Variant");
        assert_eq!(ext_kind(&Ext::new(AppId::Wadbrowser, "gmail").at("https://mail.google.com")), "Web app");
        assert!(
            ext_secret(&Ext::new(AppId::Vscode, "anthropic.claude-code"))
                .is_some_and(|s| s.kind == SecretKind::VscodeClaudeCode)
        );
        assert_eq!(steam_game("celeste").map(|g| g.appid), Some(504_230));
    }
}
