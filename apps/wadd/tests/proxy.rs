//! The streaming proxy (proxy.rs) in front of a fake Selkies sidecar: a
//! small axum server that answers every request with what it saw (its Host,
//! path, cookie and headers), and echoes a WebSocket's bytes back.
//!
//! Routing by path with the Host as it came, WebSockets both ways, the
//! redirect, the expired page and 502; the sign-in (wrong, then right: the
//! cookie and its attributes), the one-tap link's key, the rate limit, the
//! activity the idle timer reads; a private stream's own listener; and the
//! key handed on to Selkies' sockets (their query, the signalling HELLO),
//! never into a log.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body as AxumBody;
use axum::response::{IntoResponse, Response};
use reqwest::StatusCode;
use reqwest::header::{self, HeaderMap};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use wad_model::StreamMode;
use wadd::proxy::{self, Route, RouteTable};

const GUID: &str = "0123456789abcdef0123456789abcdef";
const KEY: &str = "key-0123456789abcdef0123456789ab";
const USER: &str = "viewer";
const PASSWORD: &str = "correct-horse";
const HOST: &str = "stream.example";
const EXPIRED: &str = "This link has expired";
const WRONG: &str = "That’s not it: check the user and password you were sent.";

/// The fake sidecar: what it saw, as JSON (a 101's `x-seen` header for a WebSocket).
async fn seen(mut req: axum::extract::Request) -> Response {
    let h = req.headers();
    let get = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let seen = serde_json::json!({
        "host": get("host"),
        "uri": req.uri().to_string(),
        "cookie": get("cookie"),
        "authorization": get("authorization"),
        "secret": get("x-secret"),
        "keepAlive": get("keep-alive"),
        "protocols": get("sec-websocket-protocol"),
        "extensions": get("sec-websocket-extensions"),
    });
    if get("upgrade").eq_ignore_ascii_case("websocket") {
        let on = hyper::upgrade::on(&mut req);
        tokio::spawn(async move {
            let Ok(up) = on.await else { return };
            let (mut r, mut w) = tokio::io::split(hyper_util::rt::TokioIo::new(up));
            let _ = tokio::io::copy(&mut r, &mut w).await;
        });
        return Response::builder()
            .status(101)
            .header("upgrade", "websocket")
            .header("connection", "upgrade")
            .header("sec-websocket-accept", "from-the-sidecar")
            .header("x-seen", seen.to_string())
            .body(AxumBody::empty())
            .unwrap();
    }
    axum::Json(seen).into_response()
}

async fn sidecar() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, axum::Router::new().fallback(seen)).await.unwrap() });
    port
}

/// A port nothing listens on.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

struct Up {
    table: Arc<RouteTable>,
    /// The public listener.
    port: u16,
    http: reqwest::Client,
}

impl Up {
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.http.get(self.url(path)).header(header::HOST, HOST)
    }

    fn post(&self, path: &str, form: &[(&str, &str)]) -> reqwest::RequestBuilder {
        self.http.post(self.url(path)).header(header::HOST, HOST).form(form)
    }
}

fn public(web_port: u16) -> Route {
    Route {
        web_port,
        key: KEY.into(),
        players: vec![],
        user: USER.into(),
        password: PASSWORD.into(),
        name: "Studio".into(),
        mode: StreamMode::Public,
        private_port: None,
    }
}

/// The public listener on a port of its own, with the stream GUID routed to a fake sidecar.
async fn up() -> Up {
    let table = RouteTable::new();
    table.add(GUID, public(sidecar().await)).unwrap();
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(proxy::serve_on(table.clone(), vec![l]));
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).no_proxy().build().unwrap();
    Up { table, port, http }
}

fn cookie() -> String {
    format!("{}={KEY}", proxy::COOKIE)
}

/// A WebSocket's opening through the proxy: the stream, and the response's head.
async fn websocket(port: u16, path: &str, cookie: Option<&str>) -> (TcpStream, String) {
    let cookie = cookie.map(|c| format!("Cookie: {c}\r\n")).unwrap_or_default();
    websocket_with(port, path, &cookie).await
}

