// Every change to a machine's wadspaces, by name: what the spike's
// space.*, builder.*, apps.* and tags.* actions do to the documents (the
// look's ones, now that a wadspace has one look). `apply` makes one change
// to a `Library`, all of it or none of it: on an error the library is as
// it was. It's deterministic: the time and the machine come in `Ctx`, and
// new ids come from the library's own counter (`seq`), as the spike's
// `uid()` did from the world's.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::catalog::{app_name, host_of, is_named_app, url_host};
use crate::check::{self, CheckError, MAX_NAME, MAX_VALUE};
use crate::chips::{ChipKind, chip_of};
use crate::commands::{CommandError, next_command_name, parse_command};
use crate::folders::folders_of;
use crate::games::{disc_id, game_of, is_disc_file};
use crate::groups::identity_of;
use crate::look::{IconPatch, can_launch, resolves};
use crate::names::{copy_name, free_id, new_id, numbered_name, truncate};
use crate::ops::{
    self, AddResult, Section, add_app, add_item, add_named, from_section, merge_section, remove_item, same_image,
    slot_label,
};
use crate::tags::{GENERAL_ID, GENERAL_NAME, TAG_COLORS};
use crate::types::{
    AppId, BrowserApp, BuildCommand, Category, CommandKind, DiscImage, GamePackage, KeyGroup, KeyRef, Project,
    ProjectKind, ProjectSource, Secret, SecretKind, SteamLibrary, Tag, Wadspace, Wallpaper,
};
use crate::variants::{VariantKind, find_variant, key_kind_of, sign_in_of, variants_of};

/// What edits need of a machine: its wadspaces (and the account's others it
/// knows of), the tags, its keys (saved sign-ins), its browser's web apps,
/// its Steam accounts' libraries and the account's Dolphin discs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    #[serde(default)]
    pub wadspaces: Vec<Wadspace>,
    /// Tags made to organize wadspaces by, in order (General, the default, isn't one of them).
    #[serde(default)]
    pub tags: Vec<Tag>,
    /// This machine's saved sign-ins (keys stay on their machine).
    #[serde(default)]
    pub keys: Vec<Secret>,
    /// Web apps the machine's browser keeps ("Create app from this page").
    #[serde(default)]
    pub web_apps: Vec<BrowserApp>,
    /// Steam accounts signed in on this machine (by their key's id): what each owns, and what's downloaded.
    #[serde(default)]
    pub steam: IndexMap<String, SteamLibrary>,
    /// Dolphin's games: discs uploaded to the account's GitHub packages, by id.
    #[serde(default)]
    pub discs: IndexMap<String, DiscImage>,
    /// Steam games pushed to the account's GitHub packages, by game.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub game_packages: IndexMap<String, GamePackage>,
    /// The last number a new id took ('cmd-101', 'undo-102').
    #[serde(default)]
    pub seq: u64,
}

impl Library {
    pub fn wadspace(&self, id: &str) -> Option<&Wadspace> {
        self.wadspaces.iter().find(|x| x.id == id)
    }

    pub fn wadspace_mut(&mut self, id: &str) -> Option<&mut Wadspace> {
        self.wadspaces.iter_mut().find(|x| x.id == id)
    }

    /// Wadspaces not in the Trash, in order.
    pub fn live(&self) -> Vec<&Wadspace> {
        let mut out: Vec<&Wadspace> = self.wadspaces.iter().filter(|x| !x.is_trashed()).collect();
        out.sort_by_key(|x| x.order);
        out
    }

    /// `machine`'s wadspaces not in the Trash, in order.
    pub fn on(&self, machine: &str) -> Vec<&Wadspace> {
        self.live().into_iter().filter(|x| x.machine == machine).collect()
    }

    pub fn key(&self, id: &str) -> Option<&Secret> {
        self.keys.iter().find(|k| k.id == id)
    }
}

/// What an edit is made with: when it is, and which machine it's on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ctx {
    pub now_ms: i64,
    /// This machine's id ('m-surface'): where new wadspaces go.
    pub machine: String,
}

impl Ctx {
    pub fn new(now_ms: i64, machine: impl Into<String>) -> Self {
        Ctx { now_ms, machine: machine.into() }
    }
}

/// A wadspace as it was before something was taken out of it (its toast's Undo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoSnapshot {
    /// The Undo's own id ('undo-104'), for its toast.
    pub id: String,
    pub wadspace: Wadspace,
}

/// What an edit did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// The wadspaces whose documents changed (a new one, one gone, ones untagged): to save.
    pub changed: Vec<String>,
    /// The id of what it made: a wadspace, a tag, a key, a web app, a command, or a variant ('ext:dolphin:mario-kart', 'key:app:steam').
    pub created: Option<String>,
    /// How to put it back, if it took something out.
    pub undo: Option<UndoSnapshot>,
    /// What to tell the person (the spike's toast).
    pub toast: Option<String>,
    /// What a changed wadspace's image is made of changed: built, it's out of date (`BuildState::edited`).
    pub needs_build: bool,
}

/// Why an edit wasn't made (the library is as it was).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("There's no {what} {id:?}")]
    NotFound { what: &'static str, id: String },
    /// It can't be done, in the words to say why ("It’s already in it").
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error(transparent)]
    Check(#[from] CheckError),
}

fn not_found(what: &'static str, id: &str) -> EditError {
    EditError::NotFound { what, id: id.to_string() }
}

fn refused(s: impl Into<String>) -> EditError {
    EditError::Refused(s.into())
}

/// A site as a web key lists it: its host, lowercased, without "www." ("https://www.GitHub.com/x" → "github.com").
fn web_site(site: &str) -> Option<String> {
    host_of(site)
}

/// A file or folder kept in a wadspace: a folder is mounted, a file copied in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRef {
    /// The machine it's on.
    pub machine: String,
    /// Its path there ('~/Desktop/Thesis').
    pub path: String,
    /// Its name ('Thesis'): what it's called inside the wadspace.
    pub name: String,
    #[serde(default)]
    pub folder: bool,
}

/// What a Builder drag carries: a whole section of a wadspace, one thing of a section, or a key's group (`app`'s things in `ws` signed in with `key`; none: the ones with no key).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Payload {
    Section {
        ws: String,
        section: Section,
    },
    Piece {
        section: Section,
        item: String,
        /// Where it comes from (another wadspace, for its own web apps and its look).
        #[serde(default)]
        from: Option<String>,
    },
    Folder {
        ws: String,
        app: AppId,
        #[serde(default)]
        key: Option<String>,
    },
}

/// A new variant of an app: a catalog add-on ('vscode:<extension>',
/// 'terminal:<tool>', 'wadbrowser:<web app>', 'wadbrowser:focus'), a site of
/// your own, a named one (a game), a saved key, or a new key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NewVariant {
    Addon(String),
    Site { name: String, url: String },
    Named(String),
    Key(String),
    NewKey { name: String, kind: SecretKind },
}

