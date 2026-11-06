//! Streams (streams.rs) on the fake podman and systemd (fake.rs: a streamed
//! wadspace's sidecar starts and stops with it, and answers on its port), a
//! fake `tailscale` (wad-tailscale's: what it was asked to serve), routes in
//! memory (the proxy's activity, set by hand) and a mock of Cloudflare's
//! TURN API (wiremock): never the real ones. v1's tests/it/streams.rs, on
//! wados: start, share, stop, the idle end, the person at the machine,
//! adopting after a restart, the teardown, TURN, and what can't start.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{Rig, read, rig};
use serde_json::{Value, json};
use wad_model::{AppId, JobStatus, Op, PartState, Phase, SessionDisplay, StreamMode, StreamStage, Transport};
use wad_proto::streams::{PRIVATE_NEEDS_TAILNET, PUBLIC_NEEDS_TUNNEL, SECURE_NEEDS_VM, SECURE_PUBLIC_LATER, TURN_DOWN};
use wad_proto::{ErrorCode, Event, Level, StreamInfo, StreamRequest};
use wad_tailscale::fake::{FakeCli, fake_cli};
use wadd::api_phase7::Streams;
use wadd::streams::{MachineStreams, MemRoutes, Parts, Settings, TailscaleServe};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const STREAM_IMAGE: &str = "localhost/wados-stream:44";
const HOST: &str = "surface.example.test";
const TAILNET_HOST: &str = "desk.ts.test";
const TOKEN: &str = "turn-api-token-not-for-logs";

struct Opts {
    tailnet: bool,
    tunnel: bool,
    turn_api: Option<String>,
    turn_key: bool,
    ttl: Duration,
    idle: Duration,
    stream_image: bool,
    /// A VM runtime (krun) and KVM here: secure streams can start.
    vm: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            tailnet: true,
            tunnel: true,
            turn_api: None,
            turn_key: false,
            ttl: Duration::from_secs(3600),
            idle: Duration::from_secs(600),
            stream_image: true,
            vm: false,
        }
    }
}

struct T {
    rig: Rig,
    s: Arc<MachineStreams>,
    routes: Arc<MemRoutes>,
    cli: FakeCli,
    /// The machine's name on the tailnet (none: it isn't on it).
    tailnet: Arc<Mutex<Option<String>>>,
    o: Opts,
}

fn settings(rig: &Rig, o: &Opts) -> Settings {
    // Ports of their own, so tests side by side seldom share them (taken ones are skipped).
    let base = 20000 + (rand_u16() % 2000) * 5;
    Settings {
        enabled: true,
        state_dir: rig.state_dir(),
        image: STREAM_IMAGE.into(),
        first_web_port: base,
        first_mux_port: base + 10000,
        first_private_port: base + 20000,
        ports: 5,
        idle: o.idle,
        ready_timeout: Duration::from_secs(5),
        poll: Duration::from_millis(10),
        vm_runtime: o.vm.then(|| "krun".into()),
        vm_cpus: 2,
        vm_ram_mib: 2048,
        owner: None,
        turn: wadd::turn::Settings {
            file: rig.dir.path().join("turn.toml"),
            api: o.turn_api.clone().unwrap_or_else(|| "http://127.0.0.1:9/v1".into()),
            ttl: o.ttl,
            timeout: Duration::from_millis(500),
        },
    }
}

fn rand_u16() -> u16 {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos();
    (n ^ (n >> 16)) as u16
}

/// Streams on `rig`: a new wadd's (after a restart, the same state).
fn streams_on(
    rig: &Rig,
    o: &Opts,
    cli: &FakeCli,
    tailnet: Arc<Mutex<Option<String>>>,
) -> (Arc<MachineStreams>, Arc<MemRoutes>) {
    streams_with(rig, rig.runs.clone(), o, cli, tailnet)
}

/// Runs as a new wadd makes them on `rig`'s state and podman (common::rig's).
fn runs_again(rig: &Rig) -> Arc<wadd::runs::Runs> {
    let mut config = wad_config::Config::defaults(wad_config::Profile::User);
    config.daemon.state_dir = rig.state_dir();
    config.daemon.signins_dir = rig.state_dir().join("keys");
    config.daemon.screen_dir = rig.dir.path().join("screen");
    config.daemon.ready_timeout_s = 5;
    let settings = wadd::runs::Settings {
        poll: Duration::from_millis(10),
        home: Some(rig.home.clone()),
        ..wadd::runs::Settings::new(&config, wad_config::Profile::User)
    };
    let fake: Arc<dyn wadd::backend::Backend> = rig.fake.clone();
    wadd::runs::Runs::new(fake, rig.docs.clone(), rig.bus.clone(), rig.screen.clone(), rig.builds.clone(), settings)
}

fn streams_with(
    rig: &Rig,
    runs: Arc<wadd::runs::Runs>,
    o: &Opts,
    cli: &FakeCli,
    tailnet: Arc<Mutex<Option<String>>>,
) -> (Arc<MachineStreams>, Arc<MemRoutes>) {
    let routes = Arc::new(MemRoutes::default());
    let host = tailnet.clone();
    let private =
        TailscaleServe::new(wad_tailscale::Cli::new(&cli.bin, rig.dir.path().join("tailscaled.sock")), move || {
            host.lock().unwrap().clone()
        });
    let streaming = wad_config::Streaming {
        host: if o.tunnel { HOST.into() } else { String::new() },
        tunnel: "external".into(),
        ..Default::default()
    };
    let s = MachineStreams::new(
        settings(rig, o),
        Parts {
            backend: rig.fake.clone(),
            docs: rig.docs.clone(),
            bus: rig.bus.clone(),
            jobs: rig.jobs.clone(),
            runs: runs.clone(),
            builds: rig.builds.clone(),
            routes: routes.clone(),
            private: Arc::new(private),
            tunnel: Arc::new(wadd::tunnel::Tunnel::new(&streaming)),
            peers: None,
        },
    );
    let ends: std::sync::Weak<dyn wadd::runs::StreamEnds> = Arc::downgrade(&s) as _;
    runs.attach_streams(ends);
    (s, routes)
}

async fn up(o: Opts) -> T {
    let rig = rig();
    rig.builds.refresh_base().await;
    if o.stream_image {
        rig.fake.add_image(STREAM_IMAGE, &[]);
    }
    if o.turn_key {
        let key = format!("[cloudflare.turn]\ntoken_id = \"key-1\"\napi_token = \"{TOKEN}\"\n");
        std::fs::write(rig.dir.path().join("turn.toml"), key).unwrap();
    }
    let cli = fake_cli(&rig.dir.path().join("cli"));
    let tailnet = Arc::new(Mutex::new(o.tailnet.then(|| TAILNET_HOST.to_string())));
    let (s, routes) = streams_on(&rig, &o, &cli, tailnet.clone());
    assert!(s.adopt().await);
    T { rig, s, routes, cli, tailnet, o }
}

fn req(mode: StreamMode) -> StreamRequest {
    StreamRequest { mode, takeover: false, players: 0 }
}

impl T {
    fn wadspace(&self, name: &str) -> String {
        self.rig.wadspace(name, &[AppId::Terminal], &[])
    }

