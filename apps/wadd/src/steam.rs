//! Steam (`GET /v1/steam`, `POST /v1/steam/{signin,push}`).
//!
//! **Signing in.** An account is a key, whose folder keeps Steam's own
//! session and its library (`steamapps/`), so a game downloads once however
//! many wadspaces use it. wadd never sees a password. The real client signs
//! in, in a temporary wadspace with that key, in a window, and wadd watches
//! the key's folder for `loginusers.vdf`. Then the library is read and the
//! wadspace goes.
//!
//! **The library** comes from the client's own files ([`wad_model::vdf`]), with
//! no Web API key: `localconfig.vdf` for what the account has on this client,
//! `appmanifest_*.acf` for what's downloaded. A game it hasn't opened here
//! may not be listed until it has.
//!
//! **Those folders are written from inside wadspaces, and wadd is root.** So
//! only plain files of a sane size are read ([`read_plain`]), links are never
//! followed, and a manifest's folder name can't be a path ([`plain_name`]).
//!
//! **Packages.** A game goes to GitHub's only if it's ticked in the push
//! dialog (`POST /v1/steam/push`). The first push needs a second, short GitHub
//! sign-in for `write:packages`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use wad_model::edit::Op;
use wad_model::games::{game_of, game_package_ref};
use wad_model::vdf;
use wad_model::{GamePackage, JobKind, PartState, SecretKind, SessionDisplay, SteamLibrary};
use wad_proto::{ApiError, JobInfo, SteamAccount, SteamGame, SteamPush, SteamSignIn};

use crate::discs::{Registry, pusher};
use crate::jobs::{Jobs, now_ms};
use crate::library::Docs;

const STEAM: &str = ".local/share/Steam";
/// Steam's text files are small: anything bigger isn't one.
const MOST: u64 = 16 * 1024 * 1024;
/// How long a sign-in window stays waiting.
pub const SIGN_IN_FOR: Duration = Duration::from_secs(15 * 60);
pub const STAGES: [(&str, &str); 3] = [
    ("open", "Opening Steam’s sign-in"),
    ("signin", "Signing in to Steam, in its window"),
    ("library", "Reading its library"),
];

/// The wadspace a sign-in happens in.
#[async_trait]
pub trait Spaces: Send + Sync + 'static {
    /// Steam open in a window, signing in to `key`: its wadspace.
    async fn open(&self, key: &str) -> Result<String, String>;
    async fn close(&self, ws: &str);
}

/// A file that's really a file, and small, or nothing: never through a link.
pub fn read_plain(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MOST {
        return None;
    }
    let mut text = String::new();
    std::fs::File::open(path).ok()?.take(MOST).read_to_string(&mut text).ok()?;
    Some(text)
}

/// A folder name from a manifest, if it's only that.
pub fn plain_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\0')
}

/// A folder that's really one: never a link.
fn plain_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

/// Tools Steam lists among apps: not games.
fn is_tool(name: &str) -> bool {
    let n = name.to_lowercase();
    n.starts_with("proton") || n.starts_with("steam linux runtime") || n.contains("steamworks common")
}

/// What a key's folder says: the account's name, and its library.
pub fn read_folder(folder: &Path) -> Option<(String, SteamLibrary)> {
    let steam = folder.join(STEAM);
    let (_, name) = vdf::signed_in(&read_plain(&steam.join("config/loginusers.vdf"))?)?;
    let mut lib = SteamLibrary::default();
    let mut owned: Vec<String> = vec![];
    let users = steam.join("userdata");
    if plain_dir(&users) {
        for u in std::fs::read_dir(&users).into_iter().flatten().flatten() {
            let f = u.path().join("config/localconfig.vdf");
            if !plain_dir(&u.path()) {
                continue;
            }
            for app in read_plain(&f).map(|t| vdf::apps_of(&t)).unwrap_or_default() {
                owned.push(vdf::game_id(app));
            }
        }
    }
    let apps = steam.join("steamapps");
    if plain_dir(&apps) {
        for m in std::fs::read_dir(&apps).into_iter().flatten().flatten() {
            let file = m.file_name().to_string_lossy().into_owned();
            if !(file.starts_with("appmanifest_") && file.ends_with(".acf")) {
                continue;
            }
            let Some(a) = read_plain(&m.path()).and_then(|t| vdf::manifest(&t)) else { continue };
            if is_tool(&a.name) {
                continue;
            }
            let id = vdf::game_id(a.appid);
            owned.push(id.clone());
            lib.names.insert(id.clone(), a.name);
            if a.installed {
                lib.installed.insert(id, true);
            }
        }
    }
    owned.sort();
    owned.dedup();
    lib.owned = owned;
    Some((name, lib))
}

/// A game's folder and manifest in `steamapps`.
fn game_files(apps: &Path, appid: u32) -> Option<(String, String)> {
    let file = format!("appmanifest_{appid}.acf");
    let m = vdf::manifest(&read_plain(&apps.join(&file))?)?;
    plain_name(&m.dir).then(|| (format!("common/{}", m.dir), file))
}

