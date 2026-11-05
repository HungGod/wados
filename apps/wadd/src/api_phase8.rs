//! Phase 8's routes on wadd's socket: the system apps' (Settings ▸ Wi-Fi,
//! Network, Bluetooth, Storage; the Updates app). They answer through four
//! traits, [`Network`], [`Bluetooth`], [`Updates`] and [`Storage`]: the
//! machine's (501 until they're made: [`NotYet`]), or sample data
//! ([`crate::fake_phase8::Sample`]: tests, and `wadd serve --sample`). The
//! types are wad-proto's `network`, `bluetooth`, `updates` and `storage`,
//! whose docs say what each answers when it can't.
//!
//! | Route | What |
//! |---|---|
//! | `GET /v1/network` | [`proto::Network`] |
//! | `POST /v1/network/wifi` | Wi-Fi on or off ([`Switch`]) → [`proto::Network`] |
//! | `POST /v1/network/airplane` | airplane mode ([`Switch`]) → [`proto::Network`] |
//! | `POST /v1/network/wifi/connect` | joins a network ([`WifiConnect`]) → [`proto::Network`] |
//! | `POST /v1/network/wifi/disconnect` | → [`proto::Network`] |
//! | `POST /v1/network/wifi/forget` | ([`WifiForget`]) → [`proto::Network`] |
//! | `POST /v1/network/exit-node` | ([`ExitNode`]) → [`proto::Network`] |
//! | `GET /v1/bluetooth` | [`proto::Bluetooth`] |
//! | `POST /v1/bluetooth/power` | ([`Switch`]) → [`proto::Bluetooth`] |
//! | `POST /v1/bluetooth/scan` | ([`Switch`]) → [`proto::Bluetooth`] |
//! | `POST /v1/bluetooth/devices/{address}/{pair,connect,disconnect,forget}` | → [`proto::Bluetooth`] |
//! | `POST /v1/bluetooth/prompt/{id}` | answers pairing ([`BtAnswer`]) → [`proto::Bluetooth`] |
//! | `GET /v1/update` | [`proto::Updates`] |
//! | `POST /v1/update/check` | an Update job → [`JobInfo`] |
//! | `POST /v1/update/apply` | an Update job (download, stage) → [`JobInfo`] |
//! | `POST /v1/update/cancel` | drops what's staged → [`proto::Updates`] |
//! | `POST /v1/update/rollback` | stages the one before → [`proto::Updates`] |
//! | `GET /v1/bases` | [`Bases`] |
//! | `POST /v1/bases/refresh` | a Pull job → [`JobInfo`] |
//! | `GET /v1/storage` | [`proto::Storage`] |
//! | `POST /v1/storage/cleanup` | a cleanup job → [`JobInfo`] |
//!
//! Deleting a wadspace out of the Trash for good (`DELETE /v1/wadspaces/{id}`)
//! is real already: it's [`crate::api`]'s.

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use wad_proto as proto;
use wad_proto::{ApiError, Bases, BtAnswer, ExitNode, JobInfo, Switch, WifiConnect, WifiForget};

use crate::api::{AppState, Failure};

/// Wi-Fi, airplane mode, ethernet and the exit node (owner: the network agent).
#[async_trait]
pub trait Network: Send + Sync + 'static {
    async fn network(&self) -> Result<proto::Network, ApiError>;
    async fn wifi(&self, on: bool) -> Result<proto::Network, ApiError>;
    /// Every radio off (Bluetooth too), or back.
    async fn airplane(&self, on: bool) -> Result<proto::Network, ApiError>;
    /// Answers once it's joined, or why not (a wrong password: 401).
    async fn connect(&self, req: WifiConnect) -> Result<proto::Network, ApiError>;
    async fn disconnect(&self) -> Result<proto::Network, ApiError>;
    async fn forget(&self, ssid: &str) -> Result<proto::Network, ApiError>;
    /// Out through `node` (one of `exitNodes`), or straight out.
    async fn exit_node(&self, node: Option<String>) -> Result<proto::Network, ApiError>;
}

/// Bluetooth (owner: the Bluetooth agent). Devices are known by their address.
#[async_trait]
pub trait Bluetooth: Send + Sync + 'static {
    async fn bluetooth(&self) -> Result<proto::Bluetooth, ApiError>;
    async fn power(&self, on: bool) -> Result<proto::Bluetooth, ApiError>;
    async fn scan(&self, on: bool) -> Result<proto::Bluetooth, ApiError>;
    /// Starts pairing, without waiting: what it asks is the `btPrompt` event.
    async fn pair(&self, address: &str) -> Result<proto::Bluetooth, ApiError>;
    async fn connect(&self, address: &str) -> Result<proto::Bluetooth, ApiError>;
    async fn disconnect(&self, address: &str) -> Result<proto::Bluetooth, ApiError>;
    async fn forget(&self, address: &str) -> Result<proto::Bluetooth, ApiError>;
    /// Answers the prompt `id` (gone already: 404).
    async fn answer(&self, id: &str, answer: BtAnswer) -> Result<proto::Bluetooth, ApiError>;
}

