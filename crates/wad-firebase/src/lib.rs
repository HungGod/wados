//! Firebase over its REST APIs, from Rust: Firestore (values.rs has the
//! typed values), Auth sign-in (identity.rs) and callable functions
//! (functions.rs). The app writes the values that mustn't pass through
//! JavaScript (a GitHub token) with the user's ID token; wadd's cloud relay
//! signs in as its machine. Either way Firestore's rules apply exactly as they
//! do to the JS SDK.

pub mod functions;
pub mod identity;
pub mod values;

use serde_json::{Map, Value};
use wad_proto::{ApiError, ErrorCode};

pub use functions::Functions;
pub use identity::{Claims, Identity, IdpSignIn, Tokens, claims};

const FIRESTORE: &str = "https://firestore.googleapis.com";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a valid Firebase project id: {0:?}")]
    BadProject(String),
    #[error("not a valid document path: {0:?}")]
    BadPath(String),
    #[error("signed out, or the sign-in expired")]
    Unauthenticated,
    #[error("Firestore refused the write (rules)")]
    Denied,
    #[error("couldn't reach Firestore: {0}")]
    Network(#[source] reqwest::Error),
    #[error("Firestore answered {status}: {message}")]
    Upstream { status: u16, message: String },
    #[error("sign-in refused ({status}): {message}")]
    Auth { status: u16, message: String },
    #[error("{message}")]
    Callable { status: String, message: String },
    /// A commit's precondition didn't hold (the document was there, or wasn't).
    #[error("Firestore refused the change: {0}")]
    Precondition(String),
}

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        let code = match e {
            Error::BadProject(_) | Error::BadPath(_) => ErrorCode::BadRequest,
            Error::Unauthenticated => ErrorCode::Unauthorized,
            Error::Denied => ErrorCode::Forbidden,
            Error::Network(_) => ErrorCode::Offline,
            Error::Upstream { .. } | Error::Callable { .. } => ErrorCode::Upstream,
            Error::Precondition(_) => ErrorCode::Conflict,
            Error::Auth { .. } => ErrorCode::Unauthorized,
        };
        ApiError::new(code, e.to_string())
    }
}

/// Firestore's REST API for one project.
#[derive(Debug, Clone)]
pub struct Firestore {
    http: reqwest::Client,
    base: String,
    project: String,
}

impl Firestore {
    pub fn new(http: reqwest::Client, project: &str) -> Result<Self, Error> {
        Self::with_base(http, FIRESTORE, project)
    }

    /// Against the emulator (`http://127.0.0.1:8090`) or a test server.
    pub fn with_base(http: reqwest::Client, base: &str, project: &str) -> Result<Self, Error> {
        let ok = !project.is_empty()
            && project.len() <= 64
            && project.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !ok {
            return Err(Error::BadProject(project.into()));
        }
        Ok(Self { http, base: base.trim_end_matches('/').into(), project: project.into() })
    }

    /// Writes a document whose fields are all strings, replacing what was
    /// there. `path` is like `users/<uid>/secrets/github_token`.
    pub async fn set_strings(&self, id_token: &str, path: &str, fields: &[(&str, &str)]) -> Result<(), Error> {
        check_path(path, true)?;
        let body = serde_json::json!({
            "fields": fields
                .iter()
                .map(|(k, v)| (k.to_string(), serde_json::json!({ "stringValue": v })))
                .collect::<serde_json::Map<_, _>>(),
        });
        self.send(self.http.patch(self.url(path)).bearer_auth(id_token).json(&body)).await.map(drop)
    }

