// The v2 model, as a wadspace's document has it (version 6), and the types
// the machine's parts share about what runs: sessions, streams, jobs,
// peers, focus. It stays close to the spike's src/model/types.ts, which
// stayed close to v5. Where it differs from the spike, see the crate's doc.

use std::fmt;
use std::str::FromStr;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// The document's version.
pub const VERSION: u32 = 6;

// ------------------------------------------------------------ apps

/// An app WadSpaces has (there's no package manager: these are all of them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppId {
    Wadbrowser,
    Vscode,
    Obsidian,
    Tiled,
    Libresprite,
    Audacity,
    Lmms,
    Krita,
    Openshot,
    Steam,
    Dolphin,
    Obs,
    Terminal,
}

impl AppId {
    /// Every app, in the catalog's order.
    pub const ALL: [AppId; 13] = [
        AppId::Wadbrowser,
        AppId::Vscode,
        AppId::Obsidian,
        AppId::Tiled,
        AppId::Libresprite,
        AppId::Audacity,
        AppId::Lmms,
        AppId::Krita,
        AppId::Openshot,
        AppId::Steam,
        AppId::Dolphin,
        AppId::Obs,
        AppId::Terminal,
    ];

    /// Its id ('vscode').
    pub fn as_str(self) -> &'static str {
        match self {
            AppId::Wadbrowser => "wadbrowser",
            AppId::Vscode => "vscode",
            AppId::Obsidian => "obsidian",
            AppId::Tiled => "tiled",
            AppId::Libresprite => "libresprite",
            AppId::Audacity => "audacity",
            AppId::Lmms => "lmms",
            AppId::Krita => "krita",
            AppId::Openshot => "openshot",
            AppId::Steam => "steam",
            AppId::Dolphin => "dolphin",
            AppId::Obs => "obs",
            AppId::Terminal => "terminal",
        }
    }

    /// The app an id names, if it's one.
    pub fn parse(s: &str) -> Option<AppId> {
        AppId::ALL.into_iter().find(|a| a.as_str() == s)
    }

    /// The kind of key it signs in with, if it has one of its own (WadBrowser's are its web apps' groups).
    pub fn key_kind(self) -> Option<SecretKind> {
        match self {
            AppId::Vscode => Some(SecretKind::Vscode),
            AppId::Obsidian => Some(SecretKind::Obsidian),
            AppId::Steam => Some(SecretKind::Steam),
            _ => None,
        }
    }
}

impl fmt::Display for AppId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Not an app WadSpaces has.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not an app WadSpaces has: {0:?}")]
pub struct UnknownApp(pub String);

impl FromStr for AppId {
    type Err = UnknownApp;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        AppId::parse(s).ok_or_else(|| UnknownApp(s.to_string()))
    }
}

/// An app in a wadspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppIn {
    pub app: AppId,
    /// The wallpaper when it's opened from this app (or one of its variants without its own).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<Wallpaper>,
}

impl AppIn {
    pub fn new(app: AppId) -> Self {
        AppIn { app, wallpaper: None }
    }
}

/// One of an app's variants (the Apps window shows them in the app): a VS
/// Code extension, a terminal tool, a browser web app or Focus, or, for any
/// other app, a named variant of its own (Dolphin's Mario Kart, Steam's
/// Stardew Valley).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ext {
    pub app: AppId,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    /// The wallpaper when it's opened from this variant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<Wallpaper>,
}

impl Ext {
    pub fn new(app: AppId, id: impl Into<String>) -> Self {
        Ext { app, id: id.into(), name: None, url: None, icon_url: None, wallpaper: None }
    }

    /// With a name.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// With a web address.
    pub fn at(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }
}

// ------------------------------------------------------------ the look

/// A background: a picture (WadSpaces' art, one from ~/Pictures, or one
/// uploaded there), a plain colour, or none (a new wadspace's: its icon is
/// empty glass).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Wallpaper {
    #[default]
    Empty,
    Color {
        value: String,
    },
    Image {
        value: String,
    },
}

