//! wadd's GitHub (github.rs) through the socket, against a mock GitHub
//! (wiremock: its sign-in at the root, its API under /api) and the fake
//! podman: the device-flow sign-in, the token kept as the secret, whose it
//! is, the repos, signing out, and the ways a sign-in ends without a token.
//! Never the real GitHub. And the web keys for the machine's browser.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use wad_config::{Config, Profile};
use wad_model::{JobKind, JobStatus, Op};
use wad_proto::signin::WebKey;
use wad_proto::{Event, JobInfo, SignInProvider, SignInRequest};
use wad_wadd_client::{Incoming, Wadd};
use wadd::fake::{Fake, FakeHands};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const CLIENT_ID: &str = "Iv1.wadostest";
const TOKEN: &str = "gho_mocktoken0601";

struct Up {
    dir: tempfile::TempDir,
    fake: Arc<Fake>,
    gh: MockServer,
    w: Wadd,
    events: tokio::sync::mpsc::UnboundedReceiver<Incoming>,
    follow: tokio::task::JoinHandle<()>,
}

impl Drop for Up {
    fn drop(&mut self) {
        self.follow.abort();
    }
}

/// wadd on the fake podman, its GitHub the mock (nothing mounted on it yet).
async fn up() -> Up {
    let dir = tempfile::tempdir().unwrap();
    let gh = MockServer::start().await;
    let app = dir.path().join("github.toml");
    std::fs::write(&app, format!("client_id = \"{CLIENT_ID}\"\nscopes = [\"repo\", \"workflow\", \"read:org\"]\n"))
        .unwrap();
    let mut config = Config::defaults(Profile::User);
    config.machine.id = "m-test".into();
    config.machine.name = "Test".into();
    config.daemon.state_dir = dir.path().join("state");
    config.daemon.signins_dir = dir.path().join("state/keys");
    config.daemon.screen_dir = dir.path().join("screen");
    config.display.runtime_dir = dir.path().join("rt");
    config.packages.pull = false;
    config.github.app = app;
    config.github.web = gh.uri();
    config.github.api = format!("{}/api", gh.uri());
    std::fs::create_dir_all(&config.display.runtime_dir).unwrap();
    let fake = Arc::new(Fake::new());
    let parts = wadd::Parts {
        backend: fake.clone(),
        hands: Arc::new(FakeHands::default()),
        poll: Duration::from_millis(20),
        sample: None,
    };
    let sock = dir.path().join("wadd.sock");
    let server = wadd::Server::with_parts(
        Profile::User,
        config,
        wadd::Listen::Path(sock.clone()),
        wadd::logbuf::LogBuffer::new(100),
        parts,
    )
    .unwrap();
    tokio::spawn(server.serve_until(std::future::pending()));
    let w = Wadd::new(sock);
    let (tx, events) = tokio::sync::mpsc::unbounded_channel();
    let follow = w.follow(move |i| tx.send(i).is_ok());
    Up { dir, fake, gh, w, events, follow }
}

/// Each answer in turn; the last one again after that.
struct Answers(Mutex<VecDeque<ResponseTemplate>>);

impl Respond for Answers {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let mut q = self.0.lock().unwrap();
        if q.len() > 1 { q.pop_front().unwrap() } else { q.front().cloned().unwrap() }
    }
}