pub struct MachineSteam {
    keys_dir: PathBuf,
    spaces: Arc<dyn Spaces>,
    registry: Arc<dyn Registry>,
    docs: Arc<Docs>,
    jobs: Arc<Jobs>,
    wait: Duration,
}

impl MachineSteam {
    pub fn new(
        keys_dir: &Path,
        spaces: Arc<dyn Spaces>,
        registry: Arc<dyn Registry>,
        docs: Arc<Docs>,
        jobs: Arc<Jobs>,
        wait: Duration,
    ) -> Arc<Self> {
        Arc::new(Self { keys_dir: keys_dir.to_path_buf(), spaces, registry, docs, jobs, wait })
    }

    fn folder(&self, key: &str) -> Option<PathBuf> {
        crate::store::safe_id(key).then(|| self.keys_dir.join(key))
    }

    /// Every account, read again from its folder.
    pub fn accounts(&self) -> Vec<SteamAccount> {
        let lib = self.docs.snapshot();
        let mut out = vec![];
        for k in lib.keys.iter().filter(|k| k.kind == SecretKind::Steam) {
            let Some((_, read)) = self.folder(&k.id).and_then(|f| read_folder(&f)) else { continue };
            if lib.steam.get(&k.id) != Some(&read) {
                let _ = self.docs.apply(Op::SteamLibrary { key: k.id.clone(), library: read.clone() });
            }
            let games = read
                .owned
                .iter()
                .map(|id| SteamGame {
                    id: id.clone(),
                    name: game_of(id, read.names.get(id).map(String::as_str)).name,
                    installed: read.has(id),
                    pushed: lib.game_packages.contains_key(id),
                })
                .collect();
            out.push(SteamAccount { id: k.id.clone(), name: k.name.clone(), games });
        }
        out
    }

    pub fn sign_in(self: &Arc<Self>, req: SteamSignIn) -> Result<JobInfo, ApiError> {
        let made = self.docs.apply(Op::NewKey { name: req.name, kind: SecretKind::Steam })?;
        let key = made.created.ok_or_else(|| ApiError::internal("no key was made"))?;
        let stages: Vec<(String, String)> = STAGES.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let job = self.jobs.create(JobKind::SteamLogin, "Signing in to Steam", &key, &stages);
        let (me, id) = (self.clone(), job.clone());
        let task = tokio::spawn(async move {
            me.jobs.running(&id);
            let r = me.signing_in(&id, &key).await;
            if r.is_err() {
                let _ = std::fs::remove_dir_all(me.keys_dir.join(&key));
            }
            me.jobs.finish(&id, r);
        });
        self.jobs.attach(&job, task.abort_handle());
        self.jobs.info(&job).ok_or_else(|| ApiError::internal("the job went"))
    }

    async fn signing_in(&self, job: &str, key: &str) -> Result<(), String> {
        let folder = self.folder(key).ok_or("not a key")?;
        self.jobs.stage(job, "open", PartState::Working, None);
        let ws = self.spaces.open(key).await?;
        self.jobs.stage(job, "open", PartState::Done, None);
        self.jobs.stage(job, "signin", PartState::Working, None);
        let deadline = tokio::time::Instant::now() + self.wait;
        let read = loop {
            if let Some(r) = read_folder(&folder) {
                break Some(r);
            }
            if tokio::time::Instant::now() >= deadline {
                break None;
            }
            tokio::time::sleep(Duration::from_millis(500).min(self.wait)).await;
        };
        self.spaces.close(&ws).await;
        let Some((name, library)) = read else {
            return Err("Steam wasn’t signed in to in time".into());
        };
        self.jobs.stage(job, "signin", PartState::Done, None);
        self.jobs.stage(job, "library", PartState::Working, None);
        self.jobs.line(job, &format!("» {name}: {} games", library.owned.len()));
        self.docs.apply(Op::SteamLibrary { key: key.to_string(), library }).map_err(|e| e.message)?;
        self.jobs.stage(job, "library", PartState::Done, None);
        Ok(())
    }

