//! The streaming proxy: how viewers reach a stream's Selkies sidecar.
//!
//! **Listeners**, on loopback only:
//! - **public**, `[streaming] listen` (127.0.0.1:8085 and [::1]:8085): the
//!   Cloudflare tunnel's origin. It serves the public routes.
//! - **private**, one per private stream on `127.0.0.1:<private_port>`,
//!   opened and closed with its route ([`RouteTable::add`]), and published
//!   on the tailnet by `tailscale serve --https=<port>` (the lifecycle's).
//!   It serves that stream only.
//!
//! **Routing is by path**, whatever the Host:
//! - `/ws-<guid>/…`, a route this listener serves: to its sidecar at
//!   `127.0.0.1:<web_port>`, the path and Host unchanged, WebSockets
//!   upgraded both ways, nothing buffered, no time limit;
//! - `/ws-<guid>` → 308 to `/ws-<guid>/`;
//! - anything else (an unknown guid, `/`): the expired page, 404;
//! - its sidecar not answering: 502 ("This stream isn't answering yet").
//!
//! **Signing in.** Selkies (2.x, its secure mode) takes the stream's key
//! as `#token=` (client side), `?token=`, `Authorization: Bearer`, or the
//! `selkies_token` cookie. The proxy checks the same key the same ways
//! (in constant time) before anything reaches the sidecar, except
//! Selkies' open `api/status` and `api/health`:
//! - `GET /ws-<guid>/` without it: the sign-in card ([`pages`]). Its
//!   script looks for the one-tap link's `#token=` first, and POSTs it
//!   here (`token=<key>`); the right key gets the cookie (204). Without
//!   one, a public stream's card asks for the user and password (POSTed
//!   here as a form: the right ones get the cookie and a 303 back to
//!   `/ws-<guid>/`; wrong ones the card again, "That’s not it", and only
//!   [`TRIES`] of them a minute per stream). A private stream's card has
//!   no password: its one-tap link is how it opens.
//! - Anything else without it: 401.
//!
//! The cookie: `selkies_token=<key>; Path=/ws-<guid>/; Max-Age=43200;
//! HttpOnly; Secure; SameSite=Lax`. Selkies' HTTP API reads it itself.
//!
//! **Its sockets don't** (Selkies 2.2): a handshake's key is its
//! `selkies.token.<base64url>` subprotocol or `?token=`, and a WebRTC
//! signalling socket's is `client_token` in the client's first message
//! (`HELLO client {…}`). A client the cookie signed in has none to send
//! (its page never had the key), so the proxy hands it on, to the sidecar
//! only (loopback; Selkies' access log drops queries):
//! - a WebSocket handshake that got through on the cookie (or Bearer) and
//!   carries no key of its own goes on with `token=<key>` added to its query;
//! - a signalling socket (`api/webrtc/signaling`, `api/ws`) goes on without
//!   its compression offer, so its HELLO goes plain, and a HELLO without a
//!   `client_token` gets the key there. Everything after it passes as it is.
//!
//! Nothing here logs a request's query, cookies or body: keys travel in them.

mod pages;
mod routes;

use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Empty, Full, combinators::BoxBody};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};

pub use routes::{Route, RouteTable, TRIES, TRY_EVERY};

use routes::{Entry, Seen, is_guid, same_secret};

type Body = BoxBody<Bytes, hyper::Error>;

/// Selkies' cookie for its token.
pub const COOKIE: &str = "selkies_token";
/// The cookie lasts 12 hours (and only as long as the stream's key).
pub const COOKIE_MAX_AGE_S: u64 = 12 * 3600;
/// v1's words for a sidecar that doesn't answer.
pub const NOT_ANSWERING: &str = "This stream isn't answering yet: try again in a moment.\n";
const NEEDS_KEY: &str = "This stream needs its key: open the link you were sent.\n";
/// Selkies' own routes that need no key.
const OPEN: [&str; 2] = ["/api/status", "/api/health"];
/// Selkies' WebRTC signalling sockets: the client's key goes in its HELLO.
const SIGNALLING: [&str; 3] = ["/api/webrtc/signaling", "/api/webrtc/signaling/", "/api/ws"];
/// The subprotocol a handshake carries a key in (base64url), beside `selkies`.
const TOKEN_PROTOCOL: &str = "selkies.token.";
/// The most of a signalling socket's first frame the proxy holds to fill
/// its HELLO (Selkies' are a few hundred bytes); a bigger one goes as it is.
const HELLO_MAX: usize = 64 * 1024;
const CONNECT: Duration = Duration::from_secs(5);
/// A sign-in's form or key: small.
const FORM_MAX: usize = 4096;

