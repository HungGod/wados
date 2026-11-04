// The spike's vitest cases that exercise the model (src/sim/variants.test.ts,
// suggest.test.ts, spaces.test.ts, sim.test.ts), on its populated sample,
// with what they expect made for one look per wadspace. And its browser
// tests' "two clicks or fewer" workflows (tests/e2e/workflows.spec.ts), as
// edits: each change one `Op`.

mod common;

use common::{act, ctx, library, s, ws};
use wad_model::chips::{ChipKind, chips_of};
use wad_model::folders::{Folder, Move, folders_of, move_to};
use wad_model::games::{NEW_ACCOUNT_GAMES, steam_busy};
use wad_model::groups::{groups_of, identity_of};
use wad_model::look::{icon_of, icon_picture_of};
use wad_model::suggest::suggestions;
use wad_model::tags::{GENERAL_ID, tag_groups, tag_of};
use wad_model::variants::{SignIn, VariantKind, find_variant, search, sign_in_of, variants_of, wallpaper_for};
use wad_model::wallpapers::{art_of, has_picture, is_art};
use wad_model::{
    AppId, Category, CommandKind, EditError, FileRef, IconLook, IconPatch, Library, NewVariant, Op, Payload, Section,
    SteamLibrary, Wallpaper, apply, undo,
};

fn kinds(lib: &Library, ws_id: &str, app: AppId) -> Vec<VariantKind> {
    variants_of(ws(lib, ws_id), app).into_iter().map(|v| v.kind).collect()
}

fn sign_in(lib: &Library, ws_id: &str, variant: &str) -> Option<SignIn> {
    let x = ws(lib, ws_id);
    sign_in_of(x, &find_variant(x, variant).expect("a variant"))
}

fn new_key(lib: &mut Library, name: &str, kind: wad_model::SecretKind) -> String {
    act(lib, Op::NewKey { name: s(name), kind }).created.expect("a key")
}

mod variants {
    use super::*;

    #[test]
    fn are_the_app_then_its_add_ons_and_named_variants_then_its_sign_ins() {
        let lib = library();
        let steam: Vec<(VariantKind, String)> =
            variants_of(ws(&lib, "couch-co-op"), AppId::Steam).into_iter().map(|v| (v.kind, v.name)).collect();
        assert_eq!(
            steam,
            [
                (VariantKind::App, s("Steam")),
                (VariantKind::Game, s("Stardew Valley")),
                (VariantKind::Game, s("Overcooked! 2")),
                (VariantKind::SignIn, s("Steam")),
            ]
        );
        // Its web apps' sign-ins: Work, the Google one and hunggod.
        let mut web = vec![VariantKind::App];
        web.extend([VariantKind::Web; 7]);
        web.extend([VariantKind::SignIn; 3]);
        assert_eq!(kinds(&lib, "vanua-academy", AppId::Wadbrowser), web);
        let x = ws(&lib, "vanua-academy");
        assert!(
            variants_of(x, AppId::Wadbrowser)
                .iter()
                .filter(|v| v.kind == VariantKind::Web)
                .all(|v| v.opens.starts_with("web:"))
        );
        // Claude Code's sign-in is VS Code's.
        assert_eq!(
            kinds(&lib, "vanua-academy", AppId::Vscode),
            [VariantKind::App, VariantKind::Extension, VariantKind::SignIn]
        );
    }

    #[test]
    fn a_new_one_a_game_no_build_needed_a_web_app_a_sign_in() {
        let mut lib = library();
        let id = s("couch-co-op");
        // Adding Dolphin needs a build; its games don't (they're its content, not in the image).
        assert!(act(&mut lib, Op::AddApp { ws: id.clone(), app: AppId::Dolphin }).needs_build);
        let game = act(
            &mut lib,
            Op::NewVariant { ws: id.clone(), app: AppId::Dolphin, new: NewVariant::Named(s("Mario Kart")) },
        );
        assert_eq!(game.created.as_deref(), Some("ext:dolphin:mario-kart"));
        assert!(!game.needs_build);
        assert_eq!(
            find_variant(ws(&lib, &id), "ext:dolphin:mario-kart").map(|v| v.opens),
            Some(s("variant:mario-kart"))
        );
        assert!(ws(&lib, &id).look.layout.icons.contains(&s("variant:mario-kart")));
        let out = act(&mut lib, Op::RemoveVariant { ws: id.clone(), variant: s("ext:dolphin:mario-kart") });
        assert!(!out.needs_build);
        assert!(!ws(&lib, &id).exts.iter().any(|e| e.id == "mario-kart"));
        assert!(!ws(&lib, &id).look.layout.icons.contains(&s("variant:mario-kart")));

        let gmail = act(
            &mut lib,
            Op::NewVariant { ws: id.clone(), app: AppId::Wadbrowser, new: NewVariant::Addon(s("wadbrowser:gmail")) },
        );
        assert_eq!(gmail.created.as_deref(), Some("ext:wadbrowser:gmail"));
        let key = act(
            &mut lib,
            Op::NewVariant {
                ws: id.clone(),
                app: AppId::Steam,
                new: NewVariant::NewKey { name: s("Steam (me)"), kind: wad_model::SecretKind::Steam },
            },
        );
        let v = find_variant(ws(&lib, &id), key.created.as_deref().expect("its variant")).expect("a sign-in");
        assert_eq!((v.kind, v.name.as_str(), v.app), (VariantKind::SignIn, "Steam (me)", AppId::Steam));
    }

    #[test]
    fn removing_an_app_takes_its_named_variants_and_their_icons_with_it() {
        let mut lib = library();
        act(&mut lib, Op::RemoveApp { ws: s("couch-co-op"), app: AppId::Steam });
        let x = ws(&lib, "couch-co-op");
        assert!(!x.exts.iter().any(|e| e.app == AppId::Steam));
        assert!(!x.look.layout.icons.iter().any(|i| i.starts_with("variant:")));
        // And what it started straight into, and its Steam account.
        assert_eq!(x.look.launch, None);
        assert!(x.secrets.by_slot.is_empty());
    }

    #[test]
    fn a_web_app_signs_in_as_the_browser_does_until_it_has_a_key_of_its_own() {
        let mut lib = library();
        let iq = s("intelligencequest");
        // Docs.rs isn't one any more: it isn't in it.
        assert!(find_variant(ws(&lib, &iq), "ext:wadbrowser:docs-rs").is_none());
        act(
            &mut lib,
            Op::NewVariant { ws: iq.clone(), app: AppId::Wadbrowser, new: NewVariant::Addon(s("wadbrowser:claude")) },
        );
        let claude = "ext:wadbrowser:claude";
        assert_eq!(sign_in(&lib, &iq, claude).map(|s| (s.key.id, s.own)), Some((s("hunggod"), false)));
        let personal = new_key(&mut lib, "Personal", wad_model::SecretKind::Web);
        assert_eq!(personal, "personal");
        act(&mut lib, Op::SignIn { ws: iq.clone(), variant: s(claude), key: Some(personal) });
        assert_eq!(sign_in(&lib, &iq, claude).map(|s| (s.key.id, s.own)), Some((s("personal"), true)));
        // The rest stay with the browser's; Personal is a sign-in of WadBrowser's now.
        assert_eq!(sign_in(&lib, &iq, "ext:wadbrowser:github").map(|s| s.key.id), Some(s("hunggod")));
        let names: Vec<String> = variants_of(ws(&lib, &iq), AppId::Wadbrowser)
            .into_iter()
            .filter(|v| v.kind == VariantKind::SignIn)
            .map(|v| v.name)
            .collect();
        assert_eq!(names, ["hunggod", "Personal"]);
        // Back as the browser does: its own group goes.
        act(&mut lib, Op::SignIn { ws: iq.clone(), variant: s(claude), key: None });
        assert_eq!(sign_in(&lib, &iq, claude).map(|s| (s.key.id, s.own)), Some((s("hunggod"), false)));
        let groups: Vec<&str> = ws(&lib, &iq).secrets.groups.iter().map(|g| g.key.id.as_str()).collect();
        assert_eq!(groups, ["hunggod"]);
    }

