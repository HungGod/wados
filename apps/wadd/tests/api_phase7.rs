//! Phase 7's routes (streams: api_phase7.rs) through the socket
//! (wad-wadd-client's typed calls): each answers on the sample
//! (fake_phase7.rs), and 501 on the machine's until streams are made. The
//! remote API's are in tests/tailnet.rs (they need a tailnet).

use std::sync::Arc;
use std::time::Duration;

use wad_config::{Config, Profile};
use wad_model::{JobKind, JobStatus, StreamMode, StreamStage, Transport};
use wad_proto::streams::{PRIVATE_NEEDS_TAILNET, PUBLIC_NEEDS_TUNNEL, SECURE_NEEDS_VM, SECURE_PUBLIC_LATER};
use wad_proto::{Event, JobInfo, StreamRequest};
use wad_wadd_client::{Incoming, Wadd};
use wadd::fake::{Fake, FakeHands};

/// The starter every new library has: the sample streams it publicly.
const STARTER: &str = "wadspaces";

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
    // No public listener in tests, and no VM runtime, whatever this machine has.
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

/// Waits for `f` to hold of the streams.
async fn until(w: &Wadd, what: &str, f: impl Fn(&wad_proto::StreamList) -> bool) {
    for _ in 0..500 {
        if f(&w.streams().await.unwrap()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what} didn't happen");
}

/// Follows the events from now on (once the present has come).
async fn follow(w: &Wadd) -> (tokio::task::JoinHandle<()>, tokio::sync::mpsc::UnboundedReceiver<Incoming>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let follow = w.follow(move |i| tx.send(i).is_ok());
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(i) = rx.recv().await {
            if matches!(i, Incoming::Event(Event::Library(_))) {
                return;
            }
        }
    })
    .await
    .expect("the present");
    (follow, rx)
}

/// A wadspace of this machine's: its id.
async fn create(w: &Wadd, name: &str) -> String {
    let op = serde_json::from_value(serde_json::json!({"op": "space.create", "name": name})).unwrap();
    w.op(op).await.unwrap().created.unwrap()
}

#[tokio::test]
async fn the_samples_streams_and_what_can_start() {
    let up = sample().await;
    let l = up.w.streams().await.unwrap();
    let public = l.streams.iter().find(|s| s.id == STARTER).expect("the starter's");
    assert_eq!(
        (public.mode, public.transport, public.stage),
        (StreamMode::Public, Transport::Webrtc, StreamStage::Live)
    );
    assert!(public.live && !public.turn_down && public.peer.is_none());
    assert_eq!(public.viewers, 2);
    assert!(!public.link.contains("token"), "{}", public.link);
    let private = l.streams.iter().find(|s| s.mode == StreamMode::Private).expect("a private one");
    assert_eq!((private.stage, private.live, private.peer.as_deref()), (StreamStage::Sidecar, false, Some("m-desk")));
    assert!(l.available.private.available && l.available.public.available);
    for (m, why) in [(StreamMode::PrivateSecure, SECURE_NEEDS_VM), (StreamMode::PublicSecure, SECURE_PUBLIC_LATER)] {
        let a = l.available.get(m);
        assert_eq!((a.available, a.reason.as_deref()), (false, Some(why)), "{m:?}");
    }
}

#[tokio::test]
async fn a_share_is_plainly_fake_and_only_a_public_one_signs_in() {
    let up = sample().await;
    let w = &up.w;
    let s = w.stream_share(STARTER).await.unwrap();
    assert_eq!(s.mode, StreamMode::Public);
    assert!(s.open_link.starts_with(&s.link) && s.open_link.contains("/#token="), "{}", s.open_link);
    assert!(s.link.contains(".invalid/") && s.open_link.contains("not-a-secret"));
    assert!(s.user.is_some() && s.password.as_deref().is_some_and(|p| p.contains("not-a-secret")));
    let private = w.stream_share("notes").await.unwrap();
    assert_eq!((private.mode, private.user, private.password), (StreamMode::Private, None, None));
    assert!(private.open_link.contains("#token="));
    assert_eq!(w.stream_share("nope").await.unwrap_err().status, 404);
}