/// Which routes a listener serves.
pub(crate) enum Scope {
    /// The public listener: the public routes.
    Public,
    /// A private stream's own listener: that one.
    Private(String),
}

impl Scope {
    fn serves(&self, e: &Entry) -> bool {
        match self {
            Scope::Public => !e.route.private(),
            Scope::Private(guid) => *guid == e.guid && e.route.private(),
        }
    }
}

/// The public listener: binds each of `listen` and serves the public
/// routes on them until the task is aborted (with them).
pub async fn serve(table: Arc<RouteTable>, listen: Vec<String>) {
    serve_on(table, bind(&listen).await).await
}

/// Serves the public routes on these listeners until the task is aborted.
pub async fn serve_on(table: Arc<RouteTable>, listeners: Vec<TcpListener>) {
    let all = listeners.into_iter().map(|l| serve_listener(l, table.clone(), Scope::Public));
    futures_util::future::join_all(all).await;
}

/// Binds each address, a loopback one only; one that can't be bound is
/// logged and skipped.
pub async fn bind(addresses: &[String]) -> Vec<TcpListener> {
    let mut out = vec![];
    for a in addresses {
        let addr: SocketAddr = match a.parse() {
            Ok(addr) => addr,
            Err(e) => {
                tracing::warn!("streaming proxy: {a:?}: {e}");
                continue;
            }
        };
        if !addr.ip().is_loopback() {
            tracing::warn!("streaming proxy: {a} isn't a loopback address: not listening there");
            continue;
        }
        match TcpListener::bind(addr).await {
            Ok(l) => {
                tracing::info!("streaming proxy on {}", l.local_addr().map(|x| x.to_string()).unwrap_or_default());
                out.push(l);
            }
            Err(e) => tracing::warn!("streaming proxy: can't listen on {a}: {e}"),
        }
    }
    out
}

