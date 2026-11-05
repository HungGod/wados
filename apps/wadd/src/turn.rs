//! Cloudflare's TURN, for public streams: wadd mints each stream's ICE
//! servers from the machine's TURN key and hands them to its sidecar in a
//! file (`SELKIES_RTC_CONFIG_JSON`, [`wad_gen::units::RTC_FILE`]), so the key
//! itself never enters a container.
//!
//! - **The key:** `[streaming] turn` (`/etc/wados/turn.toml`, 0600, the
//!   installer stick's), `[cloudflare.turn] token_id, api_token`.
//! - **Minting:** `POST <turn_api>/turn/keys/<token_id>/credentials/generate-ice-servers`
//!   with `{"ttl": <turn_ttl_s>}` and `Authorization: Bearer <api_token>`;
//!   Cloudflare answers 201 `{"iceServers": [...]}` (STUN, and TURN with a
//!   username and credential). URLs on port 53 are dropped: browsers block
//!   that port, and an ICE server there only times out.
//! - **How long they last** ([`Turn::refresh_at`]): Cloudflare disconnects
//!   a TURN session shortly after its credentials expire (48 hours at most),
//!   and Selkies' page only reads its ICE servers when it connects. So a
//!   stream's credentials are minted long (`turn_ttl_s`, 24 hours by
//!   default), the file is rewritten with new ones once three quarters of
//!   that has gone (a viewer who connects or reconnects then gets them), and
//!   every credential minted for a stream is revoked when the stream ends
//!   ([`Turn::revoke`]): a viewer can't keep relaying through the
//!   machine's account after the stream is over.
//! - **When TURN is missing:** no key, or Cloudflare not answering in time:
//!   STUN alone ([`stun_only`]), and the stream says so (`turnDown`,
//!   [`wad_proto::streams::TURN_DOWN`]); it still starts.
//!
//! Neither the API token nor a credential is ever logged: errors say what
//! failed, not with what.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use crate::jobs::now_ms;

/// Cloudflare's STUN, which needs no key (what a stream has without TURN).
pub const STUN: &str = "stun:stun.cloudflare.com:3478";
/// How long Cloudflare has to answer.
const TIMEOUT: Duration = Duration::from_secs(10);
/// The least a refresh leaves before the credentials run out.
const MARGIN_MS: i64 = 60_000;

#[derive(Debug, Clone)]
pub struct Settings {
    /// `[streaming] turn`: the key's file.
    pub file: PathBuf,
    /// `[streaming] turn_api`.
    pub api: String,
    /// `[streaming] turn_ttl_s`.
    pub ttl: Duration,
    /// How long Cloudflare has to answer.
    pub timeout: Duration,
}

impl Settings {
    pub fn new(config: &wad_config::Config) -> Self {
        let s = &config.streaming;
        Self {
            file: s.turn.clone(),
            api: s.turn_api.trim_end_matches('/').to_string(),
            ttl: Duration::from_secs(s.turn_ttl_s.max(60)),
            timeout: TIMEOUT,
        }
    }
}

/// The machine's TURN key. Its `Debug` leaves the token out.
struct Key {
    token_id: String,
    api_token: String,
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Key").finish_non_exhaustive()
    }
}

/// A stream's ICE servers, as minted. Its `Debug` leaves the credentials out.
#[derive(Clone, PartialEq)]
pub struct Ice {
    /// `iceServers`: Cloudflare's, without port 53.
    pub servers: Value,
    /// The TURN usernames in them (to revoke when the stream ends).
    pub users: Vec<String>,
    /// When they were minted and when they run out (ms since the epoch).
    pub minted: i64,
    pub expires: i64,
}

impl fmt::Debug for Ice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ice").field("minted", &self.minted).field("expires", &self.expires).finish_non_exhaustive()
    }
}

/// Cloudflare's TURN API, with the machine's key.
pub struct Turn {
    s: Settings,
    http: reqwest::Client,
}

impl Turn {
    pub fn new(s: Settings) -> Self {
        let http = reqwest::Client::builder().timeout(s.timeout).build().expect("a client");
        Self { s, http }
    }

    pub fn settings(&self) -> &Settings {
        &self.s
    }

