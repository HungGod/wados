//! The proxy's routes: which streams it serves, and what it saw of each.
//! The stream lifecycle (streams.rs) fills it as streams start and stop
//! (and again after a wadd restart, when it adopts them); the proxy reads
//! it on every request.
//!
//! - **A public route** is served on the public listener (`[streaming]
//!   listen`, cloudflared's origin) at `/ws-<guid>/`, with the sign-in card
//!   (user and password) or the one-tap link's key.
//! - **A private route** is served only on its own listener,
//!   `127.0.0.1:<private_port>`, which [`RouteTable::add`] opens and
//!   [`RouteTable::remove`] closes; the lifecycle publishes it on the
//!   tailnet with `tailscale serve --https=<port> http://127.0.0.1:<port>`.
//!   That listener serves `/ws-<guid>/` of this stream and nothing else, and
//!   the public listener never serves a private stream: a private stream
//!   only opens on the tailnet, with its one-tap link (no password).
//!
//! **Activity** ([`RouteTable::activity`]): every request that gets through
//! to the sidecar (signed in) and every WebSocket byte, either way, is
//! someone watching; the lifecycle's idle timer reads it. Requests that
//! don't sign in (a link preview, a bot at the sign-in card) aren't.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tokio::sync::watch;
use wad_model::StreamMode;

use crate::jobs::now_ms;

/// Wrong passwords a stream takes before it asks the next one to wait, and
/// how often it takes another: 5 at once, then one every 12 s (5 a minute).
pub const TRIES: f64 = 5.0;
pub const TRY_EVERY: Duration = Duration::from_secs(12);

/// One stream's way in, as the lifecycle tells the proxy. Its `Debug`
/// leaves out the key and the password.
#[derive(Clone, PartialEq, Eq)]
pub struct Route {
    /// Its sidecar's web port on 127.0.0.1 (Selkies, serving `/ws-<guid>/`,
    /// `SELKIES_SUBFOLDER=/ws-<guid>`). Requests go there with their path
    /// unchanged and their Host as it came.
    pub web_port: u16,
    /// Selkies' token for it (the controller row in `SELKIES_TOKENS_FILE`):
    /// the one-tap link's `#token=`, and the `selkies_token` cookie the
    /// sign-in sets. URL- and cookie-safe: `[A-Za-z0-9_-]`, 16 or more.
    pub key: String,
    /// Play together's other players' keys: each signs in as itself, and
    /// Selkies knows its slot by it.
    pub players: Vec<String>,
    /// A public stream's sign-in, as the share sheet shows it (compared in
    /// constant time). Empty for a private one: it has no sign-in card.
    pub user: String,
    pub password: String,
    /// Its wadspace's name, for the card ("{name} is shared with you").
    pub name: String,
    /// Its mode: the card's footer ("Public stream"), and which listener.
    pub mode: StreamMode,
    /// A private stream's own listener (`127.0.0.1:<it>`, opened by
    /// [`RouteTable::add`]); none for a public one. Some exactly when `mode`
    /// is private.
    pub private_port: Option<u16>,
}

impl Route {
    pub fn private(&self) -> bool {
        self.private_port.is_some()
    }
}

impl fmt::Debug for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Route")
            .field("web_port", &self.web_port)
            .field("user", &self.user)
            .field("name", &self.name)
            .field("mode", &self.mode)
            .field("private_port", &self.private_port)
            .finish_non_exhaustive()
    }
}

/// What the proxy saw of one stream.
#[derive(Debug)]
pub(super) struct Seen {
    /// ms since the epoch.
    last_active: AtomicI64,
    /// WebSockets open through it now.
    viewers: AtomicU32,
    /// WebSocket bytes through it, both ways.
    bytes: AtomicU64,
}

impl Seen {
    fn new() -> Self {
        Self { last_active: AtomicI64::new(now_ms()), viewers: AtomicU32::new(0), bytes: AtomicU64::new(0) }
    }

