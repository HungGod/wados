// The spike's populated sample (its `world('populated')`, unlocked), as a
// library: its ten live wadspaces flattened to one look each (the default
// customization's default variant), their tags on them, its keys, tags,
// Steam library, discs and the browser's web app. Made from the spike's
// own world, so the logic here sees what the spike's did. Prettier, which
// the sample's Cosmic Bodybuilding has without VS Code, isn't in it (an
// add-on without its app isn't a state an edit can make), and the trashed
// Old site is left out.

#![allow(dead_code)]

use wad_model::{Ctx, Library, Op, Outcome, Wadspace, apply};

/// 2026-10-28 09:30 in Melbourne: the sample's now.
pub const EPOCH: i64 = 1_793_140_200_000;
pub const HERE: &str = "m-surface";

pub fn library() -> Library {
    serde_json::from_str(include_str!("../data/populated.json")).expect("tests/data/populated.json is a library")
}

pub fn ctx() -> Ctx {
    Ctx::new(EPOCH, HERE)
}

/// One edit, which has to work.
pub fn act(lib: &mut Library, op: Op) -> Outcome {
    let shown = format!("{op:?}");
    apply(lib, op, &ctx()).unwrap_or_else(|e| panic!("{shown}: {e}"))
}

pub fn ws<'a>(lib: &'a Library, id: &str) -> &'a Wadspace {
    lib.wadspace(id).unwrap_or_else(|| panic!("no wadspace {id}"))
}

pub fn s(v: &str) -> String {
    v.to_string()
}
