//! wadd's WadSpaces account (account.rs) against a mock Firebase (wiremock:
//! Auth, Firestore and the enrollMachine function under one server) and the
//! fake podman: a machine signed in to GitHub links itself, keeps only its
//! refresh token (as a podman secret), heartbeats its document, fetches the
//! account's secret files, and believes a phone's ID token only when Firebase
//! says it's the owner's. Never the real Firebase or GitHub.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use wad_config::{Config, Profile};
use wad_proto::{AccountState, SecretOrigin};
use wadd::fake::{Fake, FakeHands};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const DOCS: &str = "/v1/projects/wad-test/databases/(default)/documents";

async fn mock_firebase(fb: &MockServer, secrets: Value) {
    Mock::given(method("POST"))
        .and(path("/identitytoolkit/accounts:signInWithIdp"))
        .and(body_partial_json(json!({"postBody": "access_token=gho_test&providerId=github.com"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "localId": "u1", "idToken": "person-id", "refreshToken": "person-rt", "expiresIn": "3600",
            "screenName": "octo", "rawUserInfo": "{\"id\":42,\"login\":\"octo\"}"
        })))
        .mount(fb)
        .await;
    Mock::given(method("POST"))
        .and(path("/fn/enrollMachine"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result": {
            "machineId": "mach1", "ownerUid": "u1", "customToken": "custom-tok",
            "projectId": "wad-test", "apiKey": "k", "relink": false
        }})))
        .mount(fb)
        .await;
    Mock::given(method("POST"))
        .and(path("/identitytoolkit/accounts:signInWithCustomToken"))
        .and(body_partial_json(json!({"token": "custom-tok"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "idToken": "machine-id", "refreshToken": "machine-rt", "expiresIn": "3600"
        })))
        .mount(fb)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{DOCS}:commit")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"writeResults": []})))
        .mount(fb)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{DOCS}/users/u1/secrets")))
        .respond_with(ResponseTemplate::new(200).set_body_json(secrets))
        .mount(fb)
        .await;
    Mock::given(method("POST"))
        .and(path("/identitytoolkit/accounts:lookup"))
        .and(body_partial_json(json!({"idToken": "phone-of-u1"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"users": [{"localId": "u1"}]})))
        .mount(fb)
        .await;
    Mock::given(method("POST"))
        .and(path("/identitytoolkit/accounts:lookup"))
        .and(body_partial_json(json!({"idToken": "phone-of-u2"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"users": [{"localId": "u2"}]})))
        .mount(fb)
        .await;
    Mock::given(method("POST"))
        .and(path("/identitytoolkit/accounts:lookup"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": {"message": "INVALID_ID_TOKEN"}})))
        .mount(fb)
        .await;
}

async fn bodies(fb: &MockServer, p: &str) -> Vec<Value> {
    fb.received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == p)
        .map(|r| serde_json::from_slice(&r.body).unwrap_or(Value::Null))
        .collect()
}

