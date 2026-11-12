//! The machine's network (wad-net: NetworkManager over D-Bus) for Settings
//! ▸ Wi-Fi and Network, and the tray: [`api_phase8::Network`]. It's looked
//! at every few seconds and goes out as the `network` event when it
//! changes (a change made here is told at once).
//!
//! - **Airplane mode** ([`Airplane`]): Wi-Fi off and Bluetooth off, kept
//!   in wadd's state (`airplane.json`) so it stays on across restarts; off
//!   again, Wi-Fi comes back on, and Bluetooth if it was on before.
//! - **The exit node**: the tailnet's ([`crate::tailnet::Tailnet`]): the
//!   machines of mine that offer to be one (Tailscale's `ExitNodeOption`),
//!   by the spike's rule ([`exit_node_choices`]), then any other that
//!   offers (a tagged server); set with `tailscale set --exit-node`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use wad_net::NetworkManager;
use wad_proto as proto;
use wad_proto::network::exit_node_choices;
use wad_proto::{ApiError, ErrorCode, Event, ExitNodeChoice, Tailnet, WifiConnect};

use crate::api_phase6::Net;
use crate::api_phase8;
use crate::bluetooth::MachineBluetooth;
use crate::events::Bus;
use crate::tailnet::{self, ExitOffer};

/// A fresh scan for the networks in range, this often (NetworkManager's
/// own are minutes apart once it's on one).
const RESCAN_EVERY: Duration = Duration::from_secs(60);

/// wad-net's error as the API says it.
pub fn api_error(e: wad_net::Error) -> ApiError {
    use wad_net::Error as E;
    let code = match &e {
        E::Bus(zbus::Error::MethodError(..)) => ErrorCode::Upstream,
        E::Bus(_) => ErrorCode::Offline,
        E::NoWifi | E::WifiOff | E::Blocked => ErrorCode::Conflict,
        E::NotInRange(_) | E::NotSaved(_) => ErrorCode::NotFound,
        E::NeedsPassword(_) | E::BadPassword | E::Unsupported(_) => ErrorCode::BadRequest,
        E::WrongPassword(_) => ErrorCode::Unauthorized,
        E::Failed(_) => ErrorCode::Upstream,
    };
    ApiError::new(code, e.to_string())
}

/// What can be the exit node: the spike's rule over the tailnet's devices,
/// those that offer it (and are online); then the others that offer it.
pub fn exit_choices(view: &Tailnet, offers: &[ExitOffer]) -> Vec<ExitNodeChoice> {
    let offered = |id: &str| offers.iter().any(|o| o.id == id && o.online);
    let mut out: Vec<ExitNodeChoice> = exit_node_choices(view).into_iter().filter(|c| offered(&c.id)).collect();
    if view.state != proto::TailnetState::Running {
        return out;
    }
    for o in offers.iter().filter(|o| o.online) {
        if !out.iter().any(|c| c.id == o.id) {
            out.push(ExitNodeChoice {
                id: o.id.clone(),
                name: o.name.clone(),
                os: o.os.clone(),
                kind: tailnet::device_kind(&o.os, "", o.tagged),
            });
        }
    }
    out
}

/// What airplane mode keeps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Kept {
    on: bool,
    /// Bluetooth was on before it.
    bluetooth: bool,
}

/// Airplane mode: on or not, kept in wadd's state.
pub struct Airplane {
    file: PathBuf,
    on: AtomicBool,
    kept: Mutex<Kept>,
}

impl Airplane {
    pub fn new(state_dir: &std::path::Path) -> Arc<Self> {
        let file = state_dir.join("airplane.json");
        let kept: Kept = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Arc::new(Self { file, on: AtomicBool::new(kept.on), kept: Mutex::new(kept) })
    }

    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    /// Bluetooth was on when it went on.
    fn bluetooth_was_on(&self) -> bool {
        self.kept.lock().unwrap().bluetooth
    }

    fn set(&self, on: bool, bluetooth: bool) {
        let k = Kept { on, bluetooth };
        *self.kept.lock().unwrap() = k;
        self.on.store(on, Ordering::Relaxed);
        if let Some(d) = self.file.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Err(e) = std::fs::write(&self.file, serde_json::to_vec(&k).expect("json")) {
            tracing::warn!("airplane mode: {}: {e}", self.file.display());
        }
    }
}

