//! The tailnet, peers and the remote API on a fake tailnet (wad-tailscale's
//! fake tailscaled and CLI), never the real one: whole wadds on 127.0.0.1 and
//! 127.0.0.2 (my two machines, the same login), each with its own fake
//! tailscaled over the one tailnet, and devices calling from other loopback
//! addresses (a phone at 127.0.0.3, someone else's laptop at 127.0.0.5).

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use wad_config::{Config, Profile};
use wad_model::{Built, JobKind, JobStatus};
use wad_proto::{DriveState, Event, JobInfo, SignInProvider, TailnetState};
use wad_tailscale::fake::{FAKE_AUTH_URL, FakeCli, FakeNode, FakeTailnet, fake_cli};
use wad_wadd_client::{Incoming, Wadd};
use wadd::fake::{Fake, FakeHands};

const OWNER: i64 = 1;
const FRIEND: i64 = 2;

/// My two machines, a phone, a friend's laptop shared in.
fn tailnet() -> FakeTailnet {
    let net = FakeTailnet::new("owner@example.com", "ts.test");
    net.user(OWNER, "owner@example.com", "Owner").user(FRIEND, "friend@example.com", "Friend");
    net.node(FakeNode::new("nDesk", "desk", OWNER, "127.0.0.1"))
        .node(FakeNode::new("nLaptop", "laptop", OWNER, "127.0.0.2"))
        .node(FakeNode::new("nPhone", "pixel-9", OWNER, "127.0.0.3").os("android", "Google Pixel 9"))
        .node(
            FakeNode::new("nFriend", "friends-laptop", FRIEND, "127.0.0.5")
                .os("macOS", "MacBookAir10,1")
                .shared_by(FRIEND),
        );
    net
}

/// A port free on both machines' addresses.
fn free_port() -> u16 {
    loop {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        if std::net::TcpListener::bind(("127.0.0.2", port)).is_ok() {
            return port;
        }
    }
}

struct Machine {
    dir: tempfile::TempDir,
    w: Wadd,
    cli: FakeCli,
    ip: &'static str,
    _tailscaled: tokio::task::JoinHandle<()>,
}

async fn machine(net: &FakeTailnet, node: &str, id: &str, ip: &'static str, port: u16) -> Machine {
    machine_with(net, node, id, ip, port, false).await
}

/// `refuse_drive`: its tailnet's policy doesn't let it share.
async fn machine_with(
    net: &FakeTailnet,
    node: &str,
    id: &str,
    ip: &'static str,
    port: u16,
    refuse_drive: bool,
) -> Machine {
    let dir = tempfile::tempdir().unwrap();
    let ts = dir.path().join("tailscaled.sock");
    let tailscaled = net.serve(node, &ts).unwrap();
    let cli = fake_cli(&dir.path().join("cli"));
    cli.refuse_drive(refuse_drive);
    let mut config = Config::defaults(Profile::User);
    config.machine.id = id.into();
    config.machine.name = id.trim_start_matches("m-").to_uppercase();
    config.daemon.state_dir = dir.path().join("state");
    config.daemon.signins_dir = dir.path().join("state/keys");
    config.daemon.screen_dir = dir.path().join("screen");
    config.display.runtime_dir = dir.path().join("rt");
    config.packages.pull = false;
    std::fs::create_dir_all(&config.display.runtime_dir).unwrap();
    let t = &mut config.tailnet;
    t.enabled = true;
    t.socket = ts;
    t.cli = cli.bin.clone();
    t.runuser = "/nonexistent/runuser".into();
    t.port = port;
    t.bind_device = String::new();
    t.poll_s = 1;
    let parts = wadd::Parts {
        backend: Arc::new(Fake::new()),
        hands: Arc::new(FakeHands::default()),
        poll: Duration::from_millis(20),
        sample: None,
    };
    let sock = dir.path().join("wadd.sock");
    let server = wadd::Server::with_parts(
        Profile::User,
        config,
        wadd::Listen::Path(sock.clone()),
        wadd::logbuf::LogBuffer::new(200),
        parts,
    )
    .unwrap();
    tokio::spawn(server.serve_until(std::future::pending()));
    Machine { dir, w: Wadd::new(sock), cli, ip, _tailscaled: tailscaled }
}

