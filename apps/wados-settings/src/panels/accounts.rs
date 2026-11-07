//! Settings ▸ Accounts (the spike's `panels/AccountsPanel.svelte`):
//!
//! - **GitHub:** credentials to clone projects and save your work. Not
//!   signed in: Sign in with GitHub (a Github job, in the browser); signed
//!   in: who, and Sign out.
//! - **Tailscale:** where this machine is with the tailnet (a chip), Join
//!   (a Tailnet job, in the browser) or Leave the Tailnet; this machine's
//!   name and address on it; Taildrive, and what the tailnet's policy needs
//!   when it won't share.
//! - **Asking to Be Trusted:** devices that called this machine and aren't
//!   trusted: Deny, or Trust.
//! - **Devices:** the tailnet's devices this machine knows, trusted or
//!   not, with Untrust (or Trust); my machines are marked, and always trusted.
//!
//! A sign-in waiting in the browser shows under its group, with Show the browser.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::{Rc, Weak};

use gtk::{gio, glib, prelude::*};
use indexmap::IndexMap;
use wad_gtk::kit::{self, Group, Row, Tone, Variant};
use wad_model::{JobKind, JobStatus};
use wad_proto::{
    DeviceKind, DriveState, Event, GithubAccount, JobInfo, MachineInfo, SignInProvider, SignInRequest, Tailnet,
    TailnetAsk, TailnetDevice, TailnetState,
};
use wad_wadd_client::{Failure, Incoming};

use crate::link::Link;
use crate::panel::{Ctx, Panel};

const STEAM_DESC: &str = "Your game libraries: gamespaces play the games these accounts own. Optional.";

/// What the panel knows, from wadd's events.
#[derive(Debug, Default)]
pub struct State {
    pub connected: bool,
    pub machine: Option<MachineInfo>,
    pub tailnet: Option<Tailnet>,
    /// Said yet (none: signed out).
    pub github: Option<Option<GithubAccount>>,
    /// The GitHub and Tailnet jobs, by id.
    pub jobs: IndexMap<String, JobInfo>,
    /// Sign-ins waiting in the browser.
    pub signins: HashSet<SignInProvider>,
    /// The Steam accounts here, and the games ticked to push.
    pub steam: Vec<wad_proto::SteamAccount>,
    pub ticked: HashSet<String>,
}

impl State {
    /// One of wadd's events: true if the panel shows what it changed.
    pub fn apply(&mut self, e: Event) -> bool {
        match e {
            Event::Machine(m) => self.machine = Some(m),
            Event::Tailnet(t) => self.tailnet = Some(t),
            Event::Github { account } => self.github = Some(account),
            Event::Job { job } if matches!(job.job.kind, JobKind::Github | JobKind::Tailnet | JobKind::SteamLogin) => {
                self.jobs.insert(job.job.id.clone(), *job);
            }
            Event::JobGone { id } => return self.jobs.shift_remove(&id).is_some(),
            Event::SignIn { provider, .. } => {
                self.signins.insert(provider);
            }
            Event::SignInDone { provider } => {
                self.signins.remove(&provider);
            }
            _ => return false,
        }
        true
    }

    /// The latest job of `kind`.
    pub fn latest(&self, kind: JobKind) -> Option<&JobInfo> {
        self.jobs.values().filter(|j| j.job.kind == kind).max_by_key(|j| j.job.started_at)
    }
}

// ------------------------------------------------------------ what it says

/// GitHub's group's description (Set up's words for what it's for).
pub const GITHUB_DESC: &str = "Credentials to clone projects and save your work";
pub const TAILSCALE_DESC: &str = "Your private network: your machines and your phone reach each other over it.";
pub const ASKS_DESC: &str = "Devices on your tailnet that want to open, stream and build your wadspaces from here.";
pub const DEVICES_DESC: &str = "On your tailnet. Trusted ones can manage this machine.";
/// What signing out does (and doesn't: WadOS can't revoke the token without its app's secret).
pub const SIGN_OUT_NOTE: &str = "Signing out here forgets the sign-in on this machine; GitHub still lists WadOS under your authorized apps until you revoke it there.";

