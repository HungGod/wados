//! The machine's network and Bluetooth (network.rs, bluetooth.rs) through
//! wadd's socket, against a fake NetworkManager and a fake BlueZ on a
//! private D-Bus (never the machine's), and the exit node on a fake
//! tailnet (wad-tailscale's fake tailscaled and CLI).

use std::sync::Arc;
use std::time::Duration;

use wad_bt::fake::{Asks, FakeBluez, FakeDevice};
use wad_config::{Config, Profile};
use wad_net::fake::{FakeAp, FakeNm, PrivateBus, saved_profile};
use wad_proto::{BtAnswer, BtAsk, Event, Level};
use wad_tailscale::fake::{FakeCli, FakeNode, FakeTailnet, fake_cli};
use wad_wadd_client::{Incoming, Wadd};
use wadd::fake::{Fake, FakeHands};

const BUDS: &str = "AA:BB:CC:00:00:01";
const SPEAKER: &str = "AA:BB:CC:00:00:03";

struct Up {
    dir: tempfile::TempDir,
    w: Wadd,
    nm: wad_net::fake::W,
    bz: wad_bt::fake::W,
    tailnet: FakeTailnet,
    cli: FakeCli,
    _bus: PrivateBus,
    _fakes: (FakeNm, FakeBluez),
    _tailscaled: tokio::task::JoinHandle<()>,
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

async fn up() -> Up {
    let bus = PrivateBus::start().await;
    let nm = wad_net::fake::World::with(
        vec![
            FakeAp::secure("Home", 82),
            FakeAp::secure("Neighbours", 41),
            FakeAp::open("Vanua Cafe", 58),
            FakeAp::enterprise("Uni", 30),
        ],
        &[("Home", "correct horse"), ("Neighbours", "next door 22")],
    );
    saved_profile(&nm, "Home", true);
    let fake_nm = FakeNm::serve(&bus, nm.clone()).await;
    let bz = wad_bt::fake::World::with(
        "",
        vec![
            FakeDevice::new(BUDS, "Pixel Buds Pro", "audio-headphones").paired(true).battery(80),
            FakeDevice::new(SPEAKER, "JBL Flip 6", "audio-card").asks(Asks::Confirm(492817)),
        ],
    );
    let fake_bz = FakeBluez::serve(&bus.address, bz.clone()).await;

    let dir = tempfile::tempdir().unwrap();
    let net = FakeTailnet::new("owner@example.com", "ts.test");
    net.user(1, "owner@example.com", "Owner");
    net.node(FakeNode::new("nDesk", "desk", 1, "127.0.0.1"))
        .node(FakeNode::new("nLaptop", "laptop", 1, "127.0.0.2").exit_node_option())
        .node(FakeNode::new("nVps", "vps", 1, "127.0.0.4").tagged("tag:server").exit_node_option())
        .node(FakeNode::new("nNas", "nas", 1, "127.0.0.6").exit_node_option().offline());
    let ts = dir.path().join("tailscaled.sock");
    let tailscaled = net.serve("nDesk", &ts).unwrap();
    let cli = fake_cli(&dir.path().join("cli"));

    let mut config = Config::defaults(Profile::User);
    config.machine.id = "m-desk".into();
    config.machine.name = "Desk".into();
    config.daemon.state_dir = dir.path().join("state");
    config.daemon.signins_dir = dir.path().join("state/keys");
    config.daemon.screen_dir = dir.path().join("screen");
    config.display.runtime_dir = dir.path().join("rt");
    config.packages.pull = false;
    config.streaming.listen = vec![];
    config.streaming.secure_runtime = String::new();
    std::fs::create_dir_all(&config.display.runtime_dir).unwrap();
    config.radios.enabled = true;
    config.radios.bus = bus.address.clone();
    config.radios.poll_s = 1;
    let t = &mut config.tailnet;
    t.enabled = true;
    t.socket = ts;
    t.cli = cli.bin.clone();
    t.runuser = "/nonexistent/runuser".into();
    t.port = free_port();
    t.bind_device = String::new();
    t.poll_s = 1;
    t.drive = false;
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
        wadd::logbuf::LogBuffer::new(100),
        parts,
    )
    .unwrap();
    tokio::spawn(server.serve_until(std::future::pending()));
    Up {
        dir,
        w: Wadd::new(sock),
        nm,
        bz,
        tailnet: net,
        cli,
        _bus: bus,
        _fakes: (fake_nm, fake_bz),
        _tailscaled: tailscaled,
    }
}