impl Wallpaper {
    /// A picture ('/wallpapers/art/dunes.svg', '~/Pictures/lagoon.png').
    pub fn image(value: impl Into<String>) -> Self {
        Wallpaper::Image { value: value.into() }
    }

    /// A plain colour ('#17112a').
    pub fn color(value: impl Into<String>) -> Self {
        Wallpaper::Color { value: value.into() }
    }

    /// What it is: the picture's path, or the colour (none if empty).
    pub fn value(&self) -> Option<&str> {
        match self {
            Wallpaper::Empty => None,
            Wallpaper::Color { value } | Wallpaper::Image { value } => Some(value),
        }
    }
}

/// Where a mounted or copied project's files are on their machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSource {
    pub machine_id: String,
    pub paths: Vec<String>,
}

/// A project: a repository cloned in, a folder mounted, or a file copied in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    /// What it's called inside the wadspace (the spike's `as`).
    #[serde(rename = "as")]
    pub as_name: String,
    #[serde(flatten)]
    pub kind: ProjectKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ProjectKind {
    Github {
        repo: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        setup: Option<String>,
    },
    Mount {
        source: ProjectSource,
        /// Make the folder if it isn't there.
        #[serde(default, skip_serializing_if = "is_false")]
        make: bool,
    },
    Copy {
        source: ProjectSource,
    },
}

impl Project {
    /// The same project but for its id (what copying one checks).
    pub fn same_as(&self, other: &Project) -> bool {
        self.as_name == other.as_name && self.kind == other.kind
    }
}

/// The order a wadspace's desktop's icons are in ('app:<id>', 'web:<id>', 'variant:<id>', 'project:<id>').
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    #[serde(default)]
    pub icons: Vec<String>,
}

/// A wadspace's icon: a square of a picture (its background, or a picture
/// of its own: `picture`), as the picture fills a 16:10 screen: `zoom` 1 is
/// as tall as the picture, 2 half that; `x`, `y` its middle (0 to 1 across
/// and down).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IconLook {
    pub zoom: f64,
    pub x: f64,
    pub y: f64,
    /// A picture of its own for its icon (none: its background).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<Wallpaper>,
}

impl Default for IconLook {
    fn default() -> Self {
        IconLook { zoom: 1.0, x: 0.5, y: 0.5, picture: None }
    }
}

/// What a wadspace looks like and starts with: its one look (the spike's
/// default customization's default variant).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Look {
    #[serde(default)]
    pub wallpaper: Wallpaper,
    #[serde(default)]
    pub projects: Vec<Project>,
    /// What opens with it ('app:vscode', 'web:github').
    #[serde(default)]
    pub autostart: Vec<String>,
    /// What it starts straight into, full screen and nothing else (a gamespace's game: 'variant:stardew-valley').
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<String>,
    #[serde(default)]
    pub layout: Layout,
    /// Its icon's look (none: its picture, the middle of it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<IconLook>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_bars: Option<bool>,
}

// ------------------------------------------------------------ keys

/// The kind of a saved sign-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SecretKind {
    Web,
    Vscode,
    Obsidian,
    Steam,
    VscodeClaudeCode,
}

impl SecretKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SecretKind::Web => "web",
            SecretKind::Vscode => "vscode",
            SecretKind::Obsidian => "obsidian",
            SecretKind::Steam => "steam",
            SecretKind::VscodeClaudeCode => "vscode-claude-code",
        }
    }

    /// The app a key of this kind is for, if it's an app's own (VS Code's, Obsidian's, Steam's).
    pub fn app(self) -> Option<AppId> {
        match self {
            SecretKind::Vscode => Some(AppId::Vscode),
            SecretKind::Obsidian => Some(AppId::Obsidian),
            SecretKind::Steam => Some(AppId::Steam),
            SecretKind::Web | SecretKind::VscodeClaudeCode => None,
        }
    }
}