/// GitHub's row: its title and subtitle, signed in or not.
/// A Steam account's line (the spike's).
pub fn steam_line(a: &wad_proto::SteamAccount) -> String {
    let here = a.games.iter().filter(|g| g.installed).count();
    format!("Signed in · {} games · {here} downloaded here", a.games.len())
}

pub fn github_row(a: Option<&GithubAccount>) -> (String, String) {
    match a {
        Some(a) => (a.name.clone(), format!("Signed in as @{}", a.login)),
        None => ("Not signed in".into(), "Sign in to clone your repositories here and push your work back".into()),
    }
}

/// A tailnet state as Settings words it (`panels/tailnet.ts`).
pub fn state_word(s: TailnetState) -> &'static str {
    match s {
        TailnetState::Off => "Off",
        TailnetState::NeedsLogin => "Not signed in",
        TailnetState::LoggingIn => "Waiting for you to sign in",
        TailnetState::NeedsApproval => "Waiting for approval",
        TailnetState::Stopped => "Disconnected",
        TailnetState::Starting => "Starting…",
        TailnetState::Running => "Connected",
    }
}

/// Off the tailnet: what this machine's row says under its name.
pub fn join_subtitle(t: &Tailnet) -> String {
    match t.state {
        TailnetState::Stopped if !t.tailnet.is_empty() => format!("Not on {} right now", t.tailnet),
        TailnetState::NeedsApproval => "Your tailnet's admin has to approve this machine".into(),
        TailnetState::Off => "Tailscale isn't running on this machine".into(),
        _ => "Join to reach your other machines and use the phone remote".into(),
    }
}

/// A device's glyph (AccountsPanel's KIND_ICON).
pub fn kind_icon(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::Phone | DeviceKind::Other => "smartphone",
        DeviceKind::Tablet => "tablet",
        DeviceKind::Laptop => "laptop",
        DeviceKind::Desktop => "monitor",
        DeviceKind::Server => "server",
    }
}

/// An ask's subtitle: what it runs, its address, when it asked.
pub fn ask_subtitle(a: &TailnetAsk, now_ms: i64) -> String {
    format!("{} · {} · asked {}", kit::os_name(&a.os), a.ip, kit::ago(now_ms, a.at))
}

/// A device's subtitle: what it runs, online or not (one of mine: says so).
pub fn device_subtitle(d: &TailnetDevice) -> String {
    let mut s = format!("{} · {}", kit::os_name(&d.os), if d.online { "Online" } else { "Offline" });
    if d.mine {
        s.push_str(" · One of your machines");
    }
    s
}

/// Taildrive, when it doesn't share: what's wrong, and what to do.
pub fn drive_problem(t: &Tailnet) -> Option<(String, String)> {
    if t.state != TailnetState::Running {
        return None;
    }
    match t.drive {
        DriveState::NotAllowed => Some((
            "Taildrive isn't allowed on your tailnet".into(),
            "Your machines browse each other's homes, and wadspaces' folders come over, with Taildrive. \
             The tailnet's access controls have to let this machine share (the drive:share node attribute) \
             and reach other machines' shares (drive:access, with a grant of tailscale.com/cap/drive). \
             Until they do, other machines' files can't be browsed from here, nor this one's from them. \
             Add this in the Tailscale admin console's Access controls:"
                .into(),
        )),
        DriveState::Failed => Some((
            "This machine's home isn't shared".into(),
            format!(
                "Taildrive couldn't share it{}. Your other machines can't browse it, or use its folders, until it does.",
                t.drive_error.as_deref().map(|e| format!(": {e}")).unwrap_or_default()
            ),
        )),
        DriveState::Off | DriveState::Shared => None,
    }
}

