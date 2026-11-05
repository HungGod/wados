//! Phase 7's routes on wadd's socket, typed: streams (wad-proto's
//! `streams`). Starting one answers with its Stream job ([`JobInfo`]:
//! follow it by its id, or its `job` events); the streams' changes come as
//! the `streams` event, the same fields as [`Wadd::streams`]. A stream is
//! known by its wadspace's id; another machine's here ("Stream from
//! {machine}", `StreamInfo::peer` set) by the machine's id and its wadspace's.

use wad_proto::{JobInfo, StreamInfo, StreamList, StreamRequest, StreamShare};

use crate::{Failure, Wadd};

/// How long another machine may take to start a stream (its wadspace, its sidecar, the tailnet).
const PEER_STREAM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(630);

/// Streams.
impl Wadd {
    /// The streams here, and which modes can start, each with why not (`GET /v1/streams`).
    pub async fn streams(&self) -> Result<StreamList, Failure> {
        self.get("/v1/streams").await
    }

    /// Streams a wadspace (`POST /v1/wadspaces/{id}/stream`): a Stream job.
    /// It's on the screen: 409, unless `takeover` (it comes off the screen).
    pub async fn start_stream(&self, ws: &str, req: &StreamRequest) -> Result<JobInfo, Failure> {
        self.post(&format!("/v1/wadspaces/{ws}/stream"), req).await
    }

    /// Stops a wadspace's stream (`DELETE /v1/wadspaces/{id}/stream`): it as it
    /// ended. It leaves the list shortly after (the `streams` event).
    pub async fn stop_stream(&self, ws: &str) -> Result<StreamInfo, Failure> {
        self.delete(&format!("/v1/wadspaces/{ws}/stream")).await
    }

    /// What to hand on with a stream (`GET /v1/streams/{id}/share`): its
    /// one-tap link, its link, and a public one's user and password. Its
    /// secrets: keep it out of logs (its `Debug` does).
    pub async fn stream_share(&self, ws: &str) -> Result<StreamShare, Failure> {
        self.get(&format!("/v1/streams/{ws}/share")).await
    }

    /// Streams a wadspace of another machine of mine to here (`POST
    /// /v1/peers/{id}/wadspaces/{ws}/stream`, "Stream from {machine}"): its
    /// share once it's live there, to open in a WadBrowser `--app` window.
    /// Streaming it already in that mode: its share again. The machine
    /// offline: 503.
    pub async fn start_peer_stream(&self, peer: &str, ws: &str, req: &StreamRequest) -> Result<StreamShare, Failure> {
        let path = format!("/v1/peers/{peer}/wadspaces/{ws}/stream");
        let body = serde_json::to_vec(req).expect("a body is JSON");
        // It answers once the stream's live there: its wadspace started first.
        Self::json(self.call(reqwest::Method::POST, &path, Some(body), Some(PEER_STREAM_TIMEOUT)).await?).await
    }

    /// Stops another machine's stream of its wadspace (`DELETE
    /// /v1/peers/{id}/wadspaces/{ws}/stream`): it, ended.
    pub async fn stop_peer_stream(&self, peer: &str, ws: &str) -> Result<StreamInfo, Failure> {
        self.delete(&format!("/v1/peers/{peer}/wadspaces/{ws}/stream")).await
    }
}
