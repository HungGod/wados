//! What a wadspace's Builder stack is made of (the spike's
//! `apps/builder/parts.ts`; the phone's cards too): its apps (the command
//! line last), a box per key (an app's things that sign in with it), and a
//! box per app with add-ons (the command line's with its own commands,
//! RUN_1…, even when it has no tools).

use crate::folders::{Folder, folders_of};
use crate::types::{AppId, BuildCommand, Wadspace};
use crate::variants::{AppVariant, VariantKind};

/// Its apps, the command line last.
pub fn stack_apps(x: &Wadspace) -> Vec<AppId> {
    let (term, rest): (Vec<_>, Vec<_>) = x.apps.iter().map(|a| a.app).partition(|a| *a == AppId::Terminal);
    rest.into_iter().chain(term).collect()
}

/// A key's box: `app`'s things that sign in with `folder`'s key.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyBox {
    pub app: AppId,
    pub folder: Folder,
}

/// A box per key: each app's things that sign in with one.
pub fn key_boxes(x: &Wadspace) -> Vec<KeyBox> {
    stack_apps(x)
        .into_iter()
        .flat_map(|app| {
            folders_of(x, app).into_iter().filter(|f| f.key.is_some()).map(move |folder| KeyBox { app, folder })
        })
        .collect()
}

/// An app's add-ons' box; the command line's has its own commands.
#[derive(Debug, Clone, PartialEq)]
pub struct AddonBox {
    pub app: AppId,
    pub items: Vec<AppVariant>,
    pub commands: Vec<BuildCommand>,
}

/// A box per app with add-ons; its own commands in the command line's.
pub fn addon_boxes(x: &Wadspace) -> Vec<AddonBox> {
    let mut out: Vec<AddonBox> = stack_apps(x)
        .into_iter()
        .map(|app| AddonBox {
            app,
            items: folders_of(x, app)
                .into_iter()
                .flat_map(|f| f.items.into_iter().filter(|v| v.kind != VariantKind::App))
                .collect(),
            commands: if app == AppId::Terminal { x.commands.clone() } else { vec![] },
        })
        .filter(|b| !b.items.is_empty() || !b.commands.is_empty())
        .collect();
    if !x.commands.is_empty() && !out.iter().any(|b| b.app == AppId::Terminal) {
        out.push(AddonBox { app: AppId::Terminal, items: vec![], commands: x.commands.clone() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Library;

    fn sample() -> Library {
        serde_json::from_str(include_str!("../tests/data/populated.json")).unwrap()
    }

    fn ws<'a>(lib: &'a Library, id: &str) -> &'a Wadspace {
        lib.wadspace(id).unwrap()
    }

    #[test]
    fn the_command_line_comes_last() {
        let lib = sample();
        for x in &lib.wadspaces {
            let apps = stack_apps(x);
            if let Some(i) = apps.iter().position(|a| *a == AppId::Terminal) {
                assert_eq!(i, apps.len() - 1, "{}", x.id);
            }
            assert_eq!(apps.len(), x.apps.len());
        }
    }

    #[test]
    fn vanua_academy_has_a_box_per_key_and_per_app_with_add_ons() {
        let lib = sample();
        let x = ws(&lib, "vanua-academy");
        let keys: Vec<String> =
            key_boxes(x).iter().map(|b| b.folder.key.as_ref().map(|k| k.name.clone()).unwrap_or_default()).collect();
        // Its Secrets section (the spike's 18-builder.png): Work, Google (VanuaAcademy), hunggod, …
        for k in ["Work", "Google (VanuaAcademy)", "hunggod"] {
            assert!(keys.iter().any(|n| n == k), "{k} in {keys:?}");
        }
        for b in addon_boxes(x) {
            assert!(!b.items.is_empty() || !b.commands.is_empty());
            assert!(b.items.iter().all(|v| v.kind != VariantKind::App));
        }
    }

    #[test]
    fn commands_alone_still_get_the_command_line_s_box() {
        let mut lib = sample();
        let x = lib.wadspaces.iter_mut().find(|x| !x.apps.iter().any(|a| a.app == AppId::Terminal)).unwrap();
        x.commands.push(BuildCommand {
            id: "c1".into(),
            name: "RUN_1".into(),
            kind: crate::types::CommandKind::Run,
            text: "echo hi".into(),
        });
        let boxes = addon_boxes(x);
        let last = boxes.last().unwrap();
        assert_eq!((last.app, last.items.len(), last.commands.len()), (AppId::Terminal, 0, 1));
    }
}
