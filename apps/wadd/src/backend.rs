//! What wadd asks of the machine to build and run wadspaces: podman (images,
//! builds, pulls, containers), systemd (the wadspaces' quadlet units) and a
//! download (CI's package index). A trait, so building and running are
//! tested against a fake ([`crate::fake`]).
//!
//! Every failure is the words to show (a `String`): what failed and why.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value};
pub use wad_podman::ImageInfo;
use wad_podman::Podman;
use wad_systemd::Systemd;

pub type Res<T> = Result<T, String>;
/// Lines from a build, as they come.
pub type OnLine = Box<dyn FnMut(&str) + Send>;
/// A download's bytes so far, and in all.
pub type OnBytes = Box<dyn FnMut(u64, u64) + Send>;

/// What a container's status is when there's no such container.
pub const MISSING: &str = "missing";
pub const RUNNING: &str = "running";

#[async_trait]
pub trait Backend: Send + Sync + 'static {
    /// Podman answers.
    async fn available(&self) -> bool;

    // ------------------------------------------------------------ images
    /// An image's id, size, labels and names; None if it isn't here.
    async fn image_info(&self, image: &str) -> Res<Option<ImageInfo>>;
    /// The images that carry a label.
    async fn images_labelled(&self, label: &str) -> Res<Vec<ImageInfo>>;
    /// Builds `tag` from a tar build context (its `Dockerfile`), with these
    /// build arguments and labels; returns the image's id. Dropping the
    /// future stops the build.
    async fn build(
        &self,
        context: Vec<u8>,
        tag: &str,
        args: Map<String, Value>,
        labels: Map<String, Value>,
        on_line: OnLine,
    ) -> Res<String>;
    /// Downloads an image unless it's here (`auth`: an X-Registry-Auth
    /// value); returns its id. Dropping the future stops it.
    async fn pull(&self, reference: &str, auth: Option<String>, on_bytes: OnBytes) -> Res<String>;
    /// Gives an image another name ("repo:tag").
    async fn tag(&self, image: &str, target: &str) -> Res<()>;
    /// Takes a name off an image (the image goes with its last name). False if there was none.
    async fn remove_image(&self, image: &str) -> Res<bool>;
    /// Free space where images are kept, in GB (None: unknown).
    async fn free_gb(&self) -> Option<f64>;
    /// The names of podman's secrets.
    async fn secret_names(&self) -> Res<Vec<String>>;
    /// A secret's value, for wadd's own use (signing in to a registry).
    async fn secret_value(&self, name: &str) -> Res<Option<String>>;
    /// Makes a secret, replacing one of that name (the GitHub sign-in's
    /// token). The value only ever goes to podman.
    async fn create_secret(&self, name: &str, value: &[u8]) -> Res<()>;
    /// Deletes a secret. False if there was none.
    async fn delete_secret(&self, name: &str) -> Res<bool>;
    /// A document from the network (`https://`) or a file (`file://`): CI's package index.
    async fn fetch(&self, url: &str) -> Res<Vec<u8>>;

    // ------------------------------------------------------------ containers
    /// A container's status: running, exited, ... ([`MISSING`]: there's none).
    async fn container_status(&self, name: &str) -> Res<String>;
    /// A running container's first process, on the machine.
    async fn container_pid(&self, name: &str) -> Res<Option<u32>>;
    /// A container's last lines of output.
    async fn container_logs(&self, name: &str, lines: usize) -> Res<Option<String>>;
    /// Whether a running wadspace's desktop is up: its nested labwc runs and
    /// answers on its own socket, so it's connected to the compositor wadd
    /// served it and has made its window there.
    async fn desktop_up(&self, name: &str) -> Res<bool>;

    // ------------------------------------------------------------ units
    /// Writes these unit files (name, text) and removes the files named in
    /// `remove`, in the units folder; systemd reloads (quadlet makes their
    /// services) when anything changed. True if it did.
    async fn write_units(&self, files: &[(String, String)], remove: &[String]) -> Res<bool>;
    async fn start(&self, unit: &str) -> Res<()>;
    async fn stop(&self, unit: &str) -> Res<()>;
}

