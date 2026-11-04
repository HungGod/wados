//! Keeping files in a wadspace from the desktop (the spike's `shell.keepIn`,
//! "Keep it in {name}"): files let go on a wadspace's icon (the desktop's
//! own, or another app's), or pasted on it (its menu's "Paste into it", or
//! Ctrl+V with it alone selected), are kept in it with `space.keepFiles`. A
//! folder is mounted as it opens, a file copied in. A picture among them
//! asks first: "Use as its look" (its wallpaper, not kept as a file) or
//! "Keep as a file".
//!
//! Paths in your home are kept as `~/…`, as projects name them.

use std::path::{Path, PathBuf};

use gtk::{gdk, glib, prelude::*};
use wad_model::Wallpaper;
use wad_model::edit::{FileRef, Op};
use wad_proto::Level;

use super::Desktop;
use super::grid;
use super::menu::{Entry, Item as MenuItem};

/// A path as a project names it: `~/…` in the home, else as it is.
pub fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.to_string_lossy()),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// A picture a wadspace's look can be (what its wallpaper takes).
pub fn is_picture(path: &Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif" | "svg") && !path.is_dir()
}

/// The edit keeping `paths` in `ws` (this machine's, `machine`): each a
/// project (a folder mounted, a file copied in), and `look` its wallpaper
/// instead of a file. None when there's nothing to keep.
pub fn keep_op(ws: &str, machine: &str, home: &Path, paths: &[PathBuf], look: Option<&Path>) -> Option<Op> {
    let files: Vec<FileRef> = paths
        .iter()
        .filter(|p| Some(p.as_path()) != look)
        .filter_map(|p| {
            let name = p.file_name()?.to_string_lossy().into_owned();
            Some(FileRef { machine: machine.into(), path: tilde(p, home), name, folder: p.is_dir() })
        })
        .collect();
    let wallpaper = look.map(|l| Wallpaper::image(tilde(l, home)));
    (!files.is_empty() || wallpaper.is_some()).then(|| Op::KeepFiles { ws: ws.into(), files, wallpaper })
}

/// What a keep's toast says.
pub fn kept(name: &str, paths: &[PathBuf], look: Option<&Path>) -> String {
    let names: Vec<String> = paths
        .iter()
        .filter(|p| Some(p.as_path()) != look)
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    match (names.as_slice(), look.is_some()) {
        ([], true) => format!("{name} has a new look"),
        (n, false) => format!("{} kept in {name}", n.join(", ")),
        (n, true) => format!("{} kept in {name}, and a new look", n.join(", ")),
    }
}