/// Accepts on `l` for as long as the task runs, each connection answered for `scope`.
pub(crate) async fn serve_listener(l: TcpListener, table: Arc<RouteTable>, scope: Scope) {
    let scope = Arc::new(scope);
    loop {
        let tcp = match l.accept().await {
            Ok((tcp, _)) => tcp,
            Err(e) => {
                // Out of descriptors, say: a breath, then on.
                tracing::debug!("streaming proxy: accept: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let _ = tcp.set_nodelay(true);
        let (table, scope) = (table.clone(), scope.clone());
        connection(tcp, move |req| handle(table.clone(), scope.clone(), req));
    }
}

/// One HTTP/1.1 connection (WebSockets upgraded), answered by `handle`.
fn connection<F, Fut>(tcp: TcpStream, handle: F)
where
    F: Fn(Request<Incoming>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Response<Body>, Infallible>> + Send + 'static,
{
    tokio::spawn(async move {
        let service = hyper::service::service_fn(handle);
        let conn = hyper::server::conn::http1::Builder::new()
            .timer(TokioTimer::new())
            .header_read_timeout(Duration::from_secs(30))
            .serve_connection(TokioIo::new(tcp), service)
            .with_upgrades();
        let _ = conn.await;
    });
}

async fn handle(
    table: Arc<RouteTable>,
    scope: Arc<Scope>,
    req: Request<Incoming>,
) -> Result<Response<Body>, Infallible> {
    let path = req.uri().path();
    let Some((guid, rest)) = path.strip_prefix("/ws-").map(|p| p.split_at(p.find('/').unwrap_or(p.len()))) else {
        return Ok(pages::expired());
    };
    let entry = match table.entry(guid) {
        Some(e) if is_guid(guid) && scope.serves(&e) => e,
        _ => return Ok(pages::expired()),
    };
    if rest.is_empty() {
        let to = match req.uri().query() {
            Some(q) => format!("/ws-{guid}/?{q}"),
            None => format!("/ws-{guid}/"),
        };
        return Ok(redirect(StatusCode::PERMANENT_REDIRECT, &to, None));
    }
    if rest == "/" && req.method() == Method::POST {
        return Ok(sign_in(req, &entry).await);
    }
    if OPEN.contains(&rest) {
        return Ok(forward_or_502(&table, req, &entry, false, entry.route.key.clone()).await);
    }
    if let Some(key) = signed_in_as(req.headers(), req.uri().query(), &entry.route) {
        entry.seen.touch();
        let signalling = SIGNALLING.contains(&rest);
        return Ok(forward_or_502(&table, req, &entry, signalling, key).await);
    }
    if rest == "/" && matches!(*req.method(), Method::GET | Method::HEAD) && navigation(req.headers()) {
        let host = host_of(req.headers());
        return Ok(pages::sign_in(StatusCode::OK, &card(&entry, &host, "", None)));
    }
    Ok(text(StatusCode::UNAUTHORIZED, NEEDS_KEY))
}

fn card<'a>(e: &'a Entry, host: &'a str, user: &'a str, error: Option<&'a str>) -> pages::Card<'a> {
    pages::Card { guid: &e.guid, name: &e.route.name, host, mode: e.route.mode, user, error }
}

/// Which of the stream's keys the request carries, if one: each is compared.
fn signed_in_as(h: &HeaderMap, query: Option<&str>, route: &Route) -> Option<String> {
    std::iter::once(&route.key).chain(&route.players).fold(None, |found, key| {
        let right = signed_in(h, query, key);
        if right && found.is_none() { Some(key.clone()) } else { found }
    })
}

/// The request carries the stream's key: its cookie, `?token=`, a Bearer
/// header, or (a WebSocket's) Selkies' token subprotocol.
fn signed_in(h: &HeaderMap, query: Option<&str>, key: &str) -> bool {
    let cookies = h
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| c.split_once('='))
        .filter(|(name, _)| name.trim() == COOKIE)
        .map(|(_, value)| value.trim().trim_matches('"').to_string());
    let queried = query.into_iter().flat_map(form).filter(|(k, _)| k == "token").map(|(_, v)| v);
    let bearer = h
        .get_all(header::AUTHORIZATION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.strip_prefix("Bearer ").map(|t| t.trim().to_string()));
    let protocol = offered_protocols(h).filter_map(|p| protocol_token(&p));
    // Every one is compared: none is skipped for being early.
    cookies.chain(queried).chain(bearer).chain(protocol).fold(false, |ok, given| same_secret(&given, key) | ok)
}

/// The subprotocols a WebSocket handshake offers.
fn offered_protocols(h: &HeaderMap) -> impl Iterator<Item = String> + '_ {
    h.get_all(header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|p| p.trim().to_string())
}

/// The key in Selkies' token subprotocol (`selkies.token.<base64url>`).
fn protocol_token(p: &str) -> Option<String> {
    use base64::Engine;
    let encoded = p.strip_prefix(TOKEN_PROTOCOL)?.trim_end_matches('=');
    String::from_utf8(base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded).ok()?).ok()
}

/// A handshake carries a key of its own for Selkies: a `token=` in its query,
/// or the token subprotocol. Then it goes on as it is.
fn own_key(h: &HeaderMap, query: Option<&str>) -> bool {
    query.into_iter().flat_map(form).any(|(k, v)| k == "token" && !v.is_empty())
        || offered_protocols(h).any(|p| p.starts_with(TOKEN_PROTOCOL))
}

/// `path?query` with `token=<key>` added (a `token` without a value goes).
fn with_key(pq: &str, key: &str) -> String {
    let (path, query) = pq.split_once('?').unwrap_or((pq, ""));
    let token = format!("token={key}");
    let mut items: Vec<&str> = query.split('&').filter(|i| !i.is_empty() && *i != "token" && *i != "token=").collect();
    items.push(&token);
    format!("{path}?{}", items.join("&"))
}

/// A top-level page load: what the browser says (Sec-Fetch-Mode), or a
/// GET from something that doesn't say.
fn navigation(h: &HeaderMap) -> bool {
    h.get("sec-fetch-mode").and_then(|v| v.to_str().ok()).is_none_or(|m| m == "navigate")
}

/// `POST /ws-<guid>/`: the one-tap link's key (`token`), or the card's
/// user and password.
async fn sign_in(req: Request<Incoming>, e: &Entry) -> Response<Body> {
    let host = host_of(req.headers());
    let Ok(body) = http_body_util::Limited::new(req.into_body(), FORM_MAX).collect().await else {
        return text(StatusCode::PAYLOAD_TOO_LARGE, "That's too much for a sign-in.\n");
    };
    let fields = form(&String::from_utf8_lossy(&body.to_bytes()));
    let field = |k: &str| fields.iter().find(|(name, _)| name == k).map(|(_, v)| v.as_str());
    if let Some(token) = field("token") {
        let key = std::iter::once(&e.route.key).chain(&e.route.players).find(|k| same_secret(token, k)).cloned();
        let Some(key) = key else {
            return text(StatusCode::FORBIDDEN, "That link's key doesn't open this stream.\n");
        };
        e.seen.touch();
        let mut r = text(StatusCode::NO_CONTENT, "");
        set_cookie(r.headers_mut(), e, &key);
        return r;
    }
    let (user, password) = (field("user").unwrap_or_default().trim(), field("password").unwrap_or_default());
    if e.route.private() {
        // No password opens a private stream.
        return pages::sign_in(StatusCode::FORBIDDEN, &card(e, &host, "", None));
    }
    if let Err(wait) = e.tries.lock().expect("tries").take(Instant::now()) {
        let mut r = pages::sign_in(StatusCode::TOO_MANY_REQUESTS, &card(e, &host, user, Some(pages::TOO_MANY)));
        if let Ok(v) = HeaderValue::from_str(&wait.as_secs().max(1).to_string()) {
            r.headers_mut().insert(header::RETRY_AFTER, v);
        }
        return r;
    }
    // Both compared, whichever is wrong.
    let right = same_secret(user, &e.route.user) & same_secret(password, &e.route.password);
    if !right {
        tracing::info!("stream {}: a wrong user or password", e.guid);
        return pages::sign_in(StatusCode::FORBIDDEN, &card(e, &host, user, Some(pages::WRONG)));
    }
    e.tries.lock().expect("tries").give_back();
    e.seen.touch();
    redirect(StatusCode::SEE_OTHER, &format!("/ws-{}/", e.guid), Some(e))
}

fn set_cookie(h: &mut HeaderMap, e: &Entry, key: &str) {
    let cookie =
        format!("{COOKIE}={key}; Path=/ws-{}/; Max-Age={COOKIE_MAX_AGE_S}; HttpOnly; Secure; SameSite=Lax", e.guid);
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        h.insert(header::SET_COOKIE, v);
    }
}

