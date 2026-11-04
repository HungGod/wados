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

    /// As the person reads it ('GitHub').
    pub fn name(self) -> &'static str {
        match self {
            SignInProvider::Github => "GitHub",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_read_as_wadbrowser_takes_them() {
        for p in [SignInProvider::Github, SignInProvider::Tailscale] {
            assert_eq!(serde_json::to_value(p).unwrap(), p.as_str());
        }
        assert_eq!(SignInProvider::Github.held_label(), "Signing in to GitHub in the browser");
    }
}