    /// The games ticked in the push dialog, each packed and pushed.
    pub async fn push(self: &Arc<Self>, req: SteamPush) -> Result<JobInfo, ApiError> {
        let who = pusher(self.registry.as_ref()).await?;
        let apps = self
            .folder(&req.account)
            .map(|f| f.join(STEAM).join("steamapps"))
            .ok_or_else(|| ApiError::not_found("There's no such account"))?;
        let mut stages = vec![];
        let mut plan = vec![];
        for id in &req.games {
            let appid = vdf::appid_of(id).ok_or_else(|| ApiError::bad_request(format!("{id} isn't a game")))?;
            let (dir, manifest) = game_files(&apps, appid)
                .filter(|(dir, _)| plain_dir(&apps.join(dir)))
                .ok_or_else(|| ApiError::conflict(format!("{id} isn’t downloaded here")))?;
            stages.push((id.clone(), format!("Staging {} in your packages", game_of(id, None).name)));
            plan.push((id.clone(), appid, vec![dir, manifest]));
        }
        let job = self.jobs.create(JobKind::GameStage, "Staging games in your packages", &req.account, &stages);
        let (me, id) = (self.clone(), job.clone());
        let task = tokio::spawn(async move {
            me.jobs.running(&id);
            let mut r = Ok(());
            for (game, appid, paths) in plan {
                me.jobs.stage(&id, &game, PartState::Working, None);
                let reference = format!("{}:{appid}", game_package_ref(&who.login, &game));
                if let Err(e) = me.registry.push(&apps, &paths, &reference, &who).await {
                    r = Err(e);
                    break;
                }
                let package = GamePackage {
                    game: game.clone(),
                    reference,
                    mb: 0,
                    machine: me.docs.machine().to_string(),
                    at: now_ms(),
                };
                if let Err(e) = me.docs.apply(Op::SteamPackage { package }) {
                    r = Err(e.message);
                    break;
                }
                me.jobs.stage(&id, &game, PartState::Done, None);
            }
            me.jobs.finish(&id, r);
        });
        self.jobs.attach(&job, task.abort_handle());
        self.jobs.info(&job).ok_or_else(|| ApiError::internal("the job went"))
    }

    /// Another machine's pushed game, into this account's library, not from Steam.
    pub async fn pull(self: &Arc<Self>, account: &str, game: &str) -> Result<JobInfo, ApiError> {
        let package = self.docs.snapshot().game_packages.get(game).cloned();
        let package = package.ok_or_else(|| ApiError::not_found(format!("{game} isn’t in your packages")))?;
        let who = self.registry.login().await.map_err(ApiError::internal)?;
        let who = who.ok_or_else(|| ApiError::conflict(crate::discs::NEEDS_GITHUB))?;
        let apps = self
            .folder(account)
            .map(|f| f.join(STEAM).join("steamapps"))
            .ok_or_else(|| ApiError::not_found("There's no such account"))?;
        let name = game_of(game, None).name;
        let stages = vec![("pull".to_string(), format!("Pulling {name} from your packages"))];
        let job = self.jobs.create(JobKind::GameDownload, &format!("Downloading {name}"), game, &stages);
        let (me, id) = (self.clone(), job.clone());
        let task = tokio::spawn(async move {
            me.jobs.running(&id);
            me.jobs.stage(&id, "pull", PartState::Working, None);
            let made = std::fs::create_dir_all(&apps).map_err(|e| e.to_string());
            let r = match made {
                Ok(()) => me.registry.pull(&package.reference, &apps, &who).await,
                Err(e) => Err(e),
            };
            me.jobs.finish(&id, r);
        });
        self.jobs.attach(&job, task.abort_handle());
        self.jobs.info(&job).ok_or_else(|| ApiError::internal("the job went"))
    }
}

/// The machine's: a wadspace with Steam, built and opened in a window.
pub struct RealSpaces {
    pub docs: Arc<Docs>,
    pub builds: Arc<crate::builds::Builds>,
    pub runs: Arc<crate::runs::Runs>,
}

fn op(v: serde_json::Value) -> Result<Op, String> {
    serde_json::from_value(v).map_err(|e| e.to_string())
}

#[async_trait]
impl Spaces for RealSpaces {
    async fn open(&self, key: &str) -> Result<String, String> {
        let name = "Steam sign-in";
        let made = self.docs.apply(op(json!({ "op": "space.create", "name": name }))?).map_err(|e| e.message)?;
        let ws = made.created.ok_or("no wadspace was made")?;
        self.docs.apply(op(json!({ "op": "apps.add", "ws": ws, "app": "steam" }))?).map_err(|e| e.message)?;
        self.docs
            .apply(Op::SwapKey { ws: ws.clone(), slot: "app:steam".into(), key: key.to_string() })
            .map_err(|e| e.message)?;
        if let Err(e) = self.builds.request(&ws).await {
            self.close(&ws).await;
            return Err(e.message);
        }
        // Built (opened at once), or its build still running. A build that
        // ended with no image failed: say so now, rather than wait 30 minutes
        // for a sign-in that can't happen. (Its run isn't looked at before
        // it's opened: a wadspace never opened is Idle.)
        for _ in 0..1800 {
            let doc = self.docs.wadspace(&ws).ok_or("the wadspace went")?;
            let build = self.builds.state_of(&doc);
            if build.image.is_some() {
                self.runs.open(&ws, SessionDisplay::Window).map_err(|e| e.message)?;
                return Ok(ws);
            }
            if build.job.is_none() {
                self.close(&ws).await;
                return Err("Steam's wadspace didn't build (Activity has why)".into());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        self.close(&ws).await;
        Err("Steam didn’t get built on this machine".into())
    }

    async fn close(&self, ws: &str) {
        let _ = self.runs.stop(ws).await;
        let _ = self.docs.apply(Op::Trash { ws: ws.to_string() });
        let _ = self.docs.purge(ws);
    }
}
