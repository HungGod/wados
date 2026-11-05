//! Phase 8's routes (the system apps': api_phase8.rs) through the socket
//! (wad-wadd-client's typed calls): each answers on the sample
//! (fake_phase8.rs), and 501 on the machine's until they're made. And
//! deleting a wadspace out of the Trash for good, which is real.

use std::sync::Arc;
use std::time::Duration;

use wad_config::{Config, Profile};
use wad_model::{JobKind, JobStatus};
use wad_proto::{BtAnswer, BtAsk, Event, JobInfo, Level};
use wad_wadd_client::{Incoming, Wadd};
use wadd::fake::{Fake, FakeHands};

const HEADPHONES: &str = "AA:BB:CC:00:00:01";
const MOUSE: &str = "AA:BB:CC:00:00:02";
const NEARBY: &str = "AA:BB:CC:00:00:03";

struct Up {
    _dir: tempfile::TempDir,
    w: Wadd,
}

async fn up(sample: Option<Duration>) -> Up {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::defaults(Profile::User);
    config.machine.id = "m-test".into();
    config.machine.name = "Test".into();
    config.daemon.state_dir = dir.path().join("state");
    config.daemon.signins_dir = dir.path().join("state/keys");
    config.daemon.screen_dir = dir.path().join("screen");
    config.display.runtime_dir = dir.path().join("rt");
    config.packages.pull = false;
    config.streaming.listen = vec![];
    config.streaming.secure_runtime = String::new();
    std::fs::create_dir_all(&config.display.runtime_dir).unwrap();
    let parts = wadd::Parts {
        backend: Arc::new(Fake::new()),
        hands: Arc::new(FakeHands::default()),
        poll: Duration::from_millis(20),
        sample,
    };
    let sock = dir.path().join("wadd.sock");
    let server = wadd::Server::with_parts(
        Profile::User,
        config,
        wadd::Listen::Path(sock.clone()),
        wadd::logbuf::LogBuffer::new(100),
        parts,
    )
    .unwrap();
    tokio::spawn(server.serve_until(std::future::pending()));
    Up { _dir: dir, w: Wadd::new(sock) }
}

async fn sample() -> Up {
    up(Some(Duration::from_millis(5))).await
}

/// Waits for a job to end; it, then.
async fn ended(w: &Wadd, job: &JobInfo) -> JobInfo {
    for _ in 0..500 {
        let j = w.job(&job.job.id, 0).await.unwrap().job;
        if wadd::jobs::finished(j.job.status) {
            return j;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{} didn't end", job.job.id);
}

/// Follows the events from now on (once the present has come): what came in the present, too.
async fn follow(w: &Wadd) -> (tokio::task::JoinHandle<()>, tokio::sync::mpsc::UnboundedReceiver<Incoming>, Vec<Event>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let follow = w.follow(move |i| tx.send(i).is_ok());
    let present = tokio::time::timeout(Duration::from_secs(5), async {
        let mut present = vec![];
        while let Some(i) = rx.recv().await {
            match i {
                Incoming::Event(Event::Library(_)) => return present,
                Incoming::Event(e) => present.push(e),
                _ => {}
            }
        }
        present
    })
    .await
    .expect("the present");
    (follow, rx, present)
}

/// The next event `f` picks out.
async fn next<T>(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Incoming>, f: impl Fn(Event) -> Option<T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(i) = rx.recv().await {
            if let Incoming::Event(e) = i
                && let Some(t) = f(e)
            {
                return t;
            }
        }
        panic!("the events ended");
    })
    .await
    .expect("the event")
}

#[tokio::test]
async fn the_samples_present_is_told_to_a_new_watcher() {
    let up = sample().await;
    let (f, _rx, present) = follow(&up.w).await;
    let kinds: Vec<String> = present.iter().filter_map(|e| e.latest_key()).collect();
    for k in ["network", "bluetooth", "updates", "storage"] {
        assert!(kinds.iter().any(|x| x == k), "{k} isn't in {kinds:?}");
    }
    f.abort();
}

