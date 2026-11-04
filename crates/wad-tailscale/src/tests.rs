use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;

use axum::Router;
use axum::extract::Query;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;

use super::fixtures::*;
use super::*;

/// A tailscaled that answers from the fixtures; `status` is the one it reports.
async fn fake(status: &'static str) -> (tempfile::TempDir, LocalApi) {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("tailscaled.sock");
    let app = Router::new()
        .route(
            "/localapi/v0/status",
            get(move |h: HeaderMap| async move {
                assert_eq!(h["host"], HOST, "the LocalAPI wants its own Host");
                assert!(h.get("origin").is_none() && h.get("referer").is_none(), "it refuses browsers");
                status
            }),
        )
        .route("/localapi/v0/prefs", get(|| async { PREFS }))
        .route(
            "/localapi/v0/watch-ipn-bus",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                assert_eq!(q.get("mask").map(String::as_str), Some("2"));
                WATCH_IPN_BUS
            }),
        )
        .route(
            "/localapi/v0/whois",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                assert_eq!(q.get("proto").map(String::as_str), Some("tcp"));
                match q.get("addr").map(String::as_str) {
                    Some("100.64.0.7:50123") | Some("127.0.0.1:41000") => (StatusCode::OK, WHOIS_PHONE),
                    Some("100.64.0.9:50124") => (StatusCode::OK, WHOIS_TAGGED),
                    Some(_) => (StatusCode::NOT_FOUND, "no match for IP:port"),
                    None => (StatusCode::BAD_REQUEST, "missing 'addr' parameter"),
                }
            }),
        );
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (dir, LocalApi::new(sock))
}

#[tokio::test]
async fn reads_a_running_machine() {
    let (_d, api) = fake(STATUS_RUNNING).await;
    let s = api.status().await.unwrap();
    assert!(s.running());
    assert_eq!(s.ips(), ["100.101.102.103".parse::<IpAddr>().unwrap(), "fd7a:115c:a1e0::a01:2b03".parse().unwrap()]);
    assert_eq!(s.ipv4(), Some("100.101.102.103".parse().unwrap()));
    let me = s.me.as_ref().unwrap();
    assert_eq!((me.id.as_str(), me.dns()), ("nSurf1CNTRL", "wadspaces-surface.tail1234.ts.net"));
    assert_eq!(me.machine_name(), "wadspaces-surface");
    assert_eq!(me.key_expiry.as_deref(), Some("2027-04-16T03:00:00Z"));
    assert_eq!(s.me_user().unwrap().login_name, "owner@example.com");
    assert_eq!(s.tailnet_name(), "owner@example.com");
    assert_eq!(s.magic_dns_suffix, "tail1234.ts.net");
    let names: Vec<_> = s.peer_list().iter().map(|p| p.host_name.as_str()).collect();
    assert_eq!(names, ["build-box", "desk", "friends-laptop", "Pixel 9"]);
    let phone = s.peer_list().into_iter().find(|p| p.os == "android").unwrap();
    assert_eq!((phone.user_id, phone.online, phone.tags.is_empty()), (2001, true, true));
    assert_eq!(phone.machine_name(), "pixel-9");
    let server = s.peers.values().find(|p| p.host_name == "build-box").unwrap();
    assert_eq!(server.tags, ["tag:server"]);
    assert!(s.peers.values().any(|p| p.sharee_node));
    let desk = s.peer("nDesk1CNTRL").unwrap();
    assert_eq!((desk.ipv4(), desk.last_seen_ms()), (Some("100.64.0.5".parse().unwrap()), Some(1_792_296_600_500)));
    assert!(s.peer("nSurf1CNTRL").is_some(), "the machine itself");
}

