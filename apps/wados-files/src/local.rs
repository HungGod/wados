//! This machine's files, for Files: a folder's things (through GIO), new
//! folders and names, copies and moves, and the Trash: the freedesktop.org
//! one in `$XDG_DATA_HOME/Trash` (`files/` and `info/*.trashinfo`), which
//! GIO's `trash` fills and which Files restores from, deletes from for
//! good and empties itself (no gvfs needed). The slow parts are plain
//! functions to run off the main thread.

use std::path::{Path, PathBuf};

use gtk::{gio, glib, prelude::*};

/// One thing in a folder here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub display: String,
    pub path: PathBuf,
    pub folder: bool,
    pub content_type: String,
    pub size: u64,
    /// When it last changed (seconds since the epoch).
    pub modified: i64,
    /// A folder's: how many things are in it.
    pub count: Option<usize>,
}

const ATTRS: &str = "standard::name,standard::display-name,standard::type,standard::content-type,standard::size,standard::is-hidden,standard::is-backup,time::modified";

/// What's in `dir` (not hidden ones or backups): folders first, then by name.
pub fn list(dir: &Path) -> Result<Vec<Item>, String> {
    let f = gio::File::for_path(dir);
    let en = f.enumerate_children(ATTRS, gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE).map_err(|e| why(&e))?;
    let mut out = Vec::new();
    while let Ok(Some(info)) = en.next_file(gio::Cancellable::NONE) {
        if info.is_hidden() || info.is_backup() {
            continue;
        }
        let name = info.name().to_string_lossy().into_owned();
        let path = dir.join(&name);
        let folder = info.file_type() == gio::FileType::Directory;
        let ct = info.content_type().map(|c| c.to_string()).unwrap_or_default();
        let ct = gio::content_type_get_mime_type(&ct).map(|m| m.to_string()).unwrap_or(ct);
        let count = folder.then(|| {
            std::fs::read_dir(&path)
                .map(|d| d.flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).count())
                .unwrap_or(0)
        });
        out.push(Item {
            display: info.display_name().to_string(),
            folder,
            content_type: ct,
            size: info.size().max(0) as u64,
            modified: info.attribute_uint64("time::modified") as i64,
            count,
            path,
            name,
        });
    }
    sort(&mut out);
    Ok(out)
}

