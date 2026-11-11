//! Settings ▸ System ▸ Updates (the spike's Updates app, `apps/updates/`:
//! `Updates.svelte` and `status.ts`), as a page of System's:
//!
//! - **WadSpaces itself**: what's booted, when it last looked; then where
//!   its update is: up to date (Check now), checking, downloading (Cancel),
//!   staging, ready (Later, Restart now, Don't apply it), available (one
//!   turned down, or the sample's: Download), offline (Try again), or the
//!   download failed (Retry). The version before, with Roll back (asked
//!   first).
//! - **Base images**: what every wadspace is built on, each one's size and
//!   when it was refreshed (or its progress while it is), when the next
//!   refresh is, Refresh now; a refresh that failed, with Retry.
//!
//! wadd's `updates` event says where the update is, its Update and Pull
//! jobs how far they are; the bases come from `GET /v1/bases` (again when
//! a refresh ends). Restart now asks logind (`Reboot`, interactive), as the
//! shell's Restart does (`WADOS_LOGIND_DRY_RUN=1`: it doesn't).
//!
//! The spike's made-up release notes aren't here: there are none to show.
//!
//! **For System's panel** ([`updates_page`]): the page is made once, with
//! its back button in its bar ([`UpdatesPage::back`]); System hands it each
//! of wadd's events ([`UpdatesPage::on_event`], from its own `on_event`,
//! from when Settings starts), and may ask it [`UpdatesPage::ready`] and
//! [`UpdatesPage::dump`] for `--dump`.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};

use gtk::{gio, glib, prelude::*};
use indexmap::IndexMap;
use wad_gtk::Icon;
use wad_gtk::form;
use wad_gtk::kit::{self, Tone, Variant};
use wad_model::{Job, JobKind, JobStatus, PartState};
use wad_proto::{Bases, Event, JobInfo, UpdateSource, Updates, Version};
use wad_wadd_client::{Failure, Incoming};

use crate::link::Link;
use crate::panel::Ctx;

// ------------------------------------------------------------ what it reads (status.ts)

/// Where WadSpaces' own update is (`status.ts`'s `HostPhase`, and two of
/// wadd's: one available that isn't downloading, a machine bootc doesn't run).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Checking,
    Downloading,
    Staging,
    Ready,
    Available,
    Offline,
    Error,
    Current,
    /// Not a bootc machine (a laptop's wadd).
    Unmanaged,
}

fn live(j: &Job) -> bool {
    matches!(j.status, JobStatus::Running | JobStatus::Queued)
}

/// A running Update job wins (its stage says which), then what's staged,
/// then offline, an error, one available, or up to date.
pub fn phase(u: &Updates, job: Option<&Job>) -> Phase {
    if let Some(j) = job.filter(|j| live(j)) {
        return match j.stages.get(j.cursor).map(|s| s.id.as_str()) {
            Some("check") => Phase::Checking,
            Some("download") => Phase::Downloading,
            _ => Phase::Staging,
        };
    }
    if u.staged.is_some() {
        Phase::Ready
    } else if u.booted.image.is_empty() && u.booted.version.is_empty() {
        Phase::Unmanaged
    } else if !u.online {
        Phase::Offline
    } else if u.error.is_some() {
        Phase::Error
    } else if u.available.is_some() {
        Phase::Available
    } else {
        Phase::Current
    }
}

/// "in 5 h", "in 20 min", "soon".
pub fn until(now_ms: i64, at_ms: i64) -> String {
    let s = (at_ms - now_ms) as f64 / 1000.0;
    if s < 60.0 {
        "soon".into()
    } else if s < 3600.0 {
        format!("in {} min", (s / 60.0).round() as i64)
    } else {
        format!("in {} h", (s / 3600.0).round() as i64)
    }
}

/// "44.20261028.0"-style versions, part by part (a part that isn't a number is 0).
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |s: &str| s.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let d = pa.get(i).unwrap_or(&0).cmp(pb.get(i).unwrap_or(&0));
        if d.is_ne() {
            return d;
        }
    }
    std::cmp::Ordering::Equal
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// The day a version was built ("44.20261028.0" → "28 October 2026").
pub fn release_date(v: &str) -> Option<String> {
    let mut it = v.split('.');
    let (_, day) = (it.next()?.parse::<u32>().ok()?, it.next()?);
    it.next()?;
    if day.len() != 8 || !day.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (y, m, d) = (&day[..4], day[4..6].parse::<usize>().ok()?, day[6..].parse::<u32>().ok()?);
    (1..=12).contains(&m).then(|| format!("{d} {} {y}", MONTHS[m - 1]))
}

/// The spike's `bytes`: "512 B", "40 KB", "3.4 MB", "820 MB", "1.20 GB".
pub fn bytes(n: u64) -> String {
    const K: f64 = 1024.0;
    let f = n as f64;
    if f < K {
        format!("{n} B")
    } else if f < K * K {
        format!("{:.0} KB", f / K)
    } else if f < K * K * K {
        if f < 10.0 * K * K { format!("{:.1} MB", f / K / K) } else { format!("{:.0} MB", f / K / K) }
    } else {
        format!("{:.2} GB", f / K / K / K)
    }
}