    fn url(&self, path: &str) -> String {
        format!("{}/v1/projects/{}/databases/(default)/documents/{path}", self.base, self.project)
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value, Error> {
        let res = req.send().await.map_err(Error::Network)?;
        match res.status().as_u16() {
            200..=299 => Ok(res.json().await.unwrap_or(Value::Null)),
            401 => Err(Error::Unauthenticated),
            403 => Err(Error::Denied),
            status => {
                let text = res.text().await.unwrap_or_default();
                Err(Error::Upstream { status, message: text.chars().take(200).collect() })
            }
        }
    }

    /// A document (REST form); None if there's none.
    pub async fn get(&self, id_token: &str, path: &str) -> Result<Option<Value>, Error> {
        check_path(path, true)?;
        match self.send(self.http.get(self.url(path)).bearer_auth(id_token)).await {
            Ok(d) => Ok(Some(d)),
            Err(Error::Upstream { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Every document of a collection (REST form), page by page.
    pub async fn list(&self, id_token: &str, collection: &str) -> Result<Vec<Value>, Error> {
        check_path(collection, false)?;
        let mut docs = vec![];
        let mut page: Option<String> = None;
        loop {
            let mut q: Vec<(&str, String)> = vec![("pageSize", "300".into())];
            if let Some(p) = &page {
                q.push(("pageToken", p.clone()));
            }
            let body = self.send(self.http.get(self.url(collection)).bearer_auth(id_token).query(&q)).await?;
            docs.extend(body.get("documents").and_then(Value::as_array).cloned().unwrap_or_default());
            page = body.get("nextPageToken").and_then(Value::as_str).map(String::from);
            if page.is_none() {
                return Ok(docs);
            }
        }
    }

    /// Sets these fields (and removes the `remove` ones), leaving the rest;
    /// creates the document if need be.
    pub async fn patch(
        &self,
        id_token: &str,
        path: &str,
        data: &Map<String, Value>,
        remove: &[&str],
    ) -> Result<(), Error> {
        check_path(path, true)?;
        let mask: Vec<(&str, &str)> = data
            .keys()
            .map(String::as_str)
            .chain(remove.iter().copied())
            .map(|k| ("updateMask.fieldPaths", k))
            .collect();
        let body = serde_json::json!({ "fields": values::to_fields(data) });
        self.send(self.http.patch(self.url(path)).bearer_auth(id_token).query(&mask).json(&body)).await.map(drop)
    }

    /// Replaces a document with these fields (creating it if need be).
    pub async fn set(&self, id_token: &str, path: &str, data: &Map<String, Value>) -> Result<(), Error> {
        check_path(path, true)?;
        let body = serde_json::json!({ "fields": values::to_fields(data) });
        self.send(self.http.patch(self.url(path)).bearer_auth(id_token).json(&body)).await.map(drop)
    }

    /// Writes all of `writes` or none (a transaction without reads): a
    /// precondition that fails fails them all (Error::Precondition), and
    /// the rules see the writes together (`getAfter`).
    pub async fn commit(&self, id_token: &str, writes: &[Write]) -> Result<(), Error> {
        let body =
            serde_json::json!({ "writes": writes.iter().map(|w| w.rest(&self.name_of(&w.path))).collect::<Vec<_>>() });
        for w in writes {
            check_path(&w.path, true)?;
        }
        let url = format!("{}/v1/projects/{}/databases/(default)/documents:commit", self.base, self.project);
        match self.send(self.http.post(url).bearer_auth(id_token).json(&body)).await {
            Ok(_) => Ok(()),
            Err(Error::Upstream { status: 400 | 404 | 409, message })
                if message.contains("FAILED_PRECONDITION")
                    || message.contains("ALREADY_EXISTS")
                    || message.contains("NOT_FOUND") =>
            {
                Err(Error::Precondition(message))
            }
            Err(e) => Err(e),
        }
    }

    /// A document's full name, as a write names it.
    fn name_of(&self, path: &str) -> String {
        format!("projects/{}/databases/(default)/documents/{path}", self.project)
    }

    pub async fn delete(&self, id_token: &str, path: &str) -> Result<(), Error> {
        check_path(path, true)?;
        self.send(self.http.delete(self.url(path)).bearer_auth(id_token)).await.map(drop)
    }

    /// Runs a structured query on top-level collections (`from` names them):
    /// the documents.
    pub async fn query_root(&self, id_token: &str, structured: Value) -> Result<Vec<Value>, Error> {
        let body = serde_json::json!({ "structuredQuery": structured });
        let url = format!("{}/v1/projects/{}/databases/(default)/documents:runQuery", self.base, self.project);
        let rows = self.send(self.http.post(url).bearer_auth(id_token).json(&body)).await?;
        Ok(rows.as_array().into_iter().flatten().filter_map(|r| r.get("document").cloned()).collect())
    }

    /// Runs a structured query on a document's subcollections: the documents.
    pub async fn query(&self, id_token: &str, parent: &str, structured: Value) -> Result<Vec<Value>, Error> {
        check_path(parent, true)?;
        let body = serde_json::json!({ "structuredQuery": structured });
        let rows = self
            .send(self.http.post(format!("{}:runQuery", self.url(parent))).bearer_auth(id_token).json(&body))
            .await?;
        Ok(rows.as_array().into_iter().flatten().filter_map(|r| r.get("document").cloned()).collect())
    }
}

/// One write of a commit: a document set to these fields (`$time` values as
/// in values.rs), with server timestamps and a precondition if wanted.
#[derive(Debug, Clone)]
pub struct Write {
    pub path: String,
    pub data: Map<String, Value>,
    /// Fields set to the time the server commits (Firestore's REQUEST_TIME).
    pub server_time: Vec<String>,
    /// Some(false): only if it isn't there yet; Some(true): only if it is.
    pub exists: Option<bool>,
    /// Only these fields changed (the rest as they are), not the whole
    /// document set.
    pub mask: Option<Vec<String>>,
}

impl Write {
    pub fn set(path: impl Into<String>, data: Map<String, Value>) -> Self {
        Self { path: path.into(), data, server_time: vec![], exists: None, mask: None }
    }

    /// Only `data`'s fields changed (and the server times): the document
    /// must be there.
    pub fn update(path: impl Into<String>, data: Map<String, Value>) -> Self {
        let mask = data.keys().cloned().collect();
        Self { path: path.into(), data, server_time: vec![], exists: Some(true), mask: Some(mask) }
    }

    /// Only if the document isn't there yet.
    pub fn creating(mut self) -> Self {
        self.exists = Some(false);
        self
    }

    pub fn now(mut self, field: &str) -> Self {
        self.server_time.push(field.into());
        self
    }

    fn rest(&self, name: &str) -> Value {
        let mut w = serde_json::json!({ "update": { "name": name, "fields": values::to_fields(&self.data) } });
        if !self.server_time.is_empty() {
            w["updateTransforms"] = self
                .server_time
                .iter()
                .map(|f| serde_json::json!({ "fieldPath": f, "setToServerValue": "REQUEST_TIME" }))
                .collect();
        }
        if let Some(e) = self.exists {
            w["currentDocument"] = serde_json::json!({ "exists": e });
        }
        if let Some(m) = &self.mask {
            w["updateMask"] = serde_json::json!({ "fieldPaths": m });
        }
        w
    }
}

/// A document path (`document`: an even number of segments) or a
/// collection's (odd), made of ids Firestore and WadSpaces use.
fn check_path(path: &str, document: bool) -> Result<(), Error> {
    let segments: Vec<&str> = path.split('/').collect();
    let ok = segments.len().is_multiple_of(2) == document
        && segments.iter().all(|s| {
            !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        });
    if ok { Ok(()) } else { Err(Error::BadPath(path.into())) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn writes_a_secret() {
        let s = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/v1/projects/wad-spaces/databases/(default)/documents/users/u1/secrets/github_token"))
            .and(header("authorization", "Bearer idtok"))
            .and(body_json(json!({"fields": {"value": {"stringValue": "gho_abc"}}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&s)
            .await;
        Mock::given(method("PATCH")).respond_with(ResponseTemplate::new(403)).mount(&s).await;
        let fs = Firestore::with_base(reqwest::Client::new(), &s.uri(), "wad-spaces").unwrap();
        fs.set_strings("idtok", "users/u1/secrets/github_token", &[("value", "gho_abc")]).await.unwrap();
        let denied = fs.set_strings("idtok", "users/u2/secrets/github_token", &[("value", "x")]).await;
        assert!(matches!(denied, Err(Error::Denied)));
    }

    #[tokio::test]
    async fn commits_writes_together() {
        let s = MockServer::start().await;
        let doc = |p: &str| format!("projects/wad-spaces/databases/(default)/documents/{p}");
        Mock::given(method("POST"))
            .and(path("/v1/projects/wad-spaces/databases/(default)/documents:commit"))
            .and(body_json(json!({"writes": [
                {"update": {"name": doc("usernames/octo"), "fields": {"uid": {"stringValue": "u1"}}}, "currentDocument": {"exists": false}},
                {"update": {"name": doc("profiles/u1"), "fields": {"username": {"stringValue": "octo"}}},
                 "updateTransforms": [{"fieldPath": "createdAt", "setToServerValue": "REQUEST_TIME"}]}
            ]})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"writeResults": []})))
            .up_to_n_times(1)
            .mount(&s)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string(r#"{"error":{"status":"FAILED_PRECONDITION"}}"#))
            .mount(&s)
            .await;
        let fs = Firestore::with_base(reqwest::Client::new(), &s.uri(), "wad-spaces").unwrap();
        let o = |k: &str, v: &str| json!({ k: v }).as_object().unwrap().clone();
        let writes = [
            Write::set("usernames/octo", o("uid", "u1")).creating(),
            Write::set("profiles/u1", o("username", "octo")).now("createdAt"),
        ];
        fs.commit("t", &writes).await.unwrap();
        assert!(matches!(fs.commit("t", &writes).await, Err(Error::Precondition(_))));
    }

    #[test]
    fn an_update_changes_only_its_fields() {
        let w = Write::update("users/u1/lock/password", json!({"afterHours": 12}).as_object().unwrap().clone())
            .now("changedAt");
        assert_eq!(
            w.rest("doc"),
            json!({
                "update": {"name": "doc", "fields": {"afterHours": {"integerValue": "12"}}},
                "updateTransforms": [{"fieldPath": "changedAt", "setToServerValue": "REQUEST_TIME"}],
                "currentDocument": {"exists": true},
                "updateMask": {"fieldPaths": ["afterHours"]}
            })
        );
    }

    #[tokio::test]
    async fn refuses_odd_paths_and_projects() {
        let http = reqwest::Client::new();
        assert!(Firestore::new(http.clone(), "Wad Spaces").is_err());
        // Nothing listens here: a path that got past the check would fail differently.
        let fs = Firestore::with_base(http, "http://127.0.0.1:9", "wad-spaces").unwrap();
        for p in ["users", "users/u1/secrets", "users/../x/y", "users//secrets/x", "users/u1/secrets/a?b=c"] {
            assert!(matches!(fs.set_strings("t", p, &[]).await, Err(Error::BadPath(_))), "{p}");
        }
    }
}