impl Desktop {
    /// Keeps `paths` in the wadspace `ws` (its icon's key `ws:<id>`, or its
    /// id); a picture among them asks first, by its icon.
    pub(super) fn keep_in(&self, ws: &str, paths: Vec<PathBuf>) {
        let id = ws.strip_prefix("ws:").unwrap_or(ws).to_string();
        let Some(x) = self.shell.store.wadspace(&id) else { return };
        if paths.is_empty() {
            return;
        }
        if x.machine != self.shell.store.machine_id() {
            let on = self.shell.store.machine_name(&x.machine).unwrap_or_else(|| "another machine".into());
            self.toast(Level::Info, &format!("It lives on {on}: copy it here to change it"));
            return;
        }
        let Some(pic) = paths.iter().find(|p| is_picture(p)).cloned() else {
            self.send_keep(&id, &paths, None);
            return;
        };
        let (x0, y0) = match self.s.borrow().spots.get(&format!("ws:{id}")).copied() {
            Some(spot) => {
                let (x, y) = spot.origin();
                (x + grid::ICON_W / 2.0, y + 40.0)
            }
            None => (0.0, 0.0),
        };
        let name = pic.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "The picture".into());
        let (i1, p1, l1) = (id.clone(), paths.clone(), pic.clone());
        let (i2, p2) = (id.clone(), paths.clone());
        let entries = vec![
            Entry::Heading(name),
            MenuItem::new(
                "pic-wallpaper",
                "Use as its look",
                "image",
                self.on(move |m| m.send_keep(&i1, &p1, Some(&l1))),
            )
            .hint("Its wallpaper (a gamespace's cover)")
            .entry(),
            MenuItem::new("pic-file", "Keep as a file", "file-image", self.on(move |m| m.send_keep(&i2, &p2, None)))
                .hint("On its desktop, with the rest")
                .entry(),
        ];
        self.show_menu(x0, y0, entries, Some(format!("ws:{id}")), true);
    }

    fn send_keep(&self, ws: &str, paths: &[PathBuf], look: Option<&Path>) {
        let home = glib::home_dir();
        let Some(op) = keep_op(ws, &self.shell.store.machine_id(), &home, paths, look) else { return };
        let name = self.shell.store.wadspace(ws).map(|x| x.name).unwrap_or_else(|| "it".into());
        let said = kept(&name, paths, look);
        let w = self.weak();
        self.shell.op(op, move |_| {
            if let Some(m) = w.upgrade() {
                m.toast(Level::Ok, &said);
            }
        });
    }

    /// The clipboard's files, kept in `ws`.
    pub(super) fn keep_pasted(&self, ws: &str) {
        let (w, clip, ws) = (self.weak(), self.board.clipboard(), ws.to_string());
        glib::spawn_future_local(async move {
            let got = clip.read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT).await;
            let Some(m) = w.upgrade() else { return };
            let paths = got.ok().and_then(|v| v.get::<gdk::FileList>().ok()).map(|l| super::files::local(&l.files()));
            match paths.filter(|p| !p.is_empty()) {
                Some(paths) => m.keep_in(&ws, paths),
                None => m.toast(Level::Info, "There are no files on the clipboard"),
            }
        });
    }

    /// A wadspace's menu's "Paste into it" (the clipboard's files kept in it).
    pub(super) fn keep_paste_item(&self, ws: &str) -> Entry {
        let id = ws.to_string();
        MenuItem::new("paste-into", "Paste into it", "clipboard-paste", self.on(move |m| m.keep_pasted(&id)))
            .hint("The files on the clipboard, kept in it")
            .off((!self.can_paste()).then_some("There are no files on the clipboard"))
            .entry()
    }

    /// Ctrl+V: into the one wadspace selected, else onto the desktop.
    pub(super) fn paste_here(&self) {
        let sel = self.s.borrow().sel.clone();
        match sel.as_slice() {
            [one] if one.starts_with("ws:") => self.keep_pasted(one),
            _ => self.paste(self.desk_dir.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whats_kept_is_named_from_the_home() {
        let home = Path::new("/var/home/sam");
        assert_eq!(tilde(Path::new("/var/home/sam/Desktop/Thesis"), home), "~/Desktop/Thesis");
        assert_eq!(tilde(home, home), "~");
        assert_eq!(tilde(Path::new("/run/media/sam/USB/a"), home), "/run/media/sam/USB/a");
        assert!(is_picture(Path::new("/x/Beach.JPG")) && is_picture(Path::new("a.webp")));
        assert!(!is_picture(Path::new("notes.txt")) && !is_picture(Path::new("noext")));
    }

    #[test]
    fn a_keep_is_one_edit_and_a_look_isnt_a_file() {
        let home = Path::new("/nonexistent-home");
        let paths = vec![home.join("Desktop/notes.txt"), home.join("Pictures/beach.jpg")];
        let op = keep_op("studio", "m-here", home, &paths, None).unwrap();
        let Op::KeepFiles { ws, files, wallpaper } = op else { panic!() };
        assert_eq!(ws, "studio");
        assert_eq!(wallpaper, None);
        assert_eq!(
            files,
            [
                FileRef {
                    machine: "m-here".into(),
                    path: "~/Desktop/notes.txt".into(),
                    name: "notes.txt".into(),
                    folder: false
                },
                FileRef {
                    machine: "m-here".into(),
                    path: "~/Pictures/beach.jpg".into(),
                    name: "beach.jpg".into(),
                    folder: false
                },
            ]
        );
        // Used as its look: its wallpaper, not a file.
        let look = home.join("Pictures/beach.jpg");
        let Some(Op::KeepFiles { files, wallpaper, .. }) = keep_op("studio", "m-here", home, &paths, Some(&look))
        else {
            panic!()
        };
        assert_eq!(files.len(), 1);
        assert_eq!(wallpaper, Some(Wallpaper::image("~/Pictures/beach.jpg")));
        assert_eq!(kept("Studio", &paths, Some(&look)), "notes.txt kept in Studio, and a new look");
        assert_eq!(kept("Studio", &paths[1..], Some(&look)), "Studio has a new look");
        assert_eq!(kept("Studio", &paths, None), "notes.txt, beach.jpg kept in Studio");
        assert!(keep_op("studio", "m-here", home, &[], None).is_none());
    }
}
