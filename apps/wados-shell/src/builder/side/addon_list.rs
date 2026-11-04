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
use wad_model::games::owned_games;
use wad_model::suggest::{Suggestion, suggestions};
use wad_model::{AppId, Ext, NewVariant, Section, Wadspace};

use crate::dnd::Held;

pub const STEAM_LATER: &str = "Steam's games come in phase 10, with Steam's sign-in";
pub const LINK_STEAM_LATER: &str = "Linking Steam comes in phase 10";
pub const DOLPHIN_LATER: &str = "Dolphin's games come in phase 10: their discs go in your GitHub packages";
pub const UPLOAD_LATER: &str = "Uploading discs comes in phase 10";

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
        })
        .collect();
    // Your signed-in Steam accounts' games, and Dolphin's discs: listed, lit when in it, off till phase 10.
    let steam = owned_games(&lib.steam)
        .into_iter()
        .map(|g| Item {
            key: format!("steam:{}:{}", g.account, g.game.id),
            lit: has(AppId::Steam, &|y| y.id == g.game.id),
            name: g.game.name,
            icon: ItemIcon::Glyph("gamepad-2"),
            add: Some(Op::AddGame { ws: x.id.clone(), account: g.account, game: g.game.id }),
            drag: None,
            off: Some(STEAM_LATER.into()),
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
            off: Some(DOLPHIN_LATER.into()),
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
        // Calendar, the machine's browser's own web app, is one of the usual sites already.
        assert!(!web.items.iter().any(|i| i.key.starts_with("own:")));
        let tools = group(&gs, GroupId::Tools);
        for t in ["claude-code", "hplip", "rust"] {
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
    fn games_are_listed_off_till_phase_10() {
        let lib = sample();
        let couch = lib.wadspace("couch-co-op").unwrap();
        let gs = groups(&lib, couch);
        let steam = group(&gs, GroupId::Steam);
        assert_eq!(steam.items.len(), 6);
        let stardew = item(steam, "steam:old-steam:stardew-valley");
        assert!(stardew.lit.is_some());
        assert_eq!(stardew.off.as_deref(), Some(STEAM_LATER));
        assert_eq!(toggle(couch, stardew), None);
        let retro = lib.wadspace("retro").unwrap();
        let dolphin = group(&groups(&lib, retro), GroupId::Dolphin).clone();
        assert_eq!(
            dolphin.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["Mario Kart: Double Dash!!", "Mario Party 4"]
        );
        assert!(dolphin.items.iter().all(|i| i.lit.is_some() && i.off.is_some()));
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
}
