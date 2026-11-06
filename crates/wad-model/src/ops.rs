// A wadspace's makeup, edited: its apps, add-ons (and the apps' named
// variants: games and the like), keys and look. Pure. It follows
// wad-board's v5 rules where v5 had them (fixtures/board/b.json):
//   - an add-on brings its app; apps and add-ons copy on their own;
//   - a section dropped into another wadspace merges into it;
//   - a section dropped on empty space is a new wadspace with only that.
// Removing an app takes its add-ons (and their sign-ins) too, and whatever
// is taken out goes from its look as well.

use serde::{Deserialize, Serialize};

use crate::catalog::{self, app_name, extension, focus, is_named, is_named_app, site, steam_game, tool};
use crate::look::prune;
use crate::names::{new_id, numbered_name, slug, truncate};
use crate::types::{
    AppId, AppIn, CommandKind, Ext, KeyGroup, KeyRef, Look, Project, ProjectKind, Secret, SecretKind, VERSION,
    Wadspace, Wallpaper,
};

/// A section of a wadspace, as the Builder's stack shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Section {
    Apps,
    Addons,
    Secrets,
    /// Its look: its picture, its icon and its files (the spike's "customizations").
    #[serde(alias = "customizations")]
    Look,
}

impl Section {
    pub fn label(self) -> &'static str {
        match self {
            Section::Apps => "Apps",
            Section::Addons => "Add-ons",
            Section::Secrets => "Secrets",
            Section::Look => "Customization",
        }
    }
}

/// An add-on's key in its wadspace ('vscode:anthropic.claude-code').
pub fn ext_key(e: &Ext) -> String {
    format!("{}:{}", e.app, e.id)
}

/// The add-on an item names ('vscode:anthropic.claude-code', 'terminal:rust',
/// 'wadbrowser:github', 'wadbrowser:focus', 'steam:hades'), looking in
/// `from`'s add-ons for a web app the catalog doesn't know, or a named variant.
pub fn ext_of(item: &str, from: Option<&Wadspace>) -> Option<Ext> {
    let (app, id) = item.split_once(':')?;
    let app = AppId::parse(app)?;
    match app {
        AppId::Vscode => extension(id).map(|_| Ext::new(app, id)),
        AppId::Terminal => tool(id).map(|_| Ext::new(app, id)),
        AppId::Wadbrowser => {
            if id == "focus" {
                return Some(focus());
            }
            let mut e = Ext::new(app, id);
            if let Some(known) = from.and_then(|f| f.ext(app, id)) {
                e.name.clone_from(&known.name);
                e.url.clone_from(&known.url);
            } else {
                let s = site(id)?;
                e.name = Some(s.name.to_string());
                e.url = Some(s.url.to_string());
            }
            Some(e)
        }
        _ => {
            // A named variant of any other app (a game): as `from` has it, or a Steam game from the store.
            if let Some(named) = from.and_then(|f| f.ext(app, id)).filter(|e| is_named(e)) {
                return Some(named.clone());
            }
            let game = if app == AppId::Steam { steam_game(id) } else { None }?;
            Some(Ext::new(AppId::Steam, game.id.clone()).named(game.name.clone()))
        }
    }
}

/// An app into `x` (and onto its desktop, but the terminal). False if it's there.
pub fn add_app(x: &mut Wadspace, a: AppId) -> bool {
    if x.has_app(a) {
        return false;
    }
    x.apps.push(AppIn::new(a));
    let icon = format!("app:{a}");
    if a != AppId::Terminal && !x.look.layout.icons.contains(&icon) {
        x.look.layout.icons.push(icon);
    }
    true
}

/// An add-on into `x`, bringing its app. False if it's there.
pub fn add_ext(x: &mut Wadspace, e: &Ext) -> bool {
    add_app(x, e.app);
    if x.exts.iter().any(|y| y.app == e.app && y.id == e.id) {
        return false;
    }
    x.exts.push(e.clone());
    let icon = if e.app == AppId::Wadbrowser && e.url.is_some() {
        Some(format!("web:{}", e.id))
    } else if is_named(e) {
        Some(format!("variant:{}", e.id))
    } else {
        None
    };
    if let Some(icon) = icon
        && !x.look.layout.icons.contains(&icon)
    {
        x.look.layout.icons.push(icon);
    }
    true
}

