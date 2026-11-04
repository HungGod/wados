//! Files copied into wadspaces (v1's copies.rs). A wadspace never works in
//! a copy wadd keeps: wadd makes one of the files here, at
//! `<state_dir>/copies/<wadspace>/<project>/` (`files/`, and `done` once
//! it's whole), and the wadspace copies it into its own home the first time
//! it starts (the image's `wadspaces-projects --sync`), from a read-only
//! mount of that folder (wad-gen's units: `/run/wadspaces-copies/<project>`).
//!
//! A copy is made once: from this machine's own file or folder
//! ([`Copies::copy_local`], `cp -a`, reflinked where the disk can), or from
//! another machine's over Taildrive ([`Copies::fetch`], WebDAV). It's made
//! beside its place (`.part-<project>`), counted, given to the projects
//! user (abc) and put in place with its `done`, so a wadspace never sees
//! half of one. There must be room for it and a margin.
//!
//! A copy of one file says so in its `done` (`file`: its name in `files/`):
//! the wadspace puts that file at `~/Desktop/<name>`, not a folder holding it.
//!
//! Keyed by wadspace and project: a project's id is only its wadspace's
//! (a wadspace copied from another machine keeps that machine's ids).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::backend::Res;
use crate::taildrive::{Dav, DavError, Kind, decode};

/// Room left on the disk after a copy.
pub const MARGIN_BYTES: u64 = 1 << 30;
/// Files in a copy from another machine, at most.
pub const MAX_FILES: usize = 100_000;

/// What a copy is: `done`'s contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Made {
    pub files: u64,
    pub bytes: u64,
    /// Where it came from ('<machine>:<path>'): a project of that id from
    /// somewhere else is another copy.
    pub from: String,
    /// A copy of one file: its name in `files/` (none: a folder's contents).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

pub struct Copies {
    root: PathBuf,
    /// Who owns a copy when wadd is root (abc's 1000), so a wadspace can read it.
    owner: Option<u32>,
}

fn io(p: &Path, e: std::io::Error) -> String {
    format!("{}: {e}", p.display())
}

impl Copies {
    pub fn new(state_dir: &Path, owner: Option<u32>) -> Self {
        Self { root: state_dir.join("copies"), owner }
    }

    /// `<state_dir>/copies/<ws>/<project>`: what a wadspace sees at `/run/wadspaces-copies/<project>`.
    pub fn path(&self, ws: &str, project: &str) -> PathBuf {
        self.root.join(ws).join(project)
    }

    /// The copy, when it's here whole.
    pub fn made(&self, ws: &str, project: &str) -> Option<Made> {
        let done = std::fs::read(self.path(ws, project).join("done")).ok()?;
        // An older `done` (or one without a source) is still a copy.
        Some(serde_json::from_slice(&done).unwrap_or(Made { files: 0, bytes: 0, from: String::new(), file: None }))
    }

    /// The projects a wadspace has copies (or parts of them) of here.
    pub fn ids(&self, ws: &str) -> Vec<String> {
        names(&self.root.join(ws))
    }