    pub(super) fn touch(&self) {
        self.last_active.fetch_max(now_ms(), Ordering::Relaxed);
    }

    pub(super) fn heard(&self, n: usize) {
        self.bytes.fetch_add(n as u64, Ordering::Relaxed);
        self.touch();
    }
}

/// A stream's wrong passwords: a bucket of [`TRIES`], one back every [`TRY_EVERY`].
#[derive(Debug)]
pub(super) struct Tries {
    left: f64,
    at: Instant,
}

impl Tries {
    fn new(now: Instant) -> Self {
        Self { left: TRIES, at: now }
    }

    fn refill(&mut self, now: Instant) {
        let back = now.saturating_duration_since(self.at).as_secs_f64() / TRY_EVERY.as_secs_f64();
        self.left = (self.left + back).min(TRIES);
        self.at = now;
    }

    /// Takes a try for a sign-in, before it's checked (so tries at once
    /// can't overrun it); none left: how long until one is.
    pub(super) fn take(&mut self, now: Instant) -> Result<(), Duration> {
        self.refill(now);
        if self.left >= 1.0 {
            self.left -= 1.0;
            Ok(())
        } else {
            Err(TRY_EVERY.mul_f64(1.0 - self.left))
        }
    }

    /// It was right: its try comes back (only wrong ones cost).
    pub(super) fn give_back(&mut self) {
        self.left = (self.left + 1.0).min(TRIES);
    }
}

/// Aborts its task when dropped: a private stream's listener, closed with its route.
struct Listening(tokio::task::JoinHandle<()>);

