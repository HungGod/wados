//! Projects' secret files: a GitHub project's files kept out of git (`.env`,
//! a key, a local config), kept by the account and put into every clone of
//! that repository on every machine of it (v1's, on WadOS's clones).
//!
//! - Each is `f-<12 hex>` ([`name_of`]: the repo and the path). Its bytes
//!   are the podman secret of that name here (never in a log, a job, an
//!   event or wadd's files), and, once the machine is linked, the account's
//!   `users/<uid>/secrets/<name>` (account.rs writes it: [`AccountStore`]),
//!   which every machine of the account fetches. Text as it is, anything
//!   else in base64.
//! - It goes to `<path>` in every clone of its repo here
//!   (`<projects_dir>/<wadspace>/<project>`, what the wadspace has at
//!   `~/Desktop/<name>`): put there when it's missing, updated when the
//!   copy there is still the one put there, **never written over a change
//!   made there** (that's asked about instead: keep that version for every
//!   machine?), and not put back once it's deleted there, until it changes.
//!   Right after a clone is made or found as a wadspace opens
//!   ([`SecretFiles::place`]), and every minute ([`SecretFiles::run`]).
//! - Clones are looked at for likely secret files nobody keeps yet
//!   ([`likely_secret`]: what git ignores and doesn't track, named like
//!   one, small): Settings and the phone ask about them. Any other file is
//!   kept only when asked for.
//!
//! A clone is the wadspace's (it works in it, and can put links or git
//! config there), so all of this runs as the projects user (abc, 1000),
//! never as root: a link in a clone can't make wadd read or write anything
//! that user couldn't.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use wad_model::{ProjectKind, Wadspace};
use wad_proto::SecretOrigin;
use wad_proto::{ApiError, ErrorCode, Event, SecretFile, SecretFileAsk, SecretFileAskKind, SecretFiles as Status};

use crate::backend::Backend;
use crate::events::Bus;
use crate::library::Docs;
use crate::store::safe_id;

/// A secret file's size, at most.
pub const MAX_BYTES: usize = 64 * 1024;
const BOOK: &str = "secret-files.json";
/// How often the clones are looked at.
const EVERY: Duration = Duration::from_secs(60);
/// Each script, at most.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(20);
/// Likely files asked about, at most, per clone.
const MAX_LIKELY: usize = 20;

/// What runs in a clone (`sh -c <script> sh <dir> <args>`), as the projects user.
pub mod scripts {
    /// $1 cloned (it has .git).
    pub const CLONED: &str = r#"[ -d "$1/.git" ]"#;
    /// In $1: each path's sha256, or "-" when it isn't a plain file.
    pub const DIGESTS: &str = r#"cd "$1" || exit 1; shift
for p; do if [ -f "$p" ] && [ ! -L "$p" ]; then sha256sum < "$p" | cut -c1-64; else echo -; fi; done"#;
    /// In $1: each path's size, or "-".
    pub const SIZES: &str = r#"cd "$1" || exit 1; shift
for p; do if [ -f "$p" ] && [ ! -L "$p" ]; then stat -c %s -- "$p"; else echo -; fi; done"#;
    /// In $1: $2 written from stdin (base64), private, all at once; never
    /// through a link.
    pub const WRITE: &str = r#"cd "$1" || exit 1; umask 077
[ -L "$2" ] && { echo "$2 is a link" >&2; exit 1; }
d=$(dirname -- "$2"); mkdir -p -- "$d" && base64 -d > "$2.wadspaces-new" && mv -f -- "$2.wadspaces-new" "$2""#;
    /// In $1: $2 in base64, when it's a plain file of at most $3 bytes.
    pub const READ: &str =
        r#"cd "$1" && [ -f "$2" ] && [ ! -L "$2" ] && [ "$(stat -c %s -- "$2")" -le "$3" ] && base64 -w0 -- "$2""#;
    /// In $1: what git ignores and doesn't track (folders as one, with a /).
    /// The clone's own git config can't run anything (fsmonitor off).
    pub const IGNORED: &str =
        r#"cd "$1" && git -c core.fsmonitor=false ls-files -z --others --ignored --exclude-standard --directory"#;
}

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError::new(ErrorCode::BadRequest, msg)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn now_ms() -> i64 {
    crate::jobs::now_ms()
}

/// The secret a project's file is kept as.
pub fn name_of(repo: &str, path: &str) -> String {
    format!("f-{}", &digest(format!("{}\n{path}", repo.to_ascii_lowercase()).as_bytes())[..12])
}