    /// Streams `id` to the end of its job: the job, as it ended.
    async fn stream(&self, id: &str, mode: StreamMode) -> wad_proto::JobInfo {
        let job = self.s.start(id, req(mode)).await.unwrap();
        assert_eq!(self.rig.finished(&job.job.id).await, JobStatus::Done, "{:?}", self.rig.jobs.info(&job.job.id));
        self.rig.jobs.info(&job.job.id).unwrap()
    }

    async fn info(&self, id: &str) -> Option<StreamInfo> {
        self.s.streams().await.unwrap().streams.into_iter().find(|s| s.id == id && s.peer.is_none())
    }

    fn guid(&self, id: &str) -> String {
        self.s.guid_of(id).expect("a stream")
    }

    fn unit(&self, name: &str) -> Option<String> {
        self.rig.fake.state.lock().unwrap().units.get(name).cloned()
    }

    fn running(&self, name: &str) -> bool {
        self.rig.fake.state.lock().unwrap().containers.get(name).is_some_and(|c| c.status == "running")
    }

    fn secrets(&self) -> Vec<String> {
        self.rig.fake.state.lock().unwrap().secrets.keys().cloned().collect()
    }

    fn secret(&self, name: &str) -> Option<String> {
        self.rig.fake.state.lock().unwrap().secrets.get(name).cloned()
    }

    fn folder(&self, id: &str) -> std::path::PathBuf {
        self.rig.state_dir().join("streams").join(id)
    }

    fn saved(&self) -> String {
        read(&self.rig.state_dir().join("streams.json"))
    }

    /// Nothing of `id`'s stream is left: units, containers, secrets, routes, folder.
    fn nothing_left(&self, id: &str) {
        assert!(self.unit(&format!("wadstream-{id}.container")).is_none(), "its sidecar's unit");
        assert!(self.unit(&format!("wadstream-{id}.volume")).is_none(), "its display volume's unit");
        assert!(!self.running(&format!("wadstream-{id}")) && !self.running(&format!("wad-{id}")), "containers");
        let left: Vec<String> = self.secrets().into_iter().filter(|s| s.starts_with("wad_stream_")).collect();
        assert!(left.is_empty(), "secrets left: {left:?}");
        assert!(self.routes.guids().is_empty(), "routes left: {:?}", self.routes.guids());
        assert!(!self.folder(id).exists(), "its folder");
        assert!(self.rig.fake.calls().contains(&format!("volume rm wadstream-{id}")));
        let st = self.rig.fake.state.lock().unwrap();
        let side = format!("wadstream-{id}");
        assert!(!st.volumes.contains(&side), "its display volume is left: {:?}", st.volumes);
        assert!(!st.volume_units.contains(&side), "its volume's oneshot is left active");
        drop(st);
        let saved: Value = serde_json::from_str(&self.saved()).unwrap();
        assert_eq!(saved["streams"], json!([]), "{saved}");
    }
}

fn mode_of(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
async fn a_private_stream_starts_shares_and_stops_leaving_nothing() {
    let t = up(Opts::default()).await;
    let id = t.wadspace("Studio");
    let job = t.stream(&id, StreamMode::Private).await;
    let labels: Vec<&str> = job.job.stages.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, ["Starting its container", "Starting the Selkies sidecar", "Serving it on your tailnet"]);
    let s = t.info(&id).await.unwrap();
    assert_eq!(
        (s.mode, s.transport, s.stage, s.live),
        (StreamMode::Private, Transport::Websocket, StreamStage::Live, true)
    );
    assert_eq!(s.job.as_deref(), Some(job.job.id.as_str()));
    let guid = t.guid(&id);
    assert_eq!(guid.len(), 32, "128 bits");
    let route = t.routes.get(&guid).expect("routed");
    let port = route.private_port.expect("its own listener");
    assert_eq!(s.link, format!("https://{TAILNET_HOST}:{port}/ws-{guid}/"));
    assert_eq!(
        (route.mode, route.user.as_str(), route.password.as_str(), route.name.as_str()),
        (StreamMode::Private, "", "", "Studio")
    );

    // Its units: it draws on its sidecar, which serves under its path on 127.0.0.1.
    let ws = t.unit(&format!("wad-{id}.container")).unwrap();
    assert!(ws.contains(&format!("BindsTo=wadstream-{id}.service")), "{ws}");
    assert!(ws.contains(&format!("Volume=wadstream-{id}.volume:{}", wad_gen::units::STREAM_DIR)), "{ws}");
    let side = t.unit(&format!("wadstream-{id}.container")).unwrap();
    for line in [
        format!("Image={STREAM_IMAGE}"),
        format!("Environment=SELKIES_SUBFOLDER=/ws-{guid}"),
        "Environment=SELKIES_MODE=websockets".to_string(),
        format!("Environment=SELKIES_ALLOWED_ORIGINS=https://{TAILNET_HOST}:{port}"),
        format!("PublishPort=127.0.0.1:{}:3000", route.web_port),
        format!("Secret=wad_stream_master_{id},type=env,target=SELKIES_MASTER_TOKEN"),
        format!("Volume={}:/run/wadspaces-selkies:ro,Z", t.folder(&id).display()),
    ] {
        assert!(side.lines().any(|l| l == line), "{line} in\n{side}");
    }
    assert!(t.running(&format!("wad-{id}")) && t.running(&format!("wadstream-{id}")));
    assert_eq!(read(&t.rig.state_dir().join("extra").join(&id).join("display")), "stream\n");
    let run = t.rig.runs.state(&id);
    assert_eq!((run.state.phase, run.display), (Phase::Ready, Some(SessionDisplay::Stream)));
    assert!(t.rig.runs.on_screen().is_empty(), "not on the screen");

    // Its secrets: podman's, and its folder's token table (0600 in a 0700 folder).
    let key = t.secret(&format!("wad_stream_key_{id}")).expect("its key");
    assert_eq!((key.len(), route.key.as_str()), (32, key.as_str()));
    assert_eq!(t.secret(&format!("wad_stream_master_{id}")).map(|m| m.len()), Some(64));
    assert!(t.secret(&format!("wad_stream_password_{id}")).is_none(), "a private stream has no sign-in");
    let tokens: Value = serde_json::from_str(&read(&t.folder(&id).join("tokens.json"))).unwrap();
    assert_eq!(tokens, json!({ key.clone(): {"role": "controller", "slot": null} }));
    assert_eq!((mode_of(&t.folder(&id)), mode_of(&t.folder(&id).join("tokens.json"))), (0o700, 0o600));
    assert!(!t.folder(&id).join("rtc.json").exists(), "no ICE servers: it isn't WebRTC");

    // The tailnet serves its listener.
    let runs = t.cli.runs();
    assert!(
        runs.iter().any(|r| r.ends_with(&format!("serve --bg --yes --https={port} http://127.0.0.1:{port}"))),
        "{runs:?}"
    );

    // streams.json: 0600, and nothing secret.
    let saved = t.saved();
    assert_eq!(mode_of(&t.rig.state_dir().join("streams.json")), 0o600);
    assert!(saved.contains(&guid) && !saved.contains(&key), "{saved}");

    // The share: the one-tap link, its key after the #.
    let share = t.s.share(&id).await.unwrap();
    assert_eq!(share.open_link, format!("{}#token={key}", s.link));
    assert_eq!((share.link.as_str(), share.user.as_deref(), share.password.as_deref()), (s.link.as_str(), None, None));
    assert!(!format!("{share:?}").contains(&key));

    // One stream each.
    let e = t.s.start(&id, req(StreamMode::Public)).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);
    assert!(e.message.contains("streaming already"), "{}", e.message);

    // Stopped: ended, then nothing of it is left.
    let ended = t.s.stop(&id).await.unwrap();
    assert_eq!((ended.stage, ended.live), (StreamStage::Ended, false));
    assert_eq!(t.info(&id).await.map(|i| i.stage), Some(StreamStage::Ended), "it's shown ended a moment");
    t.nothing_left(&id);
    assert!(t.cli.runs().iter().any(|r| r.ends_with(&format!("serve --https={port} off"))), "{:?}", t.cli.runs());
    assert_eq!(t.rig.runs.state(&id).state.phase, Phase::Idle);
    assert_eq!(t.s.stop(&id).await.unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(t.s.share(&id).await.unwrap_err().code, ErrorCode::NotFound);

    // Streamed again: a new guid and key.
    t.stream(&id, StreamMode::Private).await;
    assert_ne!(t.guid(&id), guid);
    assert_ne!(t.secret(&format!("wad_stream_key_{id}")).unwrap(), key);
}

