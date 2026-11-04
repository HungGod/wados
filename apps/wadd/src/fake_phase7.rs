//! Phase 7's sample data, for building the shell against before the real
//! streams are made (`Parts::sample`: wadd's tests, and `wadd serve
//! --sample`). It answers every route of [`crate::api_phase7`] and keeps
//! what it's told for as long as wadd runs:
//!
//! - **The streams:** this machine's first wadspace (the starter, in a new
//!   library) streaming publicly, live, with 2 viewers over WebRTC (TURN
//!   answering); and Desk's Notes (the phase 6 sample's peer) streaming
//!   privately to here, starting.
//! - **What can start:** private and public; secure can't
//!   ([`SECURE_NEEDS_VM`]).
//! - **Start:** a Stream job with the real one's stages
//!   ([`wad_proto::streams::stages`]), the stream following them, then live.
//! - **Stop:** ended, then gone a step later.
//! - **Share:** links and a sign-in that are plainly fake (`.invalid`
//!   hosts, `sample-…-not-a-secret`): never anything real.
//!
//! Each stage of its jobs takes `step`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use wad_model::{JobKind, PartState, Phase, StreamMode, StreamStage, Transport};
use wad_proto::streams::{SECURE_NEEDS_VM, stages};
use wad_proto::{
    ApiError, Availability, JobInfo, StreamInfo, StreamList, StreamRequest, StreamShare, StreamsAvailability,
};

use crate::api_phase7::Streams;
use crate::events::Bus;
use crate::jobs::{Jobs, now_ms};
use crate::library::Docs;
use crate::runs::Runs;

const MINUTE: i64 = 60_000;
/// The phase 6 sample's peer, and the wadspace of its this machine watches.
const PEER: &str = "m-desk";
const PEER_WADSPACE: &str = "notes";
/// Plainly fake: `.invalid` never resolves (RFC 2606), and these say what they are.
const PUBLIC_HOST: &str = "sample-host.invalid";
const TAILNET: &str = "sample-tailnet.invalid";
const TOKEN: &str = "sample-token-not-a-secret";
const USER: &str = "sample-viewer";
const PASSWORD: &str = "sample-password-not-a-secret";

/// Cheap to clone: one sample, shared.
#[derive(Clone)]
pub struct Sample(Arc<Inner>);

struct Inner {
    streams: Mutex<Vec<StreamInfo>>,
    available: StreamsAvailability,
    hostname: String,
    jobs: Arc<Jobs>,
    bus: Bus,
    docs: Arc<Docs>,
    runs: Arc<Runs>,
    step: Duration,
}

/// A stream's sample link (without its key): public at the fake host, private on the fake tailnet.
fn link(id: &str, mode: StreamMode, machine: &str) -> String {
    match mode.is_public() {
        true => format!("https://{PUBLIC_HOST}/ws-sample-{id}/"),
        false => format!("https://{machine}.{TAILNET}:48300/ws-sample-{id}/"),
    }
}

fn not_streaming(ws: &str) -> ApiError {
    ApiError::not_found(format!("{ws:?} isn't streaming"))
}

impl Sample {
    /// The sample, its present told to the watchers.
    pub fn new(hostname: &str, jobs: Arc<Jobs>, bus: Bus, docs: Arc<Docs>, runs: Arc<Runs>, step: Duration) -> Self {
        let now = now_ms();
        let mut streams = vec![];
        if let Some(w) = docs.live().into_iter().next() {
            streams.push(StreamInfo {
                id: w.id.clone(),
                mode: StreamMode::Public,
                transport: Transport::Webrtc,
                stage: StreamStage::Live,
                live: true,
                viewers: 2,
                started: now - 25 * MINUTE,
                last_active: now - MINUTE,
                link: link(&w.id, StreamMode::Public, hostname),
                turn_down: false,
                warnings: vec![],
                job: None,
                peer: None,
            });
        }
        streams.push(StreamInfo {
            id: PEER_WADSPACE.into(),
            mode: StreamMode::Private,
            transport: Transport::Websocket,
            stage: StreamStage::Sidecar,
            live: false,
            viewers: 0,
            started: now,
            last_active: now,
            link: link(PEER_WADSPACE, StreamMode::Private, "desk"),
            turn_down: false,
            warnings: vec![],
            // Its job is the peer's.
            job: None,
            peer: Some(PEER.into()),
        });
        let available = StreamsAvailability {
            private: Availability::yes(),
            private_secure: Availability::no(SECURE_NEEDS_VM),
            public: Availability::yes(),
            public_secure: Availability::no(SECURE_NEEDS_VM),
        };
        let me =
            Inner { streams: Mutex::new(streams), available, hostname: hostname.into(), jobs, bus, docs, runs, step };
        me.tell();
        Self(Arc::new(me))
    }
}

impl Inner {
    fn list(&self) -> StreamList {
        StreamList { streams: self.streams.lock().unwrap().clone(), available: self.available.clone() }
    }

    fn tell(&self) {
        self.bus.publish(self.list().into());
    }

    /// Changes `ws`'s stream (not one that ended), then tells it.
    fn with(&self, ws: &str, f: impl FnOnce(&mut StreamInfo)) {
        if let Some(s) = self.streams.lock().unwrap().iter_mut().find(|s| s.id == ws && s.stage != StreamStage::Ended) {
            f(s);
        }
        self.tell();
    }