/// The same, with `extra` header lines (each ending `\r\n`).
async fn websocket_with(port: u16, path: &str, extra: &str) -> (TcpStream, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {HOST}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\
         Origin: https://{HOST}\r\n{extra}\r\n"
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut head = vec![];
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        assert_eq!(s.read(&mut b).await.unwrap(), 1, "the head ended early: {}", String::from_utf8_lossy(&head));
        head.push(b[0]);
    }
    (s, String::from_utf8(head).unwrap())
}

/// What the sidecar saw of a WebSocket's handshake (its 101's `x-seen`).
fn x_seen(head: &str) -> Value {
    head.lines()
        .find_map(|l| l.strip_prefix("x-seen: "))
        .map(|j| serde_json::from_str(j).unwrap())
        .unwrap_or_else(|| panic!("what the sidecar saw, in {head}"))
}

/// A client's masked text frame.
fn masked(text: &str) -> Vec<u8> {
    let mask = [0x37, 0xfa, 0x21, 0x3d];
    let mut f = vec![0x81];
    match text.len() {
        n if n < 126 => f.push(0x80 | n as u8),
        n => {
            f.push(0x80 | 126);
            f.extend((n as u16).to_be_bytes());
        }
    }
    f.extend(mask);
    f.extend(text.bytes().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    f
}

/// The next masked frame's text from `s` (the fake sidecar echoes what it got).
async fn echoed(s: &mut TcpStream) -> String {
    let mut head = [0u8; 2];
    s.read_exact(&mut head).await.unwrap();
    let len = match head[1] & 0x7f {
        126 => {
            let mut n = [0u8; 2];
            s.read_exact(&mut n).await.unwrap();
            u16::from_be_bytes(n) as usize
        }
        n => n as usize,
    };
    let mut mask = [0u8; 4];
    s.read_exact(&mut mask).await.unwrap();
    let mut payload = vec![0u8; len];
    s.read_exact(&mut payload).await.unwrap();
    String::from_utf8(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]).collect()).unwrap()
}

async fn until(what: &str, f: impl Fn() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what} didn't happen");
}

fn set_cookie(h: &HeaderMap) -> String {
    h.get(header::SET_COOKIE).expect("a cookie").to_str().unwrap().to_string()
}

