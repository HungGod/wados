//! What letting something go on a wadspace does, wherever it's shown: its
//! Builder stack, its desktop icon, its tile on the bar (the spike's
//! `shell/spaces/targets.ts`). A wadspace takes another's piece (a stack's
//! section, one thing, a key's group): it's copied in. Its own pieces, and
//! whole wadspaces, it doesn't take. Another machine's says where it lives.
//!
//! And open desktop, or the Builder's slot (`offer_fresh`/`land_fresh`): a
//! piece is a new wadspace of just that, Untitled; a whole stack, a copy.
//! (A desktop icon dropped on open desktop moves: the desktop's own.)

use std::rc::Rc;

use wad_model::edit::{Op, Payload};
use wad_model::{Section, Wadspace};

use crate::dnd::{Held, SpaceFrom, Verdict};
use crate::shell::Shell;

/// What letting `held` go on `ws` would do (none: it doesn't take it).
pub fn offer_on(shell: &Shell, ws: &str, held: &Held) -> Option<Verdict> {
    let x = shell.store.wadspace(ws).filter(|x| !x.is_trashed())?;
    if x.machine != shell.store.machine_id() {
        return Verdict::no(format!("It lives on {}", machine_name(shell, &x)));
    }
    match held {
        Held::Space { .. } => None,
        Held::Part { payload } => match payload {
            Payload::Section { ws: from, .. } if *from == x.id => None,
            Payload::Section { .. } => Verdict::yes(format!("Into {}", x.name)),
            Payload::Piece { from, .. } if from.as_deref() == Some(x.id.as_str()) => None,
            Payload::Piece { .. } => Verdict::yes(format!("Add it to {}", x.name)),
            Payload::Folder { ws: from, .. } if *from == x.id => None,
            Payload::Folder { key, .. } => {
                let name = key
                    .as_ref()
                    .and_then(|k| shell.store.library().keys.iter().find(|s| s.id == *k).map(|s| s.name.clone()));
                Verdict::yes(match name {
                    Some(n) => format!("Copy {n} into {}", x.name),
                    None => format!("Copy it into {}", x.name),
                })
            }
        },
        Held::Pieces { from, .. } if *from == x.id => None,
        Held::Pieces { items, .. } => Verdict::yes(format!("Add {} to {}", count(items.len()), x.name)),
    }
}

/// Letting `held` go on `ws`: copied in.
pub fn land_on(shell: &Rc<Shell>, ws: &str, held: &Held) {
    if !offer_on(shell, ws, held).is_some_and(|v| v.ok) {
        return;
    }
    for op in ops_on(ws, held) {
        shell.op(op, |_| {});
    }
}

/// The edits letting `held` go on `ws` makes.
fn ops_on(ws: &str, held: &Held) -> Vec<Op> {
    match held {
        Held::Space { .. } => vec![],
        Held::Part { payload: Payload::Folder { ws: from, app, key } } => {
            vec![Op::CopyFolder { from: from.clone(), to: ws.to_string(), app: *app, key: key.clone() }]
        }
        Held::Part { payload } => vec![Op::DropOn { ws: ws.to_string(), payload: payload.clone() }],
        Held::Pieces { section, items, from } => {
            items.iter().map(|item| Op::DropOn { ws: ws.to_string(), payload: piece(*section, item, from) }).collect()
        }
    }
}

/// What letting `held` go on open desktop (or the Builder's slot) would do.
pub fn offer_fresh(shell: &Shell, held: &Held) -> Option<Verdict> {
    if !shell.store.connected() {
        return None;
    }
    match held {
        // A wadspace's icon moves (the desktop's own), or opens in the Builder (its board's).
        Held::Space { from: Some(_), .. } => None,
        Held::Space { ws, from: None } => {
            let name = shell.store.wadspace(ws).map(|x| x.name).unwrap_or_else(|| "it".into());
            Verdict::yes(format!("A copy of {name}"))
        }
        Held::Part { .. } | Held::Pieces { .. } => Verdict::yes("A new wadspace, of just this"),
    }
}

/// Letting `held` go on open desktop (or the slot): a new wadspace of just
/// that (in the Builder, beside where it came from), or a copy of a stack.
pub fn land_fresh(shell: &Rc<Shell>, held: &Held) {
    let Some(b) = shell.builder() else { return };
    match held {
        Held::Space { from: Some(_), .. } => {}
        Held::Space { ws, from: None } => b.copy(ws),
        Held::Part { payload } => b.fresh(payload.clone()),
        // Several picked together: a new one of the first, then the rest into it.
        Held::Pieces { section, items, from } => {
            let Some((first, rest)) = items.split_first() else { return };
            let (shell, rest, section, from) = (shell.clone(), rest.to_vec(), *section, from.clone());
            b.fresh_then(piece(section, first, &from), move |id| {
                for item in &rest {
                    shell.op(Op::DropOn { ws: id.to_string(), payload: piece(section, item, &from) }, |_| {});
                }
            });
        }
    }
}

/// Whether `held` is a wadspace's own icon (from the desktop, a folder, the bar).
pub fn is_icon(held: &Held) -> bool {
    matches!(held, Held::Space { from: Some(SpaceFrom::Desktop | SpaceFrom::Folder | SpaceFrom::Bar), .. })
}

fn piece(section: Section, item: &str, from: &str) -> Payload {
    Payload::Piece { section, item: item.to_string(), from: Some(from.to_string()) }
}

fn count(n: usize) -> String {
    if n == 1 { "it".into() } else { format!("these {n}") }
}

fn machine_name(shell: &Shell, x: &Wadspace) -> String {
    let _ = shell;
    // Peers (their names) come in phase 6.
    if x.machine.is_empty() { "another machine".into() } else { x.machine.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_model::AppId;

    #[test]
    fn a_key_s_group_is_copied_and_the_rest_dropped_on() {
        let folder = Held::Part {
            payload: Payload::Folder { ws: "a".into(), app: AppId::Wadbrowser, key: Some("work".into()) },
        };
        assert_eq!(
            ops_on("b", &folder),
            vec![Op::CopyFolder { from: "a".into(), to: "b".into(), app: AppId::Wadbrowser, key: Some("work".into()) }]
        );
        let section = Held::Part { payload: Payload::Section { ws: "a".into(), section: Section::Apps } };
        assert_eq!(
            ops_on("b", &section),
            vec![Op::DropOn { ws: "b".into(), payload: Payload::Section { ws: "a".into(), section: Section::Apps } }]
        );
        let pieces = Held::Pieces {
            section: Section::Addons,
            items: vec!["wadbrowser:gmail".into(), "wadbrowser:drive".into()],
            from: "a".into(),
        };
        assert_eq!(ops_on("b", &pieces).len(), 2);
        assert!(ops_on("b", &Held::Space { ws: "a".into(), from: None }).is_empty());
    }
}