impl Drop for Listening {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A route and what goes with it, while it's in the table.
pub(super) struct Entry {
    pub(super) guid: String,
    pub(super) route: Route,
    pub(super) seen: Arc<Seen>,
    pub(super) tries: Mutex<Tries>,
    /// Turns true when the route goes: its WebSockets are cut.
    pub(super) gone: watch::Sender<bool>,
    _listening: Option<Listening>,
}

/// The streams the proxy serves, by guid (the `<guid>` of `/ws-<guid>/`).
/// Made once ([`RouteTable::new`]), shared by the proxy and the lifecycle.
pub struct RouteTable {
    routes: RwLock<HashMap<String, Arc<Entry>>>,
    /// Bumped whenever a WebSocket opens or closes through any route.
    viewers_changed: watch::Sender<u64>,
}

impl RouteTable {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { routes: RwLock::new(HashMap::new()), viewers_changed: watch::channel(0).0 })
    }

    /// Routes `/ws-<guid>/` to `route`; a private one also gets its own
    /// listener, bound here on 127.0.0.1:<private_port> (`AddrInUse` if the
    /// port's another stream's or something else's: pick another). Call it
    /// from within wadd's runtime.
    ///
    /// Refused (`InvalidInput`): a guid that isn't `[A-Za-z0-9_-]{1,64}`, a
    /// key that isn't `[A-Za-z0-9_-]{16,}`, or a private port that doesn't
    /// go with the mode. `AlreadyExists`: the guid is routed already
    /// ([`RouteTable::remove`] it first).
    pub fn add(self: &Arc<Self>, guid: &str, route: Route) -> io::Result<()> {
        let invalid = |why: &str| Err(io::Error::new(io::ErrorKind::InvalidInput, why.to_string()));
        if !is_guid(guid) {
            return invalid("a stream's guid is 1 to 64 of [A-Za-z0-9_-]");
        }
        if route.key.len() < 16 || !route.key.bytes().all(url_safe) {
            return invalid("a stream's key is 16 or more of [A-Za-z0-9_-]");
        }
        if route.private() == route.mode.is_public() {
            return invalid("a private stream has a private port, and a public one hasn't");
        }
        if let Some(e) = clash(&self.routes.read().expect("routes"), guid, &route) {
            return Err(e);
        }
        let listening = match route.private_port {
            Some(port) => {
                // SO_REUSEPORT: a port its stream let go of a moment ago
                // (remove, then add again) binds while the runtime is still
                // dropping the old listener. Another stream's is refused
                // above; anything else's (without it) is AddrInUse here.
                let socket = tokio::net::TcpSocket::new_v4()?;
                socket.set_reuseaddr(true)?;
                socket.set_reuseport(true)?;
                socket.bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))?;
                let l = socket.listen(1024)?;
                tracing::info!("stream {guid}: its private listener is on 127.0.0.1:{port}");
                Some(Listening(tokio::spawn(super::serve_listener(
                    l,
                    self.clone(),
                    super::Scope::Private(guid.to_string()),
                ))))
            }
            None => None,
        };
        let entry = Entry {
            guid: guid.to_string(),
            route,
            seen: Arc::new(Seen::new()),
            tries: Mutex::new(Tries::new(Instant::now())),
            gone: watch::channel(false).0,
            _listening: listening,
        };
        let mut routes = self.routes.write().expect("routes");
        // Again, for an add at the same time (its listener closes with it).
        if let Some(e) = clash(&routes, guid, &entry.route) {
            return Err(e);
        }
        routes.insert(guid.to_string(), Arc::new(entry));
        Ok(())
    }

    /// Stops routing `guid`: its links get the expired page from now on,
    /// its WebSockets are cut, and a private one's listener closes (the
    /// runtime drops it a moment later; adding a route on that port again
    /// straight away is fine). The route it had, if it had one.
    pub fn remove(&self, guid: &str) -> Option<Route> {
        let entry = self.routes.write().expect("routes").remove(guid)?;
        let _ = entry.gone.send(true);
        Some(entry.route.clone())
    }

    /// The route of `guid`, if it's routed.
    pub fn get(&self, guid: &str) -> Option<Route> {
        self.entry(guid).map(|e| e.route.clone())
    }

    /// The guids routed now.
    pub fn guids(&self) -> Vec<String> {
        self.routes.read().expect("routes").keys().cloned().collect()
    }

    /// When someone last watched or touched `guid` (ms since the epoch: its
    /// route's adding, a request that got through, a WebSocket byte either
    /// way), and how many WebSockets are open through it now (viewers).
    /// None: it isn't routed.
    pub fn activity(&self, guid: &str) -> Option<(i64, u32)> {
        self.entry(guid).map(|e| (e.seen.last_active.load(Ordering::Relaxed), e.seen.viewers.load(Ordering::Relaxed)))
    }

    /// WebSocket bytes through `guid` so far, both ways.
    pub fn bytes(&self, guid: &str) -> Option<u64> {
        self.entry(guid).map(|e| e.seen.bytes.load(Ordering::Relaxed))
    }

    /// Changes whenever a viewer comes or goes on any route (a WebSocket
    /// opens or closes): the lifecycle tells the shell (`viewers`).
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.viewers_changed.subscribe()
    }

    pub(super) fn entry(&self, guid: &str) -> Option<Arc<Entry>> {
        self.routes.read().expect("routes").get(guid).cloned()
    }

    /// A WebSocket opened (`true`) or closed through `seen`.
    pub(super) fn viewer(&self, seen: &Seen, opened: bool) {
        match opened {
            true => seen.viewers.fetch_add(1, Ordering::Relaxed),
            false => seen.viewers.fetch_sub(1, Ordering::Relaxed),
        };
        self.viewers_changed.send_modify(|n| *n += 1);
    }
}

/// Why `route` can't be added as `guid` beside `routes`: the guid's routed
/// already, or its private port is another stream's.
fn clash(routes: &HashMap<String, Arc<Entry>>, guid: &str, route: &Route) -> Option<io::Error> {
    if routes.contains_key(guid) {
        return Some(io::Error::new(io::ErrorKind::AlreadyExists, format!("{guid} is routed already")));
    }
    let port = route.private_port?;
    routes
        .values()
        .any(|e| e.route.private_port == Some(port))
        .then(|| io::Error::new(io::ErrorKind::AddrInUse, format!("port {port} is another stream's")))
}

