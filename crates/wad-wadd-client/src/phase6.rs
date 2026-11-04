//! Phase 6's routes on wadd's socket, typed: the tailnet, peers and GitHub
//! (wad-proto's `tailnet`, `peers` and `github`). A call that starts a job
//! answers with the job ([`JobInfo`]: follow it by its id, or its `job`
//! events); a call that changes the tailnet answers with it as it is after.
//! Their changes come as events too (`tailnet`, `peers`, `github`, `signIn`,
//! `signInDone`).

use serde_json::json;
use wad_proto::wad_model_doc::Wadspace;
use wad_proto::{GithubAccount, JobInfo, PeerInfo, Repo, SignInRequest, Tailnet};

use crate::{Failure, Wadd};

/// The tailnet.
impl Wadd {
    /// This machine on the tailnet, its devices and the ones asking (`GET /v1/tailnet`).
    pub async fn tailnet(&self) -> Result<Tailnet, Failure> {
        self.get("/v1/tailnet").await
    }

    /// Logs in (`POST /v1/tailnet/login`): a Tailnet job, which signs in in
    /// the browser (`signIn`) and ends when the tailnet is running.
    pub async fn tailnet_login(&self) -> Result<JobInfo, Failure> {
        self.post("/v1/tailnet/login", &json!({})).await
    }

    /// Logs out (`POST /v1/tailnet/logout`).
    pub async fn tailnet_logout(&self) -> Result<Tailnet, Failure> {
        self.post("/v1/tailnet/logout", &json!({})).await
    }

    /// Trusts a device (`PUT /v1/tailnet/devices/{id}`, its node id): one on
    /// the tailnet or one asking. Every machine of mine follows.
    pub async fn trust_device(&self, id: &str) -> Result<Tailnet, Failure> {
        self.put(&format!("/v1/tailnet/devices/{id}"), &json!({})).await
    }

    /// Stops trusting a device (`DELETE /v1/tailnet/devices/{id}`).
    pub async fn untrust_device(&self, id: &str) -> Result<Tailnet, Failure> {
        self.delete(&format!("/v1/tailnet/devices/{id}")).await
    }

    /// Denies an ask (`POST /v1/tailnet/asks/{id}/deny`): the device stays quiet.
    pub async fn deny_ask(&self, id: &str) -> Result<Tailnet, Failure> {
        self.post(&format!("/v1/tailnet/asks/{id}/deny"), &json!({})).await
    }

    /// Puts an ask off for this session (`POST /v1/tailnet/asks/{id}/later`).
    pub async fn later_ask(&self, id: &str) -> Result<Tailnet, Failure> {
        self.post(&format!("/v1/tailnet/asks/{id}/later"), &json!({})).await
    }
}

/// Peers: my other machines.
impl Wadd {
    /// My machines on the tailnet (`GET /v1/peers`).
    pub async fn peers(&self) -> Result<Vec<PeerInfo>, Failure> {
        self.get("/v1/peers").await
    }

    /// A peer's live wadspaces, from it (`GET /v1/peers/{id}/wadspaces`);
    /// it has to be online (else `offline`, 503).
    pub async fn peer_wadspaces(&self, peer: &str) -> Result<Vec<Wadspace>, Failure> {
        self.get(&format!("/v1/peers/{peer}/wadspaces")).await
    }

    /// Copies one of a peer's wadspaces here (`POST
    /// /v1/peers/{id}/wadspaces/{ws}/copy`): a CopyWadspace job; the copy's
    /// id is in the `wadspace` event and the toast it makes.
    pub async fn copy_from_peer(&self, peer: &str, ws: &str) -> Result<JobInfo, Failure> {
        self.post(&format!("/v1/peers/{peer}/wadspaces/{ws}/copy"), &json!({})).await
    }
}

/// GitHub.
impl Wadd {
    /// The account wadd has a token for; none: signed out (`GET /v1/github`).
    pub async fn github(&self) -> Result<Option<GithubAccount>, Failure> {
        self.get("/v1/github").await
    }

    /// Signs in (`POST /v1/github/signin`): a Github job, the device flow in
    /// the browser (`signIn` with its code). No scopes: the first sign-in's.
    pub async fn github_signin(&self, req: &SignInRequest) -> Result<JobInfo, Failure> {
        self.post("/v1/github/signin", req).await
    }

    /// Signs out (`DELETE /v1/github`): the token goes.
    pub async fn github_signout(&self) -> Result<(), Failure> {
        self.act(reqwest::Method::DELETE, "/v1/github", None::<&()>).await
    }

    /// The repositories the account can use (`GET /v1/github/repos`).
    pub async fn github_repos(&self) -> Result<Vec<Repo>, Failure> {
        self.get("/v1/github/repos").await
    }
}
