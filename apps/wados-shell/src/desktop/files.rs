//! The desktop's files: what's in `~/Desktop` (through GIO, as the spike's
//! `fs.nodes` under the desktop root), each with what its icon shows; the
//! Trash (`~/.local/share/Trash`, the freedesktop.org one that `trash:///`
//! shows); and copying, moving and naming them. The slow parts (copies,
//! emptying the Trash) are plain functions to run off the main thread.

use std::path::{Path, PathBuf};

use gtk::{gio, glib, prelude::*};

/// What a file's icon is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    /// A picture: its thumbnail.
    Image,
    /// Anything else: a page with a glyph and its extension.
    Doc,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Its name in the folder (its key: `file:<name>`).
    pub name: String,
    /// Its name as shown.
    pub display: String,
    pub path: PathBuf,
    pub kind: Kind,
    pub content_type: String,
    /// The thumbnailer's picture of it, if there's one.
    pub thumb: Option<PathBuf>,
    /// When it last changed (seconds), so a change redraws it.
    pub modified: u64,
}

impl Entry {
    /// What its icon shows: the picture itself (or its thumbnail).
    pub fn picture(&self) -> &Path {
        self.thumb.as_deref().unwrap_or(&self.path)
    }

    pub fn file(&self) -> gio::File {
        gio::File::for_path(&self.path)
    }
}

/// `~/Desktop` (made if it isn't there, so there's somewhere to drop things).
pub fn desktop_dir() -> PathBuf {
    let d = glib::home_dir().join("Desktop");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// The kind of icon for a content type.
pub fn kind_of(content_type: &str) -> Kind {
    if content_type == "inode/directory" {
        Kind::Folder
    } else if matches!(
        content_type,
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/bmp"
            | "image/svg+xml"
            | "image/tiff"
            | "image/avif"
    ) {
        Kind::Image
    } else {
        Kind::Doc
    }
}

/// A page's glyph (lucide), for its content type.
pub fn glyph(content_type: &str) -> &'static str {
    let m = content_type;
    if m.contains("pdf") {
        "file-text"
    } else if m.contains("zip") || m.contains("tar") || m.contains("compressed") || m.contains("archive") {
        "file-archive"
    } else if m.contains("spreadsheet") || m == "text/csv" {
        "grid-3x3"
    } else if m.starts_with("video/") {
        "film"
    } else if m.starts_with("audio/") {
        "music"
    } else if m.starts_with("image/") {
        "file-image"
    } else if m.starts_with("text/") || m.contains("markdown") || m.contains("document") {
        "file-text"
    } else {
        "file"
    }
}

/// The badge on a page: its extension, up to four letters ("PDF", "RVZ").
pub fn ext_label(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => ext.to_uppercase().chars().take(4).collect(),
        _ => String::new(),
    }
}