    #[test]
    fn an_apps_own_a_games_own_and_a_key_of_the_wrong_kind_does_nothing() {
        let mut lib = library();
        let couch = s("couch-co-op");
        let stardew = "ext:steam:stardew-valley";
        // Couch Co-op signs in with the sample's Steam account.
        assert_eq!(sign_in(&lib, &couch, "app:steam").map(|s| s.key.id), Some(s("old-steam")));
        assert_eq!(sign_in(&lib, &couch, stardew).map(|s| (s.key.id, s.own)), Some((s("old-steam"), false)));
        let alt = new_key(&mut lib, "Alt account", wad_model::SecretKind::Steam);
        act(&mut lib, Op::SignIn { ws: couch.clone(), variant: s(stardew), key: Some(alt) });
        assert_eq!(sign_in(&lib, &couch, stardew).map(|s| s.key.id), Some(s("alt-account")));
        let was = lib.clone();
        let wrong =
            apply(&mut lib, Op::SignIn { ws: couch.clone(), variant: s(stardew), key: Some(s("work-web")) }, &ctx());
        assert!(matches!(wrong, Err(EditError::Refused(_))));
        assert_eq!(lib, was);
        // The browser has none to give (Dolphin's games have none at all).
        assert_eq!(sign_in(&lib, &couch, "app:wadbrowser"), None);
    }

    #[test]
    fn opening_one_is_on_its_wallpaper_or_its_apps() {
        let mut lib = library();
        let koi = art_of("koi");
        act(
            &mut lib,
            Op::SetAppWallpaper { ws: s("couch-co-op"), variant: s("app:steam"), wallpaper: Some(koi.clone()) },
        );
        let x = ws(&lib, "couch-co-op");
        let stardew = find_variant(x, "ext:steam:stardew-valley").expect("Stardew");
        assert_eq!(stardew.opens, "variant:stardew-valley");
        assert_eq!(wallpaper_for(x, &stardew), Some(&koi));
        // Its own wins over its app's.
        let own = art_of("moss");
        act(
            &mut lib,
            Op::SetAppWallpaper { ws: s("vanua-academy"), variant: s("app:wadbrowser"), wallpaper: Some(koi.clone()) },
        );
        act(
            &mut lib,
            Op::SetAppWallpaper {
                ws: s("vanua-academy"),
                variant: s("ext:wadbrowser:github"),
                wallpaper: Some(own.clone()),
            },
        );
        let x = ws(&lib, "vanua-academy");
        let gh = find_variant(x, "ext:wadbrowser:github").expect("GitHub");
        assert_eq!(wallpaper_for(x, &gh), Some(&own));
        // Taken off: its app's again.
        act(
            &mut lib,
            Op::SetAppWallpaper { ws: s("vanua-academy"), variant: s("ext:wadbrowser:github"), wallpaper: None },
        );
        let x = ws(&lib, "vanua-academy");
        assert_eq!(wallpaper_for(x, &find_variant(x, "ext:wadbrowser:github").expect("GitHub")), Some(&koi));
    }

    #[test]
    fn alt_space_finds_variants_by_name_this_machines_only() {
        let lib = library();
        let found = |q: &str| -> Vec<String> {
            search(&lib.wadspaces, common::HERE, q, 40)
                .iter()
                .map(|f| format!("{}/{}", f.ws.id, f.variant.id))
                .collect()
        };
        assert_eq!(found("stardew"), ["couch-co-op/ext:steam:stardew-valley"]);
        // Retro (and its Mario Kart) lives on Desk.
        assert!(found("mario kart").is_empty());
        assert!(found("").len() > 5);
    }
}

mod suggest {
    use super::*;

    #[test]
    fn add_ons_for_the_apps_in_it_that_it_hasnt_what_goes_with_what_it_has_first() {
        let mut lib = library();
        let id = act(&mut lib, Op::Create { name: Some(s("Rusty")), category: Some(Category::Work), machine: None })
            .created
            .expect("made");
        act(
            &mut lib,
            Op::NewVariant { ws: id.clone(), app: AppId::Terminal, new: NewVariant::Addon(s("terminal:rust")) },
        );
        act(&mut lib, Op::AddApp { ws: id.clone(), app: AppId::Vscode });
        act(
            &mut lib,
            Op::NewVariant { ws: id.clone(), app: AppId::Wadbrowser, new: NewVariant::Addon(s("wadbrowser:github")) },
        );
        let sg = suggestions(ws(&lib, &id), 3);
        let of = |app: AppId| sg.iter().filter(|x| x.app == app).collect::<Vec<_>>();
        assert_eq!(of(AppId::Vscode)[0].item, "vscode:rust-lang.rust-analyzer");
        assert_eq!(of(AppId::Vscode)[0].why.as_deref(), Some("Goes with Rust"));
        assert_eq!(
            (of(AppId::Wadbrowser)[0].id.as_str(), of(AppId::Wadbrowser)[0].why.as_deref()),
            ("claude", Some("Goes with VS Code"))
        );
        // Not what the catalog dropped: Docs.rs, Figma, Notion; Go.
        assert!(
            !sg.iter().any(|x| ["docs-rs", "figma", "notion", "golang.go", "go", "git-lfs"].contains(&x.id.as_str()))
        );
        // Not what's in it already: Rust, GitHub.
        assert!(!sg.iter().any(|x| x.item == "terminal:rust" || x.item == "wadbrowser:github"));
        assert_eq!(of(AppId::Vscode).len(), 3);
        // Taken, a suggestion makes way for the next.
        act(
            &mut lib,
            Op::NewVariant {
                ws: id.clone(),
                app: AppId::Vscode,
                new: NewVariant::Addon(s("vscode:rust-lang.rust-analyzer")),
            },
        );
        assert!(!suggestions(ws(&lib, &id), 3).iter().any(|x| x.id == "rust-lang.rust-analyzer"));
        // An app it hasn't gets none.
        assert!(!sg.iter().any(|x| x.app == AppId::Steam));
    }