fn redirect(status: StatusCode, to: &str, cookie: Option<&Entry>) -> Response<Body> {
    let mut r = text(status, "");
    if let Ok(v) = HeaderValue::from_str(to) {
        r.headers_mut().insert(header::LOCATION, v);
    }
    if let Some(e) = cookie {
        set_cookie(r.headers_mut(), e, &e.route.key);
    }
    r
}

async fn forward_or_502(
    table: &Arc<RouteTable>,
    req: Request<Incoming>,
    e: &Entry,
    signalling: bool,
    key: String,
) -> Response<Body> {
    let page = navigation(req.headers())
        && req.headers().get(header::ACCEPT).and_then(|v| v.to_str().ok()).is_some_and(|a| a.contains("text/html"));
    match forward(table, req, e, signalling, key).await {
        Ok(r) => r,
        Err(why) => {
            tracing::debug!("stream {}: its sidecar: {why}", e.guid);
            if page { pages::not_answering() } else { text(StatusCode::BAD_GATEWAY, NOT_ANSWERING) }
        }
    }
}

/// Passes a request to the sidecar unchanged (but for the hop-by-hop
/// headers, and a signed-in socket's key: the module's doc), and a
/// WebSocket through both ways.
async fn forward(
    table: &Arc<RouteTable>,
    mut req: Request<Incoming>,
    e: &Entry,
    signalling: bool,
    key: String,
) -> Result<Response<Body>, String> {
    let upgrade = wants_upgrade(req.headers()).then(|| hyper::upgrade::on(&mut req));
    let (mut parts, body) = req.into_parts();
    strip_hop_by_hop(&mut parts.headers, upgrade.is_some());
    let mut pq = parts.uri.path_and_query().map(|p| p.as_str()).unwrap_or("/").to_string();
    if upgrade.is_some() && !own_key(&parts.headers, parts.uri.query()) {
        pq = with_key(&pq, &key);
    }
    let hello = (upgrade.is_some() && signalling).then(|| {
        // No permessage-deflate: the HELLO goes plain, to be read.
        parts.headers.remove(header::SEC_WEBSOCKET_EXTENSIONS);
        key.clone()
    });
    parts.uri = pq.parse().map_err(|e: hyper::http::uri::InvalidUri| e.to_string())?;
    let tcp = tokio::time::timeout(CONNECT, TcpStream::connect(("127.0.0.1", e.route.web_port)))
        .await
        .map_err(|_| "connect: timed out".to_string())?
        .map_err(|e| format!("connect: {e}"))?;
    let _ = tcp.set_nodelay(true);
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake(TokioIo::new(tcp)).await.map_err(|e| e.to_string())?;
    tokio::spawn(async move {
        let _ = conn.with_upgrades().await;
    });
    let mut resp = sender.send_request(Request::from_parts(parts, body)).await.map_err(|e| e.to_string())?;
    if resp.status() == StatusCode::SWITCHING_PROTOCOLS
        && let Some(viewer) = upgrade
    {
        let sidecar = hyper::upgrade::on(&mut resp);
        let (table, seen, mut gone) = (table.clone(), e.seen.clone(), e.gone.subscribe());
        tokio::spawn(async move {
            let (Ok(a), Ok(b)) = (viewer.await, sidecar.await) else { return };
            table.viewer(&seen, true);
            let mut a = Heard { io: HelloFill::new(TokioIo::new(a), hello), seen: seen.clone() };
            let mut b = Heard { io: TokioIo::new(b), seen: seen.clone() };
            tokio::select! {
                _ = tokio::io::copy_bidirectional(&mut a, &mut b) => {}
                // The stream stopped: its viewers go with it.
                _ = gone.wait_for(|g| *g) => {}
            }
            table.viewer(&seen, false);
        });
        let (p, _) = resp.into_parts();
        return Ok(Response::from_parts(p, Empty::new().map_err(|e| match e {}).boxed()));
    }
    strip_hop_by_hop(resp.headers_mut(), false);
    Ok(resp.map(|b| b.boxed()))
}