/// What the tailnet's policy needs for Taildrive (Tailscale's own example).
pub const DRIVE_POLICY: &str = r#""nodeAttrs": [
  {"target": ["autogroup:member"], "attr": ["drive:share", "drive:access"]}
],
"grants": [
  {"src": ["autogroup:member"], "dst": ["autogroup:self"],
   "app": {"tailscale.com/cap/drive": [{"shares": ["*"], "access": "rw"}]}}
]"#;

/// The browser to drive (as the shell's sign-in driver).
pub fn wadbrowser() -> String {
    std::env::var("WADOS_WADBROWSER").ok().filter(|p| !p.is_empty()).unwrap_or_else(|| "wadbrowser".into())
}

/// Show the browser: `wadbrowser --signin <provider> --show`.
pub fn show_argv(provider: SignInProvider) -> Vec<String> {
    vec![wadbrowser(), "--signin".into(), provider.as_str().into(), "--show".into()]
}

pub fn now_ms() -> i64 {
    glib::real_time() / 1000
}

// ------------------------------------------------------------ the page

pub struct Accounts {
    pub root: gtk::Box,
    column: gtk::Box,
    note: gtk::Label,
    link: Link,
    me: Weak<Accounts>,
    pub state: RefCell<State>,
    /// Calls on their way, by what they do ("signin", "trust:<id>"…).
    pending: RefCell<HashSet<String>>,
    queued: std::cell::Cell<bool>,
    /// Set up WadSpaces leaves Steam out (it's Settings').
    steam: std::cell::Cell<bool>,
}

impl Accounts {
    pub fn new(link: Link) -> Rc<Self> {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        let bar = gtk::CenterBox::builder().css_classes(["kit-page-bar"]).build();
        bar.set_center_widget(Some(&gtk::Label::builder().label("Accounts").css_classes(["kit-page-title"]).build()));
        root.append(&bar);
        let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(28)
            .css_classes(["kit-column"])
            .halign(gtk::Align::Center)
            .build();
        column.set_size_request(644, -1);
        let note = kit::label("", &["kit-group-desc"]);
        note.set_visible(false);
        let outer = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        outer.append(&column);
        scroll.set_child(Some(&outer));
        root.append(&scroll);
        Rc::new_cyclic(|me| Self {
            root,
            column,
            note,
            link,
            me: me.clone(),
            state: RefCell::default(),
            pending: RefCell::default(),
            queued: std::cell::Cell::new(false),
            steam: std::cell::Cell::new(true),
        })
    }

    /// Drawn again on the next idle (a burst of events is one draw).
    pub fn queue_draw(&self) {
        if self.queued.replace(true) {
            return;
        }
        let w = self.me.clone();
        glib::idle_add_local_once(move || {
            if let Some(m) = w.upgrade() {
                m.queued.set(false);
                m.draw();
            }
        });
    }

    fn busy(&self, what: &str) -> bool {
        self.pending.borrow().contains(what)
    }

    /// A call to wadd for `what` (its button waits meanwhile); a refusal shows under the page's title.
    fn ask<T: Send + 'static, F>(&self, what: String, call: impl FnOnce(wad_wadd_client::Wadd) -> F)
    where
        F: std::future::Future<Output = Result<T, Failure>> + Send + 'static,
    {
        self.pending.borrow_mut().insert(what.clone());
        self.note.set_visible(false);
        self.queue_draw();
        let w = self.me.clone();
        self.link.call(call, move |r| {
            let Some(m) = w.upgrade() else { return };
            m.pending.borrow_mut().remove(&what);
            if let Err(e) = r {
                m.note.set_label(&e.message);
                m.note.set_visible(true);
            }
            m.queue_draw();
        });
    }

    pub fn draw(&self) {
        while let Some(c) = self.column.first_child() {
            self.column.remove(&c);
        }
        let st = self.state.borrow();
        if !st.connected {
            self.column.append(&kit::hero(
                "circle-alert",
                "wadd isn't answering",
                "Settings shows this machine's accounts once wadd, which runs them, answers.",
            ));
            return;
        }
        self.column.append(&self.note);
        self.column.append(&self.github(&st).root);
        self.column.append(&self.tailscale(&st).root);
        if self.steam.get() {
            self.column.append(&self.steam(&st).root);
        }
        let t = st.tailnet.clone().unwrap_or_default();
        if t.state == TailnetState::Running && !t.asks.is_empty() {
            self.column.append(&self.asks(&t).root);
        }
        if !t.devices.is_empty() {
            self.column.append(&self.devices(&t).root);
        }
    }

