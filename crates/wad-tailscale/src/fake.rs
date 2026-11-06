//! A fake tailscaled and a fake `tailscale` CLI, for tests (wadd's, and the
//! harness's if it wants them): never the real tailnet.
//!
//! [`FakeTailnet`] is a tailnet the test describes (its people, its nodes,
//! who's online) and changes as it goes; [`FakeTailnet::serve`] answers
//! tailscaled's LocalAPI on a Unix socket as one of its nodes would (status,
//! whois, prefs, watch-ipn-bus), refusing what the real one refuses (a wrong
//! Host, an Origin). Several nodes can be served at once, each its own
//! socket, over the same tailnet: two wadds on 127.0.0.1 and 127.0.0.2 see
//! each other as peers, and a connection from 127.0.0.3 is whichever node
//! has that address.
//!
//! [`fake_cli`] writes a `tailscale` that logs its arguments and acts out
//! `up --json` (a login link, then Running once the test approves it),
//! `logout`, `set`, and Taildrive's `drive share|unshare|list`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::HOST;

/// A node on the fake tailnet.
#[derive(Debug, Clone)]
pub struct FakeNode {
    /// Its stable node id.
    pub id: String,
    /// Its machine name (its MagicDNS name's first label).
    pub name: String,
    pub os: String,
    pub model: String,
    /// Whose it is (a [`FakeTailnet::user`]'s id). A tagged node's is the
    /// tags' pseudo-user, or (Headscale) whoever tagged it.
    pub user: i64,
    pub ips: Vec<String>,
    pub tags: Vec<String>,
    /// Who shared it in from another tailnet (0: nobody).
    pub sharer: i64,
    pub online: bool,
    /// RFC 3339.
    pub last_seen: String,
    /// It offers to be an exit node (`--advertise-exit-node`, approved).
    pub exit_option: bool,
}

impl FakeNode {
    /// An online Linux node of `user`'s at `ip`.
    pub fn new(id: &str, name: &str, user: i64, ip: &str) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            os: "linux".into(),
            model: String::new(),
            user,
            ips: vec![ip.into()],
            tags: vec![],
            sharer: 0,
            online: true,
            last_seen: "2026-11-01T10:00:00Z".into(),
            exit_option: false,
        }
    }

    /// It offers to be an exit node.
    pub fn exit_node_option(mut self) -> Self {
        self.exit_option = true;
        self
    }

    pub fn os(mut self, os: &str, model: &str) -> Self {
        self.os = os.into();
        self.model = model.into();
        self
    }

    pub fn tagged(mut self, tag: &str) -> Self {
        self.tags.push(tag.into());
        self
    }

    pub fn shared_by(mut self, sharer: i64) -> Self {
        self.sharer = sharer;
        self
    }

    pub fn offline(mut self) -> Self {
        self.online = false;
        self
    }
}

#[derive(Default)]
struct Net {
    /// The tailnet's name (CurrentTailnet.Name) and its MagicDNS suffix.
    name: String,
    suffix: String,
    users: Vec<(i64, String, String)>,
    nodes: Vec<FakeNode>,
    /// Each served node's BackendState and AuthURL (none: Running).
    states: HashMap<String, (String, String)>,
    whois_calls: u64,
    /// The exit node in use (a node's id).
    exit_node: Option<String>,
}

/// A tailnet for tests: cheap to clone, one tailnet shared.
#[derive(Clone, Default)]
pub struct FakeTailnet(Arc<Mutex<Net>>);

impl FakeTailnet {
    /// A tailnet called `name` ('owner@example.com'), its MagicDNS names under `suffix`.
    pub fn new(name: &str, suffix: &str) -> Self {
        Self(Arc::new(Mutex::new(Net { name: name.into(), suffix: suffix.into(), ..Default::default() })))
    }

    /// A person (or Tailscale's "tagged-devices" pseudo-user).
    pub fn user(&self, id: i64, login: &str, display: &str) -> &Self {
        self.0.lock().unwrap().users.push((id, login.into(), display.into()));
        self
    }