impl fmt::Display for SecretKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A key, as a wadspace names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyRef {
    pub id: String,
    pub name: String,
    pub kind: SecretKind,
}

/// Web apps (and 'browser', the browser itself) that sign in with one key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyGroup {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub members: Vec<String>,
    pub key: KeyRef,
}

/// What signs in with what: a key per slot ('app:vscode',
/// 'ext:anthropic.claude-code'), and the browser's groups.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Secrets {
    #[serde(default)]
    pub by_slot: IndexMap<String, KeyRef>,
    #[serde(default)]
    pub groups: Vec<KeyGroup>,
}

/// A saved sign-in (wadd's /v1/signins row), plus the sites a browser key signs in to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Secret {
    pub id: String,
    pub name: String,
    pub kind: SecretKind,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub size: u64,
    pub created_at: i64,
    #[serde(default)]
    pub changed_at: Option<i64>,
    #[serde(default)]
    pub used_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sites: Vec<String>,
}

impl Secret {
    /// As a wadspace names it.
    pub fn key_ref(&self) -> KeyRef {
        KeyRef { id: self.id.clone(), name: self.name.clone(), kind: self.kind }
    }
}

// ------------------------------------------------------------ wadspaces

/// A tag to organize wadspaces by, on the desktop (each wadspace has one; General has the rest).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    pub name: String,
    /// Its colour, for its group's header.
    pub color: String,
}

/// The launcher's tab a wadspace is under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Work,
    Gaming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandKind {
    Run,
    Copy,
}

impl CommandKind {
    /// As a Containerfile has it ('RUN').
    pub fn word(self) -> &'static str {
        match self {
            CommandKind::Run => "RUN",
            CommandKind::Copy => "COPY",
        }
    }
}

/// A step of its own in its image, as a Containerfile has it: RUN a command, or COPY a path on this machine into it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildCommand {
    pub id: String,
    /// What it's shown as: RUN_1, COPY_2 (numbered by kind, as they're added).
    pub name: String,
    pub kind: CommandKind,
    /// What follows RUN or COPY: the command, or `from to`.
    pub text: String,
}

/// A wadspace: a document in the account's Firestore. What a machine knows
/// about it on its own (whether it's built, how big) is `BuildState`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wadspace {
    /// Always `VERSION`.
    pub version: u32,
    pub id: String,
    /// The machine it lives on ('m-surface').
    pub machine: String,
    pub order: i64,
    pub name: String,
    #[serde(default)]
    pub apps: Vec<AppIn>,
    #[serde(default)]
    pub exts: Vec<Ext>,
    #[serde(default)]
    pub secrets: Secrets,
    #[serde(default)]
    pub look: Look,
    /// Its tag (a group on the desktop): one of the library's tags; none, the default (General).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// In the Trash since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<i64>,
    /// Agents in it may message this desktop and ask for sandboxes (the wadspaces MCP server).
    #[serde(default, skip_serializing_if = "is_false")]
    pub agents: bool,
    /// A sandbox: code an agent just wrote runs in it, so it gets nothing of
    /// the machine's, not its clipboard either.
    #[serde(default, skip_serializing_if = "is_false")]
    pub sealed: bool,
    /// When it was last launched (opened here, or streamed here from its machine): the launcher's Recent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_launched: Option<i64>,
    /// Comes with WadSpaces, built (the WadOS wadspace).
    #[serde(default, skip_serializing_if = "is_false")]
    pub starter: bool,
    /// The launcher's tab it's under (none: gaming if it has a game app, else work).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<Category>,
    /// Add-ons and downloads of its own, typed exactly: steps on top of its packages when it's built.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<BuildCommand>,
}

impl Wadspace {
    /// In the Trash.
    pub fn is_trashed(&self) -> bool {
        self.deleted_at.is_some()
    }

