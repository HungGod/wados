//! The sign-in driver. wadd is root and can't open a window in the person's
//! session, so it says what to open (`signIn {provider, url, code?}`) and
//! the shell has WadBrowser do it (wadbrowser-core's `signin` contract):
//!
//! - `signIn` → `wadbrowser --signin <provider> --url <url> [--code <code>]`,
//!   each time it comes: the same sign-in asked for again (or the present,
//!   said again on a reconnect to wadd) opens the window again, or brings it up;
//! - `signInDone` → `wadbrowser --signin <provider> --done`, when this shell
//!   opened one;
//! - "Show the browser" (a job's held stage, in the Activity flyout) → `--show`.
//!
//! WADOS_WADBROWSER names another program to run (tests).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::{gio, glib};
use wad_proto::{Level, SignInProvider};

use crate::shell::Shell;
use crate::store::{Change, SignIn};

/// What to ask of the sign-in window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Open { url: String, code: Option<String> },
    Show,
    Done,
}

/// The browser to drive.
pub fn program() -> String {
    std::env::var("WADOS_WADBROWSER").ok().filter(|p| !p.is_empty()).unwrap_or_else(|| "wadbrowser".into())
}

/// What runs for `action` on `provider`'s sign-in window.
pub fn argv(program: &str, provider: SignInProvider, action: &Action) -> Vec<String> {
    let mut v = vec![program.to_string(), "--signin".into(), provider.as_str().into()];
    match action {
        Action::Open { url, code } => {
            v.extend(["--url".into(), url.clone()]);
            if let Some(c) = code {
                v.extend(["--code".into(), c.clone()]);
            }
        }
        Action::Show => v.push("--show".into()),
        Action::Done => v.push("--done".into()),
    }
    v
}

/// What to run when wadd says the sign-in for a provider is now `now`, the
/// last this shell opened being `opened` (none: it opened none, or it's done).
pub fn next(opened: Option<&SignIn>, now: Option<&SignIn>) -> Option<Action> {
    match (opened, now) {
        (_, Some(s)) => Some(Action::Open { url: s.url.clone(), code: s.code.clone() }),
        (Some(_), None) => Some(Action::Done),
        (None, None) => None,
    }
}

thread_local! {
    /// What this shell opened, by provider.
    static OPENED: RefCell<HashMap<SignInProvider, SignIn>> = RefCell::default();
}

/// Follows wadd's sign-ins for the shell's session.
pub fn start(shell: &Rc<Shell>) {
    let s = Rc::downgrade(shell);
    shell.store.watch(move |c| {
        let (Some(shell), Change::SignIn(p)) = (s.upgrade(), c) else { return };
        let now = shell.store.sign_in(*p);
        let act = OPENED.with(|o| next(o.borrow().get(p), now.as_ref()));
        let Some(act) = act else { return };
        OPENED.with(|o| match &now {
            Some(n) => {
                o.borrow_mut().insert(*p, n.clone());
            }
            None => {
                o.borrow_mut().remove(p);
            }
        });
        run(&shell, *p, act);
    });
}

/// Show the browser: the sign-in window brought up again.
pub fn show(shell: &Rc<Shell>, provider: SignInProvider) {
    run(shell, provider, Action::Show);
}

fn run(shell: &Rc<Shell>, provider: SignInProvider, act: Action) {
    let argv = argv(&program(), provider, &act);
    tracing::info!("sign-in: {}", argv.join(" "));
    if glib::find_program_in_path(&argv[0]).is_none() {
        if act != Action::Done {
            shell.toast(
                Level::Warn,
                &format!("WadBrowser isn't on this machine, so signing in to {} can't open", provider.name()),
                None,
            );
        }
        return;
    }
    let args: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    let child = match gio::Subprocess::newv(&args, gio::SubprocessFlags::NONE) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("{}: {e}", argv[0]);
            return;
        }
    };
    let s = shell.clone();
    glib::spawn_future_local(async move {
        let ok = child.wait_check_future().await.is_ok();
        if !ok && act == Action::Show {
            s.toast(Level::Info, &format!("There's no sign-in to {} to show", provider.name()), None);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(url: &str, code: Option<&str>) -> SignIn {
        SignIn { url: url.into(), code: code.map(String::from) }
    }

    #[test]
    fn the_browser_is_asked_as_its_contract_says() {
        let gh = SignInProvider::Github;
        let open = Action::Open { url: "https://github.com/login/device".into(), code: Some("WADS-0601".into()) };
        assert_eq!(
            argv("wadbrowser", gh, &open),
            ["wadbrowser", "--signin", "github", "--url", "https://github.com/login/device", "--code", "WADS-0601"]
        );
        let ts = SignInProvider::Tailscale;
        let open = Action::Open { url: "https://login.tailscale.com/a/x".into(), code: None };
        assert_eq!(
            argv("wadbrowser", ts, &open),
            ["wadbrowser", "--signin", "tailscale", "--url", "https://login.tailscale.com/a/x"]
        );
        assert_eq!(argv("wb", ts, &Action::Show), ["wb", "--signin", "tailscale", "--show"]);
        assert_eq!(argv("wb", gh, &Action::Done), ["wb", "--signin", "github", "--done"]);
    }

    #[test]
    fn it_opens_each_time_its_asked_and_closes_only_what_it_opened() {
        let a = s("https://github.com/login/device", Some("AAAA-0001"));
        // New: open it.
        assert_eq!(next(None, Some(&a)), Some(Action::Open { url: a.url.clone(), code: a.code.clone() }));
        // The same again (asked for again, or wadd reconnected): open it again, or bring it up.
        assert_eq!(next(Some(&a), Some(&a)), Some(Action::Open { url: a.url.clone(), code: a.code.clone() }));
        // A new code (GitHub's expired): load it.
        let b = s("https://github.com/login/device", Some("BBBB-0002"));
        assert!(matches!(next(Some(&a), Some(&b)), Some(Action::Open { code: Some(c), .. }) if c == "BBBB-0002"));
        // Done: close it.
        assert_eq!(next(Some(&b), None), Some(Action::Done));
        // Done when this shell opened none (a reconnect's replay): nothing.
        assert_eq!(next(None, None), None);
    }
}