pub struct MachineNetwork {
    /// The bus NetworkManager is on ("": the system's).
    address: String,
    nm: tokio::sync::Mutex<Option<Arc<NetworkManager>>>,
    airplane: Arc<Airplane>,
    bluetooth: Arc<MachineBluetooth>,
    /// The tailnet (none: Tailscale's off here), for the exit node.
    tailnet: Option<Arc<tailnet::Tailnet>>,
    /// Its view: the devices' names and kinds.
    net: Arc<dyn Net>,
    bus: Bus,
    last: Mutex<Option<proto::Network>>,
    poke: Notify,
    every: Duration,
}

impl MachineNetwork {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        address: &str,
        airplane: Arc<Airplane>,
        bluetooth: Arc<MachineBluetooth>,
        tailnet: Option<Arc<tailnet::Tailnet>>,
        net: Arc<dyn Net>,
        bus: Bus,
        every: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            address: address.into(),
            nm: tokio::sync::Mutex::default(),
            airplane,
            bluetooth,
            tailnet,
            net,
            bus,
            last: Mutex::default(),
            poke: Notify::new(),
            every,
        })
    }

    async fn nm(&self) -> Result<Arc<NetworkManager>, ApiError> {
        let mut g = self.nm.lock().await;
        if let Some(n) = g.as_ref() {
            return Ok(n.clone());
        }
        let r = if self.address.is_empty() {
            NetworkManager::system().await
        } else {
            match zbus::connection::Builder::address(self.address.as_str()) {
                Ok(b) => b.build().await.map(NetworkManager::on).map_err(wad_net::Error::from),
                Err(e) => Err(e.into()),
            }
        };
        let n = Arc::new(r.map_err(|e| ApiError::new(ErrorCode::Offline, format!("NetworkManager: {e}")))?);
        *g = Some(n.clone());
        Ok(n)
    }

    /// The exit node in use and what can be one.
    async fn exit(&self) -> (Option<String>, Vec<ExitNodeChoice>) {
        let Some(t) = &self.tailnet else { return (None, vec![]) };
        let (using, offers) = t.exit_nodes();
        let view = self.net.tailnet().await.unwrap_or_default();
        (using, exit_choices(&view, &offers))
    }

    /// The network now, told when it changed.
    pub async fn refresh(&self) -> Result<proto::Network, ApiError> {
        let nm = self.nm().await?;
        let s = match nm.state().await {
            Ok(s) => s,
            Err(e @ wad_net::Error::Bus(zbus::Error::InputOutput(_))) => {
                *self.nm.lock().await = None;
                return Err(api_error(e));
            }
            Err(e) => return Err(api_error(e)),
        };
        let (exit_node, exit_nodes) = self.exit().await;
        let n = proto::Network {
            wifi_on: s.wifi_on,
            airplane: self.airplane.is_on(),
            connected: s.connected,
            ethernet: s.ethernet,
            networks: s.networks,
            ip: s.ip,
            dns: s.dns,
            portal: s.portal,
            exit_node,
            exit_nodes,
        };
        let changed = self.last.lock().unwrap().replace(n.clone()).as_ref() != Some(&n);
        if changed {
            self.bus.publish(Event::Network(Box::new(n.clone())));
        }
        Ok(n)
    }

    /// Follows NetworkManager for as long as wadd runs (asking for a fresh
    /// scan now and then while Wi-Fi's on).
    pub async fn run(self: Arc<Self>) {
        let mut scanned = tokio::time::Instant::now() - RESCAN_EVERY;
        loop {
            match self.refresh().await {
                Ok(n) if n.wifi_on && scanned.elapsed() >= RESCAN_EVERY => {
                    scanned = tokio::time::Instant::now();
                    if let Ok(nm) = self.nm().await {
                        let _ = nm.rescan().await;
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::debug!("network: {}", e.message),
            }
            tokio::select! {
                _ = tokio::time::sleep(self.every) => {}
                _ = self.poke.notified() => {}
            }
        }
    }

    fn no_airplane(&self) -> Result<(), ApiError> {
        if self.airplane.is_on() { Err(ApiError::conflict("Airplane mode is on")) } else { Ok(()) }
    }
}

#[async_trait]
impl api_phase8::Network for Arc<MachineNetwork> {
    async fn network(&self) -> Result<proto::Network, ApiError> {
        self.refresh().await
    }

    async fn wifi(&self, on: bool) -> Result<proto::Network, ApiError> {
        if on {
            self.no_airplane()?;
        }
        self.nm().await?.set_wifi(on).await.map_err(api_error)?;
        self.refresh().await
    }

    async fn airplane(&self, on: bool) -> Result<proto::Network, ApiError> {
        let nm = self.nm().await?;
        if on == self.airplane.is_on() {
            return self.refresh().await;
        }
        if on {
            nm.set_wifi(false).await.map_err(api_error)?;
            let bt = self.bluetooth.off_for_airplane().await.unwrap_or(false);
            self.airplane.set(true, bt);
            tracing::info!("airplane mode on");
        } else {
            let bt = self.airplane.bluetooth_was_on();
            self.airplane.set(false, false);
            if let Err(e) = nm.set_wifi(true).await {
                tracing::info!("airplane mode off: Wi-Fi stays off: {e}");
            }
            if bt {
                self.bluetooth.on_after_airplane().await;
            }
            tracing::info!("airplane mode off");
        }
        self.refresh().await
    }

    async fn connect(&self, req: WifiConnect) -> Result<proto::Network, ApiError> {
        self.no_airplane()?;
        tracing::info!(ssid = %req.ssid, "joining a Wi-Fi network");
        let r = self.nm().await?.connect(&req.ssid, req.password.as_deref()).await.map_err(api_error);
        let n = self.refresh().await;
        r?;
        n
    }

    async fn disconnect(&self) -> Result<proto::Network, ApiError> {
        self.nm().await?.disconnect().await.map_err(api_error)?;
        self.refresh().await
    }

    async fn forget(&self, ssid: &str) -> Result<proto::Network, ApiError> {
        self.nm().await?.forget(ssid).await.map_err(api_error)?;
        self.refresh().await
    }

    async fn exit_node(&self, node: Option<String>) -> Result<proto::Network, ApiError> {
        let Some(t) = &self.tailnet else { return Err(ApiError::conflict("Join Tailscale first")) };
        t.set_exit_node(node.as_deref()).await?;
        self.refresh().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_proto::{DeviceKind, TailnetDevice, TailnetState};

    fn offer(id: &str, online: bool, tagged: bool) -> ExitOffer {
        ExitOffer { id: id.into(), name: id.into(), os: "linux".into(), online, tagged }
    }

    #[test]
    fn exit_nodes_are_my_computers_that_offer_then_any_other_that_does() {
        let dev = |id: &str, kind, online| TailnetDevice {
            id: id.into(),
            name: format!("{id} name"),
            os: "linux".into(),
            kind,
            online,
            trusted: true,
            mine: true,
        };
        let view = Tailnet {
            state: TailnetState::Running,
            devices: vec![
                dev("desk", DeviceKind::Desktop, true),
                dev("laptop", DeviceKind::Laptop, true),
                dev("phone", DeviceKind::Phone, true),
            ],
            ..Default::default()
        };
        let offers = [offer("desk", true, false), offer("vps", true, true), offer("nas", false, true)];
        let c = exit_choices(&view, &offers);
        let ids: Vec<&str> = c.iter().map(|c| c.id.as_str()).collect();
        // The laptop doesn't offer; the server offline isn't one now.
        assert_eq!(ids, ["desk", "vps"]);
        assert_eq!(c[0].name, "desk name");
        assert_eq!(c[1].kind, DeviceKind::Server);
        let off = Tailnet { state: TailnetState::Stopped, ..view };
        assert!(exit_choices(&off, &offers).is_empty());
    }

    #[test]
    fn airplane_mode_is_kept() {
        let d = tempfile::tempdir().unwrap();
        let a = Airplane::new(d.path());
        assert!(!a.is_on());
        a.set(true, true);
        let again = Airplane::new(d.path());
        assert!(again.is_on() && again.bluetooth_was_on());
        assert_eq!(api_error(wad_net::Error::WrongPassword("Home".into())).code, ErrorCode::Unauthorized);
        assert_eq!(api_error(wad_net::Error::NotInRange("x".into())).code, ErrorCode::NotFound);
    }
}