/// A named variant of an app (a game, a profile), bringing the app if it isn't there. None if it has no name, or the app has no variants of its own.
pub fn add_named(x: &mut Wadspace, a: AppId, name: &str) -> Option<Ext> {
    let nm = name.trim();
    if nm.is_empty() || !is_named_app(a) {
        return None;
    }
    let taken: Vec<&str> = x.exts.iter().map(|e| e.id.as_str()).collect();
    let mut base = slug(nm);
    if base.is_empty() {
        base = "variant".to_string();
    }
    let id = new_id(&taken, &base);
    let names: Vec<String> = x.exts.iter().filter(|y| y.app == a).map(catalog::ext_name).collect();
    let e = Ext::new(a, id.clone()).named(numbered_name(&names, nm));
    add_app(x, a);
    x.exts.push(e.clone());
    let icon = format!("variant:{id}");
    if !x.look.layout.icons.contains(&icon) {
        x.look.layout.icons.push(icon);
    }
    Some(e)
}

/// The slot a key goes in, in `x`: its app's, Claude Code's, or the browser's group.
pub fn slot_for(x: &Wadspace, kind: SecretKind) -> Option<String> {
    match kind {
        SecretKind::Web => x.has_app(AppId::Wadbrowser).then(|| "browser".to_string()),
        SecretKind::VscodeClaudeCode => {
            x.exts.iter().any(|e| e.id == "anthropic.claude-code").then(|| "ext:anthropic.claude-code".to_string())
        }
        k => k.app().filter(|a| x.has_app(*a)).map(|a| format!("app:{a}")),
    }
}

/// Give `x` a key: into the slot it fits, or (a web key) the browser's group. A key for an app that isn't there brings it (as an add-on brings its app).
pub fn set_key(x: &mut Wadspace, key: &KeyRef) -> Option<String> {
    if key.kind == SecretKind::Web {
        add_app(x, AppId::Wadbrowser);
        if let Some(g) = x.secrets.groups.iter_mut().find(|g| g.members.iter().any(|m| m == "browser")) {
            g.key = key.clone();
        } else {
            // The browser, and its web apps that don't sign in with a key of their own.
            let own: Vec<&String> = x.secrets.groups.iter().flat_map(|g| &g.members).collect();
            let mut members = vec!["browser".to_string()];
            members.extend(
                x.exts
                    .iter()
                    .filter(|e| e.app == AppId::Wadbrowser && e.url.is_some())
                    .map(|e| format!("web:{}", e.id))
                    .filter(|m| !own.contains(&m)),
            );
            let ids: Vec<&str> = x.secrets.groups.iter().map(|g| g.id.as_deref().unwrap_or("")).collect();
            let id = new_id(&ids, &format!("g-{}", key.id));
            x.secrets.groups.push(KeyGroup { id: Some(id), members, key: key.clone() });
        }
        return Some("browser".to_string());
    }
    if let Some(slot) = slot_for(x, key.kind) {
        x.secrets.by_slot.insert(slot.clone(), key.clone());
        return Some(slot);
    }
    match key.kind {
        SecretKind::VscodeClaudeCode => {
            add_ext(x, &Ext::new(AppId::Vscode, "anthropic.claude-code"));
        }
        k => {
            add_app(x, k.app()?);
        }
    }
    let slot = slot_for(x, key.kind)?;
    x.secrets.by_slot.insert(slot.clone(), key.clone());
    Some(slot)
}

