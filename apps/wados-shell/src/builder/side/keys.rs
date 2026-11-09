//! The Builder's keys (the spike's `apps/builder/keys.ts`, and the choices
//! `SideSecrets.svelte` makes), as pure functions of a wadspace and the
//! library: what a key group's dropdown offers (another key of its kind,
//! none, a new one: everything in the group follows), what one thing's
//! dropdown offers, what a drop of web apps on a group or on "a key of
//! their own" does, and a dragged piece of a stack as the things it is.
//! Each choice carries the edits it makes ([`Act`]); the panel sends them.

use wad_model::catalog::app_name;
use wad_model::edit::{Library, Op, Payload};
use wad_model::folders::{Folder, folder_kinds, folders_of, move_to};
use wad_model::look::can_launch;
use wad_model::variants::{AppVariant, VariantKind, find_variant, key_kind_of, sign_in_of, variants_of};
use wad_model::{AppId, SecretKind, Section, Wadspace};

use crate::dnd::Held;

/// A kind of key, as a new one's title names it ("A new VS Code key").
pub fn kind_name(k: SecretKind) -> &'static str {
    match k {
        SecretKind::Web => "web",
        SecretKind::Vscode => "VS Code",
        SecretKind::Obsidian => "Obsidian",
        SecretKind::Steam => "Steam",
        SecretKind::VscodeClaudeCode => "Claude Code",
    }
}

/// A line of a key's dropdown.
#[derive(Debug, Clone, PartialEq)]
pub enum Choice {
    Heading(String),
    Sep,
    Item(Pick),
}

/// One choice: its id (the spike's `data-menu`: 'key:work-web', 'key:none', 'key:new', 'own-key:…').
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub id: String,
    pub label: String,
    pub icon: &'static str,
    pub checked: bool,
    /// Off, and why.
    pub off: Option<String>,
    pub act: Act,
}

/// What a choice does.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// These edits, in order.
    Ops(Vec<Op>),
    /// A new key of `kind`, named first (`secrets.create`); then these things sign in with it.
    NewKey { kind: SecretKind, then: SignIns },
    /// A disc off this machine (`DELETE /v1/discs/{id}`: not an edit of a
    /// wadspace, which is why it isn't an Op).
    RemoveDisc { id: String },
}

/// Things of a wadspace that sign in with a key once it's made: each a
/// variant's id ('app:wadbrowser' is the browser's own group).
#[derive(Debug, Clone, PartialEq)]
pub struct SignIns {
    pub ws: String,
    pub variants: Vec<String>,
}

impl SignIns {
    /// Each signs in with `key` (none: as its app does, or with nothing).
    pub fn ops(&self, key: Option<&str>) -> Vec<Op> {
        self.variants
            .iter()
            .map(|v| Op::SignIn { ws: self.ws.clone(), variant: v.clone(), key: key.map(str::to_string) })
            .collect()
    }
}

/// A group's id in the sidebar: 'home' (the app's own), else its key's.
pub fn folder_id(f: &Folder) -> String {
    if f.home { "home".into() } else { f.key.as_ref().map(|k| k.id.clone()).unwrap_or_default() }
}

/// What a group's key, swapped, changes: the app's own group is the app's
/// key; another's, each of its things'.
pub fn group_targets(x: &Wadspace, app: AppId, f: &Folder) -> SignIns {
    let variants = if f.home { vec![format!("app:{app}")] } else { f.items.iter().map(|v| v.id.clone()).collect() };
    SignIns { ws: x.id.clone(), variants }
}

/// A key group's key swapped (`key`), or taken off (none).
pub fn rekey(x: &Wadspace, app: AppId, f: &Folder, key: Option<&str>) -> Vec<Op> {
    group_targets(x, app, f).ops(key)
}

fn keys_of_kind(lib: &Library, kind: SecretKind) -> impl Iterator<Item = &wad_model::Secret> {
    lib.keys.iter().filter(move |k| k.kind == kind)
}