#[tokio::test]
async fn a_stream_starts_as_a_job_goes_live_and_stops() {
    let up = sample().await;
    let w = &up.w;
    let id = create(w, "Studio").await;
    let (follow, mut rx) = follow(w).await;
    let job = w.start_stream(&id, &StreamRequest { mode: StreamMode::Private, takeover: false }).await.unwrap();
    assert_eq!((job.job.kind, job.job.subject.as_deref()), (JobKind::Stream, Some(id.as_str())));
    let labels: Vec<&str> = job.job.stages.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, ["Starting its container", "Starting the Selkies sidecar", "Serving it on your tailnet"]);
    let done = ended(w, &job).await;
    assert_eq!(done.job.status, JobStatus::Done, "{:?}", done.job.error);
    until(w, "live", |l| l.streams.iter().any(|s| s.id == id && s.live && s.stage == StreamStage::Live)).await;
    let s = w.streams().await.unwrap().streams.into_iter().find(|s| s.id == id).unwrap();
    assert_eq!((s.transport, s.job.as_deref()), (Transport::Websocket, Some(job.job.id.as_str())));

    // One stream each: stop it first.
    let again = StreamRequest { mode: StreamMode::Public, takeover: false };
    assert_eq!(w.start_stream(&id, &again).await.unwrap_err().status, 409);
    // Stopped: ended, then gone.
    let stopped = w.stop_stream(&id).await.unwrap();
    assert_eq!((stopped.stage, stopped.live), (StreamStage::Ended, false));
    until(w, "gone", |l| !l.streams.iter().any(|s| s.id == id)).await;
    assert_eq!(w.stop_stream(&id).await.unwrap_err().status, 404);
    assert_eq!(w.stream_share(&id).await.unwrap_err().status, 404);

    // Public, again: routed through Cloudflare, with its STUN and TURN checked.
    let job = w.start_stream(&id, &again).await.unwrap();
    assert!(job.job.stages.iter().any(|s| s.label == "Routing it through Cloudflare"));
    assert!(job.job.stages.iter().any(|s| s.label == "Checking Cloudflare's STUN and TURN"));
    ended(w, &job).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    follow.abort();
    let mut heard = vec![];
    while let Ok(i) = rx.try_recv() {
        if let Incoming::Event(Event::Streams { streams, .. }) = i
            && let Some(s) = streams.into_iter().find(|s| s.id == id)
        {
            heard.push((s.stage, s.live));
        }
    }
    // Each stage told as it came, then live; then ended; then (public) live again.
    let stages: Vec<StreamStage> = heard.iter().map(|(s, _)| *s).collect();
    for (a, b) in [
        (StreamStage::Container, StreamStage::Sidecar),
        (StreamStage::Sidecar, StreamStage::Serve),
        (StreamStage::Serve, StreamStage::Live),
        (StreamStage::Live, StreamStage::Ended),
        (StreamStage::Ended, StreamStage::Route),
        (StreamStage::Route, StreamStage::Turn),
    ] {
        let (i, j) = (stages.iter().position(|s| *s == a), stages.iter().rposition(|s| *s == b));
        assert!(i.is_some() && i < j, "{a:?} before {b:?}: {stages:?}");
    }
    assert_eq!(heard.last(), Some(&(StreamStage::Live, true)));
}

