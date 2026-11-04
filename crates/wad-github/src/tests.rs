//! Against a mock GitHub (wiremock), never the real one.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use super::device::GRANT_TYPE;
use super::*;
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn ok(body: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

/// Both of GitHub's addresses on the mock: the sign-in at its root, the API under /api.
async fn server() -> (MockServer, Github) {
    let s = MockServer::start().await;
    let gh = Github::with_bases(reqwest::Client::new(), &s.uri(), &format!("{}/api", s.uri()));
    (s, gh)
}

// ------------------------------------------------------------ the REST calls

#[tokio::test]
async fn whose_a_token_is_what_it_may_do_and_revoked_tokens() {
    let (s, gh) = server().await;
    Mock::given(method("GET"))
        .and(path("/api/user"))
        .and(header("authorization", "Bearer good"))
        .respond_with(
            ok(json!({"login": "octocat", "name": "Octo Cat", "id": 1}))
                .insert_header("x-oauth-scopes", "repo, workflow, read:org"),
        )
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/user"))
        .and(header("authorization", "Bearer nameless"))
        .respond_with(ok(json!({"login": "ghost", "name": null})))
        .mount(&s)
        .await;
    Mock::given(method("GET")).and(path("/api/user")).respond_with(ResponseTemplate::new(401)).mount(&s).await;
    let u = gh.user(&Token::new("good")).await.unwrap();
    assert_eq!(format!("{:?}", Token::new("good")), "Token(…)");
    assert_eq!((u.login.as_str(), u.name.as_str()), ("octocat", "Octo Cat"));
    assert_eq!(u.scopes, ["repo", "workflow", "read:org"]);
    // No name: its login; no scopes header: none.
    let u = gh.user(&Token::new("nameless")).await.unwrap();
    assert_eq!((u.name.as_str(), u.scopes.len()), ("ghost", 0));
    assert!(matches!(gh.user(&Token::new("revoked")).await, Err(Error::BadToken)));
}

#[test]
fn errors_map_to_api_codes() {
    assert_eq!(ApiError::from(Error::Invalid("taken".into())).code, ErrorCode::BadRequest);
    assert_eq!(ApiError::from(Error::BadToken).code, ErrorCode::Unauthorized);
    assert_eq!(ApiError::from(Error::Denied).code, ErrorCode::Cancelled);
    assert_eq!(ApiError::from(Error::DeviceFlowDisabled).code, ErrorCode::Upstream);
    assert_eq!(split_scopes("repo,gist"), ["repo", "gist"]);
    assert_eq!(split_scopes(" repo, workflow "), ["repo", "workflow"]);
    assert!(split_scopes("").is_empty());
}

#[test]
fn reads_the_machine_config() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../host/usr/lib/wados/github.toml");
    let app = AppConfig::load(path.as_ref()).unwrap();
    assert_eq!(app.client_id, "Ov23lia8qVymk05q5E4i");
    assert_eq!(app.scopes, FIRST_SCOPES);
    // No scopes: the first sign-in's; no client ID: not an app.
    let dir = std::env::temp_dir().join(format!("wad-github-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.toml"), "client_id = \"x\"\n").unwrap();
    assert_eq!(AppConfig::load(&dir.join("a.toml")).unwrap().scopes, FIRST_SCOPES);
    std::fs::write(dir.join("b.toml"), "client_id = \"\"\n").unwrap();
    assert!(AppConfig::load(&dir.join("b.toml")).unwrap_err().contains("no client_id"));
    assert!(AppConfig::load(&dir.join("missing.toml")).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn every_page_of_repos() {
    let (s, gh) = server().await;
    let api = format!("{}/api", s.uri());
    let repo = |n: &str| json!({"full_name": format!("HungGod/{n}"), "name": n, "private": true, "clone_url": format!("https://github.com/HungGod/{n}.git"), "default_branch": "main", "pushed_at": "2026-10-01T00:00:00Z", "description": null});
    Mock::given(method("GET"))
        .and(path("/api/user/repos"))
        .and(query_param("page", "2"))
        .respond_with(ok(json!([repo("two")])))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/user/repos"))
        .and(query_param("affiliation", "owner,collaborator,organization_member"))
        .respond_with(ok(json!([repo("one")])).insert_header(
            "link",
            format!("<{api}/user/repos?page=2>; rel=\"next\", <{api}/user/repos?page=2>; rel=\"last\"").as_str(),
        ))
        .mount(&s)
        .await;
    let repos = gh.repos(&Token::new("gho_x")).await.unwrap();
    let names: Vec<&str> = repos.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["one", "two"]);
    let r = &repos[0];
    assert_eq!((r.url.as_str(), r.private, r.description.as_str()), ("https://github.com/HungGod/one.git", true, ""));
    assert_eq!(r.pushed_at, Some(1_790_812_800_000));
}

#[tokio::test]
async fn making_a_repo_and_what_github_says_when_it_wont() {
    let (s, gh) = server().await;
    Mock::given(method("POST"))
        .and(path("/api/user/repos"))
        .and(body_partial_json(json!({"name": "notes", "private": true, "auto_init": true})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"full_name": "HungGod/notes", "name": "notes", "private": true, "clone_url": "https://github.com/HungGod/notes.git", "default_branch": "main"})))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/user/repos"))
        .and(body_partial_json(json!({"name": "taken"})))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({"message": "Repository creation failed.", "errors": [{"resource": "Repository", "code": "custom", "field": "name", "message": "name already exists on this account"}]})))
        .mount(&s)
        .await;
    Mock::given(method("GET")).and(path("/api/user/repos")).respond_with(ResponseTemplate::new(401)).mount(&s).await;
    let tok = Token::new("gho_x");
    let r = gh.create_repo(&tok, "notes", true, "").await.unwrap();
    assert_eq!(r.full_name, "HungGod/notes");
    let e = gh.create_repo(&tok, "taken", true, "").await.unwrap_err();
    assert_eq!(e.to_string(), "name already exists on this account");
    assert!(matches!(gh.repos(&tok).await, Err(Error::BadToken)));
}