    pub fn has_app(&self, app: AppId) -> bool {
        self.apps.iter().any(|a| a.app == app)
    }

    pub fn ext(&self, app: AppId, id: &str) -> Option<&Ext> {
        self.exts.iter().find(|e| e.app == app && e.id == id)
    }

    /// Its launcher tab: what it says, or gaming if it has a game app (Steam, Dolphin).
    pub fn category_or_default(&self) -> Category {
        self.category.unwrap_or(if self.has_app(AppId::Steam) || self.has_app(AppId::Dolphin) {
            Category::Gaming
        } else {
            Category::Work
        })
    }
}

// ------------------------------------------------------------ what the machine knows on its own

/// Whether a wadspace's image is up to date with what it's made of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Built {
    #[default]
    Never,
    Update,
    Built,
}

/// A wadspace's image on one machine: not in its document (another machine's is its own).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildState {
    pub built: Built,
    pub size_mb: u64,
}

impl BuildState {
    /// It has to be built before it opens (never built, or out of date).
    pub fn needs_build(&self) -> bool {
        self.built != Built::Built
    }

    /// After an edit (an `Outcome`'s `needs_build`): a built image is out of date.
    pub fn edited(&mut self, needs_build: bool) {
        if needs_build && self.built == Built::Built {
            self.built = Built::Update;
        }
    }
}

/// How a wadspace on this machine is doing (the spike's WorkspaceState).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    #[default]
    Idle,
    Starting,
    Waiting,
    Ready,
    Stopping,
    Error,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunState {
    pub phase: Phase,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    pub since: i64,
    #[serde(default)]
    pub job: Option<String>,
}

// ------------------------------------------------------------ the browser, Steam, Dolphin

/// A web app the machine's browser keeps ("Create app from this page"), shown in the launcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserApp {
    pub id: String,
    pub name: String,
    pub url: String,
    pub profile: String,
}

/// One Steam account's library on this machine: one folder, mounted into every gamespace signed in with it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamLibrary {
    /// The games it owns (their ids, as in the catalog's Steam games).
    pub owned: Vec<String>,
    /// Downloaded into its folder (`true`).
    #[serde(default)]
    pub installed: IndexMap<String, bool>,
    /// The names of games the catalog doesn't know, by id.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub names: IndexMap<String, String>,
}

impl SteamLibrary {
    pub fn has(&self, game: &str) -> bool {
        self.installed.get(game).copied().unwrap_or(false)
    }
}

/// A Steam game staged as a package: ghcr.io/<GitHub login>/wadspaces-game-<id>, pushed by the machine that downloaded it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GamePackage {
    pub game: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub mb: u64,
    /// The machine that staged it.
    pub machine: String,
    pub at: i64,
}

/// A Dolphin game's disc, uploaded as a package: ghcr.io/<GitHub login>/wadspaces-disc-<id>.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscImage {
    /// Its variant's id ('mario-kart-double-dash').
    pub id: String,
    pub name: String,
    /// The file it came from ('mario-kart-double-dash.rvz').
    pub file: String,
    pub mb: u64,
    #[serde(rename = "ref")]
    pub reference: String,
    /// The machine it was uploaded from.
    pub machine: String,
    pub at: i64,
}

// ------------------------------------------------------------ streams

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StreamMode {
    Private,
    PrivateSecure,
    Public,
    PublicSecure,
}

impl StreamMode {
    /// Over WebRTC, at its public link (else over the tailnet).
    pub fn is_public(self) -> bool {
        matches!(self, StreamMode::Public | StreamMode::PublicSecure)
    }

    /// In a VM of its own (libkrun: `podman --runtime=krun`).
    pub fn is_secure(self) -> bool {
        matches!(self, StreamMode::PrivateSecure | StreamMode::PublicSecure)
    }
}