/// The desktop's order for files: folders first, then by name.
pub fn sort(v: &mut [Entry]) {
    v.sort_by(|a, b| {
        (b.kind == Kind::Folder)
            .cmp(&(a.kind == Kind::Folder))
            .then_with(|| a.display.to_lowercase().cmp(&b.display.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// What's in `dir` (not hidden ones or backups), in the desktop's order.
pub fn list(dir: &Path) -> Vec<Entry> {
    let f = gio::File::for_path(dir);
    let attrs = "standard::name,standard::display-name,standard::content-type,standard::is-hidden,standard::is-backup,time::modified,thumbnail::path";
    let Ok(en) = f.enumerate_children(attrs, gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    while let Ok(Some(info)) = en.next_file(gio::Cancellable::NONE) {
        if info.is_hidden() || info.is_backup() {
            continue;
        }
        let name = info.name().to_string_lossy().into_owned();
        let ct = info.content_type().map(|c| c.to_string()).unwrap_or_default();
        let ct = gio::content_type_get_mime_type(&ct).map(|m| m.to_string()).unwrap_or(ct);
        let thumb = info.attribute_byte_string("thumbnail::path").map(PathBuf::from).filter(|p| p.is_file());
        out.push(Entry {
            path: dir.join(&name),
            display: info.display_name().to_string(),
            kind: kind_of(&ct),
            content_type: ct,
            thumb,
            modified: info.attribute_uint64("time::modified"),
            name,
        });
    }
    sort(&mut out);
    out
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

// ------------------------------------------------------------ the Trash

/// The Trash's folder (`$XDG_DATA_HOME/Trash`).
pub fn trash_dir() -> PathBuf {
    glib::user_data_dir().join("Trash")
}

/// How many things are in the Trash.
pub fn trash_count() -> usize {
    std::fs::read_dir(trash_dir().join("files")).map(|d| d.count()).unwrap_or(0)
}

/// Into the Trash (GIO's, which keeps where each came from).
pub fn trash(paths: &[PathBuf]) -> Result<(), String> {
    for p in paths {
        gio::File::for_path(p).trash(gio::Cancellable::NONE).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Everything in the Trash gone for good (slow: off the main thread).
pub fn empty_trash() -> std::io::Result<()> {
    let root = trash_dir();
    for sub in ["files", "info", "expunged"] {
        let Ok(d) = std::fs::read_dir(root.join(sub)) else { continue };
        for e in d.flatten() {
            remove(&e.path())?;
        }
    }
    Ok(())
}

fn remove(p: &Path) -> std::io::Result<()> {
    if p.symlink_metadata()?.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }
}

// ------------------------------------------------------------ copies and moves

/// Copies of `srcs` in `dir`, each under a name nothing there has (slow: off
/// the main thread). The names they took.
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

/// The local paths among `files` (a drop or the clipboard); others are skipped.
pub fn local(files: &[gio::File]) -> Vec<PathBuf> {
    files.iter().filter_map(|f| f.path()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: Kind) -> Entry {
        Entry {
            name: name.into(),
            display: name.into(),
            path: PathBuf::from(name),
            kind,
            content_type: String::new(),
            thumb: None,
            modified: 0,
        }
    }

    #[test]
    fn folders_first_then_by_name() {
        let mut v = vec![
            entry("zeta.txt", Kind::Doc),
            entry("Thesis", Kind::Folder),
            entry("alpha.png", Kind::Image),
            entry("Beta.pdf", Kind::Doc),
            entry("Archive", Kind::Folder),
        ];
        sort(&mut v);
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Archive", "Thesis", "alpha.png", "Beta.pdf", "zeta.txt"]);
    }

    #[test]
    fn kinds_glyphs_and_badges() {
        assert_eq!(kind_of("inode/directory"), Kind::Folder);
        assert_eq!(kind_of("image/png"), Kind::Image);
        assert_eq!(kind_of("application/pdf"), Kind::Doc);
        assert_eq!(glyph("application/pdf"), "file-text");
        assert_eq!(glyph("application/zip"), "file-archive");
        assert_eq!(glyph("text/plain"), "file-text");
        assert_eq!(glyph("application/vnd.oasis.opendocument.spreadsheet"), "grid-3x3");
        assert_eq!(glyph("application/octet-stream"), "file");
        assert_eq!(ext_label("Pitch deck.pdf"), "PDF");
        assert_eq!(ext_label("super-smash-bros-melee.rvz"), "RVZ");
        assert_eq!(ext_label("archive.tar.gz"), "GZ");
        assert_eq!(ext_label("notes.markdown"), "MARK");
        assert_eq!(ext_label("Makefile"), "");
        assert_eq!(ext_label(".bashrc"), "");
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
        assert_eq!(valid_name("  "), None);
    }

    #[test]
    fn copies_and_moves_on_disk() {
        let root = std::env::temp_dir().join(format!("wados-desk-files-{}", std::process::id()));
        let (src, dest) = (root.join("src"), root.join("dest"));
        std::fs::create_dir_all(src.join("folder/inner")).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(src.join("a.txt"), "a").unwrap();
        std::fs::write(src.join("folder/inner/b.txt"), "b").unwrap();
        std::fs::write(dest.join("a.txt"), "already").unwrap();
        let made = copy_into(&[src.join("a.txt"), src.join("folder")], &dest).unwrap();
        assert_eq!(made, ["a (2).txt", "folder"]);
        assert_eq!(std::fs::read_to_string(dest.join("folder/inner/b.txt")).unwrap(), "b");
        assert!(src.join("a.txt").exists());
        // Into itself: no.
        assert!(copy_into(&[src.join("folder")], &src.join("folder/inner")).is_err());
        let moved = move_into(&[src.join("a.txt")], &dest).unwrap();
        assert_eq!(moved, ["a (3).txt"]);
        assert!(!src.join("a.txt").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
