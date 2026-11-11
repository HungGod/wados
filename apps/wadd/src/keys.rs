//! The machine's web keys, for its own WadBrowser (`GET /v1/keys/web`,
//! `HOOK(keys-on-machine)`): each with its folder, `<signins_dir>/<id>`,
//! which runs.rs mounts into the wadspaces that use the key. A folder that
//! isn't there yet is made as runs.rs makes it (0700, uid 1000's when wadd
//! is root), so a key saved in the machine's browser has one at once.
//!
//! A folder whose key the library no longer has is removed when the keys are
//! listed ([`MachineKeys::web`]): nothing signs in with it any more, and it
//! would otherwise stay for good.
//!
//! The browser runs as uid 1000 and can't list `/var/lib/wados` or `keys/`
//! (0711: tmpfiles.d/wados.conf), but reaches its own folder by name: its
//! profile for the key keeps its cookies there, as a wadspace's does.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use wad_model::{Secret, SecretKind};
use wad_proto::signin::WebKey;

/// Cheap to clone (a folder and its owner), so the loop can prune off-thread.
#[derive(Clone)]
pub struct MachineKeys {
    dir: PathBuf,
    /// Who owns a new folder (wadd as root: abc's, 1000).
    uid: Option<u32>,
}

impl MachineKeys {
    pub fn new(config: &wad_config::Config) -> Self {
        let d = &config.daemon;
        let root = nix::unistd::geteuid().is_root();
        Self::at(&d.signins_dir, root.then_some(d.projects_uid))
    }

    pub fn at(dir: &Path, uid: Option<u32>) -> Self {
        Self { dir: dir.to_path_buf(), uid }
    }

    /// Every key id the library keeps, and every one a wadspace still names
    /// (its slots and its groups): what a folder must be cleared with, so a
    /// key removed from the library but still used by a wadspace (or in the
    /// Trash, which can come back) keeps its cookies.
    pub fn in_use(lib: &wad_model::edit::Library) -> Vec<String> {
        let mut ids: Vec<String> = lib.keys.iter().map(|k| k.id.clone()).collect();
        for x in &lib.wadspaces {
            for k in x.secrets.by_slot.values() {
                ids.push(k.id.clone());
            }
            ids.extend(x.secrets.groups.iter().map(|g| g.key.id.clone()));
        }
        ids
    }

    /// The library's web keys, each with its folder (made if it's new). A
    /// key whose folder can't be made is left out (and logged). Folders `in
    /// use` (the library's keys, and every key a wadspace still names) doesn't
    /// have go.
    pub fn web(&self, keys: &[Secret], in_use: &[String]) -> Vec<WebKey> {
        self.prune(in_use);
        keys.iter()
            .filter(|k| k.kind == SecretKind::Web && crate::store::safe_id(&k.id))
            .filter_map(|k| match self.folder(&k.id) {
                Ok(dir) => Some(WebKey {
                    id: k.id.clone(),
                    name: k.name.clone(),
                    sites: k.sites.clone(),
                    used_by: k.used_by.clone(),
                    dir: dir.to_string_lossy().into_owned(),
                }),
                Err(e) => {
                    tracing::warn!("{}'s folder: {e}", k.name);
                    None
                }
            })
            .collect()
    }