#[tokio::test]
async fn reads_a_machine_waiting_to_log_in() {
    let (_d, api) = fake(STATUS_NEEDS_LOGIN).await;
    let s = api.status().await.unwrap();
    assert!(!s.running());
    assert_eq!(s.backend_state, "NeedsLogin");
    assert_eq!(s.auth_url, "https://login.tailscale.com/a/1a2b3c4d5e6f");
    // Go's nulls read as empty.
    assert!(s.ips().is_empty() && s.peers.is_empty() && s.users.is_empty() && s.tailnet.is_none());
    assert!(s.me_user().is_none());
    assert_eq!(s.tailnet_name(), "");
    assert_eq!(s.health, ["You are logged out."]);
}

#[tokio::test]
async fn reads_the_prefs_and_watches_the_bus() {
    let (_d, api) = fake(STATUS_RUNNING).await;
    let p = api.prefs().await.unwrap();
    assert_eq!((p.control_url.as_str(), p.want_running, p.operator_user.as_str()), ("https://hs.test:8443", true, ""));
    assert_eq!(p.drive_shares, [PrefsShare { name: "home".into(), path: "/var/home/sam".into(), who: "sam".into() }]);
    let mut bus = api.watch_ipn_bus(WATCH_INITIAL_STATE).await.unwrap();
    let mut seen = vec![];
    while let Some(n) = bus.next().await {
        let n = n.unwrap();
        seen.push((n.state.map(state_name), n.browse_to_url));
    }
    assert_eq!(
        seen,
        [
            (Some("NeedsLogin"), None),
            (None, Some("https://hs.test:8443/register/6CtCcbdJfP1KpU1y8zGtYa1T".into())),
            (Some("NeedsMachineAuth"), None),
            (Some("Starting"), None),
            (Some("Running"), None),
        ]
    );
}

#[tokio::test]
async fn says_which_device_a_connection_is_from() {
    let (_d, api) = fake(STATUS_RUNNING).await;
    let w = api.whois("100.64.0.7:50123".parse().unwrap()).await.unwrap().unwrap();
    assert_eq!(
        (w.node.stable_id.as_str(), w.node.user, w.user.login_name.as_str()),
        ("nPhone1CNTRL", 2001, "owner@example.com")
    );
    assert!(w.node.tags.is_empty());
    assert_eq!(w.device_name(), "Pixel 9");
    assert_eq!(w.node.hostinfo.os, "android");
    assert_eq!(w.node.hostinfo.device_model, "Google Pixel 9");
    // Userspace tailscaled: a forwarded connection's loopback port.
    assert!(api.whois("127.0.0.1:41000".parse().unwrap()).await.unwrap().is_some());
    let t = api.whois("100.64.0.9:50124".parse().unwrap()).await.unwrap().unwrap();
    assert_eq!(t.node.tags, ["tag:server"]);
    assert_eq!(t.user.login_name, "tagged-devices", "a tagged node's owner is the tags' pseudo-user");
    // Not a device on the tailnet.
    assert!(api.whois("192.168.1.5:40000".parse().unwrap()).await.unwrap().is_none());
    assert!(api.whois("[fd7a:115c:a1e0::99]:443".parse().unwrap()).await.unwrap().is_none());
}

#[test]
fn whois_fixtures_read() {
    let shared: WhoIs = serde_json::from_str(WHOIS_SHARED).unwrap();
    assert_eq!((shared.node.sharer, shared.node.user), (3001, 3001));
    let desk: WhoIs = serde_json::from_str(WHOIS_DESK).unwrap();
    assert_eq!((desk.machine_name().as_str(), desk.node.user, desk.node.sharer), ("desk", 2001, 0));
    // Headscale names whoever tagged it: the tags say it isn't theirs.
    let ci: WhoIs = serde_json::from_str(WHOIS_TAGGED_OWNER).unwrap();
    assert_eq!((ci.node.user, ci.node.tags.as_slice()), (2001, ["tag:ci".to_string()].as_slice()));
}

#[tokio::test]
async fn no_tailscaled() {
    let api = LocalApi::new("/nonexistent/tailscaled.sock");
    assert!(matches!(api.status().await, Err(Error::Unreachable(_))));
    assert!(api.watch_ipn_bus(WATCH_INITIAL_STATE).await.is_err());
}

