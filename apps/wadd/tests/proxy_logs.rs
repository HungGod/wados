//! The key and password never go into the proxy's logs.
//!
//! In a test binary of its own: tracing caches, per call site and for the
//! whole process, whether anyone listens. This test listens with a subscriber
//! for its thread alone (`set_default`), so another test hitting the same call
//! sites at the same moment on another thread can leave them cached as unheard,
//! and this test then sees almost nothing.

mod proxy_support;
use proxy_support::*;

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