pub fn sort(v: &mut [Item]) {
    v.sort_by(|a, b| {
        b.folder
            .cmp(&a.folder)
            .then_with(|| a.display.to_lowercase().cmp(&b.display.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Why something here didn't work, in words.
pub fn why(e: &glib::Error) -> String {
    if e.matches(gio::IOErrorEnum::NotFound) {
        "It isn't there (any more)".into()
    } else if e.matches(gio::IOErrorEnum::PermissionDenied) {
        "You can't look in there".into()
    } else {
        e.message().to_string()
    }
}

/// `wanted`, or (if `taken` says it is) "name (2).ext", "name (3).ext"…
pub fn unique_name(wanted: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(wanted) {
        return wanted.to_string();
    }
    let (stem, ext) = match wanted.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (wanted, String::new()),
    };
    (2..).map(|n| format!("{stem} ({n}){ext}")).find(|n| !taken(n)).unwrap_or_else(|| wanted.to_string())
}

/// A name in `dir` nothing has yet.
pub fn free_name(dir: &Path, wanted: &str) -> String {
    unique_name(wanted, |n| dir.join(n).symlink_metadata().is_ok())
}

/// A file name someone typed, if it can be one.
pub fn valid_name(name: &str) -> Option<String> {
    let n = name.trim();
    (!n.is_empty() && n != "." && n != ".." && !n.contains('/') && !n.contains('\0')).then(|| n.to_string())
}

/// A new folder in `dir` ("New folder", or "New folder (2)"…): its name.
pub fn new_folder(dir: &Path) -> Result<String, String> {
    let name = free_name(dir, "New folder");
    std::fs::create_dir(dir.join(&name)).map_err(|e| e.to_string())?;
    Ok(name)
}

/// `path` renamed `to` in its folder: its new path.
pub fn rename(path: &Path, to: &str) -> Result<PathBuf, String> {
    let to = valid_name(to).ok_or("That can't be a name")?;
    let dest = path.with_file_name(&to);
    if dest == path {
        return Ok(dest);
    }
    if dest.symlink_metadata().is_ok() {
        return Err(format!("There's a {to} here already"));
    }
    std::fs::rename(path, &dest).map_err(|e| e.to_string())?;
    Ok(dest)
}

// ------------------------------------------------------------ copies and moves

/// Copies of `srcs` in `dir`, each under a name nothing there has. The names they took.
pub fn copy_into(srcs: &[PathBuf], dir: &Path) -> Result<Vec<String>, String> {
    let mut made = Vec::new();
    for src in srcs {
        let Some(name) = src.file_name().and_then(|n| n.to_str()) else { continue };
        if dir.starts_with(src) {
            return Err(format!("{name} can't go inside itself"));
        }
        let to = free_name(dir, name);
        copy_tree(src, &dir.join(&to)).map_err(|e| format!("{name}: {e}"))?;
        made.push(to);
    }
    Ok(made)
}

/// `srcs` moved into `dir` (renamed if they can be, else copied and removed).
pub fn move_into(srcs: &[PathBuf], dir: &Path) -> Result<Vec<String>, String> {
    let mut made = Vec::new();
    for src in srcs {
        let Some(name) = src.file_name().and_then(|n| n.to_str()) else { continue };
        if src.parent() == Some(dir) {
            continue;
        }
        if dir.starts_with(src) {
            return Err(format!("{name} can't go inside itself"));
        }
        let to = free_name(dir, name);
        let dest = dir.join(&to);
        if std::fs::rename(src, &dest).is_err() {
            copy_tree(src, &dest).and_then(|_| remove(src)).map_err(|e| format!("{name}: {e}"))?;
        }
        made.push(to);
    }
    Ok(made)
}

fn copy_tree(src: &Path, dest: &Path) -> std::io::Result<()> {
    let meta = src.symlink_metadata()?;
    if meta.file_type().is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(src)?, dest)
    } else if meta.is_dir() {
        std::fs::create_dir(dest)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            copy_tree(&e.path(), &dest.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dest).map(|_| ())
    }
}

fn remove(p: &Path) -> std::io::Result<()> {
    if p.symlink_metadata()?.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }
}

// ------------------------------------------------------------ the Trash

/// The Trash's folder (`$XDG_DATA_HOME/Trash`).
pub fn trash_dir() -> PathBuf {
    glib::user_data_dir().join("Trash")
}

/// Something in the Trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trashed {
    /// Its name in `Trash/files` (and `Trash/info/<name>.trashinfo`).
    pub name: String,
    /// Its own name (where it came from's last part).
    pub display: String,
    /// Where it came from.
    pub from: PathBuf,
    /// When it went in (seconds since the epoch; 0 if it doesn't say).
    pub at: i64,
    pub folder: bool,
    pub size: u64,
    pub content_type: String,
}

/// A `.trashinfo`'s Path= and DeletionDate= (its local time, `2026-11-05T16:06:12`).
pub fn parse_info(text: &str) -> Option<(PathBuf, String)> {
    let mut path = None;
    let mut date = String::new();
    for l in text.lines() {
        if let Some(p) = l.strip_prefix("Path=") {
            path = Some(PathBuf::from(glib::Uri::unescape_string(p, None::<&str>)?.as_str()));
        } else if let Some(d) = l.strip_prefix("DeletionDate=") {
            date = d.trim().to_string();
        }
    }
    path.map(|p| (p, date))
}

fn when(date: &str) -> i64 {
    glib::DateTime::from_iso8601(date, Some(&glib::TimeZone::local())).map(|d| d.to_unix()).unwrap_or(0)
}

/// What's in the Trash, newest first.
pub fn trash_list() -> Vec<Trashed> {
    trash_list_in(&trash_dir())
}