#[tokio::test]
async fn wifi_and_airplane_mode() {
    let up = sample().await;
    let w = &up.w;
    let n = w.network().await.unwrap();
    assert!(n.wifi_on && !n.airplane && !n.ethernet.plugged);
    assert_eq!(n.connected.as_deref(), Some("Home"));
    assert_eq!(n.networks.len(), 5);
    assert_eq!(n.networks.iter().filter(|x| x.saved).count(), 2);
    assert_eq!(n.networks.iter().filter(|x| !x.secure).count(), 1);
    assert!(n.networks[0].active && n.networks[0].ssid == "Home");
    assert!(n.ip.is_some());

    // Joining: an open one; a secure one needs its password, and the right one.
    let n = w.wifi_connect("Vanua Cafe", None).await.unwrap();
    assert_eq!(n.connected.as_deref(), Some("Vanua Cafe"));
    assert_eq!(w.wifi_connect("Neighbours", None).await.unwrap_err().status, 400);
    assert_eq!(w.wifi_connect("Neighbours", Some("wrongpassword")).await.unwrap_err().status, 401);
    let n = w.wifi_connect("Neighbours", Some("correct horse")).await.unwrap();
    assert!(n.networks.iter().any(|x| x.ssid == "Neighbours" && x.saved && x.active));
    assert_eq!(w.wifi_connect("Nowhere", None).await.unwrap_err().status, 404);

    let n = w.wifi_disconnect().await.unwrap();
    assert!(n.connected.is_none() && n.ip.is_none() && n.networks.iter().all(|x| !x.active));
    let n = w.wifi_forget("Neighbours").await.unwrap();
    assert!(n.networks.iter().any(|x| x.ssid == "Neighbours" && !x.saved));
    assert_eq!(w.wifi_forget("Neighbours").await.unwrap_err().status, 404);

    let n = w.set_wifi(false).await.unwrap();
    assert!(!n.wifi_on && n.connected.is_none());
    let n = w.set_wifi(true).await.unwrap();
    assert_eq!(n.connected.as_deref(), Some("Home"), "a saved one, again");

    // Airplane mode: every radio off, Bluetooth too.
    let n = w.set_airplane(true).await.unwrap();
    assert!(n.airplane && !n.wifi_on && n.connected.is_none());
    let b = w.bluetooth().await.unwrap();
    assert!(!b.on && b.devices.iter().all(|d| !d.connected));
    assert_eq!(w.set_wifi(true).await.unwrap_err().status, 409);
    assert_eq!(w.set_bluetooth(true).await.unwrap_err().status, 409);
    let n = w.set_airplane(false).await.unwrap();
    assert!(!n.airplane && n.wifi_on && n.connected.is_some());
}

#[tokio::test]
async fn the_exit_node_is_one_of_the_tailnets_online_computers() {
    let up = sample().await;
    let w = &up.w;
    let n = w.network().await.unwrap();
    let ids: Vec<&str> = n.exit_nodes.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["nDesk1"], "Desk (online), not the laptop (offline) or the phone");
    assert!(n.exit_node.is_none());
    let n = w.set_exit_node(Some("nDesk1")).await.unwrap();
    assert_eq!(n.exit_node.as_deref(), Some("nDesk1"));
    assert_eq!(w.set_exit_node(Some("nPhone1")).await.unwrap_err().status, 404);
    assert_eq!(w.set_exit_node(Some("nLaptop1")).await.unwrap_err().status, 404);
    let n = w.set_exit_node(None).await.unwrap();
    assert!(n.exit_node.is_none());
    // The tailnet logged out: there's none to go through.
    w.tailnet_logout().await.unwrap();
    assert!(w.network().await.unwrap().exit_nodes.is_empty());
    assert_eq!(w.set_exit_node(Some("nDesk1")).await.unwrap_err().status, 409);
}

