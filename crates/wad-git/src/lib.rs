//! Projects' git on the machine (gitimport.py): the first clone, and keeping
//! a copy current.
//!
//! A clone goes into `<dest>.part` and is renamed into place only when it
//! worked, so a project folder is never half there; a failed or cancelled
//! clone leaves nothing behind. When wadd runs as root, git runs as the
//! projects user (abc in the images), with a HOME of its own, so the files
//! belong to whoever works on them.
//!
//! For github.com the token comes through an inline credential helper that
//! reads it from the environment: it's never on a command line (visible in
//! ps) or in a log.
//!
//! A copy that's already here is brought up to date at launch (update()):
//! fetch, then a fast-forward, only when that can't lose or tangle anything
//! (a clean tree, an upstream, nothing unpushed). Otherwise it's left as it
//! is, and update() says why. status() is the same picture from the refs
//! already here, without the network.
//!
//! v1's, as it was (`crates/wad-git`), but for [`GitStatus`], which was
//! wad-proto's and is this crate's own now.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// A project folder's git state, from the refs here (no fetch).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitStatus {
    /// Its branch (none: a detached HEAD).
    pub branch: Option<String>,
    /// Uncommitted changes, untracked files included.
    pub dirty: bool,
    /// Commits not pushed to its upstream.
    pub ahead: u32,
    /// Its upstream's commits not here.
    pub behind: u32,
    /// Its upstream branch ('origin/main'), if it has one that's there.
    pub upstream: Option<String>,
}

impl GitStatus {
    /// Nothing here that isn't on its upstream too: no changes, nothing unpushed, and an upstream.
    pub fn all_pushed(&self) -> bool {
        !self.dirty && self.ahead == 0 && self.upstream.is_some()
    }
}

pub const TOKEN_ENV: &str = "WADD_GH_TOKEN";
/// git asks the helper for credentials; it answers from TOKEN_ENV.
pub const HELPER: &str = r#"!f() { echo username=x-access-token; echo "password=$WADD_GH_TOKEN"; }; f"#;
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
/// status and merge: no network.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(20);
/// stderr lines kept for an error message.
const TAIL: usize = 20;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("{0}")]
    Failed(String),
    #[error("git isn't installed on this machine")]
    NoGit,
    #[error("git took longer than {0} s")]
    Timeout(u64),
}

/// What update() did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    /// Fast-forwarded this many new commits.
    Updated(u32),
    Current,
    Dirty,
    /// This many commits not pushed.
    Ahead(u32),
    NoUpstream,
    NotGit,
    /// The fetch failed.
    Offline(String),
    /// The fast-forward failed.
    Stuck(String),
}

pub fn is_github(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit('@').next().unwrap_or_default();
    host.split(':').next().unwrap_or_default().eq_ignore_ascii_case("github.com")
}

/// How far a download is, from a "Receiving objects:  42% (42/100)" line.
pub fn parse_progress(line: &str) -> Option<f64> {
    let rest = line.split("Receiving objects:").nth(1)?.trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || !rest[digits.len()..].starts_with('%') {
        return None;
    }
    digits.parse::<f64>().ok().map(|p| p / 100.0)
}

/// `git status --porcelain=v2 --branch`. Untracked files count as dirty (a
/// fast-forward could trip on them). Without an upstream (or with one that's
/// gone) ahead and behind are 0 and upstream is None.
pub fn parse_status(text: &str) -> GitStatus {
    let mut st = GitStatus::default();
    let mut upstream = None;
    for line in text.lines() {
        if let Some(head) = line.strip_prefix("# branch.head ") {
            st.branch = (head != "(detached)").then(|| head.to_string());
        } else if let Some(u) = line.strip_prefix("# branch.upstream ") {
            upstream = Some(u.to_string());
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            let mut it = ab.split(' ').map(|n| n.trim_start_matches(['+', '-']).parse::<u32>().unwrap_or(0));
            st.ahead = it.next().unwrap_or(0);
            st.behind = it.next().unwrap_or(0);
            st.upstream = upstream.clone();
        } else if !line.is_empty() && !line.starts_with('#') {
            st.dirty = true;
        }
    }
    st
}

pub fn is_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

/// The last fatal:/error: line of git's stderr, else its last line.
fn why(err: &str, code: i32) -> String {
    let lines: Vec<&str> = err.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    lines
        .iter()
        .rev()
        .find(|l| l.starts_with("fatal:") || l.starts_with("error:"))
        .or(lines.last())
        .map(|l| l.to_string())
        .unwrap_or_else(|| format!("exit {code}"))
}

/// Removes a .part folder unless the clone worked.
struct Part(Option<PathBuf>);

impl Drop for Part {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = std::fs::remove_dir_all(p);
        }
    }
}

/// Runs git: `program` (git, or a stand-in in tests).
#[derive(Debug, Clone)]
pub struct Git {
    program: PathBuf,
    /// Where GitHub is (wad-config's `[github] web`; https://github.com): the
    /// one origin the token is ever offered to.
    github: String,
}