async fn until<T>(what: &str, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    for _ in 0..200 {
        if let Some(t) = f().await {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{what} didn't happen");
}

/// wadd's events from now.
fn follow(w: &Wadd) -> (tokio::task::JoinHandle<()>, tokio::sync::mpsc::UnboundedReceiver<Incoming>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    (w.follow(move |i| tx.send(i).is_ok()), rx)
}

async fn next<T>(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Incoming>, f: impl Fn(Event) -> Option<T>) -> T {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
            Ok(Some(Incoming::Event(e))) => {
                if let Some(t) = f(e) {
                    return t;
                }
            }
            Ok(Some(_)) => {}
            other => panic!("no such event: {other:?}"),
        }
    }
}

#[tokio::test]
async fn wifi_joining_and_forgetting() {
    let up = up().await;
    let w = &up.w;
    let n = w.network().await.unwrap();
    assert!(n.wifi_on && !n.airplane && n.connected.is_none());
    let names: Vec<&str> = n.networks.iter().map(|x| x.ssid.as_str()).collect();
    assert_eq!(names, ["Home", "Vanua Cafe", "Neighbours", "Uni"]);
    assert!(n.networks[0].saved && n.networks[0].secure);

    // The saved one: no password asked.
    let n = w.wifi_connect("Home", None).await.unwrap();
    assert_eq!(n.connected.as_deref(), Some("Home"));
    assert!(n.networks[0].active);
    assert_eq!((n.ip.as_deref(), n.dns.as_slice()), (Some("192.168.1.42"), &["192.168.1.1".to_string()][..]));

    // A new one: a wrong password is 401, and leaves nothing saved.
    let e = w.wifi_connect("Neighbours", Some("not the one")).await.unwrap_err();
    assert_eq!((e.status, e.message.as_str()), (401, "That isn't the password for Neighbours"));
    assert_eq!(w.wifi_connect("Neighbours", None).await.unwrap_err().status, 400);
    assert_eq!(w.wifi_connect("Neighbours", Some("short")).await.unwrap_err().status, 400);
    assert_eq!(w.wifi_connect("Uni", Some("whatever1")).await.unwrap_err().status, 400);
    assert_eq!(w.wifi_connect("Elsewhere", None).await.unwrap_err().status, 404);
    let n = w.wifi_connect("Neighbours", Some("next door 22")).await.unwrap();
    assert_eq!(n.connected.as_deref(), Some("Neighbours"));
    assert!(n.networks.iter().any(|x| x.ssid == "Neighbours" && x.saved && x.active));
    {
        // The password went to NetworkManager over D-Bus, and is kept with the profile.
        let world = up.nm.lock().unwrap();
        let sent = world.added.last().unwrap();
        let psk = wad_net::fake::get_str(sent, "802-11-wireless-security", "psk");
        assert_eq!(psk.as_deref(), Some("next door 22"));
    }

    let n = w.wifi_disconnect().await.unwrap();
    assert!(n.connected.is_none() && n.ip.is_none());
    let n = w.wifi_forget("Neighbours").await.unwrap();
    assert!(n.networks.iter().any(|x| x.ssid == "Neighbours" && !x.saved));
    assert_eq!(w.wifi_forget("Neighbours").await.unwrap_err().status, 404);

    // Off: nothing joins; the cable's what it's on.
    let (_f, mut rx) = follow(w);
    let n = w.set_wifi(false).await.unwrap();
    assert!(!n.wifi_on);
    assert_eq!(w.wifi_connect("Home", None).await.unwrap_err().status, 409);
    up.nm.lock().unwrap().wired = Some((true, 1000));
    let n = next(&mut rx, |e| match e {
        Event::Network(n) if n.ethernet.plugged => Some(n),
        _ => None,
    })
    .await;
    assert_eq!((n.ethernet.speed_mbps, n.ip.as_deref()), (Some(1000), Some("192.168.1.24")));
}

#[tokio::test]
async fn airplane_mode_turns_every_radio_off_and_back() {
    let up = up().await;
    let w = &up.w;
    assert!(w.bluetooth().await.unwrap().on);
    let n = w.set_airplane(true).await.unwrap();
    assert!(n.airplane && !n.wifi_on);
    assert!(!w.bluetooth().await.unwrap().on);
    assert!(!up.bz.lock().unwrap().powered);
    assert_eq!(w.set_wifi(true).await.unwrap_err().status, 409);
    assert_eq!(w.set_bluetooth(true).await.unwrap_err().status, 409);
    assert_eq!(w.wifi_connect("Home", None).await.unwrap_err().status, 409);
    // It's kept: wadd's state has it.
    assert!(up.dir.path().join("state/airplane.json").exists());
    let n = w.set_airplane(false).await.unwrap();
    assert!(!n.airplane && n.wifi_on);
    assert!(w.bluetooth().await.unwrap().on, "Bluetooth was on before: it's on again");
}

