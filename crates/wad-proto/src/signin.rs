//! Sign-ins in the browser. wadd is root and can't open a window in the
//! person's session, so it says what to open and the shell drives WadBrowser:
//!
//! - [`crate::Event::SignIn`] `{provider, url, code?}` → the shell runs
//!   `wadbrowser --signin <provider> --url <url> [--code <code>]`. It's the
//!   present (a shell that starts meanwhile opens it) until
//! - [`crate::Event::SignInDone`] `{provider}` → `wadbrowser --signin
//!   <provider> --done` (the window closes).
//!
//! The job doing it (Github, Tailnet) waits on a held stage labelled
//! [`SignInProvider::held_label`] meanwhile. Who emits them: wadd's GitHub
//! sign-in (`POST /v1/github/signin`) and its tailnet login (`POST
//! /v1/tailnet/login`), both on **the socket**'s event stream.
//!
//! And sign-ins kept: [`WebKey`], a saved sign-in for sites as the
//! machine's own WadBrowser uses it (`GET /v1/keys/web`).

use std::fmt;

use serde::{Deserialize, Serialize};

/// What a sign-in window signs in to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignInProvider {
    Github,
    Tailscale,
}

impl SignInProvider {
    /// As `wadbrowser --signin` takes it ('github').
    pub fn as_str(self) -> &'static str {
        match self {
            SignInProvider::Github => "github",
            SignInProvider::Tailscale => "tailscale",
        }
    }

    /// As the person reads it: GitHub's sign-in is WadSpaces' (its only
    /// way in), so 'WadSpaces'.
    pub fn name(self) -> &'static str {
        match self {
            SignInProvider::Github => "WadSpaces",
            SignInProvider::Tailscale => "Tailscale",
        }
    }

    /// The label of the job's stage that waits for the sign-in.
    pub fn held_label(self) -> String {
        format!("Signing in to {} in the browser", self.name())
    }
}

impl fmt::Display for SignInProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A web key (a saved sign-in for sites) as the machine's own WadBrowser
/// uses it: `GET /v1/keys/web` on **the socket** (wadd's `keys.rs`). A tab
/// on one of its sites moves into the key's profile, whose data is the key's
/// folder (its cookies there, its cache the browser's own), as a
/// wadspace's WadBrowser does with the folder it mounts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebKey {
    pub id: String,
    pub name: String,
    /// The sites it signs in to (wad_model's `Secret::sites`).
    #[serde(default)]
    pub sites: Vec<String>,
    /// The wadspaces that use it.
    #[serde(default)]
    pub used_by: Vec<String>,
    /// Its folder on the machine (`<state>/keys/<id>`, made if it was new):
    /// uid 1000's, which reaches it by name.
    pub dir: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_read_as_wadbrowser_takes_them() {
        for p in [SignInProvider::Github, SignInProvider::Tailscale] {
            assert_eq!(serde_json::to_value(p).unwrap(), p.as_str());
        }
        assert_eq!(SignInProvider::Github.held_label(), "Signing in to WadSpaces in the browser");
    }

    #[test]
    fn a_web_key_and_its_folder() {
        let k: WebKey = serde_json::from_value(serde_json::json!({
            "id": "hunggod", "name": "hunggod", "sites": ["github.com"], "dir": "/var/lib/wados/keys/hunggod"
        }))
        .unwrap();
        assert!(k.used_by.is_empty());
        assert_eq!(serde_json::to_value(&k).unwrap()["usedBy"], serde_json::json!([]));
        assert_eq!(serde_json::from_value::<WebKey>(serde_json::to_value(&k).unwrap()).unwrap(), k);
    }
}
