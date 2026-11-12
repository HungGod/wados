//! A wadspace's add-ons as the sidebar's Add-ons tab lists them (the
//! spike's `apps/builder/addons.ts`), as pure functions of it and the
//! library: only the add-ons of the apps in it, a group per app. Web apps
//! for WadBrowser (the usual sites, then your own: the machine's browser's,
//! then any of its own no list has), VS Code's extensions, the command
//! line's tools, your signed-in Steam accounts' games, and Dolphin's discs.
//! Each says whether it's in it (its chip), how to add it, and what a drag
//! of it carries.
//!
//! Steam's games and Dolphin's need Steam's sign-in and your GitHub
//! packages (phase 10): they're listed, off, saying so.

use wad_model::catalog::{WEB_APPS, extensions, tools};
use wad_model::edit::{Library, Op, Payload};
use wad_model::games::game_of;
use wad_model::suggest::{Suggestion, suggestions};
use wad_model::{AppId, DiscImage, Ext, NewVariant, Section, Wadspace};

use crate::builder::side::keys::{Act, Choice, Pick};
use crate::dnd::Held;

/// How many suggestions per app.
pub const SUGGESTED: usize = 4;

/// An add-on's icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemIcon {
    /// A web app's: its site's own, on a white card (or its initial).
    Site { id: String, name: String, url: Option<String> },
    /// A picture of WadSpaces' own ('/icons/claude.png').
    Img(&'static str),
    /// A lucide glyph.
    Glyph(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// Its id in its group (the spike's `data-side-addon`: 'gmail', 'own:…', 'its:…', 'steam:<account>:<game>').
    pub key: String,
    pub name: String,
    pub icon: ItemIcon,
    /// Its chip in the wadspace ('ext:wadbrowser:gmail'), if it's in it.
    pub lit: Option<String>,
    /// How to add it (none: it can't be here).
    pub add: Option<Op>,
    /// What dragging it carries (none: it doesn't drag).
    pub drag: Option<Held>,
    /// Off, and why (what comes later).
    pub off: Option<String>,
    /// Where it is, said after its name in its tooltip (a Steam game's:
    /// "downloaded here", "in your packages").
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupId {
    Web,
    Extensions,
    Tools,
    Steam,
    Dolphin,
}

impl GroupId {
    /// As the spike's `data-side-addons` says it.
    pub fn id(self) -> &'static str {
        match self {
            GroupId::Web => "web",
            GroupId::Extensions => "extensions",
            GroupId::Tools => "tools",
            GroupId::Steam => "steam",
            GroupId::Dolphin => "dolphin",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub id: GroupId,
    pub title: &'static str,
    pub app: AppId,
    pub items: Vec<Item>,
}

/// The add-on of `app`'s in `x` that `test` picks, as its chip's id.
pub fn ext_in(x: &Wadspace, app: AppId, test: impl Fn(&Ext) -> bool) -> Option<String> {
    x.exts.iter().find(|e| e.app == app && test(e)).map(|e| format!("ext:{app}:{}", e.id))
}

/// A catalog add-on ('vscode:<extension>', 'terminal:<tool>', 'wadbrowser:<site>'), into `x` (its app comes along).
pub fn add_addon(x: &Wadspace, item: &str) -> Option<Op> {
    let app = AppId::parse(item.split_once(':')?.0)?;
    Some(Op::NewVariant { ws: x.id.clone(), app, new: NewVariant::Addon(item.to_string()) })
}

/// What a click on it does: out (with Undo) if it's in it, else in.
pub fn toggle(x: &Wadspace, i: &Item) -> Option<Op> {
    if i.off.is_some() {
        return None;
    }
    match &i.lit {
        Some(chip) => Some(Op::Remove { ws: x.id.clone(), chip: chip.clone() }),
        None => i.add.clone(),
    }
}

/// A disc's own menu (a right-click on its pill in Dolphin games): take it
/// off this machine. It says where it's used, since it goes from there too
/// (three names at most: the rest are counted). One pushed keeps its copy
/// in the account's packages (the toast says so).
pub fn disc_menu(lib: &Library, d: &DiscImage) -> Vec<Choice> {
    let uses: Vec<&str> = lib
        .wadspaces
        .iter()
        .filter(|x| !x.is_trashed())
        .filter(|x| {
            x.exts
                .iter()
                .any(|e| e.app == AppId::Dolphin && (e.id == d.id || e.name.as_deref() == Some(d.name.as_str())))
        })
        .map(|x| x.name.as_str())
        .collect();
    let mut out = vec![];
    if !uses.is_empty() {
        let named = uses[..uses.len().min(3)].join(", ");
        let rest = uses.len().saturating_sub(3);
        let more = if rest == 0 { String::new() } else { format!(" and {rest} more") };
        out.push(Choice::Heading(format!("Used in {named}{more}")));
    }
    out.push(Choice::Item(Pick {
        id: format!("disc-remove:{}", d.id),
        label: "Remove from this machine".into(),
        icon: "trash",
        checked: false,
        off: None,
        act: Act::RemoveDisc { id: d.id.clone() },
    }));
    out
}

/// Where a Steam game is, for the Builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameState {
    /// The account has it; not downloaded here.
    InLibrary,
    /// Downloaded into the account's folder here: it can be packaged.
    Downloaded,
    /// In your GitHub packages.
    Packaged,
}

impl GameState {
    pub fn words(self) -> &'static str {
        match self {
            GameState::InLibrary => "in your Steam library",
            GameState::Downloaded => "downloaded here",
            GameState::Packaged => "in your packages",
        }
    }
}

/// One of a Steam wadspace's games.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamGame {
    pub id: String,
    pub name: String,
    pub state: GameState,
    /// The Steam key whose library has it (none: only packaged).
    pub account: Option<String>,
}

