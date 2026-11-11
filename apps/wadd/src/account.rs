//! The machine's link to its owner's WadSpaces account (Firebase), so the
//! phone finds it without being told its name and its secret files follow
//! the owner to every machine.
//!
//! - **Linking** is signing in to GitHub. The GitHub sign-in's token (the
//!   same OAuth app as the account's GitHub provider) signs in to the
//!   account as the person once, which makes a one-use enroll code
//!   (`enrollCodes/<code>`); the account's `enrollMachine` function trades
//!   it for this machine's own sign-in (a custom token: uid `machine:<id>`,
//!   `role: machine`, its owner and machine id). Its refresh token is the
//!   podman secret [`ACCOUNT_SECRET`], never in a file, a log, a job or an
//!   event; `<state>/account.json` (0600) has only whose and which machine.
//! - **Linked**, it heartbeats `users/<uid>/machines/<id>` every
//!   `heartbeat_s` (the fields the account's rules let a machine change:
//!   its hostname, version, and `tailnet`: where on the tailnet its remote
//!   API is), and fetches the account's secret files every `sync_s`
//!   (secret_files.rs places them).
//! - The phone, signed in to the account, calls the remote API with its ID
//!   token as `Authorization: Bearer`: [`AccountService::is_owner`] says
//!   whether it's this machine's owner's (Firebase's `accounts:lookup`),
//!   which is trust enough (no ask on the machine).
//!
//! No Firebase configuration (`[account] firebase`,
//! `/etc/wados/firebase.toml`, which the installer stick puts there, never
//! the public image): the account is Off, and the machine
//! works as before (Tailscale, trust by asking, secret files kept here).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use wad_firebase::{Error as FbError, Firestore, Functions, Identity, Write, values};
use wad_model::PeerKind;
use wad_proto::{AccountLink, AccountState, ApiError, ErrorCode, Event, MachineInfo};

use crate::backend::Backend;
use crate::events::Bus;
use crate::packages::GITHUB_SECRET;
use crate::secret_files::{self, AccountFile, AccountStore, SecretFiles};
use crate::tailnet::Tailnet;

/// The machine's refresh token for the account.
pub const ACCOUNT_SECRET: &str = "wados_account";
const FILE: &str = "account.json";
/// An ID token is refreshed this long before it expires.
const EARLY: Duration = Duration::from_secs(300);
/// A phone's ID token, once Firebase said whose it is, is believed this long.
const OWNER_FOR: Duration = Duration::from_secs(300);
const MAX_OWNERS: usize = 256;
/// The remote API's port, as the phone finds it.
const REMOTE_PORT: u16 = 8087;

/// The account's Firebase web app (public: what any of its web pages has).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FirebaseApp {
    pub api_key: String,
    pub project_id: String,
    #[serde(default)]
    pub auth_domain: String,
    #[serde(default = "default_region")]
    pub functions_region: String,
}

/// Where the account's `enrollMachine` is deployed (wadspaces-client's
/// functions/.env.wad-spaces), for a firebase.toml that doesn't say.
fn default_region() -> String {
    "australia-southeast2".into()
}

impl FirebaseApp {
    /// From its file; None when there's none or it isn't one.
    pub fn read(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        // An empty one is none.
        if text.trim().is_empty() {
            return None;
        }
        match toml::from_str::<Self>(&text) {
            Ok(a) if !a.api_key.is_empty() && !a.project_id.is_empty() => Some(a),
            Ok(_) => {
                tracing::warn!("{}: no api_key or project_id: no account", path.display());
                None
            }
            Err(e) => {
                tracing::warn!("{}: {e}: no account", path.display());
                None
            }
        }
    }
}

pub struct Settings {
    pub app: Option<FirebaseApp>,
    /// Firebase's APIs (empty: Google's): an emulator's or a test's.
    pub identity: String,
    pub firestore: String,
    pub functions: String,
    pub heartbeat: Duration,
    pub sync: Duration,
    pub state_dir: PathBuf,
}