    /// Removes the folders of keys nothing is (nor names): their cookies are
    /// signed in as nothing, and nothing would ever clear them. Also run from
    /// wadd's own loop, so a machine whose browser never asks is cleared too.
    pub fn prune(&self, in_use: &[String]) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        for e in entries.flatten() {
            let Ok(id) = e.file_name().into_string() else { continue };
            if !crate::store::safe_id(&id) || in_use.iter().any(|k| k == &id) {
                continue;
            }
            match std::fs::remove_dir_all(e.path()) {
                Ok(()) => tracing::info!("{id}'s folder removed: it isn't a key any more"),
                Err(err) => tracing::warn!("{id}'s folder: {err}"),
            }
        }
    }

    /// Key `id`'s folder, made empty if it's new.
    fn folder(&self, id: &str) -> Result<PathBuf, String> {
        let dir = self.dir.join(id);
        if dir.is_dir() {
            return Ok(dir);
        }
        let err = |e: std::io::Error| format!("{}: {e}", dir.display());
        std::fs::create_dir_all(&dir).map_err(err)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(err)?;
        if let Some(uid) = self.uid {
            nix::unistd::chown(&dir, Some(uid.into()), Some(uid.into()))
                .map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        tracing::info!("{id}'s folder made, for the machine's browser");
        Ok(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every id a library of `all` keeps, as the endpoint builds it.
    fn in_use(all: &[Secret]) -> Vec<String> {
        all.iter().map(|k| k.id.clone()).collect()
    }

    fn key(id: &str, kind: SecretKind, sites: &[&str]) -> Secret {
        Secret {
            id: id.into(),
            name: id.to_uppercase(),
            kind,
            paths: vec![],
            size: 0,
            created_at: 1,
            changed_at: None,
            used_by: vec!["dev".into()],
            sites: sites.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn web_keys_with_their_folders_made() {
        let d = tempfile::tempdir().unwrap();
        let keys = MachineKeys::at(&d.path().join("keys"), None);
        let all = [
            key("gh", SecretKind::Web, &["github.com"]),
            key("vs", SecretKind::Vscode, &[]),
            key("../up", SecretKind::Web, &["evil.test"]),
            key("work", SecretKind::Web, &[]),
        ];
        let web = keys.web(&all, &in_use(&all));
        assert_eq!(web.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(), ["gh", "work"]);
        let gh = &web[0];
        assert_eq!(
            (gh.name.as_str(), gh.sites.as_slice(), gh.used_by.as_slice()),
            ("GH", &["github.com".to_string()][..], &["dev".to_string()][..])
        );
        let dir = Path::new(&gh.dir);
        assert_eq!(dir, d.path().join("keys/gh"));
        assert_eq!(std::fs::metadata(dir).unwrap().permissions().mode() & 0o777, 0o700);
        // What's in it stays.
        std::fs::write(dir.join("cookies.sqlite"), b"kept").unwrap();
        keys.web(&all, &in_use(&all));
        assert_eq!(std::fs::read(dir.join("cookies.sqlite")).unwrap(), b"kept");
        assert!(!d.path().join("up").exists() && !d.path().join("keys/vs").exists());
    }

    #[test]
    fn a_folder_nothing_names_goes_and_one_a_wadspace_still_does_stays() {
        let d = tempfile::tempdir().unwrap();
        let keys = MachineKeys::at(&d.path().join("keys"), None);
        let all = [key("gh", SecretKind::Web, &["github.com"]), key("work", SecretKind::Web, &[])];
        keys.web(&all, &in_use(&all));
        let gh = d.path().join("keys/gh");
        std::fs::write(gh.join("cookies.sqlite"), b"kept").unwrap();
        // "work" left the library, but a wadspace still names it: its folder stays.
        let gone = [key("gh", SecretKind::Web, &["github.com"])];
        let mut named = in_use(&gone);
        named.push("work".into());
        keys.web(&gone, &named);
        assert!(d.path().join("keys/work").exists(), "a wadspace names it");
        // Nothing names it now: it goes, with what's in it.
        keys.web(&gone, &in_use(&gone));
        assert!(!d.path().join("keys/work").exists());
        assert_eq!(std::fs::read(gh.join("cookies.sqlite")).unwrap(), b"kept");
    }

    #[test]
    fn a_trashed_wadspace_still_names_its_keys() {
        // A wadspace in the Trash can come back, so its keys' folders stay
        // while it's there (wadd lists it like any other document).
        let d = tempfile::tempdir().unwrap();
        let keys = MachineKeys::at(&d.path().join("keys"), None);
        let all = [key("gh", SecretKind::Web, &["github.com"])];
        keys.web(&all, &in_use(&all));
        std::fs::write(d.path().join("keys/gh/cookies.sqlite"), b"kept").unwrap();
        // "gh" is the library's; "old" only a trashed wadspace's.
        std::fs::create_dir_all(d.path().join("keys/old")).unwrap();
        let mut lib = wad_model::edit::Library { keys: all.to_vec(), ..Default::default() };
        let mut trashed = wad_model::ops::blank("club", "Club", "m", 0, 1);
        trashed.deleted_at = Some(2);
        trashed.secrets.by_slot.insert(
            "app:wadbrowser".into(),
            wad_model::types::KeyRef { id: "old".into(), name: "Old".into(), kind: SecretKind::Web },
        );
        lib.wadspaces = vec![trashed];
        let named = MachineKeys::in_use(&lib);
        keys.web(&all, &named);
        assert!(d.path().join("keys/old").exists(), "a trashed wadspace names it");
        assert!(d.path().join("keys/gh/cookies.sqlite").exists());
    }
}