/// A secret file's secret: `f-` and 12 hex.
pub fn is_name(name: &str) -> bool {
    name.len() == 14 && name.starts_with("f-") && name[2..].bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A GitHub repo: owner/name.
pub fn check_repo(repo: &str) -> Result<String, String> {
    let ok =
        |s: &str| (1..=100).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    match repo.split_once('/') {
        Some((o, n)) if ok(o) && ok(n) && !n.contains('/') && o != "." && o != ".." && n != "." && n != ".." => {
            Ok(repo.to_string())
        }
        _ => Err(format!("{repo:?} isn't a GitHub repo (owner/name)")),
    }
}

/// A path in a project: relative, plain, not in .git, at most 4 deep.
pub fn check_path(path: &str) -> Result<String, String> {
    let parts: Vec<&str> = path.split('/').collect();
    let plain = |p: &&str| !p.is_empty() && *p != "." && *p != ".." && *p != ".git" && !p.starts_with('-');
    if path.len() > 200
        || parts.len() > 4
        || !parts.iter().all(plain)
        || path.chars().any(|c| c.is_control() || c == '\\')
        || path.ends_with(".wadspaces-new")
    {
        return Err(format!("{path:?} isn't a file's place in a project (like .env or config/secrets.toml)"));
    }
    Ok(path.to_string())
}

/// Named like a secret file, and at most 3 deep: what a clone is looked at
/// for (only among what git ignores and doesn't track). A plain
/// config.toml isn't: that's kept only when asked for.
pub fn likely_secret(path: &str) -> bool {
    if path.split('/').count() > 3 {
        return false;
    }
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    if name == ".env" {
        return true;
    }
    if let Some(rest) = name.strip_prefix(".env.") {
        return !["example", "sample", "template", "dist", "defaults"].contains(&rest);
    }
    const NAMES: [&str; 5] = [".npmrc", ".pypirc", ".netrc", ".dev.vars", "local.settings.json"];
    const ENDS: [&str; 6] = [".pem", ".key", ".p12", ".pfx", ".jks", ".keystore"];
    const LOCAL: [&str; 4] = [".local.toml", ".local.json", ".local.yaml", ".local.yml"];
    NAMES.contains(&name.as_str())
        || ENDS.iter().any(|e| name.ends_with(e))
        || LOCAL.iter().any(|e| name.ends_with(e))
        || name.starts_with("credentials") && name.ends_with(".json")
        || name.starts_with("service-account") && name.ends_with(".json")
        || name.contains("secret")
        || name.contains("credential")
}

/// What to do with a kept file in a clone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// Put the kept one there.
    Write,
    /// It's the kept one (or was deleted there, or kept there on purpose).
    Nothing,
    /// Changed there: ask whether to keep that for every machine.
    Changed,
}

/// From its digest in the clone now (None: missing), the digest last put
/// there, the kept one's, and a change there the person said to keep in
/// that clone only.
pub fn decide(here: Option<&str>, put: Option<&str>, kept: &str, here_only: Option<&str>) -> Act {
    match here {
        // Never put there, or deleted there but the kept one changed since.
        None if put != Some(kept) => Act::Write,
        None => Act::Nothing,
        Some(h) if h == kept => Act::Nothing,
        // Still what was put there: the kept one changed since.
        Some(h) if put == Some(h) => Act::Write,
        Some(h) if here_only == Some(h) => Act::Nothing,
        Some(_) => Act::Changed,
    }
}

/// Where the owner's account keeps them (account.rs).
#[async_trait]
pub trait AccountStore: Send + Sync {
    fn linked(&self) -> bool;
    async fn put_file(&self, name: &str, file: &AccountFile) -> Result<(), String>;
    async fn delete_file(&self, name: &str) -> Result<(), String>;
}

/// One in the account, as its document has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountFile {
    /// Text, or with `base64` the bytes in base64.
    pub value: String,
    pub base64: bool,
    pub repo: String,
    pub path: String,
    /// When it last changed (ms), as the account says.
    pub updated_at: Option<i64>,
}

/// What's known of a kept file here (its contents are in podman).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Meta {
    repo: String,
    path: String,
    #[serde(default)]
    base64: bool,
    size: u64,
    /// Of its bytes.
    digest: String,
    #[serde(default)]
    updated_at: Option<i64>,
    /// Kept here only (not linked then): it goes to the account once linked.
    #[serde(default)]
    local: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Book {
    #[serde(default)]
    files: BTreeMap<String, Meta>,
    /// "<wadspace>/<project>\n<name>" -> the digest last put there.
    #[serde(default)]
    put: BTreeMap<String, String>,
    /// "<repo, lower case>\n<path>": likely ones the person said no to.
    #[serde(default)]
    dismissed: BTreeSet<String>,
    /// "<wadspace>/<project>\n<name>" -> a change there to keep there only.
    #[serde(default)]
    here_only: BTreeMap<String, String>,
}

/// One GitHub project's clone here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectClone {
    pub wadspace: String,
    pub wadspace_name: String,
    pub project: String,
    pub project_name: String,
    pub repo: String,
}

impl ProjectClone {
    fn key(&self, name: &str) -> String {
        format!("{}/{}\n{name}", self.wadspace, self.project)
    }

    fn ask_id(&self, path: &str, kind: SecretFileAskKind) -> String {
        format!("a-{}", &digest(format!("{}\n{}\n{path}\n{kind:?}", self.wadspace, self.project).as_bytes())[..12])
    }
}