    #[test]
    fn commands_of_its_own_run_or_copy_as_typed_named_run_1_steps_of_its_build() {
        let mut lib = library();
        let id = act(&mut lib, Op::Create { name: Some(s("Tools")), category: Some(Category::Work), machine: None })
            .created
            .expect("made");
        let add = |lib: &mut Library, kind: CommandKind, text: &str| {
            apply(lib, Op::AddCommand { ws: id.clone(), kind, text: s(text) }, &ctx()).map(|o| o.needs_build)
        };
        assert_eq!(
            add(&mut lib, CommandKind::Run, "   ").map_err(|e| e.to_string()),
            Err(s("Type the command to run"))
        );
        assert!(
            add(&mut lib, CommandKind::Copy, "~/notes")
                .is_err_and(|e| e.to_string().starts_with("COPY needs where from and where to"))
        );
        assert_eq!(add(&mut lib, CommandKind::Copy, "cargo install ripgrep"), Ok(true));
        assert_eq!(add(&mut lib, CommandKind::Run, "RUN cargo install ripgrep"), Ok(true));
        assert_eq!(add(&mut lib, CommandKind::Run, "COPY ~/dotfiles/.zshrc /home/sam/.zshrc"), Ok(true));
        assert_eq!(add(&mut lib, CommandKind::Run, "npm i -g pnpm"), Ok(true));
        // One line only.
        assert!(add(&mut lib, CommandKind::Run, "echo hi\nrm -rf ~").is_err());
        let cmds: Vec<String> =
            ws(&lib, &id).commands.iter().map(|c| format!("{} {:?} {}", c.name, c.kind, c.text)).collect();
        assert_eq!(
            cmds,
            [
                "COPY_1 Copy cargo install ripgrep",
                "RUN_1 Run cargo install ripgrep",
                "COPY_2 Copy ~/dotfiles/.zshrc /home/sam/.zshrc",
                "RUN_2 Run npm i -g pnpm",
            ]
        );
        // Taken out, the next goes on from the highest left (a gap isn't filled).
        for _ in 0..2 {
            let first = ws(&lib, &id).commands[0].id.clone();
            assert!(act(&mut lib, Op::RemoveCommand { ws: id.clone(), id: first }).needs_build);
        }
        add(&mut lib, CommandKind::Run, "cargo install just").expect("added");
        let names: Vec<&str> = ws(&lib, &id).commands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["COPY_2", "RUN_2", "RUN_3"]);
    }

    #[test]
    fn wados_comes_with_what_compiling_it_needs() {
        let lib = library();
        let os = ws(&lib, "wadspaces");
        assert_eq!(os.name, "WadOS");
        // The sample's WadOS is the starter, step for step. Their texts differ on
        // purpose: the spike's compiled v1 (WebKitGTK 4.1, binaryen), wados's
        // compiles wados (starter.rs's own test has the packages).
        let steps = |c: &[wad_model::BuildCommand]| c.iter().map(|c| (c.name.clone(), c.kind)).collect::<Vec<_>>();
        assert_eq!(steps(&os.commands), steps(&wad_model::starter::wados_commands()));
        assert!(os.exts.contains(&wad_model::Ext::new(AppId::Terminal, "rust")));
    }
}

mod spaces {
    use super::*;

    #[test]
    fn a_spaces_chips_are_everything_in_it() {
        let lib = library();
        let chips = chips_of(ws(&lib, "vanua-academy"));
        let mut want = vec![ChipKind::App; 3];
        want.extend([ChipKind::Web; 7]);
        want.extend([ChipKind::Addon; 2]);
        want.extend([ChipKind::Key; 4]);
        want.push(ChipKind::Project);
        assert_eq!(chips.iter().map(|c| c.kind).collect::<Vec<_>>(), want);
        assert_eq!(chips.iter().find(|c| c.id == "ext:wadbrowser:gmail").map(|c| c.name.as_str()), Some("Gmail"));
        // A look that was a customization of its own is a space of its own, with its files.
        let demo = ws(&lib, "vanua-academy-client-demo");
        assert_eq!(demo.name, "Client demo");
        let files: Vec<String> =
            chips_of(demo).into_iter().filter(|c| c.kind == ChipKind::Project).map(|c| c.name).collect();
        assert_eq!(files, ["Pitch deck.pdf"]);
    }

    #[test]
    fn one_click_takes_a_chip_out_undo_puts_it_back() {
        let mut lib = library();
        let out = act(&mut lib, Op::Remove { ws: s("vanua-academy"), chip: s("app:wadbrowser") });
        let x = ws(&lib, "vanua-academy");
        assert!(!x.has_app(AppId::Wadbrowser));
        assert!(!x.exts.iter().any(|e| e.id == "gmail"));
        assert!(out.needs_build);
        assert_eq!(out.toast.as_deref(), Some("Took WadBrowser out of VanuaAcademy"));
        let snap = out.undo.expect("an Undo");
        assert!(snap.id.starts_with("undo-"));
        let back = undo(&mut lib, &snap).expect("put back");
        assert!(back.needs_build);
        assert!(ws(&lib, "vanua-academy").exts.iter().any(|e| e.id == "gmail"));
        // A file: out of its look.
        act(&mut lib, Op::Remove { ws: s("vanua-academy-client-demo"), chip: s("project:deck") });
        let demo = ws(&lib, "vanua-academy-client-demo");
        assert!(demo.look.projects.is_empty());
        assert!(!demo.look.layout.icons.contains(&s("project:deck")));
        // What it started straight into goes with it; a web app it starts into stays when something else goes.
        act(&mut lib, Op::SetLaunch { ws: s("vanua-academy"), launch: Some(s("web:gmail")) });
        act(&mut lib, Op::Remove { ws: s("vanua-academy"), chip: s("ext:wadbrowser:github") });
        assert_eq!(ws(&lib, "vanua-academy").look.launch.as_deref(), Some("web:gmail"));
        act(&mut lib, Op::Remove { ws: s("vanua-academy"), chip: s("ext:wadbrowser:gmail") });
        assert_eq!(ws(&lib, "vanua-academy").look.launch, None);
    }

    #[test]
    fn dragged_onto_another_card_a_chip_is_copied() {
        let mut lib = library();
        let said = wad_model::ops::describe_add(
            ws(&lib, "cosmic-bodybuilding"),
            Section::Addons,
            "wadbrowser:gmail",
            &lib.keys,
            Some(ws(&lib, "vanua-academy")),
        );
        assert_eq!(said.said, "Adds Gmail; WadBrowser comes too");
        let copy = |from: &str, chip: &str| Op::CopyChip { from: s(from), to: s("cosmic-bodybuilding"), chip: s(chip) };
        assert!(act(&mut lib, copy("vanua-academy", "ext:wadbrowser:gmail")).needs_build);
        assert!(ws(&lib, "cosmic-bodybuilding").has_app(AppId::Wadbrowser));
        assert!(ws(&lib, "vanua-academy").exts.iter().any(|e| e.id == "gmail"));
        // A file, and a sign-in.
        assert!(!act(&mut lib, copy("vanua-academy-client-demo", "project:deck")).needs_build);
        assert!(ws(&lib, "cosmic-bodybuilding").look.projects.iter().any(|p| p.as_name == "Pitch deck.pdf"));
        act(&mut lib, copy("vanua-academy", "key:ext:anthropic.claude-code"));
        assert!(ws(&lib, "cosmic-bodybuilding").exts.iter().any(|e| e.id == "anthropic.claude-code"));
        // Again: it's there already.
        let again = apply(&mut lib, copy("vanua-academy", "ext:wadbrowser:gmail"), &ctx());
        assert_eq!(again, Err(EditError::Refused(s("It’s already in it"))));
    }

    #[test]
    fn a_new_one_is_empty_a_duplicate_is_a_copy_beside_it() {
        let mut lib = library();
        let id = act(&mut lib, Op::Create { name: None, category: Some(Category::Work), machine: None })
            .created
            .expect("made");
        assert_eq!(ws(&lib, &id).name, "New space");
        assert!(chips_of(ws(&lib, &id)).is_empty());
        let copy = act(&mut lib, Op::Duplicate { ws: s("vanua-academy"), machine: None }).created.expect("a copy");
        assert_eq!(chips_of(ws(&lib, &copy)).len(), chips_of(ws(&lib, "vanua-academy")).len());
        assert_eq!(ws(&lib, &copy).name, "VanuaAcademy (copy)");
        assert_eq!(ws(&lib, &copy).last_launched, None);
        let os = act(&mut lib, Op::Duplicate { ws: s("wadspaces"), machine: None }).created.expect("a copy");
        assert!(!ws(&lib, &os).starter);
    }

