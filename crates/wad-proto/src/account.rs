//! The WadSpaces account: the machine's link to its owner's account
//! (Firebase), and the projects' secret files it keeps. No sign-in (an ID,
//! refresh or GitHub token) and no secret file's contents are ever in these
//! types, except a file's going *in* ([`SecretFileKeep`]).
//!
//! Who serves what:
//!
//! | Route | Socket | Remote (the owner's phone, my machines) |
//! |---|---|---|
//! | `GET /v1/account` → [`AccountLink`] (also the `account` event) | ✓ | |
//! | `POST /v1/account/link` → [`AccountLink`]: links with the GitHub sign-in | ✓ | |
//! | `DELETE /v1/account` → [`AccountLink`]: unlinked | ✓ | |
//! | `GET /v1/secret-files` → [`SecretFiles`] (also the `secretFiles` event) | ✓ | `/v1/remote/secret-files` |
//! | `POST /v1/secret-files` ([`SecretFileKeep`]) → [`SecretFile`] | ✓ | `/v1/remote/secret-files` |
//! | `DELETE /v1/secret-files/{name}` → 204 | ✓ | `/v1/remote/secret-files/{name}` |
//! | `POST /v1/secret-files/asks/{id}` ([`SecretFileAnswer`]) → [`SecretFiles`] | ✓ | `/v1/remote/secret-files/asks/{id}` |

use serde::{Deserialize, Serialize};

/// How the machine is linked to an account.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountState {
    /// This WadOS has no account to link to (no Firebase configuration).
    #[default]
    Off,
    /// Not linked: signing in to GitHub links it.
    Unlinked,
    /// Linking now.
    Linking,
    /// Linked: it heartbeats, and its secret files are the account's.
    Linked,
}

/// `GET /v1/account`: the machine's link (never its sign-in).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountLink {
    pub state: AccountState,
    /// The account's uid (Firebase's), once linked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    /// The GitHub login it was linked as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    /// This machine's document in the account (`users/<uid>/machines/<id>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<String>,
    /// When it was linked (ms since the epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_at: Option<i64>,
    /// When the account last heard from it (ms since the epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<i64>,
    /// What went wrong last (linking, a heartbeat, a sync), in words to act on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Where a secret file is kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretOrigin {
    /// In the account: every machine of it has it.
    #[default]
    Account,
    /// On this machine only (no account yet): it goes to the account once linked.
    Local,
}

/// A project's file kept out of git (`.env`, a key, a local config): put in
/// every clone of its GitHub repository, on every machine of the account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretFile {
    /// `f-<12 hex>`: of its repo and path.
    pub name: String,
    /// `owner/name`.
    pub repo: String,
    /// Where in the project (`.env`, `config/secrets.toml`).
    pub path: String,
    /// Its size in bytes.
    pub size: u64,
    /// When it last changed (ms since the epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// The wadspaces here with a project of its repo.
    #[serde(default)]
    pub used_by: Vec<String>,
    pub origin: SecretOrigin,
}

/// What a secret file's ask is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretFileAskKind {
    /// A likely secret file in a clone (ignored by git, named like one) that
    /// isn't kept: keep it for every machine?
    New,
    /// A kept file changed in a clone: keep that version for every machine?
    Changed,
}

/// A question about a file in one of this machine's clones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretFileAsk {
    /// `a-<12 hex>`.
    pub id: String,
    pub kind: SecretFileAskKind,
    pub wadspace: String,
    pub wadspace_name: String,
    /// The project's id, and its name (its folder, `~/Desktop/<name>`).
    pub project: String,
    pub project_name: String,
    pub repo: String,
    pub path: String,
    pub size: u64,
}

/// `GET /v1/secret-files`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretFiles {
    pub files: Vec<SecretFile>,
    pub asks: Vec<SecretFileAsk>,
    /// They're the account's (the machine is linked); false: this machine's only.
    pub account: bool,
}

/// `POST /v1/secret-files`: a file to keep (new, or new contents).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretFileKeep {
    pub repo: String,
    pub path: String,
    /// The file's bytes, in base64 (at most 64 KiB of them).
    pub value: String,
}

/// `POST /v1/secret-files/asks/{id}`: keep it (read from the clone), or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretFileAnswer {
    pub keep: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_the_wire() {
        let f = SecretFiles {
            files: vec![SecretFile {
                name: "f-0123456789ab".into(),
                repo: "HungGod/Writing".into(),
                path: ".env".into(),
                size: 12,
                updated_at: Some(1),
                used_by: vec!["writing".into()],
                origin: SecretOrigin::Account,
            }],
            asks: vec![],
            account: true,
        };
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["files"][0]["updatedAt"], 1);
        assert_eq!(v["files"][0]["usedBy"][0], "writing");
        assert_eq!(v["files"][0]["origin"], "account");
        let l = AccountLink { state: AccountState::Linked, machine_id: Some("m1".into()), ..Default::default() };
        let v = serde_json::to_value(&l).unwrap();
        assert_eq!(v, serde_json::json!({"state": "linked", "machineId": "m1"}));
        assert_eq!(serde_json::to_value(SecretFileAskKind::Changed).unwrap(), "changed");
    }
}
