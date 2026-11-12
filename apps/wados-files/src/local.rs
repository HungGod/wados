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

/// What's in `dir`: folders first, then by name. Hidden ones (dot-files,
/// a `.hidden` file's names) and backups (`name~`) only with `hidden`.
pub fn list(dir: &Path, hidden: bool) -> Result<Vec<Item>, String> {
    let f = gio::File::for_path(dir);
    let en = f.enumerate_children(ATTRS, gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE).map_err(|e| why(&e))?;
    let mut out = Vec::new();
    // One thing that can't be read is left out, not the rest after it.
    let mut errors = 0;
    loop {
        let info = match en.next_file(gio::Cancellable::NONE) {
            Ok(Some(i)) => i,
            Ok(None) => break,
            Err(_) if errors < 64 => {
                errors += 1;
                continue;
            }
            Err(_) => break,
        };
        if !hidden && (info.is_hidden() || info.is_backup()) {
            continue;
        }
        let name = info.name().to_string_lossy().into_owned();
        let path = dir.join(&name);
        let folder = info.file_type() == gio::FileType::Directory;
        let ct = info.content_type().map(|c| c.to_string()).unwrap_or_default();
        let ct = gio::content_type_get_mime_type(&ct).map(|m| m.to_string()).unwrap_or(ct);
        let count = folder.then(|| {
            std::fs::read_dir(&path)
                .map(|d| d.flatten().filter(|e| hidden || !e.file_name().to_string_lossy().starts_with('.')).count())
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

// ------------------------------------------------------------ archives

/// The archives Files extracts, by their endings: tar's (any compression
/// tar knows), and what 7-Zip opens (7z, zip, rar).
const TARS: [&str; 10] =
    [".tar", ".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2", ".tbz2", ".tbz", ".tar.zst", ".tzst"];
const SEVENS: [&str; 3] = [".7z", ".zip", ".rar"];

/// The ending that makes `name` an archive (none: it isn't one).
fn archive_ending(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    TARS.iter()
        .chain(SEVENS.iter())
        .copied()
        .filter(|e| lower.ends_with(e) && lower.len() > e.len())
        .max_by_key(|e| e.len())
}

/// Whether Files can extract `name`.
pub fn is_archive(name: &str) -> bool {
    archive_ending(name).is_some()
}

/// The folder an archive extracts into, beside it: its name without its
/// ending ("Mario Kart.7z" → "Mario Kart"), or "… (2)" when that's taken.
pub fn extract_folder(dir: &Path, name: &str) -> String {
    let stem = archive_ending(name).map_or(name, |e| &name[..name.len() - e.len()]);
    let stem = stem.trim();
    free_name(dir, if stem.is_empty() { "Extracted" } else { stem })
}

/// Extracts `archive` into a new folder beside it: the folder's name. Off
/// the main thread (an archive can be gigabytes). A half-done folder is
/// taken away when it fails.
pub fn extract(archive: &Path) -> Result<String, String> {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = archive.parent().ok_or("It isn't in a folder")?;
    let ending = archive_ending(&name).ok_or("It isn't an archive Files knows")?;
    let folder = extract_folder(dir, &name);
    let into = dir.join(&folder);
    std::fs::create_dir(&into).map_err(|e| e.to_string())?;
    let mut cmd = if TARS.contains(&ending) {
        let mut c = std::process::Command::new("tar");
        c.arg("-xf").arg(archive).arg("-C").arg(&into).arg("--no-same-owner");
        c
    } else {
        let mut c = std::process::Command::new("7z");
        // -y: yes to its questions (there are none in a new folder); -bso0 -bsp0: quiet.
        c.args(["x", "-y", "-bso0", "-bsp0"]).arg(format!("-o{}", into.display())).arg("--").arg(archive);
        c
    };
    let out = cmd.stdin(std::process::Stdio::null()).output();
    let fail = |why: String| {
        let _ = std::fs::remove_dir_all(&into);
        Err(why)
    };
    match out {
        Ok(o) if o.status.success() => Ok(folder),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            let line = err.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("it stopped").to_string();
            fail(format!("Couldn’t extract {name}: {line}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fail(format!("Couldn’t extract {name}: {} isn’t on this machine", cmd.get_program().to_string_lossy()))
        }
        Err(e) => fail(format!("Couldn’t extract {name}: {e}")),
    }
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
    fn archives_by_their_endings() {
        assert!(is_archive("Mario Kart.7z") && is_archive("a.ZIP") && is_archive("src.tar.gz") && is_archive("x.rar"));
        assert!(!is_archive("game.rvz") && !is_archive("disc.iso") && !is_archive(".7z") && !is_archive("notes.txt"));
        let d = tempfile::tempdir().unwrap();
        assert_eq!(extract_folder(d.path(), "Mario Kart - Double Dash!! (USA).7z"), "Mario Kart - Double Dash!! (USA)");
        assert_eq!(extract_folder(d.path(), "src.tar.gz"), "src");
        std::fs::create_dir(d.path().join("src")).unwrap();
        assert_eq!(extract_folder(d.path(), "src.tar.gz"), "src (2)");
    }

    #[test]
    fn a_tar_extracts_beside_itself() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("in")).unwrap();
        std::fs::write(d.path().join("in/a.txt"), "hi").unwrap();
        let ok = std::process::Command::new("tar")
            .arg("-czf")
            .arg(d.path().join("pack.tar.gz"))
            .arg("-C")
            .arg(d.path().join("in"))
            .arg("a.txt")
            .status()
            .unwrap()
            .success();
        assert!(ok);
        assert_eq!(extract(&d.path().join("pack.tar.gz")).unwrap(), "pack");
        assert_eq!(std::fs::read_to_string(d.path().join("pack/a.txt")).unwrap(), "hi");
        // A broken one leaves nothing behind.
        std::fs::write(d.path().join("bad.zip"), "not a zip").unwrap();
        assert!(extract(&d.path().join("bad.zip")).is_err());
        assert!(!d.path().join("bad").exists());
    }

    #[test]
    fn hidden_things_only_when_asked_for() {
        let d = tempfile::tempdir().unwrap();
        for dir in ["Documents", ".config", "Documents/.git"] {
            std::fs::create_dir_all(d.path().join(dir)).unwrap();
        }
        std::fs::write(d.path().join("notes.txt~"), "").unwrap();
        std::fs::write(d.path().join("notes.txt"), "").unwrap();
        let names = |hidden| -> Vec<(String, Option<usize>)> {
            list(d.path(), hidden).unwrap().into_iter().map(|i| (i.name, i.count)).collect()
        };
        assert_eq!(names(false), [("Documents".into(), Some(0)), ("notes.txt".into(), None)]);
        assert_eq!(
            names(true),
            [
                (".config".into(), Some(0)),
                ("Documents".into(), Some(1)),
                ("notes.txt".into(), None),
                ("notes.txt~".into(), None),
            ]
        );
    }

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
