//! The installer stick and first boot (phase 9).
//!
//! | Route | |
//! |---|---|
//! | `GET /v1/install` | [`Install`]; 404 on anything but a stick |
//! | `POST /v1/install` | [`InstallRequest`] → an Install job |
//! | `GET /v1/setup` | [`Setup`] |
//! | `PUT /v1/setup` | [`Setup`] → [`Setup`] |
//! | `POST /v1/setup/design` | [`SetupDesign`] → [`Setup`] (done) |

use std::fmt;

use serde::{Deserialize, Serialize};

/// What an install does, in order: the stage ids and their words. The
/// first is only there when the stick has Cloudflare's keys ([`PROVISION`]).
pub const STAGES: [(&str, &str); 7] = [
    ("checking", "Checking the disk"),
    ("image", "Preparing the system image"),
    ("disk", "Erasing the disk"),
    ("install", "Installing WadSpaces (a few minutes)"),
    ("settings", "Writing this computer's settings and your account"),
    ("wifi", "Copying the stick's Wi-Fi networks"),
    ("finish", "Finishing"),
];
pub const PROVISION: (&str, &str) = ("address", "Setting up this computer's address");
/// The disk's stage, encrypting.
pub const ENCRYPTING: &str = "Erasing the disk and encrypting it";

/// A disk this could install on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallDisk {
    /// `/dev/disk/by-id/…`, which stays that disk.
    pub by_id: String,
    /// `nvme0n1`.
    pub name: String,
    pub model: String,
    pub serial: String,
    /// Bytes.
    pub size: u64,
    /// A USB disk: the machine would need it plugged in.
    pub removable: bool,
    /// What's on it now, or nothing.
    pub holds: Option<String>,
}

/// `GET /v1/install`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Install {
    pub disks: Vec<InstallDisk>,
    /// The stick has Cloudflare's keys: this machine gets a public address.
    pub public: bool,
    /// The install under way, or the last.
    pub job: Option<String>,
}

/// `POST /v1/install`. Its passwords never go in a log: they're not shown by `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRequest {
    /// The computer's name, and its hostname and id.
    pub name: String,
    pub username: String,
    pub password: String,
    pub by_id: String,
    pub size: u64,
    pub serial: String,
    /// The disk's password; none leaves it unencrypted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_password: Option<String>,
}

impl fmt::Debug for InstallRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InstallRequest")
            .field("name", &self.name)
            .field("username", &self.username)
            .field("by_id", &self.by_id)
            .field("encrypted", &self.disk_password.is_some())
            .finish_non_exhaustive()
    }
}

/// `POST /v1/discs`: a disc file of the person's, kept on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscAdd {
    /// The file, in the person's home.
    pub path: String,
    /// "Also keep it in your GitHub packages".
    #[serde(default)]
    pub push: bool,
}

/// A game a Steam account has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamGame {
    pub id: String,
    pub name: String,
    /// Downloaded on this machine.
    pub installed: bool,
    /// In the account's GitHub packages.
    pub pushed: bool,
}

/// A Steam account signed in here (`GET /v1/steam`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamAccount {
    /// Its key.
    pub id: String,
    pub name: String,
    pub games: Vec<SteamGame>,
}

/// `POST /v1/steam/signin`: its key's name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamSignIn {
    pub name: String,
}

/// `POST /v1/steam/push`: the games ticked in the push dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamPush {
    pub account: String,
    pub games: Vec<String>,
}

/// Where Set up WadSpaces is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SetupStep {
    Welcome,
    Wifi,
    Accounts,
    Design,
    /// Nothing to do: every machine the installer didn't make, too.
    #[default]
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setup {
    pub step: SetupStep,
    /// This machine has an NVIDIA card: Set up offers its image, already on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub nvidia_card: bool,
}

/// `POST /v1/setup/design`: the last step's Install or Skip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupDesign {
    /// Add the WadOS wadspace.
    pub starter: bool,
    /// Move to the host image with NVIDIA's driver, from the next start.
    #[serde(default)]
    pub nvidia: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_requests_passwords_never_show_in_debug() {
        let r = InstallRequest {
            name: "studio".into(),
            username: "sam".into(),
            password: "the-account-password".into(),
            by_id: "/dev/disk/by-id/x".into(),
            size: 1,
            serial: "s".into(),
            disk_password: Some("the-disk-password".into()),
        };
        let s = format!("{r:?}");
        assert!(!s.contains("password\":") && !s.contains("the-account-password") && !s.contains("the-disk-password"));
        assert!(s.contains("encrypted: true"));
    }

    #[test]
    fn the_wire_names_are_the_routes() {
        assert_eq!(
            serde_json::to_string(&Setup { step: SetupStep::Wifi, nvidia_card: false }).unwrap(),
            r#"{"step":"wifi"}"#
        );
        assert_eq!(serde_json::from_str::<Setup>(r#"{"step":"design"}"#).unwrap().step, SetupStep::Design);
    }
}
