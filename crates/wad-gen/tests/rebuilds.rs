// A wadspace rebuilds only when its apps, add-ons or commands change (its
// set's key: its packages, the base, its commands). Its look (background,
// icon), its name, its tag, its web apps, its sign-ins and what it starts
// into go into /run/wadspaces-extra when it starts: they never rebuild it.
// And wad-model's edits say so too (`needs_build`), so the Builder's hammer
// and wadd's set agree.

use wad_gen::Set;
use wad_model::edit::{NewVariant, Op};
use wad_model::{AppId, CommandKind, Ctx, IconPatch, Library, Wallpaper, apply};

fn library() -> Library {
    serde_json::from_str(include_str!("../../wad-model/tests/data/populated.json"))
        .expect("populated.json is a library")
}

const WS: &str = "vanua-academy";

fn key(lib: &Library) -> Option<String> {
    Set::of(lib.wadspace(WS).unwrap(), "sha256:base").key
}

/// `op` on the sample: whether its set's key changed, and whether the edit said it needs a build.
fn after(op: Op) -> (bool, bool) {
    let mut lib = library();
    let before = key(&lib);
    let shown = format!("{op:?}");
    let out = apply(&mut lib, op, &Ctx::new(1_793_140_200_000, "m-surface")).unwrap_or_else(|e| panic!("{shown}: {e}"));
    (key(&lib) != before, out.needs_build)
}

fn new_extension() -> String {
    let lib = library();
    let x = lib.wadspace(WS).unwrap();
    let e = wad_model::catalog::extensions()
        .iter()
        .find(|e| !x.exts.iter().any(|y| y.id == e.id))
        .expect("an extension it hasn't");
    format!("vscode:{}", e.id)
}

fn new_tool() -> String {
    let lib = library();
    let x = lib.wadspace(WS).unwrap();
    let t =
        wad_model::catalog::tools().iter().find(|t| !x.exts.iter().any(|y| y.id == t.id)).expect("a tool it hasn't");
    format!("terminal:{}", t.id)
}

#[test]
fn apps_add_ons_and_commands_rebuild_it() {
    let ws = || WS.to_string();
    for op in [
        Op::AddApp { ws: ws(), app: AppId::Krita },
        Op::RemoveApp { ws: ws(), app: AppId::Vscode },
        Op::NewVariant { ws: ws(), app: AppId::Vscode, new: NewVariant::Addon(new_extension()) },
        Op::NewVariant { ws: ws(), app: AppId::Terminal, new: NewVariant::Addon(new_tool()) },
        Op::AddCommand { ws: ws(), kind: CommandKind::Run, text: "dnf -y install ripgrep".into() },
    ] {
        let shown = format!("{op:?}");
        assert_eq!(after(op), (true, true), "{shown}: its set changes, and the edit says it needs a build");
    }
}

#[test]
fn its_look_name_tag_web_apps_and_sign_ins_never_do() {
    let ws = || WS.to_string();
    for op in [
        Op::SetWallpaper { ws: ws(), wallpaper: Wallpaper::Image { value: "/wallpapers/art/retro.svg".into() } },
        Op::SetIcon {
            ws: ws(),
            patch: Some(IconPatch { zoom: Some(Some(1.6)), x: Some(Some(0.3)), y: None, picture: None }),
        },
        Op::Rename { ws: ws(), name: "Vanua".into() },
        Op::SetTag { ws: ws(), tag: None },
        Op::AddWebApp { ws: ws(), name: "Notion".into(), url: "https://www.notion.so".into() },
        Op::NewVariant {
            ws: ws(),
            app: AppId::Wadbrowser,
            new: NewVariant::Site { name: "Figma".into(), url: "https://www.figma.com".into() },
        },
        Op::SignIn { ws: ws(), variant: "ext:wadbrowser:gmail".into(), key: Some("work-web".into()) },
        Op::SetLaunch { ws: ws(), launch: Some("app:vscode".into()) },
    ] {
        let shown = format!("{op:?}");
        assert_eq!(after(op), (false, false), "{shown}: it goes in at start, not into its image");
    }
}