/// "about 40 s left", "about 3 min left".
pub fn left(sec: u64) -> String {
    if sec < 60 {
        format!("about {sec} s left")
    } else {
        format!("about {} min left", (sec as f64 / 60.0).round() as u64)
    }
}

/// The registry an image comes from (`ghcr.io`).
pub fn registry_of(image: &str) -> &str {
    match image.split_once('/') {
        Some((host, _)) if host.contains('.') || host.contains(':') => host,
        _ => "ghcr.io",
    }
}

fn same(a: &Version, b: Option<&Version>) -> bool {
    b.is_some_and(|b| a.digest.is_some() && a.digest == b.digest || a.digest.is_none() && a.version == b.version)
}

/// What's staged is the one before (a rollback), not a newer one.
pub fn staged_is_back(u: &Updates) -> bool {
    match &u.staged {
        Some(s) => {
            same(s, u.rollback.as_ref()) || compare_versions(&s.version, &u.booted.version) == std::cmp::Ordering::Less
        }
        None => false,
    }
}

/// The Roll back dialog's words.
pub fn rollback_text(u: &Updates) -> Option<(String, String)> {
    let r = u.rollback.as_ref()?;
    let mut text = format!(
        "WadSpaces {} is staged and applies when this machine restarts. {} stays as the previous version, so you can come back to it.",
        r.version, u.booted.version
    );
    if let Some(s) = &u.staged {
        text.push_str(&format!(" It replaces the staged {}.", s.version));
    }
    Some((format!("Roll back to {}?", r.version), text))
}

pub fn now_ms() -> i64 {
    glib::real_time() / 1000
}

// ------------------------------------------------------------ the state

/// What the page knows, from wadd.
#[derive(Debug, Default)]
pub struct State {
    pub connected: bool,
    pub updates: Option<Updates>,
    pub bases: Option<Bases>,
    /// The Update and Pull jobs, by id.
    pub jobs: IndexMap<String, JobInfo>,
}

impl State {
    /// One of wadd's events: true if the page shows what it changed; and
    /// whether the bases should be asked for again (a refresh ended).
    pub fn apply(&mut self, e: &Event) -> (bool, bool) {
        match e {
            Event::Updates(u) => self.updates = Some((**u).clone()),
            Event::Job { job } if matches!(job.job.kind, JobKind::Update | JobKind::Pull) => {
                let ended = job.job.kind == JobKind::Pull
                    && !live(&job.job)
                    && self.jobs.get(&job.job.id).is_some_and(|j| live(&j.job));
                self.jobs.insert(job.job.id.clone(), (**job).clone());
                return (true, ended);
            }
            Event::JobGone { id } => return (self.jobs.shift_remove(id).is_some(), false),
            Event::Storage(_) => return (false, true),
            _ => return (false, false),
        }
        (true, false)
    }

    /// The Update job wadd says is under way (else the newest live one).
    pub fn host_job(&self) -> Option<&Job> {
        let named = self.updates.as_ref().and_then(|u| u.job.as_ref()).and_then(|id| self.jobs.get(id));
        named
            .or_else(|| self.jobs.values().rev().find(|j| j.job.kind == JobKind::Update && live(&j.job)))
            .map(|j| &j.job)
    }

    /// The last Update job, whatever became of it.
    pub fn last_host_job(&self) -> Option<&Job> {
        self.jobs.values().rev().find(|j| j.job.kind == JobKind::Update).map(|j| &j.job)
    }

    pub fn pull(&self) -> Option<&Job> {
        self.jobs.values().rev().find(|j| j.job.kind == JobKind::Pull && live(&j.job)).map(|j| &j.job)
    }

    pub fn pull_failed(&self) -> Option<&Job> {
        self.jobs
            .values()
            .rev()
            .find(|j| j.job.kind == JobKind::Pull)
            .map(|j| &j.job)
            .filter(|j| j.status == JobStatus::Error)
    }
}

// ------------------------------------------------------------ the page

const CSS: &str = r#"
.upd-card { padding: 18px; border-radius: 14px; border: 1px solid var(--line); background-color: var(--surface-2); }
.upd-h2 { font-family: 'Space Grotesk', Inter, sans-serif; font-size: 16px; font-weight: 600; }
.upd-h3 { font-size: 13px; font-weight: 600; }
.upd-small { font-size: 12px; color: var(--muted); }
.upd-faint { color: var(--faint); }
.upd-code { font-family: monospace; font-size: 12px; color: var(--muted); }
.upd-state { color: var(--muted); }
.upd-state.ok wad-icon { color: var(--accent); }
.upd-notice { padding: 12px 14px; border-radius: 12px; border: 1px solid var(--line-strong); }
.upd-notice-text { color: var(--muted); }
.upd-notice.accent { background-color: var(--accent-soft); border-color: rgb(198 255 31 / 0.3); }
.upd-notice.accent > wad-icon { color: var(--accent); }
.upd-notice.warn { background-color: var(--warn-soft); border-color: rgb(232 195 106 / 0.3); }
.upd-notice.warn > wad-icon { color: var(--warn); }
.upd-notice.danger { background-color: var(--danger-soft); border-color: rgb(255 93 122 / 0.3); }
.upd-notice.danger > wad-icon { color: var(--danger); }
.upd-aside { color: var(--muted); font-size: 13px; }
button.upd-link { min-height: 0; padding: 0; border: none; box-shadow: none; background: none; background-image: none; color: var(--muted); font-size: 12px; }
button.upd-link label { text-decoration-line: underline; }
button.upd-link:hover { color: var(--fg); }
.upd-prev { padding-top: 12px; border-top: 1px solid var(--line); }
.upd-prev > wad-icon { color: var(--muted); }
.upd-tag { font-size: 11px; padding: 2px 8px; border-radius: 99px; background-color: var(--warn-soft); color: var(--warn); }
.upd-bases { border: 1px solid var(--line); border-radius: 12px; }
.upd-sources { margin-top: 2px; }
.upd-base { padding: 10px 14px; background-color: var(--surface); }
.upd-base + .upd-base { border-top: 1px solid var(--line); }
.upd-missing { color: var(--warn); }
"#;

