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
    /// signed in as nothing, and nothing would ever clear them.
    fn prune(&self, in_use: &[String]) {
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
}