fn new_key(kind: SecretKind, then: SignIns) -> Pick {
    Pick {
        id: "key:new".into(),
        label: "New key…".into(),
        icon: "plus",
        checked: false,
        off: None,
        act: Act::NewKey { kind, then },
    }
}

/// A key group's dropdown: another key of its kind, none, a new one.
pub fn group_key_items(lib: &Library, x: &Wadspace, app: AppId, f: &Folder) -> Vec<Choice> {
    let kind = f.key.as_ref().map(|k| k.kind).or_else(|| variants_of(x, app).first().and_then(key_kind_of));
    let Some(kind) = kind else { return vec![] };
    let mut out: Vec<Choice> = keys_of_kind(lib, kind)
        .map(|k| {
            Choice::Item(Pick {
                id: format!("key:{}", k.id),
                label: k.name.clone(),
                icon: "key-round",
                checked: f.key.as_ref().is_some_and(|fk| fk.id == k.id),
                off: None,
                act: Act::Ops(rekey(x, app, f, Some(&k.id))),
            })
        })
        .collect();
    out.push(Choice::Sep);
    out.push(Choice::Item(if f.home {
        Pick {
            id: "key:none".into(),
            label: "No key".into(),
            icon: "x",
            checked: f.key.is_none(),
            off: None,
            act: Act::Ops(rekey(x, app, f, None)),
        }
    } else {
        Pick {
            id: "key:none".into(),
            label: format!("As {} does", app_name(app)),
            icon: "undo-2",
            checked: false,
            off: None,
            act: Act::Ops(rekey(x, app, f, None)),
        }
    }));
    out.push(Choice::Item(new_key(kind, group_targets(x, app, f))));
    out
}

/// One thing's dropdown (VS Code's own key, Claude Code's, a vault's, a Steam account's).
pub fn thing_key_items(lib: &Library, x: &Wadspace, v: &AppVariant) -> Vec<Choice> {
    let Some(kind) = key_kind_of(v) else { return vec![] };
    let now = sign_in_of(x, v);
    let one = SignIns { ws: x.id.clone(), variants: vec![v.id.clone()] };
    let mut out: Vec<Choice> = keys_of_kind(lib, kind)
        .map(|k| {
            Choice::Item(Pick {
                id: format!("key:{}", k.id),
                label: k.name.clone(),
                icon: "key-round",
                checked: now.as_ref().is_some_and(|s| s.own && s.key.id == k.id),
                off: None,
                act: Act::Ops(one.ops(Some(&k.id))),
            })
        })
        .collect();
    out.push(Choice::Sep);
    let label = if v.kind == VariantKind::App || kind == SecretKind::VscodeClaudeCode {
        "No key".to_string()
    } else {
        format!("As {} does", app_name(v.app))
    };
    out.push(Choice::Item(Pick {
        id: "key:none".into(),
        label,
        icon: "x",
        checked: !now.as_ref().is_some_and(|s| s.own),
        off: None,
        act: Act::Ops(one.ops(None)),
    }));
    out.push(Choice::Item(new_key(kind, one)));
    out
}

/// A thing's own menu (a right-click on its pill): start straight into it, or take it out (Undo's in the toast).
pub fn thing_menu(x: &Wadspace, v: &AppVariant) -> Vec<Choice> {
    let key = match v.kind {
        VariantKind::App => Some(format!("app:{}", v.app)),
        VariantKind::Web => v.ext.as_ref().map(|e| format!("web:{}", e.id)),
        VariantKind::Game | VariantKind::Named => v.ext.as_ref().map(|e| format!("variant:{}", e.id)),
        _ => None,
    };
    let mut out = vec![];
    if let Some(k) = key.filter(|k| can_launch(x, k)) {
        let on = x.look.launch.as_deref() == Some(k.as_str());
        out.push(Choice::Item(Pick {
            id: "chip-launch".into(),
            label: "Start straight into it".into(),
            icon: "rocket",
            checked: on,
            off: None,
            act: Act::Ops(vec![Op::SetLaunch { ws: x.id.clone(), launch: (!on).then_some(k) }]),
        }));
    }
    out.push(Choice::Item(Pick {
        id: "chip-remove".into(),
        label: "Take it out".into(),
        icon: "trash",
        checked: false,
        off: None,
        act: Act::Ops(vec![Op::Remove { ws: x.id.clone(), chip: v.id.clone() }]),
    }));
    out
}

