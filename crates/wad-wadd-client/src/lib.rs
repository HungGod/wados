//! wadd, from a program on the machine (the shell, Settings, Files): its
//! /v1 over the Unix socket, typed with wad-proto, and its event stream
//! (server-sent events) as `wad_proto::Event`s, followed for as long as
//! the program runs, reconnecting by itself.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use wad_proto::Event;

mod phase6;
mod phase7;
mod phase8;
mod sse;
pub use reqwest::Method;
pub use sse::SseParser;

/// How long to wait before following the stream again after it drops.
const RECONNECT: Duration = Duration::from_secs(2);

/// Where wadd listens: WADD_SOCKET, else the machine's, else a laptop's
/// own (`wadd serve --user`).
pub fn socket() -> PathBuf {
    if let Some(s) = std::env::var_os("WADD_SOCKET") {
        return s.into();
    }
    let system = PathBuf::from("/run/wadd/wadd.sock");
    if system.exists() {
        return system;
    }
    std::env::var_os("XDG_RUNTIME_DIR").map(|r| PathBuf::from(r).join("wadd/wadd.sock")).unwrap_or(system)
}

/// A call that didn't work: wadd's status (0 when it couldn't be reached),
/// its error code, and its message, in words.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct Failure {
    pub status: u16,
    pub code: String,
    pub message: String,
    /// A move between the screen and a stream called off: the windows
    /// that didn't close (ApiError's stillOpen).
    pub still_open: Vec<String>,
}

impl Failure {
    fn unreachable(e: impl std::fmt::Display) -> Self {
        tracing::debug!("wadd: {e}");
        Self { status: 0, code: "unreachable".into(), message: "wadd isn't answering.".into(), still_open: vec![] }
    }

    /// The request reached wadd but no answer came back: it hung up (it
    /// hit a bug: its log says), or took too long.
    fn dropped(e: reqwest::Error) -> Self {
        tracing::debug!("wadd: {e}");
        let message = if e.is_timeout() {
            "wadd took too long to answer."
        } else {
            "wadd stopped without answering (its log on this machine says why)."
        };
        Self { status: 0, code: "dropped".into(), message: message.into(), still_open: vec![] }
    }

    /// Not answering at all, or not that; one that hung up is `dropped`.
    fn lost(e: reqwest::Error) -> Self {
        if e.is_connect() { Self::unreachable(e) } else { Self::dropped(e) }
    }

    fn from_body(status: u16, body: &[u8]) -> Self {
        let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
        Self {
            status,
            code: text("code").unwrap_or_default(),
            message: text("message").or_else(|| text("detail")).unwrap_or_else(|| format!("wadd answered {status}")),
            still_open: v
                .get("stillOpen")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|w| w.as_str().map(String::from)).collect())
                .unwrap_or_default(),
        }
    }

    /// wadd couldn't be reached at all.
    pub fn is_unreachable(&self) -> bool {
        self.code == "unreachable"
    }
}

/// What the event stream delivers.
#[derive(Debug, Clone)]
pub enum Incoming {
    Event(Event),
    /// An event this program's wad-proto doesn't know (a newer wadd).
    Other(String, Value),
    /// The stream dropped: it's followed again shortly.
    Disconnected,
}

#[derive(Clone)]
pub struct Wadd {
    http: reqwest::Client,
    sock: PathBuf,
}

impl Wadd {
    pub fn new(sock: impl Into<PathBuf>) -> Self {
        let sock = sock.into();
        let http = reqwest::Client::builder().unix_socket(sock.clone()).build().expect("a client");
        Self { http, sock }
    }

    /// wadd where it is ([`socket`]).
    pub fn here() -> Self {
        Self::new(socket())
    }

    pub fn socket(&self) -> &Path {
        &self.sock
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Vec<u8>>,
        timeout: Option<Duration>,
    ) -> Result<reqwest::Response, Failure> {
        let mut req = self.http.request(method, format!("http://wadd{path}"));
        if let Some(t) = timeout {
            req = req.timeout(t);
        }
        if let Some(b) = body {
            req = req.header("content-type", "application/json").body(b);
        }
        let res = req.send().await.map_err(Failure::lost)?;
        if res.status().is_success() {
            return Ok(res);
        }
        let status = res.status().as_u16();
        let body = res.bytes().await.unwrap_or_default();
        Err(Failure::from_body(status, &body))
    }

