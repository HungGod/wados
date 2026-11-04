//! Phase 6's routes (the tailnet, peers, GitHub: api_phase6.rs) through the
//! socket (wad-wadd-client's typed calls): each answers on the sample
//! (fake_phase6.rs), and the machine's answer 501 until they're made.

use std::sync::Arc;
use std::time::Duration;

use wad_config::{Config, Profile};
use wad_model::{Built, JobKind, JobStatus};
use wad_proto::{Event, JobInfo, SignInProvider, SignInRequest, TailnetState};
use wad_wadd_client::{Incoming, Method, Wadd};
use wadd::fake::{Fake, FakeHands};

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

/// Every event heard so far.
fn heard(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Incoming>) -> Vec<Event> {
    let mut out = vec![];
    while let Ok(i) = rx.try_recv() {
        if let Incoming::Event(e) = i {
            out.push(e);
        }
    }
    out
}

#[tokio::test]
async fn the_tailnet_its_devices_and_asks() {
    let up = sample().await;
    let w = &up.w;
    let t = w.tailnet().await.unwrap();
    assert_eq!((t.state, t.tailnet.as_str(), t.hostname.as_str()), (TailnetState::Running, "tail1234.ts.net", "test"));
    assert_eq!(t.devices.iter().filter(|d| d.mine).count(), 2);
    assert_eq!(t.asks.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["nTab1"]);

    // An ask trusted: it's a trusted device; untrusted, it's one that isn't.
    let t = w.trust_device("nTab1").await.unwrap();
    assert!(t.asks.is_empty());
    assert!(t.devices.iter().any(|d| d.id == "nTab1" && d.trusted && !d.mine));
    let t = w.untrust_device("nTab1").await.unwrap();
    assert!(t.devices.iter().any(|d| d.id == "nTab1" && !d.trusted));
    // My machines are always trusted; a device there isn't is not found.
    assert_eq!(w.untrust_device("nDesk1").await.unwrap_err().status, 409);
    assert_eq!(w.trust_device("nope").await.unwrap_err().status, 404);
    assert_eq!(w.deny_ask("nTab1").await.unwrap_err().status, 404, "it's not asking now");
}

#[tokio::test]
async fn asks_denied_or_put_off_and_a_login() {
    let up = sample().await;
    let w = &up.w;
    let t = w.later_ask("nTab1").await.unwrap();
    assert!(t.asks.is_empty() && !t.devices.iter().any(|d| d.id == "nTab1"));

    // Logged out: no tailnet, no peers.
    let t = w.tailnet_logout().await.unwrap();
    assert_eq!((t.state, t.devices.len()), (TailnetState::NeedsLogin, 0));
    assert!(w.peers().await.unwrap().is_empty());

    // A login is a Tailnet job that signs in in the browser, then runs.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let follow = w.follow(move |i| tx.send(i).is_ok());
    let job = w.tailnet_login().await.unwrap();
    assert_eq!(job.job.kind, JobKind::Tailnet);
    assert!(job.job.stages.iter().any(|s| s.label == "Signing in to Tailscale in the browser"));
    assert_eq!(ended(w, &job).await.job.status, JobStatus::Done);
    let t = w.tailnet().await.unwrap();
    assert_eq!(t.state, TailnetState::Running);
    assert_eq!(w.peers().await.unwrap().len(), 2);
    tokio::time::sleep(Duration::from_millis(100)).await;
    follow.abort();
    let events = heard(&mut rx);
    let signin = events.iter().position(|e| matches!(e, Event::SignIn { provider: SignInProvider::Tailscale, .. }));
    let done = events.iter().position(|e| matches!(e, Event::SignInDone { provider: SignInProvider::Tailscale }));
    assert!(signin.is_some() && signin < done, "{events:?}");
    assert!(events.iter().any(|e| matches!(e, Event::Tailnet(t) if t.state == TailnetState::LoggingIn)));

    // Now the ask is back, denied: it stays, untrusted.
    let t = w.deny_ask("nTab1").await.unwrap();
    assert!(t.asks.is_empty());
    assert!(t.devices.iter().any(|d| d.id == "nTab1" && !d.trusted));
    assert_eq!(w.later_ask("nope").await.unwrap_err().status, 404);
}

