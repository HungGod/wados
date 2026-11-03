// What's in a wadspace, as its card shows it: one flat list of chips, in
// order: its apps, its web apps, its other add-ons (extensions, tools,
// Focus), its games and named variants, its sign-ins, then its files,
// folders and repositories. A chip's id says what it is, so removing or
// copying it to another wadspace needs nothing else:
//   'app:krita'  'ext:wadbrowser:gmail'  'key:<slot>'  'project:<id>'

use serde::{Deserialize, Serialize};

use crate::types::{AppId, Project, ProjectKind, Wadspace};
use crate::variants::{AppVariant, VariantKind, variants_of};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChipKind {
    App,
    Web,
    Addon,
    Game,
    Key,
    Project,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chip {
    pub id: String,
    pub kind: ChipKind,
    pub name: String,
    /// Its app (an add-on's, a game's, a sign-in's: the app it's for).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<AppId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<AppVariant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<Project>,
    /// The wadspace starts straight into it.
    #[serde(default, skip_serializing_if = "crate::types::is_false")]
    pub launch: bool,
}

pub fn chips_of(x: &Wadspace) -> Vec<Chip> {
    let launch = x.look.launch.as_deref();
    let mut out = vec![];
    for a in &x.apps {
        for v in variants_of(x, a.app) {
            let kind = match v.kind {
                VariantKind::App => ChipKind::App,
                VariantKind::Web => ChipKind::Web,
                VariantKind::SignIn => ChipKind::Key,
                VariantKind::Game | VariantKind::Named => ChipKind::Game,
                _ => ChipKind::Addon,
            };
            let key = match (&v.kind, &v.ext) {
                (VariantKind::App, _) => Some(format!("app:{}", v.app)),
                (_, Some(e)) => Some(format!("variant:{}", e.id)),
                _ => None,
            };
            let web = v.ext.as_ref().map(|e| format!("web:{}", e.id));
            let launches = key.is_some_and(|k| launch == Some(k.as_str()))
                || (kind == ChipKind::Web && web.is_some_and(|w| launch == Some(w.as_str())));
            // A sign-in's chip is its slot, so two apps' sign-ins with one key stay two chips.
            let id = match &v.slot {
                Some(slot) if v.kind == VariantKind::SignIn => format!("key:{slot}"),
                _ => v.id.clone(),
            };
            out.push(Chip {
                id,
                kind,
                name: v.name.clone(),
                app: Some(v.app),
                variant: Some(v),
                project: None,
                launch: launches,
            });
        }
    }
    for p in &x.look.projects {
        out.push(Chip {
            id: format!("project:{}", p.id),
            kind: ChipKind::Project,
            name: p.as_name.clone(),
            app: None,
            variant: None,
            project: Some(p.clone()),
            launch: false,
        });
    }
    out.sort_by_key(|c| c.kind);
    out
}

/// A chip of `x` by its id.
pub fn chip_of(x: &Wadspace, id: &str) -> Option<Chip> {
    chips_of(x).into_iter().find(|c| c.id == id)
}

/// What a project is, for its chip's icon (a lucide icon's name).
pub fn project_glyph(p: &Project) -> &'static str {
    match p.kind {
        ProjectKind::Github { .. } => "git-branch",
        ProjectKind::Mount { .. } => "folder",
        ProjectKind::Copy { .. } => "file",
    }
}