    pub fn node(&self, n: FakeNode) -> &Self {
        let mut net = self.0.lock().unwrap();
        net.nodes.retain(|x| x.id != n.id);
        net.nodes.push(n);
        drop(net);
        self
    }

    pub fn set_online(&self, id: &str, online: bool) {
        if let Some(n) = self.0.lock().unwrap().nodes.iter_mut().find(|n| n.id == id) {
            n.online = online;
        }
    }

    /// What `me`'s tailscaled says it is ('NeedsLogin' with its link, 'Running'…).
    pub fn set_state(&self, me: &str, state: &str, auth_url: &str) {
        self.0.lock().unwrap().states.insert(me.into(), (state.into(), auth_url.into()));
    }

    /// How many whois questions were asked (of any node's tailscaled).
    /// The exit node in use, as `tailscale set --exit-node` leaves it.
    pub fn set_exit_node(&self, id: Option<&str>) {
        self.0.lock().unwrap().exit_node = id.map(String::from);
    }

    pub fn whois_calls(&self) -> u64 {
        self.0.lock().unwrap().whois_calls
    }

    /// Serves tailscaled's LocalAPI as node `me` sees the tailnet, on a Unix socket at `sock`.
    pub fn serve(&self, me: &str, sock: &Path) -> std::io::Result<tokio::task::JoinHandle<()>> {
        let _ = std::fs::remove_file(sock);
        let listener = tokio::net::UnixListener::bind(sock)?;
        let app = Router::new()
            .route("/localapi/v0/status", get(status))
            .route("/localapi/v0/whois", get(whois))
            .route("/localapi/v0/prefs", get(prefs))
            .route("/localapi/v0/watch-ipn-bus", get(watch))
            .with_state(Served { net: self.clone(), me: me.into() });
        Ok(tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        }))
    }

    fn state_of(&self, me: &str) -> (String, String) {
        self.0.lock().unwrap().states.get(me).cloned().unwrap_or_else(|| ("Running".into(), String::new()))
    }
}

#[derive(Clone)]
struct Served {
    net: FakeTailnet,
    me: String,
}

/// The real LocalAPI refuses a wrong Host and anything a browser sends.
fn refused(h: &HeaderMap) -> Option<Response> {
    let host = h.get("host").and_then(|v| v.to_str().ok());
    if host != Some(HOST) || h.get("origin").is_some() || h.get("referer").is_some() {
        return Some((StatusCode::FORBIDDEN, "invalid localapi request").into_response());
    }
    None
}

fn peer_json(net: &Net, n: &FakeNode) -> Value {
    json!({
        "ID": n.id, "HostName": n.name, "DNSName": format!("{}.{}.", n.name, net.suffix), "OS": n.os,
        "UserID": n.user, "TailscaleIPs": n.ips, "Tags": if n.tags.is_empty() { Value::Null } else { json!(n.tags) },
        "Online": n.online, "LastSeen": n.last_seen, "ShareeNode": n.sharer != 0, "Expired": false,
        "ExitNodeOption": n.exit_option, "ExitNode": net.exit_node.as_deref() == Some(n.id.as_str()),
        "KeyExpiry": "2027-05-01T00:00:00Z",
    })
}

