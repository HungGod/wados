//! The tailnet (Tailscale): this machine on it, the devices there, the ones
//! asking to be trusted, and the trust book every machine of yours follows.
//!
//! Who serves what (phase 6; wadd's `tailnet.rs`, `remote.rs`):
//!
//! - **The socket** (`/run/wadd/wadd.sock`), for the shell and the apps:
//!   `GET /v1/tailnet` → [`Tailnet`] (also the `tailnet` event);
//!   `POST /v1/tailnet/login` → a Tailnet job; `POST /v1/tailnet/logout` →
//!   [`Tailnet`]; `PUT`/`DELETE /v1/tailnet/devices/{id}` (trust, untrust)
//!   and `POST /v1/tailnet/asks/{id}/{deny,later}` → [`Tailnet`].
//! - **The remote API** (the tailnet's addresses, port 8087), for other
//!   devices: `GET /v1/remote/hello` → [`RemoteHello`] to anyone;
//!   `GET /v1/remote/machine` → [`RemoteMachine`] to my machines and
//!   trusted devices; `GET /v1/remote/trust` → [`TrustBook`] and
//!   `POST /v1/remote/trust` ([`TrustAnswer`]) to my machines only.

use serde::{Deserialize, Serialize};
use wad_model::PeerKind;

use crate::MachineInfo;

/// The remote API's port, on the tailnet's addresses.
pub const REMOTE_PORT: u16 = 8087;

/// Where this machine is with the tailnet (the spike's seven).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TailnetState {
    /// No tailscaled, or it's turned off.
    #[default]
    Off,
    /// Not logged in: `POST /v1/tailnet/login` starts it.
    NeedsLogin,
    /// Waiting for the sign-in at `loginUrl` (in the browser).
    LoggingIn,
    /// Logged in; the tailnet's admin has to approve the machine.
    NeedsApproval,
    /// Logged in, and down (`tailscale down`).
    Stopped,
    Starting,
    Running,
}

/// What a device is, as far as the tailnet can tell (from its OS and model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Phone,
    Tablet,
    Laptop,
    Desktop,
    Server,
    /// It can't tell.
    Other,
}

/// `GET /v1/tailnet` (the `tailnet` event): this machine on the tailnet,
/// the devices there, and the ones asking to be trusted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tailnet {
    pub state: TailnetState,
    /// The tailnet's name, as Tailscale has it (its `CurrentTailnet.Name`:
    /// 'example.com', 'you@example.com', or a Headscale's server URL host,
    /// not its MagicDNS base domain: found on the test tailnet); empty
    /// until it's logged in. Taildrive's paths start with it: a peer's home is
    /// `dav://100.100.100.100:8080/<tailnet>/<its hostname>/home`.
    #[serde(default)]
    pub tailnet: String,
    /// This machine's name on it (its MagicDNS name's first label: what
    /// other devices and Taildrive's paths call it).
    #[serde(default)]
    pub hostname: String,
    /// This machine's tailnet address (IPv4); empty when it has none.
    #[serde(default)]
    pub ip: String,
    /// While logging in: where to sign in (the `signIn` event opens it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_url: Option<String>,
    /// The devices on the tailnet this machine knows: my machines, the
    /// devices of the login's own, and the ones the trust book has an answer
    /// for (online or not).
    #[serde(default)]
    pub devices: Vec<TailnetDevice>,
    /// Devices asking to be trusted now (at most 8, newest last), less the
    /// ones put off with Later this session.
    #[serde(default)]
    pub asks: Vec<TailnetAsk>,
    /// Taildrive: this machine's person's home, shared on the tailnet as
    /// theirs once it's running (what my other machines browse and mount).
    #[serde(default)]
    pub drive: DriveState,
    /// Why it isn't shared, when sharing failed (Tailscale's words).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive_error: Option<String>,
}

/// Whether this machine's person's home is shared on the tailnet (Taildrive).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DriveState {
    /// Not yet: the tailnet isn't running, or it's being shared.
    #[default]
    Off,
    /// Shared as `home`: `dav://100.100.100.100:8080/<tailnet>/<hostname>/home`.
    Shared,
    /// The tailnet's policy doesn't let this machine share (it needs the
    /// `drive:share` node attribute, and the machines that browse it
    /// `drive:access`): Settings ▸ Accounts explains it.
    NotAllowed,
    /// It failed for another reason (`driveError` says why).
    Failed,
}

/// A device on the tailnet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TailnetDevice {
    /// Its node's stable id: what the trust book and the routes know it by.
    pub id: String,
    pub name: String,
    /// What Tailscale says it runs ('android', 'linux', 'iOS'…).
    #[serde(default)]
    pub os: String,
    pub kind: DeviceKind,
    pub online: bool,
    /// It may use this machine (the remote API past `hello`): one of my
    /// machines, or the trust book says so.
    pub trusted: bool,
    /// One of my machines (a wadd of the same login): trusted without an ask,
    /// and it can't be untrusted.
    #[serde(default)]
    pub mine: bool,
}

/// A device asking to be trusted (it called the remote API and isn't).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TailnetAsk {
    /// The device's node stable id (one ask per device): `PUT
    /// /v1/tailnet/devices/{id}` trusts it, `POST /v1/tailnet/asks/{id}/…`
    /// answers otherwise.
    pub id: String,
    /// Its name on the tailnet.
    pub device: String,
    #[serde(default)]
    pub os: String,
    pub kind: DeviceKind,
    /// Its tailnet address.
    pub ip: String,
    /// When it asked (ms since the epoch).
    pub at: i64,
}

