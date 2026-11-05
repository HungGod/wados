//! The browser's own `cactus://` pages, as the v2 design draws them (the
//! spike's `src/apps/browser/pages/`): the new tab, Privacy, Settings,
//! History and About, and the pages that say why there's no page (a load
//! that failed, a social site blocked, a `cactus://` page that doesn't
//! exist).
//!
//! Each page is a function of the plain data it shows, and links the files
//! the pages share, served at `cactus://res/<file>` ([`resource`]): the
//! stylesheet, its script, the fonts and the catalog's site icons. What a
//! page asks of the browser (a toggle, "Remove") comes back through the
//! page's "cactus" script message handler as a [`PageAction`].

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::prefs::{Preferences, SearchEngine};
use crate::project::{CACTUS_COMMIT, CACTUS_LICENSE, CACTUS_REPO, CACTUS_VERSION};
use crate::store::{Bookmark, DownloadRecord, HistoryEntry};

pub fn is_internal(uri: &str) -> bool {
    uri.starts_with("cactus://") || uri.starts_with("about:")
}

pub fn map_about(uri: &str) -> Option<&'static str> {
    match uri {
        "about:blank" => Some("cactus://newtab"),
        "about:home" => Some("cactus://newtab"),
        "about:preferences" | "about:settings" => Some("cactus://settings"),
        _ => None,
    }
}

// ------------------------------------------------------------ the shared files

/// Where the shared files are: `cactus://res/<file>`.
pub const RES: &str = "cactus://res/";

/// A shared file's address (`"pages.css"` → `cactus://res/pages.css`).
pub fn res_url(path: &str) -> String {
    format!("{RES}{}", path.trim_start_matches('/'))
}

/// The pages' shared files: (path under `cactus://res/`, bytes, MIME type).
static RESOURCES: &[(&str, &[u8], &str)] = &[
    ("pages.css", include_bytes!("../res/pages.css"), "text/css"),
    ("pages.js", include_bytes!("../res/pages.js"), "text/javascript"),
    ("fonts/Inter-Variable.ttf", include_bytes!("../../../assets/fonts/Inter-Variable.ttf"), "font/ttf"),
    ("fonts/SpaceGrotesk-Variable.ttf", include_bytes!("../../../assets/fonts/SpaceGrotesk-Variable.ttf"), "font/ttf"),
    ("icons/web/calendar.png", include_bytes!("../../../assets/icons/web/calendar.png"), "image/png"),
    ("icons/web/chatgpt.svg", include_bytes!("../../../assets/icons/web/chatgpt.svg"), "image/svg+xml"),
    ("icons/web/claude.svg", include_bytes!("../../../assets/icons/web/claude.svg"), "image/svg+xml"),
    ("icons/web/docs-rs.png", include_bytes!("../../../assets/icons/web/docs-rs.png"), "image/png"),
    ("icons/web/figma.svg", include_bytes!("../../../assets/icons/web/figma.svg"), "image/svg+xml"),
    ("icons/web/github.svg", include_bytes!("../../../assets/icons/web/github.svg"), "image/svg+xml"),
    ("icons/web/gmail.png", include_bytes!("../../../assets/icons/web/gmail.png"), "image/png"),
    ("icons/web/google-cloud.png", include_bytes!("../../../assets/icons/web/google-cloud.png"), "image/png"),
    ("icons/web/google-drive.png", include_bytes!("../../../assets/icons/web/google-drive.png"), "image/png"),
    ("icons/web/google-firebase.png", include_bytes!("../../../assets/icons/web/google-firebase.png"), "image/png"),
    ("icons/web/notion.png", include_bytes!("../../../assets/icons/web/notion.png"), "image/png"),
    ("icons/web/squarespace.png", include_bytes!("../../../assets/icons/web/squarespace.png"), "image/png"),
    ("icons/web/youtube.png", include_bytes!("../../../assets/icons/web/youtube.png"), "image/png"),
];

/// A shared file, for the `cactus` scheme to serve: its bytes and MIME type.
/// `path` is what follows `cactus://res/` (a leading `/`, or the whole
/// address, is fine too); a query or fragment is ignored.
pub fn resource(path: &str) -> Option<(&'static [u8], &'static str)> {
    let path = path.strip_prefix(RES).unwrap_or(path);
    let path = path.split(['?', '#']).next().unwrap_or_default().trim_start_matches('/');
    RESOURCES.iter().find(|(p, ..)| *p == path).map(|(_, bytes, mime)| (*bytes, *mime))
}

// ------------------------------------------------------------ what a page asks of the browser

/// What a `cactus://` page asks the browser to do. Its script posts JSON to
/// the "cactus" script message handler (`{"do": "block-social", "on": true}`);
/// read it with [`PageAction::from_page`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "do", rename_all = "kebab-case")]
pub enum PageAction {
    /// The new tab's search box: what was typed (an address or words), to
    /// open as the address bar would.
    Go { to: String },
    /// "Block all social media", on or off.
    BlockSocial { on: bool },
    /// The blockers on or off for a site: Privacy's "Block again" (on), the
    /// blocked page's "Turn the blocker off for this site" (off).
    Blockers { site: String, on: bool },
    /// Settings' search engine: an id of [`crate::search::BUILTIN_ENGINES`]
    /// (see [`crate::search::engine_from_id`]).
    SearchEngine { id: String },
    /// "New profile…".
    NewProfile,
    /// A web app of yours, in a window of its own.
    OpenApp { id: String },
    /// A web app of yours, removed.
    RemoveApp { id: String },
}

impl PageAction {
    /// A message a page posted, if it's one of ours.
    pub fn parse(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }

    /// A message posted by the page at `page_uri`: only the browser's own
    /// pages may ask (the handler is there for every page a view shows).
    pub fn from_page(page_uri: &str, json: &str) -> Option<Self> {
        if !page_uri.starts_with("cactus://") {
            return None;
        }
        Self::parse(json)
    }
}

// ------------------------------------------------------------ the new tab

