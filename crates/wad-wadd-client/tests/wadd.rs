//! The client against a real wadd (a laptop's, on a socket of its own).

use std::time::Duration;

use wad_config::{Config, Profile};
use wad_proto::Event;
use wad_wadd_client::{Incoming, Wadd};

async fn wadd() -> (tempfile::TempDir, Wadd) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::defaults(Profile::User);
    config.daemon.state_dir = dir.path().join("state");
    config.machine.id = "m-test".into();
    let sock = dir.path().join("wadd.sock");
    let server =
        wadd::Server::new(Profile::User, config, wadd::Listen::Path(sock.clone()), wadd::logbuf::LogBuffer::new(100))
            .unwrap();
    tokio::spawn(server.serve());
    (dir, Wadd::new(sock))
}

#[tokio::test]
async fn edits_go_through_and_watchers_hear_them() {
    let (_dir, w) = wadd().await;
    let spaces: Vec<serde_json::Value> = w.get("/v1/wadspaces").await.unwrap();
    assert_eq!(spaces.iter().map(|s| s["name"].as_str().unwrap_or_default()).collect::<Vec<_>>(), ["WadOS"]);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let follow = w.follow(move |i| tx.send(i).is_ok());
    // The stream starts with the present (the machine first): it's listening.
    let first = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap();
    assert!(matches!(first, Some(Incoming::Event(Event::Machine(_)))), "{first:?}");
    let op = serde_json::from_value(serde_json::json!({"op": "space.create", "name": "Studio"})).unwrap();
    let made = w.op(op).await.unwrap();
    let id = made.created.expect("a new wadspace");

    let heard = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(i) = rx.recv().await {
            if let Incoming::Event(Event::Wadspace(ws)) = i
                && ws.id == id
            {
                return ws.name;
            }
        }
        String::new()
    })
    .await
    .unwrap();
    assert_eq!(heard, "Studio");
    follow.abort();
}

#[tokio::test]
async fn a_refusal_comes_back_with_its_code_and_words() {
    let (_dir, w) = wadd().await;
    let op = serde_json::from_value(serde_json::json!({"op": "builder.rename", "ws": "nope", "name": "X"})).unwrap();
    let e = w.op(op).await.unwrap_err();
    assert_eq!((e.status, e.code.as_str()), (404, "not_found"));
    assert!(e.message.contains("nope"));
}
