// Wadspaces grouped by who they sign in as: each one's identity is its most
// telling key (a web key first, then Steam, VS Code, Obsidian, Claude
// Code), and wadspaces with the same identity share a group ("Work",
// "hunggod", "Steam"); the rest are "No sign-in". Dragged into another
// group, a wadspace drops its identity and signs in with that group's key
// instead (`edit::Op::JoinGroup`).

use crate::names::locale_cmp;
use crate::ops::keys_of;
use crate::types::{KeyRef, SecretKind, Wadspace};

const ORDER: [SecretKind; 5] =
    [SecretKind::Web, SecretKind::Steam, SecretKind::Vscode, SecretKind::Obsidian, SecretKind::VscodeClaudeCode];

fn rank(k: SecretKind) -> usize {
    ORDER.iter().position(|o| *o == k).unwrap_or(ORDER.len())
}

/// Its identity: the key (and the slot it's in) its group is by.
pub fn identity_of(x: &Wadspace) -> Option<(String, KeyRef)> {
    let mut keys = keys_of(x);
    keys.sort_by_key(|(_, k)| rank(k.kind));
    keys.into_iter().next()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group<'a> {
    /// Its key (none: no sign-in).
    pub key: Option<KeyRef>,
    pub spaces: Vec<&'a Wadspace>,
}

/// `list` in groups by identity, by name (No sign-in last), each in order.
pub fn groups_of<'a>(list: impl IntoIterator<Item = &'a Wadspace>) -> Vec<Group<'a>> {
    let mut out: Vec<Group<'a>> = vec![];
    for x in list {
        let k = identity_of(x).map(|(_, k)| k);
        let id = k.as_ref().map(|k| k.id.clone());
        match out.iter_mut().find(|g| g.key.as_ref().map(|k| &k.id) == id.as_ref()) {
            Some(g) => g.spaces.push(x),
            None => out.push(Group { key: k, spaces: vec![x] }),
        }
    }
    for g in &mut out {
        g.spaces.sort_by_key(|x| x.order);
    }
    out.sort_by(|a, b| {
        a.key.is_none().cmp(&b.key.is_none()).then_with(|| {
            let name = |g: &Group| g.key.as_ref().map(|k| k.name.clone()).unwrap_or_default();
            locale_cmp(&name(a), &name(b))
        })
    });
    out
}
