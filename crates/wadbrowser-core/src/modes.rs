//! WadBrowser's kinds of window, and which window a launch opens in. Pure:
//! the UI draws a [`WindowSpec`] and carries out a [`Plan`].
//!
//! - **Full**: tabs and an address bar.
//! - **Focus**: tabs, no address bar; pages arrive by links (`--no-urlbar`,
//!   or `default = focus` for `--default`).
//! - **App**: one web app in its own window (`--app`): its name and picture,
//!   no address bar, its own Wayland app_id (`wadspaces-webapp-<id>`), and
//!   "Offsite · host" with "Back to <name>" once you leave its site
//!   ([`crate::site::offsite`]). A tab strip only once there's a second tab.
//! - **Sign-in** windows are App windows of a provider's ([`crate::signin`]).

use crate::cli::Request;
use crate::signin::{self, Provider};
use crate::{APP_ID, NEW_TAB_URI, site};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    #[default]
    Full,
    Focus,
    App,
}

/// A web app's (or a sign-in's) window: who it is and where it starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppSpec {
    /// Its Wayland app_id.
    pub app_id: String,
    pub name: String,
    /// Its start page: Home, and the site that isn't offsite.
    pub start: String,
    /// A picture for its header (absolute path).
    pub icon: Option<String>,
}

/// A sign-in window's provider, and GitHub's device code for its page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignInSpec {
    pub provider: Provider,
    pub code: Option<String>,
}

/// What a window is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowSpec {
    pub mode: Mode,
    pub private: bool,
    /// Its browser profile (None: the default one; the engine's).
    pub profile: Option<String>,
    /// App windows'.
    pub app: Option<AppSpec>,
    pub signin: Option<SignInSpec>,
}

impl WindowSpec {
    /// A Full or Focus window.
    pub fn browser(mode: Mode, private: bool, profile: Option<String>) -> WindowSpec {
        WindowSpec { mode, private, profile, app: None, signin: None }
    }

    pub fn web_app(app: AppSpec, private: bool, profile: Option<String>) -> WindowSpec {
        WindowSpec { mode: Mode::App, private, profile, app: Some(app), signin: None }
    }

    /// A provider's sign-in window, at `url`: App-like, in the default profile.
    pub fn sign_in(provider: Provider, url: &str, code: Option<String>) -> WindowSpec {
        let app = AppSpec { app_id: APP_ID.into(), name: provider.title(), start: url.into(), icon: None };
        WindowSpec {
            mode: Mode::App,
            private: false,
            profile: None,
            app: Some(app),
            signin: Some(SignInSpec { provider, code }),
        }
    }

    /// Its Wayland app_id: a web app's own, else WadBrowser's (an instance's
    /// windows too, so they go with WadBrowser's launcher).
    pub fn app_id(&self) -> &str {
        match (&self.app, &self.signin) {
            (Some(a), None) => &a.app_id,
            _ => APP_ID,
        }
    }

    /// Whether it has an address bar.
    pub fn has_urlbar(&self) -> bool {
        self.mode == Mode::Full
    }

    /// Whether its tab strip hides while there's one tab.
    pub fn autohide_tabs(&self) -> bool {
        self.mode == Mode::App
    }

    /// Whether it's part of the saved session (a browser window, not private).
    pub fn saved_in_session(&self) -> bool {
        self.app.is_none() && self.signin.is_none() && !self.private
    }

    /// The window's title: an app's name (a sign-in's title), else the page's.
    pub fn title(&self, page_title: Option<&str>) -> String {
        if let Some(a) = &self.app {
            return a.name.clone();
        }
        let app = if self.private { "WadBrowser (Private)" } else { "WadBrowser" };
        match page_title.map(str::trim).filter(|t| !t.is_empty()) {
            Some(t) => format!("{t} – {app}"),
            None => app.into(),
        }
    }

    /// Where Home goes: an app's start page, else `home` (the settings').
    pub fn home<'a>(&'a self, home: &'a str) -> &'a str {
        self.app.as_ref().map_or(home, |a| a.start.as_str())
    }

    /// Whether `url` is offsite: an app window (not a sign-in's) on another
    /// site than its start page's.
    pub fn offsite(&self, url: &str) -> bool {
        match (&self.app, &self.signin) {
            (Some(a), None) => site::offsite(url, &a.start),
            _ => false,
        }
    }

    /// A new window from this one (Ctrl+N): the same kind of browser window;
    /// an app's or a sign-in's opens a Full one.
    pub fn sibling(&self, private: bool) -> WindowSpec {
        let mode = if self.mode == Mode::Focus { Mode::Focus } else { Mode::Full };
        WindowSpec::browser(mode, private, if private { None } else { self.profile.clone() })
    }
}