    /// wadd's word on the Steam accounts, read again from their folders.
    pub fn load_steam(&self) {
        let w = self.me.clone();
        self.link.call(
            |wadd| async move { wadd.steam().await },
            move |r| {
                if let (Some(m), Ok(list)) = (w.upgrade(), r) {
                    m.state.borrow_mut().steam = list;
                    m.queue_draw();
                }
            },
        );
    }

    fn steam(&self, st: &State) -> Group {
        let g = Group::new("Steam", STEAM_DESC);
        g.root.set_widget_name("group-steam");
        let job = st.latest(JobKind::SteamLogin);
        let busy = job.is_some_and(|j| j.job.status == JobStatus::Running) || self.busy("steam");
        for a in &st.steam {
            let row = Row::new(Some("gamepad-2"), Tone::Accent, &a.name, &steam_line(a)).named("steam");
            g.add(&row.root);
            for game in a.games.iter().filter(|x| x.installed && !x.pushed) {
                let (w, id) = (self.me.clone(), game.id.clone());
                let on = st.ticked.contains(&game.id);
                let toggle = wad_gtk::form::Toggle::new(on, move |on| {
                    if let Some(m) = w.upgrade() {
                        let mut s = m.state.borrow_mut();
                        if on {
                            s.ticked.insert(id.clone());
                        } else {
                            s.ticked.remove(&id);
                        }
                    }
                });
                let r = Row::new(None, Tone::Plain, &game.name, "Downloaded here");
                r.ctl.append(&toggle.root);
                g.add(&r.root);
            }
            if a.games.iter().any(|x| x.installed && !x.pushed) {
                let push = kit::button(
                    "Keep the ones ticked in your GitHub packages",
                    Variant::Secondary,
                    Some("upload"),
                    true,
                );
                let (w, account) = (self.me.clone(), a.id.clone());
                let ids: Vec<String> = a.games.iter().map(|x| x.id.clone()).collect();
                push.connect_clicked(move |_| {
                    let Some(m) = w.upgrade() else { return };
                    let chosen: Vec<String> =
                        ids.iter().filter(|i| m.state.borrow().ticked.contains(*i)).cloned().collect();
                    if chosen.is_empty() {
                        return;
                    }
                    let account = account.clone();
                    m.ask("steam-push".into(), move |wadd| async move { wadd.steam_push(&account, chosen).await });
                });
                g.add(&push);
            }
        }
        let (title, sub) = match st.steam.is_empty() {
            true => ("Not signed in", "Sign in to play the games you own in gamespaces"),
            false => ("Another Steam account", "Its library joins the others"),
        };
        let row = Row::new(Some("gamepad-2"), Tone::Plain, title, sub).named("steam-new");
        let b = kit::button(
            if busy { "Signing in…" } else { "Sign in with Steam" },
            Variant::Primary,
            Some("log-in"),
            true,
        );
        b.set_sensitive(!busy);
        let w = self.me.clone();
        b.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.ask("steam".into(), |wadd| async move { wadd.steam_signin("Steam").await });
            }
        });
        row.ctl.append(&b);
        g.add(&row.root);
        if self.note.label().contains("sign in to GitHub once more") {
            let again = kit::button("Sign in to GitHub for packages", Variant::Secondary, Some("log-in"), true);
            let w = self.me.clone();
            again.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    let req = SignInRequest { scopes: vec!["write:packages".into()] };
                    m.ask("signin".into(), move |wadd| async move { wadd.github_signin(&req).await });
                }
            });
            g.add(&again);
        }
        g
    }

    fn github(&self, st: &State) -> Group {
        let g = Group::new("GitHub", GITHUB_DESC);
        g.root.set_widget_name("group-github");
        let account = st.github.clone().flatten();
        let job = st.latest(JobKind::Github);
        let running = job.is_some_and(|j| j.job.status == JobStatus::Running);
        let (title, sub) = github_row(account.as_ref());
        match &account {
            Some(_) => {
                let row = Row::new(Some("user"), Tone::Accent, &title, &sub).named("github");
                let out = kit::button("Sign out", Variant::Quiet, None, true);
                out.set_sensitive(!self.busy("signout"));
                let w = self.me.clone();
                out.connect_clicked(move |_| {
                    if let Some(m) = w.upgrade() {
                        m.ask("signout".into(), |wadd| async move { wadd.github_signout().await });
                    }
                });
                row.ctl.append(&out);
                g.add(&row.root);
                let note = gtk::Box::builder().css_classes(["kit-inset"]).build();
                note.append(&kit::label(SIGN_OUT_NOTE, &["kit-row-sub"]));
                g.add(&note);
            }
            None => {
                let row = Row::new(Some("user"), Tone::Plain, &title, &sub).named("github");
                let busy = running || self.busy("signin");
                let b = kit::button(
                    if busy { "Signing in…" } else { "Sign in with GitHub" },
                    Variant::Primary,
                    Some("log-in"),
                    true,
                );
                b.set_sensitive(!busy);
                let w = self.me.clone();
                b.connect_clicked(move |_| {
                    if let Some(m) = w.upgrade() {
                        m.ask(
                            "signin".into(),
                            |wadd| async move { wadd.github_signin(&SignInRequest::default()).await },
                        );
                    }
                });
                row.ctl.append(&b);
                g.add(&row.root);
            }
        }
        if let Some(j) =
            job.filter(|j| matches!(j.job.status, JobStatus::Running | JobStatus::Error) && account.is_none())
        {
            g.add(&self.stages(j, SignInProvider::Github));
        }
        g
    }

    fn tailscale(&self, st: &State) -> Group {
        let g = Group::new("Tailscale", TAILSCALE_DESC);
        g.root.set_widget_name("group-tailscale");
        let t = st.tailnet.clone().unwrap_or_default();
        let job = st.latest(JobKind::Tailnet);
        let joining = job.is_some_and(|j| j.job.status == JobStatus::Running) || self.busy("join");
        let class = if joining {
            Some("busy")
        } else if t.state == TailnetState::Running {
            Some("ok")
        } else {
            None
        };
        g.suffix.append(&kit::chip(state_word(t.state), None, class));
        if t.state == TailnetState::Running {
            let me = format!("{}.{}", t.hostname, t.tailnet);
            g.add(
                &Row::new(Some("network"), Tone::Accent, &me, "This machine on your tailnet")
                    .value(&t.ip, true)
                    .named("tailnet-me")
                    .root,
            );
            g.add(&Row::new(None, Tone::Plain, "Tailnet", "").value(&t.tailnet, true).root);
            match drive_problem(&t) {
                Some((title, body)) => {
                    let row = Row::new(Some("triangle-alert"), Tone::Warn, &title, "").named("taildrive");
                    g.add(&row.root);
                    let inset = gtk::Box::builder()
                        .orientation(gtk::Orientation::Vertical)
                        .spacing(8)
                        .css_classes(["kit-inset"])
                        .build();
                    inset.append(&kit::label(&body, &["kit-row-sub"]));
                    if t.drive == DriveState::NotAllowed {
                        let code = gtk::Label::builder()
                            .label(DRIVE_POLICY)
                            .xalign(0.0)
                            .selectable(true)
                            .css_classes(["mono", "set-code"])
                            .build();
                        inset.append(&code);
                    }
                    g.add(&inset);
                }
                None => {
                    let (sub, value) = match t.drive {
                        DriveState::Shared => ("Your home, for your other machines to browse", "Shared"),
                        _ => ("Your home, for your other machines to browse", "Sharing…"),
                    };
                    g.add(&Row::new(None, Tone::Plain, "Taildrive", sub).value(value, false).named("taildrive").root);
                }
            }
            let leave = Row::new(
                None,
                Tone::Plain,
                "Leave the Tailnet",
                "Your other machines, private streams and the phone remote stop reaching this one",
            );
            let b = kit::button("Leave", Variant::Danger, None, true);
            b.set_widget_name("tailnet-leave");
            b.set_sensitive(!self.busy("leave"));
            let w = self.me.clone();
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.ask("leave".into(), |wadd| async move { wadd.tailnet_logout().await });
                }
            });
            leave.ctl.append(&b);
            g.add(&leave.root);
        } else {
            let name = if t.hostname.is_empty() {
                st.machine.as_ref().map(|m| m.name.clone()).unwrap_or_else(|| "This machine".into())
            } else {
                t.hostname.clone()
            };
            let row = Row::new(Some("network"), Tone::Plain, &name, &join_subtitle(&t)).named("tailnet-me");
            let b = kit::button(if joining { "Joining…" } else { "Join" }, Variant::Primary, Some("log-in"), true);
            b.set_widget_name("tailnet-join");
            b.set_sensitive(!joining && t.state != TailnetState::Off);
            let w = self.me.clone();
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.ask("join".into(), |wadd| async move { wadd.tailnet_login().await });
                }
            });
            row.ctl.append(&b);
            g.add(&row.root);
            if let Some(j) = job.filter(|j| matches!(j.job.status, JobStatus::Running | JobStatus::Error)) {
                g.add(&self.stages(j, SignInProvider::Tailscale));
            }
        }
        g
    }

    /// A job's stages under its group; a sign-in waiting, with Show the browser.
    fn stages(&self, j: &JobInfo, provider: SignInProvider) -> gtk::Box {
        let job = &j.job;
        let held = job.status == JobStatus::Running && job.stages.get(job.cursor).is_some_and(|s| s.held);
        let code = job.params.get("code").and_then(|c| c.as_str()).map(String::from);
        let extra = held.then(|| {
            let w = self.me.clone();
            kit::in_browser(provider.name(), code.as_deref(), move || {
                if let Some(m) = w.upgrade() {
                    m.show_browser(provider);
                }
            })
            .upcast::<gtk::Widget>()
        });
        let inset = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["kit-inset"]).build();
        inset.append(&kit::stages(job, extra.as_ref()));
        inset
    }

    fn show_browser(&self, provider: SignInProvider) {
        let argv = show_argv(provider);
        let args: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
        match gio::Subprocess::newv(&args, gio::SubprocessFlags::NONE) {
            Ok(child) => {
                let w = self.me.clone();
                glib::spawn_future_local(async move {
                    if child.wait_check_future().await.is_err()
                        && let Some(m) = w.upgrade()
                    {
                        m.note.set_label(&format!("There's no sign-in to {} to show", provider.name()));
                        m.note.set_visible(true);
                    }
                });
            }
            Err(e) => {
                self.note.set_label(&format!("WadBrowser didn't start: {e}"));
                self.note.set_visible(true);
            }
        }
    }

    fn asks(&self, t: &Tailnet) -> Group {
        let g = Group::new("Asking to Be Trusted", ASKS_DESC);
        g.root.set_widget_name("group-asks");
        let now = now_ms();
        for a in t.asks.iter().rev() {
            let row = Row::new(Some(kind_icon(a.kind)), Tone::Warn, &a.device, &ask_subtitle(a, now))
                .named(&format!("ask-{}", a.id));
            let deny = kit::button("Deny", Variant::Ghost, None, true);
            let trust = kit::button("Trust", Variant::Primary, Some("shield-check"), true);
            let key = format!("ask:{}", a.id);
            for (b, yes) in [(&deny, false), (&trust, true)] {
                b.set_sensitive(!self.busy(&key));
                let (w, id, key) = (self.me.clone(), a.id.clone(), key.clone());
                b.connect_clicked(move |_| {
                    let Some(m) = w.upgrade() else { return };
                    let id = id.clone();
                    m.ask(key.clone(), move |wadd| async move {
                        if yes { wadd.trust_device(&id).await } else { wadd.deny_ask(&id).await }
                    });
                });
                row.ctl.append(b);
            }
            g.add(&row.root);
        }
        g
    }

    fn devices(&self, t: &Tailnet) -> Group {
        let g = Group::new("Devices", DEVICES_DESC);
        g.root.set_widget_name("group-devices");
        for d in &t.devices {
            let tone = if d.trusted { Tone::Accent } else { Tone::Plain };
            let row = Row::new(Some(kind_icon(d.kind)), tone, &d.name, &device_subtitle(d))
                .named(&format!("device-{}", d.id));
            if d.mine {
                row.ctl.append(&kit::chip("Your machine", Some("house"), None));
            }
            row.ctl.append(&kit::chip(
                if d.trusted { "Trusted" } else { "Not trusted" },
                Some(if d.trusted { "shield-check" } else { "shield-off" }),
                d.trusted.then_some("ok"),
            ));
            let key = format!("device:{}", d.id);
            if !d.mine {
                let b = if d.trusted {
                    kit::button("Untrust", Variant::Quiet, None, true)
                } else {
                    kit::button("Trust", Variant::Quiet, None, true)
                };
                b.set_sensitive(!self.busy(&key));
                let (w, id, trusted) = (self.me.clone(), d.id.clone(), d.trusted);
                b.connect_clicked(move |_| {
                    let Some(m) = w.upgrade() else { return };
                    let id = id.clone();
                    m.ask(format!("device:{id}"), move |wadd| async move {
                        if trusted { wadd.untrust_device(&id).await } else { wadd.trust_device(&id).await }
                    });
                });
                row.ctl.append(&b);
            }
            g.add(&row.root);
        }
        g
    }

    /// What it shows, in words (`--dump`, for tests).
    pub fn dump(&self) -> String {
        fn walk(w: &gtk::Widget, out: &mut Vec<String>) {
            if !w.is_visible() {
                return;
            }
            if let Some(l) = w.downcast_ref::<gtk::Label>() {
                let t = l.label();
                if !t.is_empty() {
                    out.push(t.to_string());
                }
            }
            let mut c = w.first_child();
            while let Some(x) = c {
                walk(&x, out);
                c = x.next_sibling();
            }
        }
        let mut out = vec![];
        walk(self.root.upcast_ref(), &mut out);
        out.join(" | ")
    }
}