// ------------------------------------------------------------ the device flow

/// A clock whose sleeps return at once, moving it on: it says what they were (s).
#[derive(Default)]
struct Steps {
    at: Mutex<Duration>,
    slept: Mutex<Vec<u64>>,
}

impl Clock for Steps {
    fn now(&self) -> Duration {
        *self.at.lock().unwrap()
    }
    fn sleep(&self, d: Duration) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        *self.at.lock().unwrap() += d;
        self.slept.lock().unwrap().push(d.as_secs());
        Box::pin(std::future::ready(()))
    }
}

impl Steps {
    fn slept(&self) -> Vec<u64> {
        self.slept.lock().unwrap().clone()
    }
}

/// Each answer in turn; the last one again after that.
struct Answers(Mutex<VecDeque<ResponseTemplate>>);

impl Answers {
    fn new(all: Vec<ResponseTemplate>) -> Self {
        Self(Mutex::new(all.into()))
    }
}

impl Respond for Answers {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let mut q = self.0.lock().unwrap();
        if q.len() > 1 { q.pop_front().unwrap() } else { q.front().cloned().unwrap() }
    }
}

fn oauth_error(error: &str) -> ResponseTemplate {
    // GitHub answers the device flow's errors with 200.
    ok(
        json!({"error": error, "error_description": format!("{error}, as GitHub says it"), "error_uri": "https://docs.github.com"}),
    )
}

fn pending() -> ResponseTemplate {
    oauth_error("authorization_pending")
}

fn granted(token: &str) -> ResponseTemplate {
    ok(json!({"access_token": token, "token_type": "bearer", "scope": "repo,workflow,read:org"}))
}

fn code(interval: u64) -> DeviceCode {
    DeviceCode {
        device_code: "3584d83530557fdd1f46af8289938c8ef79f9dc5".into(),
        user_code: "WDJB-MJHT".into(),
        verification_uri: "https://github.com/login/device".into(),
        expires_in: 900,
        interval,
    }
}

/// The form a request sent.
fn form(r: &Request) -> Vec<(String, String)> {
    url::form_urlencoded::parse(&r.body).into_owned().collect()
}

async fn polls_answering(answers: Vec<ResponseTemplate>) -> (MockServer, Github) {
    let (s, gh) = server().await;
    Mock::given(method("POST"))
        .and(path("/login/oauth/access_token"))
        .and(header("accept", "application/json"))
        .respond_with(Answers::new(answers))
        .mount(&s)
        .await;
    (s, gh)
}

#[tokio::test]
async fn a_device_code_for_the_apps_scopes() {
    let (s, gh) = server().await;
    Mock::given(method("POST"))
        .and(path("/login/device/code"))
        .and(header("accept", "application/json"))
        .respond_with(ok(json!({
            "device_code": "3584d83530557fdd1f46af8289938c8ef79f9dc5", "user_code": "WDJB-MJHT",
            "verification_uri": "https://github.com/login/device", "expires_in": 900, "interval": 5
        })))
        .mount(&s)
        .await;
    let scopes = FIRST_SCOPES.map(String::from).to_vec();
    let c = gh.device_code("Ov23lia8qVymk05q5E4i", &scopes).await.unwrap();
    assert_eq!(c, code(5));
    assert!(!format!("{c:?}").contains("3584d8"), "the device code stays out of logs: {c:?}");
    let asked = s.received_requests().await.unwrap();
    assert_eq!(
        form(&asked[0]),
        [("client_id".into(), "Ov23lia8qVymk05q5E4i".into()), ("scope".into(), "repo workflow read:org".into())]
    );
}