/// A stream's guid as the proxy takes it: 1 to 64 of `[A-Za-z0-9_-]`.
pub(super) fn is_guid(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(url_safe)
}

fn url_safe(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// `given` is the secret `expected`, compared in constant time (their
/// SHA-256s, every byte: neither the length nor where they differ shows).
/// An empty `expected` matches nothing.
pub(super) fn same_secret(given: &str, expected: &str) -> bool {
    let (a, b) = (Sha256::digest(given.as_bytes()), Sha256::digest(expected.as_bytes()));
    let differ = a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    std::hint::black_box(differ) == 0 && !expected.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(mode: StreamMode, private_port: Option<u16>) -> Route {
        Route {
            web_port: 9,
            key: "k".repeat(32),
            players: vec![],
            user: "viewer".into(),
            password: "pw".into(),
            name: "Studio".into(),
            mode,
            private_port,
        }
    }

    #[test]
    fn secrets_compare_whole_and_empty_matches_nothing() {
        assert!(same_secret("abc", "abc"));
        assert!(!same_secret("abd", "abc"));
        assert!(!same_secret("ab", "abc"));
        assert!(!same_secret("abcd", "abc"));
        assert!(!same_secret("", ""));
        assert!(!same_secret("x", ""));
    }

    #[test]
    fn wrong_passwords_run_out_then_come_back() {
        let t0 = Instant::now();
        let mut t = Tries::new(t0);
        // A right one costs nothing.
        t.take(t0).unwrap();
        t.give_back();
        for _ in 0..5 {
            t.take(t0).unwrap();
        }
        let wait = t.take(t0).unwrap_err();
        assert!(wait > Duration::from_secs(11) && wait <= TRY_EVERY, "{wait:?}");
        assert!(t.take(t0 + Duration::from_secs(6)).is_err());
        t.take(t0 + TRY_EVERY).unwrap();
        assert!(t.take(t0 + TRY_EVERY).is_err());
        // A long while later, all of them again (and no more).
        t.refill(t0 + Duration::from_secs(3600));
        assert_eq!(t.left, TRIES);
    }

    #[tokio::test]
    async fn a_route_is_checked_and_routed_once() {
        let t = RouteTable::new();
        let bad = |r: io::Result<()>| r.unwrap_err().kind();
        assert_eq!(bad(t.add("a/b", route(StreamMode::Public, None))), io::ErrorKind::InvalidInput);
        assert_eq!(bad(t.add("", route(StreamMode::Public, None))), io::ErrorKind::InvalidInput);
        let short = Route { key: "short".into(), ..route(StreamMode::Public, None) };
        assert_eq!(bad(t.add("g1", short)), io::ErrorKind::InvalidInput);
        let semi = Route { key: format!("{};x", "k".repeat(20)), ..route(StreamMode::Public, None) };
        assert_eq!(bad(t.add("g1", semi)), io::ErrorKind::InvalidInput);
        assert_eq!(bad(t.add("g1", route(StreamMode::Public, Some(1)))), io::ErrorKind::InvalidInput);
        assert_eq!(bad(t.add("g1", route(StreamMode::Private, None))), io::ErrorKind::InvalidInput);

        t.add("g1", route(StreamMode::PublicSecure, None)).unwrap();
        assert_eq!(bad(t.add("g1", route(StreamMode::Public, None))), io::ErrorKind::AlreadyExists);
        assert_eq!(t.get("g1").unwrap().name, "Studio");
        assert_eq!(t.guids(), ["g1"]);
        let (last, viewers) = t.activity("g1").unwrap();
        assert!(last > 0 && viewers == 0);
        assert_eq!(t.bytes("g1"), Some(0));
        assert!(!format!("{:?}", t.get("g1").unwrap()).contains("kkkk"), "the key isn't in its Debug");
        assert!(t.remove("g1").is_some());
        assert!(t.remove("g1").is_none());
        assert_eq!((t.get("g1"), t.activity("g1")), (None, None));
    }
}