#[test]
fn device_names_fall_back() {
    let mut w: WhoIs = serde_json::from_str(WHOIS_PHONE).unwrap();
    w.node.hostinfo.hostname.clear();
    assert_eq!(w.device_name(), "pixel-9");
    w.node.computed_name.clear();
    assert_eq!(w.device_name(), "pixel-9");
}

#[test]
fn finds_the_login_link() {
    let out = "\nTo authenticate, visit:\n\n\thttps://login.tailscale.com/a/5f6e7d\n\n";
    assert_eq!(url_in(out).as_deref(), Some("https://login.tailscale.com/a/5f6e7d"));
    assert_eq!(url_in("Success."), None);
    assert_eq!(url_in("https://"), None);
}

#[test]
fn reads_up_json_as_it_comes() {
    let text = Mutex::new(String::new());
    let mut buf = vec![];
    let mut events = vec![];
    // A byte at a time: nothing is read before it's whole.
    for b in UP_JSON.bytes() {
        buf.push(b);
        events.extend(take_up_json(&mut buf, &text));
    }
    buf.push(b'\n');
    events.extend(take_up_json(&mut buf, &text));
    assert_eq!(
        events,
        [
            UpEvent::AuthUrl("https://hs.test:8443/register/6CtCcbdJfP1KpU1y8zGtYa1T".into()),
            UpEvent::State("Running".into()),
        ]
    );
    assert!(text.lock().unwrap().contains("--accept-routes is false"));
    let mut err = br#"{"Error": "timeout waiting for the login"}"#.to_vec();
    assert_eq!(take_up_json(&mut err, &text), [UpEvent::Error("timeout waiting for the login".into())]);
}

#[test]
fn reads_the_drive_list() {
    let out = "name        path                         as\n\
               --------    -------------------------    ---\n\
               home        /var/home/sam                sam\n\
               my photos   /var/home/sam/Pictures       sam\n";
    assert_eq!(
        parse_drive_list(out),
        [
            Share { name: "home".into(), path: "/var/home/sam".into(), as_user: "sam".into() },
            Share { name: "my photos".into(), path: "/var/home/sam/Pictures".into(), as_user: "sam".into() },
        ]
    );
    assert!(parse_drive_list("name    path    as\n----    ----    --\n").is_empty());
    assert!(parse_drive_list("").is_empty());
}