impl Settings {
    pub fn new(config: &wad_config::Config) -> Self {
        let a = &config.account;
        Self {
            app: FirebaseApp::read(&a.firebase),
            identity: a.identity.clone(),
            firestore: a.firestore.clone(),
            functions: a.functions.clone(),
            heartbeat: Duration::from_secs(a.heartbeat_s.max(10)),
            sync: Duration::from_secs(a.sync_s.max(30)),
            state_dir: config.daemon.state_dir.clone(),
        }
    }
}

/// `account.json`: whose and which machine (no token).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    uid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    machine_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    login: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    linked_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
    /// Unlinked on purpose (or removed from the account): a GitHub sign-in
    /// already there doesn't link it again by itself; signing in again does.
    #[serde(default)]
    unlinked: bool,
}

impl Saved {
    fn linked(&self) -> Option<(&str, &str)> {
        Some((self.uid.as_deref()?, self.machine_id.as_deref()?))
    }
}

/// Firebase, against Google's APIs or the test's.
struct Apis {
    app: FirebaseApp,
    identity: Identity,
    firestore: Firestore,
    functions: Functions,
}

pub struct AccountService {
    apis: Option<Apis>,
    backend: Arc<dyn Backend>,
    bus: Bus,
    machine: MachineInfo,
    file: PathBuf,
    heartbeat: Duration,
    sync: Duration,
    saved: Mutex<Saved>,
    view: Mutex<AccountLink>,
    /// The machine's ID token and when to refresh it.
    token: tokio::sync::Mutex<Option<(String, Instant)>>,
    /// sha256 of a phone's ID token → its uid, until.
    owners: Mutex<HashMap<[u8; 32], (String, Instant)>>,
    secret_files: Arc<SecretFiles>,
    tailnet: Option<Arc<Tailnet>>,
    linking: tokio::sync::Mutex<()>,
    poke: tokio::sync::Notify,
}

fn now_ms() -> i64 {
    crate::jobs::now_ms()
}

fn upstream(e: impl std::fmt::Display) -> ApiError {
    ApiError::new(ErrorCode::Upstream, e.to_string())
}

/// A one-use code: 10 of A-Z and 0-9.
fn enroll_code() -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    // 252 is the largest multiple of 36 under 256: no letter likelier.
    let mut out = String::new();
    while out.len() < 10 {
        for b in crate::streams::random_bytes(16) {
            if b < 252 && out.len() < 10 {
                out.push(ABC[(b % 36) as usize] as char);
            }
        }
    }
    out
}

/// A Firestore document id as the account makes them.
fn is_doc_id(id: &str) -> bool {
    (1..=128).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn kind_name(k: PeerKind) -> &'static str {
    match k {
        PeerKind::Laptop => "laptop",
        PeerKind::Server => "server",
        _ => "desktop",
    }
}