    /// The key, from its file: why not, if it can't be read.
    fn key(&self) -> Result<Key, String> {
        let path = &self.s.file;
        let text = std::fs::read_to_string(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("there's no TURN key on this machine ({})", path.display()),
            _ => format!("{}: {e}", path.display()),
        })?;
        // Not the parser's message: it quotes the line it choked on.
        let t: toml::Table = text.parse().map_err(|_| format!("{} isn't TOML", path.display()))?;
        let turn = t.get("cloudflare").and_then(|c| c.get("turn"));
        let field = |k: &str| turn.and_then(|t| t.get(k)).and_then(toml::Value::as_str).map(str::trim);
        match (field("token_id"), field("api_token")) {
            (Some(id), Some(token)) if !id.is_empty() && !token.is_empty() => {
                if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
                    return Err(format!("{}: its token_id isn't a key id", path.display()));
                }
                Ok(Key { token_id: id.to_string(), api_token: token.to_string() })
            }
            _ => Err(format!("{} has no [cloudflare.turn] token_id and api_token", path.display())),
        }
    }

    /// New ICE servers for one stream, lasting `ttl`.
    pub async fn mint(&self) -> Result<Ice, String> {
        let key = self.key()?;
        let url = format!("{}/turn/keys/{}/credentials/generate-ice-servers", self.s.api, key.token_id);
        let ttl = self.s.ttl.as_secs();
        let minted = now_ms();
        let res = self
            .http
            .post(url)
            .bearer_auth(&key.api_token)
            .json(&json!({ "ttl": ttl }))
            .send()
            .await
            .map_err(|e| format!("Cloudflare's TURN didn't answer: {}", plain(&e)))?;
        let status = res.status();
        if !status.is_success() {
            return Err(match status.as_u16() {
                401 | 403 => format!("Cloudflare's TURN refused this machine's key (HTTP {status})"),
                _ => format!("Cloudflare's TURN answered HTTP {status}"),
            });
        }
        let body: Value = res.json().await.map_err(|e| format!("Cloudflare's TURN answer: {}", plain(&e)))?;
        let servers = match body.get("iceServers") {
            Some(Value::Array(a)) => Value::Array(a.clone()),
            // The older answer: one server, not a list.
            Some(o @ Value::Object(_)) => Value::Array(vec![o.clone()]),
            _ => return Err("Cloudflare's TURN answered without iceServers".into()),
        };
        let servers = without_port_53(servers);
        let users = turn_users(&servers);
        if users.is_empty() {
            return Err("Cloudflare's TURN answered without a TURN server".into());
        }
        Ok(Ice { servers, users, minted, expires: minted + (ttl as i64) * 1000 })
    }

    /// Revokes these credentials (the stream ended): Cloudflare cuts what
    /// still relays with them. Best effort: what fails is logged, then left
    /// to run out.
    pub async fn revoke(&self, users: &[String]) {
        if users.is_empty() {
            return;
        }
        let key = match self.key() {
            Ok(k) => k,
            Err(e) => {
                tracing::info!("TURN: not revoking {} credentials: {e}", users.len());
                return;
            }
        };
        for user in users {
            if user.is_empty() || !user.bytes().all(|b| b.is_ascii_graphic() && b != b'/' && b != b'?' && b != b'#') {
                continue;
            }
            let url = format!("{}/turn/keys/{}/credentials/{user}/revoke", self.s.api, key.token_id);
            match self.http.post(url).bearer_auth(&key.api_token).send().await {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => tracing::info!("TURN: revoking a stream's credentials: HTTP {}", r.status()),
                Err(e) => tracing::info!("TURN: revoking a stream's credentials: {}", plain(&e)),
            }
        }
    }

    /// When to mint again: three quarters of the way to `expires`, and a
    /// minute before at the latest.
    pub fn refresh_at(&self, ice: &Ice) -> i64 {
        let life = (ice.expires - ice.minted).max(0);
        (ice.minted + life * 3 / 4).min(ice.expires - MARGIN_MS).max(ice.minted)
    }
}

/// A request's failure without its URL (it names the key's id).
fn plain(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return "it timed out".into();
    }
    if e.is_connect() {
        return "it couldn't be reached".into();
    }
    let mut s = e.to_string();
    if let Some(u) = e.url() {
        s = s.replace(u.as_str(), "<turn api>");
    }
    s
}

