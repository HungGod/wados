//! wadd, from the browser on the machine: the library's keys and web apps,
//! and the edits the browser makes to them (a site's saved sign-in, "Create
//! app from this page"). It's wad-wadd-client's (the socket, its
//! `WADD_SOCKET` override, wad-proto's types), made blocking: every call
//! waits for wadd's answer, so call it off the main thread
//! (wadbrowser-webkit's `wadd::call` does, and brings the answer back).
//!
//! Inside a wadspace there's no wadd: [`Wadd::here`] is None, and a call
//! that can't reach it is an `Err` whose `is_unreachable()` says so; the
//! browser carries on without it.

use wad_model::{BrowserApp, Library, Op, Secret, SecretKind};
pub use wad_wadd_client::Failure;

/// wadd's API, blocking.
#[derive(Clone)]
pub struct Wadd {
    client: wad_wadd_client::Wadd,
}

impl Wadd {
    /// wadd at its usual socket (`WADD_SOCKET`, the machine's, a laptop's
    /// own), if there's one there.
    pub fn here() -> Option<Self> {
        let sock = wad_wadd_client::socket();
        sock.exists().then(|| Self::at(sock))
    }

    pub fn at(sock: impl Into<std::path::PathBuf>) -> Self {
        Self { client: wad_wadd_client::Wadd::new(sock) }
    }

    fn block<T>(&self, f: impl std::future::Future<Output = Result<T, Failure>>) -> Result<T, Failure> {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| Failure {
            status: 0,
            code: "unreachable".into(),
            message: format!("no runtime for wadd: {e}"),
            still_open: vec![],
        })?;
        rt.block_on(f)
    }

    /// The machine's library (`GET /v1/library`).
    pub fn library(&self) -> Result<Library, Failure> {
        self.block(self.client.get("/v1/library"))
    }

    /// Its web keys (saved sign-ins for sites), with their sites.
    pub fn web_keys(&self) -> Result<Vec<Secret>, Failure> {
        Ok(self.library()?.keys.into_iter().filter(|k| k.kind == SecretKind::Web).collect())
    }

    /// The web apps made from pages.
    pub fn web_apps(&self) -> Result<Vec<BrowserApp>, Failure> {
        Ok(self.library()?.web_apps)
    }

    /// One edit (`POST /v1/ops`).
    pub fn op(&self, op: Op) -> Result<wad_proto::OpResult, Failure> {
        self.block(self.client.op(op))
    }

    /// `site` signs in with key `existing`, else with a new web key named
    /// `name` (`secrets.saveWeb`): the key's id.
    pub fn save_web_key(&self, name: &str, site: &str, existing: Option<&str>) -> Result<String, Failure> {
        let op = Op::SaveWebKey { name: name.into(), site: site.into(), existing: existing.map(String::from) };
        Ok(self.op(op)?.created.unwrap_or_default())
    }

    /// Key `key` stops signing in to `site` (`secrets.removeSite`).
    pub fn remove_site(&self, key: &str, site: &str) -> Result<(), Failure> {
        self.op(Op::RemoveSite { key: key.into(), site: site.into() }).map(drop)
    }

    /// "Create app from this page" (`assets.webApp`): the web app's id.
    pub fn create_web_app(&self, name: &str, url: &str) -> Result<String, Failure> {
        Ok(self.op(Op::NewWebApp { name: name.into(), url: url.into() })?.created.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::UnixListener;

    /// A wadd that answers each request with `answers`' next, and says what it was asked.
    fn fake(answers: Vec<&'static str>) -> (tempfile::TempDir, std::thread::JoinHandle<Vec<String>>) {
        let dir = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(dir.path().join("wadd.sock")).unwrap();
        let t = std::thread::spawn(move || {
            let mut asked = vec![];
            for body in answers {
                let (stream, _) = listener.accept().unwrap();
                let mut r = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let mut len = 0;
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                }
                let mut req = vec![0; len];
                r.read_exact(&mut req).unwrap();
                asked.push(format!("{} {}", line.trim(), String::from_utf8(req).unwrap()));
                let mut w = stream;
                write!(
                    w,
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            asked
        });
        (dir, t)
    }

    #[test]
    fn reads_the_library_and_posts_ops_as_wad_proto_has_them() {
        let lib = r#"{"keys": [
            {"id": "hunggod", "name": "hunggod", "kind": "web", "createdAt": 1, "sites": ["github.com"]},
            {"id": "vs", "name": "VS Code", "kind": "vscode", "createdAt": 1}
        ], "webApps": [{"id": "calendar", "name": "Calendar", "url": "https://calendar.google.com", "profile": "default"}]}"#;
        let (dir, t) = fake(vec![lib, r#"{"changed": [], "created": "work", "needsBuild": false}"#]);
        let w = Wadd::at(dir.path().join("wadd.sock"));
        let keys = w.web_keys().expect("keys");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].sites, ["github.com"]);
        assert_eq!(w.save_web_key("Work", "gitlab.com", None).expect("saved"), "work");
        let asked = t.join().unwrap();
        assert!(asked[0].starts_with("GET /v1/library HTTP/1.1"), "{}", asked[0]);
        assert!(asked[1].starts_with("POST /v1/ops HTTP/1.1"), "{}", asked[1]);
        let body: serde_json::Value =
            serde_json::from_str(asked[1].split_once(' ').unwrap().1.rsplit_once("HTTP/1.1 ").unwrap().1).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"op": {"op": "secrets.saveWeb", "name": "Work", "site": "gitlab.com", "existing": null}})
        );
    }

    #[test]
    fn no_wadd_is_an_error_that_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let w = Wadd::at(dir.path().join("none.sock"));
        let e = w.library().expect_err("nothing there");
        assert!(e.is_unreachable(), "{e:?}");
    }
}