/// WadOS's own image (bootc) and the base images (owner: the updates agent).
#[async_trait]
pub trait Updates: Send + Sync + 'static {
    async fn status(&self) -> Result<proto::Updates, ApiError>;
    /// An Update job ([`proto::updates::CHECK`]).
    async fn check(&self) -> Result<JobInfo, ApiError>;
    /// An Update job ([`proto::updates::APPLY`]) that ends with it staged.
    async fn apply(&self) -> Result<JobInfo, ApiError>;
    /// What's staged dropped: the next start stays on what's booted.
    async fn cancel(&self) -> Result<proto::Updates, ApiError>;
    /// The one before, staged.
    async fn rollback(&self) -> Result<proto::Updates, ApiError>;
    async fn bases(&self) -> Result<Bases, ApiError>;
    /// A Pull job, a stage per base.
    async fn refresh_bases(&self) -> Result<JobInfo, ApiError>;
}

/// The disk (owner: the storage agent).
#[async_trait]
pub trait Storage: Send + Sync + 'static {
    async fn storage(&self) -> Result<proto::Storage, ApiError>;
    /// A job ([`proto::storage::CLEANUP`]); what it freed is a notice.
    async fn cleanup(&self) -> Result<JobInfo, ApiError>;
}

/// None here: 501, "comes in phase 8".
pub struct NotYet;

fn not_yet<T>(what: &str) -> Result<T, ApiError> {
    Err(ApiError::not_implemented(format!("{what} comes in phase 8")))
}

#[async_trait]
impl Network for NotYet {
    async fn network(&self) -> Result<proto::Network, ApiError> {
        not_yet("The network")
    }
    async fn wifi(&self, _: bool) -> Result<proto::Network, ApiError> {
        not_yet("Turning Wi-Fi on and off")
    }
    async fn airplane(&self, _: bool) -> Result<proto::Network, ApiError> {
        not_yet("Airplane mode")
    }
    async fn connect(&self, _: WifiConnect) -> Result<proto::Network, ApiError> {
        not_yet("Joining a Wi-Fi network")
    }
    async fn disconnect(&self) -> Result<proto::Network, ApiError> {
        not_yet("Leaving a Wi-Fi network")
    }
    async fn forget(&self, _: &str) -> Result<proto::Network, ApiError> {
        not_yet("Forgetting a Wi-Fi network")
    }
    async fn exit_node(&self, _: Option<String>) -> Result<proto::Network, ApiError> {
        not_yet("An exit node")
    }
}

#[async_trait]
impl Bluetooth for NotYet {
    async fn bluetooth(&self) -> Result<proto::Bluetooth, ApiError> {
        not_yet("Bluetooth")
    }
    async fn power(&self, _: bool) -> Result<proto::Bluetooth, ApiError> {
        not_yet("Turning Bluetooth on and off")
    }
    async fn scan(&self, _: bool) -> Result<proto::Bluetooth, ApiError> {
        not_yet("Looking for Bluetooth devices")
    }
    async fn pair(&self, _: &str) -> Result<proto::Bluetooth, ApiError> {
        not_yet("Pairing")
    }
    async fn connect(&self, _: &str) -> Result<proto::Bluetooth, ApiError> {
        not_yet("Connecting a Bluetooth device")
    }
    async fn disconnect(&self, _: &str) -> Result<proto::Bluetooth, ApiError> {
        not_yet("Disconnecting a Bluetooth device")
    }
    async fn forget(&self, _: &str) -> Result<proto::Bluetooth, ApiError> {
        not_yet("Forgetting a Bluetooth device")
    }
    async fn answer(&self, _: &str, _: BtAnswer) -> Result<proto::Bluetooth, ApiError> {
        not_yet("Pairing")
    }
}

#[async_trait]
impl Updates for NotYet {
    async fn status(&self) -> Result<proto::Updates, ApiError> {
        not_yet("Updates")
    }
    async fn check(&self) -> Result<JobInfo, ApiError> {
        not_yet("Checking for updates")
    }
    async fn apply(&self) -> Result<JobInfo, ApiError> {
        not_yet("Applying an update")
    }
    async fn cancel(&self) -> Result<proto::Updates, ApiError> {
        not_yet("Cancelling an update")
    }
    async fn rollback(&self) -> Result<proto::Updates, ApiError> {
        not_yet("Rolling back")
    }
    async fn bases(&self) -> Result<Bases, ApiError> {
        not_yet("The base images")
    }
    async fn refresh_bases(&self) -> Result<JobInfo, ApiError> {
        not_yet("Refreshing the base images")
    }
}

#[async_trait]
impl Storage for NotYet {
    async fn storage(&self) -> Result<proto::Storage, ApiError> {
        not_yet("Storage")
    }
    async fn cleanup(&self) -> Result<JobInfo, ApiError> {
        not_yet("Cleaning up")
    }
}