/// A device on the tailnet calling from `ip`.
struct Device(reqwest::Client);

impl Device {
    fn at(ip: &str) -> Self {
        let ip: IpAddr = ip.parse().unwrap();
        Self(reqwest::Client::builder().no_proxy().local_address(ip).build().unwrap())
    }

    /// `path` on `m`'s remote API: the status and the answer.
    async fn get(&self, m: &Machine, port: u16, path: &str) -> (u16, Value) {
        let to = SocketAddr::new(m.ip.parse().unwrap(), port);
        let res = self.0.get(format!("http://{to}{path}")).timeout(Duration::from_secs(5)).send().await.unwrap();
        let status = res.status().as_u16();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    /// `path` on `m`'s remote API with `method` and a JSON body: the status and the answer.
    async fn send(&self, m: &Machine, port: u16, method: reqwest::Method, path: &str, body: &Value) -> (u16, Value) {
        let to = SocketAddr::new(m.ip.parse().unwrap(), port);
        let req = self.0.request(method, format!("http://{to}{path}")).json(body);
        let res = req.timeout(Duration::from_secs(5)).send().await.unwrap();
        let status = res.status().as_u16();
        (status, res.json().await.unwrap_or(Value::Null))
    }
}

async fn until<T>(what: &str, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    for _ in 0..600 {
        if let Some(t) = f().await {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{what} didn't happen");
}

/// Waits for `m` to see machine `id` of mine online.
async fn sees(m: &Machine, id: &str) {
    until(&format!("{id} online"), async || {
        m.w.peers().await.unwrap().into_iter().any(|p| p.peer.id == id && p.peer.online).then_some(())
    })
    .await;
}

async fn ended(w: &Wadd, job: &JobInfo) -> JobInfo {
    until(&format!("{} ending", job.job.id), async || {
        let j = w.job(&job.job.id, 0).await.unwrap().job;
        wadd::jobs::finished(j.job.status).then_some(j)
    })
    .await
}

#[tokio::test]
async fn my_machines_find_each_other_and_trust_follows_an_answer() {
    let net = tailnet();
    let port = free_port();
    let desk = machine(&net, "nDesk", "m-desk", "127.0.0.1", port).await;
    let laptop = machine(&net, "nLaptop", "m-laptop", "127.0.0.2", port).await;
    sees(&desk, "m-laptop").await;
    sees(&laptop, "m-desk").await;
    let p = desk.w.peers().await.unwrap().into_iter().find(|p| p.peer.id == "m-laptop").unwrap();
    assert_eq!(
        (p.node_id.as_str(), p.peer.hostname.as_str(), p.dns_name.as_str()),
        ("nLaptop", "laptop", "laptop.ts.test")
    );
    assert!(p.peer.trusted);
    let t = desk.w.tailnet().await.unwrap();
    assert_eq!(
        (t.state, t.tailnet.as_str(), t.hostname.as_str(), t.ip.as_str()),
        (TailnetState::Running, "owner@example.com", "desk", "127.0.0.1")
    );
    assert!(t.devices.iter().any(|d| d.id == "nLaptop" && d.mine && d.trusted));
    assert!(t.asks.is_empty(), "my machines never ask: {:?}", t.asks);

    // The phone isn't trusted: hello only, and it asks.
    let phone = Device::at("127.0.0.3");
    let (status, hello) = phone.get(&desk, port, "/v1/remote/hello").await;
    assert_eq!((status, hello["wadd"].as_bool(), hello["trust"].as_str()), (200, Some(true), Some("asked")));
    assert_eq!(hello["machine"]["id"], "m-desk");
    let (status, e) = phone.get(&desk, port, "/v1/remote/wadspaces").await;
    assert_eq!((status, e["code"].as_str()), (403, Some("forbidden")));
    assert_eq!(phone.get(&desk, port, "/v1/remote/machine").await.0, 403);
    let ask = until("the phone's ask", async || desk.w.tailnet().await.unwrap().asks.into_iter().next()).await;
    assert_eq!((ask.id.as_str(), ask.device.as_str(), ask.ip.as_str()), ("nPhone", "pixel-9", "127.0.0.3"));
    assert!(laptop.w.tailnet().await.unwrap().asks.is_empty(), "only where it called");

    // Trusted on the desk: it may list the desk's wadspaces, and the laptop
    // follows the answer.
    let t = desk.w.trust_device("nPhone").await.unwrap();
    assert!(t.asks.is_empty() && t.devices.iter().any(|d| d.id == "nPhone" && d.trusted && !d.mine));
    let (status, list) = phone.get(&desk, port, "/v1/remote/wadspaces").await;
    assert_eq!(status, 200);
    assert!(list.as_array().unwrap().iter().any(|w| w["name"] == "WadOS"));
    let (status, m) = phone.get(&desk, port, "/v1/remote/machine").await;
    assert_eq!((status, m["id"].as_str()), (200, Some("m-desk")));
    until("the laptop trusting the phone", async || {
        (phone.get(&laptop, port, "/v1/remote/wadspaces").await.0 == 200).then_some(())
    })
    .await;
    assert!(laptop.w.tailnet().await.unwrap().devices.iter().any(|d| d.id == "nPhone" && d.trusted));
    // A trusted device isn't one of my machines: the trust book is theirs alone.
    assert_eq!(phone.get(&desk, port, "/v1/remote/trust").await.0, 403);

    // Untrusted on the laptop: neither lets it in now, and it asks again.
    laptop.w.untrust_device("nPhone").await.unwrap();
    until("the desk following the laptop", async || {
        (phone.get(&desk, port, "/v1/remote/wadspaces").await.0 == 403).then_some(())
    })
    .await;
    let book: wad_proto::TrustBook =
        serde_json::from_slice(&std::fs::read(desk.dir.path().join("state/tailnet-devices.json")).unwrap()).unwrap();
    assert_eq!(
        (book.user.as_str(), book.devices[0].answer, book.devices[0].by.as_str()),
        ("owner@example.com", wad_proto::Answer::Untrusted, "m-laptop")
    );
    until("the phone asking the desk again", async || {
        desk.w.tailnet().await.unwrap().asks.iter().any(|a| a.id == "nPhone").then_some(())
    })
    .await;
}

#[tokio::test]
async fn other_devices_get_hello_only_and_a_denied_one_stays_quiet() {
    let net = tailnet();
    let port = free_port();
    let desk = machine(&net, "nDesk", "m-desk", "127.0.0.1", port).await;
    until("running", async || (desk.w.tailnet().await.unwrap().state == TailnetState::Running).then_some(())).await;
    // Someone else's laptop, shared in: an ask at once.
    let friend = Device::at("127.0.0.5");
    let (status, hello) = friend.get(&desk, port, "/v1/remote/hello").await;
    assert_eq!((status, hello["trust"].as_str()), (200, Some("asked")));
    let t = desk.w.tailnet().await.unwrap();
    let ask = t.asks.iter().find(|a| a.id == "nFriend").expect("its ask");
    assert_eq!(ask.kind, wad_proto::DeviceKind::Laptop);
    desk.w.deny_ask("nFriend").await.unwrap();
    let (status, hello) = friend.get(&desk, port, "/v1/remote/hello").await;
    assert_eq!((status, hello["trust"].as_str()), (200, Some("denied")));
    let (status, e) = friend.get(&desk, port, "/v1/remote/wadspaces/x/open").await;
    assert_eq!(status, 403, "{e}");
    assert!(desk.w.tailnet().await.unwrap().asks.is_empty(), "denied: no new ask");
    assert!(desk.w.tailnet().await.unwrap().devices.iter().any(|d| d.id == "nFriend" && !d.trusted));
    // A tagged node (Headscale's: still the owner's user id) is another device, wadd or not.
    net.node(FakeNode::new("nCi", "ci", OWNER, "127.0.0.4").tagged("tag:ci"));
    let (status, hello) = Device::at("127.0.0.4").get(&desk, port, "/v1/remote/hello").await;
    assert_eq!((status, hello["trust"].as_str()), (200, Some("asked")));
    assert!(desk.w.tailnet().await.unwrap().asks.iter().any(|a| a.id == "nCi"));
    // Later: hidden for the session.
    let t = desk.w.later_ask("nCi").await.unwrap();
    assert!(!t.asks.iter().any(|a| a.id == "nCi"));
    // Not on the tailnet: refused outright.
    let (status, _) = Device::at("127.0.0.9").get(&desk, port, "/v1/remote/hello").await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn copy_here_imports_a_peers_wadspace() {
    let net = tailnet();
    let port = free_port();
    let desk = machine(&net, "nDesk", "m-desk", "127.0.0.1", port).await;
    let laptop = machine(&net, "nLaptop", "m-laptop", "127.0.0.2", port).await;
    let op = serde_json::from_value(serde_json::json!({"op": "space.create", "name": "Studio"})).unwrap();
    let studio = laptop.w.op(op).await.unwrap().created.unwrap();
    sees(&desk, "m-laptop").await;

    let theirs = desk.w.peer_wadspaces("m-laptop").await.unwrap();
    assert!(theirs.iter().any(|w| w.id == studio && w.machine == "m-laptop"));
    assert_eq!(desk.w.peer_wadspaces("m-nope").await.unwrap_err().status, 404);
    assert_eq!(desk.w.copy_from_peer("m-laptop", "nope").await.unwrap_err().status, 404);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let follow = desk.w.follow(move |i| tx.send(i).is_ok());
    let job = desk.w.copy_from_peer("m-laptop", &studio).await.unwrap();
    assert_eq!(job.job.kind, JobKind::CopyWadspace);
    let done = ended(&desk.w, &job).await;
    assert_eq!(done.job.status, JobStatus::Done, "{:?}", done.job.error);
    let id = done.job.params["created"].as_str().expect("the copy's id").to_string();
    let x: wad_model::Wadspace = desk.w.get(&format!("/v1/wadspaces/{id}")).await.unwrap();
    assert_eq!((x.name.as_str(), x.machine.as_str()), ("Studio", "m-desk"));
    let b = desk.w.builds().await.unwrap().into_iter().find(|b| b.id == id).expect("its build state");
    assert_eq!(b.state.built, Built::Never);
    // The library heard it.
    tokio::time::sleep(Duration::from_millis(100)).await;
    follow.abort();
    let mut heard = false;
    while let Ok(i) = rx.try_recv() {
        heard |= matches!(i, Incoming::Event(Event::Wadspace(w)) if w.id == id);
    }
    assert!(heard);

    // The laptop goes offline: kept, and its wadspaces can't be had.
    net.set_online("nLaptop", false);
    until("the laptop offline", async || {
        desk.w.peers().await.unwrap().into_iter().any(|p| p.peer.id == "m-laptop" && !p.peer.online).then_some(())
    })
    .await;
    let e = desk.w.peer_wadspaces("m-laptop").await.unwrap_err();
    assert_eq!((e.status, e.code.as_str()), (503, "offline"));
    assert_eq!(desk.w.copy_from_peer("m-laptop", &studio).await.unwrap_err().status, 503);
}

#[tokio::test]
async fn a_machine_coming_online_swaps_trust_books() {
    let net = tailnet();
    net.set_online("nLaptop", false);
    let port = free_port();
    let desk = machine(&net, "nDesk", "m-desk", "127.0.0.1", port).await;
    let phone = Device::at("127.0.0.3");
    until("running", async || (desk.w.tailnet().await.unwrap().state == TailnetState::Running).then_some(())).await;
    phone.get(&desk, port, "/v1/remote/hello").await;
    until("the phone's ask", async || {
        desk.w.tailnet().await.unwrap().asks.iter().any(|a| a.id == "nPhone").then_some(())
    })
    .await;
    // Trusted while the laptop is away.
    desk.w.trust_device("nPhone").await.unwrap();
    let laptop = machine(&net, "nLaptop", "m-laptop", "127.0.0.2", port).await;
    net.set_online("nLaptop", true);
    sees(&desk, "m-laptop").await;
    until("the laptop catching up", async || {
        (phone.get(&laptop, port, "/v1/remote/wadspaces").await.0 == 200).then_some(())
    })
    .await;
    let t = laptop.w.tailnet().await.unwrap();
    assert!(t.devices.iter().any(|d| d.id == "nPhone" && d.trusted), "{:?}", t.devices);
}

#[tokio::test]
async fn a_login_signs_in_in_the_browser_and_can_be_cancelled() {
    let net = tailnet();
    net.set_state("nDesk", "NeedsLogin", "");
    let port = free_port();
    let desk = machine(&net, "nDesk", "m-desk", "127.0.0.1", port).await;
    until("needing a login", async || {
        (desk.w.tailnet().await.unwrap().state == TailnetState::NeedsLogin).then_some(())
    })
    .await;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let follow = desk.w.follow(move |i| tx.send(i).is_ok());
    let job = desk.w.tailnet_login().await.unwrap();
    assert_eq!(job.job.kind, JobKind::Tailnet);
    assert_eq!(desk.w.tailnet_login().await.unwrap().job.id, job.job.id, "one at a time");
    let t = until("the login link", async || {
        let t = desk.w.tailnet().await.unwrap();
        (t.login_url.is_some()).then_some(t)
    })
    .await;
    assert_eq!((t.state, t.login_url.as_deref()), (TailnetState::LoggingIn, Some(FAKE_AUTH_URL)));
    let held = desk.w.job(&job.job.id, 0).await.unwrap().job;
    assert!(held.job.stages.iter().any(|s| s.id == "signin" && s.held), "{:?}", held.job.stages);
    // Signed in: tailscaled's running, and the job's done.
    net.set_state("nDesk", "Running", "");
    desk.cli.approve();
    let done = ended(&desk.w, &job).await;
    assert_eq!(done.job.status, JobStatus::Done, "{:?}", done.job.error);
    assert_eq!(desk.w.tailnet().await.unwrap().state, TailnetState::Running);
    tokio::time::sleep(Duration::from_millis(100)).await;
    follow.abort();
    let mut events = vec![];
    while let Ok(i) = rx.try_recv() {
        if let Incoming::Event(e) = i {
            events.push(e);
        }
    }
    let signin = events.iter().position(
        |e| matches!(e, Event::SignIn { provider: SignInProvider::Tailscale, url, .. } if url == FAKE_AUTH_URL),
    );
    let closed = events.iter().position(|e| matches!(e, Event::SignInDone { provider: SignInProvider::Tailscale }));
    assert!(signin.is_some() && signin < closed, "{events:?}");
    let up = desk.cli.runs().into_iter().find(|r| r.contains(" up ")).unwrap();
    assert!(
        up.contains("--json") && up.contains("--hostname=desk") && up.contains("--login-server=https://hs.test:8443"),
        "{up}"
    );
    assert_eq!(desk.w.tailnet_login().await.unwrap_err().status, 409, "it's running already");

    // Logged out, then a login cancelled: its `tailscale up` stops and its window closes.
    net.set_state("nDesk", "NeedsLogin", "");
    let t = desk.w.tailnet_logout().await.unwrap();
    assert_eq!(t.state, TailnetState::NeedsLogin);
    assert!(desk.cli.runs().iter().any(|r| r.ends_with(" logout")));
    assert!(desk.w.peers().await.unwrap().is_empty());
    std::fs::remove_file(desk.dir.path().join("cli/approved")).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let follow = desk.w.follow(move |i| tx.send(i).is_ok());
    let job = desk.w.tailnet_login().await.unwrap();
    until("the second login link", async || desk.w.tailnet().await.unwrap().login_url).await;
    let pid = desk.cli.up_pid().expect("its pid");
    let cancelled = desk.w.cancel_job(&job.job.id).await.unwrap();
    assert_eq!(cancelled.job.status, JobStatus::Cancelled);
    until("tailscale up stopped", async || {
        let st = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        (st.is_empty() || st.contains(") Z ")).then_some(())
    })
    .await;
    until("the window closed", async || {
        while let Ok(i) = rx.try_recv() {
            if matches!(i, Incoming::Event(Event::SignInDone { provider: SignInProvider::Tailscale })) {
                return Some(());
            }
        }
        None
    })
    .await;
    follow.abort();
    let t = desk.w.tailnet().await.unwrap();
    assert_eq!((t.state, t.login_url), (TailnetState::NeedsLogin, None));
}

#[tokio::test]
async fn the_home_is_shared_over_taildrive_as_the_person() {
    let net = tailnet();
    let port = free_port();
    let desk = machine(&net, "nDesk", "m-desk", "127.0.0.1", port).await;
    let t = until("shared", async || {
        let t = desk.w.tailnet().await.unwrap();
        (t.drive == DriveState::Shared).then_some(t)
    })
    .await;
    assert!(t.drive_error.is_none());
    let home = nix::unistd::User::from_uid(nix::unistd::getuid()).unwrap().unwrap().dir;
    // wadd runs as the person here: no runuser needed.
    assert!(desk.cli.runs().iter().any(|r| r.ends_with(&format!("drive share home {}", home.display()))));

    // A tailnet whose policy doesn't allow it.
    let laptop = machine_with(&tailnet(), "nLaptop", "m-laptop", "127.0.0.2", free_port(), true).await;
    let t = until("refused", async || {
        let t = laptop.w.tailnet().await.unwrap();
        (t.drive == DriveState::NotAllowed).then_some(t)
    })
    .await;
    assert!(t.drive_error.unwrap_or_default().contains("drive:share"));
}

#[tokio::test]
async fn streams_on_the_remote_api_are_for_my_machines_and_trusted_devices() {
    use reqwest::Method;
    let net = tailnet();
    let port = free_port();
    let desk = machine(&net, "nDesk", "m-desk", "127.0.0.1", port).await;
    let _laptop = machine(&net, "nLaptop", "m-laptop", "127.0.0.2", port).await;
    sees(&desk, "m-laptop").await;
    // The desk's starter: there, so the gate and then the streams answer.
    let path = "/v1/remote/wadspaces/wadspaces/stream";
    let body = serde_json::json!({"mode": "private"});

    // The phone isn't trusted: neither starting nor stopping a stream.
    let phone = Device::at("127.0.0.3");
    assert_eq!(phone.get(&desk, port, "/v1/remote/hello").await.1["trust"], "asked");
    for method in [Method::POST, Method::DELETE] {
        let (status, e) = phone.send(&desk, port, method.clone(), path, &body).await;
        assert_eq!((status, e["code"].as_str()), (403, Some("forbidden")), "{method}");
    }
    // My machine gets past the gate: streams aren't made yet on the machine.
    let laptop = Device::at("127.0.0.2");
    for method in [Method::POST, Method::DELETE] {
        let (status, e) = laptop.send(&desk, port, method.clone(), path, &body).await;
        assert_eq!((status, e["code"].as_str()), (501, Some("not_implemented")), "{method}: {e}");
    }
    // So does a trusted device.
    until("the phone's ask", async || desk.w.tailnet().await.unwrap().asks.into_iter().find(|a| a.id == "nPhone"))
        .await;
    desk.w.trust_device("nPhone").await.unwrap();
    let (status, e) = phone.send(&desk, port, Method::POST, path, &body).await;
    assert_eq!(status, 501, "{e}");
    // No such wadspace there.
    let (status, _) = laptop.send(&desk, port, Method::POST, "/v1/remote/wadspaces/nope/stream", &body).await;
    assert_eq!(status, 404);
}
