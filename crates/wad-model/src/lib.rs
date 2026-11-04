//! The WadSpaces v2 model: a wadspace's document (version 6), the catalog,
//! and the pure logic that reads and edits them. It's shared by the
//! machine's daemon (wadd), the shell and the other apps. No I/O, no
//! clocks, no randomness: the time and the machine come in an
//! [`edit::Ctx`], and new ids from the library's own counter.
//!
//! It's a port of the v2 UI spike (`../wadspaces-v2-spike`: its README is
//! the spec): `src/model/types.ts`, `catalog.ts` (with `data/catalog.json`
//! copied unmodified into `data/`), `wallpapers.ts`, and from `src/sim/`
//! `ops.ts`, `variants.ts`, `folders.ts`, `groups.ts`, `suggest.ts`,
//! `games.ts` (its pure parts), `looks.ts`, `chips.ts`, `tags.ts`,
//! `commands.ts`, `starter.ts`, and the wadspace-editing actions of
//! `actions/spaces.ts`, `actions/builder.ts` and `actions/browser.ts`
//! (`browser.newApp`) as [`edit::Op`]s.
//!
//! Where it isn't the spike:
//!
//! - **One look.** A wadspace has one [`types::Look`] (its picture, projects,
//!   autostart, launch, icon layout, icon and title bars), not
//!   `customizations[]` of `variants[]` with `defaultCustomization`: they
//!   weren't in the UI (a copy you change does their job). Its `tag` moved
//!   up from the customization to the wadspace. Everything that walked the
//!   customizations walks the look. The customization actions
//!   (`custom.*`, `variant.add/rename/default/remove`,
//!   `ws.newCustomization`) are gone; `variant.wallpaper` is
//!   [`edit::Op::SetWallpaper`], and `variant.removeProject`,
//!   `variant.autostart`, `variant.order` act on the look. The Look
//!   section ([`ops::Section::Look`], "customizations" still reads as it)
//!   dropped onto another wadspace gives it its picture (and icon) and adds
//!   its files beside its own, where the spike added a customization; the
//!   item `customization:<ws>:<id>` is `look:<ws>`. An icon has no `style`
//!   (it was always 'picture').
//! - **Gone:** `owner`, `sharedWith`, descriptions (a wadspace's; the
//!   catalog's apps keep theirs), `sensitive` and passwords (a stream's
//!   `user` and `password` too, and its `private`, which `mode` says),
//!   v5 and its conversion, WadNotes, light-theme anything (the catalog's
//!   screenshots aren't read), the retired backgrounds. A [`types::Wallpaper`]
//!   is a picture, a plain colour, or `Empty` (the spike's BLANK gradient):
//!   no gradients or textures, and a picture is a path (an upload goes
//!   into ~/Pictures), at most 4 KB.
//! - **What a machine knows on its own** isn't in the document: `built` and
//!   `sizeMb` are [`types::BuildState`]; an [`edit::Outcome`] says whether an
//!   edit `needs_build`. The rule is the image's: an edit needs a build if
//!   it changed the wadspace's apps, its add-ons but its games (they're its
//!   apps' content), or its commands. (The spike flagged it per action:
//!   adding Dolphin with its first game, or Steam with a key, didn't, and
//!   a game added to Steam did.) Its run state is [`types::RunState`] (the
//!   spike's WorkspaceState). `lastLaunched` stays in the document.
//! - **Types.** Apps are an enum ([`types::AppId`]); `order` is an integer;
//!   a project's `as` is `as_name`; optional flags (`agents`, `starter`,
//!   `make`, `kiosk`…) are `bool`s left out when false. `bySlot` keeps its
//!   order ([`indexmap::IndexMap`]). Sessions, streams and jobs lost their
//!   customization and variant, and the sim's own fields (rng, faults, wake
//!   times). serde's `camelCase` keeps the JSON close to the spike's.
//! - **Edits are atomic and say why.** [`edit::apply`] makes all of an edit
//!   or none of it, and checks every wadspace it changed ([`check`]): ids of
//!   safe characters, names of 1 to 80 characters on one line, a
//!   project's name never a path, commands of one line, sane counts. Where
//!   the spike did nothing (no such wadspace, variant, key, chip, tag,
//!   command or disc; a refusal it toasted) it's an [`edit::EditError`]; a
//!   change with nothing to do (taking out what isn't there) is an empty
//!   [`edit::Outcome`]. A key of the wrong kind for a slot, an unknown tag
//!   (`space.tag` made it General), a launch, autostart or icon order naming
//!   what isn't in the wadspace, a game its Steam account doesn't own (when
//!   its library is known), and a disc already in it, are refused.
//! - **Undo** comes back in the [`edit::Outcome`] ([`edit::UndoSnapshot`]),
//!   not kept in the world, and [`edit::undo`] puts it back.
//! - **What's taken out goes everywhere.** Taking an app out takes its
//!   add-ons' sign-ins too (Claude Code's with VS Code); taking a web app
//!   out takes it out of its sign-in group (an empty group goes); and what's
//!   gone leaves the look's icons, autostart and launch ([`look::prune`]).
//!   The spike's `space.remove` lost a `web:` launch on any chip taken out;
//!   it doesn't here. A web key for the browser doesn't put a web app with a
//!   key of its own in the browser's group as well.
//! - **Names.** "Name (copy)" and "Name 2" are cut short to stay names. A
//!   copy (`space.duplicate`, `ws.duplicate`: one op) isn't the starter and
//!   hasn't been launched. `builder.create` and `space.create` are one op,
//!   [`edit::Op::Create`]; a new web app with no name is named for its host.
//!   A command typed as "RUN" alone is a RUN with nothing to run.
//! - **Not here** (the machine's, not the model's): jobs, timers, streams'
//!   and sessions' lifecycles, windows, files (an edit takes a
//!   [`edit::FileRef`], not a file's node), sign-in flows (`steam.signIn`,
//!   the login job: [`edit::Op::NewKey`] makes the key; a Steam key gets
//!   its Steam folders as `paths`), uploading discs, downloading and
//!   staging games, agents, sandboxes, setup, the tailnet, the phone, the
//!   spike's overlay, the Recent/Running lists and a wadspace's own
//!   desktop entries. Whether a wadspace is this machine's to edit is the
//!   caller's to decide (the spike doesn't check either).

pub mod catalog;
pub mod check;
pub mod chips;
pub mod commands;
pub mod edit;
pub mod folders;
pub mod games;
pub mod groups;
pub mod look;
pub mod names;
pub mod ops;
pub mod stack;
pub mod starter;
pub mod suggest;
pub mod tags;
pub mod types;
pub mod variants;
pub mod wallpapers;

pub use check::CheckError;
pub use edit::{Ctx, EditError, FileRef, Library, NewVariant, Op, Outcome, Payload, UndoSnapshot, apply, undo};
pub use look::IconPatch;
pub use ops::Section;
pub use types::*;
