//! Settings ▸ Accounts (the spike's `panels/AccountsPanel.svelte`):
//!
//! - **WadSpaces:** the one sign-in, with GitHub (WadSpaces' only way in).
//!   Its GitHub credentials clone projects and save your work, and it links
//!   this machine to your account: your phone lists it, and your secret
//!   files follow you. Not signed in: Sign in to WadSpaces (a Github job, in
//!   the browser); signed in: who, how the link is, Link again when it
//!   isn't, and Sign out (which leaves the account too). On a WadOS with no
//!   account (no firebase.toml), it's the GitHub credentials alone.
//! - **Secret files:** your projects' files kept out of git, put in every
//!   clone of their repository on every machine of the account, with Remove;
//!   and what's asking about files in this machine's clones (a likely one to
//!   keep, or a kept one changed here), with its answers.
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
    AccountLink, AccountState, DeviceKind, DriveState, Event, GithubAccount, JobInfo, MachineInfo, SecretFile,
    SecretFileAsk, SecretFileAskKind, SecretFiles, SecretOrigin, SignInProvider, SignInRequest, Tailnet, TailnetAsk,
    TailnetDevice, TailnetState,
};
use wad_wadd_client::{Failure, Incoming};

use crate::link::Link;
use crate::panel::{Ctx, Panel};

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
    /// The machine's link to its WadSpaces account, and the secret files.
    pub account: Option<AccountLink>,
    pub secret_files: Option<SecretFiles>,
}

