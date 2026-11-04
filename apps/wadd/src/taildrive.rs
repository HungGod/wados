//! Another machine's files, over Taildrive: where a peer's home is
//! ([`home_url`]), what's in a folder there (WebDAV's PROPFIND, [`Dav`]), a
//! file fetched (GET), and a folder mounted on this machine ([`Rclone`]),
//! for a wadspace that keeps a folder of another machine.
//!
//! **Where.** Every machine of yours shares its person's home as `home`
//! (wadd's `tailscale drive share home <home>`, run as them). tailscaled
//! serves every share on the tailnet at `http://100.100.100.100:8080`
//! ([`TAILDRIVE`]), each at `/<tailnet>/<machine>/<share>` (Tailscale's
//! KB 1369). A project's path on its machine is `~/…`, so `~/Pictures/Sprites`
//! on Desk is `…/<tailnet>/desk/home/Pictures/Sprites`.
//!
//! ASSUMED until the tailnet agent says otherwise: `<tailnet>` is
//! `GET /v1/tailnet`'s `tailnet` ('tail1234.ts.net'), and `<machine>` is the
//! first label of the peer's MagicDNS name (its hostname when it has none).
//! Only [`home_url`] knows this.
//!
//! **Mounting** is rclone's (`rclone mount :webdav:`), as a transient
//! systemd unit of its own (`systemd-run`): it runs in the machine's own
//! mount namespace, where podman's containers get theirs (wadd.service's is
//! a private one), outlives a restart of wadd, and its mount goes when the
//! unit stops. As root it's mounted `allow_other` and owned by the projects
//! user (abc in the wadspaces), its writes cached until a file is closed
//! (`--vfs-cache-mode writes`, as editors expect). Why rclone, not davfs2:
//! Taildrive has no WebDAV locks (davfs2 locks unless told not to, and asks
//! for a password), rclone needs no config file or secrets file, stays in
//! the foreground under systemd, and caches writes where it's told.
//!
//! SELinux: a FUSE mount is `fusefs_t`, which a confined container can't
//! use; a wadspace with a mount runs with SELinux's separation off
//! (wad-gen's units), as it does for this machine's own folders.

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;

use crate::backend::Res;

/// Taildrive's WebDAV server, on every machine on the tailnet.
pub const TAILDRIVE: &str = "http://100.100.100.100:8080";
/// What each machine of yours shares its person's home as.
pub const SHARE: &str = "home";
/// A PROPFIND's answer, at most.
const MAX_LISTING: usize = 16 << 20;
/// How long a listing may take.
const LIST_TIMEOUT: Duration = Duration::from_secs(20);

/// A peer's home over Taildrive: `<base>/<tailnet>/<machine>/home`.
/// `dns_name` is its MagicDNS name ('desk.tail1234.ts.net'), `hostname` its
/// own, for when it has none.
pub fn home_url(base: &str, tailnet: &str, dns_name: &str, hostname: &str) -> String {
    let machine = dns_name.split('.').next().filter(|m| !m.is_empty()).unwrap_or(hostname).to_ascii_lowercase();
    format!("{}/{}/{}/{SHARE}", base.trim_end_matches('/'), encode(tailnet), encode(&machine))
}

/// A path on a peer ('~/Pictures/Sprites', '~') at its home's URL. Only
/// what's in its home is shared; no `..`.
pub fn url_in(home: &str, path: &str) -> Result<String, String> {
    let rest = match path.strip_prefix('~') {
        Some(r) if r.is_empty() || r.starts_with('/') => r,
        _ => return Err(format!("{path} isn't in its home, which is all Taildrive shares")),
    };
    let mut url = home.trim_end_matches('/').to_string();
    for part in rest.split('/').filter(|p| !p.is_empty() && *p != ".") {
        if part == ".." {
            return Err(format!("{path} goes out of its home"));
        }
        url.push('/');
        url.push_str(&encode(part));
    }
    Ok(url)
}