impl AccountService {
    pub fn new(
        settings: Settings,
        backend: Arc<dyn Backend>,
        bus: Bus,
        machine: MachineInfo,
        secret_files: Arc<SecretFiles>,
        tailnet: Option<Arc<Tailnet>>,
    ) -> Arc<Self> {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build().expect("http client");
        let apis = settings.app.clone().and_then(|app| {
            let identity = if settings.identity.is_empty() {
                Identity::new(http.clone())
            } else {
                Identity::with_base(http.clone(), &settings.identity)
            };
            let firestore = if settings.firestore.is_empty() {
                Firestore::new(http.clone(), &app.project_id)
            } else {
                Firestore::with_base(http.clone(), &settings.firestore, &app.project_id)
            };
            let firestore = match firestore {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!("account: {e}: no account");
                    return None;
                }
            };
            let functions = if settings.functions.is_empty() {
                Functions::for_project(http.clone(), &app.project_id, &app.functions_region)
            } else {
                Functions::new(http.clone(), &settings.functions)
            };
            Some(Apis { app, identity, firestore, functions })
        });
        let file = settings.state_dir.join(FILE);
        let mut saved: Saved =
            std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        // Another project's link (the image's configuration changed) isn't this one's.
        if let (Some(a), Some(p)) = (&apis, &saved.project_id)
            && *p != a.app.project_id
        {
            tracing::info!("account: linked to another Firebase project before: not linked now");
            saved = Saved::default();
        }
        let me = Arc::new(Self {
            apis,
            backend,
            bus,
            machine,
            file,
            heartbeat: settings.heartbeat,
            sync: settings.sync,
            saved: Mutex::new(saved),
            view: Mutex::new(AccountLink::default()),
            token: tokio::sync::Mutex::new(None),
            owners: Mutex::default(),
            secret_files,
            tailnet,
            linking: tokio::sync::Mutex::new(()),
            poke: tokio::sync::Notify::new(),
        });
        let weak: std::sync::Weak<dyn AccountStore> = Arc::downgrade(&me) as _;
        me.secret_files.set_account(weak);
        me.refresh_view(None);
        me
    }

    /// There's an account to link to.
    pub fn enabled(&self) -> bool {
        self.apis.is_some()
    }

    fn linked_ids(&self) -> Option<(String, String)> {
        let s = self.saved.lock().unwrap();
        s.linked().map(|(u, m)| (u.to_string(), m.to_string()))
    }

    /// The owner's uid, while linked.
    pub fn owner(&self) -> Option<String> {
        self.linked_ids().map(|(u, _)| u)
    }

    /// `GET /v1/account`.
    pub fn view(&self) -> AccountLink {
        self.view.lock().unwrap().clone()
    }

    /// The view again from what's saved, with `error` (None: as it was, unless linking cleared it).
    fn refresh_view(&self, error: Option<Option<String>>) {
        let s = self.saved.lock().unwrap().clone();
        let now = {
            let mut v = self.view.lock().unwrap();
            let state = if self.apis.is_none() {
                AccountState::Off
            } else if v.state == AccountState::Linking {
                AccountState::Linking
            } else if s.linked().is_some() {
                AccountState::Linked
            } else {
                AccountState::Unlinked
            };
            let linked = state == AccountState::Linked;
            v.state = state;
            v.uid = s.uid.clone().filter(|_| linked);
            v.login = s.login.clone().filter(|_| linked);
            v.machine_id = s.machine_id.clone().filter(|_| linked);
            v.linked_at = s.linked_at.filter(|_| linked);
            if !linked {
                v.last_seen = None;
            }
            if let Some(e) = error {
                v.error = e;
            }
            v.clone()
        };
        let e = Event::Account(now);
        if self.bus.latest("account").as_ref() != Some(&e) {
            self.bus.publish(e);
        }
    }

    fn set_state(&self, state: AccountState) {
        self.view.lock().unwrap().state = state;
    }

    fn save(&self) {
        let bytes = serde_json::to_vec_pretty(&*self.saved.lock().unwrap()).expect("json");
        if let Err(e) = crate::store::write_atomic_mkdir(&self.file, &bytes) {
            tracing::warn!("{}: {e}", self.file.display());
        }
    }

    fn apis(&self) -> Result<&Apis, ApiError> {
        self.apis.as_ref().ok_or_else(|| ApiError::conflict("This WadOS has no WadSpaces account to link to"))
    }

    // ------------------------------------------------------------ linking

    /// Links the machine to the GitHub sign-in's account (again: the same
    /// machine, when it was the same owner's).
    pub async fn link(&self) -> Result<AccountLink, ApiError> {
        let _one = self.linking.lock().await;
        self.apis()?;
        self.set_state(AccountState::Linking);
        self.refresh_view(None);
        let r = self.link_now().await;
        self.set_state(AccountState::Unlinked);
        match r {
            Ok(()) => {
                self.refresh_view(Some(None));
                self.poke.notify_one();
                Ok(self.view())
            }
            Err(e) => {
                tracing::warn!("account: linking: {}", e.message);
                self.refresh_view(Some(Some(e.message.clone())));
                Err(e)
            }
        }
    }

    async fn link_now(&self) -> Result<(), ApiError> {
        let a = self.apis()?;
        let token = match self.backend.secret_value(GITHUB_SECRET).await {
            Ok(Some(t)) if !t.trim().is_empty() => t.trim().to_string(),
            Ok(_) => return Err(ApiError::conflict("Sign in to WadSpaces first: that links the machine")),
            Err(e) => return Err(upstream(format!("Podman didn't say whether there's a GitHub sign-in: {e}"))),
        };
        let person = a.identity.sign_in_with_idp(&a.app.api_key, "github.com", &token).await.map_err(|e| match e {
            FbError::Auth { message, .. } if message.contains("ADMIN_ONLY_OPERATION") => ApiError::new(
                ErrorCode::Forbidden,
                "The WadSpaces account doesn't take new people: sign in to the app first with this GitHub account",
            ),
            FbError::Auth { message, .. } => ApiError::new(
                ErrorCode::Unauthorized,
                format!("The account didn't take the GitHub sign-in ({message}): sign in to WadSpaces again"),
            ),
            e => ApiError::from(e),
        })?;
        if person.uid.is_empty() || person.tokens.id_token.is_empty() {
            return Err(upstream("The account answered without a sign-in"));
        }
        let code = enroll_code();
        let mut data = Map::new();
        data.insert("uid".into(), json!(person.uid));
        data.insert("used".into(), json!(false));
        data.insert("machineName".into(), json!(self.machine.name));
        a.firestore
            .commit(
                &person.tokens.id_token,
                &[Write::set(format!("enrollCodes/{code}"), data).now("createdAt").creating()],
            )
            .await
            .map_err(|e| {
                ApiError::new(ErrorCode::Upstream, format!("The account didn't take the machine's code: {e}"))
            })?;
        // The same owner's machine again keeps its document (enrollMachine
        // checks the old sign-in is the same owner's).
        let previous = match self.linked_ids() {
            Some((uid, _)) if uid == person.uid => self.id_token().await.ok(),
            _ => None,
        };
        let mut req = json!({
            "code": code,
            "hostname": self.hostname(),
            "machineName": self.machine.name,
            "daemonVersion": self.machine.version,
        });
        if let Some(p) = previous {
            req["previousIdToken"] = json!(p);
        }
        let got = a.functions.call("enrollMachine", req).await.map_err(|e| match e {
            FbError::Callable { status, message } => {
                let code = match status.as_str() {
                    "NOT_FOUND" | "INVALID_ARGUMENT" => ErrorCode::BadRequest,
                    "FAILED_PRECONDITION" | "DEADLINE_EXCEEDED" => ErrorCode::Conflict,
                    _ => ErrorCode::Upstream,
                };
                ApiError::new(code, format!("The account didn't link the machine: {message}"))
            }
            e => ApiError::from(e),
        })?;
        let text = |k: &str| got.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
        let (Some(machine_id), Some(owner), Some(custom)) = (text("machineId"), text("ownerUid"), text("customToken"))
        else {
            return Err(upstream("The account's answer had no machine sign-in"));
        };
        if owner != person.uid || text("projectId").is_some_and(|p| p != a.app.project_id) {
            return Err(upstream("The account linked the machine to someone else"));
        }
        if !is_doc_id(&machine_id) {
            return Err(upstream("The account named the machine oddly"));
        }
        let tokens = a.identity.sign_in_with_custom_token(&a.app.api_key, &custom).await.map_err(ApiError::from)?;
        if tokens.refresh_token.is_empty() {
            return Err(upstream("The account's sign-in for the machine had no refresh token"));
        }
        self.backend
            .create_secret(ACCOUNT_SECRET, tokens.refresh_token.as_bytes())
            .await
            .map_err(|e| upstream(format!("The machine's sign-in wasn't kept: {e}")))?;
        let before = self.owner();
        *self.token.lock().await =
            Some((tokens.id_token, Instant::now() + Duration::from_secs(tokens.expires_in).saturating_sub(EARLY)));
        *self.saved.lock().unwrap() = Saved {
            uid: Some(owner.clone()),
            machine_id: Some(machine_id.clone()),
            login: person.screen_name.clone(),
            linked_at: Some(now_ms()),
            project_id: Some(a.app.project_id.clone()),
            unlinked: false,
        };
        self.save();
        if before.is_some_and(|b| b != owner) {
            self.secret_files.forget_account().await;
        }
        tracing::info!(machine = %machine_id, login = ?person.screen_name, "account: linked");
        Ok(())
    }

    /// Not linked any more (the account keeps its document: the owner
    /// removes it from there). The account's secret files go from here
    /// (the copies in clones stay).
    pub async fn unlink(&self) -> Result<AccountLink, ApiError> {
        let _one = self.linking.lock().await;
        self.apis()?;
        self.forget("unlinked").await;
        self.refresh_view(Some(None));
        Ok(self.view())
    }

    async fn forget(&self, why: &str) {
        tracing::info!("account: {why}");
        if let Err(e) = self.backend.delete_secret(ACCOUNT_SECRET).await {
            tracing::warn!("account: its sign-in: {e}");
        }
        *self.token.lock().await = None;
        {
            let mut s = self.saved.lock().unwrap();
            *s = Saved { unlinked: true, project_id: s.project_id.clone(), ..Saved::default() };
        }
        self.save();
        self.secret_files.forget_account().await;
    }

    /// GitHub signed in (or was at start): links the machine, unless it is,
    /// or was unlinked on purpose and this isn't a new sign-in.
    async fn maybe_link(&self, new_sign_in: bool) {
        if self.apis.is_none() || self.linked_ids().is_some() {
            return;
        }
        if self.saved.lock().unwrap().unlinked && !new_sign_in {
            return;
        }
        if !matches!(self.backend.secret_value(GITHUB_SECRET).await, Ok(Some(_))) {
            return;
        }
        let _ = self.link().await;
    }

    // ------------------------------------------------------------ tokens

    /// The machine's ID token, refreshed when it's due.
    pub async fn id_token(&self) -> Result<String, ApiError> {
        let a = self.apis()?;
        let mut t = self.token.lock().await;
        if let Some((tok, until)) = t.as_ref()
            && Instant::now() < *until
        {
            return Ok(tok.clone());
        }
        let refresh = match self.backend.secret_value(ACCOUNT_SECRET).await {
            Ok(Some(r)) if !r.trim().is_empty() => r.trim().to_string(),
            Ok(_) => return Err(ApiError::new(ErrorCode::Unauthorized, "The machine isn't linked")),
            Err(e) => return Err(upstream(format!("Podman: {e}"))),
        };
        let got = a.identity.refresh(&a.app.api_key, &refresh).await.map_err(ApiError::from)?;
        if got.refresh_token != refresh
            && let Err(e) = self.backend.create_secret(ACCOUNT_SECRET, got.refresh_token.as_bytes()).await
        {
            tracing::warn!("account: the new refresh token wasn't kept: {e}");
        }
        let until = Instant::now() + Duration::from_secs(got.expires_in).saturating_sub(EARLY);
        *t = Some((got.id_token.clone(), until));
        Ok(got.id_token)
    }

    /// The ID token is the owner's (Firebase says so, or said so in the
    /// last five minutes).
    pub async fn is_owner(&self, id_token: &str) -> bool {
        let (Some(a), Some(owner)) = (self.apis.as_ref(), self.owner()) else { return false };
        if id_token.is_empty() || id_token.len() > 4096 {
            return false;
        }
        let key: [u8; 32] = Sha256::digest(id_token.as_bytes()).into();
        if let Some((uid, until)) = self.owners.lock().unwrap().get(&key)
            && Instant::now() < *until
        {
            return *uid == owner;
        }
        match a.identity.lookup(&a.app.api_key, id_token).await {
            Ok(uid) => {
                // A machine's own token (uid machine:<id>) isn't the owner's.
                let mut o = self.owners.lock().unwrap();
                if o.len() >= MAX_OWNERS {
                    let now = Instant::now();
                    o.retain(|_, (_, until)| *until > now);
                    if o.len() >= MAX_OWNERS {
                        o.clear();
                    }
                }
                o.insert(key, (uid.clone(), Instant::now() + OWNER_FOR));
                uid == owner
            }
            Err(e) => {
                tracing::debug!("account: a bearer token: {e}");
                false
            }
        }
    }

    // ------------------------------------------------------------ loops

    fn hostname(&self) -> String {
        self.tailnet
            .as_ref()
            .and_then(|t| t.status())
            .and_then(|s| s.me.map(|p| p.machine_name()))
            .unwrap_or_else(|| self.machine.name.to_lowercase())
    }

    /// Where the phone reaches it: its MagicDNS name and tailnet address.
    fn tailnet_map(&self) -> Option<Value> {
        let s = self.tailnet.as_ref()?.status().filter(|s| s.running())?;
        let me = s.me.as_ref()?;
        let ip = s.ipv4()?;
        let kind = kind_name(crate::remote::this_machine(&self.machine).kind);
        Some(json!({
            "wadd": self.machine.id,
            "name": self.machine.name,
            "dnsName": me.dns(),
            "ip": ip.to_string(),
            "kind": kind,
            "port": REMOTE_PORT,
        }))
    }

    /// Tells the account the machine's on, and where.
    pub async fn heartbeat(&self) -> Result<(), ApiError> {
        let a = self.apis()?;
        let Some((uid, mid)) = self.linked_ids() else { return Ok(()) };
        let tok = self.id_token().await?;
        let mut data = Map::new();
        data.insert("hostname".into(), json!(self.hostname()));
        data.insert("daemonVersion".into(), json!(self.machine.version));
        if let Some(t) = self.tailnet_map() {
            data.insert("tailnet".into(), t);
        }
        // Only the fields the rules let the machine change, and its document must be there.
        let path = format!("users/{uid}/machines/{mid}");
        let w = Write::update(path.clone(), data).now("lastSeen");
        let r = match a.firestore.commit(&tok, &[w]).await {
            // The rules see an update of a document that's gone as a create,
            // which a machine can't make (403, not a failed precondition):
            // gone it is, if a read of it (its own: allowed) finds nothing.
            Err(FbError::Denied) if matches!(a.firestore.get(&tok, &path).await, Ok(None)) => {
                Err(FbError::Precondition("the machine's document is gone".into()))
            }
            r => r,
        };
        match r {
            Ok(()) => {
                self.view.lock().unwrap().last_seen = Some(now_ms());
                self.refresh_view(Some(None));
                Ok(())
            }
            // Its document's gone: the owner removed it from the account.
            Err(FbError::Precondition(_)) => {
                self.forget("removed from the account").await;
                self.refresh_view(Some(Some(
                    "This machine was removed from your account: sign in to WadSpaces again to link it".into(),
                )));
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// The account's secret files, to secret_files.rs.
    pub async fn sync_secrets(&self) -> Result<(), ApiError> {
        let a = self.apis()?;
        let Some((uid, _)) = self.linked_ids() else { return Ok(()) };
        let tok = self.id_token().await?;
        let docs = a.firestore.list(&tok, &format!("users/{uid}/secrets")).await.map_err(ApiError::from)?;
        let files = docs
            .iter()
            .filter_map(|d| {
                let name = values::doc_id(d);
                if !secret_files::is_name(&name) {
                    return None;
                }
                let f = values::from_fields(d.get("fields").unwrap_or(&Value::Null));
                let value = f.get("value")?.as_str()?.to_string();
                let repo = secret_files::check_repo(f.pointer("/file/repo")?.as_str()?).ok()?;
                let path = secret_files::check_path(f.pointer("/file/path")?.as_str()?).ok()?;
                // It's the file at that place, or it isn't one.
                if secret_files::name_of(&repo, &path) != name {
                    return None;
                }
                let base64 = f.get("encoding").and_then(Value::as_str) == Some("base64");
                let updated_at = f.get("updatedAt").map(values::to_ms).filter(|t| *t > 0);
                Some((name, AccountFile { value, base64, repo, path, updated_at }))
            })
            .collect();
        self.secret_files.from_account(files).await;
        Ok(())
    }

    /// Links at start (GitHub signed in already), follows the GitHub
    /// sign-in, heartbeats and fetches the secret files, for as long as wadd runs.
    pub async fn run(self: Arc<Self>) {
        if self.apis.is_none() {
            return;
        }
        let (_, mut rx) = self.bus.subscribe();
        let mut signed_in = matches!(self.bus.latest("github"), Some(Event::Github { account: Some(_) }));
        self.maybe_link(false).await;
        let mut beat = tokio::time::interval(self.heartbeat);
        let mut sync = tokio::time::interval(self.sync);
        loop {
            tokio::select! {
                _ = beat.tick() => {
                    if let Err(e) = self.heartbeat().await {
                        tracing::warn!("account: heartbeat: {}", e.message);
                        self.refresh_view(Some(Some(format!("The account didn't hear from the machine: {}", e.message))));
                    }
                }
                _ = sync.tick() => self.sync_now().await,
                _ = self.poke.notified() => {
                    let _ = self.heartbeat().await;
                    self.sync_now().await;
                }
                e = rx.recv() => match e {
                    Ok(Event::Github { account }) => {
                        let now = account.is_some();
                        if now && !signed_in {
                            self.maybe_link(true).await;
                        }
                        signed_in = now;
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => return,
                },
            }
        }
    }

    async fn sync_now(&self) {
        if let Err(e) = self.sync_secrets().await {
            tracing::warn!("account: secret files: {}", e.message);
            self.refresh_view(Some(Some(format!("The account's secret files weren't fetched: {}", e.message))));
        }
    }

    /// The secret files fetched now (after a change on the phone).
    pub fn poke(&self) {
        self.poke.notify_one();
    }
}

#[async_trait]
impl AccountStore for AccountService {
    fn linked(&self) -> bool {
        self.apis.is_some() && self.linked_ids().is_some()
    }

    async fn put_file(&self, name: &str, file: &AccountFile) -> Result<(), String> {
        let a = self.apis().map_err(|e| e.message)?;
        let (uid, _) = self.linked_ids().ok_or("not linked")?;
        let tok = self.id_token().await.map_err(|e| e.message)?;
        let mut data = Map::new();
        data.insert("value".into(), json!(file.value));
        if file.base64 {
            data.insert("encoding".into(), json!("base64"));
        }
        data.insert("file".into(), json!({"repo": file.repo, "path": file.path}));
        data.insert("updatedAt".into(), values::time(file.updated_at.unwrap_or_else(now_ms)));
        a.firestore.set(&tok, &format!("users/{uid}/secrets/{name}"), &data).await.map_err(|e| e.to_string())
    }

    async fn delete_file(&self, name: &str) -> Result<(), String> {
        let a = self.apis().map_err(|e| e.message)?;
        let (uid, _) = self.linked_ids().ok_or("not linked")?;
        let tok = self.id_token().await.map_err(|e| e.message)?;
        a.firestore.delete(&tok, &format!("users/{uid}/secrets/{name}")).await.map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_what_enroll_codes_take() {
        for _ in 0..50 {
            let c = enroll_code();
            assert_eq!(c.len(), 10);
            assert!(c.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()), "{c}");
        }
        assert_ne!(enroll_code(), enroll_code());
    }

    #[test]
    fn the_web_app_is_read_from_its_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("firebase.toml");
        assert_eq!(FirebaseApp::read(&p), None);
        std::fs::write(&p, "api_key = \"k\"\nproject_id = \"wad-spaces\"\n").unwrap();
        let a = FirebaseApp::read(&p).unwrap();
        assert_eq!((a.project_id.as_str(), a.functions_region.as_str()), ("wad-spaces", "australia-southeast2"));
        std::fs::write(&p, "api_key = \"\"\nproject_id = \"x\"\n").unwrap();
        assert_eq!(FirebaseApp::read(&p), None);
    }
}