// ------------------------------------------------------------ as a panel

/// Accounts as Settings' panel: its page is made with it, and keeps up
/// with wadd's events from the start.
pub fn make(ctx: &Ctx) -> Rc<dyn Panel> {
    Rc::new(AccountsPanel(Accounts::new(ctx.link.clone())))
}

/// Accounts as Set up WadSpaces' step: GitHub and Tailscale, no Steam.
pub fn make_for_setup(ctx: &Ctx) -> Rc<dyn Panel> {
    let a = Accounts::new(ctx.link.clone());
    a.steam.set(false);
    Rc::new(AccountsPanel(a))
}

struct AccountsPanel(Rc<Accounts>);

impl Panel for AccountsPanel {
    fn build(&self, _: &Ctx) -> gtk::Widget {
        self.0.draw();
        self.0.load_steam();
        self.0.root.clone().upcast()
    }

    fn on_event(&self, _: &Ctx, i: &Incoming) {
        let a = &self.0;
        let changed = match i {
            Incoming::Event(e) => {
                // A sign-in or a push ending changes what the accounts say.
                if let Event::Job { job } = e
                    && matches!(job.job.kind, JobKind::SteamLogin | JobKind::GameStage)
                    && job.job.status != JobStatus::Running
                {
                    a.load_steam();
                }
                let mut st = a.state.borrow_mut();
                let was = st.connected;
                st.connected = true;
                st.apply(e.clone()) || !was
            }
            Incoming::Disconnected => {
                let mut st = a.state.borrow_mut();
                std::mem::replace(&mut st.connected, false)
            }
            Incoming::Other(..) => false,
        };
        if changed {
            a.queue_draw();
        }
    }