/// A path segment, percent-encoded (all but unreserved characters).
pub fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Percent-decoding (invalid escapes kept as they are).
pub fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = b
                .get(i + 1..i + 3)
                .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ------------------------------------------------------------ WebDAV

/// What a path is over WebDAV.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    /// A file, and its size.
    File(u64),
}

/// One entry of a PROPFIND's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its path, decoded ('/tail/desk/home/Pictures/a b.png').
    pub path: String,
    pub folder: bool,
    pub size: u64,
}

/// Why a WebDAV call didn't work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DavError {
    /// Nothing at that path.
    NotFound,
    /// Taildrive said no (the tailnet's policy: drive:share, drive:access).
    Refused(u16),
    /// The machine, or Taildrive, didn't answer.
    Unreachable(String),
    /// Anything else.
    Failed(String),
}

impl std::fmt::Display for DavError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DavError::NotFound => write!(f, "it isn't there"),
            DavError::Refused(code) => write!(f, "Taildrive refused it (HTTP {code})"),
            DavError::Unreachable(e) => write!(f, "Taildrive didn't answer: {e}"),
            DavError::Failed(e) => write!(f, "{e}"),
        }
    }
}

/// A file or folder in a walk: its path under where the walk began ('a/b.txt'), and what it is.
pub type Walked = (String, Kind);

/// A WebDAV client, enough for Taildrive: PROPFIND (depth 0 or 1) and GET.
#[derive(Clone)]
pub struct Dav {
    http: reqwest::Client,
}

impl Default for Dav {
    fn default() -> Self {
        Self::new()
    }
}

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?><D:propfind xmlns:D="DAV:"><D:prop><D:resourcetype/><D:getcontentlength/></D:prop></D:propfind>"#;