pub fn trash_list_in(root: &Path) -> Vec<Trashed> {
    let mut out = Vec::new();
    let Ok(d) = std::fs::read_dir(root.join("files")) else { return out };
    for e in d.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let info = std::fs::read_to_string(root.join("info").join(format!("{name}.trashinfo"))).unwrap_or_default();
        let (from, date) = parse_info(&info).unwrap_or_else(|| (PathBuf::from(&name), String::new()));
        let meta = e.path().symlink_metadata().ok();
        let folder = meta.as_ref().is_some_and(|m| m.is_dir());
        let display = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| name.clone());
        let (ct, _) = gio::content_type_guess(Some(&display), None::<&[u8]>);
        let ct = if folder {
            "inode/directory".to_string()
        } else {
            gio::content_type_get_mime_type(&ct).map(|m| m.to_string()).unwrap_or_else(|| ct.to_string())
        };
        out.push(Trashed {
            display,
            from,
            at: when(&date),
            folder,
            size: meta.map(|m| m.len()).unwrap_or(0),
            content_type: ct,
            name,
        });
    }
    out.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.display.cmp(&b.display)));
    out
}

/// `paths` into the Trash (GIO's, which keeps where each came from): what
/// each is called there, for its Undo.
pub fn trash(paths: &[PathBuf]) -> Result<Vec<String>, String> {
    let before: std::collections::HashSet<String> = trash_list().into_iter().map(|t| t.name).collect();
    for p in paths {
        gio::File::for_path(p).trash(gio::Cancellable::NONE).map_err(|e| why(&e))?;
    }
    let wanted: Vec<&PathBuf> = paths.iter().collect();
    Ok(trash_list()
        .into_iter()
        .filter(|t| !before.contains(&t.name) && wanted.iter().any(|p| **p == t.from))
        .map(|t| t.name)
        .collect())
}

/// Back where it came from (under a free name, if that's taken now): its path.
pub fn restore(name: &str) -> Result<PathBuf, String> {
    restore_in(&trash_dir(), name)
}

pub fn restore_in(root: &Path, name: &str) -> Result<PathBuf, String> {
    let info = root.join("info").join(format!("{name}.trashinfo"));
    let (from, _) = std::fs::read_to_string(&info)
        .ok()
        .and_then(|t| parse_info(&t))
        .ok_or_else(|| format!("{name}: the Trash doesn't say where it came from"))?;
    let dir = from.parent().map(Path::to_path_buf).unwrap_or_else(glib::home_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let own = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string());
    let to = dir.join(free_name(&dir, &own));
    let src = root.join("files").join(name);
    if std::fs::rename(&src, &to).is_err() {
        copy_tree(&src, &to).and_then(|_| remove(&src)).map_err(|e| format!("{own}: {e}"))?;
    }
    let _ = std::fs::remove_file(info);
    Ok(to)
}

/// Gone for good.
pub fn delete_forever(name: &str) -> Result<(), String> {
    delete_in(&trash_dir(), name)
}

pub fn delete_in(root: &Path, name: &str) -> Result<(), String> {
    if name.contains('/') || name == ".." {
        return Err(format!("{name:?} isn't in the Trash"));
    }
    let f = root.join("files").join(name);
    if f.symlink_metadata().is_ok() {
        remove(&f).map_err(|e| format!("{name}: {e}"))?;
    }
    let _ = std::fs::remove_file(root.join("info").join(format!("{name}.trashinfo")));
    Ok(())
}

