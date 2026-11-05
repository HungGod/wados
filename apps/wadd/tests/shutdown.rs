//! wadd stops when it's told to, even with an event stream open (the shell's,
//! Settings', a peer's never end by themselves): systemd gives it 10 s.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use wad_config::{Config, Profile};
use wad_wadd_client::Wadd;
use wadd::fake::{Fake, FakeHands};

#[tokio::test]
async fn it_stops_with_an_event_stream_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::defaults(Profile::User);
    config.machine.id = "m-test".into();
    config.daemon.state_dir = dir.path().join("state");
    config.daemon.signins_dir = dir.path().join("state/keys");
    config.daemon.screen_dir = dir.path().join("screen");
    config.display.runtime_dir = dir.path().join("rt");
    config.packages.pull = false;
    config.streaming.enabled = false;
    std::fs::create_dir_all(&config.display.runtime_dir).unwrap();
    let sock = dir.path().join("wadd.sock");
    let parts = wadd::Parts {
        backend: Arc::new(Fake::new()),
        hands: Arc::new(FakeHands::default()),
        poll: Duration::from_millis(20),
        sample: None,
    };
    let server = wadd::Server::with_parts(
        Profile::User,
        config,
        wadd::Listen::Path(sock.clone()),
        wadd::logbuf::LogBuffer::new(100),
        parts,
    )
    .unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(server.serve_until(async move {
        let _ = stopped.await;
    }));

    // A watcher, connected: it has had wadd's first events.
    let heard = Arc::new(AtomicBool::new(false));
    let h = heard.clone();
    let follow = Wadd::new(sock).follow(move |_| {
        h.store(true, Ordering::SeqCst);
        true
    });
    for _ in 0..200 {
        if heard.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(heard.load(Ordering::SeqCst), "the event stream never connected");

    let asked = std::time::Instant::now();
    let _ = stop.send(());
    let done = tokio::time::timeout(Duration::from_secs(8), serving).await;
    follow.abort();
    assert!(done.is_ok(), "wadd was still serving 8 s after it was told to stop");
    assert!(asked.elapsed() < Duration::from_secs(6), "it took {:?}", asked.elapsed());
}