#[tokio::test]
async fn what_cant_start_says_why() {
    let up = sample().await;
    let w = &up.w;
    let id = create(w, "Studio").await;
    let e = w.start_stream(&id, &StreamRequest { mode: StreamMode::PublicSecure, takeover: false }).await.unwrap_err();
    assert_eq!((e.status, e.message.as_str()), (409, SECURE_PUBLIC_LATER));
    let e = w.start_stream("nope", &StreamRequest { mode: StreamMode::Private, takeover: false }).await.unwrap_err();
    assert_eq!(e.status, 404);
    // Streaming already (the sample's public one).
    let e = w.start_stream(STARTER, &StreamRequest { mode: StreamMode::Public, takeover: true }).await.unwrap_err();
    assert_eq!(e.status, 409, "{}", e.message);
    // A stream still starting, stopped: its job stops too, and it's gone.
    let job = w.start_stream(&id, &StreamRequest { mode: StreamMode::Public, takeover: false }).await.unwrap();
    w.stop_stream(&id).await.unwrap();
    let j = ended(w, &job).await;
    assert!(matches!(j.job.status, JobStatus::Cancelled | JobStatus::Done), "{:?}", j.job.status);
    until(w, "gone", |l| !l.streams.iter().any(|s| s.id == id)).await;
    // Another machine's, here: stopped too.
    assert_eq!(w.stop_stream("notes").await.unwrap().peer.as_deref(), Some("m-desk"));
}

#[tokio::test]
async fn a_new_watcher_gets_the_streams_first() {
    let up = sample().await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let follow = up.w.follow(move |i| tx.send(i).is_ok());
    let streams = tokio::time::timeout(Duration::from_secs(5), async {
        let mut found = None;
        while let Some(i) = rx.recv().await {
            match i {
                Incoming::Event(Event::Library(_)) => break,
                Incoming::Event(Event::Streams { streams, .. }) => found = Some(streams),
                _ => {}
            }
        }
        found
    })
    .await
    .expect("the present")
    .expect("the streams in it");
    follow.abort();
    assert_eq!(streams.len(), 2);
}

#[tokio::test]
async fn the_machines_streams_say_what_can_start() {
    let up = up(None).await;
    let w = &up.w;
    // A laptop off its tailnet, with no tunnel or VM runtime: nothing can start, each saying why.
    let l = w.streams().await.unwrap();
    assert!(l.streams.is_empty());
    assert_eq!(l.available.private.reason.as_deref(), Some(PRIVATE_NEEDS_TAILNET));
    assert_eq!(l.available.public.reason.as_deref(), Some(PUBLIC_NEEDS_TUNNEL));
    assert_eq!(l.available.public_secure.reason.as_deref(), Some(SECURE_PUBLIC_LATER));
    let req = StreamRequest { mode: StreamMode::Private, takeover: false };
    let e = w.start_stream(STARTER, &req).await.unwrap_err();
    assert_eq!((e.status, e.message.as_str()), (409, PRIVATE_NEEDS_TAILNET));
    assert_eq!(w.start_stream("nope", &req).await.unwrap_err().status, 404);
    assert_eq!(w.stop_stream(STARTER).await.unwrap_err().status, 404);
    assert_eq!(w.stream_share(STARTER).await.unwrap_err().status, 404);
    // No such machine of mine.
    assert_eq!(w.start_peer_stream("m-desk", "notes", &req).await.unwrap_err().status, 404);
}

#[tokio::test]
async fn the_sample_streams_from_its_peer() {
    let up = sample().await;
    let w = &up.w;
    let req = StreamRequest { mode: StreamMode::Private, takeover: false };
    let share = w.start_peer_stream("m-desk", "music", &req).await.unwrap();
    assert!(share.open_link.contains("#token=") && share.link.contains(".invalid"), "{share:?}");
    let s = w.streams().await.unwrap().streams.into_iter().find(|s| s.id == "music").expect("listed");
    assert_eq!((s.peer.as_deref(), s.live), (Some("m-desk"), true));
    let ended = w.stop_peer_stream("m-desk", "music").await.unwrap();
    assert_eq!(ended.stage, StreamStage::Ended);
    assert!(!w.streams().await.unwrap().streams.iter().any(|s| s.id == "music"));
    assert_eq!(w.start_peer_stream("m-nope", "music", &req).await.unwrap_err().status, 404);
}