/// How far a stream is: its Stream job's stage while it starts (the job's
/// stage ids are these names), then live, then ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamStage {
    /// Starting its VM (a secure stream). The spike called it `kata`.
    #[serde(alias = "kata")]
    Vm,
    Container,
    Sidecar,
    Route,
    Turn,
    Serve,
    Live,
    Ended,
}

impl StreamStage {
    /// Its name ('sidecar'): a Stream job's stage id.
    pub fn as_str(self) -> &'static str {
        match self {
            StreamStage::Vm => "vm",
            StreamStage::Container => "container",
            StreamStage::Sidecar => "sidecar",
            StreamStage::Route => "route",
            StreamStage::Turn => "turn",
            StreamStage::Serve => "serve",
            StreamStage::Live => "live",
            StreamStage::Ended => "ended",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Transport {
    Websocket,
    Webrtc,
    WebrtcRelay,
}

/// What's mounted for one run or stream alone ("without saving").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ephemeral {
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<Wallpaper>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

/// One player's seat in a multiplayer stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamPlayer {
    /// 1 to 4: the player, and Dolphin's controller port.
    pub slot: u8,
    /// Its own key (Selkies' token for that slot): its link opens straight into it.
    pub key: String,
    /// Its one-tap link.
    pub link: String,
    /// Who's playing in it (a phone), or none: free.
    #[serde(default)]
    pub device: Option<String>,
    /// Its link was sent (copied or shared).
    #[serde(default, skip_serializing_if = "is_false")]
    pub sent: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
    pub id: String,
    /// The wadspace it streams.
    pub workspace: String,
    pub mode: StreamMode,
    /// The machine it streams from.
    pub machine: String,
    pub host: String,
    pub link: String,
    pub open_link: String,
    pub started: i64,
    pub last_active: i64,
    pub viewers: u32,
    pub live: bool,
    pub transport: Transport,
    /// A public stream that started with Cloudflare's TURN not answering: only viewers with a direct path connect.
    #[serde(default, skip_serializing_if = "is_false")]
    pub turn_down: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub stage: StreamStage,
    #[serde(default)]
    pub job: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral: Option<Ephemeral>,
    /// A multiplayer stream's seats (a gamespace's, public): each its own link.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub players: Vec<StreamPlayer>,
}

// ------------------------------------------------------------ sessions

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionDisplay {
    Screen,
    Window,
    Stream,
    Background,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InnerWindow {
    pub id: String,
    /// A catalog app ('vscode'), a web app ('web:github') or a project's folder ('project:iq').
    pub app: String,
    pub title: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Inner {
    #[serde(default)]
    pub windows: Vec<InnerWindow>,
    #[serde(default)]
    pub focused: Option<String>,
}

/// A wadspace running on this machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub ws: String,
    pub display: SessionDisplay,
    #[serde(default)]
    pub secure: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral: Option<Ephemeral>,
    #[serde(default)]
    pub inner: Inner,
    pub started_at: i64,
    /// Opened from an app or its variant with a wallpaper of its own: that, over its look's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<Wallpaper>,
    /// Full screen on a display other than the primary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<String>,
    /// Started straight into its launch: that app alone, no desktop around it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub kiosk: bool,
}

/// A focus: one wadspace on the screen for a time set, with no leaving it until then (or a restart).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Focus {
    pub ws: String,
    pub started: i64,
    pub until: i64,
}

// ------------------------------------------------------------ machines

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PeerKind {
    Desktop,
    Laptop,
    Server,
}

/// Another of the account's machines, on the tailnet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub kind: PeerKind,
    pub trusted: bool,
    pub online: bool,
    pub last_seen: i64,
    pub os: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<String>,
}

// ------------------------------------------------------------ jobs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    Pull,
    Build,
    Launch,
    Stream,
    Install,
    Update,
    ImportStick,
    Transfer,
    Tailnet,
    Boot,
    Github,
    Agent,
    CopyWadspace,
    Clone,
    SteamLogin,
    GameDownload,
    GameStage,
    DiscStage,
    /// Removing what nothing uses (Settings ▸ Storage's Clean Up).
    Cleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PartState {
    Waiting,
    Working,
    Done,
    Error,
    Skipped,
}

