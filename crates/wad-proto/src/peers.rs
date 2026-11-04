//! Peers: my other machines on the tailnet (wadds of the same login), and
//! their wadspaces.
//!
//! Who serves what (phase 6; wadd's `peers.rs`): **the socket** only.
//! `GET /v1/peers` → `Vec<`[`PeerInfo`]`>` (also the `peers` event);
//! `GET /v1/peers/{id}/wadspaces` → its live documents
//! (`Vec<wad_model::Wadspace>`, from its remote API: online only, else
//! `offline`); `POST /v1/peers/{id}/wadspaces/{ws}/copy` → a CopyWadspace
//! job, which fetches the document and applies `space.import` here. What a
//! peer answers on its remote API is in [`crate::tailnet`].

use serde::{Deserialize, Serialize};
use wad_model::Peer;

/// One of my machines (`GET /v1/peers`): wad_model's [`Peer`] (its wadd
/// machine id, name, hostname, kind, online, last seen, OS, GPU; `trusted`
/// is always true), and where it is on the tailnet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfo {
    #[serde(flatten)]
    pub peer: Peer,
    /// Its node's stable id (the tailnet's devices and the trust book know it by this).
    pub node_id: String,
    /// Its MagicDNS name ('desk.tail1234.ts.net'): its remote API is
    /// `http://<dnsName>:8087`, its home over Taildrive
    /// `dav://100.100.100.100:8080/<tailnet>/<hostname>/home`.
    pub dns_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_model::PeerKind;

    #[test]
    fn a_peer_is_the_models_flat_with_where_it_is() {
        let p = PeerInfo {
            peer: Peer {
                id: "m-desk".into(),
                name: "Desk".into(),
                hostname: "desk".into(),
                kind: PeerKind::Desktop,
                trusted: true,
                online: true,
                last_seen: 9,
                os: "WadOS 44".into(),
                gpu: None,
            },
            node_id: "nDesk1".into(),
            dns_name: "desk.tail1234.ts.net".into(),
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!((v["id"].as_str(), v["kind"].as_str()), (Some("m-desk"), Some("desktop")));
        assert_eq!((v["nodeId"].as_str(), v["lastSeen"].as_i64()), (Some("nDesk1"), Some(9)));
        assert_eq!(serde_json::from_value::<PeerInfo>(v).unwrap(), p);
    }
}