/// Every key `x` uses, with where: its slots', then its groups' ('group:<id>', or 'group' for one without an id).
pub fn keys_of(x: &Wadspace) -> Vec<(String, KeyRef)> {
    let mut out: Vec<(String, KeyRef)> = x.secrets.by_slot.iter().map(|(s, k)| (s.clone(), k.clone())).collect();
    out.extend(x.secrets.groups.iter().map(|g| {
        let slot = g.id.as_ref().map_or_else(|| "group".to_string(), |id| format!("group:{id}"));
        (slot, g.key.clone())
    }));
    out
}

fn count(n: usize, one: &str) -> String {
    if n == 0 {
        "nothing new".to_string()
    } else if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {one}s")
    }
}

/// `from`'s look given to `into`: its picture (and the icon cut from it), and its files and folders beside `into`'s own.
pub fn merge_look(into: &mut Wadspace, from: &Wadspace) {
    let l = &from.look;
    if crate::wallpapers::has_picture(&l.wallpaper) {
        into.look.wallpaper = l.wallpaper.clone();
        into.look.icon.clone_from(&l.icon);
    }
    if l.title_bars.is_some() {
        into.look.title_bars = l.title_bars;
    }
    for p in &l.projects {
        if into.look.projects.iter().any(|q| q.same_as(p)) {
            continue;
        }
        let taken: Vec<&str> = into.look.projects.iter().map(|q| q.id.as_str()).collect();
        let id = new_id(&taken, &p.id);
        into.look.layout.icons.push(format!("project:{id}"));
        into.look.projects.push(Project { id, ..p.clone() });
    }
}

/// A whole section of `from`, merged into `into`. Returns what changed (for the toast).
pub fn merge_section(into: &mut Wadspace, from: &Wadspace, s: Section) -> String {
    match s {
        Section::Apps => count(from.apps.iter().filter(|a| add_app(into, a.app)).count(), "app"),
        Section::Addons => count(from.exts.iter().filter(|e| add_ext(into, e)).count(), "add-on"),
        Section::Secrets => {
            let mut n = 0;
            for (slot, key) in &from.secrets.by_slot {
                into.secrets.by_slot.insert(slot.clone(), key.clone());
                n += 1;
            }
            for g in &from.secrets.groups {
                let at =
                    into.secrets.groups.iter().position(|y| y.id == g.id || y.members.iter().any(|m| m == "browser"));
                match at {
                    Some(i) => into.secrets.groups[i] = g.clone(),
                    None => into.secrets.groups.push(g.clone()),
                }
                n += 1;
            }
            count(n, "key")
        }
        Section::Look => {
            merge_look(into, from);
            "the look".to_string()
        }
    }
}

/// A new, empty wadspace: no background yet (its icon is empty glass until one's picked).
pub fn blank(id: &str, name: &str, machine: &str, now: i64, order: i64) -> Wadspace {
    Wadspace {
        version: VERSION,
        id: id.to_string(),
        machine: machine.to_string(),
        order,
        name: name.to_string(),
        apps: Vec::new(),
        exts: Vec::new(),
        secrets: Default::default(),
        look: Look::default(),
        tag: None,
        created_at: now,
        updated_at: now,
        deleted_at: None,
        agents: false,
        sealed: false,
        last_launched: None,
        starter: false,
        category: None,
        commands: Vec::new(),
    }
}

/// A new wadspace (`target`, blank) with only `from`'s section in it, named for it ("IntelligenceQuest · Apps").
pub fn from_section(target: &mut Wadspace, from: &Wadspace, s: Section) {
    merge_section(target, from, s);
    target.name = truncate(&format!("{} · {}", from.name, s.label()), crate::check::MAX_NAME);
}

/// What adding something did or would do: whether it went in, and what to say ("Adds Claude Code; VS Code comes too").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddResult {
    pub ok: bool,
    pub said: String,
}

impl AddResult {
    fn yes(said: impl Into<String>) -> Self {
        AddResult { ok: true, said: said.into() }
    }

    fn no(said: impl Into<String>) -> Self {
        AddResult { ok: false, said: said.into() }
    }
}