#[tokio::test]
async fn no_device_code_when_the_app_has_no_device_flow_or_isnt_one() {
    let (s, gh) = server().await;
    Mock::given(method("POST"))
        .and(path("/login/device/code"))
        .and(wiremock::matchers::body_string_contains("client_id=nodevice"))
        .respond_with(ResponseTemplate::new(400).set_body_json(
            json!({"error": "device_flow_disabled", "error_description": "Device flow is disabled for this app."}),
        ))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/login/device/code"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Not Found"))
        .mount(&s)
        .await;
    let e = gh.device_code("nodevice", &[]).await.unwrap_err();
    assert!(matches!(e, Error::DeviceFlowDisabled), "{e:?}");
    assert!(e.to_string().contains("\"Enable Device Flow\" ticked"), "{e}");
    assert!(matches!(gh.device_code("nosuchapp", &[]).await, Err(Error::UnknownApp)));
}

#[tokio::test]
async fn polls_until_the_person_has_signed_in() {
    let (s, gh) = polls_answering(vec![pending(), pending(), granted("gho_new")]).await;
    let clock = Steps::default();
    let g = gh.poll("Ov23lia8qVymk05q5E4i", &code(5), &clock).await.unwrap();
    assert_eq!(g.token.expose(), "gho_new");
    assert_eq!(g.scopes, ["repo", "workflow", "read:org"]);
    // It waits the interval before every ask, the first too.
    assert_eq!(clock.slept(), [5, 5, 5]);
    let asked = s.received_requests().await.unwrap();
    assert_eq!(asked.len(), 3);
    assert_eq!(
        form(&asked[0]),
        [
            ("client_id".into(), "Ov23lia8qVymk05q5E4i".into()),
            ("device_code".into(), "3584d83530557fdd1f46af8289938c8ef79f9dc5".into()),
            ("grant_type".into(), GRANT_TYPE.into()),
        ]
    );
}

#[tokio::test]
async fn slow_down_adds_five_seconds_or_takes_githubs_interval() {
    let slow = |interval: Option<u64>| match interval {
        Some(i) => ok(json!({"error": "slow_down", "error_description": "Too many requests", "interval": i})),
        None => oauth_error("slow_down"),
    };
    let (_s, gh) = polls_answering(vec![pending(), slow(None), pending(), slow(Some(20)), granted("t")]).await;
    let clock = Steps::default();
    gh.poll("app", &code(5), &clock).await.unwrap();
    // 5, then 5 more for every later ask; then GitHub's 20 (more than 15).
    assert_eq!(clock.slept(), [5, 5, 10, 10, 20]);
    // GitHub's own interval when it's less than 5 more is 5 more.
    let (_s, gh) = polls_answering(vec![slow(Some(6)), granted("t")]).await;
    let clock = Steps::default();
    gh.poll("app", &code(5), &clock).await.unwrap();
    assert_eq!(clock.slept(), [5, 10]);
}

#[tokio::test]
async fn each_way_a_sign_in_ends_without_a_token() {
    for (error, want) in [
        ("expired_token", "expired"),
        ("access_denied", "cancelled on GitHub"),
        ("incorrect_device_code", "didn't know the sign-in's code"),
        ("device_flow_disabled", "\"Enable Device Flow\""),
        ("incorrect_client_credentials", "client ID"),
        ("unsupported_grant_type", "unsupported_grant_type, as GitHub says it (unsupported_grant_type)"),
    ] {
        let (s, gh) = polls_answering(vec![pending(), oauth_error(error)]).await;
        let clock = Steps::default();
        let e = gh.poll("app", &code(5), &clock).await.unwrap_err();
        assert!(e.to_string().contains(want), "{error}: {e}");
        // It stops at the end: no more asking.
        assert_eq!(s.received_requests().await.unwrap().len(), 2, "{error}");
    }
}

#[tokio::test]
async fn the_code_expires_here_too_and_a_blip_isnt_an_end() {
    // Pending until its time runs out: it gives up without another ask.
    let (s, gh) = polls_answering(vec![pending()]).await;
    let clock = Steps::default();
    let c = DeviceCode { expires_in: 12, ..code(5) };
    assert!(matches!(gh.poll("app", &c, &clock).await, Err(Error::Expired)));
    assert_eq!(clock.slept(), [5, 5, 5]);
    assert_eq!(s.received_requests().await.unwrap().len(), 2);
    // GitHub's trouble (a 502, a page that isn't JSON) is asked again.
    let (_s, gh) = polls_answering(vec![
        ResponseTemplate::new(502).set_body_string("<html>bad gateway</html>"),
        pending(),
        granted("after"),
    ])
    .await;
    let g = gh.poll("app", &code(1), &Steps::default()).await.unwrap();
    assert_eq!(g.token.expose(), "after");
    // An answer that's neither is an end.
    let (_s, gh) = polls_answering(vec![ok(json!({"something": "else"}))]).await;
    let e = gh.poll("app", &code(1), &Steps::default()).await.unwrap_err();
    assert!(e.to_string().contains("neither a token nor an error"), "{e}");
}

#[tokio::test]
async fn polling_with_tokios_clock_honours_the_interval() {
    let (_s, gh) = polls_answering(vec![pending(), granted("t")]).await;
    let started = std::time::Instant::now();
    gh.poll("app", &code(0), &RealClock::default()).await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
}
