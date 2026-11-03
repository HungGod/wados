//! The index CI publishes (the `packages` release's index.json): what it
//! pushed for each base ABI, which machines read before pulling a package.
//!
//! ```json
//! {"version": 1, "registry": "ghcr.io/hunggod",
//!  "abis": {"<abi>": {"base": "<digest>",
//!                     "packages": {"<id>": {"digest", "imageId", "version", "size", "needs", "content", "built"}}}}}
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::packages::{REGISTRY, source};

/// The index's version.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Index {
    pub version: u32,
    pub registry: String,
    #[serde(default)]
    pub abis: BTreeMap<String, Abi>,
}

/// One base ABI's packages.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Abi {
    /// The base CI built them on (its registry digest).
    #[serde(default)]
    pub base: String,
    #[serde(default)]
    pub packages: BTreeMap<String, Published>,
}

/// A package CI pushed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Published {
    /// Its registry digest (what a machine pulls by).
    pub digest: String,
    /// Its image id (what a machine tags it by: [`crate::packages::local_ref`]).
    pub image_id: String,
    pub version: String,
    /// Bytes, as podman counts them.
    pub size: u64,
    pub needs: Vec<String>,
    /// A hash of its files (`io.wadspaces.pkg.content`).
    pub content: String,
    /// When (RFC 3339).
    pub built: String,
}

/// What CI's run pushed for one package (its `result-<id>.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pushed {
    pub id: String,
    #[serde(flatten)]
    pub package: Published,
}

impl Default for Index {
    fn default() -> Self {
        Index { version: VERSION, registry: REGISTRY.to_string(), abis: BTreeMap::new() }
    }
}

impl Index {
    /// The index with this run's packages in, for `abi` on `base`. Other
    /// ABIs' entries stay, so machines on an older base still find theirs;
    /// an index of another version is started again. Anything that isn't a
    /// package WadOS knows is left out.
    pub fn merge(old: Option<Index>, abi: &str, base: &str, pushed: &[Pushed]) -> Index {
        let mut index = old.filter(|i| i.version == VERSION).unwrap_or_default();
        let entry = index.abis.entry(abi.to_string()).or_default();
        entry.base = base.to_string();
        for p in pushed.iter().filter(|p| source(&p.id).is_some()) {
            entry.packages.insert(p.id.clone(), p.package.clone());
        }
        index
    }

    /// What it says of package `id` for `abi`, if CI published it.
    pub fn get(&self, abi: &str, id: &str) -> Option<&Published> {
        self.abis.get(abi)?.packages.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pushed(id: &str, version: &str) -> Pushed {
        Pushed {
            id: id.into(),
            package: Published { version: version.into(), digest: "sha256:d".into(), ..Default::default() },
        }
    }

    #[test]
    fn the_index_keeps_other_abis_and_takes_new_packages() {
        let old: Index = serde_json::from_value(serde_json::json!({
            "version": 1, "registry": REGISTRY,
            "abis": {
                "f44.0": {"base": "sha256:old", "packages": {"cpp": {"version": "4.2"}}},
                "f44.1": {"base": "sha256:b1", "packages": {"cpp": {"version": "4.3"}, "rust": {"version": "1.98"}}}
            }
        }))
        .expect("an index");
        let new = Index::merge(
            Some(old.clone()),
            "f44.1",
            "sha256:b2",
            &[pushed("rust", "1.99.1"), pushed("not a package", "x")],
        );
        assert_eq!(new.abis["f44.0"], old.abis["f44.0"]);
        assert_eq!(new.abis["f44.1"].base, "sha256:b2");
        assert_eq!(new.get("f44.1", "cpp").map(|p| p.version.as_str()), Some("4.3"));
        assert_eq!(new.get("f44.1", "rust").map(|p| p.version.as_str()), Some("1.99.1"));
        assert!(new.get("f44.1", "not a package").is_none());
        // CI's result files read as they're written (the id beside the rest).
        let r: Pushed = serde_json::from_str(
            r#"{"id":"rpm-krita","digest":"sha256:d","imageId":"sha256:i","version":"6.0.4.1-1.fc44","size":1,"needs":[],"content":"c","built":"2026-11-04T00:00:00Z"}"#,
        )
        .expect("a result");
        let fresh = Index::merge(None, "f44.1", "sha256:b", &[r]);
        let json = serde_json::to_value(&fresh).expect("serializes");
        assert_eq!(json["version"], 1);
        assert_eq!(json["registry"], REGISTRY);
        assert_eq!(json["abis"]["f44.1"]["packages"]["rpm-krita"]["imageId"], "sha256:i");
        assert!(json["abis"]["f44.1"]["packages"]["rpm-krita"].get("id").is_none());
        assert!(fresh.get("f44.2", "rpm-krita").is_none());
        // Another version: started again.
        let other = Index { version: 2, ..Index::default() };
        assert_eq!(Index::merge(Some(other), "a", "b", &[]).version, VERSION);
    }
}