/// What adding `item` would do (no change made).
pub fn describe_add(x: &Wadspace, s: Section, item: &str, keys: &[Secret], from: Option<&Wadspace>) -> AddResult {
    add_item_inner(&mut x.clone(), s, item, keys, from, true)
}

/// Add one thing to a section of `x`:
///   apps     'vscode'
///   addons   'vscode:anthropic.claude-code', 'terminal:rust', 'wadbrowser:github', 'wadbrowser:focus', 'steam:hades'
///   secrets  a key's id
///   look     'wallpaper:image:<path>', 'wallpaper:color:<colour>', 'wallpaper:empty', 'repo:<owner/name>', 'look:<wadspace>' (`from`'s look)
pub fn add_item(x: &mut Wadspace, s: Section, item: &str, keys: &[Secret], from: Option<&Wadspace>) -> AddResult {
    add_item_inner(x, s, item, keys, from, false)
}

fn add_item_inner(
    x: &mut Wadspace,
    s: Section,
    item: &str,
    keys: &[Secret],
    from: Option<&Wadspace>,
    dry: bool,
) -> AddResult {
    match s {
        Section::Apps => {
            let Some(a) = AppId::parse(item) else { return AddResult::no("Not an app WadSpaces has") };
            if x.has_app(a) {
                return AddResult::no(format!("{} is already in it", app_name(a)));
            }
            add_app(x, a);
            AddResult::yes(format!("Adds {}", app_name(a)))
        }
        Section::Addons => {
            let Some(e) = ext_of(item, from) else { return AddResult::no("Not an add-on") };
            if x.ext(e.app, &e.id).is_some() {
                return AddResult::no("It’s already in it");
            }
            let brings = (!x.has_app(e.app)).then(|| app_name(e.app));
            add_ext(x, &e);
            let name = catalog::ext_name(&e);
            match brings {
                Some(app) => AddResult::yes(format!("Adds {name}; {app} comes too")),
                None => AddResult::yes(format!("Adds {name}")),
            }
        }
        Section::Secrets => {
            let Some(k) = keys.iter().find(|k| k.id == item) else { return AddResult::no("No such key") };
            match set_key(x, &k.key_ref()) {
                None => AddResult::no(format!("Nothing in it signs in with a {} key", k.kind)),
                Some(slot) if slot == "browser" => {
                    AddResult::yes(format!("The browser and its web apps sign in with {}", k.name))
                }
                Some(slot) => AddResult::yes(format!("{} signs in with {}", slot_label(&slot), k.name)),
            }
        }
        Section::Look => {
            if let Some(ws) = item.strip_prefix("look:") {
                let Some(from) = from.filter(|f| f.id == ws) else { return AddResult::no("That look is gone") };
                merge_look(x, from);
                return AddResult::yes(format!("Adds {}’s look", from.name));
            }
            if let Some(rest) = item.strip_prefix("wallpaper:") {
                let wp = match rest.split_once(':') {
                    Some(("image", v)) if !v.is_empty() => Wallpaper::image(v),
                    Some(("color", v)) if !v.is_empty() => Wallpaper::color(v),
                    None if rest == "empty" => Wallpaper::Empty,
                    _ => return AddResult::no("That isn’t a background"),
                };
                x.look.wallpaper = wp;
                return AddResult::yes(format!("{}’s wallpaper", x.name));
            }
            if let Some(repo) = item.strip_prefix("repo:") {
                let Some((owner, name)) = repo.split_once('/').filter(|(o, n)| is_repo_part(o) && is_repo_part(n))
                else {
                    return AddResult::no("That isn’t a repository");
                };
                let _ = owner;
                let has =
                    x.look.projects.iter().any(|p| matches!(&p.kind, ProjectKind::Github { repo: r, .. } if r == repo));
                if has {
                    return AddResult::no(format!("{name} is already in {}", x.name));
                }
                let taken: Vec<&str> = x.look.projects.iter().map(|p| p.id.as_str()).collect();
                let id = new_id(&taken, &format!("gh-{}", name.to_lowercase()));
                x.look.layout.icons.push(format!("project:{id}"));
                x.look.projects.push(Project {
                    id,
                    as_name: name.to_string(),
                    kind: ProjectKind::Github { repo: repo.to_string(), setup: None },
                });
                return AddResult::yes(format!("Clones {repo} into {}", x.name));
            }
            if dry { AddResult::yes("Adds it") } else { AddResult::no("It doesn’t go there") }
        }
    }
}