impl Dav {
    pub fn new() -> Self {
        Self { http: reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).build().unwrap_or_default() }
    }

    async fn propfind(&self, url: &str, depth: u8) -> Result<Vec<Entry>, DavError> {
        let method = reqwest::Method::from_bytes(b"PROPFIND").expect("a method");
        let res = self
            .http
            .request(method, url)
            .header("Depth", depth.to_string())
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(PROPFIND_BODY)
            .timeout(LIST_TIMEOUT)
            .send()
            .await
            .map_err(|e| DavError::Unreachable(short(&e)))?;
        let code = res.status().as_u16();
        match code {
            207 | 200 => {}
            404 | 410 => return Err(DavError::NotFound),
            401 | 403 => return Err(DavError::Refused(code)),
            _ => return Err(DavError::Failed(format!("PROPFIND {url}: HTTP {code}"))),
        }
        let body = res.bytes().await.map_err(|e| DavError::Unreachable(short(&e)))?;
        if body.len() > MAX_LISTING {
            return Err(DavError::Failed(format!("PROPFIND {url}: its answer is too long")));
        }
        Ok(parse_multistatus(&String::from_utf8_lossy(&body)))
    }

    /// What's at `url`.
    pub async fn stat(&self, url: &str) -> Result<Kind, DavError> {
        let entries = self.propfind(url, 0).await?;
        let me = url_path(url);
        let e = entries
            .iter()
            .find(|e| same_path(&e.path, &me))
            .or(entries.first())
            .ok_or_else(|| DavError::Failed(format!("PROPFIND {url}: nothing in its answer")))?;
        Ok(if e.folder { Kind::Folder } else { Kind::File(e.size) })
    }

    /// What's in the folder at `url` (not itself): (name, what it is).
    pub async fn list(&self, url: &str) -> Result<Vec<(String, Kind)>, DavError> {
        let me = url_path(url);
        let mut out = vec![];
        for e in self.propfind(url, 1).await? {
            if same_path(&e.path, &me) {
                continue;
            }
            let name = e.path.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_string();
            if name.is_empty() || name == "." || name == ".." || name.contains('\0') {
                continue;
            }
            out.push((name, if e.folder { Kind::Folder } else { Kind::File(e.size) }));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    /// Every file and folder under the folder at `url`, folders first in
    /// each; at most `most` of them.
    pub async fn walk(&self, url: &str, most: usize) -> Result<Vec<Walked>, DavError> {
        let mut out: Vec<Walked> = vec![];
        let mut todo: Vec<(String, String)> = vec![(url.trim_end_matches('/').to_string(), String::new())];
        while let Some((at, rel)) = todo.pop() {
            for (name, kind) in self.list(&format!("{at}/")).await? {
                let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                if kind == Kind::Folder {
                    todo.push((format!("{at}/{}", encode(&name)), r.clone()));
                }
                out.push((r, kind));
                if out.len() > most {
                    return Err(DavError::Failed(format!("there are more than {most} files in it")));
                }
            }
        }
        Ok(out)
    }

    /// The file at `url` into `to` (made new); `on_bytes(done)` as it comes. Its size.
    pub async fn download(&self, url: &str, to: &Path, mut on_bytes: impl FnMut(u64) + Send) -> Result<u64, DavError> {
        let mut res = self.http.get(url).send().await.map_err(|e| DavError::Unreachable(short(&e)))?;
        let code = res.status().as_u16();
        match code {
            200 => {}
            404 | 410 => return Err(DavError::NotFound),
            401 | 403 => return Err(DavError::Refused(code)),
            _ => return Err(DavError::Failed(format!("GET {url}: HTTP {code}"))),
        }
        let mut f =
            tokio::fs::File::create(to).await.map_err(|e| DavError::Failed(format!("{}: {e}", to.display())))?;
        let mut n = 0u64;
        while let Some(chunk) = res.chunk().await.map_err(|e| DavError::Unreachable(short(&e)))? {
            f.write_all(&chunk).await.map_err(|e| DavError::Failed(format!("{}: {e}", to.display())))?;
            n += chunk.len() as u64;
            on_bytes(n);
        }
        f.flush().await.map_err(|e| DavError::Failed(format!("{}: {e}", to.display())))?;
        Ok(n)
    }
}

/// An error's words, without reqwest's long chain.
fn short(e: &reqwest::Error) -> String {
    let mut s = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(c) = src {
        s = c.to_string();
        src = c.source();
    }
    s
}

/// The path of a URL, decoded ('http://h:1/a%20b/' → '/a b/').
fn url_path(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let path = rest.find('/').map_or("/", |i| &rest[i..]);
    decode(path.split(['?', '#']).next().unwrap_or("/"))
}

fn same_path(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

/// XML's five entities and character references.
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';') else { break };
        let ent = &rest[1..end];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A PROPFIND's `multistatus`: each response's path, whether it's a
/// collection and its length. Any namespace prefix (`D:`, `d:`, none).
pub fn parse_multistatus(xml: &str) -> Vec<Entry> {
    let mut out = vec![];
    let mut cur: Option<Entry> = None;
    let (mut in_href, mut in_len, mut in_type) = (false, false, false);
    let mut text = String::new();
    let mut rest = xml;
    while let Some(i) = rest.find('<') {
        if in_href || in_len {
            text.push_str(&rest[..i]);
        }
        rest = &rest[i + 1..];
        if let Some(r) = rest.strip_prefix("![CDATA[") {
            let end = r.find("]]>").unwrap_or(r.len());
            if in_href || in_len {
                text.push_str(&r[..end]);
            }
            rest = r.get(end + 3..).unwrap_or("");
            continue;
        }
        let Some(j) = rest.find('>') else { break };
        let tag = &rest[..j];
        rest = &rest[j + 1..];
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        let closing = tag.starts_with('/');
        let empty = tag.ends_with('/');
        let name = tag.trim_start_matches('/').trim_end_matches('/').split_whitespace().next().unwrap_or("");
        let local = name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase();
        match (local.as_str(), closing) {
            ("response", false) if !empty => cur = Some(Entry { path: String::new(), folder: false, size: 0 }),
            ("response", true) => {
                if let Some(e) = cur.take().filter(|e| !e.path.is_empty()) {
                    out.push(e);
                }
            }
            ("href", false) if !empty => {
                in_href = true;
                text.clear();
            }
            ("href", true) => {
                in_href = false;
                if let Some(e) = cur.as_mut() {
                    e.path = url_path(unescape(text.trim()).as_str());
                }
            }
            ("getcontentlength", false) if !empty => {
                in_len = true;
                text.clear();
            }
            ("getcontentlength", true) => {
                in_len = false;
                if let (Some(e), Ok(n)) = (cur.as_mut(), text.trim().parse::<u64>()) {
                    e.size = n;
                }
            }
            ("resourcetype", false) => in_type = !empty,
            ("resourcetype", true) => in_type = false,
            ("collection", false) if in_type => {
                if let Some(e) = cur.as_mut() {
                    e.folder = true;
                }
            }
            _ => {}
        }
    }
    out
}

// ------------------------------------------------------------ mounting

/// A folder over WebDAV, mounted on this machine (rclone, or tests' stand-in).
#[async_trait]
pub trait Drive: Send + Sync + 'static {
    /// The folder at `url` mounted at `at` (an empty folder), read-write, by
    /// the unit `unit` (a service of its own, running until it's unmounted).
    async fn mount(&self, unit: &str, url: &str, at: &Path) -> Res<()>;
    /// What `mount` made, gone: its unit stopped, its mount undone. Nothing
    /// there is nothing to do.
    async fn unmount(&self, unit: &str, at: &Path) -> Res<()>;
}