/// A web app on the new tab page: one of the catalog's, or one of yours
/// (made from a page).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    /// Its id (the catalog's `github`, or yours): picks its initial's colour, and what Remove removes.
    pub id: String,
    pub name: String,
    pub url: String,
    /// Its site's icon: a `cactus://res/icons/web/…` address ([`res_url`]),
    /// a `data:image/…` URI or a file. None: its initial.
    pub icon: Option<String>,
    /// Made from a page: right-click for Open in its own window and Remove.
    pub mine: bool,
}

/// The new tab's tiles: the catalog's (`suggested`), then yours; a
/// suggested one you made your own (same site) shows once, as yours.
pub fn merge_tiles(suggested: Vec<Tile>, mine: Vec<Tile>) -> Vec<Tile> {
    let hosts: Vec<String> = mine.iter().map(|t| bare_host(&t.url)).collect();
    let mut tiles: Vec<Tile> = suggested.into_iter().filter(|t| !hosts.contains(&bare_host(&t.url))).collect();
    tiles.extend(mine.into_iter().map(|t| Tile { mine: true, ..t }));
    tiles
}

/// The search engine's name ("DuckDuckGo").
pub fn engine_name(engine: &SearchEngine) -> &str {
    match engine {
        SearchEngine::DuckDuckGo => "DuckDuckGo",
        SearchEngine::Startpage => "Startpage",
        SearchEngine::Wikipedia => "Wikipedia",
        SearchEngine::Custom { name, .. } => name,
    }
}

/// The preferred engine's search address with `%s` for the words, as
/// [`Preferences::search_url`] makes it.
fn search_template(prefs: &Preferences) -> String {
    // A probe the query encoding keeps as it is, swapped for %s.
    prefs.search_url("cactusquery").replace("cactusquery", "%s")
}

/// `cactus://newtab`: the Cactus mark, a search box (the preferred engine,
/// or an address), and the web apps.
pub fn newtab_page(prefs: &Preferences, tiles: &[Tile]) -> String {
    let mut grid = String::new();
    for t in tiles {
        let title = if t.mine { "Made from a page · right-click for more".to_string() } else { t.name.clone() };
        grid.push_str(&format!(
            r#"<a class="tile{mine}" href="{href}" draggable="false" title="{title}" data-id="{id}" data-name="{data_name}">{icon}<span>{name}</span></a>"#,
            mine = if t.mine { " mine" } else { "" },
            href = esc_attr(&safe_href(&t.url)),
            title = esc_attr(&title),
            id = esc_attr(&t.id),
            data_name = esc_attr(&t.name),
            name = esc_html(&t.name),
            icon = web_icon(&t.id, &t.name, t.icon.as_deref(), 48),
        ));
    }
    let menu_icons = if tiles.iter().any(|t| t.mine) {
        let icons = serde_json::json!({
            "globe": icon("globe", 15),
            "app-window": icon("app-window", 15),
            "trash": icon("trash", 15),
        });
        // In a JSON script block: only "</" could end it early.
        format!(
            r#"<script type="application/json" id="menu-icons">{}</script>"#,
            icons.to_string().replace("</", "<\\/")
        )
    } else {
        String::new()
    };
    let placeholder = format!("Search {} or type an address", engine_name(&prefs.search_engine));
    page(
        "New tab",
        &format!(
            r#"<main class="newtab">
<div class="brand"><span class="mark">{mark}</span><span>Cactus</span></div>
<form class="search" role="search" data-template="{template}">{search}<input name="q" placeholder="{placeholder}" aria-label="{placeholder}" spellcheck="false" autocomplete="off" autofocus data-newtab-search></form>
<div class="grid">{grid}</div>
<p class="hint">Right-click any page to make it an app of its own.</p>
</main>{menu_icons}"#,
            mark = icon("compass", 26),
            search = icon("search", 18),
            template = esc_attr(&search_template(prefs)),
            placeholder = esc_attr(&placeholder),
        ),
    )
}

// ------------------------------------------------------------ privacy

/// A site this window went to, for Privacy's table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteCount {
    /// Its host ("github.com").
    pub site: String,
    /// A page of it, to link to.
    pub url: String,
    pub pages: u32,
    pub blocked: u64,
}

/// What `cactus://privacy` shows.
#[derive(Debug, Clone, Default)]
pub struct Privacy {
    /// Ads and trackers blocked on this machine, in all.
    pub blocked: u64,
    /// Blocked in this window.
    pub blocked_here: u64,
    /// A private window (it keeps no list of sites).
    pub private: bool,
    pub block_social: bool,
    /// `block_ads_trackers`.
    pub block_ads: bool,
    /// This window, by site, most blocked first.
    pub sites: Vec<SiteCount>,
    /// The sites with the blockers off.
    pub allowed: Vec<String>,
}