    async fn json<T: DeserializeOwned>(res: reqwest::Response) -> Result<T, Failure> {
        let status = res.status().as_u16();
        let bytes = res.bytes().await.map_err(Failure::dropped)?;
        // An empty answer (204) reads as null.
        let bytes: &[u8] = if bytes.is_empty() { b"null" } else { &bytes };
        serde_json::from_slice(bytes).map_err(|e| Failure {
            status,
            code: "bad_answer".into(),
            message: format!("wadd's answer didn't read: {e}"),
            still_open: vec![],
        })
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, Failure> {
        Self::json(self.call(reqwest::Method::GET, path, None, Some(Duration::from_secs(60))).await?).await
    }

    /// POST, PUT, PATCH or DELETE with a JSON body (or none), its answer as `T`.
    pub async fn send<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> Result<T, Failure> {
        let body = body.map(|b| serde_json::to_vec(b).expect("a body is JSON"));
        Self::json(self.call(method, path, body, Some(Duration::from_secs(120))).await?).await
    }

    /// A call whose answer isn't needed, only that it worked.
    pub async fn act(&self, method: reqwest::Method, path: &str, body: Option<&impl Serialize>) -> Result<(), Failure> {
        let body = body.map(|b| serde_json::to_vec(b).expect("a body is JSON"));
        self.call(method, path, body, Some(Duration::from_secs(120))).await.map(drop)
    }

    pub async fn post<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T, Failure> {
        self.send(reqwest::Method::POST, path, Some(body)).await
    }

    pub async fn put<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T, Failure> {
        self.send(reqwest::Method::PUT, path, Some(body)).await
    }

    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T, Failure> {
        self.send(reqwest::Method::DELETE, path, None::<&()>).await
    }

    /// A picture or other file wadd serves, as it is.
    pub async fn bytes(&self, path: &str) -> Result<Vec<u8>, Failure> {
        let res = self.call(reqwest::Method::GET, path, None, Some(Duration::from_secs(30))).await?;
        Ok(res.bytes().await.map_err(Failure::dropped)?.to_vec())
    }

    /// Follows the event stream until it drops (each event to `on`).
    pub async fn stream_once(&self, on: &mut impl FnMut(Incoming)) -> Result<(), Failure> {
        let mut res = self.call(reqwest::Method::GET, "/v1/events", None, None).await?;
        let mut parser = SseParser::default();
        while let Some(chunk) = res.chunk().await.map_err(Failure::dropped)? {
            for (name, data) in parser.push(&chunk) {
                on(decode(name, &data));
            }
        }
        Ok(())
    }

    /// Follows the event stream for as long as `on` takes events: it starts
    /// with the current state of each kind, and after a drop (an
    /// `Incoming::Disconnected`) begins again. On the current tokio runtime.
    pub fn follow(&self, mut on: impl FnMut(Incoming) -> bool + Send + 'static) -> tokio::task::JoinHandle<()> {
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                let mut open = true;
                let res = me
                    .stream_once(&mut |i| {
                        if open {
                            open = on(i);
                        }
                    })
                    .await;
                if !open {
                    return;
                }
                if let Err(e) = res {
                    tracing::debug!("wadd events: {e}");
                }
                if !on(Incoming::Disconnected) {
                    return;
                }
                tokio::time::sleep(RECONNECT).await;
            }
        })
    }
}

/// An SSE event as wad-proto's `Event` (`{type, data}`), else as it came.
pub fn decode(name: String, data: &str) -> Incoming {
    let data: Value = serde_json::from_str(data).unwrap_or(Value::Null);
    match serde_json::from_value::<Event>(data.clone()) {
        Ok(e) => Incoming::Event(e),
        Err(_) => {
            let kind = data.get("kind").and_then(Value::as_str).map(String::from).unwrap_or(name);
            Incoming::Other(kind, data)
        }
    }
}

impl Wadd {
    /// One edit (`POST /v1/ops`).
    pub async fn op(&self, op: wad_proto::wad_model_op::Op) -> Result<wad_proto::OpResult, Failure> {
        self.post("/v1/ops", &wad_proto::OpRequest { op }).await
    }

    /// An edit's Undo (`POST /v1/undo/{id}`).
    pub async fn undo(&self, id: &str) -> Result<wad_proto::OpResult, Failure> {
        self.post(&format!("/v1/undo/{id}"), &serde_json::json!({})).await
    }
}

impl Wadd {
    /// A wadspace's image built here (`POST /v1/wadspaces/{id}/build`): the
    /// build started, or none when it's built already.
    pub async fn build(&self, id: &str) -> Result<wad_proto::BuildAnswer, Failure> {
        self.post(&format!("/v1/wadspaces/{id}/build"), &serde_json::json!({})).await
    }

    /// Opens a wadspace, shown as `display` (`POST /v1/wadspaces/{id}/open`):
    /// it comes up in the background; its run state says how it's going.
    pub async fn open(
        &self,
        id: &str,
        display: wad_proto::wad_model_run::SessionDisplay,
    ) -> Result<wad_proto::WadspaceRun, Failure> {
        self.post(&format!("/v1/wadspaces/{id}/open"), &wad_proto::OpenRequest { display }).await
    }

    /// Stops a wadspace (`POST /v1/wadspaces/{id}/stop`).
    pub async fn stop(&self, id: &str) -> Result<wad_proto::WadspaceRun, Failure> {
        self.post(&format!("/v1/wadspaces/{id}/stop"), &serde_json::json!({})).await
    }

    /// Each wadspace's run here (`GET /v1/runs`).
    pub async fn runs(&self) -> Result<Vec<wad_proto::WadspaceRun>, Failure> {
        self.get("/v1/runs").await
    }

    /// Each wadspace's image here (`GET /v1/builds`).
    pub async fn builds(&self) -> Result<Vec<wad_proto::WadspaceBuild>, Failure> {
        self.get("/v1/builds").await
    }

    /// The jobs, newest first (`GET /v1/jobs`).
    pub async fn jobs(&self) -> Result<Vec<wad_proto::JobInfo>, Failure> {
        self.get("/v1/jobs").await
    }

    /// A job, with its log from line `since` (`GET /v1/jobs/{id}?since=`).
    pub async fn job(&self, id: &str, since: u64) -> Result<wad_proto::JobLog, Failure> {
        self.get(&format!("/v1/jobs/{id}?since={since}")).await
    }

    /// Stops a job (`POST /v1/jobs/{id}/cancel`).
    pub async fn cancel_job(&self, id: &str) -> Result<wad_proto::JobInfo, Failure> {
        self.post(&format!("/v1/jobs/{id}/cancel"), &serde_json::json!({})).await
    }

    /// The shell's compositor, as it starts (`PUT /v1/screen/session`).
    pub async fn screen_session(&self, wayland_socket: &str, uid: u32) -> Result<wad_proto::ScreenInfo, Failure> {
        self.put("/v1/screen/session", &wad_proto::ScreenSession { wayland_socket: wayland_socket.into(), uid }).await
    }
}