#[tokio::test]
async fn bluetooth_devices() {
    let up = sample().await;
    let w = &up.w;
    let b = w.bluetooth().await.unwrap();
    assert!(b.on && !b.scanning && b.discoverable_name == "test");
    let headphones = b.devices.iter().find(|d| d.address == HEADPHONES).unwrap();
    assert!(headphones.paired && headphones.connected && headphones.battery == Some(80));
    let mouse = b.devices.iter().find(|d| d.address == MOUSE).unwrap();
    assert!(mouse.paired && !mouse.connected);
    assert_eq!(b.devices.iter().filter(|d| !d.paired).count(), 1);

    assert!(w.bt_scan(true).await.unwrap().scanning);
    assert!(!w.bt_scan(false).await.unwrap().scanning);
    let b = w.bt_connect(MOUSE).await.unwrap();
    assert!(b.devices.iter().any(|d| d.address == MOUSE && d.connected));
    let b = w.bt_disconnect(HEADPHONES).await.unwrap();
    assert!(b.devices.iter().any(|d| d.address == HEADPHONES && !d.connected));
    let b = w.bt_forget(MOUSE).await.unwrap();
    assert!(b.devices.iter().any(|d| d.address == MOUSE && !d.paired && !d.connected));
    assert_eq!(w.bt_connect(MOUSE).await.unwrap_err().status, 409, "not paired");
    assert_eq!(w.bt_connect("00:00:00:00:00:00").await.unwrap_err().status, 404);
    assert_eq!(
        w.post::<serde_json::Value>(&format!("/v1/bluetooth/devices/{MOUSE}/nope"), &()).await.unwrap_err().status,
        404
    );

    let b = w.set_bluetooth(false).await.unwrap();
    assert!(!b.on && b.devices.iter().all(|d| !d.connected));
    assert_eq!(w.bt_connect(HEADPHONES).await.unwrap_err().status, 409, "Bluetooth is off");
    assert!(w.set_bluetooth(true).await.unwrap().on);
}

#[tokio::test]
async fn pairing_the_nearby_device_asks_to_confirm_a_code() {
    let up = sample().await;
    let w = &up.w;
    let (f, mut rx, _) = follow(w).await;
    let b = w.bt_pair(NEARBY).await.unwrap();
    assert!(b.devices.iter().any(|d| d.address == NEARBY && !d.paired), "it doesn't wait");
    let prompt = next(&mut rx, |e| match e {
        Event::BtPrompt { prompt } => prompt,
        _ => None,
    })
    .await;
    assert_eq!((prompt.device.as_str(), prompt.name.as_str()), (NEARBY, "JBL Flip 6"));
    assert!(matches!(&prompt.kind, BtAsk::Confirm { code } if code.len() == 6));

    assert_eq!(w.bt_answer("nope", &BtAnswer { accept: true, pin: None }).await.unwrap_err().status, 404);
    let b = w.bt_answer(&prompt.id, &BtAnswer { accept: true, pin: None }).await.unwrap();
    assert!(b.devices.iter().any(|d| d.address == NEARBY && d.paired && d.connected));
    let gone = next(&mut rx, |e| match e {
        Event::BtPrompt { prompt } => Some(prompt),
        _ => None,
    })
    .await;
    assert!(gone.is_none());
    assert_eq!(w.bt_answer(&prompt.id, &BtAnswer { accept: true, pin: None }).await.unwrap_err().status, 404);

    // Said no: not paired, and a notice says so.
    w.bt_forget(NEARBY).await.unwrap();
    w.bt_pair(NEARBY).await.unwrap();
    let again = next(&mut rx, |e| match e {
        Event::BtPrompt { prompt } => prompt,
        _ => None,
    })
    .await;
    assert_ne!(again.id, prompt.id);
    let b = w.bt_answer(&again.id, &BtAnswer { accept: false, pin: None }).await.unwrap();
    assert!(b.devices.iter().any(|d| d.address == NEARBY && !d.paired));
    let title = next(&mut rx, |e| match e {
        Event::Notice { title, .. } => Some(title),
        _ => None,
    })
    .await;
    assert!(title.contains("wasn't paired"), "{title}");
    f.abort();
}

