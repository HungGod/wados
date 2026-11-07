//! What the board decides, with no widgets: a stack's state, what a piece
//! of it carries when it's dragged (and, dropped on a key's box in its own
//! stack, what it signs in with: `keys.ts` `pieceOf` and Stack.svelte's
//! `boxTarget`), where the slot goes, a piece's menu (`chipMenu.ts`), and
//! what a stack's desktop preview shows (`DeskPreview.svelte`).

use wad_model::catalog::app_name;
use wad_model::edit::Payload;
use wad_model::folders::{Move, folders_of, move_to};
use wad_model::ops::ext_key;
use wad_model::variants::{AppVariant, VariantKind, find_variant, key_kind_of};
use wad_model::wallpapers::PICTURES;
use wad_model::{AppId, Built, Phase, ProjectKind, Section, Wadspace, Wallpaper};

use crate::dnd::Held;

// ------------------------------------------------------------ its state

/// How a stack's state pill is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Another machine's.
    Away,
    /// Building, or opening.
    Busy,
    Run,
    Ok,
    /// It has to be built (never, or again).
    Todo,
}

impl Tone {
    pub fn class(self) -> &'static str {
        match self {
            Tone::Away => "away",
            Tone::Busy => "busy",
            Tone::Run => "run",
            Tone::Ok => "ok",
            Tone::Todo => "todo",
        }
    }
}

/// A stack's state, as its header says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub label: String,
    pub tone: Tone,
}

/// What its state is made from: another machine's (by its name), its
/// build's job, its run's phase and its image here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub away: Option<String>,
    pub building: bool,
    pub phase: Option<Phase>,
    pub built: Option<Built>,
}

/// The spike's: where it lives (another machine's), Building… or
/// Opening…, Running, Ready, then Not built or Rebuilds.
pub fn status(f: &Facts) -> Status {
    let s = |label: &str, tone| Status { label: label.to_string(), tone };
    if let Some(m) = &f.away {
        return s(m, Tone::Away);
    }
    if f.building {
        return s("Building…", Tone::Busy);
    }
    match f.phase {
        Some(Phase::Starting | Phase::Waiting) => return s("Opening…", Tone::Busy),
        Some(Phase::Ready) => return s("Running", Tone::Run),
        _ => {}
    }
    match f.built.unwrap_or_default() {
        Built::Built => s("Ready", Tone::Ok),
        Built::Never => s("Not built", Tone::Todo),
        Built::Update => s("Rebuilds", Tone::Todo),
    }
}

// ------------------------------------------------------------ drags

/// What dragging one of `ws`'s things carries: the app (by its id), or an
/// add-on as `app:id` (ops' items). Sign-ins aren't things.
pub fn piece(ws: &str, v: &AppVariant) -> Option<Payload> {
    let from = Some(ws.to_string());
    match v.kind {
        VariantKind::App => Some(Payload::Piece { section: Section::Apps, item: v.app.as_str().to_string(), from }),
        VariantKind::SignIn => None,
        _ => v.ext.as_ref().map(|e| Payload::Piece { section: Section::Addons, item: ext_key(e), from }),
    }
}

/// A piece dragged from `x`'s own stack, as the thing it is (none: not one of its things).
pub fn piece_of(x: &Wadspace, held: &Held) -> Option<AppVariant> {
    let Held::Part { payload: Payload::Piece { section, item, from } } = held else { return None };
    if from.as_deref() != Some(x.id.as_str()) {
        return None;
    }
    match section {
        Section::Apps => find_variant(x, &format!("app:{item}")),
        Section::Addons => find_variant(x, &format!("ext:{item}")),
        _ => None,
    }
}

/// Letting `held` go on `app`'s box for `key` in `x`'s stack: a thing of
/// its own signs in with that key (`apps.signIn` of the variant, with the
/// move's key). None: it doesn't go there.
pub fn onto_key(x: &Wadspace, held: &Held, app: AppId, key: &str) -> Option<(String, Move)> {
    let t = piece_of(x, held)?;
    if t.app != app {
        return None;
    }
    let f = folders_of(x, app).into_iter().find(|f| f.key.as_ref().is_some_and(|k| k.id == key))?;
    move_to(x, &t, &f).map(|m| (t.id, m))
}

/// Where the slot goes while `held` is: just right of the stack it came
/// from, else after the last (none: first, there being none).
pub fn slot_after(stacks: &[String], held: &Held) -> Option<String> {
    let beside = match held {
        Held::Space { from: Some(_), .. } => None,
        h => h.source_ws(),
    };
    beside.filter(|b| stacks.iter().any(|s| s == b)).map(str::to_string).or_else(|| stacks.last().cloned())
}

