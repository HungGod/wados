//! The installer stick and first boot (`/v1/install`, `/v1/setup`).

use serde_json::json;
use wad_proto::{Install, InstallRequest, JobInfo, Setup, SetupDesign, SetupStep};

use crate::{Failure, Wadd};

impl Wadd {
    /// The disks and the install under way (`GET /v1/install`); 404 off a stick.
    pub async fn install(&self) -> Result<Install, Failure> {
        self.get("/v1/install").await
    }

    /// Erases the disk and installs (`POST /v1/install`): an Install job.
    pub async fn install_start(&self, req: &InstallRequest) -> Result<JobInfo, Failure> {
        self.post("/v1/install", req).await
    }

    /// Where Set up WadSpaces is (`GET /v1/setup`).
    pub async fn setup(&self) -> Result<Setup, Failure> {
        self.get("/v1/setup").await
    }

    /// Set up goes to a step (`PUT /v1/setup`).
    pub async fn setup_step(&self, step: SetupStep) -> Result<Setup, Failure> {
        self.put("/v1/setup", &json!({ "step": step })).await
    }

    /// Set up's last step, Install or Skip (`POST /v1/setup/design`).
    pub async fn setup_design(&self, starter: bool) -> Result<Setup, Failure> {
        self.post("/v1/setup/design", &SetupDesign { starter }).await
    }

    /// Starts a focus (`POST /v1/focus`).
    pub async fn focus_start(&self, ws: &str, minutes: u32) -> Result<wad_model::Focus, Failure> {
        self.post("/v1/focus", &wad_proto::FocusRequest { ws: ws.into(), minutes }).await
    }

    /// Server mode (`GET /v1/server`).
    pub async fn server(&self) -> Result<wad_proto::Server, Failure> {
        self.get("/v1/server").await
    }

    /// Server mode on or off (`POST /v1/server`).
    pub async fn server_set(&self, on: bool) -> Result<wad_proto::Server, Failure> {
        self.post("/v1/server", &wad_proto::Server { on }).await
    }
}