/// One edit. Each is one of the spike's actions (its name is the variant's in JSON: `{"op": "space.remove", ...}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum Op {
    // ---------------------------------------------------------- wadspaces
    /// A new, empty wadspace with no background, on this machine or `machine` (named for its tab: "New space", "New gamespace"; else "New wadspace"). Created: its id.
    #[serde(rename = "space.create")]
    Create {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        category: Option<Category>,
        #[serde(default)]
        machine: Option<String>,
    },
    /// A Builder section, one thing or a key's group dropped on empty desktop: a new wadspace of only that, Untitled. Created: its id.
    #[serde(rename = "builder.fresh")]
    Fresh { payload: Payload },
    /// A copy beside it, on this machine or `machine`. Created: its id.
    #[serde(rename = "space.duplicate")]
    Duplicate {
        ws: String,
        #[serde(default)]
        machine: Option<String>,
    },
    /// Another machine's wadspace (`doc`, as machine `from` sent it), copied
    /// here as this machine's: a new id, never built here, its name kept, its
    /// projects as they are (they still name `from`'s folders, which come
    /// over Taildrive). A tag this library doesn't have, and keys this
    /// machine doesn't have, are left off (those slots sign in with nothing).
    /// A document that doesn't check out is refused. Created: its id.
    #[serde(rename = "space.import")]
    Import { doc: Box<Wadspace>, from: String },
    /// Closing the Builder on a wadspace with nothing in it lets it go.
    #[serde(rename = "builder.discardIfEmpty")]
    DiscardIfEmpty { ws: String },
    /// Into the Trash (stopping it is the machine's).
    #[serde(rename = "ws.trash")]
    Trash { ws: String },
    /// Back out of the Trash.
    #[serde(rename = "ws.restore")]
    Restore { ws: String },
    #[serde(rename = "builder.rename")]
    Rename { ws: String, name: String },
    /// Which launcher tab it's under.
    #[serde(rename = "builder.category")]
    SetCategory { ws: String, category: Category },
    /// Let agents in it message the desktop and ask for sandboxes (the wadspaces MCP server).
    #[serde(rename = "builder.agents")]
    SetAgents { ws: String, on: bool },

    // ---------------------------------------------------------- what's in it
    /// One thing into a section (`ops::add_item`'s items), from `from` if it's another's.
    #[serde(rename = "builder.add")]
    Add {
        ws: String,
        section: Section,
        item: String,
        #[serde(default)]
        from: Option<String>,
    },
    /// One thing out of a section (`ops::remove_item`'s items).
    #[serde(rename = "builder.remove")]
    RemoveItem { ws: String, section: Section, item: String },
    /// A section or one thing dropped onto a wadspace: merged in.
    #[serde(rename = "builder.dropOn")]
    DropOn { ws: String, payload: Payload },
    /// A key's group copied from one wadspace into another: the app, its things, and the key they sign in with.
    #[serde(rename = "builder.copyFolder")]
    CopyFolder {
        from: String,
        to: String,
        app: AppId,
        #[serde(default)]
        key: Option<String>,
    },
    /// A chip taken out ('app:krita', 'ext:wadbrowser:gmail', 'key:<slot>', 'project:<id>'): an app goes with its add-ons and sign-ins. With an Undo.
    #[serde(rename = "space.remove")]
    Remove { ws: String, chip: String },
    /// A chip copied into another wadspace (the first keeps it).
    #[serde(rename = "space.copyChip")]
    CopyChip { from: String, to: String, chip: String },
    #[serde(rename = "apps.add")]
    AddApp { ws: String, app: AppId },
    /// An app out, with its variants and sign-ins.
    #[serde(rename = "apps.remove")]
    RemoveApp { ws: String, app: AppId },
    /// A new variant of an app. Created: its variant's id.
    #[serde(rename = "apps.newVariant")]
    NewVariant { ws: String, app: AppId, new: NewVariant },
    /// A variant out (a game's isn't in the image: no build for it).
    #[serde(rename = "apps.removeVariant")]
    RemoveVariant { ws: String, variant: String },
    /// A web app's or a named variant's name.
    #[serde(rename = "apps.renameVariant")]
    RenameVariant { ws: String, variant: String, name: String },
    /// Every add-on of one app's out (the Builder's Add-ons box ×). With an Undo.
    #[serde(rename = "space.removeAddons")]
    RemoveAddons { ws: String, app: AppId },
    /// A web app of your own (any site) as an add-on. Created: its variant's id.
    #[serde(rename = "builder.addWebApp")]
    AddWebApp { ws: String, name: String, url: String },

    // ---------------------------------------------------------- sign-ins
    /// A new saved sign-in (a key), empty until something signs in with it. Created: its id.
    #[serde(rename = "secrets.create")]
    NewKey { name: String, kind: SecretKind },
    /// What an app or one of its variants signs in with: a key of the kind it takes. None: an app has no sign-in; a variant signs in as its app does (Claude Code: none).
    #[serde(rename = "apps.signIn")]
    SignIn {
        ws: String,
        variant: String,
        #[serde(default)]
        key: Option<String>,
    },
    /// A sign-in chip's key swapped for another of its kind (`slot`: 'app:vscode', 'ext:…', 'group:<id>', 'group').
    #[serde(rename = "space.swapKey")]
    SwapKey { ws: String, slot: String, key: String },
    /// A key taken off what signs in with it (the Builder's Secrets box ×): `app`'s things with `key` sign in as the app does, or with nothing. With an Undo.
    #[serde(rename = "space.dropKey")]
    DropKey { ws: String, app: AppId, key: String },
    /// A site signs in with a web key (the browser's "Saved sign-ins"): `existing`'s, which gets `site` too, else a new web key named `name` (none: the site's name). Created: the key's id.
    #[serde(rename = "secrets.saveWeb")]
    SaveWebKey {
        name: String,
        site: String,
        #[serde(default)]
        existing: Option<String>,
    },
    /// A web key stops signing in to `site` (the key itself stays).
    #[serde(rename = "secrets.removeSite")]
    RemoveSite { key: String, site: String },
    /// Into another sign-in group: it drops the key it was grouped by and signs in with `key` (none: with nothing). An app the key needs comes too.
    #[serde(rename = "space.joinGroup")]
    JoinGroup {
        ws: String,
        #[serde(default)]
        key: Option<String>,
    },

    // ---------------------------------------------------------- its commands
    /// A step of its own: RUN a command, or COPY `from to`, exactly as typed ("RUN …" or "COPY …" at its start says which). Created: its id.
    #[serde(rename = "space.addCommand")]
    AddCommand { ws: String, kind: CommandKind, text: String },
    #[serde(rename = "space.removeCommand")]
    RemoveCommand { ws: String, id: String },

    // ---------------------------------------------------------- its look
    /// Its background.
    #[serde(rename = "space.look")]
    SetWallpaper { ws: String, wallpaper: Wallpaper },
    /// An app's wallpaper ('app:<id>'), or one of its variants': what it opens on. None takes it off.
    #[serde(rename = "apps.wallpaper")]
    SetAppWallpaper {
        ws: String,
        variant: String,
        #[serde(default)]
        wallpaper: Option<Wallpaper>,
    },
    /// Its icon's look (a part of it; none: back to its picture, the middle of it).
    #[serde(rename = "space.icon")]
    SetIcon {
        ws: String,
        #[serde(default)]
        patch: Option<IconPatch>,
    },
    /// What it starts straight into ('variant:hades', 'app:krita', 'web:gmail'), or (none) its desktop.
    #[serde(rename = "space.launch")]
    SetLaunch {
        ws: String,
        #[serde(default)]
        launch: Option<String>,
    },
    /// Whether something opens with it ('app:vscode', 'web:github', 'project:iq').
    #[serde(rename = "variant.autostart")]
    SetAutostart { ws: String, key: String, on: bool },
    /// Its desktop's icons, in this order.
    #[serde(rename = "variant.order")]
    SetIconOrder { ws: String, icons: Vec<String> },
    /// Files kept in it (dropped on it): a folder mounted, a file copied in; and maybe a picture for its background.
    #[serde(rename = "space.keepFiles")]
    KeepFiles {
        ws: String,
        files: Vec<FileRef>,
        #[serde(default)]
        wallpaper: Option<Wallpaper>,
    },
    #[serde(rename = "variant.removeProject")]
    RemoveProject { ws: String, project: String },

    // ---------------------------------------------------------- tags
    /// A new tag (an empty group on the desktop), and `ws` into it if given. Created: its id.
    #[serde(rename = "tags.create")]
    CreateTag {
        name: String,
        #[serde(default)]
        ws: Option<String>,
    },
    #[serde(rename = "tags.rename")]
    RenameTag { tag: String, name: String },
    /// A tag gone: its wadspaces are General's again.
    #[serde(rename = "tags.remove")]
    RemoveTag { tag: String },
    /// Its tag (none, or 'general': General).
    #[serde(rename = "space.tag")]
    SetTag {
        ws: String,
        #[serde(default)]
        tag: Option<String>,
    },

    // ---------------------------------------------------------- games
    /// A Steam game into a wadspace: Steam comes with it, signed in with the account that owns it (unless it has one), and it plays it unless it plays something else.
    #[serde(rename = "space.addGame")]
    AddGame { ws: String, account: String, game: String },
    /// A gamespace for one Steam game: named for it, straight into it. Created: its id.
    #[serde(rename = "space.fromGame")]
    FromGame { account: String, game: String },
    /// A wadspace for one app (Open with an app nothing has yet), named for it. Created: its id.
    #[serde(rename = "space.forApp")]
    ForApp { app: AppId },
    /// A disc (an .iso or .rvz file) already in the account's packages, as one of its Dolphin games (one that isn't: upload it first, then this). Created: its variant's id.
    /// A sandbox's wadspace: there's no way back (wadd's agents.rs).
    #[serde(rename = "builder.seal")]
    Seal { ws: String },
    /// What a Steam account has, as its client's files say (wadd's steam.rs).
    #[serde(rename = "steam.library")]
    SteamLibrary { key: String, library: SteamLibrary },
    /// A game pushed to the account's packages (wadd's steam.rs).
    #[serde(rename = "steam.package")]
    SteamPackage { package: GamePackage },
    /// A disc on this machine, new or pushed (wadd's discs.rs).
    #[serde(rename = "discs.keep")]
    KeepDisc { disc: DiscImage },
    #[serde(rename = "space.addDisc")]
    AddDisc { ws: String, file: String },

    // ---------------------------------------------------------- the machine's browser
    /// A web app of your own for the machine's browser, ready to drop on wadspaces. Created: its id.
    #[serde(rename = "assets.webApp")]
    NewWebApp { name: String, url: String },
    /// A web app of your own taken off the machine's browser (its new tab, Settings). Wadspaces it was dropped on keep their own copies.
    #[serde(rename = "assets.removeWebApp")]
    RemoveWebApp { id: String },
}

