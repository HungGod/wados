//! Sign-in windows: a provider's sign-in, opened in WadBrowser by WadOS (wadd
//! and the shell) while it waits for you to finish it there, then closed by
//! WadOS when it's done (the spike's `sim/auth.ts`).
//!
//! # The contract (phase 6: wadd and the shell call these)
//!
//! ```text
//! wadbrowser --signin <provider> --url <url> [--code <code>]
//!     Opens <provider>'s sign-in window at <url>, or, if it's open already,
//!     loads <url> in it (with the new code) and brings it up. Exits 0 once
//!     the running browser has it.
//! wadbrowser --signin <provider> --show
//!     Brings the sign-in window up; if it was closed, opens it again where
//!     it last was. Exits 1 (and says so) when there's no sign-in to show:
//!     none was opened since the last --done.
//! wadbrowser --signin <provider> --done
//!     Closes it (signed in, or given up on) and forgets it. Exits 0, also
//!     when it wasn't open.
//! ```
//!
//! Providers: `github` ("Sign in to GitHub") and `tailscale` ("Sign in to
//! Tailscale"). A sign-in window is like a web app's: no address bar, the
//! sign-in's title, its own window (one per provider), in the default
//! profile, so what it signs in to stays signed in in the browser. It isn't
//! part of the saved session. Where it is (its address and code) is kept in
//! `<data dir>/signins/<provider>.json` until `--done`, for `--show`.
//!
//! - **GitHub** (the OAuth device flow): wadd asks GitHub for a device code
//!   (`POST /login/device/code`), opens `--url <verification_uri>` (GitHub's
//!   `https://github.com/login/device`) with `--code <user_code>`, polls for
//!   the token, and runs `--done` when it has it (or gives up). A user script
//!   in the window types the code in on the device page ([`device_code_script`]);
//!   it never submits the form: you press Continue.
//! - **Tailscale**: `tailscale up` prints an AuthURL; wadd opens it with
//!   `--url <AuthURL>` and runs `--done` once the backend state is Running.
//!
//! The shell's "Show the browser" (`InBrowser.svelte`) is `--show`.

use crate::AppPaths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Who a sign-in window signs in to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Github,
    Tailscale,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Github, Provider::Tailscale];

    pub fn parse(s: &str) -> Option<Provider> {
        Provider::ALL.into_iter().find(|p| p.id() == s)
    }

    /// Its id on the command line (`--signin github`).
    pub fn id(self) -> &'static str {
        match self {
            Provider::Github => "github",
            Provider::Tailscale => "tailscale",
        }
    }

    /// Its name, as people know it.
    pub fn name(self) -> &'static str {
        match self {
            Provider::Github => "GitHub",
            Provider::Tailscale => "Tailscale",
        }
    }

    /// Its window's title.
    pub fn title(self) -> String {
        format!("Sign in to {}", self.name())
    }
}

/// What a `--signin` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Open it (or load this in it) at `url`, with GitHub's device code.
    Open { url: String, code: Option<String> },
    /// Bring it up (open it again where it was, if it was closed).
    Show,
    /// Close it, and forget it.
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub provider: Provider,
    pub action: Action,
}

/// Where a sign-in window is, kept for `--show` until `--done`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

fn saved_path(paths: &AppPaths, p: Provider) -> PathBuf {
    paths.data_dir.join("signins").join(format!("{}.json", p.id()))
}

impl Saved {
    pub fn load(paths: &AppPaths, p: Provider) -> Option<Saved> {
        let text = std::fs::read_to_string(saved_path(paths, p)).ok()?;
        serde_json::from_str::<Saved>(&text).ok().filter(|s| !s.url.is_empty())
    }

    pub fn store(&self, paths: &AppPaths, p: Provider) -> std::io::Result<()> {
        let path = saved_path(paths, p);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, path)
    }

    pub fn forget(paths: &AppPaths, p: Provider) {
        let _ = std::fs::remove_file(saved_path(paths, p));
    }
}