impl Default for Git {
    fn default() -> Self {
        Self { program: "git".into(), github: GITHUB.into() }
    }
}

const GITHUB: &str = "https://github.com";

impl Git {
    pub fn with_program(program: impl Into<PathBuf>) -> Self {
        Self { program: program.into(), ..Self::default() }
    }

    /// GitHub somewhere else (a stand-in GitHub in tests, an Enterprise
    /// server): its origin, e.g. "http://10.0.2.2:8399".
    pub fn with_github(mut self, origin: &str) -> Self {
        let origin = origin.trim_end_matches('/');
        if !origin.is_empty() {
            self.github = origin.to_string();
        }
        self
    }

    /// `url` is on this Git's GitHub (its origin exactly, then a path).
    pub fn is_github(&self, url: &str) -> bool {
        if self.github == GITHUB {
            return is_github(url);
        }
        url.strip_prefix(&self.github).is_some_and(|rest| rest.starts_with('/'))
    }

    /// The user git runs as: `uid` when wadd is root, else wadd's own.
    fn as_user(uid: Option<u32>) -> Option<u32> {
        uid.filter(|_| nix::unistd::geteuid().is_root())
    }

    /// git's HOME when running as the projects user: private, and theirs.
    fn home(projects_dir: &Path, uid: u32) -> std::io::Result<PathBuf> {
        let home = projects_dir.join(".home");
        std::fs::create_dir_all(&home)?;
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700))?;
        nix::unistd::chown(&home, Some(uid.into()), Some(uid.into())).map_err(std::io::Error::from)?;
        Ok(home)
    }

    fn command(&self, projects_dir: &Path, as_user: Option<u32>) -> std::io::Result<Command> {
        let mut cmd = Command::new(&self.program);
        cmd.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()))
            .env("LC_ALL", "C") // the output is parsed
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
            .stdin(Stdio::null())
            .kill_on_drop(true);
        match as_user {
            Some(uid) => {
                // std drops root's supplementary groups itself when it changes
                // uid (setgroups before setuid). Not in a pre_exec: those run
                // after the switch, as the user, where setgroups is refused
                // ("Operation not permitted": every git run failed).
                cmd.env("HOME", Self::home(projects_dir, uid)?).uid(uid).gid(uid);
            }
            None => {
                cmd.env("HOME", std::env::var_os("HOME").unwrap_or_else(|| "/".into()));
            }
        }
        Ok(cmd)
    }

    /// git clone `url` (at `git_ref`) into `dest`, through `dest.part`.
    /// `prepare` is called on the empty .part folder before git fills it
    /// (ownership and SELinux label, which the files then inherit).
    /// `on_line(text, fraction)`: each line git writes, and how far the
    /// download is when it's a "Receiving objects: NN%" line. Dropping the
    /// future stops git and leaves nothing behind.
    #[allow(clippy::too_many_arguments)]
    pub async fn clone_repo(
        &self,
        url: &str,
        git_ref: Option<&str>,
        dest: &Path,
        token: Option<&str>,
        mut on_line: impl FnMut(&str, Option<f64>),
        uid: Option<u32>,
        prepare: impl FnOnce(&Path),
    ) -> Result<(), Error> {
        if dest.exists() {
            return Err(Error::Failed(format!("{} already exists", dest.display())));
        }
        let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let part = dest.with_file_name(format!("{name}.part"));
        let _ = std::fs::remove_dir_all(&part); // left over from a crash
        let as_user = Self::as_user(uid);
        std::fs::create_dir_all(&part).map_err(|e| Error::Failed(format!("{}: {e}", part.display())))?;
        let mut guard = Part(Some(part.clone()));
        prepare(&part);
        if let Some(u) = as_user {
            let _ = nix::unistd::chown(&part, Some(u.into()), Some(u.into()));
        }
        let parent = dest.parent().unwrap_or(Path::new("/"));
        let mut cmd = self.command(parent, as_user).map_err(|e| Error::Failed(e.to_string()))?;
        if let Some(t) = token.filter(|_| self.is_github(url)) {
            cmd.env(TOKEN_ENV, t).args(["-c", "credential.helper=", "-c", &format!("credential.helper={HELPER}")]);
        }
        cmd.args(["clone", "--progress"]);
        if let Some(r) = git_ref {
            cmd.args(["--branch", r]);
        }
        cmd.arg("--").arg(url).arg(&part).current_dir(parent).stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound { Error::NoGit } else { Error::Failed(e.to_string()) }
        })?;
        let mut stderr = child.stderr.take().expect("piped");
        let mut tail: Vec<String> = vec![];
        let mut buf: Vec<u8> = vec![];
        let mut chunk = [0u8; 4096];
        let mut line = |raw: &[u8], tail: &mut Vec<String>| {
            let text = String::from_utf8_lossy(raw).trim().to_string();
            if text.is_empty() {
                return;
            }
            on_line(&text, parse_progress(&text));
            tail.push(text);
            if tail.len() > TAIL {
                tail.remove(0);
            }
        };
        // git redraws its progress with \r: each redraw is a line here.
        loop {
            let n = stderr.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            while let Some(i) = buf.iter().position(|b| *b == b'\r' || *b == b'\n') {
                let raw: Vec<u8> = buf.drain(..=i).collect();
                line(&raw[..raw.len() - 1], &mut tail);
            }
        }
        line(&buf, &mut tail);
        let status = child.wait().await.map_err(|e| Error::Failed(e.to_string()))?;
        if !status.success() {
            let why = tail
                .iter()
                .rev()
                .find(|t| t.starts_with("fatal:") || t.starts_with("error:") || t.starts_with("remote:"))
                .or(tail.last())
                .cloned()
                .unwrap_or_else(|| format!("exit {}", status.code().unwrap_or(-1)));
            return Err(Error::Failed(format!("git clone {url} failed: {why}")));
        }
        std::fs::rename(&part, dest).map_err(|e| Error::Failed(format!("{}: {e}", dest.display())))?;
        guard.0 = None;
        Ok(())
    }

    /// Runs git in a project folder: (code, stdout, stderr). With `token`,
    /// git can answer GitHub's password prompt with it (the helper is scoped
    /// to this Git's GitHub origin, so no other host sees it).
    pub async fn run(
        &self,
        path: &Path,
        args: &[&str],
        uid: Option<u32>,
        token: Option<&str>,
        timeout: Duration,
    ) -> Result<(i32, String, String), Error> {
        let as_user = Self::as_user(uid);
        let parent = path.parent().unwrap_or(Path::new("/"));
        let mut cmd = self.command(parent, as_user).map_err(|e| Error::Failed(e.to_string()))?;
        cmd.env("GIT_OPTIONAL_LOCKS", "0") // status: no index rewrite
            .env("GIT_CEILING_DIRECTORIES", parent) // never a repository the folder is in
            .arg("-c")
            .arg(format!("safe.directory={}", path.display()));
        if let Some(t) = token {
            cmd.env(TOKEN_ENV, t).args([
                "-c",
                "credential.helper=",
                "-c",
                &format!("credential.{}.helper={HELPER}", self.github),
            ]);
        }
        cmd.args(args).current_dir(path).stdout(Stdio::piped()).stderr(Stdio::piped());
        let child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound { Error::NoGit } else { Error::Failed(e.to_string()) }
        })?;
        // Timing out drops the child: kill_on_drop stops git.
        let out = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| Error::Timeout(timeout.as_secs()))?;
        let out = out.map_err(|e| Error::Failed(e.to_string()))?;
        Ok((
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ))
    }

    /// A project folder's git state from what's here (no fetch); None when
    /// it isn't a repository or git can't tell.
    pub async fn status(&self, path: &Path, uid: Option<u32>) -> Option<GitStatus> {
        if !is_repo(path) {
            return None;
        }
        match self.run(path, &["status", "--porcelain=v2", "--branch"], uid, None, LOCAL_TIMEOUT).await {
            Ok((0, out, _)) => Some(parse_status(&out)),
            Ok((code, _, err)) => {
                tracing::info!("git status in {}: {}", path.display(), why(&err, code));
                None
            }
            Err(e) => {
                tracing::info!("git status in {}: {e}", path.display());
                None
            }
        }
    }

    /// Fetch, then fast-forward when the copy is clean, has an upstream and
    /// nothing unpushed. Never fails for git's reasons: a copy that can't be
    /// brought up to date is still a copy to work in.
    pub async fn update(&self, path: &Path, token: Option<&str>, uid: Option<u32>, fetch_timeout: Duration) -> Update {
        if !is_repo(path) {
            return Update::NotGit;
        }
        match self.run(path, &["fetch", "--quiet"], uid, token, fetch_timeout).await {
            Ok((0, _, _)) => {}
            Ok((code, _, err)) => return Update::Offline(why(&err, code)),
            Err(Error::Timeout(s)) => return Update::Offline(format!("git fetch took longer than {s} s")),
            Err(e) => return Update::Offline(e.to_string()),
        }
        let Some(st) = self.status(path, uid).await else { return Update::NotGit };
        if st.dirty {
            return Update::Dirty;
        }
        if st.upstream.is_none() {
            return Update::NoUpstream;
        }
        if st.ahead > 0 {
            return Update::Ahead(st.ahead);
        }
        if st.behind == 0 {
            return Update::Current;
        }
        match self.run(path, &["merge", "--ff-only", "--quiet", "@{u}"], uid, None, LOCAL_TIMEOUT).await {
            Ok((0, _, _)) => Update::Updated(st.behind),
            Ok((code, _, err)) => Update::Stuck(why(&err, code)),
            Err(e) => Update::Stuck(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests;
