//! Phase 6's and 7's events, through JSON and back as the shell decodes them
//! (wad-wadd-client's `decode`: one `Event` from each SSE's data). Each is
//! tagged `kind` beside its own fields, so none of them may have a `kind`
//! of its own at the top (a tailnet's devices and a peer do, nested).

use serde_json::{Value, json};
use wad_model::{Peer, PeerKind, StreamMode, StreamStage, Transport};
use wad_proto::streams::{SECURE_NEEDS_VM, TURN_DOWN};
use wad_proto::{
    DeviceKind, DriveState, Event, GithubAccount, PeerInfo, SignInProvider, StreamInfo, StreamList,
    StreamsAvailability, Tailnet, TailnetAsk, TailnetDevice, TailnetState,
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
