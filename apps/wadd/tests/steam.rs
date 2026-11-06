//! Steam on the fake machine: a pretend client writes the files a real one
//! does, and the registry is fake. Nothing reaches Steam or GitHub, so what
//! this can't say is whether the real client does what these files show.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use common::{Rig, rig};
use wad_model::{JobStatus, Op, SecretKind};
use wad_proto::{SteamPush, SteamSignIn};
use wadd::discs::{Account, NEEDS_PACKAGES, Registry};
use wadd::steam::{MachineSteam, Spaces, plain_name, read_folder, read_plain};

const LOGIN: &str = r#""users" { "7656" { "AccountName" "sam" "PersonaName" "Sam" "MostRecent" "1" } }"#;
const LOCAL: &str =
    r#""UserLocalConfigStore" { "Software" { "Valve" { "Steam" { "apps" { "620" { } "99" { } } } } } }"#;

fn client(folder: &Path) {
    let s = folder.join(".local/share/Steam");
    std::fs::create_dir_all(s.join("config")).unwrap();
    std::fs::create_dir_all(s.join("userdata/1/config")).unwrap();
    std::fs::create_dir_all(s.join("steamapps/common/Portal 2")).unwrap();
    std::fs::write(s.join("config/loginusers.vdf"), LOGIN).unwrap();
    std::fs::write(s.join("userdata/1/config/localconfig.vdf"), LOCAL).unwrap();
    let game = r#""AppState" { "appid" "620" "name" "Portal 2" "installdir" "Portal 2" "StateFlags" "4" }"#;
    std::fs::write(s.join("steamapps/appmanifest_620.acf"), game).unwrap();
    let tool = r#""AppState" { "appid" "1391110" "name" "Steam Linux Runtime" "installdir" "SLR" "StateFlags" "4" }"#;
    std::fs::write(s.join("steamapps/appmanifest_1391110.acf"), tool).unwrap();
}

struct FakeSpaces {
    keys: PathBuf,
    signs_in: bool,
    closed: Mutex<Vec<String>>,
}

#[async_trait]
impl Spaces for FakeSpaces {
    async fn open(&self, key: &str) -> Result<String, String> {
        if self.signs_in {
            client(&self.keys.join(key));
        }
        Ok("temporary".into())
    }
    async fn close(&self, ws: &str) {
        self.closed.lock().unwrap().push(ws.to_string());
    }
}

#[derive(Default)]
struct Fake {
    who: Option<Account>,
    pushed: Mutex<Vec<(Vec<String>, String)>>,
}

#[async_trait]
impl Registry for Fake {
    async fn login(&self) -> Result<Option<Account>, String> {
        Ok(self.who.clone())
    }
    async fn push(&self, _: &Path, paths: &[String], reference: &str, _: &Account) -> Result<(), String> {
        self.pushed.lock().unwrap().push((paths.to_vec(), reference.to_string()));
        Ok(())
    }
    async fn pull(&self, _: &str, _: &Path, _: &Account) -> Result<(), String> {
        Ok(())
    }
}

fn steam(rig: &Rig, signs_in: bool, reg: Arc<Fake>) -> (Arc<MachineSteam>, Arc<FakeSpaces>, PathBuf) {
    let keys = rig.dir.path().join("keys");
    std::fs::create_dir_all(&keys).unwrap();
    let spaces = Arc::new(FakeSpaces { keys: keys.clone(), signs_in, closed: Mutex::default() });
    let s =
        MachineSteam::new(&keys, spaces.clone(), reg, rig.docs.clone(), rig.jobs.clone(), Duration::from_millis(50));
    (s, spaces, keys)
}

fn pusher() -> Account {
    Account { login: "hunggod".into(), token: "t".into(), scopes: vec!["write:packages".into()] }
}