    fn ready(&self) -> bool {
        self.0.state.borrow().connected
    }

    fn dump(&self, _: &gtk::Widget) -> String {
        self.0.draw();
        self.0.dump()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn github_says_who_or_how_to_sign_in() {
        let a = GithubAccount { login: "hunggod".into(), name: "Hung".into(), scopes: vec![] };
        assert_eq!(github_row(Some(&a)), ("Hung".into(), "Signed in as @hunggod".into()));
        assert_eq!(github_row(None).0, "Not signed in");
        // GitHub is for clones and packages now, not the spike's account.
        assert_eq!(GITHUB_DESC, "Credentials to clone projects and save your work");
    }

    #[test]
    fn the_tailnet_in_words() {
        assert_eq!(state_word(TailnetState::Running), "Connected");
        assert_eq!(state_word(TailnetState::NeedsLogin), "Not signed in");
        let mut t = Tailnet { state: TailnetState::Stopped, tailnet: "tail1234.ts.net".into(), ..Tailnet::default() };
        assert_eq!(join_subtitle(&t), "Not on tail1234.ts.net right now");
        t.state = TailnetState::NeedsLogin;
        assert_eq!(join_subtitle(&t), "Join to reach your other machines and use the phone remote");
    }

    #[test]
    fn asks_and_devices_say_what_and_whose() {
        let a: TailnetAsk = serde_json::from_value(json!({
            "id": "nTab1", "device": "Galaxy Tab S9", "os": "android", "kind": "tablet", "ip": "100.88.20.7", "at": 1_000_000
        }))
        .unwrap();
        assert_eq!(ask_subtitle(&a, 1_000_000 + 60_000), "Android · 100.88.20.7 · asked 1 min ago");
        let d: TailnetDevice = serde_json::from_value(json!({
            "id": "nDesk1", "name": "Desk", "os": "linux", "kind": "desktop", "online": true, "trusted": true, "mine": true
        }))
        .unwrap();
        assert_eq!(device_subtitle(&d), "Linux · Online · One of your machines");
        assert_eq!(kind_icon(d.kind), "monitor");
    }

    #[test]
    fn taildrive_refused_says_what_the_policy_needs() {
        let mut t = Tailnet { state: TailnetState::Running, drive: DriveState::Shared, ..Tailnet::default() };
        assert_eq!(drive_problem(&t), None);
        t.drive = DriveState::NotAllowed;
        let (title, body) = drive_problem(&t).unwrap();
        assert_eq!(title, "Taildrive isn't allowed on your tailnet");
        assert!(body.contains("drive:share") && body.contains("drive:access"));
        assert!(DRIVE_POLICY.contains("drive:share") && DRIVE_POLICY.contains("tailscale.com/cap/drive"));
        t.drive = DriveState::Failed;
        t.drive_error = Some("permission denied".into());
        assert!(drive_problem(&t).unwrap().1.contains(": permission denied"));
        // Off the tailnet, nothing's said of it.
        t.state = TailnetState::Stopped;
        assert_eq!(drive_problem(&t), None);
    }

    #[test]
    fn the_state_takes_the_accounts_events_only() {
        let mut s = State::default();
        assert!(s.apply(Event::Github { account: None }));
        assert_eq!(s.github, Some(None));
        assert!(s.apply(Event::SignIn { provider: SignInProvider::Github, url: "u".into(), code: None }));
        assert!(s.signins.contains(&SignInProvider::Github));
        assert!(s.apply(Event::SignInDone { provider: SignInProvider::Github }));
        assert!(s.signins.is_empty());
        assert!(!s.apply(Event::WadspaceGone { id: "x".into() }));
        assert_eq!(show_argv(SignInProvider::Tailscale)[1..], ["--signin", "tailscale", "--show"]);
    }
}