    /// A copy gone (and anything left of one being made).
    pub fn remove(&self, ws: &str, project: &str) -> Res<()> {
        let _ = std::fs::remove_dir_all(self.part(ws, project));
        match std::fs::remove_dir_all(self.path(ws, project)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io(&self.path(ws, project), e)),
        }
    }

    /// Every copy a wadspace has here.
    pub fn remove_all(&self, ws: &str) {
        let _ = std::fs::remove_dir_all(self.root.join(ws));
    }

    fn part(&self, ws: &str, project: &str) -> PathBuf {
        self.root.join(ws).join(format!(".part-{project}"))
    }

    /// A new `.part` folder with an empty `files/`: wadd's own (0700) above it.
    fn begin(&self, ws: &str, project: &str) -> Res<PathBuf> {
        for d in [self.root.clone(), self.root.join(ws)] {
            std::fs::create_dir_all(&d).map_err(|e| io(&d, e))?;
            std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).map_err(|e| io(&d, e))?;
        }
        let part = self.part(ws, project);
        let _ = std::fs::remove_dir_all(&part); // left from a crash
        std::fs::create_dir_all(part.join("files")).map_err(|e| io(&part, e))?;
        Ok(part)
    }

    fn room_for(&self, need: u64) -> Res<()> {
        if let Some(free) = free_bytes(&self.root)
            && need + MARGIN_BYTES > free
        {
            return Err(format!("there isn't room for it: it's {} MB, and {} MB are free", need >> 20, free >> 20));
        }
        Ok(())
    }

    /// A copy of this machine's `source` (a folder's contents, or a file by its name).
    pub async fn copy_local(&self, ws: &str, project: &str, from: &str, source: &Path) -> Res<Made> {
        let s = source.to_path_buf();
        let (_, need) = tokio::task::spawn_blocking(move || size_of(&s)).await.map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&self.root).map_err(|e| io(&self.root, e))?;
        self.room_for(need)?;
        let part = self.begin(ws, project)?;
        let one = (!source.is_dir()).then(|| source.file_name().map(|n| n.to_string_lossy().into_owned())).flatten();
        let what = if one.is_none() { source.join(".") } else { source.to_path_buf() };
        if let Err(e) = run_cp(&what, &part.join("files")).await {
            let _ = std::fs::remove_dir_all(&part);
            return Err(e);
        }
        self.finish(ws, project, &part, from, one.as_deref()).await
    }

    /// A copy of another machine's file or folder at `url` (WebDAV:
    /// Taildrive), what's there being `kind`. `on_bytes(done, total)` as it comes.
    #[allow(clippy::too_many_arguments)]
    pub async fn fetch(
        &self,
        ws: &str,
        project: &str,
        from: &str,
        dav: &Dav,
        url: &str,
        kind: Kind,
        mut on_bytes: impl FnMut(u64, u64) + Send,
    ) -> Result<Made, DavError> {
        let failed = DavError::Failed;
        let (dirs, files): (Vec<String>, Vec<(String, u64)>) = match kind {
            Kind::File(size) => {
                let name = decode(url.trim_end_matches('/').rsplit('/').next().unwrap_or("file"));
                (vec![], vec![(name, size)])
            }
            Kind::Folder => {
                let all = dav.walk(url, MAX_FILES).await?;
                let dirs = all.iter().filter(|(_, k)| *k == Kind::Folder).map(|(p, _)| p.clone()).collect();
                let files = all
                    .into_iter()
                    .filter_map(|(p, k)| match k {
                        Kind::File(n) => Some((p, n)),
                        Kind::Folder => None,
                    })
                    .collect();
                (dirs, files)
            }
        };
        if let Some(bad) = dirs.iter().chain(files.iter().map(|(p, _)| p)).find(|p| !safe_rel(p)) {
            return Err(failed(format!("{bad:?} isn't a name a copy can have")));
        }
        let total: u64 = files.iter().map(|(_, n)| n).sum();
        std::fs::create_dir_all(&self.root).map_err(|e| failed(io(&self.root, e)))?;
        self.room_for(total).map_err(failed)?;
        let part = self.begin(ws, project).map_err(failed)?;
        let out = part.join("files");
        let result = async {
            for d in &dirs {
                let p = out.join(d);
                std::fs::create_dir_all(&p).map_err(|e| failed(io(&p, e)))?;
            }
            let mut done = 0u64;
            on_bytes(0, total);
            for (rel, _) in &files {
                let src = match kind {
                    Kind::File(_) => url.to_string(),
                    Kind::Folder => {
                        let enc: Vec<String> = rel.split('/').map(crate::taildrive::encode).collect();
                        format!("{}/{}", url.trim_end_matches('/'), enc.join("/"))
                    }
                };
                let base = done;
                let n = dav.download(&src, &out.join(rel), |n| on_bytes(base + n, total.max(base + n))).await?;
                done += n;
            }
            Ok::<(), DavError>(())
        }
        .await;
        if let Err(e) = result {
            let _ = std::fs::remove_dir_all(&part);
            return Err(e);
        }
        let one = match kind {
            Kind::File(_) => files.first().map(|(name, _)| name.as_str()),
            Kind::Folder => None,
        };
        self.finish(ws, project, &part, from, one).await.map_err(failed)
    }

    /// A copy made beside its place: counted, the projects user's, and put
    /// in place with its `done` (`file`: it's that one file).
    async fn finish(&self, ws: &str, project: &str, part: &Path, from: &str, file: Option<&str>) -> Res<Made> {
        let files = part.join("files");
        let f = files.clone();
        let (count, bytes) = tokio::task::spawn_blocking(move || size_of(&f)).await.map_err(|e| e.to_string())?;
        let made = Made { files: count, bytes, from: from.to_string(), file: file.map(String::from) };
        let done = serde_json::to_string(&made).map_err(|e| e.to_string())? + "\n";
        std::fs::write(part.join("done"), done).map_err(|e| io(part, e))?;
        std::fs::set_permissions(part, std::fs::Permissions::from_mode(0o755)).map_err(|e| io(part, e))?;
        if let Some(uid) = self.owner.filter(|_| nix::unistd::geteuid().is_root()) {
            let out = tokio::process::Command::new("chown")
                .args(["-R", "--no-dereference", &format!("{uid}:{uid}"), "--"])
                .arg(part)
                .output()
                .await
                .map_err(|e| format!("chown: {e}"))?;
            if !out.status.success() {
                let _ = std::fs::remove_dir_all(part);
                return Err(format!("chown: {}", String::from_utf8_lossy(&out.stderr).trim()));
            }
        }
        let dest = self.path(ws, project);
        // An older copy of something else under this id.
        if dest.exists() {
            std::fs::remove_dir_all(&dest).map_err(|e| io(&dest, e))?;
        }
        std::fs::rename(part, &dest).map_err(|e| io(&dest, e))?;
        tracing::info!("{ws}: {project} copied in: {} files, {} MB, from {from}", made.files, made.bytes >> 20);
        Ok(made)
    }
}