fn ok(body: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

fn oauth_error(error: &str) -> ResponseTemplate {
    ok(json!({"error": error, "error_description": format!("{error} (the mock)")}))
}

impl Up {
    /// GitHub gives a device code (asking every `interval` s).
    async fn device_code(&self, interval: u64) {
        let page = format!("{}/login/device", self.gh.uri());
        Mock::given(method("POST"))
            .and(path("/login/device/code"))
            .respond_with(ok(json!({"device_code": "dc-0601", "user_code": "WADS-0601",
                "verification_uri": page, "expires_in": 900, "interval": interval})))
            .mount(&self.gh)
            .await;
    }

    /// The polls' answers, in turn.
    async fn polls(&self, answers: Vec<ResponseTemplate>) {
        Mock::given(method("POST"))
            .and(path("/login/oauth/access_token"))
            .respond_with(Answers(Mutex::new(answers.into())))
            .mount(&self.gh)
            .await;
    }

    /// GitHub's API knows TOKEN as hunggod's; anything else it rejects.
    async fn api(&self, scopes: &str) {
        Mock::given(method("GET"))
            .and(path("/api/user"))
            .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
            .respond_with(ok(json!({"login": "hunggod", "name": "Hung"})).insert_header("x-oauth-scopes", scopes))
            .mount(&self.gh)
            .await;
        let repo = |n: &str| {
            json!({"full_name": format!("HungGod/{n}"), "name": n, "private": true,
            "clone_url": format!("https://github.com/HungGod/{n}.git"), "default_branch": "main",
            "pushed_at": "2026-11-01T00:00:00Z", "description": "a repo"})
        };
        Mock::given(method("GET"))
            .and(path("/api/user/repos"))
            .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
            .respond_with(ok(json!([repo("WadSpaces"), repo("wados")])))
            .mount(&self.gh)
            .await;
        for p in ["/api/user", "/api/user/repos"] {
            Mock::given(method("GET")).and(path(p)).respond_with(ResponseTemplate::new(401)).mount(&self.gh).await;
        }
    }

    /// The requests GitHub got at `p`.
    async fn asked(&self, p: &str) -> Vec<Request> {
        self.gh.received_requests().await.unwrap().into_iter().filter(|r| r.url.path() == p).collect()
    }

    fn token(&self) -> Option<String> {
        self.fake.state.lock().unwrap().secrets.get("github_token").cloned()
    }

    /// Every event heard so far.
    fn heard(&mut self) -> Vec<Event> {
        let mut out = vec![];
        while let Ok(i) = self.events.try_recv() {
            if let Incoming::Event(e) = i {
                out.push(e);
            }
        }
        out
    }

    /// Until an event `f` likes comes (within 10 s); the ones before it too.
    async fn until(&mut self, f: impl Fn(&Event) -> bool) -> Vec<Event> {
        let mut seen = vec![];
        let found = tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(i) = self.events.recv().await {
                if let Incoming::Event(e) = i {
                    let hit = f(&e);
                    seen.push(e);
                    if hit {
                        return;
                    }
                }
            }
        })
        .await;
        assert!(found.is_ok(), "never came; heard {seen:?}");
        seen
    }
}