#[tokio::test]
async fn an_update_found_applied_cancelled_and_rolled_back() {
    let up = sample().await;
    let w = &up.w;
    let u = w.update_status().await.unwrap();
    assert!(u.online && !u.checking && u.staged.is_none());
    assert_eq!(u.booted.version, "44.20261104.0");
    assert_eq!(u.available.as_ref().unwrap().version, "44.20261105.0");
    assert_eq!(u.rollback.as_ref().unwrap().version, "44.20261028.0");

    let job = w.update_check().await.unwrap();
    assert_eq!(job.job.kind, JobKind::Update);
    assert_eq!(ended(w, &job).await.job.status, JobStatus::Done);
    let u = w.update_status().await.unwrap();
    assert!(!u.checking && u.job.is_none() && u.checked_at.is_some());
    assert_eq!(u.available.as_ref().unwrap().version, "44.20261105.0");

    let (f, mut rx, _) = follow(w).await;
    let job = w.update_apply().await.unwrap();
    let stages: Vec<&str> = job.job.stages.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(stages, ["download", "stage"]);
    assert_eq!(w.update_check().await.unwrap_err().status, 409, "one at a time");
    assert_eq!(ended(w, &job).await.job.status, JobStatus::Done);
    let u = w.update_status().await.unwrap();
    assert_eq!(u.staged.as_ref().unwrap().version, "44.20261105.0");
    assert!(u.available.is_none() && u.progress.is_none() && u.job.is_none());
    let (title, action) = next(&mut rx, |e| match e {
        Event::Notice { title, action, level: Level::Ok, .. } => Some((title, action)),
        _ => None,
    })
    .await;
    assert!(title.contains("44.20261105.0"), "{title}");
    assert_eq!(action.unwrap().verb, "power.restart");
    f.abort();
    assert_eq!(w.update_apply().await.unwrap_err().status, 409, "nothing to apply");
    // A check with it staged finds nothing newer.
    ended(w, &w.update_check().await.unwrap()).await;
    assert!(w.update_status().await.unwrap().available.is_none());

    let u = w.update_cancel().await.unwrap();
    assert!(u.staged.is_none());
    assert_eq!(u.available.as_ref().unwrap().version, "44.20261105.0", "it's available again");
    assert_eq!(w.update_cancel().await.unwrap_err().status, 409);
    let u = w.update_rollback().await.unwrap();
    assert_eq!(u.staged.as_ref().unwrap().version, "44.20261028.0");
    assert_eq!(w.update_rollback().await.unwrap_err().status, 409);
}

#[tokio::test]
async fn the_base_images_and_their_refresh() {
    let up = sample().await;
    let w = &up.w;
    let b = w.bases().await.unwrap();
    assert_eq!(b.bases.len(), 3);
    assert_eq!(b.bases.iter().filter(|x| x.missing).count(), 1);
    assert!(b.next_refresh_at.is_some());
    let job = w.refresh_bases().await.unwrap();
    assert_eq!((job.job.kind, job.job.stages.len()), (JobKind::Pull, 3));
    assert_eq!(w.refresh_bases().await.unwrap_err().status, 409);
    assert_eq!(ended(w, &job).await.job.status, JobStatus::Done);
    let after = w.bases().await.unwrap();
    assert!(after.bases.iter().all(|x| !x.missing && x.size.is_some() && x.refreshed_at.is_some()));
    assert!(after.next_refresh_at > b.next_refresh_at);
}