/// The routes above, for [`crate::api::router`] (which checks the caller on each).
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/network", get(network))
        .route("/v1/network/wifi", post(wifi))
        .route("/v1/network/airplane", post(airplane))
        .route("/v1/network/wifi/connect", post(wifi_connect))
        .route("/v1/network/wifi/disconnect", post(wifi_disconnect))
        .route("/v1/network/wifi/forget", post(wifi_forget))
        .route("/v1/network/exit-node", post(exit_node))
        .route("/v1/bluetooth", get(bluetooth))
        .route("/v1/bluetooth/power", post(bt_power))
        .route("/v1/bluetooth/scan", post(bt_scan))
        .route("/v1/bluetooth/devices/{address}/{action}", post(bt_device))
        .route("/v1/bluetooth/prompt/{id}", post(bt_answer))
        .route("/v1/update", get(update))
        .route("/v1/update/check", post(update_check))
        .route("/v1/update/apply", post(update_apply))
        .route("/v1/update/cancel", post(update_cancel))
        .route("/v1/update/rollback", post(update_rollback))
        .route("/v1/bases", get(bases))
        .route("/v1/bases/refresh", post(bases_refresh))
        .route("/v1/storage", get(storage))
        .route("/v1/storage/cleanup", post(storage_cleanup))
}

type Answer<T> = Result<Json<T>, Failure>;
type App = State<Arc<AppState>>;

async fn network(State(app): App) -> Answer<proto::Network> {
    Ok(Json(app.network.network().await?))
}

async fn wifi(State(app): App, Json(s): Json<Switch>) -> Answer<proto::Network> {
    Ok(Json(app.network.wifi(s.on).await?))
}

async fn airplane(State(app): App, Json(s): Json<Switch>) -> Answer<proto::Network> {
    Ok(Json(app.network.airplane(s.on).await?))
}

async fn wifi_connect(State(app): App, Json(req): Json<WifiConnect>) -> Answer<proto::Network> {
    Ok(Json(app.network.connect(req).await?))
}

async fn wifi_disconnect(State(app): App) -> Answer<proto::Network> {
    Ok(Json(app.network.disconnect().await?))
}

async fn wifi_forget(State(app): App, Json(req): Json<WifiForget>) -> Answer<proto::Network> {
    Ok(Json(app.network.forget(&req.ssid).await?))
}

async fn exit_node(State(app): App, Json(req): Json<ExitNode>) -> Answer<proto::Network> {
    Ok(Json(app.network.exit_node(req.node).await?))
}

async fn bluetooth(State(app): App) -> Answer<proto::Bluetooth> {
    Ok(Json(app.bluetooth.bluetooth().await?))
}

async fn bt_power(State(app): App, Json(s): Json<Switch>) -> Answer<proto::Bluetooth> {
    Ok(Json(app.bluetooth.power(s.on).await?))
}

async fn bt_scan(State(app): App, Json(s): Json<Switch>) -> Answer<proto::Bluetooth> {
    Ok(Json(app.bluetooth.scan(s.on).await?))
}

async fn bt_device(State(app): App, Path((address, action)): Path<(String, String)>) -> Answer<proto::Bluetooth> {
    let bt = &app.bluetooth;
    let out = match action.as_str() {
        "pair" => bt.pair(&address).await?,
        "connect" => bt.connect(&address).await?,
        "disconnect" => bt.disconnect(&address).await?,
        "forget" => bt.forget(&address).await?,
        _ => {
            return Err(ApiError::not_found(format!("no route POST /v1/bluetooth/devices/{{address}}/{action}")).into());
        }
    };
    Ok(Json(out))
}

async fn bt_answer(State(app): App, Path(id): Path<String>, Json(a): Json<BtAnswer>) -> Answer<proto::Bluetooth> {
    Ok(Json(app.bluetooth.answer(&id, a).await?))
}

async fn update(State(app): App) -> Answer<proto::Updates> {
    Ok(Json(app.updates.status().await?))
}

async fn update_check(State(app): App) -> Answer<JobInfo> {
    Ok(Json(app.updates.check().await?))
}

async fn update_apply(State(app): App) -> Answer<JobInfo> {
    Ok(Json(app.updates.apply().await?))
}

async fn update_cancel(State(app): App) -> Answer<proto::Updates> {
    Ok(Json(app.updates.cancel().await?))
}

async fn update_rollback(State(app): App) -> Answer<proto::Updates> {
    Ok(Json(app.updates.rollback().await?))
}

async fn bases(State(app): App) -> Answer<Bases> {
    Ok(Json(app.updates.bases().await?))
}

async fn bases_refresh(State(app): App) -> Answer<JobInfo> {
    Ok(Json(app.updates.refresh_bases().await?))
}

async fn storage(State(app): App) -> Answer<proto::Storage> {
    Ok(Json(app.storage.storage().await?))
}

async fn storage_cleanup(State(app): App) -> Answer<JobInfo> {
    Ok(Json(app.storage.cleanup().await?))
}
