//! Phase 6's, 7's and 8's events, through JSON and back as the shell decodes them
//! (wad-wadd-client's `decode`: one `Event` from each SSE's data). Each is
//! tagged `kind` beside its own fields, so none of them may have a `kind`
//! of its own at the top (a tailnet's devices and a peer do, nested).

use serde_json::{Value, json};
use wad_model::Built;
use wad_model::{Peer, PeerKind, StreamMode, StreamStage, Transport};
use wad_proto::streams::{SECURE_NEEDS_VM, TURN_DOWN};
use wad_proto::{
    Bluetooth, BtAsk, BtDevice, BtKind, BtPrompt, DeviceKind, DriveState, Ethernet, Event, ExitNodeChoice,
    GithubAccount, Level, Network, NoticeAction, PeerInfo, SignInProvider, Storage, StreamInfo, StreamList,
    StreamsAvailability, Tailnet, TailnetAsk, TailnetDevice, TailnetState, UpdateProgress, Updates, Version,
    WadspaceStorage, WifiBand, WifiNetwork,
};

/// To JSON and back: the same event, tagged `kind`.
fn round_trip(e: &Event, kind: &str) -> Value {
    let v = serde_json::to_value(e).unwrap_or_else(|err| panic!("{kind} doesn't serialize: {err}"));
    assert_eq!(v["kind"], kind, "{v}");
    let text = serde_json::to_string(&v).unwrap();
    assert_eq!(&serde_json::from_str::<Event>(&text).unwrap_or_else(|err| panic!("{kind}: {err}: {text}")), e);
    v
}

fn peer(id: &str, online: bool) -> PeerInfo {
    PeerInfo {
        peer: Peer {
            id: id.into(),
            name: id.trim_start_matches("m-").into(),
            hostname: id.trim_start_matches("m-").into(),
            kind: PeerKind::Laptop,
            trusted: true,
            online,
            last_seen: 1,
            os: "WadOS 44".into(),
            gpu: Some("Intel Iris Xe".into()),
        },
        node_id: format!("n{id}"),
        dns_name: format!("{id}.tail1234.ts.net"),
    }
}

#[test]
fn the_tailnet() {
    let e = Event::Tailnet(Tailnet {
        state: TailnetState::Running,
        tailnet: "tail1234.ts.net".into(),
        hostname: "surface".into(),
        ip: "100.101.102.103".into(),
        login_url: None,
        devices: vec![TailnetDevice {
            id: "nDesk1".into(),
            name: "Desk".into(),
            os: "linux".into(),
            kind: DeviceKind::Desktop,
            online: true,
            trusted: true,
            mine: true,
        }],
        asks: vec![TailnetAsk {
            id: "nTab1".into(),
            device: "Galaxy Tab S9".into(),
            os: "android".into(),
            kind: DeviceKind::Tablet,
            ip: "100.88.20.7".into(),
            at: 3,
        }],
        drive: DriveState::Shared,
        drive_error: None,
    });
    let v = round_trip(&e, "tailnet");
    // Flat: its own fields beside the tag; the devices' kinds stay theirs.
    assert_eq!((v["state"].as_str(), v["tailnet"].as_str()), (Some("running"), Some("tail1234.ts.net")));
    assert_eq!(v["devices"][0]["kind"], "desktop");
    assert_eq!(e.latest_key().as_deref(), Some("tailnet"));
    // What a wadd says before it knows anything.
    round_trip(&Event::Tailnet(Tailnet::default()), "tailnet");
}

#[test]
fn the_peers() {
    let e = Event::Peers { peers: vec![peer("m-desk", true), peer("m-laptop", false)] };
    let v = round_trip(&e, "peers");
    assert_eq!((v["peers"][1]["kind"].as_str(), v["peers"][1]["online"].as_bool()), (Some("laptop"), Some(false)));
    assert_eq!(e.latest_key().as_deref(), Some("peers"));
    round_trip(&Event::Peers { peers: vec![] }, "peers");
}

#[test]
fn github_signed_in_and_out() {
    let e = Event::Github {
        account: Some(GithubAccount { login: "hunggod".into(), name: "Hung".into(), scopes: vec!["repo".into()] }),
    };
    let v = round_trip(&e, "github");
    assert_eq!(v["account"]["login"], "hunggod");
    assert_eq!(e.latest_key().as_deref(), Some("github"));
    let out = round_trip(&Event::Github { account: None }, "github");
    assert_eq!(out, json!({"kind": "github", "account": null}));
}