/// A desktop is up: its labwc's pid file names a live process, and one of
/// its sockets (`$XDG_RUNTIME_DIR/wayland-N`) answers. labwc makes its
/// socket only after its backend has connected to the display under it, and
/// its window comes with the backend's start, a moment later; a socket left
/// from an earlier labwc doesn't answer.
const DESKTOP_UP: &str = r#"p="$(cat /run/wadspaces-labwc.pid 2> /dev/null)" && kill -0 "$p" 2> /dev/null && exec python3 -c '
import glob, socket, sys
for s in glob.glob("/config/.XDG/wayland-[0-9]*"):
    if s.endswith(".lock"):
        continue
    try:
        socket.socket(socket.AF_UNIX).connect(s)
        sys.exit(0)
    except OSError:
        pass
sys.exit(1)'"#;

/// The machine's podman and systemd: rootful and the system manager on a
/// machine, yours (rootless, your user manager) on a laptop. systemd is
/// connected to on first use.
pub struct Real {
    pub podman: Podman,
    systemd: tokio::sync::OnceCell<Systemd>,
    user: bool,
    pub units_dir: PathBuf,
    pub start_timeout: Duration,
    http: reqwest::Client,
}

impl Real {
    pub fn new(config: &wad_config::Config, profile: wad_config::Profile) -> Self {
        let d = &config.daemon;
        Self {
            podman: Podman::new(&d.podman_socket),
            systemd: tokio::sync::OnceCell::new(),
            user: profile == wad_config::Profile::User,
            units_dir: d.units_dir.clone(),
            start_timeout: Duration::from_secs(d.ready_timeout_s.max(30)),
            http: reqwest::Client::new(),
        }
    }

    async fn systemd(&self) -> Res<&Systemd> {
        self.systemd
            .get_or_try_init(|| async { if self.user { Systemd::user().await } else { Systemd::system().await } })
            .await
            .map_err(|e| e.to_string())
    }
}

