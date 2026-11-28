//! What the proxy's tests share: a fake Selkies sidecar, the proxy in front
//! of it, and a WebSocket client's few frames.
#![allow(dead_code, unused_imports)]

pub use std::sync::Arc;
pub use std::time::Duration;

pub use axum::body::Body as AxumBody;
pub use axum::response::{IntoResponse, Response};
pub use reqwest::StatusCode;
pub use reqwest::header::{self, HeaderMap};
pub use serde_json::Value;
pub use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub use tokio::net::{TcpListener, TcpStream};
pub use wad_model::StreamMode;
pub use wadd::proxy::{self, Route, RouteTable};

pub const GUID: &str = "0123456789abcdef0123456789abcdef";
pub const KEY: &str = "key-0123456789abcdef0123456789ab";
pub const USER: &str = "viewer";
pub const PASSWORD: &str = "correct-horse";
pub const HOST: &str = "stream.example";
pub const EXPIRED: &str = "This link has expired";
pub const WRONG: &str = "That’s not it: check the user and password you were sent.";

/// The fake sidecar: what it saw, as JSON (a 101's `x-seen` header for a WebSocket).
pub async fn seen(mut req: axum::extract::Request) -> Response {
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

pub async fn sidecar() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, axum::Router::new().fallback(seen)).await.unwrap() });
    port
}

/// A port nothing listens on.
pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

pub struct Up {
    pub table: Arc<RouteTable>,
    /// The public listener.
    pub port: u16,
    pub http: reqwest::Client,
}

impl Up {
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    pub fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.http.get(self.url(path)).header(header::HOST, HOST)
    }

    pub fn post(&self, path: &str, form: &[(&str, &str)]) -> reqwest::RequestBuilder {
        self.http.post(self.url(path)).header(header::HOST, HOST).form(form)
    }
}

pub fn public(web_port: u16) -> Route {
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
pub async fn up() -> Up {
    let table = RouteTable::new();
    table.add(GUID, public(sidecar().await)).unwrap();
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(proxy::serve_on(table.clone(), vec![l]));
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).no_proxy().build().unwrap();
    Up { table, port, http }
}

pub fn cookie() -> String {
    format!("{}={KEY}", proxy::COOKIE)
}

/// A WebSocket's opening through the proxy: the stream, and the response's head.
pub async fn websocket(port: u16, path: &str, cookie: Option<&str>) -> (TcpStream, String) {
    let cookie = cookie.map(|c| format!("Cookie: {c}\r\n")).unwrap_or_default();
    websocket_with(port, path, &cookie).await
}

/// The same, with `extra` header lines (each ending `\r\n`).
pub async fn websocket_with(port: u16, path: &str, extra: &str) -> (TcpStream, String) {
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
pub fn x_seen(head: &str) -> Value {
    head.lines()
        .find_map(|l| l.strip_prefix("x-seen: "))
        .map(|j| serde_json::from_str(j).unwrap())
        .unwrap_or_else(|| panic!("what the sidecar saw, in {head}"))
}

/// A client's masked text frame.
pub fn masked(text: &str) -> Vec<u8> {
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
pub async fn echoed(s: &mut TcpStream) -> String {
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

pub async fn until(what: &str, f: impl Fn() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what} didn't happen");
}

pub fn set_cookie(h: &HeaderMap) -> String {
    h.get(header::SET_COOKIE).expect("a cookie").to_str().unwrap().to_string()
}
