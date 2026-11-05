//! Storage (phase 8, Settings ▸ Storage): the disk as base images,
//! wadspaces, everything else and free; each wadspace's image here.
//!
//! On wadd's socket (`api_phase8.rs`'s `Storage` trait):
//!
//! | Route | What |
//! |---|---|
//! | `GET /v1/storage` | [`Storage`] (also the `storage` event) |
//! | `POST /v1/storage/cleanup` | removes what nothing uses (old build layers, stopped containers, caches): a job ([`CLEANUP`]) → `JobInfo`; what it freed is a notice |
//!
//! A cleanup under way: 409.

use serde::{Deserialize, Serialize};
use wad_model::Built;

/// A cleanup job's stages: (id, label).
pub const CLEANUP: &[(&str, &str)] = &[
    ("images", "Removing images nothing uses"),
    ("containers", "Removing stopped containers"),
    ("caches", "Emptying the build caches"),
];

/// `GET /v1/storage` (the `storage` event): the disk wadd keeps images on, in bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Storage {
    pub total: u64,
    pub free: u64,
    /// The base images.
    pub bases: u64,
    /// The wadspaces' images and what they keep.
    pub wadspaces: u64,
    /// The system, and everything else.
    pub other: u64,
    /// This machine's wadspaces (not in the Trash), biggest first.
    #[serde(default)]
    pub per_wadspace: Vec<WadspaceStorage>,
}

impl Storage {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }
}

/// One wadspace on the disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WadspaceStorage {
    pub id: String,
    pub bytes: u64,
    /// Built, out of date (rebuilds when it next opens) or never.
    pub built: Built,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_reads_back() {
        let s = Storage {
            total: 100,
            free: 40,
            bases: 10,
            wadspaces: 30,
            other: 20,
            per_wadspace: vec![WadspaceStorage { id: "studio".into(), bytes: 30, built: Built::Update }],
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["perWadspace"][0]["built"], "update");
        assert_eq!(serde_json::from_value::<Storage>(v).unwrap(), s);
        assert_eq!(s.used(), 60);
    }
}