/// A stack's width: half the board's viewport (two side by side), less its
/// padding and the gap between them, up to what the Builder's window opens
/// with: a bigger window shows more stacks, not wider ones.
pub fn column(viewport: i32) -> i32 {
    ((viewport - 2 * BOARD_PAD - GAP) / 2).clamp(MIN_COLUMN, MAX_COLUMN)
}

/// The board's padding and the gap between stacks (board.css has them too).
pub const BOARD_PAD: i32 = 12;
pub const GAP: i32 = 12;
const MIN_COLUMN: i32 = 240;
const MAX_COLUMN: i32 = 420;

// ------------------------------------------------------------ a piece's menu

/// What a menu item does.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// Its wadspace opened (or brought up).
    Open,
    /// What it starts straight into (none: its desktop again).
    Launch(Option<String>),
    /// The wallpaper it opens on (none: its app's, or none of its own).
    Wallpaper(Option<Wallpaper>),
    /// Asks for its new name.
    Rename,
    /// Taken out (with Undo).
    Remove,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Heading(String),
    Sep,
    Item(Item),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// A test hook (`wadsctl stack pick <id>`).
    pub id: String,
    pub label: String,
    pub icon: &'static str,
    pub hint: Option<String>,
    pub checked: bool,
    pub danger: bool,
    pub act: Option<Act>,
    /// Its own menu (opens beside it).
    pub submenu: Vec<Entry>,
}

impl Item {
    fn new(id: impl Into<String>, label: impl Into<String>, icon: &'static str, act: Act) -> Self {
        Item {
            id: id.into(),
            label: label.into(),
            icon,
            hint: None,
            checked: false,
            danger: false,
            act: Some(act),
            submenu: vec![],
        }
    }
}

/// What it starts its wadspace straight into ('app:krita', 'web:gmail', 'variant:hades'), if it can.
pub fn launch_key(v: &AppVariant) -> Option<String> {
    match v.kind {
        VariantKind::App => Some(format!("app:{}", v.app)),
        VariantKind::Web => v.ext.as_ref().map(|e| format!("web:{}", e.id)),
        VariantKind::Game | VariantKind::Named => v.ext.as_ref().map(|e| format!("variant:{}", e.id)),
        _ => None,
    }
}

/// `x` starts straight into `v`.
pub fn launches(x: &Wadspace, v: &AppVariant) -> bool {
    launch_key(v).is_some_and(|k| x.look.launch.as_deref() == Some(k.as_str()))
}

/// A piece's menu (chipMenu.ts): what it is, then Open, Start straight
/// into it, its wallpaper, Rename (a web app's, a game's) and Take it out.
pub fn chip_menu(x: &Wadspace, v: &AppVariant) -> Vec<Entry> {
    let heading = match v.kind {
        VariantKind::App => app_name(v.app).to_string(),
        k => format!("{} · {}", v.name, k.label()),
    };
    let mut out = vec![Entry::Heading(heading)];
    out.push(Entry::Item(Item::new("chip-open", format!("Open {}", v.name), "monitor-play", Act::Open)));
    if let Some(key) = launch_key(v) {
        let on = x.look.launch.as_deref() == Some(key.as_str());
        let mut it = Item::new("chip-launch", "Start straight into it", "play", Act::Launch((!on).then_some(key)));
        it.checked = on;
        it.hint = Some("Full screen on its own, no desktop".into());
        out.push(Entry::Item(it));
    }
    let addon = matches!(v.kind, VariantKind::Mode | VariantKind::Extension | VariantKind::Tool);
    if key_kind_of(v).is_some() || !addon {
        let mut it = Item::new("wallpaper", "Set wallpaper", "image", Act::Open);
        it.act = None;
        it.submenu = wallpaper_items(v);
        out.push(Entry::Item(it));
    }
    if matches!(v.kind, VariantKind::Web | VariantKind::Game | VariantKind::Named) {
        out.push(Entry::Item(Item::new("rename", "Rename…", "pencil", Act::Rename)));
    }
    out.push(Entry::Sep);
    let mut it = Item::new("chip-remove", "Take it out", "minus", Act::Remove);
    it.danger = true;
    out.push(Entry::Item(it));
    out
}

