//! Phase 7's routes on wadd's socket, typed: streams (wad-proto's
//! `streams`). Starting one answers with its Stream job ([`JobInfo`]:
//! follow it by its id, or its `job` events); the streams' changes come as
//! the `streams` event, the same fields as [`Wadd::streams`]. A stream is
//! known by its wadspace's id.

use wad_proto::{JobInfo, StreamInfo, StreamList, StreamRequest, StreamShare};

use crate::{Failure, Wadd};

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
}