/// Bold in the text's colour (`--fg`), as the spike's `<b>`.
fn brighten(markup: &str) -> String {
    markup.replace("<b>", "<b><span foreground=\"#f5f2fa\">").replace("</b>", "</span></b>")
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

fn small(text: &str) -> gtk::Label {
    kit::label(text, &["upd-small"])
}

fn spinner() -> gtk::Spinner {
    let s = gtk::Spinner::builder().spinning(true).valign(gtk::Align::Center).build();
    s.set_size_request(16, 16);
    s
}

fn progress(value: f64) -> gtk::ProgressBar {
    let p =
        gtk::ProgressBar::builder().fraction(value.clamp(0.0, 1.0)).hexpand(true).css_classes(["kit-progress"]).build();
    p.set_valign(gtk::Align::Center);
    p
}

/// A state line: an icon (or a spinner) and words.
fn state_line(icon: Option<&'static str>, text: &str, ok: bool) -> gtk::Box {
    let b = gtk::Box::builder().spacing(10).css_classes(["upd-state"]).build();
    if ok {
        b.add_css_class("ok");
    }
    match icon {
        Some(i) => {
            let ic = Icon::new(i, 16);
            ic.set_valign(gtk::Align::Center);
            b.append(&ic);
        }
        None => b.append(&spinner()),
    }
    b.append(&kit::label(text, &[]));
    b
}

/// A notice (`.notice.accent|warn|danger`): its icon, its words (markup), its buttons.
fn notice(tone: &str, icon: &'static str, markup: &str, acts: &[gtk::Button]) -> gtk::Box {
    let b = gtk::Box::builder().spacing(12).css_classes(["upd-notice", tone]).build();
    let ic = Icon::new(icon, 18);
    ic.set_valign(gtk::Align::Center);
    b.append(&ic);
    let p = kit::label("", &["upd-notice-text"]);
    p.set_markup(&brighten(markup));
    p.set_hexpand(true);
    b.append(&p);
    if !acts.is_empty() {
        let a = gtk::Box::builder().spacing(6).valign(gtk::Align::Center).build();
        for x in acts {
            a.append(x);
        }
        b.append(&a);
    }
    b
}

/// The card's head: its badge, title over a line, and a button at its end.
fn head(icon: &'static str, title: &str, sub: &str, button: Option<&gtk::Button>) -> gtk::Box {
    let h = gtk::Box::builder().spacing(12).build();
    let badge = kit::icon_tile(icon, Tone::Accent, 40, 20);
    badge.set_valign(gtk::Align::Center);
    h.append(&badge);
    let who = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
    who.set_valign(gtk::Align::Center);
    who.append(&kit::label(title, &["upd-h2"]));
    who.append(&small(sub));
    h.append(&who);
    if let Some(b) = button {
        b.set_valign(gtk::Align::Center);
        h.append(b);
    }
    h
}

fn card(name: &str) -> gtk::Box {
    let c = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).css_classes(["upd-card"]).build();
    c.set_widget_name(name);
    c
}

/// Settings ▸ System ▸ Updates. See the module's docs for how System uses it.
pub struct UpdatesPage {
    /// The page: its bar ("Updates", with a back button) over its cards.
    pub root: gtk::Widget,
    /// Its bar's back button, for System to go back to its overview with.
    pub back: Option<gtk::Button>,
    column: gtk::Box,
    note: gtk::Label,
    link: Link,
    window: gtk::ApplicationWindow,
    me: Weak<UpdatesPage>,
    pub state: RefCell<State>,
    /// "Later" tucked the prompt away for this staged version.
    later: RefCell<Option<String>>,
    /// Calls on their way ("check", "apply"…): their buttons wait.
    pending: RefCell<HashSet<String>>,
    queued: Cell<bool>,
    /// The download's first bytes seen (ms, bytes), for its rate.
    seen: Cell<Option<(i64, u64)>>,
}