#[test]
fn a_sign_in_and_its_end() {
    let e = Event::SignIn {
        provider: SignInProvider::Github,
        url: "https://github.com/login/device".into(),
        code: Some("WDOS-1234".into()),
    };
    let v = round_trip(&e, "signIn");
    assert_eq!((v["provider"].as_str(), v["code"].as_str()), (Some("github"), Some("WDOS-1234")));
    let ts = Event::SignIn {
        provider: SignInProvider::Tailscale,
        url: "https://login.tailscale.com/a/x".into(),
        code: None,
    };
    let v = round_trip(&ts, "signIn");
    assert!(v.get("code").is_none(), "{v}");
    // A sign-in is the present until it's done: then a new watcher doesn't get it.
    assert_eq!(e.latest_key().as_deref(), Some("signIn:github"));
    let done = Event::SignInDone { provider: SignInProvider::Github };
    let v = round_trip(&done, "signInDone");
    assert_eq!(v, json!({"kind": "signInDone", "provider": "github"}));
    assert_eq!(done.latest_key(), None);
    assert_eq!(done.ends_key(), e.latest_key());
    assert_eq!(ts.ends_key(), None);
}

#[test]
fn the_streams() {
    let public = StreamInfo {
        id: "studio".into(),
        mode: StreamMode::Public,
        transport: Transport::Webrtc,
        stage: StreamStage::Live,
        live: true,
        viewers: 2,
        started: 1,
        last_active: 2,
        link: "https://surface.wadspaces.com/ws-0123/".into(),
        turn_down: true,
        warnings: vec![TURN_DOWN.into()],
        job: None,
        peer: None,
    };
    let from_desk = StreamInfo {
        id: "notes".into(),
        mode: StreamMode::Private,
        transport: Transport::Websocket,
        stage: StreamStage::Sidecar,
        live: false,
        viewers: 0,
        started: 3,
        last_active: 3,
        link: "https://desk.tail1234.ts.net:48300/ws-4567/".into(),
        turn_down: false,
        warnings: vec![],
        job: Some("job-1".into()),
        peer: Some("m-desk".into()),
    };
    let list = StreamList { streams: vec![public, from_desk], available: StreamsAvailability::of(true, true, false) };
    let e = Event::from(list.clone());
    let v = round_trip(&e, "streams");
    // Its streams' modes and stages, and each mode's availability, nested: no `kind` of theirs meets the tag.
    assert_eq!((v["streams"][0]["mode"].as_str(), v["streams"][0]["viewers"].as_u64()), (Some("public"), Some(2)));
    assert_eq!(
        (v["streams"][1]["stage"].as_str(), v["streams"][1]["peer"].as_str()),
        (Some("sidecar"), Some("m-desk"))
    );
    assert_eq!(v["available"]["public-secure"], json!({"available": false, "reason": SECURE_NEEDS_VM}));
    assert!(v["streams"].as_array().unwrap().iter().all(|s| s.get("kind").is_none() && s.get("password").is_none()));
    // The event is GET /v1/streams's answer beside the tag.
    let mut answer = v.clone();
    answer.as_object_mut().unwrap().remove("kind");
    assert_eq!(serde_json::from_value::<StreamList>(answer).unwrap(), list);
    assert_eq!(e.latest_key().as_deref(), Some("streams"));
    assert_eq!(e.ends_key(), None);
    // None streaming.
    round_trip(&Event::Streams { streams: vec![], available: StreamsAvailability::of(false, false, false) }, "streams");
}

#[test]
fn the_network() {
    let e = Event::Network(Box::new(Network {
        wifi_on: true,
        airplane: false,
        connected: Some("Home".into()),
        ethernet: Ethernet { plugged: true, speed_mbps: Some(1000), ip: Some("192.168.1.9".into()) },
        networks: vec![WifiNetwork {
            ssid: "Home".into(),
            signal: 82,
            secure: true,
            saved: true,
            active: true,
            band: Some(WifiBand::Ghz5),
            security: Some("WPA2 Personal".into()),
        }],
        ip: Some("192.168.1.42".into()),
        dns: vec!["192.168.1.1".into()],
        exit_node: Some("nDesk1".into()),
        exit_nodes: vec![ExitNodeChoice {
            id: "nDesk1".into(),
            name: "Desk".into(),
            os: "linux".into(),
            kind: DeviceKind::Desktop,
        }],
    }));
    let v = round_trip(&e, "network");
    assert_eq!((v["wifiOn"].as_bool(), v["networks"][0]["band"].as_str()), (Some(true), Some("5GHz")));
    assert_eq!(v["exitNodes"][0]["kind"], "desktop");
    assert_eq!(e.latest_key().as_deref(), Some("network"));
    round_trip(&Event::Network(Box::default()), "network");
}