/// An open window, as [`plan`] sees it.
#[derive(Clone, Debug)]
pub struct Open<'a> {
    pub id: u64,
    pub spec: &'a WindowSpec,
}

/// What [`plan`] knows besides the request.
pub struct Context<'a> {
    /// The open windows, the most recently active first.
    pub open: Vec<Open<'a>>,
    /// What `--default` opens (wadbrowser.conf).
    pub default_mode: Mode,
    /// This is the browser's first request (it's just started).
    pub first: bool,
    /// Where a closed sign-in window was (for `--show`).
    pub saved_signin: &'a dyn Fn(Provider) -> Option<signin::Saved>,
}

/// What to do for a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// New tabs at `urls` in window `window` (none: just bring it up).
    Tabs { window: u64, urls: Vec<String> },
    /// A new window; `urls` its tabs (none: its home, or a new tab).
    New { spec: WindowSpec, urls: Vec<String> },
    /// The browser's first window: the last session's tabs (or a new tab).
    Restore { spec: WindowSpec },
    /// A sign-in window again: `url` loaded in it, with `code`, brought up.
    SignIn { window: u64, url: String, code: Option<String> },
    /// Close these windows (a sign-in done; none may be open).
    Close { windows: Vec<u64> },
    /// Nothing to do, and why (the launch exits 1 and says so).
    Nothing { why: String },
}