/// The Host a request came by, for the card's footer: as sent (with its
/// port), lowercased, if it's a plausible one.
fn host_of(h: &HeaderMap) -> String {
    let host = h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or_default().trim().to_ascii_lowercase();
    let plausible = host.len() <= 255
        && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']'));
    if plausible { host } else { String::new() }
}

/// `a=b&c=d`, decoded (`+` a space, `%xx` a byte).
fn form(s: &str) -> Vec<(String, String)> {
    fn hex(b: u8) -> Option<u8> {
        (b as char).to_digit(16).map(|d| d as u8)
    }
    fn decode(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            let escaped = match b[i] {
                b'%' => b.get(i + 1).copied().and_then(hex).zip(b.get(i + 2).copied().and_then(hex)),
                _ => None,
            };
            match (b[i], escaped) {
                (_, Some((hi, lo))) => {
                    out.push(hi << 4 | lo);
                    i += 2;
                }
                (b'+', None) => out.push(b' '),
                (c, None) => out.push(c),
            }
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }
    s.split('&')
        .filter(|kv| !kv.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (decode(k), decode(v))
        })
        .collect()
}

fn wants_upgrade(h: &HeaderMap) -> bool {
    h.contains_key(header::UPGRADE)
        && h.get_all(header::CONNECTION)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .any(|t| t.trim().eq_ignore_ascii_case("upgrade"))
}

/// RFC 9110 §7.6.1: what's only for this hop (and what Connection names)
/// isn't passed on; an upgrade keeps Connection and Upgrade.
fn strip_hop_by_hop(h: &mut HeaderMap, upgrade: bool) {
    let named: Vec<HeaderName> = h
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|t| HeaderName::from_bytes(t.trim().as_bytes()).ok())
        .filter(|n| !(upgrade && *n == header::UPGRADE))
        .collect();
    for n in named {
        h.remove(n);
    }
    for n in [
        header::TE,
        header::TRAILER,
        header::TRANSFER_ENCODING,
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
    ] {
        h.remove(n);
    }
    h.remove("keep-alive");
    h.remove("proxy-connection");
    if upgrade {
        h.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
    } else {
        h.remove(header::CONNECTION);
        h.remove(header::UPGRADE);
    }
}

fn text(status: StatusCode, body: &'static str) -> Response<Body> {
    let mut r = Response::new(Full::new(Bytes::from_static(body.as_bytes())).map_err(|e| match e {}).boxed());
    *r.status_mut() = status;
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    pages::no_store(h);
    r
}

/// The viewer's side of a signalling socket: its first message, Selkies'
/// `HELLO client {…}`, gets the stream's key as `client_token` when it has
/// none. Control frames before it pass; anything else first (or a frame too
/// big, split or compressed) and it stands aside. After that, as it is.
struct HelloFill<T> {
    io: T,
    /// The key, until the HELLO has gone by.
    key: Option<String>,
    /// Read, still to be handed on.
    out: Vec<u8>,
    sent: usize,
    /// The frame being gathered.
    held: Vec<u8>,
}