// ------------------------------------------------------------ the trust book

/// What a device was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Answer {
    /// It may use this machine.
    Trusted,
    /// It stays quiet: `hello` only, and no new ask.
    Denied,
    /// Trusted no more: it's a device the machine doesn't know again (its
    /// next call asks). Kept in the book so a merge doesn't bring the trust back.
    Untrusted,
}

/// One answer about a device: a trust book's entry, and what's pushed to my
/// other machines (`POST /v1/remote/trust`). Per device, the latest `at` wins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustAnswer {
    /// The device's node stable id.
    pub device: String,
    /// Its name and OS when it was answered (to show it while it's offline).
    pub name: String,
    #[serde(default)]
    pub os: String,
    pub answer: Answer,
    /// When (ms since the epoch).
    pub at: i64,
    /// The machine that answered (its wadd machine id).
    pub by: String,
}

/// The trust book (`<state>/tailnet-devices.json`, mode 0600; `GET
/// /v1/remote/trust`): the answers about devices, for one login on one
/// tailnet. It's wiped when either changes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustBook {
    /// The Tailscale login it's for ('you@example.com').
    pub user: String,
    pub tailnet: String,
    #[serde(default)]
    pub devices: Vec<TrustAnswer>,
}

// ------------------------------------------------------------ the remote API

/// How this machine sees the device calling (`hello`'s `trust`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteTrust {
    /// One of my machines.
    Machine,
    /// Trusted by the trust book.
    Trusted,
    /// Not trusted: it's asking (or was put off with Later).
    Asked,
    /// Denied: `hello` only.
    Denied,
}

/// `GET /v1/remote/hello`, to anyone on the tailnet: this is a wadd, which
/// machine, and how it sees the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteHello {
    /// Always true: how a peer tells a wadd from anything else on 8087.
    pub wadd: bool,
    pub machine: MachineInfo,
    pub trust: RemoteTrust,
}

/// `GET /v1/remote/machine`, to my machines and trusted devices: what a
/// peer card shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteMachine {
    /// Its wadd machine id.
    pub id: String,
    pub name: String,
    /// What it runs ('WadOS 44.20261021.0 · Fedora 44').
    pub os: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<String>,
    pub kind: PeerKind,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_tailnet_reads_as_the_spikes() {
        let t: Tailnet = serde_json::from_value(json!({
            "state": "needsApproval", "tailnet": "tail1234.ts.net", "hostname": "surface", "ip": "100.101.102.103",
            "loginUrl": "https://login.tailscale.com/a/x",
            "devices": [{"id": "nPhone1", "name": "Pixel 9", "os": "android", "kind": "phone", "online": true, "trusted": true}],
            "asks": [{"id": "nTab1", "device": "Galaxy Tab S9", "os": "android", "kind": "tablet", "ip": "100.88.20.7", "at": 5}],
        }))
        .unwrap();
        assert_eq!(t.state, TailnetState::NeedsApproval);
        assert!(!t.devices[0].mine);
        assert_eq!(t.asks[0].kind, DeviceKind::Tablet);
        let back = serde_json::to_value(&t).unwrap();
        assert_eq!(back["loginUrl"], "https://login.tailscale.com/a/x");
        assert_eq!(serde_json::from_value::<Tailnet>(back).unwrap(), t);
        assert_eq!(t.drive, DriveState::Off);
        let shared: Tailnet = serde_json::from_value(json!({"state": "running", "drive": "notAllowed"})).unwrap();
        assert_eq!(shared.drive, DriveState::NotAllowed);
        // Off, with nothing else: what a machine without tailscaled says.
        let off: Tailnet = serde_json::from_value(json!({"state": "off"})).unwrap();
        assert_eq!(off, Tailnet::default());
        for (s, word) in [(TailnetState::NeedsLogin, "needsLogin"), (TailnetState::LoggingIn, "loggingIn")] {
            assert_eq!(serde_json::to_value(s).unwrap(), word);
        }
    }

    #[test]
    fn the_trust_book_and_the_remote_apis_answers() {
        let book: TrustBook = serde_json::from_value(json!({
            "user": "you@example.com", "tailnet": "tail1234.ts.net",
            "devices": [{"device": "nPhone1", "name": "Pixel 9", "os": "android", "answer": "untrusted", "at": 7, "by": "m-desk"}],
        }))
        .unwrap();
        assert_eq!(book.devices[0].answer, Answer::Untrusted);
        assert_eq!(serde_json::from_value::<TrustBook>(serde_json::to_value(&book).unwrap()).unwrap(), book);

        let hello = RemoteHello {
            wadd: true,
            machine: MachineInfo {
                id: "m-desk".into(),
                name: "Desk".into(),
                version: "0.1.0".into(),
                profile: crate::Profile::System,
            },
            trust: RemoteTrust::Asked,
        };
        let v = serde_json::to_value(&hello).unwrap();
        assert_eq!((v["wadd"].as_bool(), v["trust"].as_str()), (Some(true), Some("asked")));
        assert_eq!(serde_json::from_value::<RemoteHello>(v).unwrap(), hello);
        let m: RemoteMachine = serde_json::from_value(
            json!({"id": "m-desk", "name": "Desk", "os": "WadOS 44", "gpu": "AMD Radeon 780M", "kind": "desktop"}),
        )
        .unwrap();
        assert_eq!(m.kind, PeerKind::Desktop);
    }
}