/// What it opens on: one of WadSpaces' pictures, or (one of its own) none again.
fn wallpaper_items(v: &AppVariant) -> Vec<Entry> {
    let mut out = vec![Entry::Heading("Pictures".into())];
    for p in PICTURES {
        let wp = p.wallpaper();
        let mut it = Item::new(format!("wp:{}", p.id), p.name, "", Act::Wallpaper(Some(wp.clone())));
        it.checked = v.wallpaper.as_ref() == Some(&wp);
        out.push(Entry::Item(it));
    }
    if v.wallpaper.is_some() {
        out.push(Entry::Sep);
        let label = if v.kind == VariantKind::App {
            "No wallpaper of its own".to_string()
        } else {
            format!("{}’s wallpaper", app_name(v.app))
        };
        out.push(Entry::Item(Item::new("wp-none", label, "x", Act::Wallpaper(None))));
    }
    out
}

/// An item of `entries` (or of one's submenu) by its id.
pub fn find_item<'a>(entries: &'a [Entry], id: &str) -> Option<&'a Item> {
    entries.iter().find_map(|e| match e {
        Entry::Item(it) if it.id == id => Some(it),
        Entry::Item(it) => find_item(&it.submenu, id),
        _ => None,
    })
}

// ------------------------------------------------------------ its desktop

/// One of its desktop's icons, as the preview shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct DeskIcon {
    /// Its layout key ('app:vscode', 'web:gmail', 'project:iq').
    pub id: String,
    pub name: String,
    /// The thing it opens (none: a project, a glyph).
    pub thing: Option<AppVariant>,
    pub glyph: Option<&'static str>,
    /// An app's add-ons: "focus" on the browser, how many on VS Code or the command line.
    pub badge: Option<String>,
}

/// An app's icon's badge: Focus on the browser, how many add-ons VS Code or the command line has.
pub fn badge_of(x: &Wadspace, app: AppId) -> Option<String> {
    if app == AppId::Wadbrowser {
        return x.exts.iter().any(|e| e.app == AppId::Wadbrowser && e.id == "focus").then(|| "focus".to_string());
    }
    if !matches!(app, AppId::Vscode | AppId::Terminal) {
        return None;
    }
    let n = x.exts.iter().filter(|e| e.app == app).count();
    (n > 0).then(|| n.to_string())
}

/// Its desktop's icons, as they're laid out.
pub fn desk_icons(x: &Wadspace) -> Vec<DeskIcon> {
    x.look
        .layout
        .icons
        .iter()
        .filter_map(|id| {
            if let Some(pid) = id.strip_prefix("project:") {
                let p = x.look.projects.iter().find(|p| p.id == pid)?;
                let glyph = if matches!(p.kind, ProjectKind::Copy { .. }) { "file" } else { "folder" };
                return Some(DeskIcon {
                    id: id.clone(),
                    name: p.as_name.clone(),
                    thing: None,
                    glyph: Some(glyph),
                    badge: None,
                });
            }
            let vid = if id.starts_with("app:") {
                id.clone()
            } else if let Some(w) = id.strip_prefix("web:") {
                format!("ext:wadbrowser:{w}")
            } else {
                let e = id.strip_prefix("variant:")?;
                format!("ext:{}:{e}", x.exts.iter().find(|y| y.id == e)?.app)
            };
            let t = find_variant(x, &vid)?;
            let badge = (t.kind == VariantKind::App).then(|| badge_of(x, t.app)).flatten();
            Some(DeskIcon { id: id.clone(), name: t.name.clone(), thing: Some(t), glyph: None, badge })
        })
        .collect()
}

/// The compact preview's shape (2:1), an icon's cell, its panel's height and an icon's size.
pub const DESK_ASPECT: f64 = 2.0;
pub const DESK_CELL: (i32, i32) = (54, 46);
pub const DESK_TASK: i32 = 24;
pub const DESK_ICON: i32 = 22;

/// As many of `n` icons as fit a preview `width` wide: how many show, and how many more there are (the last cell says).
pub fn fit(n: usize, width: i32) -> (usize, usize) {
    let h = (width.max(0) as f64 / DESK_ASPECT) as i32;
    let cols = ((width - 12) / DESK_CELL.0).max(1) as usize;
    let rows = ((h - 12 - DESK_TASK) / DESK_CELL.1).max(1) as usize;
    let cap = cols * rows;
    if n > cap { (cap - 1, n - cap + 1) } else { (n, 0) }
}