/// A path under a copy, as another machine named it: no `..`, nothing absolute.
fn safe_rel(p: &str) -> bool {
    !p.is_empty() && !p.starts_with('/') && p.split('/').all(|s| !s.is_empty() && s != "." && s != "..")
}

/// The names in a folder that are ids (not hidden: no `.part-`, `.home`, `.kept-`), sorted.
pub fn names(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| crate::store::safe_id(n))
        .collect();
    out.sort();
    out
}

/// cp -a (reflinked where the filesystem can) of `from` into `to`.
async fn run_cp(from: &Path, to: &Path) -> Res<()> {
    std::fs::create_dir_all(to).map_err(|e| io(to, e))?;
    let out = tokio::process::Command::new("cp")
        .args(["-a", "--reflink=auto", "--"])
        .arg(from)
        .arg(to)
        .output()
        .await
        .map_err(|e| format!("cp: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(format!("copying: {}", String::from_utf8_lossy(&out.stderr).trim())) }
}

/// Files (anything not a folder) and their bytes under `path`, links not followed.
pub fn size_of(path: &Path) -> (u64, u64) {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return (0, 0) };
    if !meta.is_dir() {
        return (1, meta.len());
    }
    let mut out = (0, 0);
    for e in std::fs::read_dir(path).into_iter().flatten().flatten() {
        let (f, b) = size_of(&e.path());
        out = (out.0 + f, out.1 + b);
    }
    out
}

fn free_bytes(path: &Path) -> Option<u64> {
    let s = nix::sys::statvfs::statvfs(path).ok()?;
    Some(s.blocks_available() as u64 * s.fragment_size() as u64)
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_a_copy_from_elsewhere_can_have() {
        assert!(super::safe_rel("a/b c.txt") && super::safe_rel("x"));
        for bad in ["", "/etc/x", "a/../b", "a//b", "./a", ".."] {
            assert!(!super::safe_rel(bad), "{bad}");
        }
    }

    #[tokio::test]
    async fn a_local_copy_lands_whole_and_names_where_it_came_from() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("Notes");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), "hello").unwrap();
        std::fs::write(src.join("sub/b.txt"), "there").unwrap();
        let c = super::Copies::new(&d.path().join("state"), None);
        let made = c.copy_local("studio", "p-1", "m:~/Notes", &src).await.unwrap();
        assert_eq!((made.files, made.bytes), (2, 10));
        let at = c.path("studio", "p-1");
        assert_eq!(std::fs::read_to_string(at.join("files/sub/b.txt")).unwrap(), "there");
        assert_eq!(c.made("studio", "p-1").unwrap().from, "m:~/Notes");
        assert_eq!(c.ids("studio"), ["p-1"]);
        // A file: by its name.
        let f = d.path().join("deck.pdf");
        std::fs::write(&f, "%PDF").unwrap();
        let one = c.copy_local("studio", "p-2", "m:~/deck.pdf", &f).await.unwrap();
        assert!(c.path("studio", "p-2").join("files/deck.pdf").is_file());
        // Its done says it's that one file (a folder's says none).
        assert_eq!(one.file.as_deref(), Some("deck.pdf"));
        assert_eq!(made.file, None);
        let done: serde_json::Value =
            serde_json::from_slice(&std::fs::read(c.path("studio", "p-2").join("done")).unwrap()).unwrap();
        assert_eq!(done["file"], "deck.pdf");
        c.remove("studio", "p-1").unwrap();
        assert_eq!(c.ids("studio"), ["p-2"]);
    }
}