/// The Updates page, for System's panel to show at its `updates` page.
pub fn updates_page(ctx: &Ctx) -> Rc<UpdatesPage> {
    if let Some(d) = gtk::gdk::Display::default() {
        wad_gtk::theme::add_css(&d, CSS);
    }
    let page = kit::Page::new("Updates", true);
    page.column.set_spacing(16);
    let note = kit::label("", &["kit-group-desc"]);
    note.set_visible(false);
    let me = Rc::new_cyclic(|me| UpdatesPage {
        root: page.root.clone().upcast(),
        back: page.back.clone(),
        column: page.column.clone(),
        note,
        link: ctx.link.clone(),
        window: ctx.window.clone(),
        me: me.clone(),
        state: RefCell::default(),
        later: RefCell::default(),
        pending: RefCell::default(),
        queued: Cell::new(false),
        seen: Cell::new(None),
    });
    // "Checked 5 min ago", "next in 9 h": kept true.
    let w = me.me.clone();
    glib::timeout_add_seconds_local(30, move || match w.upgrade() {
        Some(m) => {
            if m.root.is_mapped() {
                m.queue_draw();
            }
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
    me.draw();
    me
}

impl UpdatesPage {
    /// Each of wadd's events (System's `on_event` hands them on).
    pub fn on_event(&self, i: &Incoming) {
        let (changed, bases) = match i {
            Incoming::Event(e) => {
                let mut st = self.state.borrow_mut();
                let was = st.connected;
                st.connected = true;
                let (c, b) = st.apply(e);
                (c || !was, b || !was)
            }
            Incoming::Disconnected => (std::mem::replace(&mut self.state.borrow_mut().connected, false), false),
            Incoming::Other(..) => (false, false),
        };
        if bases {
            self.fetch_bases();
        }
        if changed {
            self.queue_draw();
        }
    }

    /// It knows what it shows (`--dump` waits for it).
    pub fn ready(&self) -> bool {
        let st = self.state.borrow();
        st.connected && st.updates.is_some() && st.bases.is_some()
    }

    /// What it shows, in words.
    pub fn dump(&self) -> String {
        self.draw();
        crate::panel::dump_labels(&self.root)
    }

    fn fetch_bases(&self) {
        let w = self.me.clone();
        self.link.call(
            |wadd| async move { wadd.bases().await },
            move |r| {
                let Some(m) = w.upgrade() else { return };
                match r {
                    Ok(b) => m.state.borrow_mut().bases = Some(b),
                    Err(e) => tracing::debug!("bases: {}", e.message),
                }
                m.queue_draw();
            },
        );
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

    /// A call to wadd (its button waits meanwhile); a refusal shows at the top.
    fn ask<T: Send + 'static, F>(&self, what: &str, call: impl FnOnce(wad_wadd_client::Wadd) -> F)
    where
        F: std::future::Future<Output = Result<T, Failure>> + Send + 'static,
    {
        let what = what.to_string();
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

    /// A button that asks wadd (`what`), waiting while it does.
    fn act<T: Send + 'static, F>(
        &self,
        b: &gtk::Button,
        what: &'static str,
        call: impl Fn(wad_wadd_client::Wadd) -> F + 'static,
    ) where
        F: std::future::Future<Output = Result<T, Failure>> + Send + 'static,
    {
        b.set_sensitive(!self.busy(what));
        let w = self.me.clone();
        let call = Rc::new(call);
        b.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                let call = call.clone();
                m.ask(what, move |wadd| call(wadd));
            }
        });
    }

    fn check_button(&self) -> gtk::Button {
        let b = kit::button("Check now", Variant::Secondary, Some("refresh-cw"), true);
        b.set_widget_name("update-check");
        self.act(&b, "check", |w| async move { w.update_check().await });
        b
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
                "Settings shows this machine's updates once wadd, which keeps them, answers.",
            ));
            return;
        }
        self.column.append(&self.note);
        if let Some(u) = &st.updates {
            self.column.append(&self.host(&st, u));
        }
        if let Some(b) = &st.bases {
            self.column.append(&self.bases(&st, b));
        }
    }

    fn host(&self, st: &State, u: &Updates) -> gtk::Box {
        let c = card("update-host");
        let now = now_ms();
        let job = st.host_job();
        let ph = phase(u, job);
        let checked = kit::ago(now, u.checked_at.unwrap_or(0));
        let title = if u.booted.version.is_empty() {
            "WadSpaces".to_string()
        } else {
            format!("WadSpaces {}", u.booted.version)
        };
        let check = (ph == Phase::Current).then(|| self.check_button());
        c.append(&head("monitor-cog", &title, &format!("Fedora 44 · bootc · Checked {checked}"), check.as_ref()));
        // Where a newer one is looked for: GitHub's CI builds, this
        // machine's own (the WadOS wadspace builds them), or the ones that
        // wadspace pushed to GitHub's registry.
        c.append(&self.source_row(u));
        if ph != Phase::Downloading {
            self.seen.set(None);
        }
        match ph {
            Phase::Current => c.append(&state_line(Some("check"), "WadSpaces is up to date.", true)),
            Phase::Checking => c.append(&state_line(
                None,
                &format!("Checking {} for a newer WadSpaces…", registry_of(&u.booted.image)),
                false,
            )),
            Phase::Downloading => {
                let j = job.expect("a job");
                let (done, total) = match u.progress {
                    Some(p) => (p.done_bytes, p.total_bytes),
                    None => (j.done_bytes, j.total_bytes),
                };
                let p = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
                let line = gtk::Box::builder().spacing(12).build();
                let what = u.available.as_ref().map(|a| a.version.clone()).unwrap_or_default();
                let l = kit::label(&format!("Downloading WadSpaces {what}"), &[]);
                l.set_hexpand(true);
                line.append(&l);
                let cancel = kit::button("Cancel", Variant::Ghost, None, true);
                cancel.set_widget_name("update-cancel-download");
                let id = j.id.clone();
                self.act(&cancel, "cancel-job", move |w| {
                    let id = id.clone();
                    async move { w.cancel_job(&id).await }
                });
                line.append(&cancel);
                p.append(&line);
                p.append(&progress(if total > 0 { done as f64 / total as f64 } else { 0.0 }));
                p.append(&small(&format!(
                    "{} of {} · {}",
                    bytes(done),
                    bytes(total),
                    left(self.seconds_left(done, total))
                )));
                c.append(&p);
            }
            Phase::Staging => {
                let j = job.expect("a job");
                let done = j
                    .stages
                    .get(j.cursor)
                    .map_or(0.0, |s| if s.state == PartState::Done { 1.0 } else { j.progress.clamp(0.0, 1.0) });
                let p = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
                p.append(&kit::label("Staging it for the next start…", &[]));
                p.append(&progress(done));
                p.append(&small("Writing the new deployment beside the running one."));
                c.append(&p);
            }
            Phase::Ready => {
                let s = u.staged.clone().expect("staged");
                let back = staged_is_back(u);
                let tucked = self.later.borrow().as_deref() == Some(s.version.as_str());
                if !tucked {
                    let later = kit::button("Later", Variant::Ghost, None, true);
                    let (w, v) = (self.me.clone(), s.version.clone());
                    later.connect_clicked(move |_| {
                        if let Some(m) = w.upgrade() {
                            *m.later.borrow_mut() = Some(v.clone());
                            m.queue_draw();
                        }
                    });
                    let restart = kit::button("Restart now", Variant::Primary, Some("power"), true);
                    restart.set_widget_name("update-restart");
                    let w = self.me.clone();
                    restart.connect_clicked(move |_| {
                        if let Some(m) = w.upgrade() {
                            m.restart();
                        }
                    });
                    c.append(&notice(
                        "accent",
                        if back { "rotate-ccw" } else { "download" },
                        &format!(
                            "<b>WadSpaces {} is ready:</b> it applies when this machine restarts.",
                            esc(&s.version)
                        ),
                        &[later, restart],
                    ));
                } else {
                    c.append(&state_line(
                        Some("clock"),
                        &format!("WadSpaces {} applies the next time this machine restarts.", s.version),
                        false,
                    ));
                }
                if back {
                    let from = release_date(&s.version).map(|d| format!(", from {d}")).unwrap_or_default();
                    c.append(&kit::label(
                        &format!(
                            "It goes back to {}{from}. {} is kept as the previous version, so you can come back to it.",
                            s.version, u.booted.version
                        ),
                        &["upd-aside"],
                    ));
                }
                let unstage = gtk::Button::builder().label("Don’t apply it").css_classes(["upd-link"]).build();
                unstage.set_halign(gtk::Align::Start);
                unstage.set_widget_name("update-unstage");
                self.act(&unstage, "unstage", |w| async move { w.update_cancel().await });
                c.append(&unstage);
            }
            Phase::Available => {
                let a = u.available.clone().expect("available");
                let get = kit::button("Download", Variant::Primary, Some("download"), true);
                get.set_widget_name("update-apply");
                self.act(&get, "apply", |w| async move { w.update_apply().await });
                c.append(&notice(
                    "accent",
                    "download",
                    &format!(
                        "<b>WadSpaces {} is available.</b> It downloads and gets ready for the next start.",
                        esc(&a.version)
                    ),
                    &[get],
                ));
            }
            Phase::Offline => {
                let again = kit::button("Try again", Variant::Quiet, None, true);
                self.act(&again, "check", |w| async move { w.update_check().await });
                c.append(&notice(
                    "warn",
                    "wifi-off",
                    "<b>This machine is offline.</b> Updates download when it’s back online.",
                    &[again],
                ));
            }
            Phase::Error => {
                // Retry carries on from where it failed: a download again, or a check.
                let downloading = st.last_host_job().is_some_and(|j| j.stages.iter().any(|s| s.id == "download"));
                let retry = kit::button("Retry", Variant::Secondary, Some("refresh-cw"), true);
                retry.set_widget_name("update-retry");
                if downloading && u.available.is_some() {
                    self.act(&retry, "apply", |w| async move { w.update_apply().await });
                } else {
                    self.act(&retry, "check", |w| async move { w.update_check().await });
                }
                let e = u.error.clone().unwrap_or_default();
                c.append(&notice(
                    "danger",
                    "circle-alert",
                    &format!("<b>The update didn’t download.</b> {}", esc(&e)),
                    &[retry],
                ));
            }
            Phase::Unmanaged => c.append(&state_line(
                Some("circle-alert"),
                u.error.as_deref().unwrap_or("This machine doesn't update through bootc"),
                false,
            )),
        }
        if let Some(r) = &u.rollback {
            let row = gtk::Box::builder().spacing(12).css_classes(["upd-prev"]).build();
            let ic = Icon::new("rotate-ccw", 16);
            ic.set_valign(gtk::Align::Center);
            row.append(&ic);
            let grow = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(3).hexpand(true).build();
            let t = gtk::Box::builder().spacing(6).build();
            t.append(&gtk::Label::new(Some("Previous version")));
            t.append(&gtk::Label::builder().label(&r.version).css_classes(["upd-code"]).build());
            grow.append(&t);
            grow.append(&small("Kept so this machine can go back to it."));
            row.append(&grow);
            if u.staged.as_ref().is_some_and(|s| same(s, Some(r))) {
                let tag =
                    gtk::Label::builder().label("Staged").css_classes(["upd-tag"]).valign(gtk::Align::Center).build();
                row.append(&tag);
            } else {
                let b = kit::button("Roll back", Variant::Quiet, None, true);
                b.set_widget_name("update-rollback");
                b.set_valign(gtk::Align::Center);
                b.set_sensitive(!self.busy("rollback"));
                let w = self.me.clone();
                b.connect_clicked(move |_| {
                    if let Some(m) = w.upgrade() {
                        m.confirm_rollback();
                    }
                });
                row.append(&b);
            }
            c.append(&row);
        }
        c
    }

    /// "Where updates come from": the three sources, the chosen one lit.
    fn source_row(self: &UpdatesPage, u: &Updates) -> gtk::Box {
        let row = gtk::Box::builder().spacing(6).css_classes(["upd-sources"]).build();
        let what = [
            (UpdateSource::Github, "GitHub", "The builds CI makes (ghcr.io/hunggod/wados-host)"),
            (UpdateSource::Local, "This machine", "What the WadOS wadspace builds here (host/build.sh image)"),
            (UpdateSource::Pushed, "Pushed to GitHub", "What the WadOS wadspace pushed to ghcr.io/hunggod/wados-host"),
        ];
        for (source, name, tip) in what {
            let b = kit::button(name, if u.source == source { Variant::Primary } else { Variant::Quiet }, None, true);
            b.set_tooltip_text(Some(tip));
            b.set_widget_name(&format!(
                "update-source-{}",
                match source {
                    UpdateSource::Github => "github",
                    UpdateSource::Local => "local",
                    UpdateSource::Pushed => "pushed",
                }
            ));
            if u.source != source {
                self.act(&b, "source", move |w| async move { w.update_source(source).await });
            }
            row.append(&b);
        }
        // What the other sources have, when it's known (one's own build is
        // known without a look: it's podman's, here).
        let mut note = vec![];
        if u.source != UpdateSource::Local
            && let Some(l) = &u.local
        {
            note.push(format!("This machine has {}", l.version));
        }
        if u.source != UpdateSource::Pushed
            && let Some(p) = &u.pushed
        {
            note.push(format!("Pushed: {}", p.version));
        }
        if u.source == UpdateSource::Local && u.local.is_none() {
            note.push("Nothing built here yet: build it in the WadOS wadspace".into());
        }
        if !note.is_empty() {
            row.append(&small(&note.join(" · ")));
        }
        row
    }

    fn seconds_left(&self, done: u64, total: u64) -> u64 {
        let now = now_ms();
        let rate = match self.seen.get() {
            Some((at, first)) if now > at + 1000 && done > first => {
                (done - first) as f64 / ((now - at) as f64 / 1000.0)
            }
            Some(_) => 44.0 * 1_048_576.0,
            None => {
                self.seen.set(Some((now, done)));
                44.0 * 1_048_576.0
            }
        };
        ((total.saturating_sub(done)) as f64 / rate).round().max(1.0) as u64
    }

    fn confirm_rollback(&self) {
        let Some(u) = self.state.borrow().updates.clone() else { return };
        let Some((title, text)) = rollback_text(&u) else { return };
        let w = self.me.clone();
        form::Dialog::new(&title)
            .icon("rotate-ccw", Tone::Warn)
            .text(&text)
            .button("cancel", "Cancel", Variant::Ghost)
            .button("rollback", "Roll back", Variant::Primary)
            .cancel("cancel")
            .named("update-rollback-dialog")
            .open(&self.window, move |_, id| {
                if id == "rollback"
                    && let Some(m) = w.upgrade()
                {
                    m.ask("rollback", |wadd| async move { wadd.update_rollback().await });
                }
            });
    }

    /// Restart now: logind's Reboot (interactive: polkit asks if it must).
    fn restart(&self) {
        if std::env::var_os("WADOS_LOGIND_DRY_RUN").is_some_and(|v| v == "1") {
            tracing::info!("restart: a dry run, so logind wasn't asked");
            self.note.set_label("Restart: a dry run, so logind wasn't asked");
            self.note.set_visible(true);
            return;
        }
        let w = self.me.clone();
        glib::spawn_future_local(async move {
            let r = async {
                let bus = gio::bus_get_future(gio::BusType::System).await?;
                bus.call_future(
                    Some("org.freedesktop.login1"),
                    "/org/freedesktop/login1",
                    "org.freedesktop.login1.Manager",
                    "Reboot",
                    Some(&(true,).to_variant()),
                    None,
                    gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
                    i32::MAX,
                )
                .await
            }
            .await;
            if let (Err(e), Some(m)) = (r, w.upgrade()) {
                m.note.set_label(&format!("Couldn't restart: {}", e.message()));
                m.note.set_visible(true);
            }
        });
    }

    fn bases(&self, st: &State, b: &Bases) -> gtk::Box {
        let c = card("update-bases");
        let now = now_ms();
        let pull = st.pull();
        let mut sub = "What every wadspace is built on. Refreshes once a day".to_string();
        if let (Some(at), None) = (b.next_refresh_at, pull) {
            sub.push_str(&format!(", next {}", until(now, at)));
        }
        sub.push('.');
        let refresh = kit::button("Refresh now", Variant::Secondary, Some("refresh-cw"), true);
        refresh.set_widget_name("bases-refresh");
        self.act(&refresh, "refresh", |w| async move { w.refresh_bases().await });
        if pull.is_some() {
            refresh.set_sensitive(false);
        }
        c.append(&head("layers", "Base images", &sub, Some(&refresh)));
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["upd-bases"]).build();
        list.set_overflow(gtk::Overflow::Hidden);
        for (i, base) in b.bases.iter().enumerate() {
            let s = pull.and_then(|j| j.stages.iter().find(|s| s.id == format!("base-{i}")));
            let row = gtk::Box::builder().spacing(14).css_classes(["upd-base"]).build();
            row.set_widget_name(&format!("base-{i}"));
            let grow = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(3).hexpand(true).build();
            grow.append(&kit::label(&base.name, &[]));
            grow.append(&gtk::Label::builder().label(&base.image).xalign(0.0).css_classes(["upd-code"]).build());
            row.append(&grow);
            let size = gtk::Label::builder()
                .label(base.size.map(bytes).unwrap_or_default())
                .xalign(1.0)
                .css_classes(["upd-small"])
                .valign(gtk::Align::Center)
                .build();
            size.set_size_request(64, -1);
            row.append(&size);
            let when = gtk::Box::builder().spacing(8).halign(gtk::Align::End).valign(gtk::Align::Center).build();
            when.set_size_request(150, -1);
            match s.map(|s| s.state) {
                Some(PartState::Working) => {
                    let s = s.expect("a stage");
                    let part = if s.bytes > 0 { s.done as f64 / s.bytes as f64 } else { 0.0 };
                    let p = progress(part);
                    p.set_size_request(100, -1);
                    when.append(&p);
                    when.append(&small(&format!("{}%", (part * 100.0).round() as u32)));
                }
                Some(PartState::Waiting) => when.append(&small("Waiting")),
                _ if base.missing => {
                    let l = small("Missing");
                    l.add_css_class("upd-missing");
                    when.append(&l);
                }
                _ => when.append(&small(&format!("Refreshed {}", kit::ago(now, base.refreshed_at.unwrap_or(0))))),
            }
            // At its end, as the spike's (justify-content: flex-end).
            if let Some(l) = when.first_child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
                l.set_hexpand(true);
                l.set_xalign(1.0);
            }
            when.set_hexpand(false);
            row.append(&when);
            list.append(&row);
        }
        c.append(&list);
        if let (Some(f), None) = (st.pull_failed(), pull) {
            let retry = kit::button("Retry", Variant::Secondary, Some("refresh-cw"), true);
            self.act(&retry, "refresh", |w| async move { w.refresh_bases().await });
            c.append(&notice(
                "danger",
                "circle-alert",
                &format!("<b>The base images didn’t refresh.</b> {}", esc(f.error.as_deref().unwrap_or_default())),
                &[retry],
            ));
        }
        c
    }
}

