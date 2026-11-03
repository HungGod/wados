// An app's things in a wadspace, in a folder per sign-in, as the Builder
// shows them. The app's own folder comes first: the app itself, with what
// signs in as it does (a web app in the browser's group, a game on its
// Steam account) and what can't sign in at all (extensions, tools). Then a
// folder for each other key something signs in with, in the order its
// sign-ins are in (Gmail and Drive as Work, GitHub as hunggod; Claude Code
// with its own key). Moving a thing to another folder signs it in with that
// folder's key.

use serde::{Deserialize, Serialize};

use crate::catalog::app_name;
use crate::ops::keys_of;
use crate::types::{AppId, KeyRef, SecretKind, Wadspace};
use crate::variants::{AppVariant, VariantKind, app_key, inherits, key_kind_of, sign_in_of, variants_of};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    /// Its sign-in (none: none).
    pub key: Option<KeyRef>,
    /// The app's own folder.
    pub home: bool,
    pub items: Vec<AppVariant>,
}

/// `app`'s things in `x` (sign-ins aren't things: they're the folders).
pub fn folders_of(x: &Wadspace, app: AppId) -> Vec<Folder> {
    let vs: Vec<AppVariant> = variants_of(x, app).into_iter().filter(|v| v.kind != VariantKind::SignIn).collect();
    if vs.is_empty() {
        return vec![];
    }
    let mut home = Folder { key: app_key(x, app).cloned(), home: true, items: vec![] };
    let mut others: Vec<Folder> = vec![];
    for v in vs {
        let s = if v.kind == VariantKind::App { None } else { sign_in_of(x, &v) };
        let Some(s) = s.filter(|s| s.own && Some(&s.key.id) != home.key.as_ref().map(|k| &k.id)) else {
            home.items.push(v);
            continue;
        };
        match others.iter_mut().find(|f| f.key.as_ref().is_some_and(|k| k.id == s.key.id)) {
            Some(f) => f.items.push(v),
            None => others.push(Folder { key: Some(s.key), home: false, items: vec![v] }),
        }
    }
    // The others in the order its sign-ins are in.
    let order: Vec<String> = keys_of(x).into_iter().map(|(_, k)| k.id).collect();
    let at = |f: &Folder| f.key.as_ref().and_then(|k| order.iter().position(|id| *id == k.id)).unwrap_or(usize::MAX);
    others.sort_by_key(|f| at(f));
    let mut out = vec![home];
    out.extend(others);
    out
}

/// The folder a thing is in.
pub fn folder_of<'a>(fs: &'a [Folder], v: &AppVariant) -> Option<&'a Folder> {
    fs.iter().find(|f| f.items.iter().any(|y| y.id == v.id))
}

/// The kinds of key `app`'s things in `x` could sign in with (a new folder's).
pub fn folder_kinds(x: &Wadspace, app: AppId) -> Vec<SecretKind> {
    let mut out = vec![];
    for k in variants_of(x, app).iter().filter_map(key_kind_of) {
        if !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

/// Moving a thing to a folder: the key it would sign in with (none: as its app does, or none at all), and what to say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Move {
    pub key: Option<String>,
    pub hint: String,
}

/// Moving `v` into `to`; none if it can't go there. The app itself takes the folder's key as its own.
pub fn move_to(x: &Wadspace, v: &AppVariant, to: &Folder) -> Option<Move> {
    let fs = folders_of(x, v.app);
    let same =
        |a: &Folder| a.home == to.home && (a.home || a.key.as_ref().map(|k| &k.id) == to.key.as_ref().map(|k| &k.id));
    if folder_of(&fs, v).is_some_and(same) {
        return None;
    }
    let kind = key_kind_of(v)?;
    if to.home {
        if v.kind == VariantKind::App {
            return None;
        }
        // A web app or a game signs in as its app does; Claude Code goes without.
        let hint = match &to.key {
            Some(k) if inherits(v) => format!("{} signs in with {}, as {} does", v.name, k.name, app_name(v.app)),
            _ => format!("{} signs in with nothing", v.name),
        };
        return Some(Move { key: None, hint });
    }
    let key = to.key.as_ref().filter(|k| k.kind == kind)?;
    Some(Move { key: Some(key.id.clone()), hint: format!("{} signs in with {}", v.name, key.name) })
}