#[tokio::test]
async fn storage_and_its_cleanup() {
    let up = sample().await;
    let w = &up.w;
    let s = w.storage().await.unwrap();
    assert_eq!(s.total, 512_000_000_000);
    assert_eq!(s.free + s.bases + s.wadspaces + s.other, s.total);
    assert!(s.bases > 0 && s.wadspaces > 0 && s.other > 0);
    assert_eq!(s.per_wadspace.len(), 1, "the starter");
    assert_eq!(s.per_wadspace.iter().map(|x| x.bytes).sum::<u64>(), s.wadspaces);

    let job = w.storage_cleanup().await.unwrap();
    assert_eq!(job.job.kind, JobKind::Cleanup);
    assert_eq!(ended(w, &job).await.job.status, JobStatus::Done);
    let after = w.storage().await.unwrap();
    assert_eq!(after.free - s.free, 6_500_000_000);
    ended(w, &w.storage_cleanup().await.unwrap()).await;
    assert_eq!(w.storage().await.unwrap().free, after.free, "nothing more to free");
}

#[tokio::test]
async fn the_machines_answer_501_until_phase_8_is_made() {
    let up = up(None).await;
    let w = &up.w;
    let all: Vec<wad_wadd_client::Failure> = vec![
        w.network().await.unwrap_err(),
        w.set_wifi(true).await.unwrap_err(),
        w.set_airplane(true).await.unwrap_err(),
        w.wifi_connect("Home", Some("password")).await.unwrap_err(),
        w.wifi_disconnect().await.unwrap_err(),
        w.wifi_forget("Home").await.unwrap_err(),
        w.set_exit_node(None).await.unwrap_err(),
        w.bluetooth().await.unwrap_err(),
        w.set_bluetooth(true).await.unwrap_err(),
        w.bt_scan(true).await.unwrap_err(),
        w.bt_pair(NEARBY).await.unwrap_err(),
        w.bt_connect(MOUSE).await.unwrap_err(),
        w.bt_disconnect(MOUSE).await.unwrap_err(),
        w.bt_forget(MOUSE).await.unwrap_err(),
        w.bt_answer("p", &BtAnswer { accept: true, pin: None }).await.unwrap_err(),
        w.update_status().await.unwrap_err(),
        w.update_check().await.unwrap_err(),
        w.update_apply().await.unwrap_err(),
        w.update_cancel().await.unwrap_err(),
        w.update_rollback().await.unwrap_err(),
        w.bases().await.unwrap_err(),
        w.refresh_bases().await.unwrap_err(),
        w.storage().await.unwrap_err(),
        w.storage_cleanup().await.unwrap_err(),
    ];
    for e in all {
        assert_eq!(e.status, 501, "{}", e.message);
        assert!(e.message.ends_with("comes in phase 8"), "{}", e.message);
    }
}

/// A wadspace of this machine's: its id.
async fn create(w: &Wadd, name: &str) -> String {
    let op = serde_json::from_value(serde_json::json!({"op": "space.create", "name": name})).unwrap();
    w.op(op).await.unwrap().created.unwrap()
}

#[tokio::test]
async fn a_wadspace_in_the_trash_goes_for_good() {
    // Real on the machine's (no sample).
    let up = up(None).await;
    let w = &up.w;
    let id = create(w, "Scratch").await;
    assert_eq!(w.purge_wadspace(&id).await.unwrap_err().status, 409, "not in the Trash");
    let trashed = w.op(serde_json::from_value(serde_json::json!({"op": "ws.trash", "ws": id})).unwrap()).await.unwrap();
    let (f, mut rx, _) = follow(w).await;
    let out = w.purge_wadspace(&id).await.unwrap();
    assert_eq!(out.changed, std::slice::from_ref(&id));
    let gone = next(&mut rx, |e| match e {
        Event::WadspaceGone { id } => Some(id),
        _ => None,
    })
    .await;
    assert_eq!(gone, id);
    f.abort();
    assert_eq!(w.get::<serde_json::Value>(&format!("/v1/wadspaces/{id}")).await.unwrap_err().status, 404);
    let lib: wad_model::edit::Library = w.get("/v1/library").await.unwrap();
    assert!(lib.wadspace(&id).is_none());
    // Its Trash's Undo can't bring it back, and it can't go twice.
    assert_eq!(w.undo(&trashed.undo.unwrap()).await.unwrap_err().status, 404);
    assert_eq!(w.purge_wadspace(&id).await.unwrap_err().status, 404);
}