/// The ICE servers without a URL on port 53 (browsers block it); a server
/// left with no URL goes.
pub fn without_port_53(servers: Value) -> Value {
    let Value::Array(list) = servers else { return Value::Array(vec![]) };
    let kept = list
        .into_iter()
        .filter_map(|mut s| {
            let urls: Vec<Value> = match s.get("urls") {
                Some(Value::String(u)) => vec![Value::String(u.clone())],
                Some(Value::Array(a)) => a.clone(),
                _ => vec![],
            };
            let urls: Vec<Value> =
                urls.into_iter().filter(|u| u.as_str().is_some_and(|u| port_of(u) != Some(53))).collect();
            if urls.is_empty() {
                return None;
            }
            s["urls"] = Value::Array(urls);
            Some(s)
        })
        .collect();
    Value::Array(kept)
}

/// An ICE URL's port: `stun:host:port`, `turn:host:port?transport=udp`, `turns:[v6]:port`.
fn port_of(url: &str) -> Option<u16> {
    let rest = url.split_once(':')?.1;
    let rest = rest.split('?').next()?;
    let port = if rest.starts_with('[') { rest.rsplit_once("]:")?.1 } else { rest.rsplit_once(':')?.1 };
    port.parse().ok()
}

/// The TURN usernames among these servers.
fn turn_users(servers: &Value) -> Vec<String> {
    let mut out = vec![];
    for s in servers.as_array().into_iter().flatten() {
        let turn =
            s["urls"].as_array().into_iter().flatten().any(|u| u.as_str().is_some_and(|u| u.starts_with("turn")));
        if let (true, Some(u)) = (turn, s.get("username").and_then(Value::as_str))
            && !out.iter().any(|o| o == u)
        {
            out.push(u.to_string());
        }
    }
    out
}

/// What a stream has without TURN: Cloudflare's STUN alone.
pub fn stun_only() -> Value {
    json!([{ "urls": [STUN] }])
}