#[cfg(test)]
mod tests {
    //! The spike's `updates.test.ts`, as far as it's this page's to say.
    use std::cmp::Ordering;

    use serde_json::json;

    use super::*;

    fn updates(v: serde_json::Value) -> Updates {
        let mut base = json!({"online": true, "checking": false,
            "booted": {"version": "44.20261021.0", "image": "ghcr.io/hunggod/wados-host:latest", "digest": "sha256:b"}});
        if let (Some(b), Some(o)) = (base.as_object_mut(), v.as_object()) {
            b.extend(o.clone());
        }
        serde_json::from_value(base).unwrap()
    }

    fn job(stage: &str, status: &str) -> Job {
        serde_json::from_value::<JobInfo>(json!({
            "id": "j", "kind": "update", "status": status, "title": "t", "subject": "wados",
            "stages": [{"id": "check", "label": "Looking", "state": "done"},
                       {"id": stage, "label": "x", "state": "working"}],
            "cursor": 1, "progress": 0.5, "doneBytes": 5, "totalBytes": 10, "startedAt": 1,
            "source": "machine", "lineCount": 0
        }))
        .unwrap()
        .job
    }

    #[test]
    fn up_to_date_checking_downloading_then_ready() {
        let u = updates(json!({}));
        assert_eq!(phase(&u, None), Phase::Current);
        assert_eq!(phase(&u, Some(&job("check", "running"))), Phase::Checking);
        assert_eq!(phase(&u, Some(&job("download", "running"))), Phase::Downloading);
        assert_eq!(phase(&u, Some(&job("stage", "running"))), Phase::Staging);
        // A job that's over doesn't count.
        assert_eq!(phase(&u, Some(&job("download", "done"))), Phase::Current);
        let ready = updates(json!({"staged": {"version": "44.20261028.0", "image": "x", "digest": "sha256:n"}}));
        assert_eq!(phase(&ready, None), Phase::Ready);
        assert!(!staged_is_back(&ready));
    }