    fn remove(&self, f: impl Fn(&StreamInfo) -> bool) {
        let gone = {
            let mut streams = self.streams.lock().unwrap();
            let before = streams.len();
            streams.retain(|s| !f(s));
            streams.len() != before
        };
        if gone {
            self.tell();
        }
    }
}

/// A stream starting: gone again if its job is dropped (cancelled) before it's live.
struct Starting {
    me: Arc<Inner>,
    ws: String,
    job: String,
    live: bool,
}

impl Drop for Starting {
    fn drop(&mut self) {
        if !self.live {
            let (ws, job) = (self.ws.clone(), Some(self.job.clone()));
            self.me.remove(|s| s.id == ws && s.job == job);
        }
    }
}

#[async_trait]
impl Streams for Sample {
    async fn streams(&self) -> Result<StreamList, ApiError> {
        Ok(self.0.list())
    }

    async fn start(&self, ws: &str, req: StreamRequest) -> Result<JobInfo, ApiError> {
        let me = &self.0;
        let w = me
            .docs
            .live()
            .into_iter()
            .find(|w| w.id == ws)
            .ok_or_else(|| ApiError::not_found(format!("There's no wadspace {ws:?} here")))?;
        if me.streams.lock().unwrap().iter().any(|s| s.id == ws && s.stage != StreamStage::Ended) {
            return Err(ApiError::conflict(format!("{} is streaming already: stop it first", w.name)));
        }
        if let Some(why) = &me.available.get(req.mode).reason {
            return Err(ApiError::conflict(why.clone()));
        }
        let up = matches!(me.runs.state(ws).state.phase, Phase::Starting | Phase::Waiting | Phase::Ready);
        if me.runs.on_screen().iter().any(|(id, _)| id == ws) && !req.takeover {
            return Err(ApiError::conflict(format!(
                "{} is on the screen: streaming it takes it off the screen",
                w.name
            )));
        }
        let plan = stages(req.mode, !up);
        let labelled: Vec<(String, String)> = plan.iter().map(|(s, l)| (s.as_str().into(), l.to_string())).collect();
        let job = me.jobs.create(JobKind::Stream, &format!("Streaming {}", w.name), ws, &labelled);
        let now = now_ms();
        {
            let mut streams = me.streams.lock().unwrap();
            streams.retain(|s| s.id != ws);
            streams.push(StreamInfo {
                id: ws.into(),
                mode: req.mode,
                transport: if req.mode.is_public() { Transport::Webrtc } else { Transport::Websocket },
                stage: plan[0].0,
                live: false,
                viewers: 0,
                started: now,
                last_active: now,
                link: link(ws, req.mode, &me.hostname),
                turn_down: false,
                warnings: vec![],
                job: Some(job.clone()),
                peer: None,
            });
        }
        me.tell();
        let mut starting = Starting { me: me.clone(), ws: ws.into(), job: job.clone(), live: false };
        let (jobs, id) = (me.jobs.clone(), job.clone());
        let task = tokio::spawn(async move {
            jobs.running(&id);
            let me = starting.me.clone();
            for (stage, _) in &plan {
                me.with(&starting.ws, |s| s.stage = *stage);
                jobs.stage(&id, stage.as_str(), PartState::Working, None);
                tokio::time::sleep(me.step).await;
                jobs.stage(&id, stage.as_str(), PartState::Done, None);
            }
            starting.live = true;
            me.with(&starting.ws, |s| {
                s.stage = StreamStage::Live;
                s.live = true;
            });
            jobs.finish(&id, Ok(()));
        });
        me.jobs.attach(&job, task.abort_handle());
        me.jobs.info(&job).ok_or_else(|| ApiError::internal("the job went"))
    }

    async fn stop(&self, ws: &str) -> Result<StreamInfo, ApiError> {
        let me = &self.0;
        let ended = {
            let mut streams = me.streams.lock().unwrap();
            let s = streams.iter_mut().find(|s| s.id == ws && s.stage != StreamStage::Ended);
            let s = s.ok_or_else(|| not_streaming(ws))?;
            s.stage = StreamStage::Ended;
            s.live = false;
            s.viewers = 0;
            s.clone()
        };
        me.tell();
        // One still starting here: its job stops too.
        if let Some(job) = &ended.job
            && me.jobs.info(job).is_some_and(|j| !crate::jobs::finished(j.job.status))
        {
            me.jobs.cancel(job)?;
        }
        let (me, id) = (me.clone(), ws.to_string());
        tokio::spawn(async move {
            tokio::time::sleep(me.step).await;
            me.remove(|s| s.id == id && s.stage == StreamStage::Ended);
        });
        Ok(ended)
    }

    async fn share(&self, ws: &str) -> Result<StreamShare, ApiError> {
        let s = self.0.streams.lock().unwrap().iter().find(|s| s.id == ws && s.stage != StreamStage::Ended).cloned();
        let s = s.ok_or_else(|| not_streaming(ws))?;
        let public = s.mode.is_public();
        Ok(StreamShare {
            mode: s.mode,
            open_link: format!("{}#token={TOKEN}", s.link),
            link: s.link,
            user: public.then(|| USER.into()),
            password: public.then(|| PASSWORD.into()),
        })
    }
}