/// "N web apps", or the one's name.
pub fn several(n: usize, one: &str) -> String {
    if n == 1 { one.to_string() } else { format!("{n} web apps") }
}

/// Web apps let go on "a key of their own": which key they share (a saved
/// web key no group of its has yet, or a new one).
pub fn own_key_items(lib: &Library, x: &Wadspace, vs: &[AppVariant]) -> Vec<Choice> {
    let Some(first) = vs.first() else { return vec![] };
    let used: Vec<String> =
        folders_of(x, AppId::Wadbrowser).iter().filter_map(|f| f.key.as_ref()).map(|k| k.id.clone()).collect();
    let them = SignIns { ws: x.id.clone(), variants: vs.iter().map(|v| v.id.clone()).collect() };
    let mut out = vec![Choice::Heading(format!(
        "{} sign{} in with",
        several(vs.len(), &first.name),
        if vs.len() == 1 { "s" } else { "" }
    ))];
    out.extend(keys_of_kind(lib, SecretKind::Web).filter(|k| !used.contains(&k.id)).map(|k| {
        Choice::Item(Pick {
            id: format!("own-key:{}", k.id),
            label: k.name.clone(),
            icon: "key-round",
            checked: false,
            off: None,
            act: Act::Ops(them.ops(Some(&k.id))),
        })
    }));
    out.push(Choice::Item(Pick { id: "own-key:new".into(), ..new_key(SecretKind::Web, them) }));
    out
}

/// One or several pieces dragged out of `x`'s own stack or sidebar (picked
/// together in the Secrets tab), as the things they are.
pub fn pieces_of(x: &Wadspace, held: &Held) -> Vec<AppVariant> {
    match held {
        Held::Pieces { section: Section::Addons, items, from } if *from == x.id => {
            items.iter().filter_map(|i| find_variant(x, &format!("ext:{i}"))).collect()
        }
        Held::Part { payload: Payload::Piece { section, item, from: Some(from) } } if *from == x.id => {
            let id = match section {
                Section::Apps => format!("app:{item}"),
                Section::Addons => format!("ext:{item}"),
                _ => return vec![],
            };
            find_variant(x, &id).into_iter().collect()
        }
        _ => vec![],
    }
}

/// The web apps among them.
pub fn web_of(x: &Wadspace, held: &Held) -> Vec<AppVariant> {
    pieces_of(x, held).into_iter().filter(|v| v.kind == VariantKind::Web).collect()
}

/// Web apps of `x`'s let go on its WadBrowser group `fid`: the edits (each
/// signs in with the group's key) and what to say under the pointer; none
/// if none of them would move.
pub fn group_drop(x: &Wadspace, fid: &str, held: &Held) -> Option<(Vec<Op>, String)> {
    let f = folders_of(x, AppId::Wadbrowser).into_iter().find(|f| folder_id(f) == fid)?;
    let moves: Vec<(AppVariant, wad_model::folders::Move)> =
        web_of(x, held).into_iter().filter_map(|v| move_to(x, &v, &f).map(|m| (v, m))).collect();
    let (_, first) = moves.first()?;
    let hint = if moves.len() == 1 {
        first.hint.clone()
    } else {
        let key = f.key.as_ref().map_or("nothing", |k| k.name.as_str());
        let as_app = if f.home && f.key.is_some() { ", as WadBrowser does" } else { "" };
        format!("{} web apps sign in with {key}{as_app}", moves.len())
    };
    let ops =
        moves.iter().map(|(v, m)| Op::SignIn { ws: x.id.clone(), variant: v.id.clone(), key: m.key.clone() }).collect();
    Some((ops, hint))
}