fn cloudflare(user: &str) -> Value {
    json!({"iceServers": [
        {"urls": ["stun:stun.cloudflare.com:3478", "stun:stun.cloudflare.com:53"]},
        {"urls": [
            "turn:turn.cloudflare.com:3478?transport=udp",
            "turn:turn.cloudflare.com:53?transport=udp",
            "turns:turn.cloudflare.com:443?transport=tcp"
        ], "username": user, "credential": format!("credential-of-{user}")}
    ]})
}

async fn cloudflare_mock() -> MockServer {
    let cf = MockServer::start().await;
    // The first mint, then the next (a refresh).
    Mock::given(method("POST"))
        .and(path("/v1/turn/keys/key-1/credentials/generate-ice-servers"))
        .respond_with(ResponseTemplate::new(201).set_body_json(cloudflare("u-1")))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&cf)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/turn/keys/key-1/credentials/generate-ice-servers"))
        .respond_with(ResponseTemplate::new(201).set_body_json(cloudflare("u-2")))
        .with_priority(2)
        .mount(&cf)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/v1/turn/keys/key-1/credentials/u-[0-9]/revoke$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&cf)
        .await;
    cf
}

/// The usernames Cloudflare was asked to revoke.
async fn revoked(cf: &MockServer) -> Vec<String> {
    let mut out: Vec<String> = cf
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|r| r.url.path().strip_suffix("/revoke").map(|p| p.rsplit('/').next().unwrap().to_string()))
        .collect();
    out.sort();
    out
}

#[tokio::test]
async fn a_public_stream_signs_in_relays_through_turn_and_renews_it() {
    let cf = cloudflare_mock().await;
    let o = Opts {
        turn_api: Some(format!("{}/v1", cf.uri())),
        turn_key: true,
        ttl: Duration::from_secs(2),
        ..Default::default()
    };
    let t = up(o).await;
    let id = t.wadspace("Studio");
    let job = t.stream(&id, StreamMode::Public).await;
    let ids: Vec<&str> = job.job.stages.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["container", "sidecar", "route", "turn"]);
    assert!(job.job.stages.iter().all(|s| s.state == PartState::Done && s.error.is_none()), "{:?}", job.job.stages);
    let s = t.info(&id).await.unwrap();
    let guid = t.guid(&id);
    assert_eq!(s.link, format!("https://{HOST}/ws-{guid}/"));
    assert_eq!((s.transport, s.turn_down, s.warnings.len()), (Transport::Webrtc, false, 0));

    // Its ICE servers: Cloudflare's, without port 53, in its folder.
    let rtc = t.folder(&id).join("rtc.json");
    let ice: Value = serde_json::from_str(&read(&rtc)).unwrap();
    assert_eq!(ice["iceServers"][1]["username"], "u-1");
    assert!(!read(&rtc).contains(":53?") && !read(&rtc).contains(":53\""), "{ice}");
    assert_eq!(mode_of(&rtc), 0o600);
    let side = t.unit(&format!("wadstream-{id}.container")).unwrap();
    assert!(
        side.contains("Environment=SELKIES_MODE=webrtc")
            && side.contains("SELKIES_RTC_CONFIG_JSON=/run/wadspaces-selkies/rtc.json"),
        "{side}"
    );
    assert!(side.contains("/udp") && side.contains("/tcp"), "its mux port: {side}");
    assert!(side.contains(&format!("SELKIES_ALLOWED_ORIGINS=https://{HOST}")), "{side}");

    // Its sign-in: a user and password, the proxy's and the share's.
    let share = t.s.share(&id).await.unwrap();
    let (user, password) = (share.user.clone().unwrap(), share.password.clone().unwrap());
    assert!(user.starts_with("wad-") && password.len() == 20, "{user}");
    let route = t.routes.get(&guid).unwrap();
    assert_eq!(
        (route.user.as_str(), route.password.as_str(), route.private_port),
        (user.as_str(), password.as_str(), None)
    );
    assert_eq!(t.secret(&format!("wad_stream_password_{id}")).as_deref(), Some(password.as_str()));
    assert!(share.open_link.starts_with(&s.link) && share.open_link.contains("#token="));
    // Never in streams.json: the key, the password, Cloudflare's token or a credential.
    let saved = t.saved();
    for secret in [password.as_str(), &route.key, TOKEN, "credential-of"] {
        assert!(!saved.contains(secret), "{secret} in {saved}");
    }
    assert!(saved.contains(&user));

    // Renewed before they run out: new credentials in its file.
    tokio::time::sleep(Duration::from_millis(50)).await;
    t.s.tick().await;
    let ice: Value = serde_json::from_str(&read(&rtc)).unwrap();
    assert_eq!(ice["iceServers"][1]["username"], "u-2", "{ice}");
    assert!(!t.info(&id).await.unwrap().turn_down);

    // Ended: everything it minted is revoked.
    t.s.stop(&id).await.unwrap();
    t.nothing_left(&id);
    assert_eq!(revoked(&cf).await, ["u-1", "u-2"]);
}

#[tokio::test]
async fn without_turn_a_public_stream_still_starts_and_says_so() {
    let t = up(Opts::default()).await;
    let id = t.wadspace("Studio");
    let job = t.stream(&id, StreamMode::Public).await;
    let turn = job.job.stages.iter().find(|s| s.id == "turn").unwrap();
    assert_eq!((turn.state, turn.error.as_deref()), (PartState::Done, Some(TURN_DOWN)));
    let s = t.info(&id).await.unwrap();
    assert!(s.live && s.turn_down, "{s:?}");
    assert_eq!(s.warnings, [TURN_DOWN]);
    // STUN alone.
    let ice: Value = serde_json::from_str(&read(&t.folder(&id).join("rtc.json"))).unwrap();
    assert_eq!(ice, json!({"iceServers": [{"urls": ["stun:stun.cloudflare.com:3478"]}]}));
    let log = t.rig.jobs.get(&job.job.id, 0).unwrap().lines;
    assert!(log.iter().any(|l| l.contains("no TURN key")), "{log:?}");
}