#[tokio::test]
async fn signing_in_reads_the_library_and_the_temporary_wadspace_goes() {
    let rig = rig();
    let (s, spaces, _) = steam(&rig, true, Arc::new(Fake::default()));
    let job = s.sign_in(SteamSignIn { name: "Weekend".into() }).unwrap();
    assert_eq!(rig.finished(&job.job.id).await, JobStatus::Done);
    assert_eq!(*spaces.closed.lock().unwrap(), ["temporary"]);
    let a = s.accounts();
    assert_eq!((a.len(), a[0].name.as_str()), (1, "Weekend"));
    let ids: Vec<&str> = a[0].games.iter().map(|g| g.id.as_str()).collect();
    assert_eq!(ids.len(), 2, "{ids:?}: the runtime isn't a game");
    let portal = a[0].games.iter().find(|g| g.name == "Portal 2").unwrap();
    assert!(portal.installed && !portal.pushed);
    assert!(!a[0].games.iter().any(|g| g.name.contains("Runtime")));
}

#[tokio::test]
async fn nobody_signing_in_ends_it_and_leaves_no_folder() {
    let rig = rig();
    let (s, spaces, keys) = steam(&rig, false, Arc::new(Fake::default()));
    let job = s.sign_in(SteamSignIn { name: "Weekend".into() }).unwrap();
    assert_eq!(rig.finished(&job.job.id).await, JobStatus::Error);
    assert_eq!(spaces.closed.lock().unwrap().len(), 1);
    assert_eq!(std::fs::read_dir(keys).unwrap().count(), 0);
}

#[tokio::test]
async fn a_chosen_game_is_pushed_with_its_manifest() {
    let rig = rig();
    let reg = Arc::new(Fake { who: Some(pusher()), ..Fake::default() });
    let (s, _, keys) = steam(&rig, true, reg.clone());
    let key = rig.docs.apply(Op::NewKey { name: "W".into(), kind: SecretKind::Steam }).unwrap().created.unwrap();
    client(&keys.join(&key));
    let id = s.accounts()[0].games.iter().find(|g| g.installed).unwrap().id.clone();
    let job = s.push(SteamPush { account: key.clone(), games: vec![id.clone()] }).await.unwrap();
    assert_eq!(rig.finished(&job.job.id).await, JobStatus::Done);
    let pushed = reg.pushed.lock().unwrap();
    assert_eq!(pushed[0].0, ["common/Portal 2", "appmanifest_620.acf"]);
    assert!(pushed[0].1.ends_with(":620"), "{}", pushed[0].1);
    drop(pushed);
    assert!(s.accounts()[0].games.iter().find(|g| g.id == id).unwrap().pushed);
}

#[tokio::test]
async fn what_isnt_downloaded_or_allowed_isnt_pushed() {
    let rig = rig();
    let first = Account { scopes: vec!["repo".into()], ..pusher() };
    let (s, _, keys) = steam(&rig, true, Arc::new(Fake { who: Some(first), ..Fake::default() }));
    client(&keys.join("k"));
    let games = vec!["app-620".to_string()];
    let e = s.push(SteamPush { account: "k".into(), games: games.clone() }).await.unwrap_err();
    assert_eq!(e.message, NEEDS_PACKAGES);
    let (s, _, _) = steam(&rig, true, Arc::new(Fake { who: Some(pusher()), ..Fake::default() }));
    let e = s.push(SteamPush { account: "k".into(), games: vec!["app-99".into()] }).await.unwrap_err();
    assert!(e.message.contains("isn’t downloaded here"), "{}", e.message);
}

#[test]
fn root_never_follows_what_a_wadspace_planted() {
    let d = tempfile::tempdir().unwrap();
    let secret = d.path().join("shadow");
    std::fs::write(&secret, LOGIN).unwrap();
    let folder = d.path().join("key");
    let config = folder.join(".local/share/Steam/config");
    std::fs::create_dir_all(&config).unwrap();
    std::os::unix::fs::symlink(&secret, config.join("loginusers.vdf")).unwrap();
    assert_eq!(read_plain(&config.join("loginusers.vdf")), None);
    assert!(read_folder(&folder).is_none());
    std::fs::write(d.path().join("real"), "x").unwrap();
    assert_eq!(read_plain(&d.path().join("real")).as_deref(), Some("x"));
    for bad in ["", ".", "..", "../../etc", "a/b"] {
        assert!(!plain_name(bad), "{bad}");
    }
    assert!(plain_name("Portal 2"));
}
