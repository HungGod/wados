// An app's variants: what the Apps window shows inside each of a
// wadspace's apps, and what Alt+Space searches. They're the app itself, its
// add-ons (VS Code extensions, terminal tools, the browser's web apps and
// Focus), its named variants (Dolphin's Mario Kart, Steam's Stardew Valley)
// and its sign-ins. Opening one opens its wadspace with that in front, on
// its wallpaper (or its app's) if it has one.

use serde::{Deserialize, Serialize};

use crate::catalog::{GAME_APPS, app_name, ext_name};
use crate::ops::keys_of;
use crate::types::{AppId, Ext, KeyRef, SecretKind, Wadspace, Wallpaper};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VariantKind {
    App,
    Web,
    Mode,
    Extension,
    Tool,
    Game,
    Named,
    SignIn,
}

impl VariantKind {
    pub fn label(self) -> &'static str {
        match self {
            VariantKind::App => "App",
            VariantKind::Web => "Web app",
            VariantKind::Mode => "Mode",
            VariantKind::Extension => "Extension",
            VariantKind::Tool => "Tool",
            VariantKind::Game => "Game",
            VariantKind::Named => "Variant",
            VariantKind::SignIn => "Sign-in",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppVariant {
    /// 'app:vscode', 'ext:wadbrowser:gmail', 'key:app:vscode'.
    pub id: String,
    pub app: AppId,
    pub kind: VariantKind,
    pub name: String,
    /// The window it opens inside its wadspace ('vscode', 'web:gmail', 'variant:mario-kart').
    pub opens: String,
    /// Its own wallpaper (not its app's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<Wallpaper>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<Ext>,
    /// A sign-in's slot ('app:vscode', 'ext:anthropic.claude-code', 'group:…').
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    /// A sign-in's key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
}

fn from_ext(e: &Ext) -> AppVariant {
    let (kind, opens) = match e.app {
        AppId::Wadbrowser if e.url.is_some() => (VariantKind::Web, format!("web:{}", e.id)),
        AppId::Wadbrowser => (VariantKind::Mode, "wadbrowser".to_string()),
        AppId::Vscode => (VariantKind::Extension, "vscode".to_string()),
        AppId::Terminal => (VariantKind::Tool, "terminal".to_string()),
        app => {
            let kind = if GAME_APPS.contains(&app) { VariantKind::Game } else { VariantKind::Named };
            (kind, format!("variant:{}", e.id))
        }
    };
    AppVariant {
        id: format!("ext:{}:{}", e.app, e.id),
        app: e.app,
        kind,
        name: ext_name(e),
        opens,
        wallpaper: e.wallpaper.clone(),
        ext: Some(e.clone()),
        slot: None,
        key_id: None,
    }
}

/// The kinds of sign-in an app in `x` can keep (Claude Code's goes with VS Code, once it has it).
pub fn sign_in_kinds(x: &Wadspace, app: AppId) -> Vec<SecretKind> {
    match app {
        AppId::Wadbrowser => vec![SecretKind::Web],
        AppId::Vscode if x.exts.iter().any(|e| e.id == "anthropic.claude-code") => {
            vec![SecretKind::Vscode, SecretKind::VscodeClaudeCode]
        }
        AppId::Vscode => vec![SecretKind::Vscode],
        AppId::Obsidian => vec![SecretKind::Obsidian],
        AppId::Steam => vec![SecretKind::Steam],
        _ => vec![],
    }
}

/// The kind of key it signs in with, if it can have one: an app's own
/// (WadBrowser's is a web key), a web app's (a web key), Claude Code's, or a
/// game's or named variant's (its app's kind: another Steam account).
pub fn key_kind_of(v: &AppVariant) -> Option<SecretKind> {
    match v.kind {
        VariantKind::SignIn => None,
        VariantKind::App if v.app == AppId::Wadbrowser => Some(SecretKind::Web),
        VariantKind::App => v.app.key_kind(),
        VariantKind::Web => Some(SecretKind::Web),
        VariantKind::Extension if v.ext.as_ref().is_some_and(|e| e.id == "anthropic.claude-code") => {
            Some(SecretKind::VscodeClaudeCode)
        }
        VariantKind::Game | VariantKind::Named => {
            v.app.key_kind().filter(|k| matches!(k, SecretKind::Obsidian | SecretKind::Steam))
        }
        _ => None,
    }
}

/// Without a key of its own, it signs in as its app does (a web app, a game); Claude Code doesn't.
pub fn inherits(v: &AppVariant) -> bool {
    matches!(v.kind, VariantKind::Web | VariantKind::Game | VariantKind::Named)
}

/// The key an app's own sign-in is (WadBrowser's: its group with the browser in it).
pub fn app_key(x: &Wadspace, app: AppId) -> Option<&KeyRef> {
    if app == AppId::Wadbrowser {
        return x.secrets.groups.iter().find(|g| g.members.iter().any(|m| m == "browser")).map(|g| &g.key);
    }
    x.secrets.by_slot.get(&format!("app:{app}"))
}

/// What a variant signs in with: a key, its own or its app's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignIn {
    pub key: KeyRef,
    /// Its own (else its app's).
    pub own: bool,
}

/// What it signs in with in `x`: its own key, or its app's (`own` false).
pub fn sign_in_of(x: &Wadspace, v: &AppVariant) -> Option<SignIn> {
    key_kind_of(v)?;
    let app = app_key(x, v.app);
    if v.kind == VariantKind::App {
        return app.map(|k| SignIn { key: k.clone(), own: true });
    }
    let id = v.ext.as_ref().map(|e| e.id.as_str()).unwrap_or_default();
    let own = if v.kind == VariantKind::Web {
        let m = format!("web:{id}");
        x.secrets
            .groups
            .iter()
            .find(|g| !g.members.iter().any(|y| y == "browser") && g.members.contains(&m))
            .map(|g| &g.key)
    } else {
        x.secrets.by_slot.get(&format!("ext:{id}"))
    };
    if let Some(k) = own {
        return Some(SignIn { key: k.clone(), own: true });
    }
    if inherits(v) { app.map(|k| SignIn { key: k.clone(), own: false }) } else { None }
}

/// The app a sign-in's slot belongs to.
fn app_of_slot(x: &Wadspace, slot: &str) -> Option<AppId> {
    if let Some(a) = slot.strip_prefix("app:") {
        return AppId::parse(a);
    }
    if let Some(e) = slot.strip_prefix("ext:") {
        return x.exts.iter().find(|y| y.id == e).map(|y| y.app);
    }
    slot.starts_with("group").then_some(AppId::Wadbrowser)
}

/// An app's variants in `x`: the app, then its add-ons and named variants, then its sign-ins.
pub fn variants_of(x: &Wadspace, app: AppId) -> Vec<AppVariant> {
    let Some(a) = x.apps.iter().find(|y| y.app == app) else { return vec![] };
    let mut out = vec![AppVariant {
        id: format!("app:{app}"),
        app,
        kind: VariantKind::App,
        name: app_name(app).to_string(),
        opens: app.as_str().to_string(),
        wallpaper: a.wallpaper.clone(),
        ext: None,
        slot: None,
        key_id: None,
    }];
    out.extend(x.exts.iter().filter(|e| e.app == app).map(from_ext));
    out.extend(keys_of(x).into_iter().filter(|(slot, _)| app_of_slot(x, slot) == Some(app)).map(|(slot, key)| {
        AppVariant {
            id: format!("key:{slot}"),
            app,
            kind: VariantKind::SignIn,
            name: key.name,
            opens: app.as_str().to_string(),
            wallpaper: None,
            ext: None,
            slot: Some(slot),
            key_id: Some(key.id),
        }
    }));
    out
}

/// One of `x`'s variants, by its id.
pub fn find_variant(x: &Wadspace, id: &str) -> Option<AppVariant> {
    x.apps.iter().find_map(|a| variants_of(x, a.app).into_iter().find(|v| v.id == id))
}

/// The wallpaper opening it gives: its own, else its app's.
pub fn wallpaper_for<'a>(x: &'a Wadspace, v: &'a AppVariant) -> Option<&'a Wallpaper> {
    v.wallpaper.as_ref().or_else(|| x.apps.iter().find(|a| a.app == v.app).and_then(|a| a.wallpaper.as_ref()))
}

/// A variant, with its wadspace.
#[derive(Debug, Clone, PartialEq)]
pub struct Found<'a> {
    pub ws: &'a Wadspace,
    pub variant: AppVariant,
}