/// A WebSocket frame at the start of some bytes.
enum Frame {
    /// Not all of it yet.
    Partial,
    /// More than [`HELLO_MAX`].
    TooBig,
    Whole {
        total: usize,
        header: usize,
        fin: bool,
        rsv: u8,
        opcode: u8,
        mask: Option<[u8; 4]>,
    },
}

impl<T> HelloFill<T> {
    fn new(io: T, key: Option<String>) -> Self {
        Self { io, key, out: vec![], sent: 0, held: vec![] }
    }

    /// Moves what's gathered on, as far as it goes: control frames as they
    /// are, then the first data frame (filled, if it's a HELLO), then the rest.
    fn sort(&mut self) {
        while let Some(key) = &self.key {
            match frame(&self.held) {
                Frame::Partial => return,
                Frame::Whole { total, opcode, .. } if opcode & 0x8 != 0 => {
                    self.out.extend(self.held.drain(..total));
                }
                Frame::Whole { total, header, fin, rsv, opcode, mask } => {
                    let whole: Vec<u8> = self.held.drain(..total).collect();
                    let filled = match (fin, rsv, opcode, mask) {
                        (true, 0, 1, Some(mask)) => fill_hello(&whole[header..], mask, key),
                        _ => None,
                    };
                    self.out.extend(filled.unwrap_or(whole));
                    self.stand_aside();
                }
                Frame::TooBig => self.stand_aside(),
            }
        }
    }

    fn stand_aside(&mut self) {
        self.key = None;
        self.out.append(&mut self.held);
    }
}

/// The frame at the start of `b`.
fn frame(b: &[u8]) -> Frame {
    let (Some(&b0), Some(&b1)) = (b.first(), b.get(1)) else { return Frame::Partial };
    let (len, mut header) = match b1 & 0x7f {
        126 => match b.get(2..4) {
            Some(n) => (u16::from_be_bytes([n[0], n[1]]) as u64, 4),
            None => return Frame::Partial,
        },
        127 => match b.get(2..10) {
            Some(n) => (u64::from_be_bytes(n.try_into().expect("8 bytes")), 10),
            None => return Frame::Partial,
        },
        n => (n as u64, 2),
    };
    if len > HELLO_MAX as u64 {
        return Frame::TooBig;
    }
    let mask = match b1 & 0x80 != 0 {
        true => match b.get(header..header + 4) {
            Some(m) => {
                header += 4;
                Some([m[0], m[1], m[2], m[3]])
            }
            None => return Frame::Partial,
        },
        false => None,
    };
    let total = header + len as usize;
    if b.len() < total {
        return Frame::Partial;
    }
    Frame::Whole { total, header, fin: b0 & 0x80 != 0, rsv: b0 & 0x70, opcode: b0 & 0x0f, mask }
}

/// A masked text frame's payload, if it's a client's HELLO without a key:
/// the frame again, with the key as its `client_token`.
fn fill_hello(masked: &[u8], mask: [u8; 4], key: &str) -> Option<Vec<u8>> {
    let unmask = |b: &[u8]| -> Vec<u8> { b.iter().enumerate().map(|(i, x)| x ^ mask[i % 4]).collect() };
    let text = String::from_utf8(unmask(masked)).ok()?;
    let (peer, meta) = text.strip_prefix("HELLO ")?.split_once(' ')?;
    if peer != "client" {
        return None;
    }
    let mut meta: serde_json::Value = serde_json::from_str(meta).ok()?;
    let fields = meta.as_object_mut()?;
    if fields.get("client_token").and_then(serde_json::Value::as_str).is_some_and(|t| !t.is_empty()) {
        return None;
    }
    fields.insert("client_token".into(), key.into());
    let payload = format!("HELLO {peer} {meta}").into_bytes();
    let mut out = vec![0x81];
    match payload.len() {
        n if n < 126 => out.push(0x80 | n as u8),
        n if n <= 0xffff => {
            out.push(0x80 | 126);
            out.extend((n as u16).to_be_bytes());
        }
        n => {
            out.push(0x80 | 127);
            out.extend((n as u64).to_be_bytes());
        }
    }
    out.extend(mask);
    out.extend(unmask(&payload));
    Some(out)
}