/// The sidecar's file: `{"iceServers": [...]}`.
pub fn rtc_json(servers: &Value) -> Vec<u8> {
    let mut b = serde_json::to_vec_pretty(&json!({ "iceServers": servers })).expect("json");
    b.push(b'\n');
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TOKEN: &str = "api-token-not-for-logs";

    fn cloudflares() -> Value {
        json!({"iceServers": [
            {"urls": ["stun:stun.cloudflare.com:3478", "stun:stun.cloudflare.com:53"]},
            {"urls": [
                "turn:turn.cloudflare.com:3478?transport=udp",
                "turn:turn.cloudflare.com:53?transport=udp",
                "turn:turn.cloudflare.com:3478?transport=tcp",
                "turns:turn.cloudflare.com:5349?transport=tcp",
                "turn:turn.cloudflare.com:80?transport=tcp",
                "turns:turn.cloudflare.com:443?transport=tcp"
            ], "username": "u-1", "credential": "credential-not-for-logs"}
        ]})
    }

    fn turn(dir: &std::path::Path, api: &str, key: Option<&str>) -> Turn {
        let file = dir.join("turn.toml");
        if let Some(k) = key {
            std::fs::write(&file, k).unwrap();
        }
        Turn::new(Settings {
            file,
            api: api.into(),
            ttl: Duration::from_secs(600),
            timeout: Duration::from_millis(500),
        })
    }

    fn key() -> String {
        format!("[cloudflare.turn]\ntoken_id = \"key-1\"\napi_token = \"{TOKEN}\"\n")
    }

    #[test]
    fn port_53_goes_and_the_rest_stays() {
        let s = without_port_53(cloudflares()["iceServers"].clone());
        let urls: Vec<&str> = s
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|s| s["urls"].as_array().unwrap())
            .filter_map(Value::as_str)
            .collect();
        assert!(!urls.iter().any(|u| port_of(u) == Some(53)), "{urls:?}");
        assert_eq!(urls.len(), 6);
        assert_eq!(s[1]["username"], "u-1");
        assert_eq!(port_of("turns:[2606:4700::1]:5349?transport=tcp"), Some(5349));
        assert_eq!(port_of("stun:stun.cloudflare.com:53"), Some(53));
        // A server left with nothing, gone; a lone URL as a string, read.
        let s = without_port_53(json!([{"urls": "stun:x:53"}, {"urls": "stun:x:3478"}]));
        assert_eq!(s, json!([{"urls": ["stun:x:3478"]}]));
        assert_eq!(turn_users(&without_port_53(cloudflares()["iceServers"].clone())), ["u-1"]);
    }

    #[tokio::test]
    async fn a_stream_gets_cloudflares_servers_without_port_53() {
        let cf = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/turn/keys/key-1/credentials/generate-ice-servers"))
            .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
            .and(body_json(json!({"ttl": 600})))
            .respond_with(ResponseTemplate::new(201).set_body_json(cloudflares()))
            .expect(1)
            .mount(&cf)
            .await;
        let d = tempfile::tempdir().unwrap();
        let t = turn(d.path(), &format!("{}/v1", cf.uri()), Some(&key()));
        let ice = t.mint().await.unwrap();
        assert_eq!(ice.users, ["u-1"]);
        assert_eq!(ice.expires - ice.minted, 600_000);
        let text = String::from_utf8(rtc_json(&ice.servers)).unwrap();
        assert!(
            text.contains("turns:turn.cloudflare.com:443") && !text.contains(":53\"") && !text.contains(":53?"),
            "{text}"
        );
        // Its Debug says nothing of the credentials.
        let debug = format!("{ice:?} {:?}", t.key().unwrap());
        assert!(!debug.contains("not-for-logs") && !debug.contains("u-1"), "{debug}");
        // Refreshed at three quarters of its life.
        assert_eq!(t.refresh_at(&ice), ice.minted + 450_000);
    }

    #[tokio::test]
    async fn a_refused_key_a_timeout_or_no_key_is_turn_down_and_never_says_the_token() {
        let cf = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(401)).mount(&cf).await;
        let d = tempfile::tempdir().unwrap();
        let e = turn(d.path(), &format!("{}/v1", cf.uri()), Some(&key())).mint().await.unwrap_err();
        assert!(e.contains("refused") && !e.contains(TOKEN) && !e.contains("key-1"), "{e}");

        let slow = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(201).set_body_json(cloudflares()).set_delay(Duration::from_secs(3)))
            .mount(&slow)
            .await;
        let d = tempfile::tempdir().unwrap();
        let e = turn(d.path(), &format!("{}/v1", slow.uri()), Some(&key())).mint().await.unwrap_err();
        assert!(e.contains("didn't answer") && e.contains("timed out") && !e.contains("key-1"), "{e}");

        let d = tempfile::tempdir().unwrap();
        let e = turn(d.path(), "http://127.0.0.1:9/v1", None).mint().await.unwrap_err();
        assert!(e.contains("no TURN key"), "{e}");
        let e = turn(d.path(), "http://127.0.0.1:9/v1", Some("[cloudflare]\nx = 1\n")).mint().await.unwrap_err();
        assert!(e.contains("token_id and api_token"), "{e}");
        let e = turn(d.path(), "http://127.0.0.1:9/v1", Some(&format!("api_token = \"{TOKEN}\" =")))
            .mint()
            .await
            .unwrap_err();
        assert!(!e.contains(TOKEN), "{e}");
        // An answer without TURN in it isn't TURN.
        let stun = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"iceServers": [{"urls": [STUN]}]})))
            .mount(&stun)
            .await;
        let e = turn(d.path(), &format!("{}/v1", stun.uri()), Some(&key())).mint().await.unwrap_err();
        assert!(e.contains("without a TURN server"), "{e}");
    }

    #[tokio::test]
    async fn an_ended_streams_credentials_are_revoked() {
        let cf = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/turn/keys/key-1/credentials/u-1/revoke"))
            .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&cf)
            .await;
        let d = tempfile::tempdir().unwrap();
        let t = turn(d.path(), &format!("{}/v1", cf.uri()), Some(&key()));
        // A name that would leave the path isn't sent.
        t.revoke(&["u-1".into(), "../x".into(), String::new()]).await;
    }
}