fn s(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[async_trait]
impl Backend for Real {
    async fn available(&self) -> bool {
        // Socket-activated podman stops its API service after a few idle
        // seconds and starts it again on the next call; a call that lands
        // just as it stops goes unanswered. Asked again before giving up.
        if self.podman.ping().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        self.podman.ping().await
    }

    async fn image_info(&self, image: &str) -> Res<Option<ImageInfo>> {
        self.podman.image_info(image).await.map_err(s)
    }

    async fn images_labelled(&self, label: &str) -> Res<Vec<ImageInfo>> {
        self.podman.images_labelled(label).await.map_err(s)
    }

    async fn build(
        &self,
        context: Vec<u8>,
        tag: &str,
        args: Map<String, Value>,
        labels: Map<String, Value>,
        mut on_line: OnLine,
    ) -> Res<String> {
        let id = self.podman.build(context, tag, &args, &labels, |l| on_line(l)).await.map_err(s)?;
        if !id.is_empty() {
            return Ok(id);
        }
        // An answer without the id line: the image under its name says.
        Ok(self.podman.image_info(tag).await.map_err(s)?.map(|i| i.id).unwrap_or_default())
    }

    async fn pull(&self, reference: &str, auth: Option<String>, on_bytes: OnBytes) -> Res<String> {
        self.podman.pull(reference, auth.as_deref(), on_bytes).await.map_err(s)
    }

    async fn tag(&self, image: &str, target: &str) -> Res<()> {
        self.podman.tag(image, target).await.map_err(s)
    }

    async fn remove_image(&self, image: &str) -> Res<bool> {
        self.podman.remove_image(image).await.map_err(s)
    }

    async fn free_gb(&self) -> Option<f64> {
        let root = self.podman.graph_root().await.ok().flatten().map(PathBuf::from);
        let path = root.filter(|r| r.is_dir()).unwrap_or_else(|| "/".into());
        let st = nix::sys::statvfs::statvfs(&path).ok()?;
        Some(st.blocks_available() as f64 * st.fragment_size() as f64 / 1e9)
    }

    async fn secret_names(&self) -> Res<Vec<String>> {
        self.podman.secret_names().await.map_err(s)
    }

    async fn secret_value(&self, name: &str) -> Res<Option<String>> {
        self.podman.secret_value(name).await.map_err(s)
    }

    async fn create_secret(&self, name: &str, value: &[u8]) -> Res<()> {
        self.podman.create_secret(name, value).await.map_err(s)
    }

    async fn delete_secret(&self, name: &str) -> Res<bool> {
        self.podman.delete_secret(name).await.map_err(s)
    }

    async fn fetch(&self, url: &str) -> Res<Vec<u8>> {
        if let Some(path) = url.strip_prefix("file://") {
            return std::fs::read(path).map_err(|e| format!("{path}: {e}"));
        }
        let res =
            self.http.get(url).timeout(Duration::from_secs(15)).send().await.map_err(|e| format!("{url}: {e}"))?;
        if !res.status().is_success() {
            return Err(format!("{url}: HTTP {}", res.status()));
        }
        Ok(res.bytes().await.map_err(|e| format!("{url}: {e}"))?.to_vec())
    }

    async fn container_status(&self, name: &str) -> Res<String> {
        self.podman.container_status(name).await.map_err(s)
    }

    async fn container_pid(&self, name: &str) -> Res<Option<u32>> {
        let info = self.podman.inspect_container(name).await.map_err(s)?;
        Ok(info
            .and_then(|i| i.pointer("/State/Pid").and_then(Value::as_u64))
            .filter(|p| *p > 0)
            .and_then(|p| u32::try_from(p).ok()))
    }

    async fn container_logs(&self, name: &str, lines: usize) -> Res<Option<String>> {
        self.podman.container_logs(name, lines).await.map_err(s)
    }

    async fn desktop_up(&self, name: &str) -> Res<bool> {
        let out = self.podman.exec(name, "abc", &["sh", "-c", DESKTOP_UP], &[]).await.map_err(s)?;
        Ok(out.code == 0)
    }

    async fn write_units(&self, files: &[(String, String)], remove: &[String]) -> Res<bool> {
        let changed = write_unit_files(&self.units_dir, files, remove)?;
        if changed {
            self.systemd().await?.reload().await.map_err(s)?;
        }
        Ok(changed)
    }

    async fn start(&self, unit: &str) -> Res<()> {
        self.systemd().await?.start(unit, self.start_timeout).await.map_err(s)
    }

    async fn stop(&self, unit: &str) -> Res<()> {
        self.systemd().await?.stop(unit, Duration::from_secs(120)).await.map_err(s)
    }
}

/// The unit files in `dir` as asked: each of `files` written (atomically,
/// when its text changed), each of `remove` gone. True if anything changed.
pub fn write_unit_files(dir: &std::path::Path, files: &[(String, String)], remove: &[String]) -> Res<bool> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut changed = false;
    for (name, text) in files {
        if name.contains('/') || name.starts_with('.') {
            return Err(format!("not a unit's name: {name:?}"));
        }
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
            continue;
        }
        let tmp = dir.join(format!(".{name}.tmp"));
        std::fs::write(&tmp, text)
            .and_then(|_| std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)))
            .and_then(|_| std::fs::rename(&tmp, &path))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        changed = true;
    }
    for name in remove.iter().filter(|n| !n.contains('/') && !files.iter().any(|(f, _)| f == *n)) {
        match std::fs::remove_file(dir.join(name)) {
            Ok(()) => changed = true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", dir.join(name).display())),
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_files_are_written_once_and_removed() {
        let d = tempfile::tempdir().unwrap();
        let files = vec![("wad-a.container".to_string(), "[Container]\n".to_string())];
        assert!(write_unit_files(d.path(), &files, &[]).unwrap());
        // The same again: nothing to reload.
        assert!(!write_unit_files(d.path(), &files, &[]).unwrap());
        assert!(write_unit_files(d.path(), &[], &["wad-a.container".into()]).unwrap());
        assert!(!d.path().join("wad-a.container").exists());
        assert!(!write_unit_files(d.path(), &[], &["wad-a.container".into()]).unwrap());
        assert!(write_unit_files(d.path(), &[("../x".into(), String::new())], &[]).is_err());
    }
}