/// rclone's mount, each a transient unit (`systemd-run`): the system's on a
/// machine (wadd is root), yours on a laptop.
pub struct Rclone {
    /// The system's manager, rclone as root (`allow_other`); else yours.
    pub system: bool,
    /// Who owns what's mounted (abc in the wadspaces).
    pub uid: u32,
    /// Where rclone caches what's being written (`<cache>/<unit>`).
    pub cache: PathBuf,
    /// How long a mount may take to appear.
    pub timeout: Duration,
}

impl Rclone {
    /// `systemd-run`'s arguments for a mount.
    pub fn argv(&self, unit: &str, url: &str, at: &Path) -> Vec<String> {
        let mut a: Vec<String> = vec![];
        if !self.system {
            a.push("--user".into());
        }
        a.extend([
            format!("--unit={unit}"),
            format!("--description=WadSpaces: {url} over Taildrive"),
            "--collect".into(),
            "--quiet".into(),
            "--property=Type=simple".into(),
            // The mount ends with it: SIGTERM, which rclone unmounts on.
            "--property=KillMode=mixed".into(),
            "--property=TimeoutStopSec=15".into(),
            "rclone".into(),
            "mount".into(),
            ":webdav:".into(),
            at.to_string_lossy().into_owned(),
            format!("--webdav-url={url}"),
            "--webdav-vendor=other".into(),
            // Editors write and rename: files open for writing are kept here until they're closed.
            "--vfs-cache-mode=writes".into(),
            format!("--cache-dir={}", self.cache.join(unit).display()),
            // Taildrive can't say what changed: a listing is asked again after this.
            "--dir-cache-time=10s".into(),
            format!("--uid={}", self.uid),
            format!("--gid={}", self.uid),
            "--umask=022".into(),
            "--log-level=NOTICE".into(),
        ]);
        if self.system {
            // Root mounts it, abc (another uid, in a wadspace's user namespace) uses it.
            a.push("--allow-other".into());
        }
        a
    }