    #[test]
    fn steam_the_account_first_then_its_library() {
        let mut lib = library();
        let key = new_key(&mut lib, "Weekend", wad_model::SecretKind::Steam);
        // Its sign-in (wadd's job) reads its library.
        lib.steam.insert(
            key.clone(),
            SteamLibrary { owned: NEW_ACCOUNT_GAMES.map(String::from).to_vec(), ..Default::default() },
        );
        let a = act(&mut lib, Op::FromGame { account: key.clone(), game: s("celeste") }).created.expect("a gamespace");
        let x = ws(&lib, &a);
        assert_eq!((x.name.as_str(), x.category), ("Celeste", Some(Category::Gaming)));
        assert_eq!(x.look.launch.as_deref(), Some("variant:celeste"));
        // Another gamespace with it: only one plays per account at a time.
        let b = act(&mut lib, Op::Duplicate { ws: a.clone(), machine: None }).created.expect("a copy");
        assert_eq!(steam_busy(&lib.wadspaces, &[a.as_str()], ws(&lib, &b)).map(|y| y.id.as_str()), Some(a.as_str()));
        assert_eq!(steam_busy(&lib.wadspaces, &[b.as_str()], ws(&lib, &b)), None);
        // A game it doesn't own, it doesn't get.
        let hades = apply(&mut lib, Op::FromGame { account: key.clone(), game: s("hades") }, &ctx());
        assert_eq!(hades, Err(EditError::Refused(s("Weekend doesn’t own Hades"))));
        // The sample's account owns Hades.
        let h = act(&mut lib, Op::FromGame { account: s("old-steam"), game: s("hades") }).created.expect("a gamespace");
        assert_eq!(ws(&lib, &h).secrets.by_slot["app:steam"].id, "old-steam");
    }

    #[test]
    fn each_wadspace_by_its_most_telling_key_no_sign_in_last() {
        let lib = library();
        let groups = groups_of(lib.live());
        let names: Vec<String> =
            groups.iter().map(|g| g.key.as_ref().map_or(s("No sign-in"), |k| k.name.clone())).collect();
        assert_eq!(names, ["hunggod", "Notes vault", "Steam", "Work", "No sign-in"]);
        let work: Vec<&str> = groups
            .iter()
            .find(|g| g.key.as_ref().is_some_and(|k| k.id == "work-web"))
            .expect("Work")
            .spaces
            .iter()
            .map(|x| x.id.as_str())
            .collect();
        assert_eq!(work, ["vanua-academy", "vanua-academy-client-demo"]);
        // VanuaAcademy has Claude Code's key too: its web key is the one it's grouped by.
        assert_eq!(identity_of(ws(&lib, "vanua-academy")).map(|(_, k)| k.id), Some(s("work-web")));
    }

    #[test]
    fn dragged_into_another_group_a_wadspace_drops_its_key_for_the_groups() {
        let mut lib = library();
        act(&mut lib, Op::JoinGroup { ws: s("intelligencequest"), key: Some(s("work-web")) });
        assert_eq!(identity_of(ws(&lib, "intelligencequest")).map(|(_, k)| k.id), Some(s("work-web")));
        let groups: Vec<&str> =
            ws(&lib, "intelligencequest").secrets.groups.iter().map(|g| g.key.id.as_str()).collect();
        assert_eq!(groups, ["work-web"]);
        // Into Steam's group: Steam comes too.
        let out = act(&mut lib, Op::JoinGroup { ws: s("wadspaces"), key: Some(s("old-steam")) });
        assert!(ws(&lib, "wadspaces").has_app(AppId::Steam));
        assert!(out.needs_build);
        assert_eq!(out.toast.as_deref(), Some("WadOS: Steam signs in with Steam"));
        act(&mut lib, Op::JoinGroup { ws: s("wadspaces"), key: None });
        assert_eq!(identity_of(ws(&lib, "wadspaces")), None);
    }