impl<T: AsyncRead + Unpin> AsyncRead for HelloFill<T> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let me = self.get_mut();
        loop {
            if me.sent < me.out.len() {
                let n = buf.remaining().min(me.out.len() - me.sent);
                buf.put_slice(&me.out[me.sent..me.sent + n]);
                me.sent += n;
                if me.sent == me.out.len() {
                    me.out.clear();
                    me.sent = 0;
                }
                return Poll::Ready(Ok(()));
            }
            if me.key.is_none() {
                return Pin::new(&mut me.io).poll_read(cx, buf);
            }
            let mut chunk = [0u8; 4096];
            let mut read = ReadBuf::new(&mut chunk);
            match Pin::new(&mut me.io).poll_read(cx, &mut read) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                // It ended first: what there is, then the end.
                Poll::Ready(Ok(())) if read.filled().is_empty() => {
                    me.stand_aside();
                    if me.out.is_empty() {
                        return Poll::Ready(Ok(()));
                    }
                }
                Poll::Ready(Ok(())) => {
                    me.held.extend_from_slice(read.filled());
                    me.sort();
                }
            }
        }
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for HelloFill<T> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

/// One side of a WebSocket: what it reads is someone watching (the idle
/// timer's clock), and its bytes are counted.
struct Heard<T> {
    io: T,
    seen: Arc<Seen>,
}

