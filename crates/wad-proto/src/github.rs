//! GitHub on this machine: whose token wadd has (the podman secret
//! `github_token`), and the repositories it can clone. The token itself is
//! never in these types.
//!
//! Who serves what (phase 6; wadd's GitHub sign-in): **the socket** only.
//! `GET /v1/github` → `Option<`[`GithubAccount`]`>` (null: signed out; also
//! the `github` event); `POST /v1/github/signin` ([`SignInRequest`]) → a
//! Github job (the device flow, in the browser: the `signIn` event);
//! `DELETE /v1/github` signs out (null); `GET /v1/github/repos` →
//! `Vec<`[`Repo`]`>`, cached for 60 s.

use serde::{Deserialize, Serialize};

/// What the first sign-in asks for. `write:packages` comes from a second,
/// short sign-in the first time a push needs it.
pub const FIRST_SCOPES: [&str; 3] = ["repo", "workflow", "read:org"];

/// The GitHub account wadd has a token for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubAccount {
    pub login: String,
    /// Its name (its login when it hasn't set one).
    pub name: String,
    /// What the token may do (GitHub's `X-OAuth-Scopes`: 'repo', 'workflow'…).
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl GithubAccount {
    /// The token may do `scope`.
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }
}

/// A repository the account can use (v1's shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    /// 'owner/name': what a project's `repo` is.
    pub full_name: String,
    pub name: String,
    pub private: bool,
    /// Its https clone URL.
    pub url: String,
    pub default_branch: String,
    /// Last pushed to (ms since the epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pushed_at: Option<i64>,
    #[serde(default)]
    pub description: String,
}

/// `POST /v1/github/signin`: what to ask for. None: [`FIRST_SCOPES`]; else
/// these, with the ones the token has already.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInRequest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_account_its_repos_and_a_sign_in() {
        let a: GithubAccount =
            serde_json::from_value(json!({"login": "hunggod", "name": "Hung", "scopes": ["repo", "workflow"]}))
                .unwrap();
        assert!(a.has_scope("workflow") && !a.has_scope("write:packages"));
        let r: Repo = serde_json::from_value(json!({
            "fullName": "HungGod/WadSpaces", "name": "WadSpaces", "private": true,
            "url": "https://github.com/HungGod/WadSpaces.git", "defaultBranch": "main", "pushedAt": 1793140200000_i64,
        }))
        .unwrap();
        assert_eq!((r.full_name.as_str(), r.pushed_at), ("HungGod/WadSpaces", Some(1_793_140_200_000)));
        assert_eq!(serde_json::from_value::<Repo>(serde_json::to_value(&r).unwrap()).unwrap(), r);
        // An empty body is the first sign-in.
        assert_eq!(serde_json::from_value::<SignInRequest>(json!({})).unwrap(), SignInRequest::default());
        assert_eq!(serde_json::to_value(SignInRequest::default()).unwrap(), json!({}));
    }
}