/// The key `x`'s Steam signs in with.
pub fn steam_account(x: &Wadspace) -> Option<String> {
    x.secrets.by_slot.get(wad_model::edit::STEAM_SLOT).map(|k| k.id.clone())
}

/// `x`'s Steam games: its account's library (as wadd last read it from
/// the key's folder), then what's packaged that the library hasn't, by name.
pub fn steam_games(lib: &Library, x: &Wadspace) -> Vec<SteamGame> {
    let account = steam_account(x);
    let library = account.as_ref().and_then(|a| lib.steam.get(a));
    let mut out: Vec<SteamGame> = library
        .map(|l| {
            l.owned
                .iter()
                .map(|id| SteamGame {
                    id: id.clone(),
                    name: game_of(id, l.names.get(id).map(String::as_str)).name,
                    state: if lib.game_packages.contains_key(id) {
                        GameState::Packaged
                    } else if l.has(id) {
                        GameState::Downloaded
                    } else {
                        GameState::InLibrary
                    },
                    account: account.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    for p in lib.game_packages.values() {
        if !out.iter().any(|g| g.id == p.game) {
            out.push(SteamGame {
                id: p.game.clone(),
                name: game_of(&p.game, None).name,
                state: GameState::Packaged,
                account: None,
            });
        }
    }
    out.sort_by_key(|g| g.name.to_lowercase());
    out
}

/// A Steam game's own menu (a right-click on its pill): keep it in your
/// GitHub packages once it's downloaded here; else what it takes.
pub fn steam_menu(g: &SteamGame) -> Vec<Choice> {
    let mut out = vec![Choice::Heading(format!("{}: {}", g.name, g.state.words()))];
    let (off, act) = match (g.state, &g.account) {
        (GameState::Downloaded, Some(account)) => {
            (None, Act::PackageGame { account: account.clone(), game: g.id.clone() })
        }
        (GameState::Packaged, _) => (Some("It's in your packages already".to_string()), Act::Ops(vec![])),
        _ => (Some("Download it in Steam, in this wadspace, first".to_string()), Act::Ops(vec![])),
    };
    out.push(Choice::Item(Pick {
        id: format!("steam-package:{}", g.id),
        label: "Keep it in your GitHub packages".into(),
        icon: "upload",
        checked: false,
        off,
        act,
    }));
    out
}

/// WadBrowser's Focus in `x`, as its chip.
pub fn focus(x: &Wadspace) -> Option<String> {
    ext_in(x, AppId::Wadbrowser, |e| e.id == "focus")
}

/// Focus on, or off.
pub fn focus_toggle(x: &Wadspace) -> Op {
    match focus(x) {
        Some(chip) => Op::Remove { ws: x.id.clone(), chip },
        None => Op::NewVariant {
            ws: x.id.clone(),
            app: AppId::Wadbrowser,
            new: NewVariant::Addon("wadbrowser:focus".into()),
        },
    }
}

/// What's suggested for it (none for one that can't be changed here).
pub fn suggested(x: &Wadspace, editable: bool) -> Vec<Suggestion> {
    if editable { suggestions(x, SUGGESTED) } else { vec![] }
}

fn piece(item: String) -> Option<Held> {
    Some(Held::Part { payload: Payload::Piece { section: Section::Addons, item, from: None } })
}

fn site_icon(id: &str, name: &str, url: Option<&str>) -> ItemIcon {
    ItemIcon::Site { id: id.into(), name: name.into(), url: url.map(str::to_string) }
}

/// `x`'s add-ons, a group per app in it that has them.
pub fn groups(lib: &Library, x: &Wadspace) -> Vec<Group> {
    let has = |app: AppId, test: &dyn Fn(&Ext) -> bool| ext_in(x, app, test);
    let addon = |item: String| add_addon(x, &item);

    let mut web: Vec<Item> = WEB_APPS
        .iter()
        .map(|s| Item {
            key: s.id.into(),
            name: s.name.into(),
            icon: site_icon(s.id, s.name, Some(s.url)),
            lit: has(AppId::Wadbrowser, &|e| e.id == s.id || e.url.as_deref() == Some(s.url)),
            add: addon(format!("wadbrowser:{}", s.id)),
            drag: piece(format!("wadbrowser:{}", s.id)),
            off: None,
            note: None,
        })
        .collect();
    let known = |web: &[Item], url: &str| {
        web.iter().any(|i| matches!(&i.icon, ItemIcon::Site { url: Some(u), .. } if u == url))
    };
    for a in &lib.web_apps {
        if known(&web, &a.url) {
            continue;
        }
        web.push(Item {
            key: format!("own:{}", a.id),
            name: a.name.clone(),
            icon: site_icon(&a.id, &a.name, Some(&a.url)),
            lit: has(AppId::Wadbrowser, &|e| e.url.as_deref() == Some(a.url.as_str())),
            add: Some(Op::NewVariant {
                ws: x.id.clone(),
                app: AppId::Wadbrowser,
                new: NewVariant::Site { name: a.name.clone(), url: a.url.clone() },
            }),
            // The machine's browser's own web app, by its id there.
            drag: Some(Held::WebApp { id: a.id.clone() }),
            off: None,
            note: None,
        });
    }
    let its: Vec<&Ext> = x
        .exts
        .iter()
        .filter(|e| e.app == AppId::Wadbrowser)
        .filter(|e| e.url.as_deref().is_some_and(|u| !known(&web, u) && !lib.web_apps.iter().any(|a| a.url == u)))
        .collect();
    for e in its {
        let name = e.name.clone().unwrap_or_else(|| e.id.clone());
        web.push(Item {
            key: format!("its:{}", e.id),
            icon: site_icon(&e.id, &name, e.url.as_deref()),
            name,
            lit: Some(format!("ext:wadbrowser:{}", e.id)),
            add: None,
            drag: Some(Held::Part {
                payload: Payload::Piece {
                    section: Section::Addons,
                    item: format!("wadbrowser:{}", e.id),
                    from: Some(x.id.clone()),
                },
            }),
            off: None,
            note: None,
        });
    }

    let exts = extensions()
        .iter()
        .map(|e| Item {
            key: e.id.clone(),
            name: e.name.clone(),
            icon: if e.id == "anthropic.claude-code" {
                ItemIcon::Img("/icons/claude.png")
            } else {
                ItemIcon::Glyph("puzzle")
            },
            lit: has(AppId::Vscode, &|y| y.id == e.id),
            add: addon(format!("vscode:{}", e.id)),
            drag: piece(format!("vscode:{}", e.id)),
            off: None,
            note: None,
        })
        .collect();
    let tools = tools()
        .iter()
        .map(|t| Item {
            key: t.id.clone(),
            name: t.name.clone(),
            icon: ItemIcon::Glyph("square-terminal"),
            lit: has(AppId::Terminal, &|y| y.id == t.id),
            add: addon(format!("terminal:{}", t.id)),
            drag: piece(format!("terminal:{}", t.id)),
            off: None,
            note: None,
        })
        .collect();
    // Steam's games: what the wadspace's Steam account has (signed in to in
    // the wadspace, read by wadd from its key's folder), and what's packaged
    // (in your GitHub packages, from any machine); and Dolphin's discs:
    // listed, lit when in it.
    let steam = steam_games(lib, x)
        .into_iter()
        .map(|g| Item {
            key: format!("steam:{}", g.id),
            lit: has(AppId::Steam, &|y| y.id == g.id),
            add: addon(format!("steam:{}", g.id)),
            icon: ItemIcon::Glyph("gamepad-2"),
            note: Some(g.state.words().to_string()),
            name: g.name,
            drag: None,
            off: None,
        })
        .collect();
    let dolphin = lib
        .discs
        .values()
        .map(|d| Item {
            key: format!("dolphin:{}", d.id),
            name: d.name.clone(),
            icon: ItemIcon::Glyph("disc-3"),
            lit: has(AppId::Dolphin, &|y| y.id == d.id || y.name.as_deref() == Some(d.name.as_str())),
            add: Some(Op::NewVariant { ws: x.id.clone(), app: AppId::Dolphin, new: NewVariant::Named(d.name.clone()) }),
            drag: None,
            off: None,
            note: None,
        })
        .collect();

    [
        Group { id: GroupId::Web, title: "Web apps", app: AppId::Wadbrowser, items: web },
        Group { id: GroupId::Extensions, title: "VS Code extensions", app: AppId::Vscode, items: exts },
        Group { id: GroupId::Tools, title: "Command line", app: AppId::Terminal, items: tools },
        Group { id: GroupId::Steam, title: "Steam games", app: AppId::Steam, items: steam },
        Group { id: GroupId::Dolphin, title: "Dolphin games", app: AppId::Dolphin, items: dolphin },
    ]
    .into_iter()
    .filter(|g| x.has_app(g.app))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_model::edit::{Ctx, apply};

    fn sample() -> Library {
        let text = include_str!("../../../../../crates/wad-model/tests/data/populated.json");
        serde_json::from_str(text).expect("the sample library")
    }

    fn group(gs: &[Group], id: GroupId) -> &Group {
        gs.iter().find(|g| g.id == id).expect("that group")
    }

    fn item<'a>(g: &'a Group, key: &str) -> &'a Item {
        g.items.iter().find(|i| i.key == key).expect("that item")
    }

    #[test]
    fn only_the_add_ons_of_its_apps_lit_when_in_it() {
        let lib = sample();
        let x = lib.wadspace("vanua-academy").unwrap();
        let gs = groups(&lib, x);
        assert_eq!(gs.iter().map(|g| g.id.id()).collect::<Vec<_>>(), ["web", "extensions", "tools"]);
        let web = group(&gs, GroupId::Web);
        assert_eq!(item(web, "gmail").lit.as_deref(), Some("ext:wadbrowser:gmail"));
        assert_eq!(item(web, "youtube").lit, None);
        // Calendar, the machine's browser's own web app, isn't one of the catalog's sites: it's offered as its own.
        assert!(
            web.items.iter().any(|i| i.key.starts_with("own:")),
            "{:?}",
            web.items.iter().map(|i| &i.key).collect::<Vec<_>>()
        );
        let tools = group(&gs, GroupId::Tools);
        for t in ["claude-code", "ori", "hplip", "rust"] {
            item(tools, t);
        }
        assert!(!tools.items.iter().any(|i| i.key == "go" || i.key == "git-lfs"));
        assert_eq!(
            item(group(&gs, GroupId::Extensions), "anthropic.claude-code").icon,
            ItemIcon::Img("/icons/claude.png")
        );
    }

    #[test]
    fn a_click_adds_it_or_takes_it_out() {
        let mut lib = sample();
        let x = lib.wadspace("vanua-academy").unwrap().clone();
        let gs = groups(&lib, &x);
        let web = group(&gs, GroupId::Web);
        assert_eq!(
            toggle(&x, item(web, "gmail")),
            Some(Op::Remove { ws: x.id.clone(), chip: "ext:wadbrowser:gmail".into() })
        );
        let yt = toggle(&x, item(web, "youtube")).unwrap();
        assert_eq!(
            yt,
            Op::NewVariant {
                ws: x.id.clone(),
                app: AppId::Wadbrowser,
                new: NewVariant::Addon("wadbrowser:youtube".into())
            }
        );
        assert_eq!(
            item(web, "youtube").drag,
            Some(Held::Part {
                payload: Payload::Piece { section: Section::Addons, item: "wadbrowser:youtube".into(), from: None }
            })
        );
        let ctx = Ctx::new(1, "m-surface");
        apply(&mut lib, yt, &ctx).unwrap();
        let x = lib.wadspace("vanua-academy").unwrap();
        assert!(item(group(&groups(&lib, x), GroupId::Web), "youtube").lit.is_some());
        // Focus, on and off.
        assert_eq!(focus(x), None);
        let on = focus_toggle(x);
        apply(&mut lib, on, &ctx).unwrap();
        let x = lib.wadspace("vanua-academy").unwrap();
        assert_eq!(focus(x).as_deref(), Some("ext:wadbrowser:focus"));
        assert!(matches!(focus_toggle(x), Op::Remove { .. }));
    }

    #[test]
    fn your_own_web_apps_and_its_own_that_no_list_has() {
        let mut lib = sample();
        lib.web_apps.push(wad_model::BrowserApp {
            id: "notes".into(),
            name: "Notes".into(),
            url: "https://notes.example.com".into(),
            profile: "default".into(),
        });
        let ctx = Ctx::new(1, "m-surface");
        apply(
            &mut lib,
            Op::AddWebApp { ws: "travel".into(), name: "Maps".into(), url: "maps.example.org".into() },
            &ctx,
        )
        .unwrap();
        let x = lib.wadspace("travel").unwrap();
        let gs = groups(&lib, x);
        let web = group(&gs, GroupId::Web);
        let own = item(web, "own:notes");
        assert_eq!(own.lit, None);
        assert!(matches!(&own.add, Some(Op::NewVariant { new: NewVariant::Site { .. }, .. })));
        let its = item(web, "its:maps");
        assert_eq!(its.lit.as_deref(), Some("ext:wadbrowser:maps"));
        assert_eq!(its.add, None);
    }

    #[test]
    fn a_steam_wadspaces_games_are_its_accounts_and_downloaded_ones_package() {
        let mut lib = sample();
        let mut x = lib.wadspace("couch-co-op").unwrap().clone();
        let key = wad_model::KeyRef { id: "steam-1".into(), name: "Steam".into(), kind: wad_model::SecretKind::Steam };
        x.secrets.by_slot.insert("app:steam".into(), key);
        // Signed in to in the wadspace (wadd read the key's folder): two games, one downloaded.
        let mut library =
            wad_model::SteamLibrary { owned: vec!["portal-2".into(), "celeste".into()], ..Default::default() };
        library.installed.insert("portal-2".into(), true);
        lib.steam.insert("steam-1".into(), library);
        // And one packaged on another machine.
        lib.game_packages.insert(
            "hollow-knight".into(),
            wad_model::GamePackage {
                game: "hollow-knight".into(),
                reference: "ghcr.io/hunggod/wadspaces-game-hollow-knight:367520".into(),
                mb: 0,
                machine: "m-desk".into(),
                at: 0,
            },
        );
        let games = steam_games(&lib, &x);
        let seen: Vec<(&str, GameState)> = games.iter().map(|g| (g.id.as_str(), g.state)).collect();
        assert_eq!(
            seen,
            [
                ("celeste", GameState::InLibrary),
                ("hollow-knight", GameState::Packaged),
                ("portal-2", GameState::Downloaded),
            ]
        );
        let steam = group(&groups(&lib, &x), GroupId::Steam).clone();
        assert_eq!(item(&steam, "steam:portal-2").note.as_deref(), Some("downloaded here"));
        // Its menu: a downloaded one packages; the others say why not.
        let pick = |g: &SteamGame| match &steam_menu(g)[1] {
            Choice::Item(p) => p.clone(),
            other => panic!("{other:?}"),
        };
        let portal = pick(&games[2]);
        assert_eq!(
            (portal.off, portal.act),
            (None, Act::PackageGame { account: "steam-1".into(), game: "portal-2".into() })
        );
        assert_eq!(pick(&games[0]).off.as_deref(), Some("Download it in Steam, in this wadspace, first"));
        assert_eq!(pick(&games[1]).off.as_deref(), Some("It's in your packages already"));
        // Not signed in (no key): only what's packaged.
        x.secrets.by_slot.clear();
        assert_eq!(steam_games(&lib, &x).iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), ["hollow-knight"]);
    }

    #[test]
    fn packaged_games_and_discs_are_listed() {
        let mut lib = sample();
        // Not signed in to Steam (no key): only what's packaged.
        let mut couch = lib.wadspace("couch-co-op").unwrap().clone();
        couch.secrets.by_slot.shift_remove("app:steam");
        assert!(group(&groups(&lib, &couch), GroupId::Steam).items.is_empty(), "nothing packaged yet");
        for game in ["stardew-valley", "hollow-knight"] {
            let p = wad_model::GamePackage {
                game: game.into(),
                reference: format!("ghcr.io/hunggod/wadspaces-game-{game}:1"),
                mb: 500,
                machine: "m-desk".into(),
                at: 0,
            };
            lib.game_packages.insert(game.into(), p);
        }
        let gs = groups(&lib, &couch);
        let steam = group(&gs, GroupId::Steam);
        assert_eq!(
            steam.items.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(),
            ["steam:hollow-knight", "steam:stardew-valley"]
        );
        let stardew = item(steam, "steam:stardew-valley");
        assert!(stardew.lit.is_some(), "couch-co-op has it");
        assert_eq!(stardew.off, None);
        let hollow = item(steam, "steam:hollow-knight");
        assert_eq!(
            hollow.add,
            Some(Op::NewVariant {
                ws: "couch-co-op".into(),
                app: AppId::Steam,
                new: NewVariant::Addon("steam:hollow-knight".into())
            })
        );
        let retro = lib.wadspace("retro").unwrap();
        let dolphin = group(&groups(&lib, retro), GroupId::Dolphin).clone();
        assert_eq!(
            dolphin.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["Mario Kart: Double Dash!!", "Mario Party 4"]
        );
        assert!(dolphin.items.iter().all(|i| i.lit.is_some() && i.off.is_none()));
    }

    #[test]
    fn suggested_first_for_its_apps_and_none_where_it_cant_change() {
        let lib = sample();
        let x = lib.wadspace("wadspaces").unwrap();
        let s = suggested(x, true);
        assert!(s.iter().any(|s| s.item == "vscode:eamodio.gitlens"));
        assert!(!s.iter().any(|s| s.item == "vscode:anthropic.claude-code"));
        assert!(suggested(x, false).is_empty());
        assert_eq!(
            add_addon(x, "vscode:eamodio.gitlens"),
            Some(Op::NewVariant {
                ws: "wadspaces".into(),
                app: AppId::Vscode,
                new: NewVariant::Addon("vscode:eamodio.gitlens".into())
            })
        );
    }

    #[test]
    fn a_discs_menu_says_where_its_used_and_takes_it_off_this_machine() {
        let lib = sample();
        let disc = lib.discs.get("mario-kart-double-dash").expect("the sample's disc").clone();
        let menu = disc_menu(&lib, &disc);
        let Choice::Heading(h) = &menu[0] else { panic!("it says where it's used: {menu:?}") };
        assert!(h.starts_with("Used in ") && h.contains("Retro"), "{h}");
        let Choice::Item(p) = &menu[1] else { panic!("then the way out: {menu:?}") };
        assert_eq!((p.label.as_str(), p.icon), ("Remove from this machine", "trash"));
        assert_eq!(p.act, Act::RemoveDisc { id: "mario-kart-double-dash".into() });
    }

    #[test]
    fn a_disc_no_wadspace_has_has_no_where_line() {
        let mut lib = sample();
        let disc = lib.discs.get("mario-party-4").expect("the sample's disc").clone();
        // Taken out of every wadspace that had it.
        for x in &mut lib.wadspaces {
            x.exts.retain(|e| !(e.app == AppId::Dolphin && (e.id == disc.id || e.name.as_deref() == Some(&disc.name))));
        }
        let menu = disc_menu(&lib, &disc);
        assert!(matches!(menu[0], Choice::Item(_)), "nothing to say about where: {menu:?}");
    }
}
