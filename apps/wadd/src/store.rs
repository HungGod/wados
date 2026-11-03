//! The machine's library on disk, under `state_dir`:
//!
//! - `wadspaces/<id>.json`: each wadspace's document (version 6), on its own,
//!   so one edit rewrites one file;
//! - `library.json`: the rest (tags, keys, the browser's web apps, Steam
//!   libraries, discs, the id counter).
//!
//! Every write is atomic (a temporary file, fsynced, renamed over the old
//! one, the folder fsynced): a crash leaves the old file or the new, never
//! half of one. A file that won't parse is set aside (`.bad`), not lost.

use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wad_model::Wadspace;
use wad_model::edit::Library;

pub struct Store {
    dir: PathBuf,
}

/// library.json: everything but the wadspaces.
#[derive(Serialize, Deserialize, Default)]
struct Rest {
    #[serde(flatten)]
    lib: Library,
}

impl Store {
    pub fn new(state_dir: &Path) -> std::io::Result<Self> {
        let dir = state_dir.to_path_buf();
        for d in [dir.clone(), dir.join("wadspaces")] {
            std::fs::create_dir_all(&d)?;
            std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self { dir })
    }

    fn doc_path(&self, id: &str) -> Option<PathBuf> {
        safe_id(id).then(|| self.dir.join("wadspaces").join(format!("{id}.json")))
    }

    /// The library as it was saved (an empty one the first time).
    pub fn load(&self) -> std::io::Result<Library> {
        let mut lib = match std::fs::read(self.dir.join("library.json")) {
            Ok(b) => match serde_json::from_slice::<Rest>(&b) {
                Ok(r) => r.lib,
                Err(e) => {
                    tracing::error!("library.json: {e}: set aside, starting from an empty library");
                    set_aside(&self.dir.join("library.json"));
                    Library::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Library::default(),
            Err(e) => return Err(e),
        };
        lib.wadspaces.clear();
        let mut entries: Vec<_> = std::fs::read_dir(self.dir.join("wadspaces"))?.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read(&p)
                .map_err(|e| e.to_string())
                .and_then(|b| serde_json::from_slice::<Wadspace>(&b).map_err(|e| e.to_string()))
            {
                Ok(w) if self.doc_path(&w.id).as_deref() == Some(p.as_path()) => lib.wadspaces.push(w),
                Ok(w) => {
                    tracing::error!("{}: holds {:?}, not its own file's: set aside", p.display(), w.id);
                    set_aside(&p);
                }
                Err(e) => {
                    tracing::error!("{}: {e}: set aside", p.display());
                    set_aside(&p);
                }
            }
        }
        Ok(lib)
    }

    /// Saves one wadspace's document.
    pub fn save_wadspace(&self, w: &Wadspace) -> std::io::Result<()> {
        let p = self.doc_path(&w.id).ok_or_else(|| std::io::Error::other(format!("not an id: {:?}", w.id)))?;
        write_atomic(&p, &serde_json::to_vec_pretty(w).map_err(std::io::Error::other)?)
    }

    /// Deletes one wadspace's document (gone for good).
    pub fn delete_wadspace(&self, id: &str) -> std::io::Result<()> {
        let Some(p) = self.doc_path(id) else { return Ok(()) };
        match std::fs::remove_file(&p) {
            Ok(()) => sync_dir(p.parent().unwrap_or(&self.dir)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Saves everything but the wadspaces.
    pub fn save_rest(&self, lib: &Library) -> std::io::Result<()> {
        let rest = Rest { lib: Library { wadspaces: vec![], ..lib.clone() } };
        write_atomic(&self.dir.join("library.json"), &serde_json::to_vec_pretty(&rest).map_err(std::io::Error::other)?)
    }
}

/// An id that can be a file's name (a wadspace's document, its build record, its folders).
pub fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !id.starts_with('.')
}

/// `bytes` as `path`, all or nothing.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name().and_then(|n| n.to_str()).ok_or_else(|| std::io::Error::other("no name"))?
    ));
    {
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    sync_dir(dir)
}

/// As `write_atomic`, making its folder first.
pub fn write_atomic_mkdir(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_atomic(path, bytes)
}

fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

fn set_aside(p: &Path) {
    let to = p.with_extension("json.bad");
    if let Err(e) = std::fs::rename(p, &to) {
        tracing::error!("{}: couldn't set it aside: {e}", p.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starter() -> Wadspace {
        wad_model::starter::starter_wadspace("m-test", 1_000)
    }

    #[test]
    fn a_library_comes_back_as_it_was_saved() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path()).unwrap();
        let mut lib = Library::default();
        lib.wadspaces.push(starter());
        lib.seq = 41;
        store.save_rest(&lib).unwrap();
        for w in &lib.wadspaces {
            store.save_wadspace(w).unwrap();
        }
        let back = store.load().unwrap();
        assert_eq!(back.wadspaces, lib.wadspaces);
        assert_eq!(back.seq, 41);
        // library.json holds no wadspaces of its own.
        let rest = std::fs::read_to_string(dir.path().join("library.json")).unwrap();
        assert!(!rest.contains("\"wadspaces\": [\n"));
    }

    #[test]
    fn a_broken_file_is_set_aside_and_the_rest_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path()).unwrap();
        store.save_wadspace(&starter()).unwrap();
        std::fs::write(dir.path().join("wadspaces/broken.json"), b"{ not json").unwrap();
        let lib = store.load().unwrap();
        assert_eq!(lib.wadspaces.len(), 1);
        assert!(dir.path().join("wadspaces/broken.json.bad").exists());
    }

    #[test]
    fn ids_that_arent_file_names_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path()).unwrap();
        let mut w = starter();
        w.id = "../escape".into();
        assert!(store.save_wadspace(&w).is_err());
        assert!(!dir.path().join("escape.json").exists());
    }
}