/// Everything in the Trash gone for good.
pub fn empty_trash() -> Result<(), String> {
    let root = trash_dir();
    for sub in ["files", "info", "expunged"] {
        let Ok(d) = std::fs::read_dir(root.join(sub)) else { continue };
        for e in d.flatten() {
            remove(&e.path()).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// ------------------------------------------------------------ words

/// "Folder", "PDF document", "PNG image" (the spike's `kindLabel`).
pub fn kind_label(name: &str, mime: &str, folder: bool) -> String {
    if folder {
        return "Folder".into();
    }
    let ext = name.rsplit_once('.').filter(|(s, _)| !s.is_empty()).map(|(_, e)| e.to_uppercase());
    if mime.starts_with("image/") {
        return ext.map_or_else(|| "Image".into(), |e| format!("{e} image"));
    }
    if mime.contains("pdf") {
        return "PDF document".into();
    }
    if mime.contains("markdown") {
        return "Markdown".into();
    }
    if mime.starts_with("text/") {
        return "Text".into();
    }
    if mime.contains("spreadsheet") {
        return "Spreadsheet".into();
    }
    if mime.contains("zip") {
        return "Zip archive".into();
    }
    if mime.contains("tar") {
        return "Tar archive".into();
    }
    ext.map_or_else(|| "File".into(), |e| format!("{e} file"))
}

/// Its glyph (a lucide name).
pub fn glyph(mime: &str, folder: bool) -> &'static str {
    if folder {
        "folder"
    } else if mime.starts_with("image/") {
        "image"
    } else if mime.starts_with("video/") {
        "film"
    } else if mime.starts_with("audio/") {
        "music"
    } else if mime.contains("zip") || mime.contains("tar") || mime.contains("compressed") {
        "file-archive"
    } else if mime.starts_with("text/") || mime.contains("pdf") || mime.contains("document") || mime.contains("json") {
        "file-text"
    } else {
        "file"
    }
}

/// A path as Files says it: `~/Pictures` in the home, else as it is.
pub fn pretty(p: &Path) -> String {
    let home = glib::home_dir();
    match p.strip_prefix(&home) {
        Ok(r) if r.as_os_str().is_empty() => "~".into(),
        Ok(r) => format!("~/{}", r.display()),
        Err(_) => p.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_are_free() {
        let taken = ["a.txt", "a (2).txt", "New folder"];
        let t = |n: &str| taken.contains(&n);
        assert_eq!(unique_name("b.txt", t), "b.txt");
        assert_eq!(unique_name("a.txt", t), "a (3).txt");
        assert_eq!(unique_name("New folder", t), "New folder (2)");
        assert_eq!(valid_name("  Notes "), Some("Notes".into()));
        assert_eq!(valid_name("a/b"), None);
        assert_eq!(valid_name(".."), None);
    }

    #[test]
    fn kinds_in_words() {
        assert_eq!(kind_label("Thesis", "", true), "Folder");
        assert_eq!(kind_label("cover.png", "image/png", false), "PNG image");
        assert_eq!(kind_label("deck.pdf", "application/pdf", false), "PDF document");
        assert_eq!(kind_label("notes.md", "text/markdown", false), "Markdown");
        assert_eq!(kind_label("a.txt", "text/plain", false), "Text");
        assert_eq!(kind_label("game.rvz", "application/octet-stream", false), "RVZ file");
        assert_eq!(kind_label("Makefile", "application/octet-stream", false), "File");
        assert_eq!(glyph("image/png", false), "image");
        assert_eq!(glyph("", true), "folder");
    }

    #[test]
    fn the_trash_restores_and_forgets() {
        let root = std::env::temp_dir().join(format!("wados-files-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (trash, home) = (root.join("Trash"), root.join("home"));
        std::fs::create_dir_all(trash.join("files")).unwrap();
        std::fs::create_dir_all(trash.join("info")).unwrap();
        std::fs::create_dir_all(home.join("Documents")).unwrap();
        let put = |name: &str, from: &Path, date: &str| {
            std::fs::write(trash.join("files").join(name), "x").unwrap();
            let p = glib::Uri::escape_string(&from.to_string_lossy(), Some("/"), false);
            std::fs::write(
                trash.join("info").join(format!("{name}.trashinfo")),
                format!("[Trash Info]\nPath={p}\nDeletionDate={date}\n"),
            )
            .unwrap();
        };
        put("my notes.txt", &home.join("Documents/my notes.txt"), "2026-11-05T10:00:00");
        put("cover.png", &home.join("Pictures/cover.png"), "2026-11-05T11:00:00");
        let l = trash_list_in(&trash);
        let names: Vec<&str> = l.iter().map(|t| t.display.as_str()).collect();
        assert_eq!(names, ["cover.png", "my notes.txt"], "newest first");
        assert_eq!(l[1].from, home.join("Documents/my notes.txt"));
        // Restored where it was; a folder that's gone is made again.
        std::fs::write(home.join("Documents/my notes.txt"), "new").unwrap();
        let back = restore_in(&trash, "my notes.txt").unwrap();
        assert_eq!(back, home.join("Documents/my notes (2).txt"));
        assert_eq!(restore_in(&trash, "cover.png").unwrap(), home.join("Pictures/cover.png"));
        assert!(trash_list_in(&trash).is_empty());
        put("old.txt", &home.join("old.txt"), "");
        delete_in(&trash, "old.txt").unwrap();
        assert!(trash_list_in(&trash).is_empty());
        assert!(delete_in(&trash, "../x").is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