async fn status(State(s): State<Served>, h: HeaderMap) -> Response {
    if let Some(r) = refused(&h) {
        return r;
    }
    let (state, auth_url) = s.net.state_of(&s.me);
    let net = s.net.0.lock().unwrap();
    let running = state == "Running";
    let me = net.nodes.iter().find(|n| n.id == s.me).cloned().unwrap_or_else(|| FakeNode::new(&s.me, &s.me, 0, ""));
    let mut this = peer_json(&net, &me);
    if !running {
        this["TailscaleIPs"] = Value::Null;
        this["UserID"] = json!(0);
    }
    let peers: serde_json::Map<String, Value> = if running {
        net.nodes.iter().filter(|n| n.id != s.me).map(|n| (format!("nodekey:{}", n.id), peer_json(&net, n))).collect()
    } else {
        Default::default()
    };
    let users: serde_json::Map<String, Value> = net
        .users
        .iter()
        .map(|(id, login, display)| (id.to_string(), json!({"ID": id, "LoginName": login, "DisplayName": display})))
        .collect();
    Json(json!({
        "Version": "1.104.5-fake",
        "BackendState": state,
        "AuthURL": auth_url,
        "TailscaleIPs": if running { json!(me.ips) } else { Value::Null },
        "Self": this,
        "Health": Value::Null,
        "MagicDNSSuffix": if running { net.suffix.clone() } else { String::new() },
        "CurrentTailnet": if running {
            json!({"Name": net.name, "MagicDNSSuffix": net.suffix, "MagicDNSEnabled": true})
        } else { Value::Null },
        "Peer": if running { Value::Object(peers) } else { Value::Null },
        "User": if running { Value::Object(users) } else { Value::Null },
        "ExitNodeStatus": match net.exit_node.as_ref().and_then(|id| net.nodes.iter().find(|n| &n.id == id)) {
            Some(n) if running => json!({"ID": n.id, "Online": n.online, "TailscaleIPs": n.ips}),
            _ => Value::Null,
        },
    }))
    .into_response()
}

async fn whois(State(s): State<Served>, h: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Some(r) = refused(&h) {
        return r;
    }
    let mut net = s.net.0.lock().unwrap();
    net.whois_calls += 1;
    let Some(addr) = q.get("addr") else {
        return (StatusCode::BAD_REQUEST, "missing 'addr' parameter").into_response();
    };
    let ip = match addr.parse::<SocketAddr>() {
        Ok(a) => a.ip().to_string(),
        Err(_) => addr.clone(),
    };
    let Some(n) = net.nodes.iter().find(|n| n.ips.contains(&ip)).cloned() else {
        return (StatusCode::NOT_FOUND, "no match for IP:port").into_response();
    };
    let user =
        net.users.iter().find(|(id, ..)| *id == n.user).cloned().unwrap_or((n.user, String::new(), String::new()));
    Json(json!({
        "Node": {
            "StableID": n.id, "Name": format!("{}.{}.", n.name, net.suffix), "User": n.user, "Sharer": n.sharer,
            "Tags": if n.tags.is_empty() { Value::Null } else { json!(n.tags) },
            "Addresses": n.ips.iter().map(|i| format!("{i}/32")).collect::<Vec<_>>(),
            "Hostinfo": {"OS": n.os, "Hostname": n.name, "DeviceModel": n.model},
            "Online": n.online, "MachineAuthorized": true, "ComputedName": n.name,
        },
        "UserProfile": {"ID": user.0, "LoginName": user.1, "DisplayName": user.2},
        "CapMap": Value::Null,
    }))
    .into_response()
}

async fn prefs(State(s): State<Served>, h: HeaderMap) -> Response {
    if let Some(r) = refused(&h) {
        return r;
    }
    let (state, _) = s.net.state_of(&s.me);
    Json(json!({
        "ControlURL": "https://hs.test:8443", "WantRunning": state == "Running", "LoggedOut": state == "NeedsLogin",
        "Hostname": s.me, "OperatorUser": "", "ExitNodeID": "", "DriveShares": Value::Null,
    }))
    .into_response()
}

/// The state now, then it closes (a watcher reads again).
async fn watch(State(s): State<Served>, h: HeaderMap) -> Response {
    if let Some(r) = refused(&h) {
        return r;
    }
    let (state, url) = s.net.state_of(&s.me);
    let n = match state.as_str() {
        "NeedsLogin" => 2,
        "NeedsMachineAuth" => 3,
        "Stopped" => 4,
        "Starting" => 5,
        "Running" => 6,
        _ => 0,
    };
    let mut body = format!("{}\n", json!({"State": n}));
    if !url.is_empty() {
        body.push_str(&format!("{}\n", json!({"BrowseToURL": url})));
    }
    ([("content-type", "application/json")], body).into_response()
}