    #[test]
    fn every_wadspace_has_a_picture_of_its_own_a_new_one_none() {
        let mut lib = library();
        let walls: Vec<&Wallpaper> = lib.live().into_iter().map(|x| &x.look.wallpaper).collect();
        assert!(walls.iter().all(|w| is_art(w)));
        let mut unique: Vec<&str> = walls.iter().filter_map(|w| w.value()).collect();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), walls.len());
        let id = act(&mut lib, Op::Create { name: None, category: Some(Category::Work), machine: None })
            .created
            .expect("made");
        assert!(!has_picture(&ws(&lib, &id).look.wallpaper));
        let bare: Vec<&str> =
            lib.live().into_iter().filter(|x| !has_picture(&x.look.wallpaper)).map(|x| x.id.as_str()).collect();
        assert_eq!(bare, [id.as_str()]);
        // Given art: the first nobody has.
        let taken: Vec<Wallpaper> = lib.live().into_iter().map(|x| x.look.wallpaper.clone()).collect();
        assert!(!taken.contains(&wad_model::wallpapers::default_art(&taken)));
    }

    #[test]
    fn tags_one_per_wadspace_general_for_the_rest_first_empty_tags_still_show() {
        let mut lib = library();
        let live = lib.live();
        let groups: Vec<(String, Vec<&str>)> = tag_groups(&lib.tags, &live)
            .into_iter()
            .map(|g| (g.tag.name, g.spaces.iter().map(|x| x.id.as_str()).collect()))
            .collect();
        assert_eq!(
            groups,
            [
                (s("General"), vec!["wadspaces", "desk-lab", "travel"]),
                (s("Studio"), vec!["vanua-academy", "vanua-academy-client-demo", "intelligencequest"]),
                (s("Writing"), vec!["cosmic-bodybuilding", "cosmic-bodybuilding-training"]),
                (s("Play"), vec!["couch-co-op", "retro"]),
            ]
        );
        let id = act(&mut lib, Op::CreateTag { name: s("Clients"), ws: None }).created.expect("a tag");
        let live = lib.live();
        let last = tag_groups(&lib.tags, &live).pop().expect("a group");
        assert_eq!((last.tag.name.as_str(), last.spaces.len()), ("Clients", 0));
        act(&mut lib, Op::SetTag { ws: s("wadspaces"), tag: Some(id) });
        assert_eq!(tag_of(&lib.tags, ws(&lib, "wadspaces")), "clients");
        assert_eq!(ws(&lib, "wadspaces").tag.as_deref(), Some("clients"));
        // A tag there's none of isn't one.
        assert!(matches!(
            apply(&mut lib, Op::SetTag { ws: s("wadspaces"), tag: Some(s("nope")) }, &ctx()),
            Err(EditError::NotFound { .. })
        ));
    }

    #[test]
    fn a_tag_taken_away_sends_its_wadspaces_back_to_general_a_copy_keeps_its_tag() {
        let mut lib = library();
        let copy = act(&mut lib, Op::Duplicate { ws: s("couch-co-op"), machine: None }).created.expect("a copy");
        assert_eq!(tag_of(&lib.tags, ws(&lib, &copy)), "play");
        act(&mut lib, Op::RenameTag { tag: s("play"), name: s("Games") });
        assert_eq!(lib.tags.iter().find(|t| t.id == "play").map(|t| t.name.as_str()), Some("Games"));
        let out = act(&mut lib, Op::RemoveTag { tag: s("play") });
        assert_eq!(out.changed, ["couch-co-op", "retro", copy.as_str()]);
        assert_eq!(tag_of(&lib.tags, ws(&lib, "couch-co-op")), GENERAL_ID);
        assert_eq!(ws(&lib, &copy).tag, None);
        // Made from a wadspace's tag menu: tagged with it at once.
        let id = act(&mut lib, Op::CreateTag { name: s("Retro games"), ws: Some(s("retro")) }).created.expect("a tag");
        assert_eq!(tag_of(&lib.tags, ws(&lib, "retro")), id);
    }

    fn names(fs: &[Folder]) -> Vec<(&'static str, String, String)> {
        fs.iter()
            .map(|f| {
                let items: Vec<&str> = f.items.iter().map(|v| v.name.as_str()).collect();
                (
                    if f.home { "home" } else { "own" },
                    f.key.as_ref().map_or(s("none"), |k| k.name.clone()),
                    items.join(","),
                )
            })
            .collect()
    }

    fn folder(home: bool, key: &str, items: &str) -> (&'static str, String, String) {
        (if home { "home" } else { "own" }, s(key), s(items))
    }

    #[test]
    fn the_apps_own_folder_first_then_one_per_other_key() {
        let lib = library();
        let x = ws(&lib, "vanua-academy");
        assert_eq!(
            names(&folders_of(x, AppId::Wadbrowser)),
            [
                folder(true, "none", "WadBrowser"),
                folder(false, "Work", "Claude,Gmail,Google Drive"),
                folder(false, "Google (VanuaAcademy)", "Google FireBase,Google Cloud,Squarespace"),
                folder(false, "hunggod", "GitHub"),
            ]
        );
        assert_eq!(
            names(&folders_of(x, AppId::Vscode)),
            [folder(true, "none", "VS Code"), folder(false, "VanuaAcademy-claude", "Claude Code")]
        );
        assert_eq!(names(&folders_of(x, AppId::Terminal)), [folder(true, "none", "Command line,Firebase CLI")]);
        assert_eq!(
            names(&folders_of(ws(&lib, "couch-co-op"), AppId::Steam)),
            [folder(true, "Steam", "Steam,Stardew Valley,Overcooked! 2")]
        );
    }

    #[test]
    fn a_thing_goes_to_a_folder_whose_key_is_its_kind() {
        let mut lib = library();
        let id = s("vanua-academy");
        let x = ws(&lib, &id);
        let gmail = find_variant(x, "ext:wadbrowser:gmail").expect("Gmail");
        let fs = folders_of(x, AppId::Wadbrowser);
        let hunggod = fs.iter().find(|f| f.key.as_ref().is_some_and(|k| k.id == "hunggod")).expect("hunggod's");
        assert_eq!(move_to(x, &gmail, &fs[1]), None);
        assert_eq!(
            move_to(x, &gmail, hunggod),
            Some(Move { key: Some(s("hunggod")), hint: s("Gmail signs in with hunggod") })
        );
        act(&mut lib, Op::SignIn { ws: id.clone(), variant: gmail.id.clone(), key: Some(s("hunggod")) });
        assert_eq!(names(&folders_of(ws(&lib, &id), AppId::Wadbrowser))[3], folder(false, "hunggod", "GitHub,Gmail"));
        // The browser signs in with nothing: back in its folder, neither does Gmail.
        let x = ws(&lib, &id);
        let home = &folders_of(x, AppId::Wadbrowser)[0];
        assert_eq!(
            move_to(x, &find_variant(x, &gmail.id).expect("Gmail"), home),
            Some(Move { key: None, hint: s("Gmail signs in with nothing") })
        );
        // Not into a folder of another kind of key, and an extension that can't sign in goes nowhere.
        let vs = folders_of(x, AppId::Vscode);
        assert_eq!(move_to(x, &find_variant(x, "app:vscode").expect("VS Code"), &vs[1]), None);
        let term = folders_of(x, AppId::Terminal);
        assert_eq!(move_to(x, &find_variant(x, "ext:terminal:firebase").expect("Firebase"), &term[0]), None);
        // The app itself into another key's folder: that key is its own, and its folder takes the others in.
        act(&mut lib, Op::SignIn { ws: id.clone(), variant: s("app:wadbrowser"), key: Some(s("hunggod")) });
        assert_eq!(
            names(&folders_of(ws(&lib, &id), AppId::Wadbrowser))[0],
            folder(true, "hunggod", "WadBrowser,GitHub,Gmail")
        );
    }

    #[test]
    fn a_key_group_dropped_on_the_desktop_is_a_new_wadspace_untitled_with_only_that() {
        let mut lib = library();
        let payload = Payload::Folder { ws: s("vanua-academy"), app: AppId::Wadbrowser, key: Some(s("vanua-google")) };
        let out = act(&mut lib, Op::Fresh { payload });
        assert_eq!(out.toast.as_deref(), Some("New wadspace “Untitled”"));
        let x = ws(&lib, out.created.as_deref().expect("made"));
        assert_eq!(x.name, "Untitled");
        assert_eq!(x.apps.iter().map(|a| a.app).collect::<Vec<_>>(), [AppId::Wadbrowser]);
        assert_eq!(
            x.exts.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["google-firebase", "google-cloud", "squarespace"]
        );
        let groups: Vec<(String, String)> =
            x.secrets.groups.iter().map(|g| (g.key.name.clone(), g.members.join(","))).collect();
        assert_eq!(groups, [(s("Google (VanuaAcademy)"), s("web:google-firebase,web:google-cloud,web:squarespace"))]);
        // Another: Untitled 2.
        let two =
            act(&mut lib, Op::Fresh { payload: Payload::Section { ws: s("vanua-academy"), section: Section::Apps } });
        assert_eq!(ws(&lib, two.created.as_deref().expect("made")).name, "Untitled 2");
        // A piece that isn't one makes nothing.
        let n = lib.wadspaces.len();
        let bad = apply(
            &mut lib,
            Op::Fresh { payload: Payload::Piece { section: Section::Addons, item: s("vscode:nope"), from: None } },
            &ctx(),
        );
        assert_eq!(bad, Err(EditError::Refused(s("Couldn’t make a wadspace of that: Not an add-on"))));
        assert_eq!(lib.wadspaces.len(), n);
    }

    #[test]
    fn its_icon_is_its_picture_its_square_kept_inside_null_puts_it_back() {
        let mut lib = library();
        let id = s("wadspaces");
        let icon = |lib: &mut Library, patch: Option<IconPatch>| {
            act(lib, Op::SetIcon { ws: id.clone(), patch });
        };
        assert_eq!(icon_of(ws(&lib, &id)), IconLook::default());
        assert_eq!(icon_picture_of(ws(&lib, &id)), &ws(&lib, &id).look.wallpaper);
        icon(&mut lib, Some(IconPatch { x: Some(Some(0.0)), y: Some(Some(0.9)), ..Default::default() }));
        // At 1×, the square's as tall as the picture: it can only move across, as far as its edges.
        assert_eq!(icon_of(ws(&lib, &id)), IconLook { zoom: 1.0, x: 0.3125, y: 0.5, picture: None });
        // A picture of its own: its icon is cut out of that; its background stays.
        let own = Wallpaper::image("/wallpapers/art/ember.svg");
        icon(&mut lib, Some(IconPatch { picture: Some(Some(own.clone())), ..Default::default() }));
        assert_eq!(icon_picture_of(ws(&lib, &id)), &own);
        assert_ne!(ws(&lib, &id).look.wallpaper, own);
        icon(&mut lib, Some(IconPatch { picture: Some(None), ..Default::default() }));
        assert_eq!(icon_picture_of(ws(&lib, &id)), &ws(&lib, &id).look.wallpaper);
        icon(&mut lib, Some(IconPatch { zoom: Some(Some(9.0)), y: Some(Some(0.9)), ..Default::default() }));
        let i = icon_of(ws(&lib, &id));
        assert_eq!(i.zoom, 3.0);
        assert!((i.y - (1.0 - 1.0 / 6.0)).abs() < 1e-9);
        icon(&mut lib, None);
        assert_eq!(ws(&lib, &id).look.icon, None);
    }
}

mod sim {
    use super::*;

