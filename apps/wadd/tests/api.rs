//! wadd's routes for builds, runs, jobs and the screen, through its socket
//! (wad-wadd-client), on the fake podman, systemd and screen hands.

use std::sync::Arc;
use std::time::Duration;

use wad_config::{Config, Profile};
use wad_gen::{BASE_IMAGE, labels};
use wad_model::{Built, JobStatus, Phase, SessionDisplay};
use wad_proto::{Event, ScreenFrom};
use wad_wadd_client::{Incoming, Wadd};
use wadd::fake::{Fake, FakeHands};

struct Up {
    _dir: tempfile::TempDir,
    w: Wadd,
    fake: Arc<Fake>,
    hands: Arc<FakeHands>,
    shell: std::path::PathBuf,
    _compositor: std::os::unix::net::UnixListener,
}

async fn up() -> Up {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::defaults(Profile::User);
    config.machine.id = "m-test".into();
    config.daemon.state_dir = dir.path().join("state");
    config.daemon.signins_dir = dir.path().join("state/keys");
    config.daemon.screen_dir = dir.path().join("screen");
    config.daemon.reconcile_s = 1;
    config.display.runtime_dir = dir.path().join("rt");
    config.packages.pull = false;
    std::fs::create_dir_all(&config.display.runtime_dir).unwrap();
    let fake = Arc::new(Fake::new());
    fake.add_image(BASE_IMAGE, &[(labels::ABI, "fedora44.1"), (labels::EXTRAS, "1")]);
    let hands = Arc::new(FakeHands::default());
    let shell = dir.path().join("shell.sock");
    let compositor = std::os::unix::net::UnixListener::bind(&shell).unwrap();
    let sock = dir.path().join("wadd.sock");
    let parts = wadd::Parts { backend: fake.clone(), hands: hands.clone(), poll: Duration::from_millis(20) };
    let server = wadd::Server::with_parts(
        Profile::User,
        config,
        wadd::Listen::Path(sock.clone()),
        wadd::logbuf::LogBuffer::new(100),
        parts,
    )
    .unwrap();
    tokio::spawn(server.serve_until(std::future::pending()));
    Up { _dir: dir, w: Wadd::new(sock), fake, hands, shell, _compositor: compositor }
}

async fn create(w: &Wadd, name: &str, addons: &[&str]) -> String {
    let op = serde_json::from_value(serde_json::json!({"op": "space.create", "name": name})).unwrap();
    let id = w.op(op).await.unwrap().created.unwrap();
    let op = serde_json::from_value(serde_json::json!({"op": "apps.add", "ws": id, "app": "terminal"})).unwrap();
    w.op(op).await.unwrap();
    for a in addons {
        let op =
            serde_json::from_value(serde_json::json!({"op": "builder.add", "ws": id, "section": "addons", "item": a}))
                .unwrap();
        w.op(op).await.unwrap();
    }
    id
}

