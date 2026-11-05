//! Phase 8's routes on wadd's socket, typed: the system apps' (wad-proto's
//! `network`, `bluetooth`, `updates`, `storage`), and deleting a wadspace
//! out of the Trash for good. A call that changes something answers with it
//! as it is after; one that starts a job answers with the job ([`JobInfo`]:
//! follow it by its id, or its `job` events). Their changes come as events
//! too (`network`, `bluetooth`, `btPrompt`, `updates`, `storage`; a notice
//! for what a job did).

use serde_json::json;
use wad_proto::{
    Bases, Bluetooth, BtAnswer, ExitNode, JobInfo, Network, OpResult, Storage, Switch, Updates, WifiConnect, WifiForget,
};

use crate::{Failure, Wadd};

/// The network.
impl Wadd {
    /// Wi-Fi, ethernet, airplane mode and the exit node (`GET /v1/network`).
    pub async fn network(&self) -> Result<Network, Failure> {
        self.get("/v1/network").await
    }

    /// Wi-Fi on or off (`POST /v1/network/wifi`).
    pub async fn set_wifi(&self, on: bool) -> Result<Network, Failure> {
        self.post("/v1/network/wifi", &Switch { on }).await
    }

    /// Airplane mode (`POST /v1/network/airplane`): every radio off, Bluetooth too.
    pub async fn set_airplane(&self, on: bool) -> Result<Network, Failure> {
        self.post("/v1/network/airplane", &Switch { on }).await
    }

    /// Joins a network (`POST /v1/network/wifi/connect`), once it's joined.
    /// A secure one not saved needs its password (wrong: 401).
    pub async fn wifi_connect(&self, ssid: &str, password: Option<&str>) -> Result<Network, Failure> {
        // NetworkManager gives up after 90 s; a call may take 120.
        let req = WifiConnect { ssid: ssid.into(), password: password.map(String::from) };
        self.post("/v1/network/wifi/connect", &req).await
    }

    /// Leaves the network it's on (`POST /v1/network/wifi/disconnect`).
    pub async fn wifi_disconnect(&self) -> Result<Network, Failure> {
        self.post("/v1/network/wifi/disconnect", &json!({})).await
    }

    /// Forgets a saved network (`POST /v1/network/wifi/forget`).
    pub async fn wifi_forget(&self, ssid: &str) -> Result<Network, Failure> {
        self.post("/v1/network/wifi/forget", &WifiForget { ssid: ssid.into() }).await
    }

    /// Out through a machine on the tailnet (one of `exitNodes`, by its id),
    /// or (none) straight out (`POST /v1/network/exit-node`).
    pub async fn set_exit_node(&self, node: Option<&str>) -> Result<Network, Failure> {
        self.post("/v1/network/exit-node", &ExitNode { node: node.map(String::from) }).await
    }
}

/// Bluetooth. Devices are known by their address.
impl Wadd {
    /// The adapter and its devices (`GET /v1/bluetooth`).
    pub async fn bluetooth(&self) -> Result<Bluetooth, Failure> {
        self.get("/v1/bluetooth").await
    }

    /// On or off (`POST /v1/bluetooth/power`).
    pub async fn set_bluetooth(&self, on: bool) -> Result<Bluetooth, Failure> {
        self.post("/v1/bluetooth/power", &Switch { on }).await
    }

    /// Looking for devices, or not (`POST /v1/bluetooth/scan`).
    pub async fn bt_scan(&self, on: bool) -> Result<Bluetooth, Failure> {
        self.post("/v1/bluetooth/scan", &Switch { on }).await
    }

    /// Starts pairing (`POST /v1/bluetooth/devices/{address}/pair`), at once:
    /// what it asks is the `btPrompt` event, to answer with [`Wadd::bt_answer`].
    pub async fn bt_pair(&self, address: &str) -> Result<Bluetooth, Failure> {
        self.bt_device(address, "pair").await
    }

    /// Connects a paired device (`POST …/connect`).
    pub async fn bt_connect(&self, address: &str) -> Result<Bluetooth, Failure> {
        self.bt_device(address, "connect").await
    }

    /// `POST …/disconnect`.
    pub async fn bt_disconnect(&self, address: &str) -> Result<Bluetooth, Failure> {
        self.bt_device(address, "disconnect").await
    }

    /// Unpairs a device (`POST …/forget`).
    pub async fn bt_forget(&self, address: &str) -> Result<Bluetooth, Failure> {
        self.bt_device(address, "forget").await
    }

    async fn bt_device(&self, address: &str, action: &str) -> Result<Bluetooth, Failure> {
        self.post(&format!("/v1/bluetooth/devices/{address}/{action}"), &json!({})).await
    }

    /// Answers what pairing asked (`POST /v1/bluetooth/prompt/{id}`).
    pub async fn bt_answer(&self, id: &str, answer: &BtAnswer) -> Result<Bluetooth, Failure> {
        self.post(&format!("/v1/bluetooth/prompt/{id}"), answer).await
    }
}

/// Updates and the base images.
impl Wadd {
    /// WadOS's own image: booted, available, staged (`GET /v1/update`).
    pub async fn update_status(&self) -> Result<Updates, Failure> {
        self.get("/v1/update").await
    }

    /// Looks for a newer one (`POST /v1/update/check`): an Update job.
    pub async fn update_check(&self) -> Result<JobInfo, Failure> {
        self.post("/v1/update/check", &json!({})).await
    }

    /// Downloads and stages what's available (`POST /v1/update/apply`): an Update job.
    pub async fn update_apply(&self) -> Result<JobInfo, Failure> {
        self.post("/v1/update/apply", &json!({})).await
    }

    /// Drops what's staged (`POST /v1/update/cancel`).
    pub async fn update_cancel(&self) -> Result<Updates, Failure> {
        self.post("/v1/update/cancel", &json!({})).await
    }

    /// Stages the one before (`POST /v1/update/rollback`).
    pub async fn update_rollback(&self) -> Result<Updates, Failure> {
        self.post("/v1/update/rollback", &json!({})).await
    }

    /// The base images here (`GET /v1/bases`).
    pub async fn bases(&self) -> Result<Bases, Failure> {
        self.get("/v1/bases").await
    }

    /// Pulls them again now (`POST /v1/bases/refresh`): a Pull job.
    pub async fn refresh_bases(&self) -> Result<JobInfo, Failure> {
        self.post("/v1/bases/refresh", &json!({})).await
    }
}

/// Storage, and the Trash.
impl Wadd {
    /// The disk (`GET /v1/storage`).
    pub async fn storage(&self) -> Result<Storage, Failure> {
        self.get("/v1/storage").await
    }

    /// Removes what nothing uses (`POST /v1/storage/cleanup`): a Cleanup job.
    pub async fn storage_cleanup(&self) -> Result<JobInfo, Failure> {
        self.post("/v1/storage/cleanup", &json!({})).await
    }

    /// Deletes a wadspace in the Trash for good (`DELETE /v1/wadspaces/{id}`):
    /// its document, image, units and state. Not in the Trash: 409.
    pub async fn purge_wadspace(&self, id: &str) -> Result<OpResult, Failure> {
        self.delete(&format!("/v1/wadspaces/{id}")).await
    }
}