/// A wadspace's GitHub projects.
pub fn clones_of(w: &Wadspace) -> Vec<ProjectClone> {
    w.look
        .projects
        .iter()
        .filter_map(|p| match &p.kind {
            ProjectKind::Github { repo, .. } if check_repo(repo).is_ok() && safe_id(&p.id) && safe_id(&w.id) => {
                Some(ProjectClone {
                    wadspace: w.id.clone(),
                    wadspace_name: w.name.clone(),
                    project: p.id.clone(),
                    project_name: p.as_name.clone(),
                    repo: repo.clone(),
                })
            }
            _ => None,
        })
        .collect()
}

fn same_repo(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// A script's answer.
#[derive(Debug, Clone, Default)]
pub struct Out {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

/// Runs the scripts (a test's, or the machine's [`AsUser`]).
#[async_trait]
pub trait Shell: Send + Sync {
    async fn sh(&self, script: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<Out, String>;
}

/// `sh` as the projects user (when wadd is root; else as wadd).
pub struct AsUser {
    pub uid: Option<u32>,
    /// HOME for it (git's): `<projects_dir>/.home`, as wad-git makes it.
    pub home: PathBuf,
}

#[async_trait]
impl Shell for AsUser {
    async fn sh(&self, script: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<Out, String> {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
            .arg(script)
            .arg("sh")
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("HOME", &self.home)
            .current_dir("/")
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(uid) = self.uid {
            // std drops root's supplementary groups itself (setgroups before setuid).
            cmd.uid(uid).gid(uid);
        }
        let mut child = cmd.spawn().map_err(|e| format!("sh: {e}"))?;
        if let Some(input) = stdin {
            let mut pipe = child.stdin.take().ok_or("sh: no stdin")?;
            pipe.write_all(input).await.map_err(|e| format!("sh: {e}"))?;
            drop(pipe);
        }
        let out = tokio::time::timeout(SCRIPT_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| "it took too long".to_string())?
            .map_err(|e| format!("sh: {e}"))?;
        Ok(Out {
            code: out.status.code().unwrap_or(-1),
            stdout: out.stdout,
            stderr: String::from_utf8_lossy(&out.stderr).chars().take(300).collect(),
        })
    }
}

pub struct SecretFiles {
    backend: Arc<dyn Backend>,
    docs: Arc<Docs>,
    bus: Bus,
    shell: Arc<dyn Shell>,
    projects_dir: PathBuf,
    file: PathBuf,
    book: Mutex<Book>,
    /// What's asking, by clone ("<wadspace>/<project>").
    asks: Mutex<HashMap<String, Vec<SecretFileAsk>>>,
    /// A changed one's digest in its clone, by ask id.
    asked_digest: Mutex<HashMap<String, String>>,
    account: Mutex<Option<Weak<dyn AccountStore>>>,
    /// One look at a time.
    looking: tokio::sync::Mutex<()>,
    poke: tokio::sync::Notify,
}

impl SecretFiles {
    pub fn new(
        backend: Arc<dyn Backend>,
        docs: Arc<Docs>,
        bus: Bus,
        shell: Arc<dyn Shell>,
        state_dir: &Path,
        projects_dir: &Path,
    ) -> Arc<Self> {
        let file = state_dir.join(BOOK);
        let book = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let me = Arc::new(Self {
            backend,
            docs,
            bus,
            shell,
            projects_dir: projects_dir.to_path_buf(),
            file,
            book: Mutex::new(book),
            asks: Mutex::default(),
            asked_digest: Mutex::default(),
            account: Mutex::new(None),
            looking: tokio::sync::Mutex::new(()),
            poke: tokio::sync::Notify::new(),
        });
        me.publish();
        me
    }

    /// The machine's: scripts as the projects user.
    pub fn machine(backend: Arc<dyn Backend>, docs: Arc<Docs>, bus: Bus, config: &wad_config::Config) -> Arc<Self> {
        let d = &config.daemon;
        let root = nix::unistd::geteuid().is_root();
        let shell = Arc::new(AsUser { uid: root.then_some(d.projects_uid), home: d.projects_dir.join(".home") });
        Self::new(backend, docs, bus, shell, &d.state_dir, &d.projects_dir)
    }

    /// The account they're kept in.
    pub fn set_account(&self, account: Weak<dyn AccountStore>) {
        *self.account.lock().unwrap() = Some(account);
    }

    fn account(&self) -> Option<Arc<dyn AccountStore>> {
        self.account.lock().unwrap().as_ref().and_then(Weak::upgrade).filter(|a| a.linked())
    }

    fn save_book(&self) {
        let bytes = serde_json::to_vec_pretty(&*self.book.lock().unwrap()).expect("json");
        if let Err(e) = crate::store::write_atomic_mkdir(&self.file, &bytes) {
            tracing::warn!("{}: {e}", self.file.display());
        }
    }

    fn clone_dir(&self, c: &ProjectClone) -> PathBuf {
        self.projects_dir.join(&c.wadspace).join(&c.project)
    }

    /// Every GitHub project's clone of the live wadspaces.
    fn clones(&self) -> Vec<ProjectClone> {
        self.docs.live().iter().flat_map(clones_of).collect()
    }

    // ------------------------------------------------------------ state

    pub fn status(&self) -> Status {
        let clones = self.clones();
        let book = self.book.lock().unwrap();
        let files = book
            .files
            .iter()
            .map(|(name, m)| {
                let mut used_by: Vec<String> =
                    clones.iter().filter(|c| same_repo(&c.repo, &m.repo)).map(|c| c.wadspace.clone()).collect();
                used_by.dedup();
                SecretFile {
                    name: name.clone(),
                    repo: m.repo.clone(),
                    path: m.path.clone(),
                    size: m.size,
                    updated_at: m.updated_at,
                    used_by,
                    origin: if m.local { SecretOrigin::Local } else { SecretOrigin::Account },
                }
            })
            .collect();
        let mut asks: Vec<SecretFileAsk> = self.asks.lock().unwrap().values().flatten().cloned().collect();
        asks.sort_by(|a, b| (&a.wadspace, &a.project, &a.path).cmp(&(&b.wadspace, &b.project, &b.path)));
        drop(book);
        Status { files, asks, account: self.account().is_some() }
    }

    pub fn publish(&self) {
        let now = Event::SecretFiles(self.status());
        if self.bus.latest("secretFiles").as_ref() != Some(&now) {
            self.bus.publish(now);
        }
    }

    /// The account's secret files (account.rs's sync): what's known of them
    /// follows the account, their bytes into podman (only what changed).
    /// One kept here before the machine was linked goes there now.
    pub async fn from_account(&self, files: Vec<(String, AccountFile)>) {
        let mut pending = vec![];
        let mut gone = vec![];
        let mut changed = vec![];
        {
            let book = self.book.lock().unwrap();
            let names: BTreeSet<&String> = files.iter().map(|(n, _)| n).collect();
            for (n, m) in &book.files {
                if !m.local && !names.contains(n) {
                    gone.push(n.clone());
                }
            }
            for (name, f) in &files {
                let Some(bytes) = decode(&f.value, f.base64) else { continue };
                let d = digest(&bytes);
                if book.files.get(name).is_some_and(|o| o.digest == d && !o.local) {
                    continue;
                }
                changed.push((name.clone(), f.clone(), bytes.len() as u64, d));
            }
            for (name, m) in book.files.iter().filter(|(n, m)| m.local && !names.contains(n)) {
                pending.push((name.clone(), m.clone()));
            }
        }
        for (name, f, size, d) in changed {
            // Its bytes first: the book never says there's one podman hasn't.
            if let Err(e) = self.backend.create_secret(&name, f.value.as_bytes()).await {
                tracing::warn!("secret file {name} from the account: {e}");
                continue;
            }
            let meta = Meta {
                repo: f.repo.clone(),
                path: f.path.clone(),
                base64: f.base64,
                size,
                digest: d,
                updated_at: f.updated_at.or(Some(now_ms())),
                local: false,
            };
            self.book.lock().unwrap().files.insert(name, meta);
        }
        for name in gone {
            tracing::info!("secret file {name}: no longer in the account");
            self.drop_file(&name).await;
        }
        if let Some(account) = self.account() {
            for (name, m) in pending {
                let Ok(Some(value)) = self.backend.secret_value(&name).await else { continue };
                let f = AccountFile { value, base64: m.base64, repo: m.repo, path: m.path, updated_at: m.updated_at };
                match account.put_file(&name, &f).await {
                    Ok(()) => {
                        tracing::info!("secret file {name}: kept here before linking, now in the account");
                        if let Some(m) = self.book.lock().unwrap().files.get_mut(&name) {
                            m.local = false;
                        }
                    }
                    Err(e) => tracing::warn!("secret file {name} into the account: {e}"),
                }
            }
        }
        self.save_book();
        self.publish();
        self.poke.notify_one();
    }

    /// Linked to another owner now: the last one's files mustn't become
    /// theirs (the copies in clones stay; the ones kept here only go to the
    /// new account).
    pub async fn forget_account(&self) {
        let names: Vec<String> =
            self.book.lock().unwrap().files.iter().filter(|(_, m)| !m.local).map(|(n, _)| n.clone()).collect();
        for n in names {
            self.drop_file(&n).await;
        }
        self.save_book();
        self.publish();
    }

    /// No longer kept here: its secret and what's known of it.
    async fn drop_file(&self, name: &str) {
        if let Err(e) = self.backend.delete_secret(name).await {
            tracing::warn!("secret file {name}: {e}");
        }
        let mut book = self.book.lock().unwrap();
        book.files.remove(name);
        let suffix = format!("\n{name}");
        book.put.retain(|k, _| !k.ends_with(&suffix));
        book.here_only.retain(|k, _| !k.ends_with(&suffix));
    }

    // ---------------------------------------------------------- keeping

    /// Keeps a project's file (new, or new contents), here and in the
    /// account: what's kept.
    pub async fn keep(&self, repo: &str, path: &str, bytes: &[u8]) -> Result<SecretFile, ApiError> {
        let repo = check_repo(repo).map_err(bad)?;
        let path = check_path(path).map_err(bad)?;
        if bytes.is_empty() {
            return Err(bad("That file is empty"));
        }
        if bytes.len() > MAX_BYTES {
            return Err(bad("A secret file can be up to 64 KB"));
        }
        let name = name_of(&repo, &path);
        let (value, b64) = match std::str::from_utf8(bytes) {
            Ok(text) => (text.to_string(), false),
            Err(_) => (base64::engine::general_purpose::STANDARD.encode(bytes), true),
        };
        let account = self.account();
        let at = now_ms();
        if let Some(a) = &account {
            let f = AccountFile {
                value: value.clone(),
                base64: b64,
                repo: repo.clone(),
                path: path.clone(),
                updated_at: Some(at),
            };
            a.put_file(&name, &f)
                .await
                .map_err(|e| ApiError::new(ErrorCode::Upstream, format!("Your account didn't keep it: {e}")))?;
        }
        self.backend
            .create_secret(&name, value.as_bytes())
            .await
            .map_err(|e| ApiError::new(ErrorCode::Upstream, format!("Podman didn't keep it: {e}")))?;
        {
            let mut book = self.book.lock().unwrap();
            let meta = Meta {
                repo: repo.clone(),
                path: path.clone(),
                base64: b64,
                size: bytes.len() as u64,
                digest: digest(bytes),
                updated_at: Some(at),
                local: account.is_none(),
            };
            book.files.insert(name.clone(), meta);
            book.dismissed.remove(&format!("{}\n{path}", repo.to_ascii_lowercase()));
        }
        self.save_book();
        tracing::info!("secret file {repo} {path} kept ({name})");
        self.publish();
        self.poke.notify_one();
        self.status().files.into_iter().find(|f| f.name == name).ok_or_else(|| ApiError::internal("the kept file went"))
    }

    /// No longer kept, here or in the account (the copies in clones stay).
    pub async fn remove(&self, name: &str) -> Result<(), ApiError> {
        let local = match self.book.lock().unwrap().files.get(name) {
            Some(m) => m.local,
            None => return Err(ApiError::not_found("There's no such secret file")),
        };
        if !local && let Some(a) = self.account() {
            a.delete_file(name)
                .await
                .map_err(|e| ApiError::new(ErrorCode::Upstream, format!("Your account didn't remove it: {e}")))?;
        }
        self.drop_file(name).await;
        self.save_book();
        self.publish();
        Ok(())
    }

    /// The ask answered: a new one kept, or a changed one's version kept
    /// for every machine; or not (asked no more, or that version stays in
    /// that clone only).
    pub async fn answer(&self, id: &str, keep: bool) -> Result<Status, ApiError> {
        let ask = self.asks.lock().unwrap().values().flatten().find(|a| a.id == id).cloned();
        let ask = ask.ok_or_else(|| ApiError::not_found("Nothing's asking that now"))?;
        let c = self
            .clones()
            .into_iter()
            .find(|c| c.wadspace == ask.wadspace && c.project == ask.project)
            .ok_or_else(|| ApiError::conflict("That project isn't on this machine any more"))?;
        if keep {
            let bytes = self.read_in(&c, &ask.path).await?;
            let kept = self.keep(&ask.repo, &ask.path, &bytes).await?;
            // That's what's there now.
            self.book.lock().unwrap().put.insert(c.key(&kept.name), digest(&bytes));
        } else {
            let mut book = self.book.lock().unwrap();
            match ask.kind {
                SecretFileAskKind::New => {
                    book.dismissed.insert(format!("{}\n{}", ask.repo.to_ascii_lowercase(), ask.path));
                }
                SecretFileAskKind::Changed => {
                    let name = name_of(&ask.repo, &ask.path);
                    let here = self.asked_digest.lock().unwrap().get(&ask.id).cloned().unwrap_or_default();
                    book.here_only.insert(c.key(&name), here);
                }
            }
        }
        self.save_book();
        for list in self.asks.lock().unwrap().values_mut() {
            list.retain(|a| a.id != id);
        }
        self.publish();
        Ok(self.status())
    }

    /// A file's bytes in a clone.
    async fn read_in(&self, c: &ProjectClone, path: &str) -> Result<Vec<u8>, ApiError> {
        let dir = self.clone_dir(c);
        let max = MAX_BYTES.to_string();
        let out = self
            .shell
            .sh(scripts::READ, &[&dir.to_string_lossy(), path, &max], None)
            .await
            .map_err(|e| ApiError::new(ErrorCode::Upstream, e))?;
        if out.code != 0 {
            return Err(bad(format!("{path} isn't there, or is over 64 KB")));
        }
        base64::engine::general_purpose::STANDARD
            .decode(out.stdout.trim_ascii())
            .map_err(|e| ApiError::internal(e.to_string()))
    }

    // ----------------------------------------------------------- looking

    /// Looks at the clones every minute, or sooner when something changed,
    /// for as long as wadd runs.
    pub async fn run(self: Arc<Self>) {
        loop {
            self.look_all().await;
            tokio::select! {
                _ = tokio::time::sleep(EVERY) => {}
                _ = self.poke.notified() => {}
            }
        }
    }

    /// Every clone here, and the asks of the ones gone dropped.
    pub async fn look_all(&self) {
        let clones = self.clones();
        let gone = {
            let mut asks = self.asks.lock().unwrap();
            let before = asks.len();
            asks.retain(|k, _| clones.iter().any(|c| &format!("{}/{}", c.wadspace, c.project) == k));
            asks.len() != before
        };
        if gone {
            self.publish();
        }
        for c in clones {
            self.look(&c).await;
        }
    }

    /// A wadspace's clones, just made or found as it opens.
    pub async fn place(&self, ws: &Wadspace, project: &str) {
        if let Some(c) = clones_of(ws).into_iter().find(|c| c.project == project) {
            self.look(&c).await;
        }
    }

    /// One clone: the kept files put there, and what's likely asked about.
    async fn look(&self, c: &ProjectClone) {
        let _one = self.looking.lock().await;
        let asks = match self.look_clone(c).await {
            Ok(a) => a,
            Err(e) => {
                tracing::debug!("secret files in {}/{}: {e}", c.wadspace, c.project);
                return;
            }
        };
        let k = format!("{}/{}", c.wadspace, c.project);
        let changed = {
            let mut all = self.asks.lock().unwrap();
            let same = all.get(&k).is_some_and(|old| *old == asks) || (asks.is_empty() && !all.contains_key(&k));
            all.insert(k, asks);
            !same
        };
        self.save_book();
        if changed {
            self.publish();
        }
    }

    async fn look_clone(&self, c: &ProjectClone) -> Result<Vec<SecretFileAsk>, String> {
        let dir = self.clone_dir(c).to_string_lossy().into_owned();
        // Not cloned yet: nothing to put there, nothing to look at.
        if self.shell.sh(scripts::CLONED, &[&dir], None).await?.code != 0 {
            return Ok(vec![]);
        }
        let kept: Vec<(String, Meta)> = {
            let book = self.book.lock().unwrap();
            book.files
                .iter()
                .filter(|(_, m)| same_repo(&m.repo, &c.repo))
                .map(|(n, m)| (n.clone(), m.clone()))
                .collect()
        };
        let mut asks = vec![];
        if !kept.is_empty() {
            let mut args = vec![dir.as_str()];
            args.extend(kept.iter().map(|(_, m)| m.path.as_str()));
            let out = self.shell.sh(scripts::DIGESTS, &args, None).await?;
            let here: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(String::from).collect();
            if out.code != 0 || here.len() != kept.len() {
                return Err(format!("looking at the kept files: {}", out.stderr.trim()));
            }
            for ((name, m), h) in kept.iter().zip(&here) {
                let h = (h != "-").then_some(h.as_str());
                let k = c.key(name);
                let (put, here_only) = {
                    let book = self.book.lock().unwrap();
                    (book.put.get(&k).cloned(), book.here_only.get(&k).cloned())
                };
                match decide(h, put.as_deref(), &m.digest, here_only.as_deref()) {
                    Act::Write => match self.write(&dir, name, m).await {
                        Ok(()) => {
                            self.book.lock().unwrap().put.insert(k, m.digest.clone());
                            tracing::info!("secret file {} put in {}/{}", m.path, c.wadspace, c.project);
                        }
                        Err(e) => tracing::warn!("secret file {} in {}/{}: {e}", m.path, c.wadspace, c.project),
                    },
                    Act::Nothing => {
                        if h == Some(m.digest.as_str()) {
                            self.book.lock().unwrap().put.insert(k, m.digest.clone());
                        }
                    }
                    Act::Changed => {
                        let id = c.ask_id(&m.path, SecretFileAskKind::Changed);
                        self.asked_digest.lock().unwrap().insert(id.clone(), h.unwrap_or_default().to_string());
                        asks.push(self.ask(c, id, SecretFileAskKind::Changed, &m.path, m.size));
                    }
                }
            }
        }
        asks.extend(self.likely(c, &dir, &kept).await?);
        Ok(asks)
    }

    fn ask(&self, c: &ProjectClone, id: String, kind: SecretFileAskKind, path: &str, size: u64) -> SecretFileAsk {
        SecretFileAsk {
            id,
            kind,
            wadspace: c.wadspace.clone(),
            wadspace_name: c.wadspace_name.clone(),
            project: c.project.clone(),
            project_name: c.project_name.clone(),
            repo: c.repo.clone(),
            path: path.to_string(),
            size,
        }
    }

    /// Puts a kept file in place (made private; through a temporary name,
    /// so it's never half there).
    async fn write(&self, dir: &str, name: &str, m: &Meta) -> Result<(), String> {
        let value = self.backend.secret_value(name).await?.ok_or("its secret isn't here")?;
        let bytes = decode(&value, m.base64).ok_or("its secret isn't base64")?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let out = self.shell.sh(scripts::WRITE, &[dir, m.path.as_str()], Some(b64.as_bytes())).await?;
        if out.code != 0 {
            return Err(format!("putting it there: {}", out.stderr.trim()));
        }
        Ok(())
    }

    /// The likely secret files there nobody's kept or said no to: what git
    /// ignores and doesn't track, named like one, small and not empty.
    async fn likely(&self, c: &ProjectClone, dir: &str, kept: &[(String, Meta)]) -> Result<Vec<SecretFileAsk>, String> {
        let out = self.shell.sh(scripts::IGNORED, &[dir], None).await?;
        if out.code != 0 {
            return Err(format!("git: {}", out.stderr.trim()));
        }
        let dismissed = self.book.lock().unwrap().dismissed.clone();
        let candidates: Vec<String> = out
            .stdout
            .split(|b| *b == 0)
            .filter_map(|p| std::str::from_utf8(p).ok())
            .filter(|p| !p.is_empty() && !p.ends_with('/') && likely_secret(p) && check_path(p).is_ok())
            .filter(|p| !kept.iter().any(|(_, m)| m.path == *p))
            .filter(|p| !dismissed.contains(&format!("{}\n{p}", c.repo.to_ascii_lowercase())))
            .map(String::from)
            .take(MAX_LIKELY)
            .collect();
        if candidates.is_empty() {
            return Ok(vec![]);
        }
        let mut args = vec![dir];
        args.extend(candidates.iter().map(String::as_str));
        let out = self.shell.sh(scripts::SIZES, &args, None).await?;
        let sizes: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(String::from).collect();
        if sizes.len() != candidates.len() {
            return Err("sizing what's likely".into());
        }
        Ok(candidates
            .into_iter()
            .zip(sizes)
            .filter_map(|(path, size)| {
                let size: u64 = size.parse().ok()?;
                (size > 0 && size as usize <= MAX_BYTES).then(|| {
                    let id = c.ask_id(&path, SecretFileAskKind::New);
                    self.ask(c, id, SecretFileAskKind::New, &path, size)
                })
            })
            .collect())
    }
}

fn decode(value: &str, b64: bool) -> Option<Vec<u8>> {
    if b64 {
        base64::engine::general_purpose::STANDARD.decode(value.trim()).ok()
    } else {
        Some(value.as_bytes().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        let n = name_of("HungGod/Writing", ".env");
        assert!(is_name(&n), "{n}");
        assert_eq!(n, name_of("hunggod/writing", ".env"));
        assert_ne!(n, name_of("hunggod/writing", ".env.local"));
        assert!(!is_name("github_token") && !is_name("f-0123") && !is_name("f-0123456789AB"));
    }

    #[test]
    fn repos_and_paths() {
        assert!(check_repo("HungGod/wad-spaces").is_ok());
        for r in ["nope", "a/b/c", "/b", "a b/c", "", "../x", "a/.."] {
            assert!(check_repo(r).is_err(), "{r}");
        }
        for p in [".env", "config/secrets.toml", "a/b/c/d.key"] {
            assert!(check_path(p).is_ok(), "{p}");
        }
        for p in [
            "",
            "/etc/passwd",
            "../x",
            "a/../b",
            ".git/config",
            "a//b",
            "a/b/c/d/e",
            "./x",
            "x\\y",
            "a\nb",
            "-rf",
            ".env.wadspaces-new",
        ] {
            assert!(check_path(p).is_err(), "{p:?}");
        }
    }

    #[test]
    fn what_looks_like_a_secret_file() {
        for p in [
            ".env",
            ".env.local",
            "backend/.env.production",
            "certs/server.pem",
            "id.key",
            "credentials.json",
            "service-account-prod.json",
            "secrets.toml",
            "config/app.local.yaml",
            ".npmrc",
            ".dev.vars",
        ] {
            assert!(likely_secret(p), "{p}");
        }
        for p in
            [".env.example", ".env.sample", "config.toml", "Cargo.toml", "settings.json", "a/b/c/.env", "README.md"]
        {
            assert!(!likely_secret(p), "{p}");
        }
    }

    #[test]
    fn what_happens_to_a_kept_file_in_a_clone() {
        use Act::*;
        // Never there: put it there.
        assert_eq!(decide(None, None, "k", None), Write);
        // Deleted there: left alone, unless the kept one changed since.
        assert_eq!(decide(None, Some("k"), "k", None), Nothing);
        assert_eq!(decide(None, Some("old"), "k", None), Write);
        // The kept one: nothing to do.
        assert_eq!(decide(Some("k"), None, "k", None), Nothing);
        // Still what was put there, and the kept one changed: updated.
        assert_eq!(decide(Some("old"), Some("old"), "k", None), Write);
        // Changed there: asked about, unless kept there on purpose.
        assert_eq!(decide(Some("mine"), Some("k"), "k", None), Changed);
        assert_eq!(decide(Some("mine"), Some("k"), "k", Some("mine")), Nothing);
        assert_eq!(decide(Some("mine2"), Some("k"), "k", Some("mine")), Changed);
        // There before it was kept, and different: never written over.
        assert_eq!(decide(Some("theirs"), None, "k", None), Changed);
    }

    /// The starter wadspace's GitHub project, cloned (a git repo with a
    /// .gitignore), and secret files over a real `sh` (as whoever runs the
    /// tests), with no account.
    #[tokio::test]
    async fn kept_files_go_into_clones_and_changes_there_are_asked_about() {
        use crate::fake::Fake;
        use wad_proto::{MachineInfo, Profile};
        let d = tempfile::tempdir().unwrap();
        let bus =
            Bus::new(MachineInfo { id: "m".into(), name: "M".into(), version: "0".into(), profile: Profile::User });
        let docs =
            Arc::new(Docs::open(crate::store::Store::new(&d.path().join("state")).unwrap(), bus.clone(), "m").unwrap());
        let ws = docs.live().into_iter().next().expect("the starter");
        let c = clones_of(&ws).into_iter().next().expect("its GitHub project");
        let projects = d.path().join("projects");
        let dir = projects.join(&c.wadspace).join(&c.project);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git").args(args).current_dir(&dir).output().unwrap().status.success();
            assert!(ok, "git {args:?}");
        };
        git(&["init", "-q"]);
        std::fs::write(dir.join(".gitignore"), ".env*\n*.key\n").unwrap();
        std::fs::write(dir.join(".env.local"), "TOKEN=here\n").unwrap();
        let fake = Arc::new(Fake::new());
        let shell = Arc::new(AsUser { uid: None, home: d.path().join("home") });
        let f = SecretFiles::new(fake.clone(), docs, bus, shell, &d.path().join("state"), &projects);

        // A likely one nobody keeps: asked about.
        f.look_all().await;
        let s = f.status();
        assert_eq!(s.asks.len(), 1, "{:?}", s.asks);
        assert_eq!(
            (s.asks[0].kind, s.asks[0].path.as_str(), s.asks[0].size),
            (SecretFileAskKind::New, ".env.local", 11)
        );
        assert!(!s.account);

        // Kept: here only (no account), and put in the clone.
        let kept = f.keep(&c.repo, ".env", b"A=1\n").await.unwrap();
        assert_eq!(kept.origin, SecretOrigin::Local);
        assert_eq!(kept.used_by, vec![c.wadspace.clone()]);
        assert_eq!(fake.state.lock().unwrap().secrets.get(&kept.name).map(String::as_str), Some("A=1\n"));
        f.look_all().await;
        assert_eq!(std::fs::read(dir.join(".env")).unwrap(), b"A=1\n");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dir.join(".env")).unwrap().permissions().mode() & 0o777, 0o600);

        // New contents kept: the copy there, still the one put there, follows.
        f.keep(&c.repo, ".env", b"A=2\n").await.unwrap();
        f.look_all().await;
        assert_eq!(std::fs::read(dir.join(".env")).unwrap(), b"A=2\n");

        // Changed there: never written over, asked about.
        std::fs::write(dir.join(".env"), "A=mine\n").unwrap();
        f.keep(&c.repo, ".env", b"A=3\n").await.unwrap();
        f.look_all().await;
        assert_eq!(std::fs::read(dir.join(".env")).unwrap(), b"A=mine\n");
        let changed = f.status().asks.into_iter().find(|a| a.kind == SecretFileAskKind::Changed).expect("asked");
        // Kept for every machine: that's the kept one now.
        let s = f.answer(&changed.id, true).await.unwrap();
        assert!(s.asks.iter().all(|a| a.kind != SecretFileAskKind::Changed), "{:?}", s.asks);
        assert_eq!(fake.state.lock().unwrap().secrets.get(&kept.name).map(String::as_str), Some("A=mine\n"));

        // The likely one said no to: asked no more.
        let new = f.status().asks.into_iter().find(|a| a.kind == SecretFileAskKind::New).unwrap();
        let s = f.answer(&new.id, false).await.unwrap();
        assert!(s.asks.is_empty(), "{:?}", s.asks);
        f.look_all().await;
        assert!(f.status().asks.is_empty());

        // Deleted there: not put back; removed: its secret goes, the copy stays.
        std::fs::remove_file(dir.join(".env")).unwrap();
        f.look_all().await;
        assert!(!dir.join(".env").exists());
        f.remove(&kept.name).await.unwrap();
        assert!(f.status().files.is_empty());
        assert!(!fake.state.lock().unwrap().secrets.contains_key(&kept.name));
    }

    #[tokio::test]
    async fn a_link_in_a_clone_isnt_written_through() {
        let d = tempfile::tempdir().unwrap();
        let sh = AsUser { uid: None, home: d.path().to_path_buf() };
        let target = d.path().join("outside");
        std::fs::write(&target, "keep").unwrap();
        std::os::unix::fs::symlink(&target, d.path().join(".env")).unwrap();
        let dir = d.path().to_string_lossy().into_owned();
        let out = sh.sh(scripts::WRITE, &[&dir, ".env"], Some(b"QT0x")).await.unwrap();
        assert_ne!(out.code, 0);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
        let out = sh.sh(scripts::DIGESTS, &[&dir, ".env", "nope"], None).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "-\n-\n");
    }

    #[test]
    fn values_in_base64_come_back() {
        assert_eq!(decode("AAEC", true), Some(vec![0, 1, 2]));
        assert_eq!(decode("text", false), Some(b"text".to_vec()));
        assert_eq!(decode("!!", true), None);
    }
}