    #[test]
    fn offline_then_an_error_then_one_available() {
        assert_eq!(phase(&updates(json!({"online": false})), None), Phase::Offline);
        assert_eq!(phase(&updates(json!({"error": "manifest unknown"})), None), Phase::Error);
        let a = updates(json!({"available": {"version": "44.20261028.0", "image": "x"}}));
        assert_eq!(phase(&a, None), Phase::Available);
        let laptop: Updates = serde_json::from_value(
            json!({"online": false, "checking": false, "booted": {"version": "", "image": ""}, "error": "no bootc"}),
        )
        .unwrap();
        assert_eq!(phase(&laptop, None), Phase::Unmanaged);
    }

    #[test]
    fn rolling_back_is_said_as_going_back() {
        let before = json!({"version": "44.20261014.0", "image": "x", "digest": "sha256:r"});
        let u = updates(json!({"rollback": before, "staged": before}));
        assert_eq!(phase(&u, None), Phase::Ready);
        assert!(staged_is_back(&u));
        let (title, text) = rollback_text(&updates(json!({"rollback": before}))).unwrap();
        assert_eq!(title, "Roll back to 44.20261014.0?");
        assert_eq!(
            text,
            "WadSpaces 44.20261014.0 is staged and applies when this machine restarts. 44.20261021.0 stays as the previous version, so you can come back to it."
        );
        let staged = json!({"version": "44.20261028.0", "image": "x", "digest": "sha256:n"});
        let (_, text) = rollback_text(&updates(json!({"rollback": before, "staged": staged}))).unwrap();
        assert!(text.ends_with(" It replaces the staged 44.20261028.0."));
    }