/// One part of a GitHub repository's name ('HungGod', 'IntelligenceQuest').
pub(crate) fn is_repo_part(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s != "."
        && s != ".."
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// A slot as words ("VS Code", "Claude Code", "The browser").
pub fn slot_label(slot: &str) -> String {
    if slot == "browser" || slot.starts_with("group") {
        return "The browser".to_string();
    }
    if let Some(a) = slot.strip_prefix("app:") {
        return AppId::parse(a).map_or_else(|| a.to_string(), |a| app_name(a).to_string());
    }
    if let Some(e) = slot.strip_prefix("ext:") {
        return extension(e).map_or_else(|| e.to_string(), |x| x.name.clone());
    }
    if let Some(w) = slot.strip_prefix("web:") {
        return site(w).map_or_else(|| w.to_string(), |s| s.name.to_string());
    }
    slot.to_string()
}

/// Take one thing out of a section (an app: with its add-ons and their
/// sign-ins; an add-on: with its sign-in; a key: by its slot, 'app:vscode',
/// 'ext:…', 'group:<id>'; the look: a project, 'project:<id>'). Whatever it
/// was goes from the look too. False if it wasn't there.
pub fn remove_item(x: &mut Wadspace, s: Section, item: &str) -> bool {
    let done = match s {
        Section::Apps => {
            let Some(a) = AppId::parse(item).filter(|a| x.has_app(*a)) else { return false };
            x.apps.retain(|y| y.app != a);
            let gone: Vec<Ext> = x.exts.iter().filter(|e| e.app == a).cloned().collect();
            x.exts.retain(|e| e.app != a);
            x.secrets.by_slot.shift_remove(&format!("app:{a}"));
            for e in &gone {
                drop_ext_refs(x, e);
            }
            if a == AppId::Wadbrowser {
                x.secrets.groups.clear();
            }
            true
        }
        Section::Addons => {
            let Some(e) = x.exts.iter().find(|e| ext_key(e) == item).cloned() else { return false };
            x.exts.retain(|y| ext_key(y) != item);
            drop_ext_refs(x, &e);
            true
        }
        Section::Secrets => {
            if let Some(rest) = item.strip_prefix("group") {
                let gid = rest.strip_prefix(':');
                let before = x.secrets.groups.len();
                x.secrets.groups.retain(|g| g.id.as_deref().unwrap_or("") != gid.unwrap_or(""));
                x.secrets.groups.len() < before
            } else {
                x.secrets.by_slot.shift_remove(item).is_some()
            }
        }
        Section::Look => {
            let Some(pid) = item.strip_prefix("project:") else { return false };
            let before = x.look.projects.len();
            x.look.projects.retain(|p| p.id != pid);
            x.look.projects.len() < before
        }
    };
    if done {
        prune(x);
    }
    done
}

/// An add-on's sign-in and its place in the browser's groups, gone with it (a group left with nothing in it goes too).
fn drop_ext_refs(x: &mut Wadspace, e: &Ext) {
    if x.exts.iter().any(|y| y.id == e.id) {
        return;
    }
    x.secrets.by_slot.shift_remove(&format!("ext:{}", e.id));
    if e.app == AppId::Wadbrowser {
        let m = format!("web:{}", e.id);
        for g in &mut x.secrets.groups {
            g.members.retain(|y| *y != m);
        }
        x.secrets.groups.retain(|g| !g.members.is_empty());
    }
}

/// Does editing this section mean the wadspace has to be built again? (Its image is its apps and add-ons.)
pub fn needs_build(s: Section) -> bool {
    matches!(s, Section::Apps | Section::Addons)
}

/// Its apps, its add-ons (app and id) and its commands (kind and text).
type Image<'a> = (Vec<AppId>, Vec<(AppId, &'a str)>, Vec<(CommandKind, &'a str)>);

/// What a wadspace's image is made of: its apps, the add-ons built into it
/// (VS Code's extensions and the command line's tools: packages), and its
/// own commands, in order. Web apps, games and discs aren't: they're its
/// apps' content, put in when it starts (`/run/wadspaces-extra`), so adding
/// one never rebuilds it (wad-gen's set key agrees: `tests/rebuilds.rs`).
fn image(x: &Wadspace) -> Image<'_> {
    let mut apps: Vec<AppId> = x.apps.iter().map(|a| a.app).collect();
    apps.sort();
    let mut exts: Vec<(AppId, &str)> = x
        .exts
        .iter()
        .filter(|e| !is_named(e) && matches!(e.app, AppId::Vscode | AppId::Terminal))
        .map(|e| (e.app, e.id.as_str()))
        .collect();
    exts.sort();
    let commands = x.commands.iter().map(|c| (c.kind, c.text.as_str())).collect();
    (apps, exts, commands)
}