/// Web apps of `x`'s let go on "a key of their own": them, and what to say.
pub fn own_drop(x: &Wadspace, held: &Held) -> Option<(Vec<AppVariant>, String)> {
    let vs = web_of(x, held);
    let first = vs.first()?;
    let hint = format!("A key of their own for {}", several(vs.len(), &first.name));
    Some((vs, hint))
}

/// The apps in `x` that sign in, in its order.
pub fn secret_apps(x: &Wadspace) -> Vec<AppId> {
    x.apps.iter().map(|a| a.app).filter(|app| !folder_kinds(x, *app).is_empty()).collect()
}

/// Things of `app`'s that sign in on their own (not WadBrowser's web apps: those are its groups).
pub fn lines(x: &Wadspace, app: AppId) -> Vec<AppVariant> {
    variants_of(x, app)
        .into_iter()
        .filter(|v| {
            v.kind != VariantKind::SignIn
                && key_kind_of(v).is_some()
                && matches!(v.kind, VariantKind::App | VariantKind::Extension)
        })
        .collect()
}

/// A line's name: the app's own sign-in by what it is (VS Code's Settings, Obsidian's Vault, Steam's Account).
pub fn line_label(v: &AppVariant) -> String {
    if v.kind != VariantKind::App {
        return v.name.clone();
    }
    match v.app {
        AppId::Vscode => "Settings".into(),
        AppId::Obsidian => "Vault".into(),
        AppId::Steam => "Account".into(),
        app => app_name(app).into(),
    }
}