/// `cactus://privacy`: what the blockers have stopped (on this machine, and
/// in this window by site), the social media blocker, and the sites they're
/// off for.
pub fn privacy_page(p: &Privacy) -> String {
    let sites = if !p.sites.is_empty() {
        let mut rows = String::new();
        for s in &p.sites {
            rows.push_str(&format!(
                r#"<tr><td><a href="{href}">{site}</a></td><td>{pages}</td><td>{blocked}</td></tr>"#,
                href = esc_attr(&safe_href(&s.url)),
                site = esc_html(&s.site),
                pages = s.pages,
                blocked = thousands(s.blocked),
            ));
        }
        format!(
            r#"<table class="sites"><thead><tr><th>Site</th><th>Pages</th><th>Blocked</th></tr></thead><tbody>{rows}</tbody></table>"#
        )
    } else if p.private {
        r#"<p class="empty">A private window keeps no list of where it’s been.</p>"#.to_string()
    } else {
        r#"<p class="empty">Nothing yet: open a site.</p>"#.to_string()
    };
    const NO_ALLOWED: &str = "No sites: the blockers are on everywhere.";
    let allowed = if p.allowed.is_empty() {
        format!(r#"<p class="empty">{NO_ALLOWED}</p>"#)
    } else {
        let mut li = String::new();
        for site in &p.allowed {
            li.push_str(&format!(
                r#"<li data-row><span>{site}</span><button class="btn quiet sm" data-do="blockers" data-args="{args}" data-gone>{icon}<span>Block again</span></button></li>"#,
                site = esc_html(site),
                args = esc_attr(&serde_json::json!({ "site": site, "on": true }).to_string()),
                icon = icon("shield-check", 14),
            ));
        }
        format!(r#"<ul class="plain allowed" data-empty="{NO_ALLOWED}">{li}</ul>"#)
    };
    let ads_note = if p.block_ads {
        "Brave’s adblock engine, with EasyList and EasyPrivacy’s filters. Always on."
    } else {
        "Brave’s adblock engine, with EasyList and EasyPrivacy’s filters. Off in prefs.toml."
    };
    page(
        "Privacy",
        &format!(
            r#"<main class="cactus-page wide">
<h1>{h1icon} Privacy</h1>
<div class="stats">
<div class="stat big"><b>{blocked}</b><span>ads and trackers blocked on this machine</span></div>
<div class="stat"><b>{here}</b><span>{where_}</span></div>
<div class="stat"><b>{n_allowed}</b><span>sites with the blockers off</span></div>
</div>
<section>
<div class="row">{ban}<div class="grow"><span>Block all social media</span><small>X, Facebook, Instagram, TikTok, Reddit, Threads, LinkedIn, Snapchat and Bluesky, in every window and web app.</small></div>{social}</div>
<div class="row">{shield}<div class="grow"><span>Block ads and trackers</span><small>{ads_note}</small></div>{ads}</div>
</section>
<h2>This window, by site</h2>
{sites}
<h2>Blockers off for</h2>
{allowed}
</main>"#,
            h1icon = icon_class("shield-check", 26, "lime"),
            blocked = thousands(p.blocked),
            here = thousands(p.blocked_here),
            where_ = if p.private { "in this private window" } else { "in this window" },
            n_allowed = p.allowed.len(),
            ban = icon("shield-ban", 18),
            shield = icon("shield", 18),
            social = toggle("block-social", p.block_social, false, "Block all social media"),
            ads = toggle("", p.block_ads, true, "Block ads and trackers"),
        ),
    )
}

// ------------------------------------------------------------ settings

/// A profile, as Settings lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRow {
    pub id: String,
    pub name: String,
    /// Its colour, `#rrggbb`.
    pub color: String,
}

/// What `cactus://settings` shows besides the preferences.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub block_social: bool,
    pub profiles: Vec<ProfileRow>,
    /// Your web apps (made from pages).
    pub apps: Vec<Tile>,
    /// WebKitGTK's version, for About.
    pub webkit_version: String,
}