async fn until<T>(what: &str, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    for _ in 0..250 {
        if let Some(t) = f().await {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{what} didn't happen");
}

#[tokio::test]
async fn a_wadspace_opens_on_the_shells_screen_and_stops() {
    let up = up().await;
    let w = &up.w;
    let me = nix::unistd::getuid().as_raw();
    // Before the shell says, there's no screen (the runtime dir has none).
    let info: wad_proto::ScreenInfo = w.get("/v1/screen").await.unwrap();
    assert_eq!(info.from, ScreenFrom::None);
    let info = w.screen_session(&up.shell.to_string_lossy(), me).await.unwrap();
    assert_eq!(info.from, ScreenFrom::Shell);
    let bad = w.screen_session("wayland-0", me).await.unwrap_err();
    assert_eq!((bad.status, bad.code.as_str()), (400, "bad_request"));

    let id = create(w, "Plain", &[]).await;
    let built = w.build(&id).await.unwrap();
    assert!(built.job.is_none());
    assert_eq!(built.state.state.built, Built::Built);
    assert_eq!(built.state.image.as_deref(), Some(BASE_IMAGE));

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let follow = w.follow(move |i| tx.send(i).is_ok());
    let st = w.open(&id, SessionDisplay::Window).await.unwrap();
    assert_eq!(st.display, Some(SessionDisplay::Window));
    let ready = until("ready", async || {
        w.runs().await.unwrap().into_iter().find(|r| r.id == id && r.state.phase == Phase::Ready)
    })
    .await;
    assert_eq!(ready.display, Some(SessionDisplay::Window));
    // Watchers heard it come up, shown in a window (the shell places it so).
    let heard = until("the ready event", async || {
        while let Ok(i) = rx.try_recv() {
            if let Incoming::Event(Event::Run(r)) = i
                && r.id == id
                && r.state.phase == Phase::Ready
            {
                return Some(r);
            }
        }
        None
    })
    .await;
    assert_eq!(heard.display, Some(SessionDisplay::Window));
    assert_eq!(up.hands.listens.lock().unwrap()[0].0, format!("io.wadspaces.{id}"));
    let info: wad_proto::ScreenInfo = w.get("/v1/screen").await.unwrap();
    assert_eq!(info.served, std::slice::from_ref(&id));

    let st = w.stop(&id).await.unwrap();
    assert_eq!(st.state.phase, Phase::Idle);
    assert!(up.fake.calls().contains(&format!("stop wad-{id}.service")));
    follow.abort();

    let e = w.open("nope", SessionDisplay::Screen).await.unwrap_err();
    assert_eq!((e.status, e.code.as_str()), (404, "not_found"));
    let e = w.open(&id, SessionDisplay::Stream).await.unwrap_err();
    assert_eq!(e.status, 400);
}

#[tokio::test]
async fn a_build_is_a_job_with_its_stages_and_log() {
    let up = up().await;
    let w = &up.w;
    let id = create(w, "Node", &["terminal:nodejs"]).await;
    let builds = w.builds().await.unwrap();
    assert_eq!(builds.iter().find(|b| b.id == id).map(|b| b.state.built), Some(Built::Never));
    // Not built: it doesn't open.
    let e = w.open(&id, SessionDisplay::Screen).await.unwrap_err();
    assert_eq!(e.status, 409);

    let answer = w.build(&id).await.unwrap();
    let job = answer.job.expect("a job").job.id;
    let done = until("the build", async || {
        let j = w.job(&job, 0).await.unwrap();
        matches!(j.job.job.status, JobStatus::Done | JobStatus::Error).then_some(j)
    })
    .await;
    assert_eq!(done.job.job.status, JobStatus::Done, "{:?}", done.job.job.error);
    assert!(done.lines.iter().any(|l| l.contains("Node.js: building it here")), "{:?}", done.lines);
    assert_eq!(done.job.job.stages.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["pkg:nodejs", "stack", "set"]);
    let later = w.job(&job, done.job.line_count - 1).await.unwrap();
    assert_eq!(later.lines.len(), 1);
    assert_eq!(w.jobs().await.unwrap()[0].job.id, job);
    let b = until("built", async || {
        w.builds().await.unwrap().into_iter().find(|b| b.id == id && b.state.built == Built::Built)
    })
    .await;
    assert!(b.image.unwrap().starts_with("localhost/wados-set-"));
    // Cancelling one that's over changes nothing.
    assert_eq!(w.cancel_job(&job).await.unwrap().job.status, JobStatus::Done);
    assert_eq!(w.job("nope", 0).await.unwrap_err().status, 404);

    // Edited: an update, until it's built again.
    let op =
        serde_json::from_value(serde_json::json!({"op": "space.addCommand", "ws": id, "kind": "run", "text": "true"}))
            .unwrap();
    assert!(w.op(op).await.unwrap().needs_build);
    let b = w.builds().await.unwrap().into_iter().find(|b| b.id == id).unwrap();
    assert_eq!(b.state.built, Built::Update);
}