    async fn systemctl(&self, args: &[&str]) -> Res<String> {
        let mut cmd = tokio::process::Command::new("systemctl");
        if !self.system {
            cmd.arg("--user");
        }
        let out = cmd.args(args).output().await.map_err(|e| format!("systemctl: {e}"))?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Whether `at` is a FUSE mount, where podman sees it (PID 1's mounts, on a machine).
    async fn mounted(&self, at: &Path) -> bool {
        let at = at.to_path_buf();
        let system = self.system;
        tokio::task::spawn_blocking(move || {
            if system && nix::unistd::geteuid().is_root() {
                crate::mounts::inside(1, move || Ok(is_fuse(&at))).unwrap_or(false)
            } else {
                is_fuse(&at)
            }
        })
        .await
        .unwrap_or(false)
    }
}

fn is_fuse(at: &Path) -> bool {
    nix::sys::statfs::statfs(at).is_ok_and(|s| s.filesystem_type() == nix::sys::statfs::FUSE_SUPER_MAGIC)
}

#[async_trait]
impl Drive for Rclone {
    async fn mount(&self, unit: &str, url: &str, at: &Path) -> Res<()> {
        if which("rclone").is_none() {
            return Err("rclone isn't on this machine, so another machine's folders can't be mounted".into());
        }
        let out = tokio::process::Command::new("systemd-run")
            .args(self.argv(unit, url, at))
            .output()
            .await
            .map_err(|e| format!("systemd-run: {e}"))?;
        if !out.status.success() {
            return Err(format!("systemd-run: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        let deadline = tokio::time::Instant::now() + self.timeout;
        loop {
            if self.mounted(at).await {
                return Ok(());
            }
            let state = self.systemctl(&["is-active", unit]).await.unwrap_or_default();
            if matches!(state.as_str(), "failed" | "inactive") {
                let how = if self.system { "journalctl" } else { "journalctl --user" };
                return Err(format!("rclone didn't mount it ({how} -u {unit} says why)"));
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = self.unmount(unit, at).await;
                return Err(format!("rclone didn't mount it within {} s", self.timeout.as_secs()));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn unmount(&self, unit: &str, at: &Path) -> Res<()> {
        let _ = self.systemctl(&["stop", unit]).await;
        if !self.mounted(at).await {
            return Ok(());
        }
        // rclone gone without unmounting (killed): taken off by force.
        if self.system && nix::unistd::geteuid().is_root() {
            let p = at.to_path_buf();
            tokio::task::spawn_blocking(move || crate::mounts::inside(1, move || crate::mounts::unmount(&p)))
                .await
                .map_err(|e| e.to_string())??;
        } else {
            let out = tokio::process::Command::new("fusermount3")
                .args(["-u", "-z", "--"])
                .arg(at)
                .output()
                .await
                .map_err(|e| format!("fusermount3: {e}"))?;
            if !out.status.success() {
                return Err(format!("fusermount3: {}", String::from_utf8_lossy(&out.stderr).trim()));
            }
        }
        Ok(())
    }
}

/// A program on PATH.
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into());
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

/// A mount's unit: one per wadspace and project, from both (ids may share
/// dashes, so a hash keeps any two apart).
pub fn unit_name(ws: &str, project: &str) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(format!("{ws}\0{project}").as_bytes());
    let hex: String = h.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("wadtaildrive-{hex}.service")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peers_home_and_its_paths() {
        let home = home_url(TAILDRIVE, "tail1234.ts.net", "desk.tail1234.ts.net", "Desk");
        assert_eq!(home, "http://100.100.100.100:8080/tail1234.ts.net/desk/home");
        assert_eq!(home_url("http://x/", "t", "", "Laptop"), "http://x/t/laptop/home");
        assert_eq!(url_in(&home, "~/Pictures/My Sprites").unwrap(), format!("{home}/Pictures/My%20Sprites"));
        assert_eq!(url_in(&home, "~").unwrap(), home);
        assert_eq!(url_in(&home, "~//Desktop/./x").unwrap(), format!("{home}/Desktop/x"));
        assert!(url_in(&home, "/etc/passwd").is_err());
        assert!(url_in(&home, "~/../root").is_err());
        assert!(url_in(&home, "~root/x").is_err());
        assert_eq!(decode("a%20b%2Fc%zz%"), "a b/c%zz%");
        assert_eq!(encode("é x"), "%C3%A9%20x");
    }

    #[test]
    fn multistatus_in_any_prefix() {
        // golang.org/x/net/webdav's (Taildrive's) shape.
        let go = r#"<?xml version="1.0" encoding="UTF-8"?><D:multistatus xmlns:D="DAV:"><D:response><D:href>/tail/desk/home/Pictures/</D:href><D:propstat><D:prop><D:resourcetype><D:collection xmlns:D="DAV:"/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat><D:propstat><D:prop><D:getcontentlength></D:getcontentlength></D:prop><D:status>HTTP/1.1 404 Not Found</D:status></D:propstat></D:response><D:response><D:href>/tail/desk/home/Pictures/a%20b.png</D:href><D:propstat><D:prop><D:resourcetype></D:resourcetype><D:getcontentlength>1234</D:getcontentlength></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;
        assert_eq!(
            parse_multistatus(go),
            [
                Entry { path: "/tail/desk/home/Pictures/".into(), folder: true, size: 0 },
                Entry { path: "/tail/desk/home/Pictures/a b.png".into(), folder: false, size: 1234 },
            ]
        );
        // Another server's: a lowercase prefix, whole URLs, an entity, an empty resourcetype.
        let other = "<d:multistatus xmlns:d='DAV:'><d:response><d:href>http://h:8/x/&amp;y/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>\
            <d:response><d:href>http://h:8/x/&amp;y/z.txt</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>7</d:getcontentlength></d:prop></d:propstat></d:response></d:multistatus>";
        let e = parse_multistatus(other);
        assert_eq!(e[0], Entry { path: "/x/&y/".into(), folder: true, size: 0 });
        assert_eq!(e[1], Entry { path: "/x/&y/z.txt".into(), folder: false, size: 7 });
        // No prefix at all.
        let plain = r#"<multistatus xmlns="DAV:"><response><href>/a/</href><propstat><prop><resourcetype><collection/></resourcetype></prop></propstat></response></multistatus>"#;
        assert!(parse_multistatus(plain)[0].folder);
    }

    #[test]
    fn rclone_as_root_mounts_for_abc() {
        let r = Rclone {
            system: true,
            uid: 1000,
            cache: "/var/lib/wados/taildrive/.cache".into(),
            timeout: Duration::from_secs(20),
        };
        let unit = unit_name("studio", "p-3");
        let a = r.argv(
            &unit,
            "http://100.100.100.100:8080/t/desk/home/Pictures",
            Path::new("/var/lib/wados/taildrive/studio/p-3"),
        );
        assert!(!a.contains(&"--user".to_string()));
        let at = a.iter().position(|x| x == "rclone").unwrap();
        assert_eq!(a[at..at + 4], ["rclone", "mount", ":webdav:", "/var/lib/wados/taildrive/studio/p-3"]);
        for want in ["--allow-other", "--uid=1000", "--gid=1000", "--webdav-vendor=other", "--vfs-cache-mode=writes"] {
            assert!(a.contains(&want.to_string()), "{want} in {a:?}");
        }
        assert!(
            a.contains(&format!("--unit={unit}"))
                && a.contains(&"--webdav-url=http://100.100.100.100:8080/t/desk/home/Pictures".to_string())
        );
        assert!(a.contains(&format!("--cache-dir=/var/lib/wados/taildrive/.cache/{unit}")));
        // Yours, on a laptop: your manager, and no allow_other (fuse.conf would have to allow it).
        let mine = Rclone { system: false, ..r };
        let a = mine.argv(&unit, "http://x/", Path::new("/tmp/m"));
        assert_eq!(a[0], "--user");
        assert!(!a.contains(&"--allow-other".to_string()));
        // A unit each, apart even when ids share dashes.
        assert_ne!(unit_name("a-b", "c"), unit_name("a", "b-c"));
        assert!(unit_name("a", "b").starts_with("wadtaildrive-") && unit_name("a", "b").ends_with(".service"));
    }
}