/// `cactus://settings`: privacy, search, profiles, your web apps, and what
/// this browser is built from. Of Cactus's preferences, only what the v2
/// design shows: blocking ads and trackers, and the search engine.
pub fn settings_page(prefs: &Preferences, s: &Settings) -> String {
    let chosen = crate::search::id_for_engine(&prefs.search_engine);
    let mut options = String::new();
    for e in crate::search::BUILTIN_ENGINES {
        options.push_str(&format!(
            r#"<option value="{id}"{sel}>{name}</option>"#,
            id = e.id,
            sel = if e.id == chosen { " selected" } else { "" },
            name = esc_html(e.name),
        ));
    }
    if chosen == "custom" {
        options.push_str(&format!(
            r#"<option value="custom" selected disabled>{}</option>"#,
            esc_html(engine_name(&prefs.search_engine))
        ));
    }

    let mut profiles = String::new();
    for p in &s.profiles {
        profiles.push_str(&format!(
            r#"<div class="row"><span class="swatch" style="background:{color}"></span><div class="grow"><span>{name}</span><small class="line">Its own sign-ins, cookies and history.</small></div></div>"#,
            color = safe_color(&p.color),
            name = esc_html(&p.name),
        ));
    }
    profiles.push_str(&format!(
        r#"<div class="row"><button class="btn quiet sm" data-do="new-profile">{}<span>New profile…</span></button></div>"#,
        icon("user-round-plus", 14)
    ));

    let mut apps = String::new();
    for a in &s.apps {
        let id = esc_attr(&serde_json::json!({ "id": a.id }).to_string());
        apps.push_str(&format!(
            r#"<div class="row" data-row>{icon}<div class="grow"><span>{name}</span><small class="line">{url}</small></div><button class="btn ghost sm" data-do="open-app" data-args="{id}">Open</button><button class="btn danger sm" data-do="remove-app" data-args="{id}" data-gone>Remove</button></div>"#,
            icon = web_icon(&a.id, &a.name, a.icon.as_deref(), 28),
            name = esc_html(&a.name),
            url = esc_html(&a.url),
        ));
    }
    const NO_APPS: &str = "None yet. Right-click a page and choose “Create app from this page…”.";
    if apps.is_empty() {
        apps = format!(r#"<div class="row"><small>{NO_APPS}</small></div>"#);
    }

    page(
        "Settings",
        &format!(
            r#"<main class="cactus-page">
<h1>{h1icon} Settings</h1>
<h2>Privacy</h2>
<section>
<div class="row"><div class="grow"><span>Block all social media</span><small class="line">A plugin of WadSpaces’ own: social sites show a blocked page instead.</small></div>{social}</div>
<div class="row"><div class="grow"><span>Block ads and trackers</span><small class="line">{ads_note} <a href="cactus://privacy" class="inline">See what it’s blocked</a></small></div>{ads}</div>
</section>
<h2>Search</h2>
<section>
<div class="row"><div class="grow"><span>Search engine</span><small class="line">What the address bar and new tabs search with.</small></div><select class="choice" data-do="search-engine" aria-label="Search engine">{options}</select></div>
</section>
<h2>Profiles</h2>
<section>{profiles}</section>
<h2>Web apps</h2>
<section data-empty="{NO_APPS}">{apps}</section>
<h2>About</h2>
<section>
<div class="row"><div class="grow"><span>{name} {version} (WadOS’ fork of Cactus {cactus})</span><small class="line">WebKitGTK {webkit} · GTK 4 · libadwaita · adblock-rust · <a href="cactus://about" class="inline">More</a></small></div></div>
</section>
</main>"#,
            h1icon = icon("settings", 24),
            social = toggle("block-social", s.block_social, false, "Block all social media"),
            ads_note = if prefs.block_ads_trackers { "Always on." } else { "Off in prefs.toml." },
            ads = toggle("", prefs.block_ads_trackers, true, "Block ads and trackers"),
            name = crate::APP_NAME,
            version = crate::APP_VERSION,
            cactus = CACTUS_VERSION,
            webkit = esc_html(&s.webkit_version),
        ),
    )
}

// ------------------------------------------------------------ history

/// `cactus://history`: the pages visited, newest first (the browser's own
/// left out); a private window keeps none. `now` dates them ("5 min ago").
pub fn history_page(entries: &[HistoryEntry], private: bool, now: DateTime<Utc>) -> String {
    let body = if private {
        format!(r#"<p class="empty line">{} Private windows keep no history.</p>"#, icon("eye-off", 16))
    } else {
        let mut li = String::new();
        for e in entries.iter().filter(|e| !is_internal(&e.url)) {
            li.push_str(&format!(
                r#"<li><small>{ago}</small><a class="t" href="{href}">{title}</a><span class="host">{host}</span></li>"#,
                ago = ago(now, e.last_visit),
                href = esc_attr(&safe_href(&e.url)),
                title = esc_html(if e.title.trim().is_empty() { &e.url } else { &e.title }),
                host = esc_html(&host_of(&e.url)),
            ));
        }
        if li.is_empty() {
            r#"<p class="empty line">Nothing yet.</p>"#.to_string()
        } else {
            format!(r#"<ul class="plain visits">{li}</ul>"#)
        }
    };
    page("History", &format!(r#"<main class="cactus-page list"><h1>{} History</h1>{body}</main>"#, icon("clock", 24)))
}

/// `cactus://bookmarks` (Cactus's; the v2 design has none, so nothing links here).
pub fn bookmarks_html(entries: &[Bookmark]) -> String {
    let mut li = String::new();
    for e in entries {
        li.push_str(&format!(
            r#"<li><small>{folder}</small><a class="t" href="{href}">{title}</a><span class="host">{host}</span></li>"#,
            folder = esc_html(&e.folder),
            href = esc_attr(&safe_href(&e.url)),
            title = esc_html(if e.title.trim().is_empty() { &e.url } else { &e.title }),
            host = esc_html(&host_of(&e.url)),
        ));
    }
    let body = if li.is_empty() {
        r#"<p class="empty line">No bookmarks yet.</p>"#.to_string()
    } else {
        format!(r#"<ul class="plain visits">{li}</ul>"#)
    };
    page(
        "Bookmarks",
        &format!(r#"<main class="cactus-page list"><h1>{} Bookmarks</h1>{body}</main>"#, icon("star", 24)),
    )
}

/// `cactus://downloads`.
pub fn downloads_html(entries: &[DownloadRecord]) -> String {
    let mut li = String::new();
    for e in entries {
        li.push_str(&format!(
            r#"<li><small>{status}</small><span class="t">{name}</span><span class="host">{host}</span></li>"#,
            status = esc_html(&e.status),
            name = esc_html(&e.filename),
            host = esc_html(&host_of(&e.url)),
        ));
    }
    let body = if li.is_empty() {
        r#"<p class="empty line">No downloads yet.</p>"#.to_string()
    } else {
        format!(r#"<ul class="plain visits">{li}</ul>"#)
    };
    page(
        "Downloads",
        &format!(r#"<main class="cactus-page list"><h1>{} Downloads</h1>{body}</main>"#, icon("download", 24)),
    )
}

// ------------------------------------------------------------ about

/// `cactus://about`: WadBrowser, what it's based on, its WebKit and its licence.
pub fn about_html(webkit_version: &str) -> String {
    let repo_shown = CACTUS_REPO.trim_start_matches("https://");
    page(
        "About",
        &format!(
            r#"<main class="cactus-page">
<h1>{h1icon} {name}</h1>
<section>
<div class="row"><div class="grow"><span>{name} {version}</span><small class="line">The web browser of WadOS.</small></div></div>
<div class="row"><div class="grow"><span>Based on Cactus {cactus_version}</span><small class="line"><a class="inline" href="{repo}">{repo_shown}</a> · commit <code>{commit}</code></small></div></div>
<div class="row"><div class="grow"><span>WebKitGTK {webkit}</span><small class="line">GTK 4 · libadwaita · adblock-rust</small></div></div>
<div class="row"><div class="grow"><span>Licence</span><small class="line">{license} (Mozilla Public License 2.0)</small></div></div>
</section>
</main>"#,
            h1icon = icon_class("compass", 24, "lime"),
            name = crate::APP_NAME,
            version = crate::APP_VERSION,
            cactus_version = CACTUS_VERSION,
            repo = CACTUS_REPO,
            commit = CACTUS_COMMIT,
            webkit = esc_html(webkit_version),
            license = CACTUS_LICENSE,
        ),
    )
}

// ------------------------------------------------------------ why there's no page

/// A page that didn't load: `title` ("This page didn't load"), why
/// (`message`), and the address, with Try again and New tab.
pub fn error_html(title: &str, message: &str, uri: &str) -> String {
    page(
        title,
        &format!(
            r#"<main class="notice failed">
<span class="badge">{badge}</span>
<h1>{title}</h1>
<p>{message}</p>
<p class="where"><code>{uri}</code></p>
<div class="acts"><a class="btn ghost" href="cactus://newtab">{plus}<span>New tab</span></a><a class="btn secondary" href="{href}">{again}<span>Try again</span></a></div>
</main>"#,
            badge = icon("wifi-off", 34),
            title = esc_html(title),
            message = esc_html(message),
            uri = esc_html(&shown_uri(uri)),
            href = esc_attr(&safe_href(uri)),
            plus = icon("plus", 16),
            again = icon("refresh-cw", 16),
        ),
    )
}

/// An address as a page shows it: without its fragment, and with a `token`
/// in its query masked. A stream's one-tap link carries its key there
/// (`/ws-<guid>/#token=…`), and a failed load must not put it on screen.
/// "Try again" keeps the whole address: the key is needed to load it.
pub fn shown_uri(uri: &str) -> String {
    let base = uri.split('#').next().unwrap_or_default();
    let Some((path, query)) = base.split_once('?') else { return base.to_string() };
    let query: Vec<String> = query
        .split('&')
        .map(|kv| match kv.split_once('=') {
            Some((k, _)) if k.eq_ignore_ascii_case("token") => format!("{k}=…"),
            _ => kv.to_string(),
        })
        .collect();
    format!("{path}?{}", query.join("&"))
}

/// The social sites the blocker knows, by name (the spike's `socialName`).
pub fn social_name(site: &str) -> &str {
    match site {
        "x.com" | "twitter.com" => "X",
        "facebook.com" => "Facebook",
        "instagram.com" => "Instagram",
        "tiktok.com" => "TikTok",
        "reddit.com" => "Reddit",
        "threads.net" => "Threads",
        "linkedin.com" => "LinkedIn",
        "snapchat.com" => "Snapchat",
        "bsky.app" => "Bluesky",
        other => other,
    }
}

/// What a social site shows while "Block all social media" is on: Go back
/// (when there's a page to go back to), and the blockers off for `site`.
/// Load it at a `cactus://` address (`cactus://blocked?site=x.com`), so its
/// button may ask ([`PageAction::from_page`]).
pub fn blocked_html(site: &str) -> String {
    let name = social_name(site);
    page(
        &format!("{name}: blocked"),
        &format!(
            r#"<main class="notice blocked">
<span class="badge">{badge}</span>
<h1>Blocked: social media</h1>
<p>{name} ({site}) is social media, and <b>Block all social media</b> is on in this browser.</p>
<div class="acts"><button class="btn ghost" data-back hidden>{back}<span>Go back</span></button><button class="btn secondary" data-do="blockers" data-args="{args}">{off}<span>Turn the blocker off for this site</span></button></div>
</main>
<script>if (history.length > 1) document.querySelector('[data-back]').hidden = false;</script>"#,
            badge = icon("shield-ban", 34),
            name = esc_html(name),
            site = esc_html(site),
            args = esc_attr(&serde_json::json!({ "site": site, "on": false }).to_string()),
            back = icon("arrow-left", 16),
            off = icon("shield-off", 16),
        ),
    )
}

/// A `cactus://` page that doesn't exist.
pub fn missing_html(uri: &str) -> String {
    page(
        "No such page",
        &format!(
            r#"<main class="notice missing">{compass}<p>Cactus has no page called <code>{uri}</code>.</p><a class="home" href="cactus://newtab">Open a new tab</a></main>"#,
            compass = icon("compass", 28),
            uri = esc_html(uri),
        ),
    )
}

// ------------------------------------------------------------ pieces

/// A page: its title, the shared stylesheet and script, and `body`.
fn page(title: &str, body: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark">
<title>{title}</title>
<link rel="stylesheet" href="{css}">
<script src="{js}" defer></script>
</head>
<body>
{body}
</body>
</html>"#,
        title = esc_html(title),
        css = res_url("pages.css"),
        js = res_url("pages.js"),
    )
}

/// A switch (ui/Toggle): `action` is posted with its new state (`on`).
fn toggle(action: &str, on: bool, disabled: bool, label: &str) -> String {
    format!(
        r#"<button class="toggle{on_class}" role="switch" aria-checked="{on}" aria-label="{label}"{dis}{act}><span class="knob"></span></button>"#,
        on_class = if on { " on" } else { "" },
        label = esc_attr(label),
        dis = if disabled { " disabled" } else { "" },
        act = if action.is_empty() { String::new() } else { format!(r#" data-do="{}""#, esc_attr(action)) },
    )
}

/// A web app's icon (ui/AppIcon): its site's on a white card, else its
/// initial in one of the tints on a violet card.
fn web_icon(id: &str, name: &str, src: Option<&str>, size: u32) -> String {
    if let Some(src) = src.and_then(safe_img) {
        return format!(
            r#"<span class="site-icon" style="--s:{size}px"><img src="{src}" alt="" draggable="false" referrerpolicy="no-referrer"></span>"#,
            src = esc_attr(src),
        );
    }
    const TINTS: [&str; 6] = ["#c6ff1f", "#7fd3ff", "#ff3d81", "#ffb36b", "#b58cff", "#5cf2c4"];
    let h = id.encode_utf16().fold(0u32, |h, c| h.wrapping_mul(31).wrapping_add(c as u32));
    let shown = if name.trim().is_empty() { id } else { name };
    let shown = match shown.get(..7) {
        Some(g) if g.eq_ignore_ascii_case("google ") => shown[7..].trim_start(),
        _ => shown,
    };
    let letter: String = shown.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into());
    format!(
        r#"<span class="web-icon" style="--s:{size}px; --tint:{tint}"><span>{letter}</span></span>"#,
        tint = TINTS[(h % 6) as usize],
        letter = esc_html(&letter),
    )
}

/// The lucide icons the pages draw (assets/lucide, as wad-gtk's).
static ICONS: &[(&str, &str)] = &[
    ("app-window", include_str!("../../../assets/lucide/app-window.svg")),
    ("arrow-left", include_str!("../../../assets/lucide/arrow-left.svg")),
    ("clock", include_str!("../../../assets/lucide/clock.svg")),
    ("compass", include_str!("../../../assets/lucide/compass.svg")),
    ("download", include_str!("../../../assets/lucide/download.svg")),
    ("eye-off", include_str!("../../../assets/lucide/eye-off.svg")),
    ("globe", include_str!("../../../assets/lucide/globe.svg")),
    ("plus", include_str!("../../../assets/lucide/plus.svg")),
    ("refresh-cw", include_str!("../../../assets/lucide/refresh-cw.svg")),
    ("search", include_str!("../../../assets/lucide/search.svg")),
    ("settings", include_str!("../../../assets/lucide/settings.svg")),
    ("shield", include_str!("../../../assets/lucide/shield.svg")),
    ("shield-ban", include_str!("../../../assets/lucide/shield-ban.svg")),
    ("shield-check", include_str!("../../../assets/lucide/shield-check.svg")),
    ("shield-off", include_str!("../../../assets/lucide/shield-off.svg")),
    ("star", include_str!("../../../assets/lucide/star.svg")),
    ("trash", include_str!("../../../assets/lucide/trash.svg")),
    ("user-round-plus", include_str!("../../../assets/lucide/user-round-plus.svg")),
    ("wifi-off", include_str!("../../../assets/lucide/wifi-off.svg")),
];

/// A lucide icon, inline, `size` pixels, stroked in the text's colour at
/// lucide-svelte's 1.75 (the spike's `Icon`).
fn icon(name: &str, size: u32) -> String {
    icon_class(name, size, "")
}

fn icon_class(name: &str, size: u32, class: &str) -> String {
    let Some((_, svg)) = ICONS.iter().find(|(n, _)| *n == name) else { return String::new() };
    let class = if class.is_empty() { "icon".to_string() } else { format!("icon {class}") };
    svg.trim()
        .replacen(
            r#"width="24" height="24""#,
            &format!(r#"width="{size}" height="{size}" class="{class}" aria-hidden="true""#),
            1,
        )
        .replacen(r#"stroke-width="2""#, r#"stroke-width="1.75""#, 1)
}

/// 1284 → "1,284".
fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "just now", "5 min ago", "3 h ago", "2 days ago" (the spike's `ago`).
fn ago(now: DateTime<Utc>, at: DateTime<Utc>) -> String {
    let s = (now - at).num_seconds().max(0) as f64;
    if s < 45.0 {
        return "just now".into();
    }
    if s < 3600.0 {
        return format!("{} min ago", (s / 60.0).round());
    }
    if s < 86400.0 {
        return format!("{} h ago", (s / 3600.0).round());
    }
    let d = (s / 86400.0).round() as u64;
    format!("{d} day{} ago", if d == 1 { "" } else { "s" })
}

/// A URL's host and port, lowercased ("github.com"), or the URL itself.
fn host_of(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => match (u.host_str(), u.port()) {
            (Some(h), Some(p)) => format!("{h}:{p}"),
            (Some(h), None) => h.to_string(),
            _ => url.to_string(),
        },
        Err(_) => url.to_string(),
    }
}

/// A URL's host without `www.` and its port, to tell sites apart.
fn bare_host(url: &str) -> String {
    let h = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_lowercase)).unwrap_or_default();
    h.strip_prefix("www.").map(str::to_string).unwrap_or(h)
}

/// `url` if a link may go there (the web, files, the browser's own pages); else nowhere.
fn safe_href(url: &str) -> String {
    let lower = url.trim().to_ascii_lowercase();
    let ok = ["http://", "https://", "cactus://", "file://", "about:"].iter().any(|s| lower.starts_with(s));
    if ok { url.trim().to_string() } else { "#".into() }
}

/// `src` if an image may come from it.
fn safe_img(src: &str) -> Option<&str> {
    let lower = src.trim().to_ascii_lowercase();
    ["cactus://res/", "data:image/", "file://", "https://", "http://"]
        .iter()
        .any(|s| lower.starts_with(s))
        .then_some(src.trim())
}

/// A colour for a style attribute: `#rgb` or `#rrggbb`, else the accent.
fn safe_color(c: &str) -> &str {
    let hex = c.strip_prefix('#').unwrap_or("");
    if matches!(hex.len(), 3 | 6) && hex.bytes().all(|b| b.is_ascii_hexdigit()) { c } else { "#c6ff1f" }
}

fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn esc_attr(s: &str) -> String {
    esc_html(s).replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn an_error_page_never_shows_a_streams_key() {
        let link = "https://m.wadspaces.com/ws-abc/#token=SECRETKEY0123456789";
        assert_eq!(shown_uri(link), "https://m.wadspaces.com/ws-abc/");
        assert_eq!(shown_uri("https://h/ws-abc/?token=SECRETKEY&x=1#y"), "https://h/ws-abc/?token=…&x=1");
        assert_eq!(shown_uri("https://example.com/a?b=c"), "https://example.com/a?b=c");
        let page = error_html("It didn't load", "No answer", link);
        let shown = page.split("class=\"where\"").nth(1).unwrap_or_default();
        assert!(!shown.split("href=").next().unwrap_or_default().contains("SECRETKEY"), "{shown}");
    }

    /// Markup, and a way out of a double-quoted attribute (every attribute here is one).
    const EVIL: &str = r#"<script>alert('x')</script>" onmouseover="x"#;

    fn tile(id: &str, name: &str, url: &str, icon: Option<&str>) -> Tile {
        Tile { id: id.into(), name: name.into(), url: url.into(), icon: icon.map(str::to_string), mine: false }
    }

    fn entry(title: &str, url: &str, ago: Duration) -> HistoryEntry {
        HistoryEntry { id: 1, title: title.into(), url: url.into(), visit_count: 1, last_visit: Utc::now() - ago }
    }

    /// No raw markup from `EVIL` got through.
    fn clean(html: &str) {
        assert!(!html.contains("<script>alert"), "{html}");
        assert!(!html.contains(r#"" onmouseover=""#), "{html}");
        assert!(!html.contains(r#"href="javascript:"#) && !html.contains(r#"src="javascript:"#), "{html}");
    }

    fn links_shared_files(html: &str) {
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains(r#"<link rel="stylesheet" href="cactus://res/pages.css">"#));
        assert!(html.contains(r#"<script src="cactus://res/pages.js" defer></script>"#));
    }

    #[test]
    fn every_page_renders_with_the_shared_files() {
        let prefs = Preferences::default();
        let pages = [
            newtab_page(
                &prefs,
                &[tile("github", "GitHub", "https://github.com", Some("cactus://res/icons/web/github.svg"))],
            ),
            privacy_page(&Privacy::default()),
            settings_page(&prefs, &Settings::default()),
            history_page(&[], false, Utc::now()),
            bookmarks_html(&[]),
            downloads_html(&[]),
            about_html("2.54.2"),
            error_html("This page didn’t load", "Could not resolve host", "https://nowhere.invalid/"),
            blocked_html("x.com"),
            missing_html("cactus://nowhere"),
        ];
        for html in &pages {
            links_shared_files(html);
            assert!(html.contains("<main class="), "{html}");
        }
    }

    #[test]
    fn the_new_tab_searches_with_the_preferred_engine() {
        let mut prefs = Preferences::default();
        let html = newtab_page(&prefs, &[]);
        assert!(html.contains(r#"placeholder="Search DuckDuckGo or type an address""#));
        assert!(html.contains(r#"data-template="https://duckduckgo.com/?q=%s""#));
        prefs.search_engine = SearchEngine::Startpage;
        let html = newtab_page(&prefs, &[]);
        assert!(html.contains("Search Startpage or type an address"));
        assert!(html.contains(r#"data-template="https://www.startpage.com/sp/search?query=%s""#));
        prefs.search_engine = SearchEngine::Custom {
            name: "Kagi".into(),
            url_template: "https://kagi.com/search?q={searchTerms}".into(),
        };
        assert!(newtab_page(&prefs, &[]).contains(r#"data-template="https://kagi.com/search?q=%s""#));
    }

    #[test]
    fn the_new_tab_draws_its_tiles() {
        let tiles = merge_tiles(
            vec![
                tile("github", "GitHub", "https://github.com", Some("cactus://res/icons/web/github.svg")),
                tile(
                    "calendar",
                    "Calendar",
                    "https://calendar.google.com",
                    Some("cactus://res/icons/web/calendar.png"),
                ),
            ],
            vec![tile("cal", "My calendar", "https://calendar.google.com/r/week", None)],
        );
        let html = newtab_page(&Preferences::default(), &tiles);
        assert!(html.contains(r#"<img src="cactus://res/icons/web/github.svg""#));
        assert!(!html.contains("icons/web/calendar.png"), "the catalog's Calendar shows once, as yours");
        assert!(html.contains(r#"class="tile mine""#));
        assert!(html.contains(r#"<span class="web-icon" style="--s:48px; --tint:"#));
        assert!(html.contains(">M</span>"));
        assert!(html.contains(r#"id="menu-icons""#));
        assert!(html.contains("Right-click any page to make it an app of its own."));
        // The initial skips "Google ".
        assert!(web_icon("drive", "Google Drive", None, 48).contains(">D</span>"));
    }

    #[test]
    fn merging_tiles_keeps_yours_last_and_drops_the_catalogs_twin() {
        let t = merge_tiles(
            vec![
                tile("github", "GitHub", "https://github.com", None),
                tile("yt", "YouTube", "https://youtube.com", None),
            ],
            vec![tile("mine", "Tube", "https://www.youtube.com/feed", None)],
        );
        let ids: Vec<(&str, bool)> = t.iter().map(|t| (t.id.as_str(), t.mine)).collect();
        assert_eq!(ids, [("github", false), ("mine", true)]);
    }

    #[test]
    fn pages_escape_what_they_are_given() {
        let prefs = Preferences::default();
        let mut evil_tile = tile(EVIL, EVIL, "javascript:alert(1)", Some("javascript:alert(1)"));
        evil_tile.mine = true;
        clean(&newtab_page(&prefs, &[evil_tile.clone()]));

        let p = Privacy {
            sites: vec![SiteCount { site: EVIL.into(), url: "javascript:alert(1)".into(), pages: 1, blocked: 2 }],
            allowed: vec![EVIL.into()],
            ..Privacy::default()
        };
        clean(&privacy_page(&p));

        let s = Settings {
            profiles: vec![ProfileRow { id: "p".into(), name: EVIL.into(), color: "red;background:url(x)".into() }],
            apps: vec![evil_tile],
            webkit_version: EVIL.into(),
            ..Settings::default()
        };
        let html = settings_page(&prefs, &s);
        clean(&html);
        assert!(html.contains("background:#c6ff1f"), "a colour that isn't one falls back to the accent");

        clean(&history_page(&[entry(EVIL, "javascript:alert(1)", Duration::minutes(1))], false, Utc::now()));
        clean(&error_html(EVIL, EVIL, "javascript:alert(1)"));
        clean(&blocked_html(EVIL));
        clean(&missing_html(EVIL));
        clean(&about_html(EVIL));
        clean(&bookmarks_html(&[Bookmark {
            id: 1,
            title: EVIL.into(),
            url: "javascript:alert(1)".into(),
            folder: EVIL.into(),
            position: 0,
            created_at: Utc::now(),
        }]));
    }

    #[test]
    fn privacy_shows_the_counts_and_the_sites() {
        let p = Privacy {
            blocked: 1291,
            blocked_here: 7,
            block_social: true,
            block_ads: true,
            sites: vec![SiteCount { site: "x.com".into(), url: "https://x.com".into(), pages: 1, blocked: 7 }],
            allowed: vec!["old.reddit.com".into()],
            ..Privacy::default()
        };
        let html = privacy_page(&p);
        assert!(html.contains("<b>1,291</b><span>ads and trackers blocked on this machine</span>"));
        assert!(html.contains("<b>7</b><span>in this window</span>"));
        assert!(html.contains("<b>1</b><span>sites with the blockers off</span>"));
        assert!(html.contains(r#"<td><a href="https://x.com">x.com</a></td><td>1</td><td>7</td>"#));
        assert!(html.contains(r#"class="toggle on" role="switch" aria-checked="true" aria-label="Block all social media" data-do="block-social""#));
        assert!(html.contains(r#"data-args="{&quot;site&quot;:&quot;old.reddit.com&quot;,&quot;on&quot;:true}""#));

        let private = privacy_page(&Privacy { private: true, ..Privacy::default() });
        assert!(private.contains("in this private window"));
        assert!(private.contains("A private window keeps no list of where it’s been."));
        assert!(private.contains("No sites: the blockers are on everywhere."));
    }

    #[test]
    fn settings_shows_the_engine_profiles_and_apps() {
        let prefs = Preferences { search_engine: SearchEngine::Wikipedia, ..Preferences::default() };
        let s = Settings {
            profiles: vec![ProfileRow { id: "default".into(), name: "Personal".into(), color: "#c6ff1f".into() }],
            apps: vec![tile("cal", "Calendar", "https://calendar.google.com", None)],
            webkit_version: "2.54.2".into(),
            ..Settings::default()
        };
        let html = settings_page(&prefs, &s);
        assert!(html.contains(r#"<option value="wikipedia" selected>Wikipedia</option>"#));
        assert!(html.contains(r#"<option value="duckduckgo">DuckDuckGo</option>"#));
        assert!(html.contains(r#"<span class="swatch" style="background:#c6ff1f"></span>"#));
        assert!(html.contains(r#"data-do="remove-app" data-args="{&quot;id&quot;:&quot;cal&quot;}""#));
        assert!(html.contains("WebKitGTK 2.54.2"));
        assert!(!html.contains("Restore session"), "only what the v2 design shows");
        assert!(settings_page(&prefs, &Settings::default()).contains("None yet. Right-click a page"));
    }

    #[test]
    fn history_lists_web_pages_newest_first_and_private_keeps_none() {
        let now = Utc::now();
        let entries = [
            entry("Docs.rs", "https://docs.rs/", Duration::seconds(10)),
            entry("Settings", "cactus://settings", Duration::seconds(20)),
            entry("", "https://github.com:8443/x", Duration::hours(3)),
        ];
        let html = history_page(&entries, false, now);
        assert!(html.contains(r#"<li><small>just now</small><a class="t" href="https://docs.rs/">Docs.rs</a><span class="host">docs.rs</span></li>"#));
        assert!(html.contains("<small>3 h ago</small>"));
        assert!(html.contains(">https://github.com:8443/x</a><span class=\"host\">github.com:8443</span>"));
        assert!(!html.contains("cactus://settings"));
        assert!(history_page(&entries, true, now).contains("Private windows keep no history."));
        assert!(history_page(&[], false, now).contains("Nothing yet."));
    }

    #[test]
    fn ago_reads_like_the_spikes() {
        let now = Utc::now();
        assert_eq!(ago(now, now - Duration::seconds(30)), "just now");
        assert_eq!(ago(now, now - Duration::minutes(5)), "5 min ago");
        assert_eq!(ago(now, now - Duration::hours(2)), "2 h ago");
        assert_eq!(ago(now, now - Duration::days(1)), "1 day ago");
        assert_eq!(ago(now, now - Duration::days(9)), "9 days ago");
        assert_eq!(ago(now, now + Duration::days(1)), "just now");
    }

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1284), "1,284");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn about_page_credits_cactus() {
        let html = about_html("2.54.2");
        assert!(html.contains("WadBrowser"));
        assert!(html.contains("github.com/yohannes-mesay/cactus-browser"));
        assert!(html.contains("8ae5a66"));
        assert!(html.contains("2.54.2"));
        assert!(html.contains("MPL-2.0"));
        assert!(!html.contains("cactus-browser.tech"));
    }

    #[test]
    fn the_blocked_page_names_the_site_and_offers_to_unblock_it() {
        let html = blocked_html("x.com");
        assert!(html.contains("X (x.com) is social media"));
        assert!(
            html.contains(
                r#"data-do="blockers" data-args="{&quot;site&quot;:&quot;x.com&quot;,&quot;on&quot;:false}""#
            )
        );
        assert!(blocked_html("bsky.app").contains("Bluesky (bsky.app)"));
    }

    #[test]
    fn error_pages_offer_another_go() {
        let html = error_html("This page didn’t load", "Could not resolve host", "https://nowhere.invalid/a?b=1&c=2");
        assert!(html.contains(r#"<a class="btn secondary" href="https://nowhere.invalid/a?b=1&amp;c=2">"#));
        assert!(html.contains("Could not resolve host"));
        assert!(missing_html("cactus://nowhere").contains("Cactus has no page called <code>cactus://nowhere</code>."));
    }

    #[test]
    fn the_shared_files_are_served() {
        let (css, mime) = resource("pages.css").unwrap();
        assert_eq!(mime, "text/css");
        assert!(std::str::from_utf8(css).unwrap().contains("--accent: #c6ff1f"));
        assert_eq!(resource("/pages.js").unwrap().1, "text/javascript");
        assert_eq!(resource("cactus://res/fonts/Inter-Variable.ttf").unwrap().1, "font/ttf");
        assert!(resource("fonts/SpaceGrotesk-Variable.ttf?v=1").unwrap().0.len() > 10_000);
        assert_eq!(resource("icons/web/github.svg").unwrap().1, "image/svg+xml");
        assert_eq!(resource("icons/web/gmail.png").unwrap().1, "image/png");
        assert!(resource("../Cargo.toml").is_none());
        assert!(resource("nope.css").is_none());
        assert_eq!(res_url("/icons/web/github.svg"), "cactus://res/icons/web/github.svg");
        // Every icon the pages draw is there.
        for (name, svg) in ICONS {
            assert!(icon(name, 16).contains(r#"width="16" height="16" class="icon""#), "{name}: {svg}");
        }
    }

    #[test]
    fn pages_ask_with_actions_only_their_own() {
        assert_eq!(
            PageAction::from_page("cactus://privacy", r#"{"do":"block-social","on":true}"#),
            Some(PageAction::BlockSocial { on: true })
        );
        assert_eq!(
            PageAction::parse(r#"{"do":"blockers","site":"x.com","on":false}"#),
            Some(PageAction::Blockers { site: "x.com".into(), on: false })
        );
        assert_eq!(PageAction::parse(r#"{"do":"go","to":"rust gtk"}"#), Some(PageAction::Go { to: "rust gtk".into() }));
        assert_eq!(
            PageAction::parse(r#"{"do":"search-engine","id":"startpage"}"#),
            Some(PageAction::SearchEngine { id: "startpage".into() })
        );
        assert_eq!(PageAction::parse(r#"{"do":"new-profile"}"#), Some(PageAction::NewProfile));
        assert_eq!(
            PageAction::parse(r#"{"do":"open-app","id":"cal"}"#),
            Some(PageAction::OpenApp { id: "cal".into() })
        );
        assert_eq!(
            PageAction::parse(r#"{"do":"remove-app","id":"cal"}"#),
            Some(PageAction::RemoveApp { id: "cal".into() })
        );
        assert_eq!(PageAction::parse(r#"{"do":"format-disk"}"#), None);
        assert_eq!(PageAction::parse("not json"), None);
        assert_eq!(PageAction::from_page("https://evil.example", r#"{"do":"block-social","on":false}"#), None);
    }
}
