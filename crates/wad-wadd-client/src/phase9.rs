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
    pub async fn setup_design(&self, starter: bool, nvidia: bool) -> Result<Setup, Failure> {
        self.post("/v1/setup/design", &SetupDesign { starter, nvidia }).await
    }

    /// Starts a focus (`POST /v1/focus`).
    pub async fn focus_start(&self, ws: &str, minutes: u32) -> Result<wad_model::Focus, Failure> {
        self.post("/v1/focus", &wad_proto::FocusRequest { ws: ws.into(), minutes }).await
    }

    /// Keeps a disc on this machine (`POST /v1/discs`): a job.
    pub async fn disc_add(&self, path: &str, push: bool) -> Result<JobInfo, Failure> {
        self.post("/v1/discs", &wad_proto::DiscAdd { path: path.into(), push }).await
    }

    /// The Steam accounts signed in here (`GET /v1/steam`).
    pub async fn steam(&self) -> Result<Vec<wad_proto::SteamAccount>, Failure> {
        self.get("/v1/steam").await
    }

    /// Signs in to a Steam account (`POST /v1/steam/signin`): a job.
    pub async fn steam_signin(&self, name: &str) -> Result<JobInfo, Failure> {
        self.post("/v1/steam/signin", &wad_proto::SteamSignIn { name: name.into() }).await
    }

    /// Pushes games to the account's packages (`POST /v1/steam/push`): a job.
    pub async fn steam_push(&self, account: &str, games: Vec<String>) -> Result<JobInfo, Failure> {
        self.post("/v1/steam/push", &wad_proto::SteamPush { account: account.into(), games }).await
    }

    /// Every agent and sandbox (`GET /v1/agents`).
    pub async fn agents(&self) -> Result<wad_proto::Agents, Failure> {
        self.get("/v1/agents").await
    }

    /// A job for a wadspace's Claude Code (`POST /v1/agents`).
    pub async fn agent_send(&self, ws: &str, text: &str) -> Result<wad_model::agents::Agent, Failure> {
        self.post("/v1/agents", &wad_proto::AgentSend { ws: ws.into(), text: text.into() }).await
    }

    /// `stop`, `allow`, `deny` or `ship`.
    pub async fn agent_act(&self, id: &str, what: &str) -> Result<(), Failure> {
        self.post(&format!("/v1/agents/{id}/{what}"), &json!({})).await
    }

    /// A word to a job, between its turns.
    pub async fn agent_say(&self, id: &str, text: &str) -> Result<(), Failure> {
        let body = wad_proto::AgentSend { ws: String::new(), text: text.into() };
        self.post(&format!("/v1/agents/{id}/say"), &body).await
    }

    /// A verdict on a checklist's item.
    pub async fn sandbox_mark(
        &self,
        id: &str,
        item: &str,
        verdict: Option<wad_model::agents::Verdict>,
        note: &str,
    ) -> Result<(), Failure> {
        let body = wad_proto::ItemMark { item: item.into(), verdict, note: note.into() };
        self.post(&format!("/v1/sandboxes/{id}/mark"), &body).await
    }

    /// `send` or `close`.
    pub async fn sandbox_act(&self, id: &str, what: &str) -> Result<(), Failure> {
        self.post(&format!("/v1/sandboxes/{id}/{what}"), &json!({})).await
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