/// The rows a preview `width` wide has room for (its icons go down each column, then across).
pub fn desk_rows(width: i32) -> usize {
    let h = (width.max(0) as f64 / DESK_ASPECT) as i32;
    ((h - 12 - DESK_TASK) / DESK_CELL.1).max(1) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_model::edit::Library;
    use wad_model::stack::key_boxes;

    fn sample() -> Library {
        serde_json::from_str(include_str!("../../../../../crates/wad-model/tests/data/populated.json")).unwrap()
    }

    fn ws(lib: &Library, id: &str) -> Wadspace {
        lib.wadspace(id).unwrap().clone()
    }

    #[test]
    fn its_state_in_the_spike_s_order() {
        let f = |away: Option<&str>, building, phase, built| Facts {
            away: away.map(str::to_string),
            building,
            phase,
            built: Some(built),
        };
        assert_eq!(status(&f(Some("Desk"), true, None, Built::Built)).tone, Tone::Away);
        assert_eq!(status(&f(None, true, Some(Phase::Ready), Built::Built)).label, "Building…");
        assert_eq!(status(&f(None, false, Some(Phase::Starting), Built::Built)).label, "Opening…");
        assert_eq!(status(&f(None, false, Some(Phase::Ready), Built::Update)).label, "Running");
        assert_eq!(status(&f(None, false, Some(Phase::Idle), Built::Built)).label, "Ready");
        assert_eq!(status(&f(None, false, Some(Phase::Error), Built::Never)).label, "Not built");
        assert_eq!(status(&f(None, false, None, Built::Update)).label, "Rebuilds");
        assert_eq!(status(&Facts::default()).label, "Not built");
    }

    #[test]
    fn a_thing_drags_as_ops_item() {
        let lib = sample();
        let x = ws(&lib, "vanua-academy");
        let gmail = find_variant(&x, "ext:wadbrowser:gmail").unwrap();
        assert_eq!(
            piece(&x.id, &gmail),
            Some(Payload::Piece {
                section: Section::Addons,
                item: "wadbrowser:gmail".into(),
                from: Some("vanua-academy".into())
            })
        );
        let code = find_variant(&x, "app:vscode").unwrap();
        assert_eq!(
            piece(&x.id, &code),
            Some(Payload::Piece { section: Section::Apps, item: "vscode".into(), from: Some("vanua-academy".into()) })
        );
        // Back, as the thing it is: only in its own stack.
        let held = Held::Part { payload: piece(&x.id, &gmail).unwrap() };
        assert_eq!(piece_of(&x, &held).map(|v| v.id), Some("ext:wadbrowser:gmail".into()));
        assert_eq!(piece_of(&ws(&lib, "intelligencequest"), &held), None);
    }

    #[test]
    fn a_thing_of_its_own_on_another_key_s_box_signs_in_with_it() {
        let lib = sample();
        let x = ws(&lib, "vanua-academy");
        let gmail = Held::Part { payload: piece(&x.id, &find_variant(&x, "ext:wadbrowser:gmail").unwrap()).unwrap() };
        let (v, m) = onto_key(&x, &gmail, AppId::Wadbrowser, "vanua-google").expect("it goes there");
        assert_eq!((v.as_str(), m.key.as_deref()), ("ext:wadbrowser:gmail", Some("vanua-google")));
        assert_eq!(m.hint, "Gmail signs in with Google (VanuaAcademy)");
        // Not onto the box it's in, not another app's, not another stack's.
        assert_eq!(onto_key(&x, &gmail, AppId::Wadbrowser, "work-web"), None);
        assert_eq!(onto_key(&x, &gmail, AppId::Vscode, "vanua-claude"), None);
        let other = ws(&lib, "vanua-academy-client-demo");
        assert_eq!(onto_key(&other, &gmail, AppId::Wadbrowser, "vanua-google"), None);
        // A whole section isn't a thing.
        let sec = Held::Part { payload: Payload::Section { ws: x.id.clone(), section: Section::Secrets } };
        assert_eq!(onto_key(&x, &sec, AppId::Wadbrowser, "vanua-google"), None);
        // Every key box names a key the boxes know.
        assert!(key_boxes(&x).iter().all(|b| b.folder.key.is_some()));
    }

    #[test]
    fn the_slot_goes_beside_where_it_came_from() {
        let stacks: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        let piece = Held::Part {
            payload: Payload::Piece { section: Section::Apps, item: "krita".into(), from: Some("a".into()) },
        };
        assert_eq!(slot_after(&stacks, &piece).as_deref(), Some("a"));
        let stack = Held::Space { ws: "b".into(), from: None };
        assert_eq!(slot_after(&stacks, &stack).as_deref(), Some("b"));
        // From the desktop, or from a wadspace not here: after the last.
        let icon = Held::Space { ws: "a".into(), from: Some(crate::dnd::SpaceFrom::Desktop) };
        assert_eq!(slot_after(&stacks, &icon).as_deref(), Some("c"));
        let away = Held::Pieces { section: Section::Addons, items: vec![], from: "zz".into() };
        assert_eq!(slot_after(&stacks, &away).as_deref(), Some("c"));
        assert_eq!(slot_after(&[], &away), None);
    }

    #[test]
    fn two_stacks_side_by_side() {
        // The spike's 636 px board: 300 px stacks.
        assert_eq!(column(636), 300);
        assert_eq!(column(100), MIN_COLUMN);
        assert_eq!(column(2600), MAX_COLUMN);
    }

    #[test]
    fn a_piece_s_menu() {
        let lib = sample();
        let x = ws(&lib, "intelligencequest");
        let code = find_variant(&x, "app:vscode").unwrap();
        let m = chip_menu(&x, &code);
        assert_eq!(m[0], Entry::Heading("VS Code".into()));
        let ids: Vec<&str> = m
            .iter()
            .filter_map(|e| match e {
                Entry::Item(i) => Some(i.id.as_str()),
                _ => None,
            })
            .collect();
        // An app isn't renamed (it's named for itself).
        assert_eq!(ids, ["chip-open", "chip-launch", "wallpaper", "chip-remove"]);
        let launch = find_item(&m, "chip-launch").unwrap();
        assert_eq!(launch.act, Some(Act::Launch(Some("app:vscode".into()))));
        assert!(!launch.checked);
        // Started straight into: checked, and picking it again takes it off.
        let mut y = x.clone();
        y.look.launch = Some("app:vscode".into());
        let launch = find_item(&chip_menu(&y, &code), "chip-launch").cloned().unwrap();
        assert!(launch.checked && launch.act == Some(Act::Launch(None)));
        assert!(launches(&y, &code));
        // A web app: renamed; a tool: no launch, no wallpaper, no rename.
        let gh = find_variant(&x, "ext:wadbrowser:github").unwrap();
        let m = chip_menu(&x, &gh);
        assert_eq!(m[0], Entry::Heading("GitHub · Web app".into()));
        assert!(find_item(&m, "rename").is_some());
        assert_eq!(find_item(&m, "chip-launch").unwrap().act, Some(Act::Launch(Some("web:github".into()))));
        let rust = find_variant(&x, "ext:terminal:rust").unwrap();
        let m = chip_menu(&x, &rust);
        assert!(find_item(&m, "chip-launch").is_none() && find_item(&m, "wallpaper").is_none());
        assert!(find_item(&m, "rename").is_none() && find_item(&m, "chip-remove").is_some());
        // Claude Code signs in, so it opens on a wallpaper of its own.
        let cc = find_variant(&x, "ext:vscode:anthropic.claude-code").unwrap();
        let wp = find_item(&chip_menu(&x, &cc), "wallpaper").cloned().unwrap();
        assert_eq!(wp.submenu.len(), 1 + PICTURES.len());
        assert!(find_item(&wp.submenu, "wp:dunes").is_some() && find_item(&wp.submenu, "wp-none").is_none());
    }

    #[test]
    fn its_desktop_s_icons_badges_and_what_fits() {
        let lib = sample();
        let x = ws(&lib, "vanua-academy");
        let icons = desk_icons(&x);
        assert_eq!(icons.len(), x.look.layout.icons.len());
        let code = icons.iter().find(|i| i.id == "app:vscode").unwrap();
        assert_eq!(code.badge.as_deref(), Some("1"));
        let term = icons.iter().find(|i| i.id == "app:terminal").unwrap();
        assert_eq!(term.badge.as_deref(), Some("1"));
        let p = icons.iter().find(|i| i.id == "project:vanua-academy").unwrap();
        assert_eq!((p.name.as_str(), p.glyph), ("VanuaAcademy", Some("folder")));
        assert_eq!(icons.iter().find(|i| i.id == "web:gmail").unwrap().name, "Gmail");
        let iq = ws(&lib, "intelligencequest");
        assert_eq!(badge_of(&iq, AppId::Wadbrowser).as_deref(), Some("focus"));
        assert_eq!(badge_of(&iq, AppId::Tiled), None);
        // The spike's 300 px stack (a 266 px preview): 4 across, 2 down; 11 icons are 7 and "4 more".
        assert_eq!(fit(11, 266), (7, 4));
        assert_eq!(fit(8, 266), (8, 0));
        assert_eq!(desk_rows(266), 2);
    }
}