impl State {
    /// One of wadd's events: true if the panel shows what it changed.
    pub fn apply(&mut self, e: Event) -> bool {
        match e {
            Event::Machine(m) => self.machine = Some(m),
            Event::Tailnet(t) => self.tailnet = Some(t),
            Event::Github { account } => self.github = Some(account),
            Event::Account(a) => self.account = Some(a),
            Event::SecretFiles(f) => self.secret_files = Some(f),
            Event::Job { job } if matches!(job.job.kind, JobKind::Github | JobKind::Tailnet) => {
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

/// The sign-in's group's description (Set up's words for what it's for).
pub const WADSPACES_DESC: &str = "One sign-in, with GitHub: your projects clone here and push your work back, your \
     phone lists this machine, and your secret files follow you.";
/// The same on a WadOS with no account to link to.
pub const WADSPACES_DESC_NO_ACCOUNT: &str = "Sign in with GitHub to clone your projects here and push your work back.";
pub const SECRETS_DESC: &str = "Your projects' files kept out of git (.env, keys, local config): put in every clone \
     of their repository, on every machine of your account. A file changed in a clone is never written over.";
pub const TAILSCALE_DESC: &str = "Your private network: your machines and your phone reach each other over it.";
pub const ASKS_DESC: &str = "Devices on your tailnet that want to open, stream and build your wadspaces from here.";
pub const DEVICES_DESC: &str = "On your tailnet. Trusted ones can manage this machine.";
/// What signing out does (and doesn't: WadOS can't revoke the token without its app's secret).
pub const SIGN_OUT_NOTE: &str = "Signing out forgets the sign-in on this machine and takes it off your WadSpaces \
     account. GitHub still lists WadOS under your authorized apps until you revoke it there.";

/// The sign-in's row: its title and subtitle. `github`: who's signed in here;
/// `account`: the machine's link (None until wadd says, or Off: no account).
pub fn signin_row(github: Option<&GithubAccount>, account: Option<&AccountLink>, now_ms: i64) -> (String, String) {
    let account = account.filter(|a| a.state != AccountState::Off);
    let Some(gh) = github else {
        let sub = match account {
            Some(_) => {
                "Sign in to clone your repositories, reach this machine from your phone, and keep your secret files"
            }
            None => "Sign in to clone your repositories here and push your work back",
        };
        return ("Not signed in".into(), sub.into());
    };
    let title = if gh.name.is_empty() { gh.login.clone() } else { gh.name.clone() };
    let who = format!("Signed in as @{}", gh.login);
    let sub = match account {
        None => who,
        Some(a) => match a.state {
            AccountState::Linking => format!("{who} · Joining your account…"),
            AccountState::Linked => match a.last_seen {
                Some(t) => format!("{who} · Your account heard from it {}", kit::ago(now_ms, t)),
                None => format!("{who} · Your phone lists this machine"),
            },
            _ => format!("{who} · Not linked to your account"),
        },
    };
    (title, sub)
}

/// A size in words (a secret file is at most 64 KB).
pub fn size_words(n: u64) -> String {
    match n {
        0..1024 => format!("{n} bytes"),
        _ => format!("{:.1} KB", n as f64 / 1024.0),
    }
}

/// A secret file's subtitle: its repository, size, and where it's used.
pub fn secret_subtitle(f: &SecretFile) -> String {
    let mut s = format!("{} · {}", f.repo, size_words(f.size));
    match f.used_by.len() {
        0 => s.push_str(" · No clone here"),
        1 => s.push_str(" · In 1 wadspace here"),
        n => s.push_str(&format!(" · In {n} wadspaces here")),
    }
    if f.origin == SecretOrigin::Local {
        s.push_str(" · This machine only, until it's linked");
    }
    s
}

/// An ask's title, subtitle, and its two answers (no, then yes).
pub fn secret_ask_words(a: &SecretFileAsk) -> (String, String, &'static str, &'static str) {
    let place = format!("{} in {}", a.project_name, a.wadspace_name);
    match a.kind {
        SecretFileAskKind::New => (
            format!("Keep {}?", a.path),
            format!(
                "{place} has it ({}) and git ignores it; kept, every clone of {} gets it",
                size_words(a.size),
                a.repo
            ),
            "Leave it",
            "Keep",
        ),
        SecretFileAskKind::Changed => (
            format!("{} changed", a.path),
            format!(
                "{place} has another version ({}): keep it for every machine, or for this clone only",
                size_words(a.size)
            ),
            "This clone only",
            "Every machine",
        ),
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
        self.column.append(&self.wadspaces(&st).root);
        if let Some(g) = self.secret_files(&st) {
            self.column.append(&g.root);
        }
        self.column.append(&self.tailscale(&st).root);
        let t = st.tailnet.clone().unwrap_or_default();
        if t.state == TailnetState::Running && !t.asks.is_empty() {
            self.column.append(&self.asks(&t).root);
        }
        if !t.devices.is_empty() {
            self.column.append(&self.devices(&t).root);
        }
    }

    fn wadspaces(&self, st: &State) -> Group {
        let link = st.account.clone().filter(|a| a.state != AccountState::Off);
        let desc = if link.is_some() { WADSPACES_DESC } else { WADSPACES_DESC_NO_ACCOUNT };
        let g = Group::new("WadSpaces", desc);
        g.root.set_widget_name("group-wadspaces");
        let github = st.github.clone().flatten();
        let job = st.latest(JobKind::Github);
        let running = job.is_some_and(|j| j.job.status == JobStatus::Running);
        let (title, sub) = signin_row(github.as_ref(), link.as_ref(), now_ms());
        match &github {
            Some(_) => {
                let linked = link.as_ref().is_some_and(|a| a.state == AccountState::Linked);
                let tone = if linked || link.is_none() { Tone::Accent } else { Tone::Plain };
                let row = Row::new(Some("user"), tone, &title, &sub).named("signin");
                // Signed in but not linked (unlinked, removed, or it failed): Link again.
                if let Some(a) = link.as_ref().filter(|a| a.state != AccountState::Linked) {
                    let busy = self.busy("account") || a.state == AccountState::Linking;
                    let b = kit::button(if busy { "Linking…" } else { "Link" }, Variant::Primary, Some("link"), true);
                    b.set_widget_name("account-link");
                    b.set_sensitive(!busy);
                    let w = self.me.clone();
                    b.connect_clicked(move |_| {
                        if let Some(m) = w.upgrade() {
                            m.ask("account".into(), |wadd| async move { wadd.account_link().await });
                        }
                    });
                    row.ctl.append(&b);
                }
                let out = kit::button("Sign out", Variant::Quiet, None, true);
                out.set_widget_name("signout");
                out.set_sensitive(!self.busy("signout"));
                let w = self.me.clone();
                out.connect_clicked(move |_| {
                    if let Some(m) = w.upgrade() {
                        m.ask("signout".into(), |wadd| async move { wadd.github_signout().await });
                    }
                });
                row.ctl.append(&out);
                g.add(&row.root);
                if let Some(e) = link.as_ref().and_then(|a| a.error.as_ref()) {
                    g.add(&Row::new(Some("triangle-alert"), Tone::Warn, e, "").named("account-error").root);
                }
                // Pushing to GitHub's packages (a WadOS wadspace's host image,
                // a game, a disc) takes write:packages, a second, short sign-in:
                // offered before a push needs it, not only after one failed.
                if github.as_ref().is_some_and(|a| !a.has_scope("write:packages")) {
                    let row = Row::new(
                        Some("package"),
                        Tone::Plain,
                        "Pushing packages",
                        "Not allowed yet: needed to push images, games and discs to your GitHub packages",
                    )
                    .named("packages-signin");
                    let busy = running || self.busy("signin");
                    let b = kit::button(
                        if busy { "Signing in…" } else { "Allow" },
                        Variant::Secondary,
                        Some("log-in"),
                        true,
                    );
                    b.set_widget_name("packages-signin-button");
                    b.set_sensitive(!busy);
                    let w = self.me.clone();
                    b.connect_clicked(move |_| {
                        if let Some(m) = w.upgrade() {
                            let req = SignInRequest { scopes: vec!["write:packages".into()] };
                            m.ask("signin".into(), move |wadd| async move { wadd.github_signin(&req).await });
                        }
                    });
                    row.ctl.append(&b);
                    g.add(&row.root);
                }
                let note = gtk::Box::builder().css_classes(["kit-inset"]).build();
                note.append(&kit::label(SIGN_OUT_NOTE, &["kit-row-sub"]));
                g.add(&note);
            }
            None => {
                let row = Row::new(Some("user"), Tone::Plain, &title, &sub).named("signin");
                let busy = running || self.busy("signin");
                let b = kit::button(
                    if busy { "Signing in…" } else { "Sign in to WadSpaces" },
                    Variant::Primary,
                    Some("log-in"),
                    true,
                );
                b.set_widget_name("signin-button");
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
            job.filter(|j| matches!(j.job.status, JobStatus::Running | JobStatus::Error) && github.is_none())
        {
            g.add(&self.stages(j, SignInProvider::Github));
        }
        g
    }

    /// None until wadd says, or when nothing's kept or asking.
    fn secret_files(&self, st: &State) -> Option<Group> {
        let f = st.secret_files.clone()?;
        if f.files.is_empty() && f.asks.is_empty() {
            return None;
        }
        let g = Group::new("Secret Files", SECRETS_DESC);
        g.root.set_widget_name("group-secret-files");
        for a in &f.asks {
            let (title, sub, no, yes) = secret_ask_words(a);
            let row = Row::new(Some("file-search"), Tone::Warn, &title, &sub).named(&format!("secret-ask-{}", a.id));
            let key = format!("secret-ask:{}", a.id);
            for (label, keep) in [(no, false), (yes, true)] {
                let b = kit::button(label, if keep { Variant::Primary } else { Variant::Ghost }, None, true);
                b.set_sensitive(!self.busy(&key));
                let (w, id, key) = (self.me.clone(), a.id.clone(), key.clone());
                b.connect_clicked(move |_| {
                    let Some(m) = w.upgrade() else { return };
                    let id = id.clone();
                    m.ask(key.clone(), move |wadd| async move { wadd.secret_file_answer(&id, keep).await });
                });
                row.ctl.append(&b);
            }
            g.add(&row.root);
        }
        for file in &f.files {
            let row = Row::new(Some("key-round"), Tone::Accent, &file.path, &secret_subtitle(file))
                .named(&format!("secret-{}", file.name));
            let key = format!("secret:{}", file.name);
            let b = kit::button("Remove", Variant::Quiet, None, true);
            b.set_sensitive(!self.busy(&key));
            let (w, name) = (self.me.clone(), file.name.clone());
            b.connect_clicked(move |_| {
                let Some(m) = w.upgrade() else { return };
                let name = name.clone();
                m.ask(format!("secret:{name}"), move |wadd| async move { wadd.secret_file_remove(&name).await });
            });
            row.ctl.append(&b);
            g.add(&row.root);
        }
        if !f.account && !f.files.is_empty() {
            let note = gtk::Box::builder().css_classes(["kit-inset"]).build();
            note.append(&kit::label(
                "These are on this machine only: linking it to your account takes them there.",
                &["kit-row-sub"],
            ));
            g.add(&note);
        }
        Some(g)
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

/// Accounts as Set up WadSpaces' step: the WadSpaces sign-in and Tailscale.
pub fn make_for_setup(ctx: &Ctx) -> Rc<dyn Panel> {
    Rc::new(AccountsPanel(Accounts::new(ctx.link.clone())))
}

struct AccountsPanel(Rc<Accounts>);

impl Panel for AccountsPanel {
    fn build(&self, _: &Ctx) -> gtk::Widget {
        self.0.draw();
        self.0.root.clone().upcast()
    }

    fn on_event(&self, _: &Ctx, i: &Incoming) {
        let a = &self.0;
        let changed = match i {
            Incoming::Event(e) => {
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
    fn the_wadspaces_sign_in_says_who_and_how_the_link_is() {
        let gh = GithubAccount { login: "hunggod".into(), name: "Hung".into(), scopes: vec![] };
        let mut a = AccountLink { state: AccountState::Unlinked, ..AccountLink::default() };
        // Signed out: one sign-in, for the clones and the account both.
        assert_eq!(signin_row(None, Some(&a), 0).0, "Not signed in");
        assert!(signin_row(None, Some(&a), 0).1.contains("phone"));
        // A WadOS with no account: GitHub's credentials alone.
        let off = AccountLink { state: AccountState::Off, ..AccountLink::default() };
        assert_eq!(
            signin_row(None, Some(&off), 0).1,
            "Sign in to clone your repositories here and push your work back"
        );
        assert_eq!(signin_row(Some(&gh), Some(&off), 0), ("Hung".into(), "Signed in as @hunggod".into()));
        assert_eq!(signin_row(Some(&gh), None, 0).1, "Signed in as @hunggod");
        assert_eq!(signin_row(Some(&gh), Some(&a), 0).1, "Signed in as @hunggod · Not linked to your account");
        a.state = AccountState::Linking;
        assert_eq!(signin_row(Some(&gh), Some(&a), 0).1, "Signed in as @hunggod · Joining your account…");
        a = AccountLink { state: AccountState::Linked, last_seen: Some(1_000_000), ..AccountLink::default() };
        assert_eq!(
            signin_row(Some(&gh), Some(&a), 1_060_000).1,
            "Signed in as @hunggod · Your account heard from it 1 min ago"
        );
        a.last_seen = None;
        assert_eq!(signin_row(Some(&gh), Some(&a), 0).1, "Signed in as @hunggod · Your phone lists this machine");
        assert!(SIGN_OUT_NOTE.contains("off your WadSpaces account"));
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
    fn the_secret_files_in_words() {
        assert_eq!(size_words(512), "512 bytes");
        assert_eq!(size_words(2048), "2.0 KB");
        let f: SecretFile = serde_json::from_value(json!({
            "name": "f-0123456789ab", "repo": "HungGod/wados", "path": ".env", "size": 12,
            "usedBy": ["w1"], "origin": "local"
        }))
        .unwrap();
        assert_eq!(
            secret_subtitle(&f),
            "HungGod/wados · 12 bytes · In 1 wadspace here · This machine only, until it's linked"
        );
        let ask: SecretFileAsk = serde_json::from_value(json!({
            "id": "a-0123456789ab", "kind": "changed", "wadspace": "w1", "wadspaceName": "Work",
            "project": "p1", "projectName": "wados", "repo": "HungGod/wados", "path": ".env", "size": 20
        }))
        .unwrap();
        let (title, sub, no, yes) = secret_ask_words(&ask);
        assert_eq!((title.as_str(), no, yes), (".env changed", "This clone only", "Every machine"));
        assert!(sub.starts_with("wados in Work has another version (20 bytes)"), "{sub}");
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
        assert!(s.apply(Event::Account(AccountLink { state: AccountState::Linked, ..AccountLink::default() })));
        assert_eq!(s.account.as_ref().map(|a| a.state), Some(AccountState::Linked));
        assert!(s.apply(Event::SecretFiles(SecretFiles::default())));
        assert!(s.secret_files.is_some());
        assert!(!s.apply(Event::WadspaceGone { id: "x".into() }));
        assert_eq!(show_argv(SignInProvider::Tailscale)[1..], ["--signin", "tailscale", "--show"]);
    }
}