#[test]
fn bluetooth_and_what_pairing_asks() {
    let e = Event::Bluetooth(Box::new(Bluetooth {
        on: true,
        discoverable_name: "surface".into(),
        devices: vec![BtDevice {
            address: "AA:BB:CC:00:00:01".into(),
            name: "Pixel Buds Pro".into(),
            kind: BtKind::Headphones,
            paired: true,
            connected: true,
            battery: Some(80),
        }],
        scanning: false,
    }));
    let v = round_trip(&e, "bluetooth");
    // A device's kind is nested: it doesn't meet the tag.
    assert_eq!((v["devices"][0]["kind"].as_str(), v["devices"][0]["battery"].as_u64()), (Some("headphones"), Some(80)));
    assert_eq!(e.latest_key().as_deref(), Some("bluetooth"));

    // A prompt has a kind of its own (what it asks): nested under `prompt`.
    let p = Event::BtPrompt {
        prompt: Some(BtPrompt {
            id: "bt-prompt-1".into(),
            device: "AA:BB:CC:00:00:03".into(),
            name: "JBL Flip 6".into(),
            kind: BtAsk::Confirm { code: "492817".into() },
        }),
    };
    let v = round_trip(&p, "btPrompt");
    assert_eq!((v["prompt"]["kind"].as_str(), v["prompt"]["code"].as_str()), (Some("confirm"), Some("492817")));
    for ask in [BtAsk::Show { code: "1234".into() }, BtAsk::Enter, BtAsk::Authorize] {
        let p = Event::BtPrompt {
            prompt: Some(BtPrompt { id: "p".into(), device: "AA".into(), name: "Old".into(), kind: ask }),
        };
        round_trip(&p, "btPrompt");
    }
    // None: answered (or gone), and it's the present too.
    let none = Event::BtPrompt { prompt: None };
    assert_eq!(round_trip(&none, "btPrompt"), json!({"kind": "btPrompt", "prompt": null}));
    assert_eq!(none.latest_key(), p.latest_key());
}

#[test]
fn updates() {
    let v = |s: &str| Version { version: s.into(), image: "ghcr.io/hunggod/wados-host:44".into(), digest: None };
    let e = Event::Updates(Box::new(Updates {
        online: true,
        checking: false,
        booted: v("44.20261104.0"),
        available: Some(v("44.20261105.0")),
        staged: None,
        rollback: Some(v("44.20261028.0")),
        error: None,
        checked_at: Some(5),
        progress: Some(UpdateProgress { done_bytes: 1, total_bytes: 2 }),
        job: Some("job-1".into()),
        source: wad_proto::updates::UpdateSource::Pushed,
        local: Some(v("44.20261106.0")),
        pushed: Some(v("44.20261105.1")),
    }));
    let j = round_trip(&e, "updates");
    assert_eq!(
        (j["booted"]["version"].as_str(), j["progress"]["totalBytes"].as_u64()),
        (Some("44.20261104.0"), Some(2))
    );
    assert!(j.get("staged").is_none(), "{j}");
    assert_eq!(e.latest_key().as_deref(), Some("updates"));
}

#[test]
fn storage() {
    let e = Event::Storage(Box::new(Storage {
        total: 512,
        free: 300,
        bases: 3,
        wadspaces: 100,
        other: 109,
        per_wadspace: vec![WadspaceStorage { id: "studio".into(), bytes: 100, built: Built::Built }],
    }));
    let v = round_trip(&e, "storage");
    assert_eq!(v["perWadspace"][0]["built"], "built");
    assert_eq!(e.latest_key().as_deref(), Some("storage"));
}

#[test]
fn a_notice_from_before_and_a_richer_one() {
    // What wadd sent before phase 8 (and the shell's own): a toast.
    let old: Event = serde_json::from_value(
        json!({"kind": "notice", "level": "info", "title": "Moved to the Trash", "undo": "undo-1"}),
    )
    .unwrap();
    assert_eq!(old, Event::notice(Level::Info, "Moved to the Trash", Some("undo-1".into())));
    let Event::Notice { toast, body, action, .. } = &old else { panic!("{old:?}") };
    assert!(*toast && body.is_none() && action.is_none());
    // It goes out as it did.
    assert_eq!(
        round_trip(&old, "notice"),
        json!({"kind": "notice", "level": "info", "title": "Moved to the Trash", "undo": "undo-1"})
    );
    let rich = Event::Notice {
        level: Level::Ok,
        title: "WadSpaces 44.20261105.0 is ready".into(),
        undo: None,
        body: Some("Restart to start it.".into()),
        icon: Some("download".into()),
        action: Some(NoticeAction::new("Restart", "power.restart", &[])),
        toast: false,
    };
    let v = round_trip(&rich, "notice");
    assert_eq!((v["action"]["verb"].as_str(), v["toast"].as_bool()), (Some("power.restart"), Some(false)));
    assert!(v["action"].get("args").is_none(), "{v}");
    assert_eq!(rich.latest_key(), None);
}