/// Waits for a job to end; it, then.
async fn ended(w: &Wadd, job: &JobInfo) -> JobInfo {
    for _ in 0..1000 {
        let j = w.job(&job.job.id, 0).await.unwrap().job;
        if wadd::jobs::finished(j.job.status) {
            return j;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{} didn't end", job.job.id);
}

fn form(r: &Request) -> Vec<(String, String)> {
    url::form_urlencoded::parse(&r.body).into_owned().collect()
}

fn signin(e: &Event) -> bool {
    matches!(e, Event::SignIn { provider: SignInProvider::Github, .. })
}

fn signin_done(e: &Event) -> bool {
    matches!(e, Event::SignInDone { provider: SignInProvider::Github })
}

#[tokio::test]
async fn a_sign_in_keeps_the_token_then_whose_it_is_its_repos_and_a_sign_out() {
    let mut up = up().await;
    up.device_code(0).await;
    up.polls(vec![
        oauth_error("authorization_pending"),
        oauth_error("authorization_pending"),
        ok(json!({"access_token": TOKEN, "token_type": "bearer", "scope": "repo,workflow,read:org"})),
    ])
    .await;
    up.api("repo, workflow, read:org").await;
    let w = &up.w;
    // Signed out: no account, no repos.
    assert_eq!(w.github().await.unwrap(), None);
    let e = w.github_repos().await.unwrap_err();
    assert_eq!((e.status, e.message.as_str()), (401, "Sign in to WadSpaces first"));

    let job = w.github_signin(&SignInRequest::default()).await.unwrap();
    assert_eq!(job.job.kind, JobKind::Github);
    let labels: Vec<&str> = job.job.stages.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(
        labels,
        ["Opening GitHub's sign-in in the browser", "Signing in to WadSpaces in the browser", "Saving the sign-in"]
    );
    let done = ended(w, &job).await;
    assert_eq!(done.job.status, JobStatus::Done, "{:?}", done.job.error);
    assert_eq!(done.job.params["code"], "WADS-0601");
    assert_eq!(done.job.params["login"], "hunggod");

    // The token is the secret, and nowhere else.
    assert_eq!(up.token().as_deref(), Some(TOKEN));
    let calls = up.fake.calls();
    assert!(calls.contains(&"secret create github_token".to_string()), "{calls:?}");
    assert!(!calls.iter().any(|c| c.contains(TOKEN)), "{calls:?}");
    let log = w.job(&job.job.id, 0).await.unwrap();
    assert!(log.lines.iter().any(|l| l.contains("WADS-0601")), "{:?}", log.lines);
    assert!(!log.lines.iter().any(|l| l.contains(TOKEN)), "{:?}", log.lines);
    let kept = std::fs::read_to_string(up.dir.path().join("state/github.json")).unwrap();
    assert!(kept.contains("hunggod") && !kept.contains(TOKEN), "{kept}");

    // What GitHub was asked: the app's scopes, then the device code until the token came.
    let asked = up.asked("/login/device/code").await;
    assert_eq!(
        form(&asked[0]),
        [("client_id".into(), CLIENT_ID.into()), ("scope".into(), "repo workflow read:org".into())]
    );
    let polls = up.asked("/login/oauth/access_token").await;
    assert_eq!(polls.len(), 3);
    assert!(form(&polls[0]).contains(&("device_code".into(), "dc-0601".into())));

    // The window: opened with the code, closed when it was over; then whose it is.
    let events = up.until(|e| matches!(e, Event::Github { account: Some(_) })).await;
    let page = format!("{}/login/device", up.gh.uri());
    let opened = events.iter().position(|e| {
        matches!(e, Event::SignIn { provider: SignInProvider::Github, url, code: Some(c) } if *url == page && c == "WADS-0601")
    });
    let closed = events.iter().position(signin_done);
    assert!(opened.is_some() && opened < closed, "{events:?}");

    let w = &up.w;
    let a = w.github().await.unwrap().expect("signed in");
    assert_eq!((a.login.as_str(), a.name.as_str()), ("hunggod", "Hung"));
    assert_eq!(a.scopes, ["repo", "workflow", "read:org"]);
    // Kept: GitHub isn't asked again for a while.
    w.github().await.unwrap();
    assert_eq!(up.asked("/api/user").await.len(), 1);

    // The repos, kept for a minute.
    let repos = w.github_repos().await.unwrap();
    assert_eq!(repos.iter().map(|r| r.full_name.as_str()).collect::<Vec<_>>(), ["HungGod/WadSpaces", "HungGod/wados"]);
    assert_eq!(repos[0].pushed_at, Some(1_793_491_200_000));
    w.github_repos().await.unwrap();
    assert_eq!(up.asked("/api/user/repos").await.len(), 1);

    // Signed out: the secret's gone, and so is the account.
    w.github_signout().await.unwrap();
    assert_eq!(up.token(), None);
    assert!(up.fake.calls().contains(&"secret delete github_token".to_string()));
    assert_eq!(w.github().await.unwrap(), None);
    assert_eq!(w.github_repos().await.unwrap_err().status, 401);
    assert!(!up.dir.path().join("state/github.json").exists());
    up.until(|e| matches!(e, Event::Github { account: None })).await;
}

#[tokio::test]
async fn a_second_sign_in_asks_for_more_and_keeps_what_the_token_had() {
    let up = up().await;
    up.device_code(0).await;
    up.polls(vec![ok(json!({"access_token": TOKEN, "scope": "repo,workflow,read:org,write:packages"}))]).await;
    up.api("repo, workflow, read:org, write:packages").await;
    let w = &up.w;
    let job = w.github_signin(&SignInRequest::default()).await.unwrap();
    assert_eq!(ended(w, &job).await.job.status, JobStatus::Done);
    // A push needs write:packages: the second sign-in asks for it, and the rest again.
    let job = w.github_signin(&SignInRequest { scopes: vec!["write:packages".into()] }).await.unwrap();
    assert_eq!(ended(w, &job).await.job.status, JobStatus::Done);
    let asked = up.asked("/login/device/code").await;
    assert_eq!(form(&asked[1])[1], ("scope".into(), "repo workflow read:org write:packages".into()));
    assert!(w.github().await.unwrap().unwrap().has_scope("write:packages"));
    // The secret was replaced, not added to.
    assert_eq!(up.fake.calls().iter().filter(|c| *c == "secret create github_token").count(), 2);
}

#[tokio::test]
async fn sign_ins_that_end_without_a_token_say_why() {
    for (error, says) in [
        ("access_denied", "The sign-in was cancelled on GitHub"),
        ("expired_token", "The code expired before the sign-in was finished on GitHub"),
    ] {
        let mut up = up().await;
        up.device_code(0).await;
        up.polls(vec![oauth_error("authorization_pending"), oauth_error(error)]).await;
        let job = up.w.github_signin(&SignInRequest::default()).await.unwrap();
        let done = ended(&up.w, &job).await;
        assert_eq!(done.job.status, JobStatus::Error, "{error}");
        let why = done.job.error.clone().unwrap_or_default();
        assert!(why.starts_with(says), "{error}: {why}");
        assert_eq!(up.token(), None, "{error}");
        // Its window opened, and closed again.
        let events = up.until(signin_done).await;
        assert!(events.iter().any(signin), "{error}: {events:?}");
        let held = done.job.stages.iter().find(|s| s.id == "signin").unwrap();
        assert!(!held.held, "{error}: still held");
    }

    // The app without the device flow: no code, no window, and what to do about it.
    let mut up = up().await;
    Mock::given(method("POST"))
        .and(path("/login/device/code"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error": "device_flow_disabled", "error_description": "x"})),
        )
        .mount(&up.gh)
        .await;
    let job = up.w.github_signin(&SignInRequest::default()).await.unwrap();
    let done = ended(&up.w, &job).await;
    assert_eq!(done.job.status, JobStatus::Error);
    let why = done.job.error.clone().unwrap_or_default();
    assert!(why.contains("\"Enable Device Flow\" ticked"), "{why}");
    assert!(up.asked("/login/oauth/access_token").await.is_empty());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!up.heard().iter().any(signin));
}

#[tokio::test]
async fn cancelling_the_sign_in_stops_polling_and_closes_its_window() {
    let mut up = up().await;
    up.device_code(1).await;
    up.polls(vec![oauth_error("authorization_pending")]).await;
    let job = up.w.github_signin(&SignInRequest::default()).await.unwrap();
    up.until(signin).await;
    // The same ask again is the same sign-in, its window asked for again.
    let again = up.w.github_signin(&SignInRequest::default()).await.unwrap();
    assert_eq!(again.job.id, job.job.id);
    up.until(signin).await;
    let j = up.w.job(&job.job.id, 0).await.unwrap().job;
    assert!(j.job.stages.iter().any(|s| s.id == "signin" && s.held), "{:?}", j.job.stages);

    up.w.cancel_job(&job.job.id).await.unwrap();
    up.until(signin_done).await;
    assert_eq!(ended(&up.w, &job).await.job.status, JobStatus::Cancelled);
    let polled = up.asked("/login/oauth/access_token").await.len();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(up.asked("/login/oauth/access_token").await.len(), polled, "it stopped polling");
    assert_eq!(up.token(), None);
}

#[tokio::test]
async fn another_ask_replaces_the_sign_in_going_on() {
    let mut up = up().await;
    up.device_code(1).await;
    up.polls(vec![oauth_error("authorization_pending")]).await;
    let first = up.w.github_signin(&SignInRequest::default()).await.unwrap();
    up.until(signin).await;
    let second = up.w.github_signin(&SignInRequest { scopes: vec!["write:packages".into()] }).await.unwrap();
    assert_ne!(first.job.id, second.job.id);
    assert_eq!(ended(&up.w, &first).await.job.status, JobStatus::Cancelled);
    // The first one's window closes before the second's opens.
    let events = up.until(signin).await;
    assert!(events.iter().any(signin_done), "{events:?}");
    let j = up.w.job(&second.job.id, 0).await.unwrap().job;
    assert_eq!(j.job.status, JobStatus::Running);
    up.w.cancel_job(&second.job.id).await.unwrap();
}

#[tokio::test]
async fn a_token_github_rejects_is_no_account_and_a_known_one_survives_github_being_away() {
    let up = up().await;
    up.api("repo").await;
    // Someone else's token, or one revoked on GitHub.
    up.fake.state.lock().unwrap().secrets.insert("github_token".into(), "gho_revoked".into());
    assert_eq!(up.w.github().await.unwrap(), None);
    let e = up.w.github_repos().await.unwrap_err();
    assert_eq!(e.status, 401, "{}", e.message);
    // A known one: GitHub said whose it was before wadd restarted, and is away now.
    let hash: String = {
        use sha2::Digest;
        sha2::Sha256::digest(TOKEN.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
    };
    std::fs::write(
        up.dir.path().join("state/github.json"),
        json!({"token": hash, "account": {"login": "hunggod", "name": "Hung", "scopes": ["repo"]}}).to_string(),
    )
    .unwrap();
    up.fake.state.lock().unwrap().secrets.insert("github_token".into(), TOKEN.into());
    up.gh.reset().await;
    Mock::given(method("GET")).and(path("/api/user")).respond_with(ResponseTemplate::new(503)).mount(&up.gh).await;
    let a = up.w.github().await.unwrap().expect("as last known");
    assert_eq!(a.login, "hunggod");
}

#[tokio::test]
async fn the_machines_browser_gets_the_web_keys_and_their_folders() {
    let up = up().await;
    let made =
        up.w.op(Op::SaveWebKey { name: "hunggod".into(), site: "github.com".into(), existing: None })
            .await
            .unwrap()
            .created
            .expect("a key");
    let keys: Vec<WebKey> = up.w.get("/v1/keys/web").await.unwrap();
    assert_eq!(keys.len(), 1);
    let k = &keys[0];
    assert_eq!((k.id.as_str(), k.sites.as_slice()), (made.as_str(), &["github.com".to_string()][..]));
    let dir = std::path::Path::new(&k.dir);
    assert_eq!(dir, up.dir.path().join("state/keys").join(&made));
    assert!(dir.is_dir());
}