/// Every variant of every app in `machine`'s wadspaces (not the Trash's), in their order, for Alt+Space (sign-ins only open their app, so they're left out).
pub fn every_variant<'a>(list: &'a [Wadspace], machine: &str) -> Vec<Found<'a>> {
    let mut mine: Vec<&Wadspace> = list.iter().filter(|x| !x.is_trashed() && x.machine == machine).collect();
    mine.sort_by_key(|x| x.order);
    mine.into_iter()
        .flat_map(|x| {
            x.apps.iter().flat_map(move |a| {
                variants_of(x, a.app)
                    .into_iter()
                    .filter(|v| v.kind != VariantKind::SignIn)
                    .map(move |variant| Found { ws: x, variant })
            })
        })
        .collect()
}

/// Alt+Space's matches for `q`: names starting with it first, then a word in them starting with it, then containing it, then by app or wadspace.
pub fn search<'a>(list: &'a [Wadspace], machine: &str, q: &str, limit: usize) -> Vec<Found<'a>> {
    let all = every_variant(list, machine);
    let t = q.trim().to_lowercase();
    if t.is_empty() {
        return all.into_iter().take(limit).collect();
    }
    let score = |f: &Found| -> Option<u8> {
        let n = f.variant.name.to_lowercase();
        if n.starts_with(&t) {
            Some(0)
        } else if n.split(|c: char| c.is_whitespace() || matches!(c, ':' | '·' | '-')).any(|w| w.starts_with(&t)) {
            Some(1)
        } else if n.contains(&t) {
            Some(2)
        } else if app_name(f.variant.app).to_lowercase().contains(&t) {
            Some(3)
        } else if f.ws.name.to_lowercase().contains(&t) {
            Some(4)
        } else {
            None
        }
    };
    let mut scored: Vec<(u8, Found)> = all.into_iter().filter_map(|f| score(&f).map(|s| (s, f))).collect();
    scored.sort_by_key(|(s, _)| *s);
    scored.into_iter().map(|(_, f)| f).take(limit).collect()
}
