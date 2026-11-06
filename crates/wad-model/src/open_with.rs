//! Open with (the spike's `shell/common/menus.ts` `openWithMenu`, and the
//! double-click of `shell/controller.ts`): the apps that open some files,
//! one row per wadspace of this machine's that has the app ("Krita · in
//! Art"), then the catalog's others ("Krita · in a new space"). Opened for
//! now: nothing's kept in the space. The app that last opened a kind of
//! file is remembered ([`Remembered`], the session's), ticked on the menu
//! and used by a double-click.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::catalog::{app, app_name, apps_for, mime_of};
use crate::{AppId, Wadspace};

/// A file (or folder) to open: its name, its type if it's known, and whether it's a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opening<'a> {
    pub name: &'a str,
    pub mime: Option<&'a str>,
    pub folder: bool,
}

impl<'a> Opening<'a> {
    pub fn new(name: &'a str, mime: Option<&'a str>, folder: bool) -> Self {
        Self { name, mime, folder }
    }

    /// Its type, as Open with remembers apps by.
    pub fn mime(&self) -> String {
        mime_of(self.name, self.mime, self.folder)
    }
}

/// One of Open with's rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// Open them with `app`: in `ws` (a wadspace that has it), or in a new one for it.
    App {
        app: AppId,
        ws: Option<String>,
        label: String,
        hint: Option<String>,
        checked: bool,
    },
    Sep,
    /// "No app opens it".
    Nothing,
}

impl Row {
    /// Its test hook (`open-with:krita:art`, `open-with:krita:new`).
    pub fn id(&self) -> String {
        match self {
            Row::App { app, ws, .. } => format!("open-with:{}:{}", app.as_str(), ws.as_deref().unwrap_or("new")),
            Row::Sep => "sep".into(),
            Row::Nothing => "open-with:none".into(),
        }
    }
}

/// The apps that open every one of `files`, best first.
pub fn fitting(files: &[Opening]) -> Vec<AppId> {
    let mut each = files.iter().map(|f| apps_for(f.name, f.mime, f.folder));
    let Some(first) = each.next() else { return vec![] };
    each.fold(first, |a, b| a.into_iter().filter(|x| b.contains(x)).collect())
}

/// Open with's rows for `files`: `here` is this machine's wadspaces (any
/// in the Trash are left out), `last` the app that last opened the first
/// one's kind (its first row is ticked).
pub fn rows(files: &[Opening], here: &[Wadspace], last: Option<AppId>) -> Vec<Row> {
    if files.is_empty() {
        return vec![];
    }
    let fit = fitting(files);
    if fit.is_empty() {
        return vec![Row::Nothing];
    }
    let mut spaces: Vec<&Wadspace> = here.iter().filter(|x| !x.is_trashed()).collect();
    spaces.sort_by_key(|x| x.order);
    let has = |x: &Wadspace, a: AppId| x.apps.iter().any(|y| y.app == a);
    let mut out = vec![];
    let mut ticked = false;
    for &a in &fit {
        for x in spaces.iter().filter(|x| has(x, a)) {
            let checked = !ticked && last == Some(a);
            ticked |= checked;
            out.push(Row::App {
                app: a,
                ws: Some(x.id.clone()),
                label: format!("{} · in {}", app_name(a), x.name),
                hint: None,
                checked,
            });
        }
    }
    let fresh: Vec<AppId> = fit.iter().copied().filter(|&a| !spaces.iter().any(|x| has(x, a))).collect();
    if !out.is_empty() && !fresh.is_empty() {
        out.push(Row::Sep);
    }
    for a in fresh {
        out.push(Row::App {
            app: a,
            ws: None,
            label: format!("{} · in a new space", app_name(a)),
            hint: app(a).map(|c| c.summary.clone()),
            checked: false,
        });
    }
    out
}

/// What a double-click opens a file with: the app that last opened its
/// kind, in the first wadspace here that has it; none (a viewer shows it).
pub fn remembered_for(file: &Opening, here: &[Wadspace], last: &Remembered) -> Option<(AppId, String)> {
    let a = last.get(&file.mime())?;
    let mut spaces: Vec<&Wadspace> = here.iter().filter(|x| !x.is_trashed()).collect();
    spaces.sort_by_key(|x| x.order);
    spaces.into_iter().find(|x| x.apps.iter().any(|y| y.app == a)).map(|x| (a, x.id.clone()))
}

/// The app that last opened each kind of file (by MIME type), this session's
/// (`~/.config/wados/open-with.json`, which the shell and Files share).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remembered {
    #[serde(default)]
    pub by_mime: BTreeMap<String, AppId>,
}

impl Remembered {
    /// Read from its JSON (anything else: nothing remembered).
    pub fn parse(json: &str) -> Self {
        serde_json::from_str(json).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn get(&self, mime: &str) -> Option<AppId> {
        self.by_mime.get(mime).copied()
    }

    /// `app` opened a file of `mime`.
    pub fn set(&mut self, mime: &str, app: AppId) {
        self.by_mime.insert(mime.to_string(), app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::blank;

    fn ws(id: &str, name: &str, order: i64, apps: &[AppId]) -> Wadspace {
        let mut w = blank(id, name, "m-here", 0, order);
        for &a in apps {
            w.apps.push(crate::types::AppIn::new(a));
        }
        w
    }

    fn labels(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                Row::App { label, checked: true, .. } => format!("✓ {label}"),
                Row::App { label, .. } => label.clone(),
                Row::Sep => "—".into(),
                Row::Nothing => "No app opens it".into(),
            })
            .collect()
    }

    #[test]
    fn staged_apps_come_first_then_a_new_space_for_each_other() {
        let here = [ws("art", "Art", 1, &[AppId::Krita]), ws("code", "Code", 0, &[AppId::Vscode, AppId::Krita])];
        let png = Opening::new("cover.png", Some("image/png"), false);
        let r = rows(std::slice::from_ref(&png), &here, Some(AppId::Krita));
        assert_eq!(labels(&r), ["✓ Krita · in Code", "Krita · in Art", "—", "LibreSprite · in a new space"]);
        assert_eq!(r[1].id(), "open-with:krita:art");
        assert_eq!(r[3].id(), "open-with:libresprite:new");
        // A text file: VS Code, in the space that has it; nothing else opens it.
        let txt = Opening::new("notes.txt", None, false);
        assert_eq!(labels(&rows(std::slice::from_ref(&txt), &here, None)), ["VS Code · in Code"]);
        // Two kinds at once: only what opens both.
        assert_eq!(labels(&rows(&[png, txt], &here, None)), ["No app opens it"]);
        assert_eq!(labels(&rows(&[Opening::new("a.bin", None, false)], &here, None)), ["No app opens it"]);
        assert!(rows(&[], &here, None).is_empty());
    }

    #[test]
    fn a_double_click_opens_with_what_opened_its_kind_last() {
        let here = [ws("art", "Art", 0, &[AppId::Krita])];
        let png = Opening::new("cover.png", None, false);
        let mut last = Remembered::default();
        assert_eq!(remembered_for(&png, &here, &last), None);
        last.set(&png.mime(), AppId::Krita);
        assert_eq!(remembered_for(&png, &here, &last), Some((AppId::Krita, "art".into())));
        // Remembered, but no space has it any more: a viewer, then.
        last.set("text/plain", AppId::Vscode);
        assert_eq!(remembered_for(&Opening::new("n.txt", None, false), &here, &last), None);
        let again = Remembered::parse(&last.to_json());
        assert_eq!(again, last);
        assert_eq!(Remembered::parse("not json"), Remembered::default());
    }
}