/// What a line's dropdown says: its key (its app's, said so), or No key; and whether it has one.
pub fn line_key(x: &Wadspace, v: &AppVariant) -> (String, bool) {
    match sign_in_of(x, v) {
        Some(s) if s.own => (s.key.name, true),
        Some(s) => (format!("{} (as {})", s.key.name, app_name(v.app)), true),
        None => ("No key".into(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_model::edit::{Ctx, apply};

    fn sample() -> Library {
        let text = include_str!("../../../../../crates/wad-model/tests/data/populated.json");
        serde_json::from_str(text).expect("the sample library")
    }

    fn run(lib: &mut Library, ops: Vec<Op>) {
        let ctx = Ctx::new(1, "m-surface");
        for op in ops {
            apply(lib, op, &ctx).expect("applies");
        }
    }

    fn picks(c: &[Choice]) -> Vec<&str> {
        c.iter()
            .filter_map(|c| match c {
                Choice::Item(p) => Some(p.id.as_str()),
                _ => None,
            })
            .collect()
    }

    fn pick<'a>(c: &'a [Choice], id: &str) -> &'a Pick {
        c.iter()
            .find_map(|c| match c {
                Choice::Item(p) if p.id == id => Some(p),
                _ => None,
            })
            .expect("that choice")
    }

    fn ops(p: &Pick) -> Vec<Op> {
        match &p.act {
            Act::Ops(o) => o.clone(),
            Act::NewKey { .. } | Act::RemoveDisc { .. } => panic!("not edits"),
        }
    }

    #[test]
    fn from_scratch_the_browsers_dropdown_signs_it_and_gmail_in_with_work() {
        let mut lib = sample();
        let ctx = Ctx::new(1, "m-surface");
        let id = apply(&mut lib, Op::Create { name: Some("Art studio".into()), category: None, machine: None }, &ctx)
            .unwrap()
            .created
            .unwrap();
        run(
            &mut lib,
            vec![
                Op::AddApp { ws: id.clone(), app: AppId::Krita },
                Op::AddApp { ws: id.clone(), app: AppId::Wadbrowser },
                Op::NewVariant {
                    ws: id.clone(),
                    app: AppId::Wadbrowser,
                    new: wad_model::NewVariant::Addon("wadbrowser:gmail".into()),
                },
            ],
        );
        let x = lib.wadspace(&id).unwrap().clone();
        assert_eq!(secret_apps(&x), [AppId::Wadbrowser]);
        let fs = folders_of(&x, AppId::Wadbrowser);
        assert_eq!(fs.iter().map(folder_id).collect::<Vec<_>>(), ["home"]);
        let menu = group_key_items(&lib, &x, AppId::Wadbrowser, &fs[0]);
        assert_eq!(picks(&menu), ["key:work-web", "key:hunggod", "key:vanua-google", "key:none", "key:new"]);
        assert!(pick(&menu, "key:none").checked);
        run(&mut lib, ops(pick(&menu, "key:work-web")));
        let x = lib.wadspace(&id).unwrap();
        assert_eq!(x.secrets.groups.iter().map(|g| g.key.id.as_str()).collect::<Vec<_>>(), ["work-web"]);
        assert!(x.secrets.groups[0].members.contains(&"web:gmail".to_string()));
    }

    #[test]
    fn another_groups_key_moves_everything_in_it_and_none_is_as_the_browser_does() {
        let lib = sample();
        let x = lib.wadspace("vanua-academy").unwrap();
        let fs = folders_of(x, AppId::Wadbrowser);
        let work = fs.iter().find(|f| folder_id(f) == "work-web").unwrap();
        let menu = group_key_items(&lib, x, AppId::Wadbrowser, work);
        assert!(pick(&menu, "key:work-web").checked);
        assert_eq!(pick(&menu, "key:none").label, "As WadBrowser does");
        let to_hunggod = ops(pick(&menu, "key:hunggod"));
        assert_eq!(to_hunggod.len(), 3);
        assert!(to_hunggod.iter().all(|o| matches!(o, Op::SignIn { key: Some(k), .. } if k == "hunggod")));
        match &pick(&menu, "key:new").act {
            Act::NewKey { kind: SecretKind::Web, then } => {
                assert_eq!(then.variants.len(), 3);
                assert!(then.ops(Some("k")).iter().all(|o| matches!(o, Op::SignIn { key: Some(k), .. } if k == "k")));
            }
            a => panic!("{a:?}"),
        }
        // The browser's own group is the app's key.
        let home = fs.iter().find(|f| f.home).unwrap();
        assert_eq!(
            rekey(x, AppId::Wadbrowser, home, Some("work-web")),
            vec![Op::SignIn { ws: x.id.clone(), variant: "app:wadbrowser".into(), key: Some("work-web".into()) }]
        );
    }

    #[test]
    fn a_things_dropdown_and_steams_new_key() {
        let lib = sample();
        let x = lib.wadspace("vanua-academy").unwrap();
        let ls = lines(x, AppId::Vscode);
        assert_eq!(
            ls.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
            ["app:vscode", "ext:vscode:anthropic.claude-code"]
        );
        assert_eq!(line_label(&ls[0]), "Settings");
        assert_eq!(line_key(x, &ls[0]), ("No key".to_string(), false));
        assert_eq!(line_key(x, &ls[1]), ("VanuaAcademy-claude".to_string(), true));
        let menu = thing_key_items(&lib, x, &ls[1]);
        assert_eq!(picks(&menu), ["key:vanua-claude", "key:none", "key:new"]);
        assert!(pick(&menu, "key:vanua-claude").checked);
        assert_eq!(pick(&menu, "key:none").label, "No key");
        assert!(pick(&menu, "key:new").off.is_none());

        let couch = lib.wadspace("couch-co-op").unwrap();
        let account = lines(couch, AppId::Steam);
        assert_eq!(line_label(&account[0]), "Account");
        let menu = thing_key_items(&lib, couch, &account[0]);
        assert!(pick(&menu, "key:old-steam").checked);
        assert_eq!(pick(&menu, "key:new").off, None);
    }

    #[test]
    fn picked_web_apps_dropped_on_a_group_or_a_key_of_their_own() {
        let mut lib = sample();
        let x = lib.wadspace("vanua-academy").unwrap().clone();
        let both = Held::Pieces {
            section: Section::Addons,
            items: vec!["wadbrowser:gmail".into(), "wadbrowser:google-drive".into()],
            from: x.id.clone(),
        };
        assert_eq!(web_of(&x, &both).len(), 2);
        // Onto their own group: nothing moves.
        assert!(group_drop(&x, "work-web", &both).is_none());
        let (ops, hint) = group_drop(&x, "hunggod", &both).expect("they'd move");
        assert_eq!(hint, "2 web apps sign in with hunggod");
        assert_eq!(ops.len(), 2);
        // Another wadspace's pieces aren't this one's.
        let other =
            Held::Pieces { section: Section::Addons, items: vec!["wadbrowser:gmail".into()], from: "travel".into() };
        assert!(web_of(&x, &other).is_empty());

        let (vs, hint) = own_drop(&x, &both).expect("web apps");
        assert_eq!(hint, "A key of their own for 2 web apps");
        let menu = own_key_items(&lib, &x, &vs);
        assert_eq!(menu[0], Choice::Heading("2 web apps sign in with".into()));
        // Every web key has a group of its already: only a new one.
        assert_eq!(picks(&menu), ["own-key:new"]);
        let Act::NewKey { kind, then } = &pick(&menu, "own-key:new").act else { panic!() };
        assert_eq!(*kind, SecretKind::Web);
        let ctx = Ctx::new(1, "m-surface");
        let key =
            apply(&mut lib, Op::NewKey { name: "Mail and Drive".into(), kind: *kind }, &ctx).unwrap().created.unwrap();
        run(&mut lib, then.ops(Some(&key)));
        let x = lib.wadspace("vanua-academy").unwrap();
        let fs = folders_of(x, AppId::Wadbrowser);
        let own = fs.iter().find(|f| folder_id(f) == key).expect("a group of their own");
        assert_eq!(
            own.items.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
            ["ext:wadbrowser:gmail", "ext:wadbrowser:google-drive"]
        );
        let work = fs.iter().find(|f| folder_id(f) == "work-web").unwrap();
        assert_eq!(work.items.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["ext:wadbrowser:claude"]);
    }

    #[test]
    fn a_things_menu_starts_straight_into_it_or_takes_it_out() {
        let lib = sample();
        let x = lib.wadspace("vanua-academy").unwrap();
        let gmail = find_variant(x, "ext:wadbrowser:gmail").unwrap();
        let menu = thing_menu(x, &gmail);
        assert_eq!(picks(&menu), ["chip-launch", "chip-remove"]);
        assert_eq!(
            ops(pick(&menu, "chip-launch")),
            vec![Op::SetLaunch { ws: x.id.clone(), launch: Some("web:gmail".into()) }]
        );
        assert_eq!(
            ops(pick(&menu, "chip-remove")),
            vec![Op::Remove { ws: x.id.clone(), chip: "ext:wadbrowser:gmail".into() }]
        );
    }

    #[test]
    fn one_piece_from_its_own_stack_is_the_thing_it_is() {
        let lib = sample();
        let x = lib.wadspace("vanua-academy").unwrap();
        let gmail = Held::Part {
            payload: Payload::Piece {
                section: Section::Addons,
                item: "wadbrowser:gmail".into(),
                from: Some(x.id.clone()),
            },
        };
        assert_eq!(pieces_of(x, &gmail)[0].id, "ext:wadbrowser:gmail");
        let (_, hint) = group_drop(x, "hunggod", &gmail).unwrap();
        assert_eq!(hint, "Gmail signs in with hunggod");
        let app = Held::Part {
            payload: Payload::Piece { section: Section::Apps, item: "wadbrowser".into(), from: Some(x.id.clone()) },
        };
        assert_eq!(pieces_of(x, &app)[0].id, "app:wadbrowser");
        // The browser itself isn't a web app: no key of its own from a drop.
        assert!(own_drop(x, &app).is_none());
        let loose = Held::Part {
            payload: Payload::Piece { section: Section::Addons, item: "wadbrowser:gmail".into(), from: None },
        };
        assert!(pieces_of(x, &loose).is_empty());
    }
}