/// Make one edit. On an error nothing changed.
pub fn apply(lib: &mut Library, op: Op, ctx: &Ctx) -> Result<Outcome, EditError> {
    let before = lib.clone();
    let mut tx = Tx { lib, ctx, out: Outcome::default() };
    match tx.run(op).and_then(|()| tx.finish(&before)) {
        Ok(()) => Ok(tx.out),
        Err(e) => {
            *tx.lib = before;
            Err(e)
        }
    }
}

/// Put a wadspace back as it was (an Undo). Its document is the snapshot's, its time too.
pub fn undo(lib: &mut Library, snapshot: &UndoSnapshot) -> Result<Outcome, EditError> {
    let w = &snapshot.wadspace;
    check::wadspace(w)?;
    let x = lib.wadspace_mut(&w.id).ok_or_else(|| not_found("wadspace", &w.id))?;
    let needs_build = !same_image(x, w);
    *x = w.clone();
    Ok(Outcome { changed: vec![w.id.clone()], needs_build, ..Outcome::default() })
}

struct Tx<'a> {
    lib: &'a mut Library,
    ctx: &'a Ctx,
    out: Outcome,
}

impl Tx<'_> {
    /// A new id, unique in this library ("cmd-12").
    fn uid(&mut self, prefix: &str) -> String {
        self.lib.seq += 1;
        format!("{prefix}-{}", self.lib.seq)
    }

    fn get(&self, id: &str) -> Result<&Wadspace, EditError> {
        self.lib.wadspace(id).ok_or_else(|| not_found("wadspace", id))
    }

    fn ws(&mut self, id: &str) -> Result<&mut Wadspace, EditError> {
        self.lib.wadspace_mut(id).ok_or_else(|| not_found("wadspace", id))
    }

    /// Mark it changed (now).
    fn touched(&mut self, id: &str) {
        let now = self.ctx.now_ms;
        if let Some(x) = self.lib.wadspace_mut(id) {
            x.updated_at = now;
        }
        if !self.out.changed.iter().any(|c| c == id) {
            self.out.changed.push(id.to_string());
        }
    }

    fn keep_undo(&mut self, before: Wadspace, toast: String) {
        let id = self.uid("undo");
        self.out.undo = Some(UndoSnapshot { id, wadspace: before });
        self.out.toast = Some(toast);
    }

    /// What it changed is a document still: and does it need a build?
    fn finish(&mut self, before: &Library) -> Result<(), EditError> {
        for id in &self.out.changed {
            let Some(x) = self.lib.wadspace(id) else { continue };
            check::wadspace(x)?;
            let was = before.wadspace(id);
            if was.is_none_or(|w| !same_image(w, x)) {
                self.out.needs_build = true;
            }
        }
        Ok(())
    }

    fn next_order(&self) -> i64 {
        self.lib.wadspaces.iter().map(|x| x.order).max().unwrap_or(0).max(0) + 1
    }

    /// One thing into a section of `id` (the keys are the library's).
    fn add(&mut self, id: &str, s: Section, item: &str, from: Option<&Wadspace>) -> Result<AddResult, EditError> {
        let lib = &mut *self.lib;
        let x = lib.wadspaces.iter_mut().find(|x| x.id == id).ok_or_else(|| not_found("wadspace", id))?;
        Ok(add_item(x, s, item, &lib.keys, from))
    }

    /// One thing into a section of `id`, or why not.
    fn must_add(&mut self, id: &str, s: Section, item: &str, from: Option<&Wadspace>) -> Result<String, EditError> {
        let r = self.add(id, s, item, from)?;
        if !r.ok {
            return Err(refused(r.said));
        }
        self.touched(id);
        Ok(r.said)
    }

    fn remove(&mut self, id: &str, s: Section, item: &str) -> Result<bool, EditError> {
        let done = remove_item(self.ws(id)?, s, item);
        if done {
            self.touched(id);
        }
        Ok(done)
    }

    fn run(&mut self, op: Op) -> Result<(), EditError> {
        match op {
            Op::Create { name, category, machine } => {
                self.create(name.as_deref(), category, machine.as_deref()).map(drop)
            }
            Op::Fresh { payload } => self.fresh(payload),
            Op::Duplicate { ws, machine } => self.duplicate(&ws, machine.as_deref()),
            Op::Import { doc, from } => self.import(*doc, &from),
            Op::DiscardIfEmpty { ws } => {
                let x = self.get(&ws)?;
                if x.apps.is_empty() && x.exts.is_empty() && x.look.projects.is_empty() && x.commands.is_empty() {
                    self.lib.wadspaces.retain(|y| y.id != ws);
                    self.out.changed.push(ws);
                }
                Ok(())
            }
            Op::Trash { ws } => {
                let now = self.ctx.now_ms;
                let before = self.get(&ws)?.clone();
                let x = self.ws(&ws)?;
                if x.deleted_at.is_none() {
                    x.deleted_at = Some(now);
                    self.touched(&ws);
                    // The spike's toast, with its Undo (the wadspace as it was: out of the Trash).
                    self.keep_undo(before, "Moved to the Trash".to_string());
                }
                Ok(())
            }
            Op::Restore { ws } => {
                let x = self.ws(&ws)?;
                if x.deleted_at.take().is_some() {
                    self.touched(&ws);
                }
                Ok(())
            }
            Op::Rename { ws, name } => {
                let name = check::name(&name)?;
                self.ws(&ws)?.name = name;
                self.touched(&ws);
                Ok(())
            }
            Op::SetCategory { ws, category } => {
                self.ws(&ws)?.category = Some(category);
                self.touched(&ws);
                Ok(())
            }
            Op::SetAgents { ws, on } => {
                self.ws(&ws)?.agents = on;
                self.touched(&ws);
                Ok(())
            }

            Op::Add { ws, section, item, from } => {
                let from = from.map(|f| self.get(&f).cloned()).transpose()?;
                self.must_add(&ws, section, &item, from.as_ref()).map(drop)
            }
            Op::RemoveItem { ws, section, item } => self.remove(&ws, section, &item).map(drop),
            Op::DropOn { ws, payload } => self.drop_on(&ws, payload),
            Op::CopyFolder { from, to, app, key } => self.copy_folder(&from, &to, app, key.as_deref()),
            Op::Remove { ws, chip } => self.remove_chip(&ws, &chip),
            Op::CopyChip { from, to, chip } => self.copy_chip(&from, &to, &chip),
            Op::AddApp { ws, app } => self.must_add(&ws, Section::Apps, app.as_str(), None).map(drop),
            Op::RemoveApp { ws, app } => self.remove(&ws, Section::Apps, app.as_str()).map(drop),
            Op::NewVariant { ws, app, new } => {
                let v = self.new_variant(&ws, app, new)?;
                self.out.created = v;
                Ok(())
            }
            Op::RemoveVariant { ws, variant } => self.remove_variant(&ws, &variant),
            Op::RenameVariant { ws, variant, name } => {
                let v = find_variant(self.get(&ws)?, &variant).ok_or_else(|| not_found("variant", &variant))?;
                let e = v.ext.ok_or_else(|| refused(format!("{} is named for its app", v.name)))?;
                let name = check::name(&name)?;
                if let Some(y) = self.ws(&ws)?.exts.iter_mut().find(|y| y.app == e.app && y.id == e.id) {
                    y.name = Some(name);
                }
                self.touched(&ws);
                Ok(())
            }
            Op::RemoveAddons { ws, app } => {
                let x = self.get(&ws)?;
                let ids: Vec<String> =
                    x.exts.iter().filter(|e| e.app == app).map(|e| format!("ext:{app}:{}", e.id)).collect();
                if ids.is_empty() {
                    return Ok(());
                }
                let before = x.clone();
                for id in &ids {
                    self.remove_variant(&ws, id)?;
                }
                self.keep_undo(before.clone(), format!("Took {}’s add-ons out of {}", app_name(app), before.name));
                Ok(())
            }
            Op::AddWebApp { ws, name, url } => {
                let id = self.add_web_app(&ws, &name, &url)?;
                self.out.created = Some(format!("ext:wadbrowser:{id}"));
                Ok(())
            }

            Op::NewKey { name, kind } => self.new_key(&name, kind).map(drop),
            Op::SignIn { ws, variant, key } => self.sign_in(&ws, &variant, key.as_deref()),
            Op::SwapKey { ws, slot, key } => self.swap_key(&ws, &slot, &key),
            Op::DropKey { ws, app, key } => self.drop_key(&ws, app, &key),
            Op::JoinGroup { ws, key } => self.join_group(&ws, key.as_deref()),
            Op::SaveWebKey { name, site, existing } => self.save_web_key(&name, &site, existing.as_deref()),
            Op::RemoveSite { key, site } => {
                let site = web_site(&site).unwrap_or_else(|| site.trim().to_ascii_lowercase());
                let now = self.ctx.now_ms;
                let k = self.lib.keys.iter_mut().find(|k| k.id == key).ok_or_else(|| not_found("key", &key))?;
                if k.sites.contains(&site) {
                    k.sites.retain(|s| *s != site);
                    k.changed_at = Some(now);
                }
                Ok(())
            }

            Op::AddCommand { ws, kind, text } => {
                let (kind, text) = parse_command(kind, &text)?;
                let id = self.uid("cmd");
                let x = self.ws(&ws)?;
                let name = next_command_name(&x.commands, kind);
                x.commands.push(BuildCommand { id: id.clone(), name, kind, text });
                self.touched(&ws);
                self.out.created = Some(id);
                Ok(())
            }
            Op::RemoveCommand { ws, id } => {
                let x = self.ws(&ws)?;
                if !x.commands.iter().any(|c| c.id == id) {
                    return Err(not_found("command", &id));
                }
                x.commands.retain(|c| c.id != id);
                self.touched(&ws);
                Ok(())
            }

            Op::SetWallpaper { ws, wallpaper } => {
                check::wallpaper(&wallpaper)?;
                self.ws(&ws)?.look.wallpaper = wallpaper;
                self.touched(&ws);
                Ok(())
            }
            Op::SetAppWallpaper { ws, variant, wallpaper } => self.app_wallpaper(&ws, &variant, wallpaper),
            Op::SetIcon { ws, patch } => {
                if let Some(Some(Some(w))) = patch.as_ref().map(|p| &p.picture) {
                    check::wallpaper(w)?;
                }
                let x = self.ws(&ws)?;
                x.look.icon = patch.map(|p| p.apply(x.look.icon.as_ref()));
                self.touched(&ws);
                Ok(())
            }
            Op::SetLaunch { ws, launch } => {
                let x = self.ws(&ws)?;
                if let Some(k) = &launch
                    && !can_launch(x, k)
                {
                    return Err(refused(format!("{} can’t start straight into that", x.name)));
                }
                x.look.launch = launch;
                self.touched(&ws);
                Ok(())
            }
            Op::SetAutostart { ws, key, on } => {
                let x = self.ws(&ws)?;
                if on {
                    if !resolves(x, &key) {
                        return Err(refused("That isn’t in it"));
                    }
                    if !x.look.autostart.contains(&key) {
                        x.look.autostart.push(key);
                    }
                } else {
                    x.look.autostart.retain(|k| *k != key);
                }
                self.touched(&ws);
                Ok(())
            }
            Op::SetIconOrder { ws, icons } => {
                let x = self.ws(&ws)?;
                let mut order: Vec<String> = vec![];
                for i in icons {
                    if !resolves(x, &i) {
                        return Err(refused(format!("{i} isn’t in it")));
                    }
                    if !order.contains(&i) {
                        order.push(i);
                    }
                }
                x.look.layout.icons = order;
                self.touched(&ws);
                Ok(())
            }
            Op::KeepFiles { ws, files, wallpaper } => self.keep_files(&ws, files, wallpaper),
            Op::RemoveProject { ws, project } => {
                if !self.get(&ws)?.look.projects.iter().any(|p| p.id == project) {
                    return Err(not_found("project", &project));
                }
                self.remove(&ws, Section::Look, &format!("project:{project}")).map(drop)
            }

            Op::CreateTag { name, ws } => self.create_tag(&name, ws.as_deref()),
            Op::RenameTag { tag, name } => {
                let name = check::name(&name)?;
                let t = self.lib.tags.iter_mut().find(|t| t.id == tag).ok_or_else(|| not_found("tag", &tag))?;
                t.name = name;
                Ok(())
            }
            Op::RemoveTag { tag } => {
                if !self.lib.tags.iter().any(|t| t.id == tag) {
                    return Err(not_found("tag", &tag));
                }
                self.lib.tags.retain(|t| t.id != tag);
                let ids: Vec<String> = self
                    .lib
                    .wadspaces
                    .iter()
                    .filter(|x| x.tag.as_deref() == Some(tag.as_str()))
                    .map(|x| x.id.clone())
                    .collect();
                for id in ids {
                    self.ws(&id)?.tag = None;
                    self.touched(&id);
                }
                Ok(())
            }
            Op::SetTag { ws, tag } => self.set_tag(&ws, tag.as_deref()),

            Op::AddGame { ws, account, game } => self.add_game(&ws, &account, &game),
            Op::FromGame { account, game } => {
                let g = game_of(&game, None);
                let id = self.create(Some(&g.name), Some(Category::Gaming), None)?;
                self.add_game(&id, &account, &game)
            }
            Op::ForApp { app } => {
                let category =
                    if matches!(app, AppId::Steam | AppId::Dolphin) { Category::Gaming } else { Category::Work };
                let id = self.create(Some(app_name(app)), Some(category), None)?;
                self.must_add(&id, Section::Apps, app.as_str(), None).map(drop)
            }
            Op::Seal { ws } => {
                self.ws(&ws)?.sealed = true;
                Ok(())
            }
            Op::SteamLibrary { key, library } => {
                self.lib.steam.insert(key, library);
                Ok(())
            }
            Op::SteamPackage { package } => {
                self.lib.game_packages.insert(package.game.clone(), package);
                Ok(())
            }
            Op::KeepDisc { disc } => {
                self.lib.discs.insert(disc.id.clone(), disc);
                Ok(())
            }

            Op::AddDisc { ws, file } => {
                self.get(&ws)?;
                if !is_disc_file(&file) {
                    return Err(refused("A disc is an .iso or an .rvz file"));
                }
                let id = disc_id(&file);
                let disc = self.lib.discs.get(&id).cloned().ok_or_else(|| not_found("disc", &id))?;
                let x = self.get(&ws)?;
                if x.exts
                    .iter()
                    .any(|e| e.app == AppId::Dolphin && (e.id == disc.id || e.name.as_deref() == Some(&disc.name)))
                {
                    return Err(refused(format!("{} is already in {}", disc.name, x.name)));
                }
                let v = self.new_variant(&ws, AppId::Dolphin, NewVariant::Named(disc.name))?;
                self.out.created = v;
                Ok(())
            }

            Op::NewWebApp { name, url } => {
                let host = url_host(&url).ok_or_else(|| refused("That isn’t a web address"))?;
                let url = url.trim();
                if url.len() > MAX_VALUE {
                    return Err(refused("That web address is too long"));
                }
                let lower = url.to_ascii_lowercase();
                let url = if lower.starts_with("https://") || lower.starts_with("http://") {
                    url.to_string()
                } else {
                    format!("https://{url}")
                };
                let shown = if name.trim().is_empty() { truncate(&host, MAX_NAME) } else { check::name(&name)? };
                let id = self.uid("webapp");
                self.lib.web_apps.push(BrowserApp {
                    id: id.clone(),
                    name: shown.clone(),
                    url,
                    profile: "default".to_string(),
                });
                self.out.toast = Some(format!("{shown} is an app"));
                self.out.created = Some(id);
                Ok(())
            }
            Op::RemoveWebApp { id } => {
                let i = self.lib.web_apps.iter().position(|a| a.id == id).ok_or_else(|| not_found("web app", &id))?;
                let gone = self.lib.web_apps.remove(i);
                self.out.toast = Some(format!("Removed {}", gone.name));
                Ok(())
            }
        }
    }

    // ---------------------------------------------------------- wadspaces

    fn create(
        &mut self,
        name: Option<&str>,
        category: Option<Category>,
        machine: Option<&str>,
    ) -> Result<String, EditError> {
        let default = match category {
            Some(Category::Gaming) => "New gamespace",
            Some(Category::Work) => "New space",
            None => "New wadspace",
        };
        let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => check::name(n)?,
            None => default.to_string(),
        };
        let on = match machine {
            Some(m) => {
                check::id("machine", m)?;
                m.to_string()
            }
            None => self.ctx.machine.clone(),
        };
        let live: Vec<&str> = self.lib.wadspaces.iter().filter(|x| !x.is_trashed()).map(|x| x.name.as_str()).collect();
        let name = copy_name(&live, &name);
        let ids: Vec<&str> = self.lib.wadspaces.iter().map(|x| x.id.as_str()).collect();
        let id = free_id(&ids, &name);
        let mut x = ops::blank(&id, &name, &on, self.ctx.now_ms, self.next_order());
        x.category = category;
        self.lib.wadspaces.push(x);
        self.touched(&id);
        self.out.created = Some(id.clone());
        Ok(id)
    }

    fn fresh(&mut self, payload: Payload) -> Result<(), EditError> {
        let id = self.create(Some("Untitled"), None, None)?;
        match payload {
            Payload::Section { ws, section } => {
                let from = self.get(&ws)?.clone();
                from_section(self.ws(&id)?, &from, section);
            }
            Payload::Folder { ws, app, key } => self.copy_folder(&ws, &id, app, key.as_deref())?,
            Payload::Piece { section, item, from } => {
                let from = from.map(|f| self.get(&f).cloned()).transpose()?;
                let r = self.add(&id, section, &item, from.as_ref())?;
                if !r.ok {
                    return Err(refused(format!("Couldn’t make a wadspace of that: {}", r.said)));
                }
            }
        }
        let taken: Vec<String> =
            self.lib.wadspaces.iter().filter(|y| !y.is_trashed() && y.id != id).map(|y| y.name.clone()).collect();
        let name = numbered_name(&taken, "Untitled");
        self.ws(&id)?.name = name.clone();
        self.touched(&id);
        self.out.toast = Some(format!("New wadspace “{name}”"));
        self.out.created = Some(id);
        Ok(())
    }

    fn duplicate(&mut self, ws: &str, machine: Option<&str>) -> Result<(), EditError> {
        let mut copy = self.get(ws)?.clone();
        let on = match machine {
            Some(m) => {
                check::id("machine", m)?;
                m.to_string()
            }
            None => self.ctx.machine.clone(),
        };
        let ids: Vec<&str> = self.lib.wadspaces.iter().map(|x| x.id.as_str()).collect();
        copy.id = free_id(&ids, &copy.name);
        let live: Vec<&str> = self.lib.wadspaces.iter().filter(|x| !x.is_trashed()).map(|x| x.name.as_str()).collect();
        copy.name = copy_name(&live, &copy.name);
        copy.machine = on;
        copy.order = self.next_order();
        copy.created_at = self.ctx.now_ms;
        copy.updated_at = self.ctx.now_ms;
        copy.deleted_at = None;
        copy.starter = false;
        copy.last_launched = None;
        let id = copy.id.clone();
        self.lib.wadspaces.push(copy);
        self.touched(&id);
        self.out.created = Some(id);
        Ok(())
    }

    fn import(&mut self, mut doc: Wadspace, from: &str) -> Result<(), EditError> {
        check::id("machine", from)?;
        check::wadspace(&doc)?;
        let ids: Vec<&str> = self.lib.wadspaces.iter().map(|x| x.id.as_str()).collect();
        doc.id = free_id(&ids, &doc.name);
        doc.machine = self.ctx.machine.clone();
        doc.order = self.next_order();
        doc.created_at = self.ctx.now_ms;
        doc.deleted_at = None;
        doc.starter = false;
        doc.last_launched = None;
        if doc.tag.as_ref().is_some_and(|t| !self.lib.tags.iter().any(|y| y.id == *t)) {
            doc.tag = None;
        }
        // Keys stay on their machine: one this machine has (of that kind) is
        // named as it is here; any other is left off.
        let keys = &self.lib.keys;
        let here = |k: &KeyRef| keys.iter().find(|s| s.id == k.id && s.kind == k.kind).map(Secret::key_ref);
        doc.secrets.by_slot = std::mem::take(&mut doc.secrets.by_slot)
            .into_iter()
            .filter_map(|(slot, k)| here(&k).map(|k| (slot, k)))
            .collect();
        doc.secrets.groups = std::mem::take(&mut doc.secrets.groups)
            .into_iter()
            .filter_map(|g| here(&g.key).map(|key| KeyGroup { key, ..g }))
            .collect();
        let (id, name) = (doc.id.clone(), doc.name.clone());
        self.lib.wadspaces.push(doc);
        self.touched(&id);
        self.out.created = Some(id);
        self.out.toast = Some(format!("{name} is on this machine"));
        Ok(())
    }

    // ---------------------------------------------------------- what's in it

    fn drop_on(&mut self, ws: &str, payload: Payload) -> Result<(), EditError> {
        match payload {
            Payload::Section { ws: from, section } => {
                let src = self.get(&from)?.clone();
                let x = self.ws(ws)?;
                let said = merge_section(x, &src, section);
                let name = x.name.clone();
                self.touched(ws);
                self.out.toast = Some(format!("{name}: {said} from {}", src.name));
                Ok(())
            }
            Payload::Piece { section, item, from } => {
                let from = from.map(|f| self.get(&f).cloned()).transpose()?;
                self.must_add(ws, section, &item, from.as_ref()).map(drop)
            }
            Payload::Folder { ws: from, app, key } => self.copy_folder(&from, ws, app, key.as_deref()),
        }
    }

    fn copy_folder(&mut self, from: &str, to: &str, app: AppId, key: Option<&str>) -> Result<(), EditError> {
        let src = self.get(from)?.clone();
        self.get(to)?;
        if from == to {
            return Err(refused("Nothing to copy"));
        }
        let f = folders_of(&src, app)
            .into_iter()
            .find(|f| f.key.as_ref().map(|k| k.id.as_str()) == key)
            .ok_or_else(|| refused("That group is gone"))?;
        self.add(to, Section::Apps, app.as_str(), None)?;
        for v in &f.items {
            if let Some(e) = &v.ext {
                self.add(to, Section::Addons, &format!("{}:{}", v.app, e.id), Some(&src))?;
            }
        }
        // The key, if this machine has it (keys stay on their machine): the app's own as its own; another's, as theirs.
        if let Some(key) = key.filter(|k| self.lib.key(k).is_some()) {
            if f.home {
                self.sign_in(to, &format!("app:{app}"), Some(key))?;
            } else {
                for v in &f.items {
                    self.sign_in(to, &v.id, Some(key))?;
                }
            }
        }
        self.touched(to);
        Ok(())
    }

    fn remove_chip(&mut self, ws: &str, chip: &str) -> Result<(), EditError> {
        let x = self.get(ws)?;
        let c = chip_of(x, chip).ok_or_else(|| not_found("chip", chip))?;
        let before = x.clone();
        match c.kind {
            ChipKind::App => {
                let app = c.app.ok_or_else(|| not_found("app", chip))?;
                self.remove(ws, Section::Apps, app.as_str())?;
            }
            ChipKind::Key => {
                self.remove(ws, Section::Secrets, &chip["key:".len()..])?;
            }
            ChipKind::Project => {
                self.remove(ws, Section::Look, chip)?;
            }
            ChipKind::Web | ChipKind::Addon | ChipKind::Game => self.remove_variant(ws, chip)?,
        }
        let toast = format!("Took {} out of {}", c.name, before.name);
        self.keep_undo(before, toast);
        Ok(())
    }

    fn copy_chip(&mut self, from: &str, to: &str, chip: &str) -> Result<(), EditError> {
        let src = self.get(from)?.clone();
        self.get(to)?;
        let c = chip_of(&src, chip).filter(|_| from != to).ok_or_else(|| refused("Nothing to copy"))?;
        match (c.kind, c.app, c.variant, c.project) {
            (ChipKind::App, Some(app), _, _) => self.must_add(to, Section::Apps, app.as_str(), None).map(drop),
            (ChipKind::Key, _, Some(v), _) => {
                let key = v.key_id.unwrap_or_default();
                self.must_add(to, Section::Secrets, &key, None).map(drop)
            }
            (ChipKind::Project, _, _, Some(p)) => {
                let x = self.ws(to)?;
                if x.look.projects.iter().any(|q| q.same_as(&p)) {
                    return Err(refused(format!("{} is already in it", p.as_name)));
                }
                let ids: Vec<&str> = x.look.projects.iter().map(|q| q.id.as_str()).collect();
                let id = free_id(&ids, &p.id);
                x.look.layout.icons.push(format!("project:{id}"));
                x.look.projects.push(Project { id, ..p });
                self.touched(to);
                Ok(())
            }
            (_, _, Some(v), _) => {
                let e = v.ext.ok_or_else(|| refused("Nothing to copy"))?;
                self.must_add(to, Section::Addons, &format!("{}:{}", e.app, e.id), Some(&src)).map(drop)
            }
            _ => Err(refused("Nothing to copy")),
        }
    }

    fn new_variant(&mut self, ws: &str, app: AppId, new: NewVariant) -> Result<Option<String>, EditError> {
        match new {
            NewVariant::Addon(item) => {
                self.must_add(ws, Section::Addons, &item, None)?;
                Ok(Some(format!("ext:{item}")))
            }
            NewVariant::Site { name, url } => {
                self.add_web_app(ws, &name, &url).map(|id| Some(format!("ext:wadbrowser:{id}")))
            }
            NewVariant::Named(name) => {
                let name = check::name(&name)?;
                if !is_named_app(app) {
                    return Err(refused(format!("{} has no variants of its own", app_name(app))));
                }
                let x = self.ws(ws)?;
                let e = add_named(x, app, &name).ok_or_else(|| refused("Give it a name"))?;
                // A gamespace's first Dolphin game is what it starts straight into (as Steam's first is).
                if app == AppId::Dolphin && x.look.launch.is_none() {
                    x.look.launch = Some(format!("variant:{}", e.id));
                }
                self.touched(ws);
                Ok(Some(format!("ext:{}:{}", e.app, e.id)))
            }
            NewVariant::Key(key) => self.sign_in_new(ws, app, &key),
            NewVariant::NewKey { name, kind } => {
                let key = self.new_key(&name, kind)?;
                self.sign_in_new(ws, app, &key)
            }
        }
    }

    /// A key into `ws`, as a sign-in of `app`'s: its variant's id.
    fn sign_in_new(&mut self, ws: &str, app: AppId, key: &str) -> Result<Option<String>, EditError> {
        self.must_add(ws, Section::Secrets, key, None)?;
        let x = self.get(ws)?;
        Ok(variants_of(x, app)
            .into_iter()
            .find(|v| v.kind == VariantKind::SignIn && v.key_id.as_deref() == Some(key))
            .map(|v| v.id))
    }

    fn remove_variant(&mut self, ws: &str, variant: &str) -> Result<(), EditError> {
        let v = find_variant(self.get(ws)?, variant).ok_or_else(|| not_found("variant", variant))?;
        match (v.kind, &v.slot, &v.ext) {
            (VariantKind::App, _, _) => Err(refused(format!("{} is the app itself: take the app out", v.name))),
            (VariantKind::SignIn, Some(slot), _) => self.remove(ws, Section::Secrets, slot).map(drop),
            (_, _, Some(e)) => self.remove(ws, Section::Addons, &format!("{}:{}", e.app, e.id)).map(drop),
            _ => Err(not_found("variant", variant)),
        }
    }

    fn add_web_app(&mut self, ws: &str, name: &str, url: &str) -> Result<String, EditError> {
        let host = url_host(url).ok_or_else(|| refused("That isn’t a web address"))?;
        let name = if name.trim().is_empty() { truncate(&host, MAX_NAME) } else { check::name(name)? };
        let x = self.ws(ws)?;
        let ids: Vec<&str> = x.exts.iter().map(|e| e.id.as_str()).collect();
        let id = free_id(&ids, &name.to_lowercase());
        add_app(x, AppId::Wadbrowser);
        x.exts.push(crate::types::Ext::new(AppId::Wadbrowser, id.clone()).named(name).at(format!("https://{host}")));
        x.look.layout.icons.push(format!("web:{id}"));
        self.touched(ws);
        Ok(id)
    }

    // ---------------------------------------------------------- sign-ins

    fn new_key(&mut self, name: &str, kind: SecretKind) -> Result<String, EditError> {
        let name = if name.trim().is_empty() { kind.as_str().to_string() } else { check::name(name)? };
        let ids: Vec<&str> = self.lib.keys.iter().map(|k| k.id.as_str()).collect();
        let id = free_id(&ids, &name);
        // A Steam account's sign-in is its Steam folders.
        let paths = if kind == SecretKind::Steam {
            [".steam/", ".local/share/Steam/config/", ".local/share/Steam/userdata/", ".local/share/Steam/steamapps/"]
                .map(String::from)
                .to_vec()
        } else {
            vec![]
        };
        self.lib.keys.push(Secret {
            id: id.clone(),
            name,
            kind,
            paths,
            size: 0,
            created_at: self.ctx.now_ms,
            changed_at: None,
            used_by: vec![],
            sites: vec![],
        });
        self.out.created = Some(id.clone());
        Ok(id)
    }

    /// A site into a web key's sites (`existing`'s), else a new web key for it.
    fn save_web_key(&mut self, name: &str, site: &str, existing: Option<&str>) -> Result<(), EditError> {
        let site = web_site(site).ok_or_else(|| refused("That isn’t a web address"))?;
        let now = self.ctx.now_ms;
        if let Some(id) = existing {
            let k = self.lib.keys.iter_mut().find(|k| k.id == id).ok_or_else(|| not_found("key", id))?;
            if k.kind != SecretKind::Web {
                return Err(refused(format!("{} isn’t a web sign-in", k.name)));
            }
            if !k.sites.contains(&site) {
                k.sites.push(site.clone());
                k.changed_at = Some(now);
            }
            self.out.toast = Some(format!("{site} signs in with {}", k.name));
            self.out.created = Some(k.id.clone());
            return Ok(());
        }
        let shown = if name.trim().is_empty() { truncate(&site, MAX_NAME) } else { check::name(name)? };
        let id = self.new_key(&shown, SecretKind::Web)?;
        if let Some(k) = self.lib.keys.iter_mut().find(|k| k.id == id) {
            k.sites = vec![site.clone()];
        }
        self.out.toast = Some(format!("Saved sign-in “{shown}”"));
        Ok(())
    }

    /// What `variant` in `ws` signs in with (none: as its app does, or nothing). A web app with a key of its own leaves the browser's group for one of its own (or one with that key already).
    fn sign_in(&mut self, ws: &str, variant: &str, key: Option<&str>) -> Result<(), EditError> {
        let v = find_variant(self.get(ws)?, variant).ok_or_else(|| not_found("variant", variant))?;
        let kind = key_kind_of(&v).ok_or_else(|| refused(format!("{} doesn’t sign in", v.name)))?;
        let r: Option<KeyRef> = match key {
            None => None,
            Some(k) => {
                let s = self.lib.key(k).ok_or_else(|| not_found("key", k))?;
                if s.kind != kind {
                    return Err(refused(format!("{} doesn’t sign in with a {} key", v.name, s.kind)));
                }
                Some(s.key_ref())
            }
        };
        let x = self.ws(ws)?;
        let groups = &mut x.secrets.groups;
        let is_browser = |g: &KeyGroup| g.members.iter().any(|m| m == "browser");
        let free_group = |groups: &[KeyGroup], key: &str| {
            let ids: Vec<&str> = groups.iter().map(|g| g.id.as_deref().unwrap_or("")).collect();
            new_id(&ids, &format!("g-{key}"))
        };
        match (v.kind, &v.ext) {
            (VariantKind::App, _) if v.app == AppId::Wadbrowser => match (r, groups.iter().position(is_browser)) {
                (Some(r), Some(i)) => groups[i].key = r,
                (Some(r), None) => {
                    let own: Vec<&String> = groups.iter().flat_map(|g| &g.members).collect();
                    let mut members = vec!["browser".to_string()];
                    members.extend(
                        x.exts
                            .iter()
                            .filter(|e| e.app == AppId::Wadbrowser && e.url.is_some())
                            .map(|e| format!("web:{}", e.id))
                            .filter(|m| !own.contains(&m)),
                    );
                    let id = free_group(groups, &r.id);
                    groups.push(KeyGroup { id: Some(id), members, key: r });
                }
                (None, Some(i)) => {
                    groups.remove(i);
                }
                (None, None) => {}
            },
            (VariantKind::App, _) => {
                let slot = format!("app:{}", v.app);
                match r {
                    Some(r) => {
                        x.secrets.by_slot.insert(slot, r);
                    }
                    None => {
                        x.secrets.by_slot.shift_remove(&slot);
                    }
                }
            }
            (VariantKind::Web, Some(e)) => {
                let m = format!("web:{}", e.id);
                for g in groups.iter_mut() {
                    g.members.retain(|y| *y != m);
                }
                groups.retain(|g| !g.members.is_empty());
                let b = groups.iter().position(is_browser);
                let to = match &r {
                    None => b,
                    Some(r) if b.is_some_and(|i| groups[i].key.id == r.id) => b,
                    Some(r) => groups.iter().position(|g| !is_browser(g) && g.key.id == r.id),
                };
                match (to, r) {
                    (Some(i), _) => groups[i].members.push(m),
                    (None, Some(r)) => {
                        let id = free_group(groups, &r.id);
                        groups.push(KeyGroup { id: Some(id), members: vec![m], key: r });
                    }
                    (None, None) => {}
                }
            }
            (_, Some(e)) => {
                let slot = format!("ext:{}", e.id);
                match r {
                    Some(r) => {
                        x.secrets.by_slot.insert(slot, r);
                    }
                    None => {
                        x.secrets.by_slot.shift_remove(&slot);
                    }
                }
            }
            _ => return Err(refused(format!("{} doesn’t sign in", v.name))),
        }
        self.touched(ws);
        Ok(())
    }

    fn swap_key(&mut self, ws: &str, slot: &str, key: &str) -> Result<(), EditError> {
        let k = self.lib.key(key).ok_or_else(|| not_found("key", key))?.key_ref();
        let x = self.ws(ws)?;
        let done = if let Some(rest) = slot.strip_prefix("group") {
            let gid = rest.strip_prefix(':');
            match x.secrets.groups.iter_mut().find(|g| g.id.as_deref() == gid) {
                Some(g) if g.key.kind == k.kind => {
                    g.key = k.clone();
                    true
                }
                _ => false,
            }
        } else {
            match x.secrets.by_slot.get_mut(slot) {
                Some(r) if r.kind == k.kind => {
                    *r = k.clone();
                    true
                }
                _ => false,
            }
        };
        if !done {
            return Err(refused(format!("{} doesn’t sign in with {}", slot_label(slot), k.name)));
        }
        self.touched(ws);
        Ok(())
    }

    fn drop_key(&mut self, ws: &str, app: AppId, key: &str) -> Result<(), EditError> {
        let x = self.get(ws)?;
        let f = folders_of(x, app)
            .into_iter()
            .find(|f| f.key.as_ref().is_some_and(|k| k.id == key))
            .ok_or_else(|| not_found("sign-in", key))?;
        let before = x.clone();
        if f.home {
            self.sign_in(ws, &format!("app:{app}"), None)?;
        }
        for v in &f.items {
            if v.kind != VariantKind::App && sign_in_of(self.get(ws)?, v).is_some_and(|s| s.own) {
                self.sign_in(ws, &v.id, None)?;
            }
        }
        self.touched(ws);
        let key_name = f.key.map(|k| k.name).unwrap_or_default();
        let toast = format!("{} doesn’t sign in with {key_name} now", before.name);
        self.keep_undo(before, toast);
        Ok(())
    }

    fn join_group(&mut self, ws: &str, key: Option<&str>) -> Result<(), EditError> {
        let x = self.get(ws)?;
        let name = x.name.clone();
        let was = identity_of(x);
        if was.as_ref().map(|(_, k)| k.id.as_str()) == key {
            return Ok(());
        }
        if let Some((slot, _)) = was {
            self.remove(ws, Section::Secrets, &slot)?;
        }
        let said = match key {
            None => format!("{name} signs in with nothing now"),
            Some(k) => format!("{name}: {}", self.must_add(ws, Section::Secrets, k, None)?),
        };
        self.touched(ws);
        self.out.toast = Some(said);
        Ok(())
    }

    // ---------------------------------------------------------- its look

    fn app_wallpaper(&mut self, ws: &str, variant: &str, wallpaper: Option<Wallpaper>) -> Result<(), EditError> {
        let v = find_variant(self.get(ws)?, variant).ok_or_else(|| not_found("variant", variant))?;
        if v.kind == VariantKind::SignIn {
            return Err(refused("A sign-in opens its app, on its app’s wallpaper"));
        }
        if let Some(w) = &wallpaper {
            check::wallpaper(w)?;
        }
        let x = self.ws(ws)?;
        match &v.ext {
            None => {
                if let Some(a) = x.apps.iter_mut().find(|a| a.app == v.app) {
                    a.wallpaper = wallpaper;
                }
            }
            Some(e) => {
                if let Some(y) = x.exts.iter_mut().find(|y| y.app == e.app && y.id == e.id) {
                    y.wallpaper = wallpaper;
                }
            }
        }
        self.touched(ws);
        Ok(())
    }

    fn keep_files(&mut self, ws: &str, files: Vec<FileRef>, wallpaper: Option<Wallpaper>) -> Result<(), EditError> {
        self.get(ws)?;
        if let Some(w) = &wallpaper {
            check::wallpaper(w)?;
        }
        let mut added = vec![];
        for f in files {
            let source = ProjectSource { machine_id: f.machine, paths: vec![f.path] };
            let kind = if f.folder { ProjectKind::Mount { source, make: false } } else { ProjectKind::Copy { source } };
            let p = Project { id: self.uid("p"), as_name: f.name, kind };
            check::project(&p)?;
            added.push(p);
        }
        let x = self.ws(ws)?;
        x.look.layout.icons.extend(added.iter().map(|p| format!("project:{}", p.id)));
        x.look.projects.extend(added);
        if let Some(w) = wallpaper {
            x.look.wallpaper = w;
        }
        self.touched(ws);
        Ok(())
    }

    // ---------------------------------------------------------- tags

    fn create_tag(&mut self, name: &str, ws: Option<&str>) -> Result<(), EditError> {
        let clean = if name.trim().is_empty() { "New tag".to_string() } else { check::name(name)? };
        let ids: Vec<&str> = std::iter::once(GENERAL_ID).chain(self.lib.tags.iter().map(|t| t.id.as_str())).collect();
        let id = free_id(&ids, &clean);
        let names: Vec<&str> =
            std::iter::once(GENERAL_NAME).chain(self.lib.tags.iter().map(|t| t.name.as_str())).collect();
        let name = copy_name(&names, &clean);
        let color = TAG_COLORS[self.lib.tags.len() % TAG_COLORS.len()].to_string();
        self.lib.tags.push(Tag { id: id.clone(), name, color });
        if let Some(ws) = ws {
            self.set_tag(ws, Some(&id))?;
        }
        self.out.created = Some(id);
        Ok(())
    }

    fn set_tag(&mut self, ws: &str, tag: Option<&str>) -> Result<(), EditError> {
        let tag = match tag {
            None | Some(GENERAL_ID) => None,
            Some(t) if self.lib.tags.iter().any(|y| y.id == t) => Some(t.to_string()),
            Some(t) => return Err(not_found("tag", t)),
        };
        self.ws(ws)?.tag = tag;
        self.touched(ws);
        Ok(())
    }

    // ---------------------------------------------------------- games

    fn add_game(&mut self, ws: &str, account: &str, game: &str) -> Result<(), EditError> {
        let k = self.lib.key(account).ok_or_else(|| not_found("key", account))?;
        if k.kind != SecretKind::Steam {
            return Err(refused(format!("{} isn’t a Steam account", k.name)));
        }
        if self.lib.steam.get(account).is_some_and(|l| !l.owned.iter().any(|g| g == game)) {
            return Err(refused(format!("{} doesn’t own {}", k.name, game_of(game, None).name)));
        }
        let key = k.key_ref();
        self.must_add(ws, Section::Addons, &format!("steam:{game}"), None)?;
        let x = self.ws(ws)?;
        x.secrets.by_slot.entry("app:steam".to_string()).or_insert(key);
        if x.look.launch.is_none() {
            x.look.launch = Some(format!("variant:{game}"));
        }
        x.category = Some(Category::Gaming);
        self.touched(ws);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_read_as_the_spikes_actions() {
        let op: Op = serde_json::from_str(r#"{"op": "space.remove", "ws": "vanua-academy", "chip": "app:wadbrowser"}"#)
            .expect("an op");
        assert_eq!(op, Op::Remove { ws: "vanua-academy".into(), chip: "app:wadbrowser".into() });
        let op: Op = serde_json::from_str(
            r#"{"op": "apps.newVariant", "ws": "x", "app": "dolphin", "new": {"named": "Mario Kart"}}"#,
        )
        .expect("an op");
        assert_eq!(
            op,
            Op::NewVariant { ws: "x".into(), app: AppId::Dolphin, new: NewVariant::Named("Mario Kart".into()) }
        );
        let op: Op = serde_json::from_str(
            r#"{"op": "builder.fresh", "payload": {"kind": "folder", "ws": "vanua-academy", "app": "wadbrowser", "key": "vanua-google"}}"#,
        )
        .expect("an op");
        assert!(matches!(op, Op::Fresh { payload: Payload::Folder { app: AppId::Wadbrowser, .. } }));
        let op: Op = serde_json::from_str(
            r#"{"op": "builder.add", "ws": "x", "section": "customizations", "item": "repo:a/b"}"#,
        )
        .expect("an op");
        assert!(matches!(op, Op::Add { section: Section::Look, .. }));
        let back = serde_json::to_value(Op::SetTag { ws: "retro".into(), tag: None }).expect("serializes");
        assert_eq!(back, serde_json::json!({"op": "space.tag", "ws": "retro", "tag": null}));
    }

    #[test]
    fn an_edit_that_fails_changes_nothing() {
        let mut lib = Library::default();
        let ctx = Ctx::new(1000, "m-surface");
        let made = apply(&mut lib, Op::Create { name: Some("Tools".into()), category: None, machine: None }, &ctx)
            .expect("made");
        let id = made.created.expect("an id");
        assert_eq!(made.changed, std::slice::from_ref(&id));
        let was = lib.clone();
        // A key made, then the add fails (a web key can't go… it can: but a missing one can't).
        let r = apply(
            &mut lib,
            Op::NewVariant { ws: id.clone(), app: AppId::Steam, new: NewVariant::Key("nope".into()) },
            &ctx,
        );
        assert!(matches!(r, Err(EditError::Refused(_))));
        assert_eq!(lib, was);
        let r = apply(&mut lib, Op::Rename { ws: id.clone(), name: "x".repeat(81) }, &ctx);
        assert_eq!(r, Err(EditError::Check(CheckError::Name)));
        assert_eq!(lib, was);
        let r = apply(&mut lib, Op::Rename { ws: "nope".into(), name: "A".into() }, &ctx);
        assert_eq!(r, Err(EditError::NotFound { what: "wadspace", id: "nope".into() }));
    }
}