    #[test]
    fn populated_this_machines_wadspaces_and_the_others_with_them() {
        let lib = library();
        let names: Vec<&str> = lib.live().into_iter().map(|x| x.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "WadOS",
                "Cosmic Bodybuilding",
                "Training log",
                "VanuaAcademy",
                "Client demo",
                "IntelligenceQuest",
                "Desk Lab",
                "Travel",
                "Couch Co-op",
                "Retro"
            ]
        );
        let gaming: Vec<&str> = lib
            .live()
            .into_iter()
            .filter(|x| x.category_or_default() == Category::Gaming)
            .map(|x| x.name.as_str())
            .collect();
        assert_eq!(gaming, ["Couch Co-op", "Retro"]);
        assert_eq!(lib.steam["old-steam"].installed.keys().collect::<Vec<_>>(), ["stardew-valley"]);
        assert_eq!(ws(&lib, "couch-co-op").look.launch.as_deref(), Some("variant:stardew-valley"));
        assert_eq!(lib.on("m-desk").iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["desk-lab", "retro"]);
    }

    #[test]
    fn the_same_edits_make_the_same_library() {
        let run = || {
            let mut lib = library();
            let mut outs = vec![];
            for op in [
                Op::Create { name: Some(s("Art studio")), category: None, machine: None },
                Op::AddCommand { ws: s("wadspaces"), kind: CommandKind::Run, text: s("cargo install just") },
                Op::KeepFiles {
                    ws: s("wadspaces"),
                    files: vec![FileRef {
                        machine: s("m-surface"),
                        path: s("~/Desktop/Thesis"),
                        name: s("Thesis"),
                        folder: true,
                    }],
                    wallpaper: None,
                },
                Op::Remove { ws: s("vanua-academy"), chip: s("app:terminal") },
                Op::NewWebApp { name: s(""), url: s("example.org/app") },
            ] {
                outs.push(act(&mut lib, op));
            }
            (serde_json::to_string(&lib).expect("serializes"), serde_json::to_string(&outs).expect("serializes"))
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn trashed_and_restored() {
        let mut lib = library();
        act(&mut lib, Op::Trash { ws: s("cosmic-bodybuilding") });
        assert_eq!(ws(&lib, "cosmic-bodybuilding").deleted_at, Some(common::EPOCH));
        assert!(!lib.live().iter().any(|x| x.id == "cosmic-bodybuilding"));
        act(&mut lib, Op::Restore { ws: s("cosmic-bodybuilding") });
        assert_eq!(ws(&lib, "cosmic-bodybuilding").deleted_at, None);
        // The Trash's toast has an Undo: it comes back out.
        let out = act(&mut lib, Op::Trash { ws: s("cosmic-bodybuilding") });
        assert_eq!(out.toast.as_deref(), Some("Moved to the Trash"));
        wad_model::undo(&mut lib, &out.undo.expect("an Undo")).expect("undone");
        assert_eq!(ws(&lib, "cosmic-bodybuilding").deleted_at, None);
        // A copy of a wadspace: "(copy)".
        let copy = act(&mut lib, Op::Duplicate { ws: s("vanua-academy"), machine: None }).created.expect("a copy");
        assert_eq!(ws(&lib, &copy).name, "VanuaAcademy (copy)");
    }

    #[test]
    fn the_model_stays_pure() {
        // No wall clock, no randomness, no files, network or environment outside tests.
        let bad =
            ["SystemTime", "Instant::now", "std::fs", "std::net", "std::env", "std::process", "rand::", "thread_rng"];
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for entry in std::fs::read_dir(&dir).expect("src") {
            let path = entry.expect("an entry").path();
            let text = std::fs::read_to_string(&path).expect("a file");
            let code = text.split("#[cfg(test)]").next().unwrap_or("");
            for b in bad {
                assert!(!code.contains(b), "{} uses {b}", path.display());
            }
        }
    }
}

/// The browser tests' workflows, two clicks or fewer: each change is one edit.
mod two_clicks {
    use super::*;

    #[test]
    fn a_wadspace_from_scratch() {
        let mut lib = library();
        // The desktop's menu: New wadspace (two clicks), in the Builder with its title ready to type.
        let id = act(&mut lib, Op::Create { name: None, category: None, machine: None }).created.expect("made");
        assert_eq!(ws(&lib, &id).name, "New wadspace");
        // On the desktop: empty glass (no background yet).
        assert!(!has_picture(&ws(&lib, &id).look.wallpaper));
        act(&mut lib, Op::Rename { ws: id.clone(), name: s("Art studio") });
        // Apps: Krita and the browser; Gmail; the browser signs in with Work (Gmail with it); a folder from the desktop; a look.
        for op in [
            Op::AddApp { ws: id.clone(), app: AppId::Krita },
            Op::AddApp { ws: id.clone(), app: AppId::Wadbrowser },
            Op::NewVariant { ws: id.clone(), app: AppId::Wadbrowser, new: NewVariant::Addon(s("wadbrowser:gmail")) },
            Op::SignIn { ws: id.clone(), variant: s("app:wadbrowser"), key: Some(s("work-web")) },
            Op::KeepFiles {
                ws: id.clone(),
                files: vec![FileRef {
                    machine: s("m-surface"),
                    path: s("~/Desktop/Thesis"),
                    name: s("Thesis"),
                    folder: true,
                }],
                wallpaper: None,
            },
            Op::SetWallpaper { ws: id.clone(), wallpaper: art_of("retro") },
        ] {
            act(&mut lib, op);
        }
        let x = ws(&lib, &id);
        let summary = [
            x.apps.iter().map(|a| a.app.as_str()).collect::<Vec<_>>().join(","),
            x.exts.iter().map(|e| e.id.as_str()).collect::<Vec<_>>().join(","),
            x.secrets.groups.iter().map(|g| g.key.id.as_str()).collect::<Vec<_>>().join(","),
            x.look
                .projects
                .iter()
                .map(|p| {
                    format!(
                        "{}:{}",
                        serde_json::to_value(p).expect("a project")["kind"].as_str().unwrap_or(""),
                        p.as_name
                    )
                })
                .collect::<Vec<_>>()
                .join(","),
            x.look.wallpaper.value().unwrap_or("").to_string(),
        ]
        .join(" | ");
        assert_eq!(summary, "krita,wadbrowser | gmail | work-web | mount:Thesis | /wallpapers/art/retro.svg");
        // Gmail's in Work's box.
        let fs = folders_of(x, AppId::Wadbrowser);
        let work = fs.iter().find(|f| f.key.as_ref().is_some_and(|k| k.id == "work-web")).expect("Work's");
        assert!(work.items.iter().any(|v| v.id == "ext:wadbrowser:gmail"));
    }

    #[test]
    fn changing_whats_in_one() {
        let mut lib = library();
        // Take out (and Undo).
        let out = act(&mut lib, Op::Remove { ws: s("vanua-academy"), chip: s("ext:wadbrowser:gmail") });
        assert_eq!(out.toast.as_deref(), Some("Took Gmail out of VanuaAcademy"));
        assert!(!ws(&lib, "vanua-academy").exts.iter().any(|e| e.id == "gmail"));
        undo(&mut lib, &out.undo.expect("an Undo")).expect("put back");
        assert!(ws(&lib, "vanua-academy").exts.iter().any(|e| e.id == "gmail"));
        // Copy to another (this one keeps it).
        act(
            &mut lib,
            Op::CopyChip { from: s("vanua-academy"), to: s("intelligencequest"), chip: s("ext:wadbrowser:gmail") },
        );
        assert!(ws(&lib, "intelligencequest").exts.iter().any(|e| e.id == "gmail"));
        assert!(ws(&lib, "vanua-academy").exts.iter().any(|e| e.id == "gmail"));
        // The browser's own sign-in.
        act(&mut lib, Op::SignIn { ws: s("vanua-academy"), variant: s("app:wadbrowser"), key: Some(s("hunggod")) });
        let browser = ws(&lib, "vanua-academy")
            .secrets
            .groups
            .iter()
            .find(|g| g.members.iter().any(|m| m == "browser"))
            .map(|g| g.key.id.as_str());
        assert_eq!(browser, Some("hunggod"));
        // Start straight into VS Code, in IntelligenceQuest.
        act(&mut lib, Op::SetLaunch { ws: s("intelligencequest"), launch: Some(s("app:vscode")) });
        assert!(chips_of(ws(&lib, "intelligencequest")).iter().any(|c| c.id == "app:vscode" && c.launch));
    }

    #[test]
    fn a_gamespace_steam_an_account_one_of_its_games() {
        let mut lib = library();
        // On the Gaming tab: Ctrl+N, its name typed.
        let id = act(&mut lib, Op::Create { name: None, category: Some(Category::Gaming), machine: None })
            .created
            .expect("made");
        assert_eq!(ws(&lib, &id).name, "New gamespace");
        act(&mut lib, Op::Rename { ws: id.clone(), name: s("Weekend") });
        act(&mut lib, Op::AddApp { ws: id.clone(), app: AppId::Steam });
        // The account first: Steam signs in with a new key (its sign-in, in the browser, is wadd's).
        let made = act(
            &mut lib,
            Op::NewVariant {
                ws: id.clone(),
                app: AppId::Steam,
                new: NewVariant::NewKey { name: s("Weekend"), kind: wad_model::SecretKind::Steam },
            },
        );
        assert_eq!(made.created.as_deref(), Some("key:app:steam"));
        assert_eq!(lib.key("weekend").map(|k| k.paths.len()), Some(3));
        lib.steam.insert(
            s("weekend"),
            SteamLibrary { owned: NEW_ACCOUNT_GAMES.map(String::from).to_vec(), ..Default::default() },
        );
        // Then one of its games: it plays it.
        act(&mut lib, Op::AddGame { ws: id.clone(), account: s("weekend"), game: s("celeste") });
        let x = ws(&lib, &id);
        assert_eq!(x.look.launch.as_deref(), Some("variant:celeste"));
        let fs = folders_of(x, AppId::Steam);
        assert_eq!(fs[0].key.as_ref().map(|k| k.id.as_str()), Some("weekend"));
        assert!(fs[0].items.iter().any(|v| v.id == "ext:steam:celeste"));
        // A copy of Couch Co-op keeps its tag.
        let copy = act(&mut lib, Op::Duplicate { ws: s("couch-co-op"), machine: None }).created.expect("a copy");
        assert_eq!(ws(&lib, &copy).tag.as_deref(), Some("play"));
    }

    #[test]
    fn a_disc_dropped_on_a_wadspace_with_dolphin() {
        let mut lib = library();
        let id = act(&mut lib, Op::ForApp { app: AppId::Dolphin }).created.expect("made");
        assert_eq!((ws(&lib, &id).name.as_str(), ws(&lib, &id).category), ("Dolphin", Some(Category::Gaming)));
        let out = act(&mut lib, Op::AddDisc { ws: id.clone(), file: s("mario-party-4.rvz") });
        assert_eq!(out.created.as_deref(), Some("ext:dolphin:mario-party-4"));
        assert_eq!(ws(&lib, &id).look.launch.as_deref(), Some("variant:mario-party-4"));
        // Not twice; one that isn't in your packages is uploaded first.
        assert!(matches!(
            apply(&mut lib, Op::AddDisc { ws: id.clone(), file: s("mario-party-4.rvz") }, &ctx()),
            Err(EditError::Refused(_))
        ));
        assert_eq!(
            apply(&mut lib, Op::AddDisc { ws: id.clone(), file: s("super-smash-bros-melee.rvz") }, &ctx()),
            Err(EditError::NotFound { what: "disc", id: s("super-smash-bros-melee") })
        );
    }
}

/// The browser's saved sign-ins for sites (src/sim/actions/browser.ts's
/// secrets.saveWeb and secrets.removeSite).
mod web_keys {
    use super::*;
    use wad_model::SecretKind;

    #[test]
    fn a_site_signs_in_with_a_new_web_key_or_one_it_has() {
        let mut lib = library();
        // A new one: a web key with that site, named as asked (or for the site).
        let out =
            act(&mut lib, Op::SaveWebKey { name: s("Personal"), site: s("https://www.GitLab.com/x"), existing: None });
        let id = out.created.expect("a key");
        let k = lib.key(&id).expect("made");
        assert_eq!(
            (k.name.as_str(), k.kind, k.sites.as_slice()),
            ("Personal", SecretKind::Web, &[s("gitlab.com")][..])
        );
        assert_eq!(out.toast.as_deref(), Some("Saved sign-in “Personal”"));
        assert!(out.changed.is_empty() && !out.needs_build, "no wadspace changes");
        let unnamed = act(&mut lib, Op::SaveWebKey { name: s(" "), site: s("news.ycombinator.com"), existing: None });
        assert_eq!(lib.key(&unnamed.created.expect("a key")).map(|k| k.name.as_str()), Some("news.ycombinator.com"));
        // One it has: the site joins its sites, once.
        for _ in 0..2 {
            let out =
                act(&mut lib, Op::SaveWebKey { name: s(""), site: s("gitlab.com"), existing: Some(s("hunggod")) });
            assert_eq!(out.created.as_deref(), Some("hunggod"));
            assert_eq!(out.toast.as_deref(), Some("gitlab.com signs in with hunggod"));
        }
        let k = lib.key("hunggod").expect("hunggod");
        assert_eq!(k.sites, [s("github.com"), s("gitlab.com")]);
        assert_eq!(k.changed_at, Some(common::EPOCH));
    }

    #[test]
    fn only_a_web_key_a_web_address_and_a_key_there_is() {
        let mut lib = library();
        let was = lib.clone();
        let r =
            apply(&mut lib, Op::SaveWebKey { name: s(""), site: s("x.com"), existing: Some(s("old-steam")) }, &ctx());
        assert_eq!(r, Err(EditError::Refused(s("Steam isn’t a web sign-in"))));
        let r = apply(&mut lib, Op::SaveWebKey { name: s(""), site: s("not a site"), existing: None }, &ctx());
        assert_eq!(r, Err(EditError::Refused(s("That isn’t a web address"))));
        let r = apply(&mut lib, Op::SaveWebKey { name: s(""), site: s("x.com"), existing: Some(s("gone")) }, &ctx());
        assert_eq!(r, Err(EditError::NotFound { what: "key", id: s("gone") }));
        let r = apply(&mut lib, Op::RemoveSite { key: s("gone"), site: s("x.com") }, &ctx());
        assert_eq!(r, Err(EditError::NotFound { what: "key", id: s("gone") }));
        assert_eq!(lib, was);
    }

    #[test]
    fn a_site_taken_off_a_key_the_key_stays() {
        let mut lib = library();
        act(&mut lib, Op::RemoveSite { key: s("work-web"), site: s("https://mail.google.com/mail/u/0") });
        let k = lib.key("work-web").expect("still there");
        assert_eq!(k.sites, [s("claude.ai"), s("drive.google.com")]);
        assert_eq!(k.changed_at, Some(common::EPOCH));
        // A site it doesn't have: nothing changes.
        let was = lib.clone();
        act(&mut lib, Op::RemoveSite { key: s("work-web"), site: s("github.com") });
        assert_eq!(lib, was);
    }

    #[test]
    fn they_read_as_the_spikes_actions() {
        let op: Op =
            serde_json::from_str(r#"{"op": "secrets.saveWeb", "name": "Work", "site": "github.com"}"#).expect("an op");
        assert_eq!(op, Op::SaveWebKey { name: s("Work"), site: s("github.com"), existing: None });
        let op: Op =
            serde_json::from_str(r#"{"op": "secrets.saveWeb", "name": "", "site": "x.com", "existing": "hunggod"}"#)
                .expect("an op");
        assert_eq!(op, Op::SaveWebKey { name: s(""), site: s("x.com"), existing: Some(s("hunggod")) });
        let op: Op = serde_json::from_str(r#"{"op": "secrets.removeSite", "key": "hunggod", "site": "github.com"}"#)
            .expect("an op");
        assert_eq!(op, Op::RemoveSite { key: s("hunggod"), site: s("github.com") });
    }
}

/// Another machine's wadspace copied here (`space.import`: a peer's "Copy
/// here", over the tailnet). Not the spike's: its copies came from the account.
mod import {
    use super::*;
    use wad_model::{CheckError, KeyRef, Project, ProjectKind, ProjectSource, SecretKind, Wadspace};

    /// Desk's VanuaAcademy, as its remote API sends it: a folder of Desk's
    /// in it, Desk's own tag, and a Google key only Desk has.
    fn from_desk(lib: &Library) -> Wadspace {
        let mut doc = ws(lib, "vanua-academy").clone();
        doc.machine = s("m-desk");
        doc.tag = Some(s("desk-only"));
        doc.last_launched = Some(5);
        let source = ProjectSource { machine_id: s("m-desk"), paths: vec![s("~/Pictures/Sprites")] };
        doc.look.projects.push(Project {
            id: s("sprites"),
            as_name: s("Sprites"),
            kind: ProjectKind::Mount { source, make: false },
        });
        doc.look.layout.icons.push(s("project:sprites"));
        let g = doc.secrets.groups.iter_mut().find(|g| g.id.as_deref() == Some("g-google")).expect("its Google group");
        g.key = KeyRef { id: s("desk-google"), name: s("Google (Desk)"), kind: SecretKind::Web };
        doc
    }

    fn import(lib: &mut Library, doc: &Wadspace) -> wad_model::Outcome {
        act(lib, Op::Import { doc: Box::new(doc.clone()), from: s("m-desk") })
    }

    #[test]
    fn a_copy_is_this_machines_with_a_new_id_and_never_built_here() {
        let mut lib = library();
        let doc = from_desk(&lib);
        let was = ws(&lib, "vanua-academy").clone();
        let out = import(&mut lib, &doc);
        // A new id by the usual rules (a slug of its name, as a duplicate's); the name kept as it is.
        let id = out.created.clone().expect("its id");
        assert_eq!(id, "vanuaacademy");
        assert_eq!(out.changed, std::slice::from_ref(&id));
        assert_eq!(out.toast.as_deref(), Some("VanuaAcademy is on this machine"));
        let x = ws(&lib, &id);
        assert_eq!((x.name.as_str(), x.machine.as_str()), ("VanuaAcademy", common::HERE));
        // Never built here: it needs a build, it isn't the starter (which comes built), and it hasn't been opened here.
        assert!(out.needs_build);
        assert!(!x.starter && x.last_launched.is_none());
        assert_eq!((x.created_at, x.updated_at), (common::EPOCH, common::EPOCH));
        assert!(x.order > lib.wadspaces.iter().filter(|y| y.id != id).map(|y| y.order).max().unwrap_or(0));
        // Its projects as they are: Desk's folder is still Desk's (it comes over Taildrive).
        assert_eq!(x.look.projects, doc.look.projects);
        assert!(
            x.look
                .projects
                .iter()
                .any(|p| matches!(&p.kind, ProjectKind::Mount { source, .. } if source.machine_id == "m-desk"))
        );
        assert_eq!((&x.apps, &x.exts, &x.look.layout), (&doc.apps, &doc.exts, &doc.look.layout));
        // This machine's own is as it was.
        assert_eq!(ws(&lib, "vanua-academy"), &was);
        // Again: another copy, another id (that one's taken now).
        assert_eq!(import(&mut lib, &doc).created.as_deref(), Some("vanuaacademy-2"));
    }

    #[test]
    fn a_tag_or_a_key_this_machine_doesnt_have_is_left_off() {
        let mut lib = library();
        let mut doc = from_desk(&lib);
        // A key this machine has, by another name there: named as it is here.
        doc.secrets.by_slot.get_mut("ext:anthropic.claude-code").expect("Claude Code's").name = s("Claude (Desk)");
        // A key with an id this machine has, of another kind: not the same key.
        doc.secrets
            .by_slot
            .insert(s("app:vscode"), KeyRef { id: s("old-steam"), name: s("VS Code"), kind: SecretKind::Vscode });
        let id = import(&mut lib, &doc).created.expect("its id");
        let x = ws(&lib, &id);
        assert_eq!(x.tag, None, "Desk's tag isn't one here: General");
        let groups: Vec<&str> = x.secrets.groups.iter().filter_map(|g| g.id.as_deref()).collect();
        assert_eq!(
            groups,
            ["g-work", "g-hunggod"],
            "the Google group's key is Desk's: its web apps sign in with nothing"
        );
        assert!(x.exts.iter().any(|e| e.id == "google-firebase"), "the web apps stay");
        let slots: Vec<(&str, &str)> = x.secrets.by_slot.iter().map(|(s, k)| (s.as_str(), k.name.as_str())).collect();
        assert_eq!(slots, [("ext:anthropic.claude-code", "VanuaAcademy-claude")]);
        // A tag this machine has stays.
        doc.tag = Some(s("studio"));
        let id = import(&mut lib, &doc).created.expect("its id");
        assert_eq!(ws(&lib, &id).tag.as_deref(), Some("studio"));
    }

    #[test]
    fn a_document_that_doesnt_check_out_is_refused() {
        let mut lib = library();
        let was = lib.clone();
        let good = from_desk(&lib);
        let try_ = |lib: &mut Library, doc: Wadspace, from: &str| {
            apply(lib, Op::Import { doc: Box::new(doc), from: s(from) }, &ctx())
        };
        let mut old = good.clone();
        old.version = 5;
        assert!(matches!(try_(&mut lib, old, "m-desk"), Err(EditError::Check(CheckError::Bad(_)))));
        let mut unnamed = good.clone();
        unnamed.name = s(" ");
        assert_eq!(try_(&mut lib, unnamed, "m-desk"), Err(EditError::Check(CheckError::Name)));
        let mut escape = good.clone();
        escape.look.projects[0].as_name = s("../../etc");
        assert!(matches!(try_(&mut lib, escape, "m-desk"), Err(EditError::Check(_))));
        assert!(matches!(
            try_(&mut lib, good, "../desk"),
            Err(EditError::Check(CheckError::Id { what: "machine", .. }))
        ));
        assert_eq!(lib, was);
    }

    #[test]
    fn it_reads_as_its_action() {
        let lib = library();
        let doc = serde_json::to_value(ws(&lib, "retro")).expect("a document");
        let op: Op = serde_json::from_value(serde_json::json!({"op": "space.import", "doc": doc, "from": "m-desk"}))
            .expect("an op");
        assert!(matches!(&op, Op::Import { doc, from } if doc.id == "retro" && from == "m-desk"));
    }
}

/// The browser's web apps of your own (src/sim/actions/browser.ts's
/// browser.newApp and browser.removeApp).
mod web_apps {
    use super::*;

    #[test]
    fn a_web_app_of_your_own_comes_and_goes() {
        let mut lib = library();
        let id = act(&mut lib, Op::NewWebApp { name: s("Docs"), url: s("docs.rs/gtk4") }).created.expect("an id");
        assert!(lib.web_apps.iter().any(|a| a.id == id && a.url == "https://docs.rs/gtk4"));
        let out = act(&mut lib, Op::RemoveWebApp { id: id.clone() });
        assert_eq!(out.toast.as_deref(), Some("Removed Docs"));
        assert!(out.changed.is_empty() && !out.needs_build, "no wadspace changes");
        assert!(!lib.web_apps.iter().any(|a| a.id == id));
        // The sample's own, Calendar, goes too; one that isn't there is an error, and nothing changes.
        act(&mut lib, Op::RemoveWebApp { id: s("calendar") });
        assert!(lib.web_apps.is_empty());
        let was = lib.clone();
        let r = apply(&mut lib, Op::RemoveWebApp { id: s("calendar") }, &ctx());
        assert_eq!(r, Err(EditError::NotFound { what: "web app", id: s("calendar") }));
        assert_eq!(lib, was);
    }

    #[test]
    fn it_reads_as_the_spikes_action() {
        let op: Op = serde_json::from_str(r#"{"op": "assets.removeWebApp", "id": "calendar"}"#).expect("an op");
        assert_eq!(op, Op::RemoveWebApp { id: s("calendar") });
    }
}
