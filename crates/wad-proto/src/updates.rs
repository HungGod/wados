//! Updates (phase 8, the Updates app): WadOS's own image (bootc: found,
//! downloaded and staged; the next start boots it; the one before is kept
//! to roll back to), and the base images wadspaces are built on.
//!
//! On wadd's socket (`api_phase8.rs`'s `Updates` trait):
//!
//! | Route | What |
//! |---|---|
//! | `GET /v1/update` | [`Updates`] (also the `updates` event) |
//! | `POST /v1/update/check` | looks for a newer image: an Update job ([`CHECK`]) → `JobInfo`; it ends with `available` set, or not |
//! | `POST /v1/update/apply` | downloads and stages `available`: an Update job ([`APPLY`]) → `JobInfo`; it ends `staged` |
//! | `POST /v1/update/cancel` | drops what's staged (it stays on `booted`) → [`Updates`] |
//! | `POST /v1/update/rollback` | stages `rollback` → [`Updates`] |
//! | `GET /v1/bases` | the base images here ([`Bases`]) |
//! | `POST /v1/bases/refresh` | pulls them again now: a Pull job, a stage per base → `JobInfo` |
//!
//! Nothing to apply, roll back to or cancel: 409; a check or download
//! under way: 409, its job's id in the message.

use serde::{Deserialize, Serialize};

/// An Update job's stages for a check: (id, label).
pub const CHECK: &[(&str, &str)] = &[("check", "Looking for an update")];
/// An Update job's stages for applying: (id, label).
pub const APPLY: &[(&str, &str)] = &[("download", "Downloading"), ("stage", "Getting it ready for the next start")];

/// `GET /v1/update` (the `updates` event): where WadOS's own image is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Updates {
    /// Its image comes from the registry and the registry answers (a
    /// stick's copy, or no network: not).
    pub online: bool,
    /// A check is under way.
    pub checking: bool,
    /// What it's running.
    pub booted: Version,
    /// Newer, on the registry (what the last check found).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available: Option<Version>,
    /// What the next start boots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged: Option<Version>,
    /// The one before, kept to go back to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback: Option<Version>,
    /// Why the last check or download failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// When it last looked (ms since the epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<i64>,
    /// While downloading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<UpdateProgress>,
    /// The Update job under way (a check or a download).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
}

/// One of WadOS's images (bootc's deployment).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    /// `44.20261104.0`: Fedora's release, the day it was built, the build that day.
    pub version: String,
    /// Where it comes from (`ghcr.io/hunggod/wados-host:44`).
    pub image: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

/// How far a download is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub done_bytes: u64,
    pub total_bytes: u64,
}

/// `GET /v1/bases`: the base images wadspaces are built on, refreshed every 12 hours.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bases {
    pub bases: Vec<Base>,
    /// When they're pulled again by themselves (ms since the epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_refresh_at: Option<i64>,
}

/// A base image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Base {
    /// What people read ("Fedora 44 base").
    pub name: String,
    /// Its reference (`ghcr.io/hunggod/wados-base:44`).
    pub image: String,
    /// Its size here, in bytes (none: not here).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// When it was last pulled (ms since the epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refreshed_at: Option<i64>,
    /// Not on this machine: the next build pulls it.
    #[serde(default)]
    pub missing: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn updates_read_as_the_spikes() {
        let u: Updates = serde_json::from_value(json!({
            "online": true, "checking": false,
            "booted": {"version": "44.20261104.0", "image": "ghcr.io/hunggod/wados-host:44", "digest": "sha256:aaa"},
            "available": {"version": "44.20261105.0", "image": "ghcr.io/hunggod/wados-host:44"},
            "progress": {"doneBytes": 5, "totalBytes": 10},
        }))
        .unwrap();
        assert_eq!(u.available.as_ref().map(|v| v.version.as_str()), Some("44.20261105.0"));
        assert_eq!(u.progress, Some(UpdateProgress { done_bytes: 5, total_bytes: 10 }));
        assert_eq!(serde_json::from_value::<Updates>(serde_json::to_value(&u).unwrap()).unwrap(), u);
        let b: Bases =
            serde_json::from_value(json!({"bases": [{"name": "Base", "image": "x", "missing": true}]})).unwrap();
        assert!(b.bases[0].missing && b.bases[0].size.is_none());
    }
}