#[tokio::test]
async fn turn_refusing_the_key_is_turn_down_too() {
    let cf = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(401)).mount(&cf).await;
    let t = up(Opts { turn_api: Some(format!("{}/v1", cf.uri())), turn_key: true, ..Default::default() }).await;
    let id = t.wadspace("Studio");
    t.stream(&id, StreamMode::Public).await;
    let s = t.info(&id).await.unwrap();
    assert!(s.live && s.turn_down && s.warnings == [TURN_DOWN], "{s:?}");
}

#[tokio::test]
async fn a_stream_nobody_watches_ends() {
    let t = up(Opts { idle: Duration::from_millis(300), ..Default::default() }).await;
    let id = t.wadspace("Studio");
    t.stream(&id, StreamMode::Private).await;
    let guid = t.guid(&id);
    // Someone's watching: it stays, however long ago they last did something.
    t.routes.set_activity(&guid, 1, 2);
    tokio::time::sleep(Duration::from_millis(400)).await;
    t.s.tick().await;
    let s = t.info(&id).await.unwrap();
    assert_eq!((s.live, s.viewers), (true, 2));
    // Nobody now, and not for a while: it ends.
    t.routes.set_activity(&guid, wadd::jobs::now_ms(), 0);
    t.s.tick().await;
    assert!(t.info(&id).await.unwrap().live, "watched just now");
    tokio::time::sleep(Duration::from_millis(400)).await;
    t.s.tick().await;
    assert_eq!(t.info(&id).await.map(|s| s.stage), Some(StreamStage::Ended));
    t.nothing_left(&id);
}

#[tokio::test]
async fn the_person_at_the_machine_wins() {
    let t = up(Opts::default()).await;
    let id = t.wadspace("Studio");
    t.stream(&id, StreamMode::Private).await;
    let (_, mut rx) = t.rig.bus.subscribe();
    // Opened on the screen: its stream ends, and it opens here.
    t.rig.runs.open(&id, SessionDisplay::Screen).unwrap();
    for _ in 0..300 {
        let r = t.rig.runs.state(&id);
        if r.state.phase == Phase::Ready && r.display == Some(SessionDisplay::Screen) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let r = t.rig.runs.state(&id);
    assert_eq!((r.state.phase, r.display), (Phase::Ready, Some(SessionDisplay::Screen)), "{r:?}");
    assert_eq!(t.info(&id).await.map(|s| s.stage), Some(StreamStage::Ended));
    assert!(t.routes.guids().is_empty() && t.unit(&format!("wadstream-{id}.container")).is_none());
    assert!(!t.secrets().iter().any(|s| s.starts_with("wad_stream_")));
    let unit = t.unit(&format!("wad-{id}.container")).unwrap();
    assert!(!unit.contains("BindsTo=") && unit.contains(&t.rig.screen_dir(&id).display().to_string()), "{unit}");
    assert_eq!(read(&t.rig.state_dir().join("extra").join(&id).join("display")), "screen\n");
    let told = common::drain(&mut rx)
        .into_iter()
        .any(|e| matches!(e, Event::Notice { level: Level::Info, ref title, .. } if title.contains("stream ended")));
    assert!(told, "a notice says its stream ended");
}

#[tokio::test]
async fn streaming_what_is_on_the_screen_takes_it_off_when_confirmed() {
    let t = up(Opts::default()).await;
    let id = t.wadspace("Studio");
    t.rig.runs.open(&id, SessionDisplay::Screen).unwrap();
    t.rig.runs.settled(&id).await;
    let e = t.s.start(&id, req(StreamMode::Private)).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);
    assert!(e.message.contains("takes it off the screen"), "{}", e.message);
    assert!(t.info(&id).await.is_none());
    let job = t.s.start(&id, StreamRequest { mode: StreamMode::Private, takeover: true, players: 0 }).await.unwrap();
    assert_eq!(t.rig.finished(&job.job.id).await, JobStatus::Done, "{:?}", t.rig.jobs.info(&job.job.id));
    let calls = t.rig.fake.calls();
    let stop = calls.iter().position(|c| *c == format!("stop wad-{id}.service")).expect("stopped on the screen");
    let start = calls.iter().rposition(|c| *c == format!("start wad-{id}.service")).unwrap();
    assert!(stop < start, "{calls:?}");
    assert_eq!(t.rig.runs.state(&id).display, Some(SessionDisplay::Stream));
    assert!(t.rig.runs.on_screen().is_empty());
    // Started again for it, so stopped with it: not back on the screen.
    t.s.stop(&id).await.unwrap();
    assert_eq!(t.rig.runs.state(&id).state.phase, Phase::Idle);
    t.nothing_left(&id);
}

#[tokio::test]
async fn a_stream_ends_when_its_wadspace_stops() {
    let t = up(Opts::default()).await;
    let id = t.wadspace("Studio");
    t.stream(&id, StreamMode::Private).await;
    // Its window closed: its desktop halted it, and its sidecar went with it.
    t.rig.fake.exit(&format!("wad-{id}"));
    t.s.tick().await;
    assert_eq!(t.info(&id).await.map(|s| s.stage), Some(StreamStage::Ended));
    t.nothing_left(&id);
    // Stopped from the shell: the same.
    t.stream(&id, StreamMode::Private).await;
    t.rig.runs.stop(&id).await.unwrap();
    assert!(!t.running(&format!("wadstream-{id}")), "the sidecar stops with it");
    t.s.tick().await;
    t.nothing_left(&id);
}