    #[test]
    fn compares_versions_part_by_part() {
        assert_eq!(compare_versions("44.20261028.0", "44.20261021.0"), Ordering::Greater);
        assert_eq!(compare_versions("44.20261014.0", "44.20261021.0"), Ordering::Less);
        assert_eq!(compare_versions("44.20261021.0", "44.20261021.0"), Ordering::Equal);
        assert_eq!(compare_versions("45.20250101.0", "44.20261231.9"), Ordering::Greater);
    }

    #[test]
    fn dates_a_version() {
        assert_eq!(release_date("44.20261028.0").as_deref(), Some("28 October 2026"));
        assert_eq!(release_date("44.20261105.412").as_deref(), Some("5 November 2026"));
        assert_eq!(release_date("nope"), None);
        assert_eq!(release_date("44.20261328.0"), None);
    }

    #[test]
    fn says_when_the_next_refresh_is_and_how_much() {
        assert_eq!(until(0, 30_000), "soon");
        assert_eq!(until(0, 20 * 60_000), "in 20 min");
        assert_eq!(until(0, 5 * 3_600_000), "in 5 h");
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(600 * 1_048_576), "600 MB");
        assert_eq!(bytes(1_288_490_189), "1.20 GB");
        assert_eq!(left(40), "about 40 s left");
        assert_eq!(left(200), "about 3 min left");
        assert_eq!(registry_of("ghcr.io/hunggod/wados-host:latest"), "ghcr.io");
        assert_eq!(registry_of("10.0.2.2:5000/wados-host:latest"), "10.0.2.2:5000");
    }

    #[test]
    fn the_state_takes_its_events() {
        let mut s = State::default();
        let u = updates(json!({}));
        assert_eq!(s.apply(&Event::Updates(Box::new(u))), (true, false));
        let pull = |status: &str| {
            serde_json::from_value::<JobInfo>(json!({"id": "p", "kind": "pull", "status": status, "title": "t",
                "subject": "bases", "stages": [], "cursor": 0, "progress": 0.0, "doneBytes": 0, "totalBytes": 0, "startedAt": 1,
                "source": "machine", "lineCount": 0}))
            .unwrap()
        };
        assert_eq!(s.apply(&Event::Job { job: Box::new(pull("running")) }), (true, false));
        assert!(s.pull().is_some());
        // A refresh ended: the bases are asked for again.
        assert_eq!(s.apply(&Event::Job { job: Box::new(pull("error")) }), (true, true));
        assert!(s.pull().is_none() && s.pull_failed().is_some());
    }
}