/// The user script that types GitHub's device code into its device page
/// (`<host>/login/device`), as one field (`user-code`) or one field per
/// character (`user-code-0`, `user-code-1`, …), with `input` and `change`
/// events as typing would. Sign-in pages come first, and the page may draw
/// its form late, so it tries again as the page changes until the code is
/// in, then stops. It never submits. `host` is the device page's (with its
/// port, if it has one): GitHub's, or a test's.
pub fn device_code_script(host: &str, code: &str) -> String {
    let lit = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
    format!("(() => {{\n  const HOST = {}, CODE = {};\n{}}})();\n", lit(&host.to_ascii_lowercase()), lit(code), FILLER)
}

/// The body of [`device_code_script`] (HOST and CODE are defined before it).
const FILLER: &str = r#"  'use strict';
  if (location.host.toLowerCase() !== HOST || !/^\/login\/device\/?$/.test(location.pathname)) return;
  if (window.__wadspacesDeviceCode === CODE) return;
  window.__wadspacesDeviceCode = CODE;
  const chars = Array.from(CODE.replace(/[^0-9A-Za-z]/g, ''));
  const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
  const put = (el, v) => {
    if (el.value === v) return;
    // The prototype's setter, so a page's own value tracking sees it as typed.
    setValue.call(el, v);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  };
  const usable = (el) => el.type !== 'hidden' && !el.disabled && !el.readOnly;
  const fill = () => {
    const split = new Map();
    for (const el of document.querySelectorAll('input[name^="user-code-"], input[id^="user-code-"]')) {
      const m = /^user-code-(\d+)$/.exec(el.name) || /^user-code-(\d+)$/.exec(el.id);
      if (m && usable(el) && !split.has(+m[1])) split.set(+m[1], el);
    }
    if (split.size) {
      const fields = [...split.entries()].sort((a, b) => a[0] - b[0]).map((e) => e[1]);
      if (fields.length < chars.length) return false;
      fields.slice(0, chars.length).forEach((el, i) => put(el, chars[i]));
      return true;
    }
    const one = document.querySelector('input[name="user-code"], input#user-code');
    if (one && usable(one)) {
      put(one, CODE);
      return true;
    }
    return false;
  };
  const filled = () => document.documentElement.setAttribute('data-wadspaces-device-code', 'filled');
  if (fill()) return filled();
  const watch = new MutationObserver(() => {
    if (fill()) {
      watch.disconnect();
      filled();
    }
  });
  watch.observe(document.documentElement, { childList: true, subtree: true, attributes: true });
  setTimeout(() => watch.disconnect(), 15 * 60 * 1000);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers() {
        assert_eq!(Provider::parse("github"), Some(Provider::Github));
        assert_eq!(Provider::parse("tailscale"), Some(Provider::Tailscale));
        assert_eq!(Provider::parse("steam"), None);
        assert_eq!(Provider::parse("GitHub"), None);
        assert_eq!(Provider::Github.title(), "Sign in to GitHub");
        assert_eq!(Provider::Tailscale.title(), "Sign in to Tailscale");
    }

    #[test]
    fn the_script_carries_its_code_and_host_safely() {
        let s = device_code_script("GitHub.com", "WDJB-MJHT");
        assert!(s.contains(r#"const HOST = "github.com", CODE = "WDJB-MJHT";"#), "{s}");
        // Whatever the code is, it stays a string literal.
        let s = device_code_script("github.com", "a\"; alert(1); \"");
        assert!(s.contains(r#"CODE = "a\"; alert(1); \"";"#), "{s}");
        // It never submits anything.
        assert!(!s.contains("submit"));
        assert!(!s.contains(".click("));
    }

    #[test]
    fn saved_signins_come_back_until_forgotten() {
        let dir = std::env::temp_dir().join(format!("wadbrowser-signin-test-{}", std::process::id()));
        let paths = AppPaths { config_dir: dir.join("c"), data_dir: dir.join("d"), cache_dir: dir.join("k") };
        assert_eq!(Saved::load(&paths, Provider::Github), None);
        let s = Saved { url: "https://github.com/login/device".into(), code: Some("WDJB-MJHT".into()) };
        s.store(&paths, Provider::Github).unwrap();
        assert_eq!(Saved::load(&paths, Provider::Github), Some(s));
        assert_eq!(Saved::load(&paths, Provider::Tailscale), None);
        Saved::forget(&paths, Provider::Github);
        assert_eq!(Saved::load(&paths, Provider::Github), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