/// A fake `tailscale` ([`fake_cli`]) and what it leaves.
pub struct FakeCli {
    pub bin: PathBuf,
    /// Each run's arguments, a line each.
    pub log: PathBuf,
    dir: PathBuf,
}

/// The login link the fake `up` gives.
pub const FAKE_AUTH_URL: &str = "https://hs.test:8443/register/fake-auth-id";

impl FakeCli {
    /// Its runs' arguments so far.
    pub fn runs(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log).unwrap_or_default().lines().map(String::from).collect()
    }

    /// The waiting `up` goes on: the sign-in happened.
    pub fn approve(&self) {
        std::fs::write(self.dir.join("approved"), "").unwrap();
    }

    /// `drive share` is refused as a tailnet without `drive:share` refuses it.
    pub fn refuse_drive(&self, refuse: bool) {
        let f = self.dir.join("drive-refused");
        if refuse {
            std::fs::write(f, "").unwrap();
        } else {
            let _ = std::fs::remove_file(f);
        }
    }

    /// The pid of the `up` running now (or last).
    pub fn up_pid(&self) -> Option<u32> {
        std::fs::read_to_string(self.dir.join("up.pid")).ok()?.trim().parse().ok()
    }
}

/// Writes a fake `tailscale` into `dir`. `up` prints [`FAKE_AUTH_URL`] (as
/// `up --json` does), waits for [`FakeCli::approve`] (20 s at most), then
/// says Running; `drive` keeps its shares in a file and prints them as the
/// real one's table.
pub fn fake_cli(dir: &Path) -> FakeCli {
    std::fs::create_dir_all(dir).unwrap();
    let log = dir.join("args");
    let bin = dir.join("tailscale");
    let d = dir.display();
    let script = format!(
        r#"#!/bin/sh
echo "$@" >> '{d}/args'
case "$2" in
  up)
    echo $$ > '{d}/up.pid'
    printf '{{\n\t"AuthURL": "{url}",\n\t"BackendState": "NeedsLogin"\n}}\n'
    i=0
    while [ ! -e '{d}/approved' ]; do
      sleep 0.05; i=$((i+1))
      if [ $i -gt 400 ]; then echo 'timeout waiting for the login' >&2; exit 1; fi
    done
    printf '{{\n  "BackendState": "Running"\n}}\n' ;;
  logout|set) ;;
  drive)
    case "$3" in
      share)
        if [ -e '{d}/drive-refused' ]; then
          echo 'taildrive sharing not enabled, please add the attribute "drive:share" to this node in your ACLs'"'"' "nodeAttrs" section' >&2
          exit 1
        fi
        grep -v "^$4 " '{d}/shares' > '{d}/shares.new' 2> /dev/null; mv '{d}/shares.new' '{d}/shares'
        echo "$4 $5 $(id -un)" >> '{d}/shares'
        echo "Sharing \"$5\" as \"$4\"" ;;
      unshare)
        grep -v "^$4 " '{d}/shares' > '{d}/shares.new' 2> /dev/null; mv '{d}/shares.new' '{d}/shares' ;;
      list)
        printf '%-8s    %-40s    %s\n' name path as
        printf '%-8s    %-40s    %s\n' -------- ---------------------------------------- --
        if [ -e '{d}/shares' ]; then
          while read -r n p a; do printf '%-8s    %-40s    %s\n' "$n" "$p" "$a"; done < '{d}/shares'
        fi ;;
    esac ;;
esac
"#,
        url = FAKE_AUTH_URL,
    );
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    // A file just written can still be "busy" (ETXTBSY) for a moment if
    // another test thread forked meanwhile: run it once before the test does.
    for _ in 0..50 {
        match std::process::Command::new(&bin).arg("--probe").status() {
            Ok(_) => break,
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let _ = std::fs::remove_file(&log);
    FakeCli { bin, log, dir: dir.to_path_buf() }
}