impl<T: AsyncRead + Unpin> AsyncRead for Heard<T> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let r = Pin::new(&mut self.io).poll_read(cx, buf);
        let n = buf.filled().len() - before;
        if matches!(r, Poll::Ready(Ok(()))) && n > 0 {
            self.seen.heard(n);
        }
        r
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Heard<T> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hop_by_hop_headers_stay_on_their_hop() {
        let mut h = HeaderMap::new();
        h.insert(header::CONNECTION, HeaderValue::from_static("keep-alive, x-secret"));
        h.insert("x-secret", HeaderValue::from_static("1"));
        h.insert("keep-alive", HeaderValue::from_static("timeout=5"));
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer k"));
        h.insert(header::COOKIE, HeaderValue::from_static("a=b"));
        strip_hop_by_hop(&mut h, false);
        assert!(h.get("x-secret").is_none() && h.get("keep-alive").is_none() && h.get(header::CONNECTION).is_none());
        assert_eq!(h[header::AUTHORIZATION], "Bearer k");
        assert_eq!(h[header::COOKIE], "a=b");

        let mut ws = HeaderMap::new();
        ws.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
        ws.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        assert!(wants_upgrade(&ws));
        strip_hop_by_hop(&mut ws, true);
        assert_eq!(ws[header::CONNECTION], "upgrade");
        assert_eq!(ws[header::UPGRADE], "websocket");
    }

    #[test]
    fn forms_decode() {
        assert_eq!(
            form("user=a+b&password=p%26%3D%25&x&=y&bad=%zz%4"),
            [
                ("user".into(), "a b".into()),
                ("password".into(), "p&=%".into()),
                ("x".into(), String::new()),
                (String::new(), "y".into()),
                ("bad".into(), "%zz%4".into()),
            ]
        );
    }

    #[test]
    fn the_key_is_taken_from_its_cookie_query_or_header() {
        let key = "k".repeat(32);
        let mut h = HeaderMap::new();
        assert!(!signed_in(&h, None, &key));
        h.insert(header::COOKIE, HeaderValue::from_str(&format!("other=1; {COOKIE}={key}")).unwrap());
        assert!(signed_in(&h, None, &key));
        h.insert(header::COOKIE, HeaderValue::from_static("selkies_token=wrong"));
        assert!(!signed_in(&h, None, &key));
        assert!(signed_in(&h, Some(&format!("a=1&token={key}")), &key));
        assert!(!signed_in(&h, Some("token="), &key));
        h.insert(header::AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {key}")).unwrap());
        assert!(signed_in(&h, None, &key));
    }

    #[test]
    fn the_footer_shows_a_plausible_host_only() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_static("Desk.Tail-Net.ts.net:48300"));
        assert_eq!(host_of(&h), "desk.tail-net.ts.net:48300");
        h.insert(header::HOST, HeaderValue::from_static("<script>"));
        assert_eq!(host_of(&h), "");
    }

    /// A client's masked text frame.
    fn masked(text: &str, mask: [u8; 4]) -> Vec<u8> {
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

    /// A masked frame's text.
    fn unmasked(f: &[u8]) -> String {
        let Frame::Whole { header, mask: Some(mask), total, .. } = frame(f) else { panic!("not a whole masked frame") };
        assert_eq!(total, f.len());
        String::from_utf8(f[header..].iter().enumerate().map(|(i, b)| b ^ mask[i % 4]).collect()).unwrap()
    }

    /// What a HelloFill reads from `chunks`, given one at a time.
    async fn through(chunks: Vec<Vec<u8>>, key: Option<&str>) -> Vec<u8> {
        use tokio::io::AsyncReadExt;
        let (mut w, r) = tokio::io::duplex(1 << 20);
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            for c in chunks {
                w.write_all(&c).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        let mut out = vec![];
        HelloFill::new(r, key.map(String::from)).read_to_end(&mut out).await.unwrap();
        out
    }

    #[tokio::test]
    async fn a_hello_without_a_key_gets_it() {
        let key = "k".repeat(32);
        let hello = masked(
            r#"HELLO client {"client_type":"controller","client_token":"","display_id":"primary"}"#,
            [1, 2, 3, 4],
        );
        let later = masked("SESSION 1", [5, 6, 7, 8]);
        let ping = vec![0x89, 0x80, 9, 9, 9, 9];
        // In bits and pieces, a ping first: the ping as it was, the HELLO filled, the rest untouched.
        let mut all = [ping.clone(), hello.clone(), later.clone()].concat();
        let chunks: Vec<Vec<u8>> = all.chunks(3).map(<[u8]>::to_vec).collect();
        let out = through(chunks, Some(&key)).await;
        assert_eq!(&out[..ping.len()], &ping[..]);
        let Frame::Whole { total, .. } = frame(&out[ping.len()..]) else { panic!() };
        let text = unmasked(&out[ping.len()..ping.len() + total]);
        assert_eq!(
            text,
            format!(r#"HELLO client {{"client_type":"controller","client_token":"{key}","display_id":"primary"}}"#)
        );
        assert_eq!(&out[ping.len() + total..], &later[..]);
        // A big one (126+ bytes) too, and a HELLO with no client_token at all.
        let pad = "x".repeat(200);
        let big = masked(&format!(r#"HELLO client {{"pad":"{pad}"}}"#), [7; 4]);
        let out = through(vec![big], Some(&key)).await;
        assert_eq!(unmasked(&out), format!(r#"HELLO client {{"pad":"{pad}","client_token":"{key}"}}"#));

        // Its own key, a server's HELLO, something else first, or no key to give: as they came.
        all = masked(r#"HELLO client {"client_token":"mine"}"#, [1; 4]);
        assert_eq!(through(vec![all.clone()], Some(&key)).await, all);
        let server = masked(r#"HELLO server {"server_token":""}"#, [1; 4]);
        assert_eq!(through(vec![server.clone()], Some(&key)).await, server);
        let other = [later.clone(), hello.clone()].concat();
        assert_eq!(through(vec![other.clone()], Some(&key)).await, other);
        assert_eq!(through(vec![hello.clone()], None).await, hello);
        // Split across frames, or cut short: as they came.
        let mut split = hello.clone();
        split[0] = 0x01;
        assert_eq!(through(vec![split.clone()], Some(&key)).await, split);
        let cut = hello[..10].to_vec();
        assert_eq!(through(vec![cut.clone()], Some(&key)).await, cut);
    }

    #[test]
    fn a_socket_takes_the_key_on_unless_it_has_its_own() {
        let key = "k".repeat(32);
        assert_eq!(with_key("/ws-g/api/websockets", &key), format!("/ws-g/api/websockets?token={key}"));
        assert_eq!(
            with_key("/ws-g/api/websockets?a=1&token=&b", &key),
            format!("/ws-g/api/websockets?a=1&b&token={key}")
        );
        let mut h = HeaderMap::new();
        assert!(!own_key(&h, None) && !own_key(&h, Some("a=1&token=")));
        assert!(own_key(&h, Some(&format!("token={key}"))));
        h.insert(header::SEC_WEBSOCKET_PROTOCOL, HeaderValue::from_static("selkies, selkies.token.a2tr"));
        assert!(own_key(&h, None));
        // The subprotocol's key opens the stream as the cookie does (base64url, padded or not).
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, &key);
        h.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_str(&format!("selkies, selkies.token.{encoded}")).unwrap(),
        );
        assert!(signed_in(&h, None, &key));
        h.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_str(&format!("selkies,selkies.token.{encoded}==")).unwrap(),
        );
        assert!(signed_in(&h, None, &key));
        h.insert(header::SEC_WEBSOCKET_PROTOCOL, HeaderValue::from_static("selkies, selkies.token.d3Jvbmc"));
        assert!(!signed_in(&h, None, &key));
    }
}