#[tokio::test]
async fn a_streams_path_goes_to_its_sidecar_with_its_host_and_websockets_both_ways() {
    let up = up().await;
    let r = up
        .get(&format!("/ws-{GUID}/assets/app.js?v=2"))
        .header(header::COOKIE, format!("theme=dark; {}", cookie()))
        .header(header::CONNECTION, "keep-alive, x-secret")
        .header("x-secret", "hop")
        .header("keep-alive", "timeout=5")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v: Value = r.json().await.unwrap();
    // The path unchanged, the Host as it came, the cookie for Selkies; nothing of this hop's.
    assert_eq!(v["uri"], format!("/ws-{GUID}/assets/app.js?v=2"));
    assert_eq!(v["host"], HOST);
    assert_eq!(v["cookie"], format!("theme=dark; {}", cookie()));
    assert_eq!((v["secret"].as_str(), v["keepAlive"].as_str()), (Some(""), Some("")), "{v}");

    // The key as ?token= or a Bearer header does as well.
    let r = up.get(&format!("/ws-{GUID}/x?token={KEY}")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let r = up.get(&format!("/ws-{GUID}/x")).bearer_auth(KEY).send().await.unwrap();
    assert_eq!(r.json::<Value>().await.unwrap()["authorization"], format!("Bearer {KEY}"));

    // A WebSocket: upgraded through, the Host (and so its Origin check) intact, bytes echoed both ways.
    let changes = up.table.subscribe();
    let (mut ws, head) = websocket(up.port, &format!("/ws-{GUID}/api/websockets"), Some(&cookie())).await;
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert!(head.to_ascii_lowercase().contains("sec-websocket-accept: from-the-sidecar"), "{head}");
    let seen = x_seen(&head);
    // (The key goes on in its query: signed in by the cookie, it carried none of its own.)
    assert_eq!(
        (seen["host"].as_str(), seen["uri"].as_str()),
        (Some(HOST), Some(&*format!("/ws-{GUID}/api/websockets?token={KEY}")))
    );
    let frame = b"\x81\x85abcd\x09\x07\x0f\x08\x0e"; // a masked "hello"
    ws.write_all(frame).await.unwrap();
    let mut back = [0u8; 11];
    ws.read_exact(&mut back).await.unwrap();
    assert_eq!(&back, frame);
    // A viewer, while it's open; its bytes count as watching.
    until("a viewer", || up.table.activity(GUID).unwrap().1 == 1).await;
    assert!(changes.has_changed().unwrap());
    assert!(up.table.bytes(GUID).unwrap() >= 22, "{:?}", up.table.bytes(GUID));
    drop(ws);
    until("the viewer leaving", || up.table.activity(GUID).unwrap().1 == 0).await;

    // Without the key, no WebSocket.
    let (_, head) = websocket(up.port, &format!("/ws-{GUID}/api/websockets"), None).await;
    assert!(head.starts_with("HTTP/1.1 401"), "{head}");
}

#[tokio::test]
async fn the_bare_path_redirects_and_unknown_links_have_expired() {
    let up = up().await;
    let r = up.get(&format!("/ws-{GUID}")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(r.headers()[header::LOCATION], format!("/ws-{GUID}/"));
    let r = up.get(&format!("/ws-{GUID}?a=1")).send().await.unwrap();
    assert_eq!(r.headers()[header::LOCATION], format!("/ws-{GUID}/?a=1"));

    for path in ["/", "/ws-ffffffffffffffffffffffffffffffff/", "/ws-ffff", "/favicon.ico", "/ws-/"] {
        let r = up.get(path).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{path}");
        let html = r.text().await.unwrap();
        assert!(html.contains(EXPIRED) && html.contains("Ask whoever sent it for a new one."), "{path}: {html}");
    }
    // Gone with its stream: the same page.
    up.table.remove(GUID).unwrap();
    let r = up.get(&format!("/ws-{GUID}/")).header(header::COOKIE, cookie()).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert!(r.text().await.unwrap().contains(EXPIRED));
}

#[tokio::test]
async fn a_sidecar_that_doesnt_answer_is_502() {
    let up = up().await;
    up.table.add("quiet", public(free_port())).unwrap();
    let r =
        up.get("/ws-quiet/").header(header::COOKIE, cookie()).header(header::ACCEPT, "text/html").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_GATEWAY);
    let html = r.text().await.unwrap();
    assert!(html.contains("This stream isn't answering yet") && html.contains("http-equiv=\"refresh\""), "{html}");
    let r = up.get("/ws-quiet/api/websockets").header(header::COOKIE, cookie()).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(r.text().await.unwrap(), proxy::NOT_ANSWERING);
}

#[tokio::test]
async fn the_sign_in_card_then_the_cookie_then_the_stream() {
    let up = up().await;
    let page = format!("/ws-{GUID}/");
    // Not signed in: the card at the stream's page, its name and where it's from, and 401 for the rest.
    let r = up.get(&page).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(r.headers()[header::CONTENT_SECURITY_POLICY].to_str().unwrap().contains("frame-ancestors 'none'"));
    let html = r.text().await.unwrap();
    assert!(html.contains("Studio is shared with you"), "{html}");
    assert!(html.contains("Sign in with the user and password you were sent."));
    assert!(html.contains(&format!("{HOST} · Public stream")));
    assert!(html.contains("location.hash"), "the one-tap link's script");
    assert_eq!(up.get(&format!("/ws-{GUID}/index.html")).send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
    // A fetch of the page (not a navigation) gets no card.
    let r = up.get(&page).header("sec-fetch-mode", "cors").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    // Selkies' open routes need no key, and aren't someone watching.
    let (before, _) = up.table.activity(GUID).unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    for open in ["api/status", "api/health"] {
        let r = up.get(&format!("/ws-{GUID}/{open}")).send().await.unwrap();
        assert_eq!(r.json::<Value>().await.unwrap()["uri"], format!("/ws-{GUID}/{open}"));
    }
    assert_eq!(up.table.activity(GUID).unwrap().0, before, "the card and open routes aren't activity");

    // The wrong password: the card again, saying so, the user kept.
    let r = up.post(&page, &[("user", USER), ("password", "nope")]).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert!(r.headers().get(header::SET_COOKIE).is_none());
    let html = r.text().await.unwrap();
    assert!(html.contains(WRONG) && html.contains(&format!("value=\"{USER}\"")), "{html}");
    // The right one (the user as typed, spaces and all): the cookie, and back to the stream.
    let r = up.post(&page, &[("user", " viewer "), ("password", PASSWORD)]).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(r.headers()[header::LOCATION], page);
    let c = set_cookie(r.headers());
    let parts: Vec<&str> = c.split("; ").collect();
    assert_eq!(parts[0], cookie());
    for attr in [&*format!("Path=/ws-{GUID}/"), "Max-Age=43200", "HttpOnly", "Secure", "SameSite=Lax"] {
        assert!(parts.contains(&attr), "{attr} in {c}");
    }
    assert!(up.table.activity(GUID).unwrap().0 > before, "signing in is activity");
    // With it, the page is Selkies'.
    let r = up.get(&page).header(header::COOKIE, parts[0]).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.json::<Value>().await.unwrap()["uri"], page);
    // A cookie with another key is no cookie.
    let r = up.get(&page).header(header::COOKIE, format!("{}=stale", proxy::COOKIE)).send().await.unwrap();
    assert!(r.text().await.unwrap().contains("is shared with you"));
}

#[tokio::test]
async fn the_one_tap_links_key_is_checked_and_sets_the_cookie() {
    let up = up().await;
    let page = format!("/ws-{GUID}/");
    let r = up.post(&page, &[("token", "not-the-key")]).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert!(r.headers().get(header::SET_COOKIE).is_none());
    let r = up.post(&page, &[("token", KEY)]).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let c = set_cookie(r.headers());
    assert!(c.starts_with(&format!("{}; Path=/ws-{GUID}/;", cookie())) && c.contains("HttpOnly"), "{c}");
}

#[tokio::test]
async fn wrong_passwords_are_limited_per_stream() {
    let up = up().await;
    up.table.add("other", public(sidecar().await)).unwrap();
    let page = format!("/ws-{GUID}/");
    for _ in 0..proxy::TRIES as usize {
        let r = up.post(&page, &[("user", USER), ("password", "guess")]).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
    }
    // Then even the right one waits, saying so.
    let r = up.post(&page, &[("user", USER), ("password", PASSWORD)]).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    let wait: u64 = r.headers()[header::RETRY_AFTER].to_str().unwrap().parse().unwrap();
    assert!((1..=proxy::TRY_EVERY.as_secs()).contains(&wait), "{wait}");
    assert!(r.headers().get(header::SET_COOKIE).is_none());
    assert!(r.text().await.unwrap().contains("Too many tries: wait a minute, then try again."));
    // The one-tap link still opens it, and another stream's sign-in is its own.
    assert_eq!(up.post(&page, &[("token", KEY)]).send().await.unwrap().status(), StatusCode::NO_CONTENT);
    let r = up.post("/ws-other/", &[("user", USER), ("password", PASSWORD)]).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn requests_that_get_through_are_activity() {
    let up = up().await;
    let (added, viewers) = up.table.activity(GUID).unwrap();
    assert_eq!(viewers, 0);
    tokio::time::sleep(Duration::from_millis(5)).await;
    up.get(&format!("/ws-{GUID}/")).send().await.unwrap();
    assert_eq!(up.table.activity(GUID).unwrap().0, added, "the card isn't someone watching");
    up.get(&format!("/ws-{GUID}/")).header(header::COOKIE, cookie()).send().await.unwrap();
    assert!(up.table.activity(GUID).unwrap().0 > added);
    // A WebSocket's bytes are, too (and both ways count).
    let (mut ws, _) = websocket(up.port, &format!("/ws-{GUID}/api/websockets"), Some(&cookie())).await;
    let (t0, b0) = (up.table.activity(GUID).unwrap().0, up.table.bytes(GUID).unwrap());
    tokio::time::sleep(Duration::from_millis(5)).await;
    ws.write_all(b"input").await.unwrap();
    let mut back = [0u8; 5];
    ws.read_exact(&mut back).await.unwrap();
    assert_eq!(up.table.bytes(GUID).unwrap(), b0 + 10);
    assert!(up.table.activity(GUID).unwrap().0 > t0);
    // Its stream stopping cuts it.
    up.table.remove(GUID);
    let n = tokio::time::timeout(Duration::from_secs(2), ws.read(&mut back)).await.expect("cut").unwrap_or(0);
    assert_eq!(n, 0);
}

#[tokio::test]
async fn a_private_stream_has_a_listener_of_its_own_and_no_password() {
    let up = up().await;
    let port = free_port();
    let private = Route {
        user: String::new(),
        password: String::new(),
        name: "Notes".into(),
        mode: StreamMode::Private,
        private_port: Some(port),
        ..public(sidecar().await)
    };
    up.table.add("priv", private).unwrap();
    let on =
        |path: &str| up.http.get(format!("http://127.0.0.1:{port}{path}")).header(header::HOST, "desk.ts.test:48300");
    // Its card: no password to ask for, its one-tap link opens it.
    let html = on("/ws-priv/").send().await.unwrap().text().await.unwrap();
    assert!(html.contains("Notes is shared with you") && !html.contains("<form"), "{html}");
    assert!(html.contains("one-tap link") && html.contains("desk.ts.test:48300 · Private stream"));
    let r = up
        .http
        .post(format!("http://127.0.0.1:{port}/ws-priv/"))
        .form(&[("user", ""), ("password", "")])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert!(r.headers().get(header::SET_COOKIE).is_none());
    let r = up.http.post(format!("http://127.0.0.1:{port}/ws-priv/")).form(&[("token", KEY)]).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let r = on("/ws-priv/").header(header::COOKIE, cookie()).send().await.unwrap();
    assert_eq!(r.json::<Value>().await.unwrap()["host"], "desk.ts.test:48300");
    // It serves only its stream; the public listener never serves it.
    let r = on(&format!("/ws-{GUID}/")).header(header::COOKIE, cookie()).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = up.get("/ws-priv/").header(header::COOKIE, cookie()).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    // Its port is taken while it's routed (by another stream, or by anything else).
    let again = up.table.get("priv").unwrap();
    assert_eq!(up.table.add("priv2", again.clone()).unwrap_err().kind(), std::io::ErrorKind::AddrInUse);
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let elsewhere = Route { private_port: Some(taken.local_addr().unwrap().port()), ..again.clone() };
    assert_eq!(up.table.add("priv3", elsewhere).unwrap_err().kind(), std::io::ErrorKind::AddrInUse);
    // Taken off and put back at once (an adoption), it serves on.
    up.table.remove("priv").unwrap();
    up.table.add("priv", again).unwrap();
    // (A connection in the moment both are open may land on the closing one: a viewer's retries.)
    tokio::time::sleep(Duration::from_millis(50)).await;
    let r = on("/ws-priv/").header(header::COOKIE, cookie()).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    // And its listener closes with it.
    up.table.remove("priv").unwrap();
    for _ in 0..200 {
        if TcpStream::connect(("127.0.0.1", port)).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("its listener didn't close");
}

#[tokio::test]
async fn a_signed_in_socket_takes_the_key_on_to_selkies() {
    let up = up().await;
    let data = format!("/ws-{GUID}/api/websockets");
    // Signed in by the cookie: the key goes on in its query, beside what was there.
    let (_ws, head) = websocket(up.port, &format!("{data}?role=viewer"), Some(&cookie())).await;
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert_eq!(x_seen(&head)["uri"], format!("{data}?role=viewer&token={KEY}"));
    // By a Bearer header (a script), the same.
    let (_ws, head) = websocket_with(up.port, &data, &format!("Authorization: Bearer {KEY}\r\n")).await;
    assert_eq!(x_seen(&head)["uri"], format!("{data}?token={KEY}"));
    // With a key of its own (Selkies' subprotocol, or ?token=): as it came, the subprotocol opening it.
    let encoded = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, KEY);
    let protocols = format!("selkies, selkies.token.{encoded}");
    let (_ws, head) = websocket_with(up.port, &data, &format!("Sec-WebSocket-Protocol: {protocols}\r\n")).await;
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    let seen = x_seen(&head);
    assert_eq!((seen["uri"].as_str(), seen["protocols"].as_str()), (Some(&*data), Some(&*protocols)));
    let (_ws, head) = websocket(up.port, &format!("{data}?token={KEY}"), Some(&cookie())).await;
    assert_eq!(x_seen(&head)["uri"], format!("{data}?token={KEY}"));
    // Without the key: refused here, never reaching Selkies.
    let (_, head) = websocket(up.port, &data, None).await;
    assert!(head.starts_with("HTTP/1.1 401") && !head.contains("x-seen"), "{head}");
    let (_, head) = websocket_with(up.port, &data, "Sec-WebSocket-Protocol: selkies, selkies.token.d3Jvbmc\r\n").await;
    assert!(head.starts_with("HTTP/1.1 401"), "{head}");
    let (_, head) = websocket(up.port, &format!("{data}?token=wrong"), None).await;
    assert!(head.starts_with("HTTP/1.1 401"), "{head}");
}

#[tokio::test]
async fn a_signalling_hello_without_a_key_gets_it() {
    let up = up().await;
    let signalling = format!("/ws-{GUID}/api/webrtc/signaling");
    let deflate = "Sec-WebSocket-Extensions: permessage-deflate; client_max_window_bits\r\n";
    let (mut ws, head) = websocket_with(up.port, &signalling, &format!("Cookie: {}\r\n{deflate}", cookie())).await;
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    // Its compression offer stays here, so the HELLO goes plain.
    assert_eq!(x_seen(&head)["extensions"], "");
    ws.write_all(&masked(r#"HELLO client {"client_type":"controller","client_token":"","display_id":"primary"}"#))
        .await
        .unwrap();
    let hello = echoed(&mut ws).await;
    assert_eq!(
        hello,
        format!(r#"HELLO client {{"client_type":"controller","client_token":"{KEY}","display_id":"primary"}}"#)
    );
    // What follows passes as it is.
    ws.write_all(&masked(r#"{"sdp":{"type":"answer"}}"#)).await.unwrap();
    assert_eq!(echoed(&mut ws).await, r#"{"sdp":{"type":"answer"}}"#);

    // A HELLO with its own key keeps it.
    let (mut ws, _) = websocket(up.port, &signalling, Some(&cookie())).await;
    ws.write_all(&masked(r#"HELLO client {"client_token":"its-own"}"#)).await.unwrap();
    assert_eq!(echoed(&mut ws).await, r#"HELLO client {"client_token":"its-own"}"#);
    // The data socket's messages are never touched.
    let (mut ws, _) = websocket(up.port, &format!("/ws-{GUID}/api/websockets"), Some(&cookie())).await;
    ws.write_all(&masked(r#"HELLO client {"client_token":""}"#)).await.unwrap();
    assert_eq!(echoed(&mut ws).await, r#"HELLO client {"client_token":""}"#);
}

/// Everything the proxy logs, into a buffer.
#[derive(Clone, Default)]
struct Logs(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Logs {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn the_key_and_password_are_never_logged() {
    let logs = Logs::default();
    let into = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || into.clone())
        .finish();
    // This test's thread (tokio's current-thread runtime: the proxy's tasks run on it too).
    let _logging = tracing::subscriber::set_default(subscriber);
    let up = up().await;
    up.table.add("quiet", public(free_port())).unwrap();
    let page = format!("/ws-{GUID}/");
    // Everything the key and password go through: the card, a wrong and a right sign-in,
    // the one-tap key, the key on Selkies' sockets and in a HELLO, a sidecar not answering.
    up.get(&page).send().await.unwrap();
    up.post(&page, &[("user", USER), ("password", "nope")]).send().await.unwrap();
    up.post(&page, &[("user", USER), ("password", PASSWORD)]).send().await.unwrap();
    up.post(&page, &[("token", KEY)]).send().await.unwrap();
    up.get(&format!("/ws-{GUID}/x?token={KEY}")).send().await.unwrap();
    let (_ws, _) = websocket(up.port, &format!("/ws-{GUID}/api/websockets"), Some(&cookie())).await;
    let (mut ws, _) = websocket(up.port, &format!("/ws-{GUID}/api/webrtc/signaling"), Some(&cookie())).await;
    ws.write_all(&masked(r#"HELLO client {"client_token":""}"#)).await.unwrap();
    assert!(echoed(&mut ws).await.contains(KEY));
    up.get("/ws-quiet/").header(header::COOKIE, cookie()).send().await.unwrap();
    let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    // It logged (what's worth it), and never a secret.
    assert!(logs.contains("a wrong user or password") && logs.contains("its sidecar"), "{logs}");
    let encoded = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, KEY);
    for secret in [KEY, PASSWORD, &encoded] {
        assert!(!logs.contains(secret), "{secret} in the logs: {logs}");
    }
}