/// A step of a job, as the person sees it (a launch's parts, an install's steps, a stream's stages).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStage {
    pub id: String,
    pub label: String,
    pub state: PartState,
    /// Bytes to move, if it's a download or a copy.
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub done: u64,
    /// Lines it prints as it goes.
    #[serde(default)]
    pub lines: Vec<String>,
    /// It waits for you (signing in, in the browser), not for time.
    #[serde(default, skip_serializing_if = "is_false")]
    pub held: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// Who asked for something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Machine,
    Phone,
    Viewer,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub kind: JobKind,
    pub status: JobStatus,
    pub title: String,
    /// What it's for: a wadspace's id, a stream's id, a peer...
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub params: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub stages: Vec<JobStage>,
    #[serde(default)]
    pub cursor: usize,
    /// 0 to 1.
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub done_bytes: u64,
    #[serde(default)]
    pub total_bytes: u64,
    #[serde(default)]
    pub log: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
    pub started_at: i64,
    #[serde(default)]
    pub ended_at: Option<i64>,
    pub source: Source,
    /// Shown in the tray's jobs list (not boot's or the tailnet's own).
    #[serde(default)]
    pub visible: bool,
}

pub(crate) fn is_false(b: &bool) -> bool {
    !*b
}

/// `Option<Option<T>>` as a field that's absent (leave it), null (unset it) or a value (set it).
pub(crate) mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<T: Serialize, S: Serializer>(v: &Option<Option<T>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(inner) => inner.serialize(s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(d).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_ids_round_trip() {
        for a in AppId::ALL {
            assert_eq!(AppId::parse(a.as_str()), Some(a));
            assert_eq!(serde_json::to_string(&a).ok(), Some(format!("\"{a}\"")));
        }
        assert_eq!("krita".parse::<AppId>(), Ok(AppId::Krita));
        assert!("notepad".parse::<AppId>().is_err());
    }

    #[test]
    fn wallpapers_and_projects_keep_the_spikes_shape() {
        let wp = Wallpaper::image("/wallpapers/art/koi.svg");
        assert_eq!(
            serde_json::to_value(&wp).ok(),
            Some(serde_json::json!({"type": "image", "value": "/wallpapers/art/koi.svg"}))
        );
        assert_eq!(serde_json::to_value(Wallpaper::Empty).ok(), Some(serde_json::json!({"type": "empty"})));
        // A gradient or a texture isn't a background any more.
        assert!(serde_json::from_value::<Wallpaper>(serde_json::json!({"type": "texture", "value": "stars"})).is_err());
        let p: Project = serde_json::from_value(serde_json::json!(
            {"id": "mount-thesis", "kind": "mount", "source": {"machineId": "m-surface", "paths": ["~/Thesis"]}, "as": "Thesis", "make": true}
        ))
        .expect("a mount");
        assert_eq!(p.as_name, "Thesis");
        assert!(matches!(p.kind, ProjectKind::Mount { make: true, .. }));
        let back = serde_json::to_value(&p).expect("serializes");
        assert_eq!(back["as"], "Thesis");
        assert_eq!(back["kind"], "mount");
        assert_eq!(back["source"]["machineId"], "m-surface");
    }

    #[test]
    fn a_built_image_goes_out_of_date_with_an_edit_that_needs_a_build() {
        let mut s = BuildState { built: Built::Built, size_mb: 2000 };
        s.edited(false);
        assert!(!s.needs_build());
        s.edited(true);
        assert_eq!(s.built, Built::Update);
        assert!(s.needs_build());
        let mut never = BuildState::default();
        never.edited(true);
        assert_eq!(never.built, Built::Never);
    }
}