#[tokio::test]
async fn peers_their_wadspaces_and_a_copy_here() {
    let up = sample().await;
    let w = &up.w;
    let peers = w.peers().await.unwrap();
    let online: Vec<(&str, bool)> = peers.iter().map(|p| (p.peer.id.as_str(), p.peer.online)).collect();
    assert_eq!(online, [("m-desk", true), ("m-laptop", false)]);
    assert!(peers.iter().all(|p| p.peer.trusted && p.dns_name.ends_with(".tail1234.ts.net")));

    let theirs = w.peer_wadspaces("m-desk").await.unwrap();
    assert!(theirs.iter().all(|x| x.machine == "m-desk"));
    let e = w.peer_wadspaces("m-laptop").await.unwrap_err();
    assert_eq!((e.status, e.code.as_str()), (503, "offline"));
    assert_eq!(w.peer_wadspaces("nope").await.unwrap_err().status, 404);
    assert_eq!(w.copy_from_peer("m-laptop", "notes").await.unwrap_err().status, 503);
    assert_eq!(w.copy_from_peer("m-desk", "nope").await.unwrap_err().status, 404);

    // Copied: this machine's, never built here, its folder still Desk's.
    let job = w.copy_from_peer("m-desk", "pixel-art").await.unwrap();
    assert_eq!(job.job.kind, JobKind::CopyWadspace);
    let done = ended(w, &job).await;
    assert_eq!(done.job.status, JobStatus::Done, "{:?}", done.job.error);
    let id = done.job.params["created"].as_str().expect("the copy's id").to_string();
    let x: wad_model::Wadspace = w.get(&format!("/v1/wadspaces/{id}")).await.unwrap();
    assert_eq!((x.name.as_str(), x.machine.as_str()), ("Pixel Art", "m-test"));
    assert!(x.look.projects.iter().all(|p| matches!(&p.kind,
        wad_model::ProjectKind::Mount { source, .. } if source.machine_id == "m-desk")));
    let b = w.builds().await.unwrap().into_iter().find(|b| b.id == id).expect("its build state");
    assert_eq!(b.state.built, Built::Never);
}

#[tokio::test]
async fn github_its_repos_a_sign_out_and_a_sign_in() {
    let up = sample().await;
    let w = &up.w;
    let a = w.github().await.unwrap().expect("signed in");
    assert_eq!(a.login, "hunggod");
    assert!(["repo", "workflow", "read:org"].iter().all(|s| a.has_scope(s)));
    let repos = w.github_repos().await.unwrap();
    assert_eq!(repos.len(), 3);
    assert!(repos.iter().all(|r| r.full_name.starts_with("HungGod/")));

    w.github_signout().await.unwrap();
    assert_eq!(w.github().await.unwrap(), None);
    assert_eq!(w.github_repos().await.unwrap_err().status, 401);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let follow = w.follow(move |i| tx.send(i).is_ok());
    let job = w.github_signin(&SignInRequest { scopes: vec!["write:packages".into()] }).await.unwrap();
    assert_eq!(job.job.kind, JobKind::Github);
    assert_eq!(ended(w, &job).await.job.status, JobStatus::Done);
    let a = w.github().await.unwrap().expect("signed in again");
    assert!(a.has_scope("write:packages") && a.has_scope("repo"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    follow.abort();
    let events = heard(&mut rx);
    assert!(events.iter().any(|e| matches!(e,
        Event::SignIn { provider: SignInProvider::Github, code: Some(c), .. } if !c.is_empty())));
    assert!(events.iter().any(|e| matches!(e, Event::SignInDone { provider: SignInProvider::Github })));
    assert!(events.iter().any(|e| matches!(e, Event::Github { account: Some(_) })));
    // A sign-in without a body (curl's) is the first one.
    w.act(Method::POST, "/v1/github/signin", None::<&()>).await.unwrap();
}

#[tokio::test]
async fn a_new_watcher_gets_the_tailnet_peers_and_github_first() {
    let up = sample().await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let follow = up.w.follow(move |i| tx.send(i).is_ok());
    let mut kinds = vec![];
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(i) = rx.recv().await {
            match i {
                Incoming::Event(Event::Library(_)) => break,
                Incoming::Event(Event::Tailnet(_)) => kinds.push("tailnet"),
                Incoming::Event(Event::Peers { .. }) => kinds.push("peers"),
                Incoming::Event(Event::Github { .. }) => kinds.push("github"),
                _ => {}
            }
        }
    })
    .await
    .expect("the present");
    follow.abort();
    assert_eq!(kinds, ["tailnet", "peers", "github"]);
}

#[tokio::test]
async fn the_machines_answer_501_until_theyre_made() {
    let up = up(None).await;
    let w = &up.w;
    let codes = [
        w.tailnet().await.map(drop),
        w.tailnet_login().await.map(drop),
        w.tailnet_logout().await.map(drop),
        w.trust_device("n1").await.map(drop),
        w.untrust_device("n1").await.map(drop),
        w.deny_ask("n1").await.map(drop),
        w.later_ask("n1").await.map(drop),
        w.peers().await.map(drop),
        w.peer_wadspaces("m-desk").await.map(drop),
        w.copy_from_peer("m-desk", "x").await.map(drop),
        w.github().await.map(drop),
        w.github_signin(&SignInRequest::default()).await.map(drop),
        w.github_signout().await,
        w.github_repos().await.map(drop),
    ];
    for c in codes {
        let e = c.unwrap_err();
        assert_eq!((e.status, e.code.as_str()), (501, "not_implemented"), "{}", e.message);
        assert!(e.message.ends_with("comes in phase 6"), "{}", e.message);
    }
}
