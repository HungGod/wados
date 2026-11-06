//! Against a mock only: nothing here reaches Cloudflare.
use serde_json::json;
use wiremock::matchers::{bearer_token, body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const FILE: &str = r#"
[cloudflare.tunnel]
dns_edit_api_token = "dns-key"
tunnel_creator_api_token = "tunnel-key"
[cloudflare.turn]
token_id = "turn-id"
api_token = "turn-key"
"#;

fn ok(result: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "errors": [], "result": result }))
}

async fn zone(s: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/zones"))
        .and(query_param("name", "wadspaces.com"))
        .and(bearer_token("dns-key"))
        .respond_with(ok(json!([{ "id": "z1", "account": { "id": "a1" } }])))
        .mount(s)
        .await;
}

async fn tunnels(s: &MockServer, found: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/accounts/a1/cfd_tunnel"))
        .and(query_param("name", "wadspaces-studio"))
        .and(bearer_token("tunnel-key"))
        .respond_with(ok(found))
        .mount(s)
        .await;
}

async fn ingress(s: &MockServer, tunnel: &str) {
    Mock::given(method("PUT"))
        .and(path(format!("/accounts/a1/cfd_tunnel/{tunnel}/configurations")))
        .and(bearer_token("tunnel-key"))
        .and(body_partial_json(json!({ "config": { "ingress": [
            { "hostname": "studio.wadspaces.com", "service": "http://localhost:8085" },
            { "service": "http_status:404" },
        ]}})))
        .respond_with(ok(json!({})))
        .expect(1)
        .mount(s)
        .await;
}

fn client(s: &MockServer) -> Cloudflare {
    Cloudflare::new(&s.uri(), "wadspaces.com", &Keys::parse(FILE).unwrap())
}

#[tokio::test]
async fn a_new_machine_gets_a_tunnel_and_one_proxied_record() {
    let s = MockServer::start().await;
    zone(&s).await;
    tunnels(&s, json!([])).await;
    Mock::given(method("POST"))
        .and(path("/accounts/a1/cfd_tunnel"))
        .and(bearer_token("tunnel-key"))
        .and(body_partial_json(json!({ "name": "wadspaces-studio", "config_src": "cloudflare" })))
        .respond_with(ok(json!({ "id": "t1", "token": "machine-token" })))
        .expect(1)
        .mount(&s)
        .await;
    ingress(&s, "t1").await;
    Mock::given(method("GET"))
        .and(path("/zones/z1/dns_records"))
        .and(query_param("name", "studio.wadspaces.com"))
        .and(bearer_token("dns-key"))
        .respond_with(ok(json!([])))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/zones/z1/dns_records"))
        .and(bearer_token("dns-key"))
        .and(body_partial_json(json!({
            "type": "CNAME", "name": "studio.wadspaces.com",
            "content": "t1.cfargotunnel.com", "proxied": true,
        })))
        .respond_with(ok(json!({ "id": "r1" })))
        .expect(1)
        .mount(&s)
        .await;
    let a = client(&s).provision("studio").await.unwrap();
    assert_eq!(
        (a.host.as_str(), a.tunnel_id.as_str(), a.token.as_str()),
        ("studio.wadspaces.com", "t1", "machine-token")
    );
}

#[tokio::test]
async fn an_install_run_again_reuses_the_idle_tunnel_and_its_record() {
    let s = MockServer::start().await;
    zone(&s).await;
    tunnels(&s, json!([{ "id": "t1", "status": "inactive" }])).await;
    Mock::given(method("GET"))
        .and(path("/accounts/a1/cfd_tunnel/t1/token"))
        .respond_with(ok(json!("machine-token")))
        .mount(&s)
        .await;
    ingress(&s, "t1").await;
    Mock::given(method("GET"))
        .and(path("/zones/z1/dns_records"))
        .respond_with(ok(json!([{ "id": "r1" }])))
        .mount(&s)
        .await;
    Mock::given(method("PUT"))
        .and(path("/zones/z1/dns_records/r1"))
        .respond_with(ok(json!({})))
        .expect(1)
        .mount(&s)
        .await;
    let a = client(&s).provision("studio").await.unwrap();
    assert_eq!(a.token, "machine-token");
}

#[tokio::test]
async fn a_name_thats_connected_is_refused_and_changes_nothing() {
    let s = MockServer::start().await;
    zone(&s).await;
    tunnels(&s, json!([{ "id": "t1", "status": "healthy" }])).await;
    // No ingress or record calls: wiremock answers those 404, which would be another error.
    let e = client(&s).provision("studio").await.unwrap_err();
    assert!(matches!(e, Error::Taken(_)));
    assert_eq!(e.to_string(), "A machine called studio is connected already: give this one another name");
}

#[tokio::test]
async fn cloudflares_refusal_is_its_own_words() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/zones"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(json!({ "success": false, "errors": [{ "message": "Authentication error" }] })),
        )
        .mount(&s)
        .await;
    assert_eq!(client(&s).provision("studio").await.unwrap_err().to_string(), "Cloudflare said: Authentication error");
}

#[tokio::test]
async fn retiring_a_machine_removes_its_record_and_tunnel() {
    let s = MockServer::start().await;
    zone(&s).await;
    tunnels(&s, json!([{ "id": "t1", "status": "inactive" }])).await;
    Mock::given(method("GET"))
        .and(path("/zones/z1/dns_records"))
        .respond_with(ok(json!([{ "id": "r1" }])))
        .mount(&s)
        .await;
    for p in ["/zones/z1/dns_records/r1", "/accounts/a1/cfd_tunnel/t1/connections", "/accounts/a1/cfd_tunnel/t1"] {
        Mock::given(method("DELETE")).and(path(p)).respond_with(ok(json!({}))).expect(1).mount(&s).await;
    }
    assert!(client(&s).remove("studio").await.unwrap());
}

#[test]
fn the_turn_pair_is_the_only_key_the_machine_gets() {
    let turn = Keys::parse(FILE).unwrap().turn_file().unwrap();
    assert!(turn.contains("turn-id") && turn.contains("turn-key"));
    assert!(!turn.contains("dns-key") && !turn.contains("tunnel-key"));
    let parsed: toml::Table = turn.parse().unwrap();
    assert_eq!(parsed["cloudflare"]["turn"]["token_id"].as_str(), Some("turn-id"));
}

#[test]
fn neither_keys_nor_tokens_show_in_debug_or_errors() {
    let k = Keys::parse(FILE).unwrap();
    assert!(!format!("{k:?}").contains("key"));
    let a = Address { host: "h".into(), tunnel_id: "t".into(), token: "machine-token".into() };
    assert!(!format!("{a:?}").contains("machine-token"));
    let e = Keys::parse("[cloudflare.tunnel]\ndns_edit_api_token = \"a-secret-value\"").unwrap_err();
    assert!(!e.to_string().contains("a-secret-value"));
}