async fn until(what: &str, mut f: impl AsyncFnMut() -> bool) {
    for _ in 0..200 {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("never: {what}");
}

#[tokio::test]
async fn a_machine_signed_in_to_github_links_itself_and_follows_the_account() {
    let dir = tempfile::tempdir().unwrap();
    let fb = MockServer::start().await;
    let name = wadd::secret_files::name_of("HungGod/wados", ".env");
    let doc = |n: &str| format!("projects/wad-test/databases/(default)/documents/users/u1/secrets/{n}");
    mock_firebase(
        &fb,
        json!({"documents": [
            {"name": doc(&name), "fields": {
                "value": {"stringValue": "A=1\n"},
                "file": {"mapValue": {"fields": {"repo": {"stringValue": "HungGod/wados"}, "path": {"stringValue": ".env"}}}},
                "updatedAt": {"timestampValue": "2026-11-01T00:00:00Z"}
            }},
            // Not a secret file, and one whose name isn't its place: neither is taken.
            {"name": doc("github_token"), "fields": {"value": {"stringValue": "gho_x"}}},
            {"name": doc("f-000000000000"), "fields": {
                "value": {"stringValue": "B=1"},
                "file": {"mapValue": {"fields": {"repo": {"stringValue": "HungGod/wados"}, "path": {"stringValue": ".env"}}}}
            }}
        ]}),
    )
    .await;
    let firebase = dir.path().join("firebase.toml");
    std::fs::write(&firebase, "api_key = \"k\"\nproject_id = \"wad-test\"\nfunctions_region = \"us-central1\"\n")
        .unwrap();
    let mut config = Config::defaults(Profile::User);
    config.machine.id = "m-test".into();
    config.machine.name = "Test".into();
    config.daemon.state_dir = dir.path().join("state");
    config.daemon.signins_dir = dir.path().join("state/keys");
    config.daemon.screen_dir = dir.path().join("screen");
    config.daemon.projects_dir = dir.path().join("projects");
    config.display.runtime_dir = dir.path().join("rt");
    config.packages.pull = false;
    // GitHub: nothing answers (the mock has nothing there).
    config.github.app = dir.path().join("no-github.toml");
    config.github.web = fb.uri();
    config.github.api = format!("{}/api", fb.uri());
    config.account.firebase = firebase;
    config.account.identity = fb.uri();
    config.account.firestore = fb.uri();
    config.account.functions = format!("{}/fn", fb.uri());
    std::fs::create_dir_all(&config.display.runtime_dir).unwrap();
    let fake = Arc::new(Fake::new());
    fake.state.lock().unwrap().secrets.insert("github_token".into(), "gho_test".into());
    let parts = wadd::Parts {
        backend: fake.clone(),
        hands: Arc::new(FakeHands::default()),
        poll: Duration::from_millis(20),
        sample: None,
    };
    let server = wadd::Server::with_parts(
        Profile::User,
        config,
        wadd::Listen::Path(dir.path().join("wadd.sock")),
        wadd::logbuf::LogBuffer::new(100),
        parts,
    )
    .unwrap();
    let app = server.state.clone();
    assert_eq!(app.account.view().state, AccountState::Unlinked);
    let serving = tokio::spawn(server.serve_until(std::future::pending()));

    // Linked, by itself.
    until("linked", async || app.account.view().state == AccountState::Linked).await;
    let v = app.account.view();
    assert_eq!(
        (v.uid.as_deref(), v.machine_id.as_deref(), v.login.as_deref()),
        (Some("u1"), Some("mach1"), Some("octo"))
    );

    // The code the person's sign-in made is the one enrollMachine got.
    let commits = bodies(&fb, &format!("{DOCS}:commit")).await;
    let code_doc = commits[0]["writes"][0]["update"]["name"].as_str().unwrap().to_string();
    let code = code_doc.rsplit('/').next().unwrap();
    assert!(code_doc.contains("/documents/enrollCodes/"), "{code_doc}");
    assert_eq!(commits[0]["writes"][0]["currentDocument"], json!({"exists": false}));
    assert_eq!(commits[0]["writes"][0]["update"]["fields"]["uid"], json!({"stringValue": "u1"}));
    let enroll = bodies(&fb, "/fn/enrollMachine").await;
    assert_eq!(enroll[0]["data"]["code"], json!(code));
    assert_eq!(enroll[0]["data"]["machineName"], json!("Test"));

    // Only the refresh token is kept, in podman; the file has no token.
    assert_eq!(fake.state.lock().unwrap().secrets.get("wados_account").map(String::as_str), Some("machine-rt"));
    let saved = std::fs::read_to_string(dir.path().join("state/account.json")).unwrap();
    for t in ["machine-rt", "machine-id", "custom-tok", "person-id", "person-rt", "gho_test"] {
        assert!(!saved.contains(t), "{t} in account.json");
    }

    // It heartbeats its own document, which must be there.
    until("a heartbeat", async || {
        bodies(&fb, &format!("{DOCS}:commit"))
            .await
            .iter()
            .any(|b| b["writes"][0]["update"]["name"].as_str().is_some_and(|n| n.ends_with("/users/u1/machines/mach1")))
    })
    .await;
    let beat = bodies(&fb, &format!("{DOCS}:commit"))
        .await
        .into_iter()
        .find(|b| b["writes"][0]["update"]["name"].as_str().is_some_and(|n| n.ends_with("/machines/mach1")))
        .unwrap();
    let w = &beat["writes"][0];
    assert_eq!(w["currentDocument"], json!({"exists": true}));
    assert_eq!(w["updateTransforms"][0]["fieldPath"], json!("lastSeen"));
    let mask: Vec<&str> = w["updateMask"]["fieldPaths"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    let allowed = ["hostname", "daemonVersion", "tailnet"];
    assert!(mask.iter().all(|f| allowed.contains(f)), "{mask:?}");

    // The account's secret file is here now (only the real one).
    until("the secret file", async || !app.secret_files.status().files.is_empty()).await;
    let files = app.secret_files.status().files;
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!((files[0].name.as_str(), files[0].origin), (name.as_str(), SecretOrigin::Account));
    assert_eq!(fake.state.lock().unwrap().secrets.get(&name).map(String::as_str), Some("A=1\n"));
    assert!(app.secret_files.status().account);

    // A phone's ID token: the owner's, someone else's, or no good.
    assert!(app.account.is_owner("phone-of-u1").await);
    assert!(!app.account.is_owner("phone-of-u2").await);
    assert!(!app.account.is_owner("forged").await);
    assert!(!app.account.is_owner("").await);

    // Unlinked: its sign-in and the account's files go from here.
    let v = app.account.unlink().await.unwrap();
    assert_eq!(v.state, AccountState::Unlinked);
    assert!(!fake.state.lock().unwrap().secrets.contains_key("wados_account"));
    assert!(app.secret_files.status().files.is_empty());
    assert!(!app.account.is_owner("phone-of-u1").await);

    // Linked again, then signed out: one sign-in, so it leaves the account too.
    assert_eq!(app.account.link().await.unwrap().state, AccountState::Linked);
    wad_wadd_client::Wadd::new(dir.path().join("wadd.sock")).github_signout().await.unwrap();
    assert_eq!(app.account.view().state, AccountState::Unlinked);
    let secrets = fake.state.lock().unwrap().secrets.clone();
    assert!(!secrets.contains_key("wados_account") && !secrets.contains_key("github_token"), "{secrets:?}");
    serving.abort();
}

#[tokio::test]
async fn without_firebase_the_account_is_off() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::defaults(Profile::User);
    config.daemon.state_dir = dir.path().join("state");
    config.daemon.signins_dir = dir.path().join("state/keys");
    config.daemon.screen_dir = dir.path().join("screen");
    config.display.runtime_dir = dir.path().join("rt");
    config.packages.pull = false;
    config.account.firebase = dir.path().join("none.toml");
    std::fs::create_dir_all(&config.display.runtime_dir).unwrap();
    let parts = wadd::Parts {
        backend: Arc::new(Fake::new()),
        hands: Arc::new(FakeHands::default()),
        poll: Duration::from_millis(20),
        sample: None,
    };
    let server = wadd::Server::with_parts(
        Profile::User,
        config,
        wadd::Listen::Path(dir.path().join("wadd.sock")),
        wadd::logbuf::LogBuffer::new(100),
        parts,
    )
    .unwrap();
    let app = server.state.clone();
    assert_eq!(app.account.view().state, AccountState::Off);
    assert!(app.account.link().await.is_err());
    // Secret files are kept here alone.
    assert!(!app.secret_files.status().account);
}