#[test]
fn reads_times() {
    assert_eq!(rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(rfc3339_ms("2026-10-18T04:00:00Z"), Some(1_792_296_000_000));
    assert_eq!(rfc3339_ms("2026-10-18T16:00:00.25+12:00"), Some(1_792_296_000_250));
    assert_eq!(rfc3339_ms("2026-10-17T23:00:00.123456789-05:00"), Some(1_792_296_000_123));
    assert_eq!(rfc3339_ms("0001-01-01T00:00:00Z"), None, "Go's zero time: never");
    assert_eq!(rfc3339_ms("yesterday"), None);
}

/// Waits for an executable just written to be runnable (ETXTBSY while
/// another test thread forks).
fn settle(bin: &Path) {
    for _ in 0..50 {
        match std::process::Command::new(bin).arg("--probe").status() {
            Ok(_) => break,
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// A `tailscale` that writes its arguments down; `up` prints a login link
/// and waits a moment, `logout` fails with a message, `drive list` has one.
fn fake_cli(dir: &Path) -> (Cli, PathBuf) {
    let log = dir.join("args");
    let bin = dir.join("tailscale");
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\ncase \"$2\" in\n  up) printf '{{\\n\\t\"AuthURL\": \"https://login.example/a/77\"\\n}}\\n'; sleep 0.3; printf 'warning: something\\n{{\\n  \"BackendState\": \"Running\"\\n}}\\n' ;;\n  logout) echo 'not logged in' >&2; exit 1 ;;\n  drive) if [ \"$3\" = list ]; then printf 'name    path         as\\n----    ---------    ---\\nhome    /home/sam    sam\\n'; fi ;;\nesac\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    settle(&bin);
    let _ = std::fs::remove_file(&log);
    (Cli::new(&bin, "/run/ts.sock"), log)
}

#[tokio::test]
async fn joins_and_leaves_with_the_cli() {
    let dir = tempfile::tempdir().unwrap();
    let (cli, log) = fake_cli(dir.path());
    let opts = UpOptions {
        hostname: Some("wadspaces-surface".into()),
        login_server: Some("https://hs.test:8443".into()),
        timeout: Duration::from_secs(600),
    };
    let mut login = cli.up(&opts).unwrap();
    assert_eq!(login.next().await, Some(UpEvent::AuthUrl("https://login.example/a/77".into())));
    assert_eq!(login.next().await, Some(UpEvent::State("Running".into())));
    assert_eq!(login.next().await, None);
    login.wait().await.unwrap();
    let e = cli.logout().await.unwrap_err();
    assert_eq!(e.to_string(), "not logged in");
    cli.set_hostname("surface-2").await.unwrap();
    let args = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        [
            "--socket=/run/ts.sock up --json --reset --timeout=600s --hostname=wadspaces-surface --login-server=https://hs.test:8443",
            "--socket=/run/ts.sock logout",
            "--socket=/run/ts.sock set --hostname=surface-2",
        ]
    );
}

#[tokio::test]
async fn a_login_stopped_is_killed() {
    let dir = tempfile::tempdir().unwrap();
    let (cli, _) = fake_cli(dir.path());
    let mut login = cli.up(&UpOptions { timeout: Duration::from_secs(5), ..Default::default() }).unwrap();
    assert!(matches!(login.next().await, Some(UpEvent::AuthUrl(_))));
    login.cancel().await;
}

#[tokio::test]
async fn taildrive_runs_as_the_person() {
    let dir = tempfile::tempdir().unwrap();
    let (cli, log) = fake_cli(dir.path());
    // A runuser that says how it was asked, then runs the command (as us).
    let runuser = dir.path().join("runuser");
    std::fs::write(
        &runuser,
        format!(
            "#!/bin/sh\n[ \"$1\" = --probe ] && exit 0\necho \"runuser $1 $2 $3\" >> '{}'\nshift 3\nexec \"$@\"\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&runuser, std::fs::Permissions::from_mode(0o755)).unwrap();
    settle(&runuser);
    let sam = cli.with_runuser(&runuser).as_user("sam");
    sam.drive_share("home", Path::new("/home/sam")).await.unwrap();
    assert_eq!(
        sam.drive_list().await.unwrap(),
        [Share { name: "home".into(), path: "/home/sam".into(), as_user: "sam".into() }]
    );
    sam.drive_unshare("home").await.unwrap();
    let args = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        [
            "runuser -u sam --",
            "--socket=/run/ts.sock drive share home /home/sam",
            "runuser -u sam --",
            "--socket=/run/ts.sock drive list",
            "runuser -u sam --",
            "--socket=/run/ts.sock drive unshare home",
        ]
    );
}

#[test]
fn drive_refusals_read_as_the_policys() {
    let e = Error::Cli(
        "taildrive sharing not enabled, please add the attribute \"drive:share\" to this node in your ACLs' \"nodeAttrs\" section".into(),
    );
    assert!(e.drive_not_allowed());
    assert!(!Error::Cli("no such file".into()).drive_not_allowed());
}

#[tokio::test]
async fn a_cli_that_isnt_there() {
    let cli = Cli::new("/nonexistent/tailscale", "/run/ts.sock");
    assert!(matches!(cli.up(&UpOptions::default()), Err(Error::Cli(_))));
    assert!(cli.logout().await.is_err());
}

#[test]
fn the_ports_served() {
    let json = r#"{"TCP": {"48301": {"HTTPS": true}, "443": {"HTTPS": true}},
                   "Web": {"desk.tail1234.ts.net:48301": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:48301"}}}}}"#;
    assert_eq!(served_ports(json), [443, 48301]);
    assert!(served_ports("{}").is_empty());
    assert!(served_ports("").is_empty());
}

#[cfg(feature = "fake")]
mod fakes {
    use super::*;
    use crate::fake::{FakeNode, FakeTailnet, fake_cli};

    #[tokio::test]
    async fn the_fake_tailnet_answers_as_each_node() {
        let dir = tempfile::tempdir().unwrap();
        let net = FakeTailnet::new("owner@example.com", "ts.test");
        net.user(1, "owner@example.com", "Owner").user(9, "tagged-devices", "Tagged Devices");
        net.node(FakeNode::new("nA", "desk", 1, "127.0.0.1"))
            .node(FakeNode::new("nB", "laptop", 1, "127.0.0.2").offline())
            .node(FakeNode::new("nT", "ci", 9, "127.0.0.4").tagged("tag:ci"));
        let sock = dir.path().join("a.sock");
        let _served = net.serve("nA", &sock).unwrap();
        let api = LocalApi::new(&sock);
        let s = api.status().await.unwrap();
        assert!(s.running());
        assert_eq!(s.me.as_ref().unwrap().id, "nA");
        assert_eq!(
            (s.tailnet_name(), s.me_user().unwrap().login_name.as_str()),
            ("owner@example.com", "owner@example.com")
        );
        let b = s.peer("nB").unwrap();
        assert_eq!((b.online, b.machine_name().as_str(), b.dns()), (false, "laptop", "laptop.ts.test"));
        net.set_online("nB", true);
        assert!(api.status().await.unwrap().peer("nB").unwrap().online);
        let w = api.whois("127.0.0.4:5000".parse().unwrap()).await.unwrap().unwrap();
        assert_eq!((w.node.stable_id.as_str(), w.node.tags.len()), ("nT", 1));
        assert!(api.whois("127.0.0.9:5000".parse().unwrap()).await.unwrap().is_none());
        assert_eq!(net.whois_calls(), 2);
        net.set_state("nA", "NeedsLogin", "https://hs.test:8443/register/x");
        let s = api.status().await.unwrap();
        assert_eq!((s.backend_state.as_str(), s.peers.len(), s.ips().len()), ("NeedsLogin", 0, 0));
        let mut bus = api.watch_ipn_bus(WATCH_INITIAL_STATE).await.unwrap();
        assert_eq!(bus.next().await.unwrap().unwrap().state, Some(2));
        let n = bus.next().await.unwrap().unwrap();
        assert_eq!(n.browse_to_url.as_deref(), Some("https://hs.test:8443/register/x"));
    }

    #[tokio::test]
    async fn the_fake_cli_logs_in_once_approved_and_shares() {
        let dir = tempfile::tempdir().unwrap();
        let fake = fake_cli(dir.path());
        let cli = Cli::new(&fake.bin, "/run/ts.sock");
        let mut login = cli.up(&UpOptions { timeout: Duration::from_secs(5), ..Default::default() }).unwrap();
        assert_eq!(login.next().await, Some(UpEvent::AuthUrl(crate::fake::FAKE_AUTH_URL.into())));
        fake.approve();
        assert_eq!(login.next().await, Some(UpEvent::State("Running".into())));
        login.wait().await.unwrap();
        cli.drive_share("home", Path::new("/home/sam")).await.unwrap();
        let shares = cli.drive_list().await.unwrap();
        assert_eq!((shares.len(), shares[0].path.as_str()), (1, "/home/sam"));
        fake.refuse_drive(true);
        assert!(cli.drive_share("home", Path::new("/home/sam")).await.unwrap_err().drive_not_allowed());
        assert!(fake.runs()[0].contains(" up --json "));
    }
}