/// Which window `req` opens in.
pub fn plan(req: &Request, cx: &Context) -> Plan {
    let urls = req.pages();

    if let Some(s) = &req.signin {
        let p = s.provider;
        let open: Vec<u64> =
            cx.open.iter().filter(|o| o.spec.signin.as_ref().is_some_and(|x| x.provider == p)).map(|o| o.id).collect();
        return match &s.action {
            signin::Action::Done => Plan::Close { windows: open },
            signin::Action::Show => match (open.first(), (cx.saved_signin)(p)) {
                (Some(&w), _) => Plan::Tabs { window: w, urls: vec![] },
                (None, Some(saved)) => {
                    Plan::New { spec: WindowSpec::sign_in(p, &saved.url, saved.code), urls: vec![saved.url] }
                }
                (None, None) => Plan::Nothing { why: format!("There's no {} sign-in to show", p.name()) },
            },
            signin::Action::Open { url, code } => match open.first() {
                Some(&w) => Plan::SignIn { window: w, url: url.clone(), code: code.clone() },
                None => Plan::New { spec: WindowSpec::sign_in(p, url, code.clone()), urls: vec![url.clone()] },
            },
        };
    }

    if let Some(app_id) = &req.app_id {
        if !req.new_window
            && let Some(o) = cx
                .open
                .iter()
                .find(|o| o.spec.signin.is_none() && o.spec.app.as_ref().is_some_and(|a| &a.app_id == app_id))
        {
            return Plan::Tabs { window: o.id, urls };
        }
        let start = req.start.clone().or_else(|| urls.first().cloned()).unwrap_or_else(|| NEW_TAB_URI.into());
        let name =
            req.name.clone().unwrap_or_else(|| app_id.strip_prefix(crate::cli::WEBAPP_PREFIX).unwrap_or(app_id).into());
        let app = AppSpec { app_id: app_id.clone(), name, start, icon: req.icon.clone() };
        return Plan::New { spec: WindowSpec::web_app(app, req.private, req.profile.clone()), urls };
    }

    let mode = if req.default {
        cx.default_mode
    } else if req.no_urlbar {
        Mode::Focus
    } else {
        Mode::Full
    };
    let spec = WindowSpec::browser(mode, req.private, if req.private { None } else { req.profile.clone() });
    if !urls.is_empty()
        && !req.new_window
        && let Some(o) = cx.open.iter().find(|o| {
            o.spec.app.is_none()
                && o.spec.mode == mode
                && o.spec.private == spec.private
                && o.spec.profile == spec.profile
        })
    {
        return Plan::Tabs { window: o.id, urls };
    }
    if cx.first && urls.is_empty() && !req.private && spec.profile.is_none() {
        return Plan::Restore { spec };
    }
    Plan::New { spec, urls }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli;
    use std::path::Path;

    fn req(args: &[&str]) -> Request {
        cli::parse(args.iter().copied(), Path::new("/")).unwrap()
    }

    fn none(_: Provider) -> Option<signin::Saved> {
        None
    }

    fn cx<'a>(open: &'a [(u64, WindowSpec)]) -> Context<'a> {
        Context {
            open: open.iter().map(|(id, spec)| Open { id: *id, spec }).collect(),
            default_mode: Mode::Full,
            first: false,
            saved_signin: &none,
        }
    }

    fn full() -> WindowSpec {
        WindowSpec::browser(Mode::Full, false, None)
    }

    fn claude() -> WindowSpec {
        let app = AppSpec {
            app_id: "wadspaces-webapp-claude".into(),
            name: "Claude".into(),
            start: "https://claude.ai".into(),
            icon: None,
        };
        WindowSpec::web_app(app, false, None)
    }

    #[test]
    fn a_first_plain_launch_restores_the_session() {
        let mut c = cx(&[]);
        c.first = true;
        assert_eq!(plan(&req(&[]), &c), Plan::Restore { spec: full() });
        // Not with pages, privately, or once it's running.
        assert_eq!(plan(&req(&["a.example"]), &c), Plan::New { spec: full(), urls: vec!["a.example".into()] });
        assert!(matches!(plan(&req(&["--private"]), &c), Plan::New { spec, .. } if spec.private));
        c.first = false;
        assert_eq!(plan(&req(&[]), &c), Plan::New { spec: full(), urls: vec![] });
    }

    #[test]
    fn a_link_goes_to_the_last_active_window_that_fits() {
        let private = WindowSpec::browser(Mode::Full, true, None);
        let focus = WindowSpec::browser(Mode::Focus, false, None);
        let open = [(7, private), (3, claude()), (5, focus), (2, full()), (1, full())];
        // Most recent first: 2 is the last active Full window (7 is private, 3 an app's, 5 Focus).
        assert_eq!(plan(&req(&["b.example"]), &cx(&open)), Plan::Tabs { window: 2, urls: vec!["b.example".into()] });
        assert_eq!(
            plan(&req(&["--no-urlbar", "b.example"]), &cx(&open)),
            Plan::Tabs { window: 5, urls: vec!["b.example".into()] }
        );
        assert_eq!(
            plan(&req(&["--private", "b.example"]), &cx(&open)),
            Plan::Tabs { window: 7, urls: vec!["b.example".into()] }
        );
        // --new-window: a window of its own.
        assert_eq!(
            plan(&req(&["--new-window", "b.example"]), &cx(&open)),
            Plan::New { spec: full(), urls: vec!["b.example".into()] }
        );
        // No page: a new window, as other browsers do.
        assert_eq!(plan(&req(&[]), &cx(&open)), Plan::New { spec: full(), urls: vec![] });
        // Launchers' unexpanded %U isn't a page.
        assert_eq!(plan(&req(&["%U"]), &cx(&open)), Plan::New { spec: full(), urls: vec![] });
        // None that fits: a new one.
        let only_apps = [(3, claude())];
        assert_eq!(
            plan(&req(&["b.example"]), &cx(&only_apps)),
            Plan::New { spec: full(), urls: vec!["b.example".into()] }
        );
    }

    #[test]
    fn default_is_what_the_conf_says() {
        let mut c = cx(&[]);
        assert_eq!(
            plan(&req(&["--default", "x.example"]), &c),
            Plan::New { spec: full(), urls: vec!["x.example".into()] }
        );
        c.default_mode = Mode::Focus;
        let focus = WindowSpec::browser(Mode::Focus, false, None);
        assert_eq!(
            plan(&req(&["--default", "x.example"]), &c),
            Plan::New { spec: focus, urls: vec!["x.example".into()] }
        );
    }

    #[test]
    fn profiles_keep_to_their_windows() {
        let work = WindowSpec::browser(Mode::Full, false, Some("work".into()));
        let open = [(1, full()), (2, work.clone())];
        assert_eq!(
            plan(&req(&["--profile", "work", "x.example"]), &cx(&open)),
            Plan::Tabs { window: 2, urls: vec!["x.example".into()] }
        );
        assert_eq!(plan(&req(&["x.example"]), &cx(&open)), Plan::Tabs { window: 1, urls: vec!["x.example".into()] });
    }

    #[test]
    fn web_apps_have_a_window_each() {
        let r = req(&["--app", "claude", "--name", "Claude", "--url", "https://claude.ai", "--icon", "/i/c.png"]);
        let Plan::New { spec, urls } = plan(&r, &cx(&[])) else { panic!("a new window") };
        assert_eq!(spec.mode, Mode::App);
        assert_eq!(spec.app_id(), "wadspaces-webapp-claude");
        assert_eq!(spec.title(Some("whatever")), "Claude");
        assert_eq!(spec.app.as_ref().unwrap().icon.as_deref(), Some("/i/c.png"));
        assert!(urls.is_empty());
        assert!(!spec.has_urlbar() && spec.autohide_tabs() && !spec.saved_in_session());
        assert_eq!(spec.home("cactus://newtab"), "https://claude.ai");
        // Open already: its window, with any pages as tabs.
        let open = [(1, full()), (4, claude())];
        assert_eq!(plan(&r, &cx(&open)), Plan::Tabs { window: 4, urls: vec![] });
        let mut with_page = r.clone();
        with_page.urls = vec!["https://claude.ai/new".into()];
        assert_eq!(plan(&with_page, &cx(&open)), Plan::Tabs { window: 4, urls: vec!["https://claude.ai/new".into()] });
        // --new-window: another window of it.
        let mut again = r.clone();
        again.new_window = true;
        assert!(matches!(plan(&again, &cx(&open)), Plan::New { .. }));
        // Without --url or --name: its first page and its id.
        let Plan::New { spec, .. } = plan(&req(&["--app", "maps", "https://maps.example"]), &cx(&[])) else { panic!() };
        let app = spec.app.unwrap();
        assert_eq!((app.name.as_str(), app.start.as_str()), ("maps", "https://maps.example"));
    }

    #[test]
    fn offsite_is_an_apps_and_not_a_sign_ins() {
        let c = claude();
        assert!(!c.offsite("https://claude.ai/chat"));
        assert!(c.offsite("https://github.com/x"));
        assert!(!c.offsite("cactus://newtab"));
        assert!(!full().offsite("https://github.com/x"));
        let s = WindowSpec::sign_in(Provider::Tailscale, "https://login.tailscale.com/a/1", None);
        assert!(!s.offsite("https://accounts.google.com/o/oauth2"));
    }

    #[test]
    fn sign_ins_open_reuse_show_and_close() {
        let gh = WindowSpec::sign_in(Provider::Github, "https://github.com/login/device", Some("A-B".into()));
        assert_eq!(gh.title(None), "Sign in to GitHub");
        assert_eq!(gh.app_id(), APP_ID);
        assert!(!gh.private && gh.profile.is_none() && !gh.has_urlbar() && !gh.saved_in_session());

        let open_it = req(&["--signin", "github", "--url", "https://github.com/login/device", "--code", "WDJB-MJHT"]);
        let Plan::New { spec, urls } = plan(&open_it, &cx(&[])) else { panic!("a new window") };
        assert_eq!(spec.signin, Some(SignInSpec { provider: Provider::Github, code: Some("WDJB-MJHT".into()) }));
        assert_eq!(urls, ["https://github.com/login/device"]);

        // Open already: reused, with the new code.
        let open = [(1, full()), (9, gh.clone())];
        assert_eq!(
            plan(&open_it, &cx(&open)),
            Plan::SignIn { window: 9, url: "https://github.com/login/device".into(), code: Some("WDJB-MJHT".into()) }
        );
        // Tailscale's is another window.
        let ts = req(&["--signin", "tailscale", "--url", "https://login.tailscale.com/a/1"]);
        assert!(matches!(plan(&ts, &cx(&open)), Plan::New { .. }));

        let show = req(&["--signin", "github", "--show"]);
        assert_eq!(plan(&show, &cx(&open)), Plan::Tabs { window: 9, urls: vec![] });
        assert!(matches!(plan(&show, &cx(&[])), Plan::Nothing { why } if why.contains("GitHub")));
        // Closed: opened again where it was.
        let saved = |p: Provider| {
            (p == Provider::Github).then(|| signin::Saved {
                url: "https://github.com/login/device/success".into(),
                code: Some("X".into()),
            })
        };
        let c = Context { open: vec![], default_mode: Mode::Full, first: true, saved_signin: &saved };
        let Plan::New { spec, urls } = plan(&show, &c) else { panic!("opened again") };
        assert_eq!(urls, ["https://github.com/login/device/success"]);
        assert_eq!(spec.signin.unwrap().code.as_deref(), Some("X"));

        let done = req(&["--signin", "github", "--done"]);
        assert_eq!(plan(&done, &cx(&open)), Plan::Close { windows: vec![9] });
        assert_eq!(plan(&done, &cx(&[])), Plan::Close { windows: vec![] });
    }

    #[test]
    fn titles_and_new_windows() {
        assert_eq!(full().title(Some("Rust")), "Rust – WadBrowser");
        assert_eq!(full().title(Some("  ")), "WadBrowser");
        assert_eq!(WindowSpec::browser(Mode::Full, true, None).title(None), "WadBrowser (Private)");
        let focus = WindowSpec::browser(Mode::Focus, false, Some("work".into()));
        assert_eq!(focus.sibling(false), focus);
        assert_eq!(focus.sibling(true), WindowSpec::browser(Mode::Focus, true, None));
        assert_eq!(claude().sibling(false), full());
        assert!(full().has_urlbar() && !focus.has_urlbar() && !full().autohide_tabs());
        assert!(full().saved_in_session() && !WindowSpec::browser(Mode::Full, true, None).saved_in_session());
    }
}