#[tokio::test]
async fn bluetooth_pairing_asks_and_is_answered() {
    let up = up().await;
    let w = &up.w;
    let b = w.bluetooth().await.unwrap();
    assert_eq!(b.discoverable_name, "desk");
    let names: Vec<&str> = b.devices.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["Pixel Buds Pro", "JBL Flip 6"]);
    assert_eq!(b.devices[0].battery, Some(80));
    let b = w.bt_scan(true).await.unwrap();
    assert!(b.scanning && up.bz.lock().unwrap().discoverable);

    let (_f, mut rx) = follow(w);
    w.bt_pair(SPEAKER).await.unwrap();
    let p = next(&mut rx, |e| match e {
        Event::BtPrompt { prompt: Some(p) } => Some(p),
        _ => None,
    })
    .await;
    assert_eq!((p.device.as_str(), p.name.as_str()), (SPEAKER, "JBL Flip 6"));
    assert_eq!(p.kind, BtAsk::Confirm { code: "492817".into() });
    assert_eq!(w.bt_answer("bt-nope", &BtAnswer { accept: true, pin: None }).await.unwrap_err().status, 404);
    w.bt_answer(&p.id, &BtAnswer { accept: true, pin: None }).await.unwrap();
    let b = next(&mut rx, |e| match e {
        Event::Bluetooth(b) if b.devices.iter().any(|d| d.address == SPEAKER && d.paired && d.connected) => Some(b),
        _ => None,
    })
    .await;
    assert!(b.devices[0].connected);

    // Turned down: a notice says so.
    w.bt_forget(SPEAKER).await.unwrap();
    w.bt_pair(SPEAKER).await.unwrap();
    let p = next(&mut rx, |e| match e {
        Event::BtPrompt { prompt: Some(p) } => Some(p),
        _ => None,
    })
    .await;
    w.bt_answer(&p.id, &BtAnswer { accept: false, pin: None }).await.unwrap();
    let (level, title) = next(&mut rx, |e| match e {
        Event::Notice { level, title, .. } => Some((level, title)),
        _ => None,
    })
    .await;
    assert_eq!((level, title.as_str()), (Level::Error, "Couldn't pair JBL Flip 6"));

    assert_eq!(w.bt_connect("AA:BB:CC:99:99:99").await.unwrap_err().status, 404);
    assert_eq!(w.bt_connect("nope").await.unwrap_err().status, 400);
    let b = w.bt_disconnect(BUDS).await.unwrap();
    assert!(!b.devices.iter().any(|d| d.connected));
    w.bt_connect(BUDS).await.unwrap();
    let b = w.set_bluetooth(false).await.unwrap();
    assert!(!b.on && b.devices.is_empty());
    assert_eq!(w.bt_connect(BUDS).await.unwrap_err().status, 409);
}

#[tokio::test]
async fn the_exit_node_is_set_with_tailscale() {
    let up = up().await;
    let w = &up.w;
    let n =
        until("the tailnet's exit nodes", async || w.network().await.ok().filter(|n| !n.exit_nodes.is_empty())).await;
    let mut ids: Vec<&str> = n.exit_nodes.iter().map(|c| c.id.as_str()).collect();
    ids.sort();
    // Those that offer, online (the NAS is offline; the desk is this machine).
    assert_eq!(ids, ["nLaptop", "nVps"]);
    assert_eq!(n.exit_node, None);
    assert_eq!(w.set_exit_node(Some("nNas")).await.unwrap_err().status, 404);
    assert_eq!(w.set_exit_node(Some("nPhone")).await.unwrap_err().status, 404);
    w.set_exit_node(Some("nLaptop")).await.unwrap();
    let ran = up.cli.runs();
    assert!(ran.iter().any(|r| r.ends_with("set --exit-node=127.0.0.2 --exit-node-allow-lan-access=true")), "{ran:?}");
    // Tailscale took it.
    up.tailnet.set_exit_node(Some("nLaptop"));
    until("the exit node in use", async || {
        w.network().await.ok().filter(|n| n.exit_node.as_deref() == Some("nLaptop"))
    })
    .await;
    w.set_exit_node(None).await.unwrap();
    assert!(up.cli.runs().iter().any(|r| r.ends_with("set --exit-node=")));
}