/// Are `a` and `b` built into the same image? (If not, a change from one to the other needs a build.)
pub fn same_image(a: &Wadspace, b: &Wadspace) -> bool {
    image(a) == image(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Secret;

    fn key(id: &str, name: &str, kind: SecretKind) -> Secret {
        Secret {
            id: id.into(),
            name: name.into(),
            kind,
            paths: vec![],
            size: 0,
            created_at: 0,
            changed_at: None,
            used_by: vec![],
            sites: vec![],
        }
    }

    fn keys() -> Vec<Secret> {
        vec![
            key("work", "Work", SecretKind::Web),
            key("cc", "Claude", SecretKind::VscodeClaudeCode),
            key("vault", "Vault", SecretKind::Obsidian),
        ]
    }

    fn apps(x: &Wadspace) -> Vec<&str> {
        x.apps.iter().map(|a| a.app.as_str()).collect()
    }

    fn exts(x: &Wadspace) -> Vec<String> {
        x.exts.iter().map(ext_key).collect()
    }

    /// b.json's IntelligenceQuest, Cosmic Bodybuilding and VanuaAcademy, as v5 had them.
    fn board() -> (Wadspace, Wadspace, Wadspace) {
        let mut iq = blank("iq", "IntelligenceQuest", "surface", 0, 1);
        for a in [AppId::Wadbrowser, AppId::Vscode, AppId::Terminal, AppId::Libresprite, AppId::Tiled] {
            add_app(&mut iq, a);
        }
        for e in ["wadbrowser:github", "vscode:anthropic.claude-code", "vscode:ms-vscode.cpptools", "terminal:cpp"] {
            add_item(&mut iq, Section::Addons, e, &[], None);
        }
        iq.look.wallpaper = Wallpaper::image("/wallpapers/art/tide.svg");
        iq.look.projects.push(Project {
            id: "project-iq".into(),
            as_name: "IntelligenceQuest".into(),
            kind: ProjectKind::Github { repo: "HungGod/IntelligenceQuest".into(), setup: None },
        });
        iq.secrets.by_slot.insert(
            "ext:anthropic.claude-code".into(),
            KeyRef { id: "iq-claude".into(), name: "IQ-claude".into(), kind: SecretKind::VscodeClaudeCode },
        );
        iq.secrets.groups.push(KeyGroup {
            id: Some("g1".into()),
            members: vec!["browser".into(), "web:github".into()],
            key: KeyRef { id: "hunggod".into(), name: "hunggod".into(), kind: SecretKind::Web },
        });
        let mut cosmic = blank("cosmic", "Cosmic Bodybuilding", "surface", 0, 2);
        add_app(&mut cosmic, AppId::Obsidian);
        add_app(&mut cosmic, AppId::Wadbrowser);
        let mut vanua = blank("vanua", "VanuaAcademy", "surface", 0, 3);
        add_app(&mut vanua, AppId::Vscode);
        (iq, cosmic, vanua)
    }

    #[test]
    fn an_add_on_brings_its_app_apps_and_add_ons_on_their_own() {
        let (iq, mut cosmic, mut vanua) = board();
        // An add-on dropped in brings its app.
        add_item(&mut cosmic, Section::Addons, "vscode:ms-vscode.cpptools", &keys(), Some(&iq));
        assert!(apps(&cosmic).contains(&"vscode"));
        assert!(exts(&cosmic).contains(&"vscode:ms-vscode.cpptools".to_string()));
        // Focus from the catalog brings the browser.
        add_item(&mut vanua, Section::Addons, "wadbrowser:focus", &keys(), None);
        assert!(apps(&vanua).contains(&"wadbrowser"));
        // A section of add-ons, and of apps, on their own.
        merge_section(&mut vanua, &iq, Section::Addons);
        merge_section(&mut cosmic, &iq, Section::Apps);
        add_item(&mut vanua, Section::Apps, "krita", &keys(), None);
        // Apps match v5's result (v5 then removes VS Code from Vanua; v6 takes its add-ons with it).
        assert_eq!(apps(&cosmic), ["obsidian", "wadbrowser", "vscode", "terminal", "libresprite", "tiled"]);
        remove_item(&mut cosmic, Section::Addons, "vscode:ms-vscode.cpptools");
        assert!(exts(&cosmic).is_empty());
        remove_item(&mut vanua, Section::Apps, "vscode");
        assert!(!apps(&vanua).contains(&"vscode"));
        assert!(exts(&vanua).iter().all(|e| !e.starts_with("vscode:")));
    }

    #[test]
    fn secrets_copy_by_slot_as_in_v5() {
        let (iq, mut cosmic, _) = board();
        assert_eq!(merge_section(&mut cosmic, &iq, Section::Secrets), "2 keys");
        assert_eq!(cosmic.secrets, iq.secrets);
    }

    #[test]
    fn a_look_dropped_in_gives_its_picture_and_adds_its_files() {
        let (iq, mut cosmic, _) = board();
        cosmic.look.projects.push(Project {
            id: "project-iq".into(),
            as_name: "Notes".into(),
            kind: ProjectKind::Github { repo: "HungGod/Writing".into(), setup: None },
        });
        assert_eq!(merge_section(&mut cosmic, &iq, Section::Look), "the look");
        assert_eq!(cosmic.look.wallpaper, iq.look.wallpaper);
        assert_eq!(
            cosmic.look.projects.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["project-iq", "project-iq-2"]
        );
        assert!(cosmic.look.layout.icons.contains(&"project:project-iq-2".to_string()));
        // Again: nothing more (the same project's there).
        merge_section(&mut cosmic, &iq, Section::Look);
        assert_eq!(cosmic.look.projects.len(), 2);
    }

    #[test]
    fn a_wallpaper_and_a_repository_go_on_its_look() {
        let (_, _, mut vanua) = board();
        assert!(add_item(&mut vanua, Section::Look, "wallpaper:image:/wallpapers/art/koi.svg", &[], None).ok);
        assert_eq!(vanua.look.wallpaper, Wallpaper::image("/wallpapers/art/koi.svg"));
        assert!(!add_item(&mut vanua, Section::Look, "wallpaper:texture:grid", &[], None).ok);
        assert!(add_item(&mut vanua, Section::Look, "wallpaper:empty", &[], None).ok);
        assert_eq!(vanua.look.wallpaper, Wallpaper::Empty);
        let r = add_item(&mut vanua, Section::Look, "repo:HungGod/VanuaAcademy", &[], None);
        assert_eq!(r.said, "Clones HungGod/VanuaAcademy into VanuaAcademy");
        assert_eq!(vanua.look.projects[0].id, "gh-vanuaacademy");
        assert!(!add_item(&mut vanua, Section::Look, "repo:HungGod/VanuaAcademy", &[], None).ok);
        assert!(!add_item(&mut vanua, Section::Look, "repo:../etc", &[], None).ok);
    }

    #[test]
    fn a_web_key_goes_to_the_browsers_group_bringing_the_browser() {
        let mut x = blank("a", "A", "m", 0, 0);
        let r = add_item(&mut x, Section::Secrets, "work", &keys(), None);
        assert!(r.ok);
        assert_eq!(apps(&x), ["wadbrowser"]);
        assert_eq!(keys_of(&x)[0].1.name, "Work");
    }

    #[test]
    fn a_claude_code_key_brings_claude_code_and_vs_code() {
        let mut x = blank("a", "A", "m", 0, 0);
        add_item(&mut x, Section::Secrets, "cc", &keys(), None);
        assert_eq!(exts(&x), ["vscode:anthropic.claude-code"]);
        assert_eq!(x.secrets.by_slot["ext:anthropic.claude-code"].name, "Claude");
        // Taking VS Code out takes Claude Code and its key.
        remove_item(&mut x, Section::Apps, "vscode");
        assert!(x.secrets.by_slot.is_empty());
    }

    #[test]
    fn says_what_an_add_would_do_without_doing_it() {
        let x = blank("a", "A", "m", 0, 0);
        assert_eq!(
            describe_add(&x, Section::Addons, "vscode:anthropic.claude-code", &keys(), None).said,
            "Adds Claude Code; VS Code comes too"
        );
        assert!(x.exts.is_empty());
        assert_eq!(describe_add(&x, Section::Look, "node:fs-12", &keys(), None).said, "Adds it");
    }

    #[test]
    fn a_section_on_empty_space_is_a_wadspace_with_only_that() {
        let (iq, _, _) = board();
        let mut x = blank("n", "New", "surface", 0, 9);
        from_section(&mut x, &iq, Section::Apps);
        assert_eq!(apps(&x), ["wadbrowser", "vscode", "terminal", "libresprite", "tiled"]);
        assert!(x.exts.is_empty());
        assert_eq!(x.name, "IntelligenceQuest · Apps");
        let mut y = blank("m", "New", "surface", 0, 9);
        from_section(&mut y, &iq, Section::Look);
        assert_eq!(y.look.wallpaper, iq.look.wallpaper);
        assert_eq!(y.look.layout.icons, ["project:project-iq"]);
        assert!(y.apps.is_empty());
    }

    #[test]
    fn named_variants_get_ids_and_names_of_their_own() {
        let mut x = blank("a", "A", "m", 0, 0);
        assert_eq!(add_named(&mut x, AppId::Dolphin, "Mario Kart").map(|e| e.id), Some("mario-kart".into()));
        assert_eq!(
            add_named(&mut x, AppId::Dolphin, "Mario Kart").map(|e| (e.id, e.name)),
            Some(("mario-kart-2".into(), Some("Mario Kart 2".into())))
        );
        assert_eq!(add_named(&mut x, AppId::Dolphin, "  "), None);
        assert_eq!(add_named(&mut x, AppId::Vscode, "Mine"), None);
        assert_eq!(x.look.layout.icons, ["app:dolphin", "variant:mario-kart", "variant:mario-kart-2"]);
    }

    #[test]
    fn taking_an_add_on_out_takes_its_sign_in_and_its_icon() {
        let (mut iq, _, _) = board();
        assert!(remove_item(&mut iq, Section::Addons, "wadbrowser:github"));
        assert!(!iq.look.layout.icons.contains(&"web:github".to_string()));
        assert_eq!(iq.secrets.groups[0].members, ["browser"]);
        assert!(remove_item(&mut iq, Section::Addons, "vscode:anthropic.claude-code"));
        assert!(iq.secrets.by_slot.is_empty());
        assert!(!remove_item(&mut iq, Section::Addons, "vscode:anthropic.claude-code"));
    }
}
