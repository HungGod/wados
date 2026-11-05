//! The network (phase 8, Settings ▸ Wi-Fi and Network): Wi-Fi, airplane
//! mode, ethernet, and the exit node (the machine of mine this one's
//! internet traffic goes out through, over the tailnet).
//!
//! On wadd's socket (`api_phase8.rs`'s `Network` trait), each answering
//! with [`Network`] as it is after (also the `network` event):
//!
//! | Route | What |
//! |---|---|
//! | `GET /v1/network` | [`Network`] |
//! | `POST /v1/network/wifi` | Wi-Fi on or off ([`Switch`]) |
//! | `POST /v1/network/airplane` | airplane mode ([`Switch`]): every radio off, Bluetooth too |
//! | `POST /v1/network/wifi/connect` | joins a network ([`WifiConnect`]); once it's joined (a wrong password: 401) |
//! | `POST /v1/network/wifi/disconnect` | leaves the one it's on |
//! | `POST /v1/network/wifi/forget` | forgets a saved one ([`WifiForget`]) |
//! | `POST /v1/network/exit-node` | out through a machine of mine, or straight out ([`ExitNode`]) |

use serde::{Deserialize, Serialize};

use crate::tailnet::{DeviceKind, Tailnet, TailnetDevice};

/// `GET /v1/network` (the `network` event).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Network {
    pub wifi_on: bool,
    /// Every radio off (Wi-Fi and Bluetooth).
    pub airplane: bool,
    /// The Wi-Fi network it's on (its SSID); none: not on one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected: Option<String>,
    pub ethernet: Ethernet,
    /// The Wi-Fi networks in range, and the saved ones out of range: the one
    /// it's on first, then the saved ones, then by signal.
    #[serde(default)]
    pub networks: Vec<WifiNetwork>,
    /// This machine's address on the network it's on (IPv4, else IPv6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    /// The DNS servers it uses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dns: Vec<String>,
    /// The tailnet device its internet traffic goes out through (`tailscale
    /// set --exit-node`), by its node's stable id; none: straight out. It
    /// may be offline (then nothing gets out until it's back).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_node: Option<String>,
    /// What can be one now ([`exit_node_choices`]: online machines on the
    /// tailnet that offer it), for Settings' list. Empty while the tailnet isn't running.
    #[serde(default)]
    pub exit_nodes: Vec<ExitNodeChoice>,
}

/// The wired network.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ethernet {
    /// A cable in, with a link.
    pub plugged: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mbps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
}

/// A Wi-Fi network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WifiNetwork {
    pub ssid: String,
    /// 0-100; 0 when it's saved and out of range.
    pub signal: u8,
    /// It needs a password (WPA2/WPA3).
    pub secure: bool,
    /// Its password is saved: it joins without asking.
    pub saved: bool,
    /// The one it's on.
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band: Option<WifiBand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WifiBand {
    #[serde(rename = "2.4GHz")]
    Ghz2,
    #[serde(rename = "5GHz")]
    Ghz5,
    #[serde(rename = "6GHz")]
    Ghz6,
}

/// A machine on the tailnet the traffic can go out through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExitNodeChoice {
    /// Its node's stable id (as [`TailnetDevice::id`]): what [`ExitNode`] takes.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub os: String,
    pub kind: DeviceKind,
}

impl From<&TailnetDevice> for ExitNodeChoice {
    fn from(d: &TailnetDevice) -> Self {
        Self { id: d.id.clone(), name: d.name.clone(), os: d.os.clone(), kind: d.kind }
    }
}

/// The spike's rule (Settings ▸ Network): online machines on the tailnet
/// (desktops, laptops, servers; not phones or tablets), while it's running.
/// The machine's wadd also asks Tailscale which of them offer to be one
/// (`ExitNodeOption`); this is the rule without it.
pub fn exit_node_choices(t: &Tailnet) -> Vec<ExitNodeChoice> {
    if t.state != crate::TailnetState::Running {
        return vec![];
    }
    t.devices
        .iter()
        .filter(|d| d.online && matches!(d.kind, DeviceKind::Desktop | DeviceKind::Laptop | DeviceKind::Server))
        .map(ExitNodeChoice::from)
        .collect()
}

/// `POST /v1/network/wifi`, `/airplane` (and Bluetooth's `/power`, `/scan`): on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Switch {
    pub on: bool,
}

/// `POST /v1/network/wifi/connect`: joins `ssid`; a secure one not saved
/// needs its password. Its `Debug` keeps the password out of logs.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WifiConnect {
    pub ssid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

impl std::fmt::Debug for WifiConnect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WifiConnect")
            .field("ssid", &self.ssid)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// `POST /v1/network/wifi/forget`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiForget {
    pub ssid: String,
}

/// `POST /v1/network/exit-node`: out through `node` (an [`ExitNodeChoice`]'s
/// id), or (none) straight out. Not one: 404; the tailnet not running: 409.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitNode {
    #[serde(default)]
    pub node: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_network_reads_as_the_spikes_and_keeps_its_password_out_of_logs() {
        let n: Network = serde_json::from_value(json!({
            "wifiOn": true, "airplane": false, "connected": "Home",
            "ethernet": {"plugged": true, "speedMbps": 1000},
            "networks": [{"ssid": "Home", "signal": 80, "secure": true, "saved": true, "active": true, "band": "5GHz"}],
        }))
        .unwrap();
        assert_eq!(n.networks[0].band, Some(WifiBand::Ghz5));
        assert_eq!(n.ethernet.speed_mbps, Some(1000));
        assert!(n.exit_nodes.is_empty() && n.dns.is_empty());
        assert_eq!(serde_json::from_value::<Network>(serde_json::to_value(&n).unwrap()).unwrap(), n);

        let c = WifiConnect { ssid: "Cafe".into(), password: Some("hunter22".into()) };
        let shown = format!("{c:?}");
        assert!(!shown.contains("hunter22") && shown.contains("Cafe"), "{shown}");
        let open: WifiConnect = serde_json::from_value(json!({"ssid": "Library"})).unwrap();
        assert_eq!(open.password, None);
        let straight: ExitNode = serde_json::from_value(json!({})).unwrap();
        assert_eq!(straight.node, None);
    }

    #[test]
    fn exit_nodes_are_online_computers_on_a_running_tailnet() {
        let dev = |id: &str, kind, online| TailnetDevice {
            id: id.into(),
            name: id.into(),
            os: "linux".into(),
            kind,
            online,
            trusted: true,
            mine: true,
        };
        let mut t = Tailnet {
            state: crate::TailnetState::Running,
            devices: vec![
                dev("desk", DeviceKind::Desktop, true),
                dev("phone", DeviceKind::Phone, true),
                dev("nas", DeviceKind::Server, false),
            ],
            ..Default::default()
        };
        let ids: Vec<String> = exit_node_choices(&t).into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["desk"]);
        t.state = crate::TailnetState::Stopped;
        assert!(exit_node_choices(&t).is_empty());
    }
}