#[tokio::test]
async fn a_start_that_fails_or_is_cancelled_leaves_nothing() {
    let t = up(Opts::default()).await;
    let id = t.wadspace("Studio");
    // Its sidecar never answers.
    t.rig.fake.state.lock().unwrap().sidecars_silent = true;
    let job = t.s.start(&id, req(StreamMode::Private)).await.unwrap();
    assert_eq!(t.rig.finished(&job.job.id).await, JobStatus::Error);
    let err = t.rig.jobs.info(&job.job.id).unwrap().job.error.unwrap();
    assert!(err.contains("Selkies sidecar didn't answer"), "{err}");
    t.nothing_left(&id);
    assert!(!t.cli.runs().iter().any(|r| r.contains("serve --bg")), "never served");

    // Cancelled while its desktop comes up.
    t.rig.fake.state.lock().unwrap().sidecars_silent = false;
    t.rig.fake.state.lock().unwrap().desktop_after = 10_000;
    let job = t.s.start(&id, req(StreamMode::Private)).await.unwrap();
    for _ in 0..200 {
        if t.running(&format!("wad-{id}")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    t.rig.jobs.cancel(&job.job.id).unwrap();
    for _ in 0..300 {
        if !t.running(&format!("wad-{id}")) && t.s.guid_of(&id).is_none() && !t.folder(&id).exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    t.nothing_left(&id);
    assert_eq!(t.rig.jobs.info(&job.job.id).unwrap().job.status, JobStatus::Cancelled);
}

#[tokio::test]
async fn after_a_restart_running_streams_carry_on_and_the_rest_go() {
    let t = up(Opts::default()).await;
    let kept = t.wadspace("Kept");
    let gone = t.wadspace("Gone");
    t.stream(&kept, StreamMode::Private).await;
    t.stream(&gone, StreamMode::Private).await;
    let (kept_guid, kept_key) = (t.guid(&kept), t.routes.get(&t.guid(&kept)).unwrap().key);
    let gone_port = t.routes.get(&t.guid(&gone)).unwrap().private_port.unwrap();
    // wadd stops: its routes go with it. Meanwhile one sidecar stops, and
    // something no stream has is left behind.
    t.rig.fake.exit(&format!("wadstream-{gone}"));
    let idle = t.wadspace("Idle");
    {
        let mut st = t.rig.fake.state.lock().unwrap();
        st.secrets.insert("wad_stream_key_ghost".into(), "x".into());
        // A display volume no stream has, and a volume's oneshot left active with its volume gone.
        st.volumes.insert("wadstream-ghost".into());
        st.volume_units.insert(format!("wadstream-{idle}"));
    }
    std::fs::create_dir_all(t.folder("ghost")).unwrap();

    let (again, routes) = streams_on(&t.rig, &t.o, &t.cli, t.tailnet.clone());
    let before = again.streams().await.unwrap();
    assert!(before.streams.iter().all(|s| !s.live), "not routed until it's looked at");
    assert!(again.adopt().await);
    let s = again.streams().await.unwrap().streams.into_iter().find(|s| s.id == kept).unwrap();
    assert_eq!((s.stage, s.live), (StreamStage::Live, true));
    let route = routes.get(&kept_guid).expect("routed again");
    assert_eq!(route.key, kept_key, "its key, from podman");
    // The one whose sidecar stopped ended: nothing of it is left.
    let g = again.streams().await.unwrap().streams.into_iter().find(|s| s.id == gone).unwrap();
    assert_eq!(g.stage, StreamStage::Ended);
    assert!(routes.guids() == [kept_guid.clone()], "{:?}", routes.guids());
    assert!(!t.folder(&gone).exists() && !t.folder("ghost").exists() && t.folder(&kept).exists());
    let secrets = t.secrets();
    assert!(!secrets.iter().any(|s| s.ends_with(&format!("_{gone}")) || s.ends_with("_ghost")), "{secrets:?}");
    assert!(secrets.contains(&format!("wad_stream_key_{kept}")));
    assert!(t.cli.runs().iter().any(|r| r.ends_with(&format!("serve --https={gone_port} off"))));
    {
        let st = t.rig.fake.state.lock().unwrap();
        assert!(!st.volumes.contains("wadstream-ghost") && !st.volumes.contains(&format!("wadstream-{gone}")));
        assert!(st.volumes.contains(&format!("wadstream-{kept}")), "the running stream's stays");
        assert!(!st.volume_units.contains(&format!("wadstream-{idle}")), "the stale oneshot is stopped");
    }
    assert!(t.rig.fake.calls().contains(&format!("reset-failed wadstream-{idle}-volume.service")));
    let saved = t.saved();
    assert!(saved.contains(&kept_guid) && !saved.contains(&format!("\"{gone}\"")), "{saved}");
    // And it stops as any other.
    again.stop(&kept).await.unwrap();
    assert!(routes.guids().is_empty());
}

#[tokio::test]
async fn what_cant_start_says_why() {
    let t = up(Opts { tailnet: false, tunnel: false, ..Default::default() }).await;
    let id = t.wadspace("Studio");
    let a = t.s.streams().await.unwrap().available;
    assert_eq!(a.private.reason.as_deref(), Some(PRIVATE_NEEDS_TAILNET));
    assert_eq!(a.public.reason.as_deref(), Some(PUBLIC_NEEDS_TUNNEL));
    assert_eq!(a.private_secure.reason.as_deref(), Some(SECURE_NEEDS_VM));
    assert_eq!(a.public_secure.reason.as_deref(), Some(SECURE_PUBLIC_LATER));
    for (mode, why) in [
        (StreamMode::Private, PRIVATE_NEEDS_TAILNET),
        (StreamMode::Public, PUBLIC_NEEDS_TUNNEL),
        (StreamMode::PrivateSecure, SECURE_NEEDS_VM),
        (StreamMode::PublicSecure, SECURE_PUBLIC_LATER),
    ] {
        let e = t.s.start(&id, req(mode)).await.unwrap_err();
        assert_eq!((e.code, e.message.as_str()), (ErrorCode::Conflict, why), "{mode:?}");
    }
    // On the tailnet now: private can start, and the list says so.
    *t.tailnet.lock().unwrap() = Some(TAILNET_HOST.into());
    let a = t.s.streams().await.unwrap().available;
    assert!(a.private.available && !a.public.available);
    assert_eq!(t.s.start("nope", req(StreamMode::Private)).await.unwrap_err().code, ErrorCode::NotFound);
    // Not built here.
    let node = t.rig.wadspace("Node", &[AppId::Terminal], &["terminal:nodejs"]);
    let e = t.s.start(&node, req(StreamMode::Private)).await.unwrap_err();
    assert!(e.code == ErrorCode::Conflict && e.message.contains("build it first"), "{}", e.message);
    assert!(t.secrets().is_empty() && t.routes.guids().is_empty(), "nothing made for what didn't start");
}

#[tokio::test]
async fn without_the_sidecars_image_nothing_starts() {
    let t = up(Opts { stream_image: false, ..Default::default() }).await;
    let id = t.wadspace("Studio");
    let e = t.s.start(&id, req(StreamMode::Private)).await.unwrap_err();
    assert!(e.code == ErrorCode::Conflict && e.message.contains(STREAM_IMAGE), "{}", e.message);
    assert!(t.info(&id).await.is_none());
}

/// The base, as images/base labels it: its desktop moves as it runs.
async fn moving_base(t: &T) {
    use wad_gen::{BASE_IMAGE, labels};
    t.rig
        .fake
        .add_image(BASE_IMAGE, &[(labels::ABI, common::ABI), (labels::EXTRAS, "1"), (wadd::runs::MOVES_LABEL, "2")]);
    t.rig.builds.refresh_base().await;
}

async fn until_shown(t: &T, id: &str, display: SessionDisplay) {
    for _ in 0..300 {
        let r = t.rig.runs.state(id);
        if r.state.phase == Phase::Ready && r.display == Some(display) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{id} wasn't shown {display:?}: {:?}", t.rig.runs.state(id));
}

#[tokio::test]
async fn a_moving_wadspace_goes_to_its_stream_and_back_as_it_runs() {
    let t = up(Opts::default()).await;
    moving_base(&t).await;
    let id = t.wadspace("Studio");
    t.rig.runs.open(&id, SessionDisplay::Screen).unwrap();
    t.rig.runs.settled(&id).await;
    // On the screen with both displays, ready to move.
    let unit = t.unit(&format!("wad-{id}.container")).unwrap();
    assert!(!unit.contains("BindsTo=") && unit.contains(wad_gen::units::STREAM_DIR), "{unit}");
    assert!(t.unit(&format!("wadstream-{id}.volume")).is_some() && t.rig.runs.moves(&id));
    let extra = t.rig.state_dir().join("extra").join(&id);
    assert_eq!(read(&extra.join("display")), "screen\n");

    // Streamed, as confirmed: no container stage, and it isn't started again.
    let job = t.s.start(&id, StreamRequest { mode: StreamMode::Private, takeover: true, players: 0 }).await.unwrap();
    let ids: Vec<&str> = job.job.stages.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["sidecar", "serve"]);
    assert_eq!(t.rig.finished(&job.job.id).await, JobStatus::Done, "{:?}", t.rig.jobs.info(&job.job.id));
    let calls = t.rig.fake.calls();
    let side = calls.iter().position(|c| *c == format!("start wadstream-{id}.service")).expect("its sidecar started");
    let switch = format!("exec wad-{id} abc wadspaces-session switch --timeout 20");
    let moved = calls.iter().position(|c| *c == switch).expect("moved");
    assert!(side < moved, "the sidecar first: {calls:?}");
    assert!(!calls.contains(&format!("stop wad-{id}.service")), "{calls:?}");
    assert_eq!(calls.iter().filter(|c| **c == format!("start wad-{id}.service")).count(), 1, "started once");
    assert_eq!(read(&extra.join("display")), "stream\n");
    let r = t.rig.runs.state(&id);
    assert_eq!((r.state.phase, r.display), (Phase::Ready, Some(SessionDisplay::Stream)));
    assert!(t.info(&id).await.unwrap().live);
    let side_unit = t.unit(&format!("wadstream-{id}.container")).unwrap();
    assert!(!side_unit.contains("StopWhenUnneeded"), "unbound: {side_unit}");

    // Opened at the machine: back on the screen as it runs, its stream over.
    t.rig.runs.open(&id, SessionDisplay::Window).unwrap();
    until_shown(&t, &id, SessionDisplay::Window).await;
    for _ in 0..300 {
        if t.s.guid_of(&id).is_none() && t.routes.guids().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let calls = t.rig.fake.calls();
    assert_eq!(calls.iter().filter(|c| **c == switch).count(), 2, "moved back: {calls:?}");
    assert!(!calls.contains(&format!("stop wad-{id}.service")) && t.running(&format!("wad-{id}")), "never stopped");
    assert!(!t.running(&format!("wadstream-{id}")) && t.unit(&format!("wadstream-{id}.container")).is_none());
    assert!(t.unit(&format!("wadstream-{id}.volume")).is_some(), "a moving wadspace keeps its display volume");
    assert_eq!(read(&extra.join("display")), "screen\n");
    assert!(!t.secrets().iter().any(|s| s.starts_with("wad_stream_")) && !t.folder(&id).exists());
}

#[tokio::test]
async fn apps_that_wont_close_keep_it_on_the_screen() {
    let t = up(Opts::default()).await;
    moving_base(&t).await;
    let id = t.wadspace("Studio");
    t.rig.runs.open(&id, SessionDisplay::Screen).unwrap();
    t.rig.runs.settled(&id).await;
    t.rig.fake.state.lock().unwrap().exec_answers = vec![(
        "wadspaces-session switch".into(),
        3,
        r#"{"still_open": [{"app_id": "gedit", "title": "Notes.txt"}]}"#.into(),
    )];
    let job = t.s.start(&id, StreamRequest { mode: StreamMode::Private, takeover: true, players: 0 }).await.unwrap();
    assert_eq!(t.rig.finished(&job.job.id).await, JobStatus::Error);
    let err = t.rig.jobs.info(&job.job.id).unwrap().job.error.unwrap();
    assert!(err.contains("Notes.txt didn't close"), "{err}");
    // Where it was, what closed open again, and nothing of the stream left.
    let r = t.rig.runs.state(&id);
    assert_eq!((r.state.phase, r.display), (Phase::Ready, Some(SessionDisplay::Screen)));
    assert!(t.rig.fake.calls().contains(&format!("exec wad-{id} abc wadspaces-session restore")));
    assert_eq!(read(&t.rig.state_dir().join("extra").join(&id).join("display")), "screen\n");
    assert!(!t.running(&format!("wadstream-{id}")) && t.routes.guids().is_empty());
    assert!(!t.secrets().iter().any(|s| s.starts_with("wad_stream_")));
    assert_eq!(t.info(&id).await.map(|s| s.stage), Some(StreamStage::Ended));
}

// ------------------------------------------------------------ secure streams

#[tokio::test]
async fn a_private_secure_stream_runs_in_its_vm_and_stops_leaving_nothing() {
    let t = up(Opts { vm: true, ..Default::default() }).await;
    let id = t.wadspace("Studio");
    let a = t.s.streams().await.unwrap().available;
    assert!(a.private_secure.available, "{a:?}");
    assert_eq!(a.public_secure.reason.as_deref(), Some(SECURE_PUBLIC_LATER));
    let e = t.s.start(&id, req(StreamMode::PublicSecure)).await.unwrap_err();
    assert_eq!((e.code, e.message.as_str()), (ErrorCode::Conflict, SECURE_PUBLIC_LATER));

    let job = t.stream(&id, StreamMode::PrivateSecure).await;
    let labels: Vec<&str> = job.job.stages.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(
        labels,
        ["Starting its VM", "Starting its container", "Starting the Selkies sidecar", "Serving it on your tailnet"]
    );
    let s = t.info(&id).await.unwrap();
    assert_eq!(
        (s.mode, s.transport, s.stage, s.live),
        (StreamMode::PrivateSecure, Transport::Websocket, StreamStage::Live, true)
    );
    let guid = t.guid(&id);
    let route = t.routes.get(&guid).expect("routed");
    assert!(route.private_port.is_some() && route.mode == StreamMode::PrivateSecure);

    // One container, its own image in its VM, with Selkies inside: no sidecar.
    let ws = t.unit(&format!("wad-{id}.container")).unwrap();
    for line in [
        "GlobalArgs=--runtime=krun".to_string(),
        "Annotation=krun.cpus=2".into(),
        "Annotation=krun.ram_mib=2048".into(),
        "Entrypoint=/usr/libexec/wados-secure-init".into(),
        "StopSignal=SIGINT".into(),
        "StopTimeout=30".into(),
        format!("Label=wadspaces.stream={guid}"),
        format!("Volume=wadsecure-{id}.volume:/opt/wados-stream:ro"),
        format!("Environment=SELKIES_SUBFOLDER=/ws-{guid}"),
        "Environment=SELKIES_MODE=websockets".into(),
        format!("PublishPort=127.0.0.1:{}:3000", route.web_port),
        format!("Secret=wad_stream_master_{id},type=env,target=SELKIES_MASTER_TOKEN"),
        format!("Volume={}:/run/wadspaces-selkies:ro,Z", t.folder(&id).display()),
    ] {
        assert!(ws.lines().any(|l| l == line), "{line} in\n{ws}");
    }
    assert!(!ws.contains("BindsTo") && !ws.contains("wadstream-"), "{ws}");
    let volume = t.unit(&format!("wadsecure-{id}.volume")).expect("its image volume's unit");
    assert!(volume.contains("Driver=image") && volume.contains(&format!("Image={STREAM_IMAGE}")), "{volume}");
    assert!(
        t.unit(&format!("wadstream-{id}.container")).is_none() && t.unit(&format!("wadstream-{id}.volume")).is_none()
    );
    assert!(t.running(&format!("wad-{id}")) && !t.running(&format!("wadstream-{id}")));
    assert_eq!(read(&t.rig.state_dir().join("extra").join(&id).join("display")), "stream\n");
    let run = t.rig.runs.state(&id);
    assert_eq!((run.state.phase, run.display), (Phase::Ready, Some(SessionDisplay::Stream)));
    // Ready by Selkies' health on the port its VM publishes: nothing exec'd
    // into the VM (the fake, as crun-krun, has no exec there).
    let calls = t.rig.fake.calls();
    assert!(!calls.iter().any(|c| c.starts_with("exec ")), "{calls:?}");

    // Its secrets and token table, as any stream's; its one-tap link.
    let key = t.secret(&format!("wad_stream_key_{id}")).expect("its key");
    let tokens: Value = serde_json::from_str(&read(&t.folder(&id).join("tokens.json"))).unwrap();
    assert_eq!(tokens, json!({ key.clone(): {"role": "controller", "slot": null} }));
    assert_eq!(t.s.share(&id).await.unwrap().open_link, format!("{}#token={key}", s.link));

    // Stopped: its unit's SIGINT stop; its image volume's unit, then the volume, go.
    let ended = t.s.stop(&id).await.unwrap();
    assert_eq!((ended.stage, ended.live), (StreamStage::Ended, false));
    t.nothing_left(&id);
    assert!(t.unit(&format!("wadsecure-{id}.volume")).is_none(), "its image volume's unit");
    let calls = t.rig.fake.calls();
    let at = |c: String| calls.iter().rposition(|x| *x == c).unwrap_or_else(|| panic!("no {c}: {calls:?}"));
    assert!(at(format!("stop wad-{id}.service")) < at(format!("volume rm wadsecure-{id}")));
    assert!(at(format!("stop wadsecure-{id}-volume.service")) < at(format!("volume rm wadsecure-{id}")));
    assert!(!calls.iter().any(|c| c.starts_with("exec ")), "{calls:?}");
    assert_eq!(t.rig.runs.state(&id).state.phase, Phase::Idle);

    // Streamed again (a new image volume made from the stream image), and
    // then not securely: no VM, and no secure stream's units.
    t.stream(&id, StreamMode::PrivateSecure).await;
    t.s.stop(&id).await.unwrap();
    t.stream(&id, StreamMode::Private).await;
    let ws = t.unit(&format!("wad-{id}.container")).unwrap();
    assert!(!ws.contains("GlobalArgs") && ws.contains(&format!("BindsTo=wadstream-{id}.service")), "{ws}");
    assert!(t.unit(&format!("wadsecure-{id}.volume")).is_none());
    t.s.stop(&id).await.unwrap();
    t.nothing_left(&id);
}

#[tokio::test]
async fn a_secure_stream_refuses_the_machines_own_folders() {
    let t = up(Opts { vm: true, ..Default::default() }).await;
    let base = t.wadspace("Thesis");
    let mut doc = t.rig.docs.wadspace(&base).unwrap();
    doc.look.projects = vec![wad_model::Project {
        id: "p-thesis".into(),
        as_name: "Thesis".into(),
        kind: wad_model::ProjectKind::Mount {
            source: wad_model::ProjectSource { machine_id: "m-test".into(), paths: vec!["~/Thesis".into()] },
            make: false,
        },
    }];
    let id = t.rig.edit(Op::Import { doc: Box::new(doc), from: "m-test".into() }).created.unwrap();
    let e = t.s.start(&id, req(StreamMode::PrivateSecure)).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);
    assert!(e.message.starts_with("Secure streams can't take a folder from this machine (Thesis)"), "{}", e.message);
    assert!(t.info(&id).await.is_none() && t.secrets().is_empty() && t.routes.guids().is_empty());
    assert!(t.unit(&format!("wad-{id}.container")).is_none(), "nothing written for it");
}

#[test]
fn secure_streams_need_their_runtime_and_kvm() {
    let d = tempfile::tempdir().unwrap();
    let krun = d.path().join("krun");
    std::fs::write(&krun, "").unwrap();
    let kvm = d.path().join("kvm");
    let mut config = wad_config::Config::defaults(wad_config::Profile::User);
    config.streaming.secure_runtime = krun.to_string_lossy().into_owned();
    (config.streaming.secure_cpus, config.streaming.secure_ram_mib) = (3, 3072);
    assert_eq!(Settings::with_kvm(&config, &kvm).vm_runtime, None, "no /dev/kvm");
    std::fs::write(&kvm, "").unwrap();
    let s = Settings::with_kvm(&config, &kvm);
    assert_eq!((s.vm_runtime.as_deref(), s.vm_cpus, s.vm_ram_mib), (krun.to_str(), 3, 3072));
    config.streaming.secure_runtime.clear();
    assert_eq!(Settings::with_kvm(&config, &kvm).vm_runtime, None, "secure streams off");
    config.streaming.secure_runtime = d.path().join("none").to_string_lossy().into_owned();
    assert_eq!(Settings::with_kvm(&config, &kvm).vm_runtime, None, "no such runtime");
}

#[tokio::test]
async fn without_kvm_a_secure_stream_says_why() {
    let t = up(Opts { vm: false, ..Default::default() }).await;
    let id = t.wadspace("Studio");
    let a = t.s.streams().await.unwrap().available;
    assert!(a.private.available);
    assert_eq!(a.private_secure.reason.as_deref(), Some(SECURE_NEEDS_VM));
    let e = t.s.start(&id, req(StreamMode::PrivateSecure)).await.unwrap_err();
    assert_eq!((e.code, e.message.as_str()), (ErrorCode::Conflict, SECURE_NEEDS_VM));
}

#[tokio::test]
async fn after_a_restart_a_secure_stream_carries_on() {
    let t = up(Opts { vm: true, ..Default::default() }).await;
    let id = t.wadspace("Studio");
    t.stream(&id, StreamMode::PrivateSecure).await;
    let guid = t.guid(&id);
    let (again, routes) = streams_on(&t.rig, &t.o, &t.cli, t.tailnet.clone());
    assert!(again.adopt().await);
    let s = again.streams().await.unwrap().streams.into_iter().find(|s| s.id == id).unwrap();
    assert_eq!((s.mode, s.stage, s.live), (StreamMode::PrivateSecure, StreamStage::Live, true));
    assert!(routes.get(&guid).is_some(), "routed again");
    t.rig.runs.reconcile().await;
    let run = t.rig.runs.state(&id);
    assert_eq!((run.state.phase, run.display), (Phase::Ready, Some(SessionDisplay::Stream)));
    assert!(!t.rig.fake.calls().iter().any(|c| c.starts_with("exec ")));
    again.stop(&id).await.unwrap();
    assert!(routes.guids().is_empty() && !t.running(&format!("wad-{id}")));
}

#[tokio::test]
async fn a_slow_stop_never_leaves_the_display_volume() {
    let t = up(Opts::default()).await;
    let id = t.wadspace("Studio");
    t.stream(&id, StreamMode::Private).await;
    let side = format!("wadstream-{id}");
    assert!(t.rig.fake.state.lock().unwrap().volumes.contains(&side));
    // Podman takes a while to remove a stopped container, and its volume is in use till then.
    t.rig.fake.state.lock().unwrap().slow_stops = 8;
    {
        use wadd::backend::Backend;
        let ws = format!("wad-{id}.service");
        t.rig.fake.stop(&ws).await.unwrap();
        let e = t.rig.fake.remove_volume(&side).await.unwrap_err();
        assert!(e.contains("being used"), "the race: {e}");
        t.rig.fake.start(&ws).await.unwrap();
    }
    t.s.stop(&id).await.unwrap();
    t.nothing_left(&id);
    // In order: its containers gone, its units removed (reloaded), its oneshot stopped and reset, then the volume.
    let calls = t.rig.fake.calls();
    let at = |c: &str| calls.iter().rposition(|x| x == c).unwrap_or_else(|| panic!("{c} in {calls:?}"));
    let oneshot = format!("{side}-volume.service");
    let (stop, reset, rm) =
        (at(&format!("stop {oneshot}")), at(&format!("reset-failed {oneshot}")), at(&format!("volume rm {side}")));
    assert!(at("reload") < stop && stop < reset && reset < rm, "{calls:?}");
    assert_eq!(
        calls.iter().filter(|c| **c == format!("volume rm {side}")).count(),
        2,
        "once in the test, once for good"
    );
}

#[tokio::test]
async fn a_public_streams_transport_follows_selkies() {
    let t = up(Opts::default()).await;
    let id = t.wadspace("Studio");
    t.stream(&id, StreamMode::Public).await;
    assert_eq!(t.info(&id).await.unwrap().transport, Transport::Webrtc);
    // A controller switched every viewer to WebSockets (dual mode), then back.
    let status = |mode: &str| Some(format!(r#"{{"current_mode": "{mode}", "enable_dual_mode": true}}"#));
    t.rig.fake.state.lock().unwrap().sidecar_status = status("websockets");
    t.s.tick().await;
    assert_eq!(t.info(&id).await.unwrap().transport, Transport::Websocket);
    t.rig.fake.state.lock().unwrap().sidecar_status = status("webrtc");
    t.s.tick().await;
    assert_eq!(t.info(&id).await.unwrap().transport, Transport::Webrtc);
}

#[tokio::test]
async fn stopping_a_stream_that_moved_puts_it_back_on_the_screen() {
    let t = up(Opts::default()).await;
    moving_base(&t).await;
    let id = t.wadspace("Studio");
    t.rig.runs.open(&id, SessionDisplay::Window).unwrap();
    t.rig.runs.settled(&id).await;
    let job = t.s.start(&id, StreamRequest { mode: StreamMode::Private, takeover: true, players: 0 }).await.unwrap();
    assert_eq!(t.rig.finished(&job.job.id).await, JobStatus::Done);
    assert_eq!(t.rig.runs.state(&id).display, Some(SessionDisplay::Stream));
    // Stopped (the shell's DELETE): back in its window as it runs, never stopped.
    t.s.stop(&id).await.unwrap();
    let r = t.rig.runs.state(&id);
    assert_eq!((r.state.phase, r.display), (Phase::Ready, Some(SessionDisplay::Window)));
    let calls = t.rig.fake.calls();
    let switch = format!("exec wad-{id} abc wadspaces-session switch --timeout 20");
    assert_eq!(calls.iter().filter(|c| **c == switch).count(), 2, "{calls:?}");
    assert!(!calls.contains(&format!("stop wad-{id}.service")) && t.running(&format!("wad-{id}")));
    assert!(!t.running(&format!("wadstream-{id}")) && t.unit(&format!("wadstream-{id}.container")).is_none());
    assert!(t.routes.guids().is_empty() && !t.secrets().iter().any(|s| s.starts_with("wad_stream_")));
    assert_eq!(read(&t.rig.state_dir().join("extra").join(&id).join("display")), "screen\n");
}

#[tokio::test]
async fn after_a_restart_a_stream_that_moved_still_goes_back_on_the_screen() {
    let t = up(Opts::default()).await;
    moving_base(&t).await;
    let id = t.wadspace("Studio");
    let switch = format!("exec wad-{id} abc wadspaces-session switch --timeout 20");
    let switches = |t: &T| t.rig.fake.calls().iter().filter(|c| **c == switch).count();
    let starts = |t: &T| t.rig.fake.calls().iter().filter(|c| **c == format!("start wad-{id}.service")).count();
    t.rig.runs.open(&id, SessionDisplay::Window).unwrap();
    t.rig.runs.settled(&id).await;
    let job = t.s.start(&id, StreamRequest { mode: StreamMode::Private, takeover: true, players: 0 }).await.unwrap();
    assert_eq!(t.rig.finished(&job.job.id).await, JobStatus::Done);
    assert_eq!(switches(&t), 1);

    // wadd restarts: new runs and streams, on the same state and podman.
    // Runs takes the wadspace on (as it was shown), streams adopts its stream.
    let runs = runs_again(&t.rig);
    let (again, routes) = streams_with(&t.rig, runs.clone(), &t.o, &t.cli, t.tailnet.clone());
    runs.reconcile().await;
    assert!(again.adopt().await);
    let r = runs.state(&id);
    assert_eq!((r.state.phase, r.display), (Phase::Ready, Some(SessionDisplay::Stream)));
    assert!(runs.moves(&id), "it can move back");
    assert!(!routes.guids().is_empty(), "its stream carries on");

    // Stopped: back in its window as it runs, never restarted.
    again.stop(&id).await.unwrap();
    let r = runs.state(&id);
    assert_eq!((r.state.phase, r.display), (Phase::Ready, Some(SessionDisplay::Window)), "{r:?}");
    assert_eq!((switches(&t), starts(&t)), (2, 1));
    assert!(t.running(&format!("wad-{id}")) && !t.running(&format!("wadstream-{id}")));
    assert!(routes.guids().is_empty() && !t.secrets().iter().any(|s| s.starts_with("wad_stream_")));
    assert_eq!(read(&t.rig.state_dir().join("extra").join(&id).join("display")), "screen\n");

    // Again, but adopted before runs takes it on, and its apps won't close
    // to move back: it stops, as before.
    let job = again.start(&id, StreamRequest { mode: StreamMode::Private, takeover: true, players: 0 }).await.unwrap();
    assert_eq!(t.rig.finished(&job.job.id).await, JobStatus::Done);
    let runs = runs_again(&t.rig);
    let (third, routes) = streams_with(&t.rig, runs.clone(), &t.o, &t.cli, t.tailnet.clone());
    assert!(third.adopt().await);
    runs.reconcile().await;
    assert!(runs.moves(&id) && runs.state(&id).display == Some(SessionDisplay::Stream));
    t.rig.fake.state.lock().unwrap().exec_answers =
        vec![("wadspaces-session switch".into(), 3, r#"{"still_open": [{"title": "Notes.txt"}]}"#.into())];
    third.stop(&id).await.unwrap();
    assert_eq!(runs.state(&id).state.phase, Phase::Idle);
    assert!(routes.guids().is_empty());
    assert!(t.rig.fake.calls().contains(&format!("stop wad-{id}.service")));
}
