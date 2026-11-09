//! The tray, at the bar's right end (`Tray.svelte`, `Flyout.svelte`,
//! `Notifications.svelte`): "show hidden icons", the jobs' ring while
//! something builds, this machine's streams while it streams, the clipboard, Bluetooth, brightness, sound, the
//! network, the agents, the bell and the clock. Each opens its flyout by it:
//! over the dock on the launcher, under the bar on the desktop.
//!
//! Real where the machine says: wadd's jobs (with Cancel; a sign-in waiting
//! in the browser, with Show the browser), this session's notices (with
//! Undo), the time, the tailnet (More's Tailscale tile: how many machines,
//! a pip with how many devices ask to be trusted, Settings ▸ Accounts on a
//! click; the asks under it, with Deny and Trust), the streams (More's
//! Streams tile, how many are live; the Streams flyout: each one, its Link
//! and Stop), read-only the network,
//! Bluetooth and the battery, and the volume and the brightness as their
//! keys leave them. What isn't here yet says so.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::{glib, prelude::*};
use wad_gtk::{Icon, Svg};
use wad_model::{JobKind, JobStatus};
use wad_proto::{JobInfo, SignInProvider, Tailnet, TailnetAsk, TailnetState};

use super::notices::{ago, level_class};
use super::shared::{Shared, Topic, now};
use super::system;
use super::ui::{self, label, later, set_class};
use crate::shell::Shell;
use crate::store::Change;

/// The network and Bluetooth flyouts (wadd's).
mod radios;

/// What the Tailscale tile says under its name: how many of my machines are
/// online when it runs, else where it is (Settings' words).
pub fn tailscale_sub(t: Option<&Tailnet>, online: usize) -> String {
    match t.map(|t| t.state) {
        Some(TailnetState::Running) => format!("{online} machine{}", if online == 1 { "" } else { "s" }),
        Some(s) => state_word(s).into(),
        None => "Not answering".into(),
    }
}

/// What the Updates tile says under its name (the spike's Flyout).
pub fn updates_sub(u: Option<&wad_proto::Updates>) -> &'static str {
    match u {
        Some(u) if u.staged.is_some() => "Restart to update",
        Some(u) if u.checking => "Checking…",
        _ => "Up to date",
    }
}

/// The Updates tile's tooltip.
pub const UPDATES_TIP: &str = "Updates: check for one, or restart into it";

/// A tailnet state as Settings words it (`panels/tailnet.ts`).
/// The Agents icon's tooltip, and how many need you.
pub fn agents_tip(a: &wad_proto::Agents) -> (String, usize) {
    let need = a.agents.iter().filter(|x| x.needs_you()).count();
    let work = a.agents.iter().filter(|x| x.status == wad_model::agents::AgentStatus::Working).count();
    let tip = if need > 0 {
        format!("AI agents: {need} need{} you", if need == 1 { "s" } else { "" })
    } else if work > 0 {
        format!("AI agents: {work} working")
    } else {
        "AI agents: the last runs".to_string()
    };
    (tip, need)
}

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

/// The sign-in a job waits on in the browser, while its stage is held.
pub fn waiting_sign_in(j: &wad_model::Job) -> Option<SignInProvider> {
    if j.status != JobStatus::Running {
        return None;
    }
    let s = j.stages.get(j.cursor).filter(|s| s.held)?;
    [SignInProvider::Github, SignInProvider::Tailscale].into_iter().find(|p| s.label == p.held_label()).or(
        match j.kind {
            JobKind::Github => Some(SignInProvider::Github),
            JobKind::Tailnet => Some(SignInProvider::Tailscale),
            _ => None,
        },
    )
}

const WIDTH: i32 = 332;

/// The Tailscale tile's pip, and an ask (Flyout.svelte's `.pip`, `.ask`).
const CSS: &str = r#"
.wb-pip { min-width: 6px; min-height: 16px; padding: 0 5px; margin: 8px 8px 0 0; border-radius: 99px; background-color: var(--accent); color: var(--accent-fg); font-size: 10.5px; font-weight: 700; }
.wb-ask { margin-top: 6px; padding: 10px; border-radius: 12px; background-color: var(--warn-soft); border: 1px solid rgb(232 195 106 / 0.3); font-size: 13px; }
.wb-ask > wad-icon { color: var(--warn); }
"#;

thread_local! {
    static CSS_ADDED: Cell<bool> = const { Cell::new(false) };
}

/// The flyouts, by name (what `wadsctl bar flyout <name>` opens).
pub const FLYOUTS: [&str; 11] = [
    "hidden",
    "jobs",
    "streams",
    "clipboard",
    "bluetooth",
    "brightness",
    "volume",
    "network",
    "agents",
    "notifications",
    "clock",
];

struct Fly {
    name: &'static str,
    button: gtk::Button,
    pop: gtk::Popover,
    closed_at: Cell<Option<Instant>>,
}

pub struct Tray {
    pub root: gtk::Box,
    shell: Rc<Shell>,
    shared: Rc<Shared>,
    below: Cell<bool>,
    flys: RefCell<Vec<Rc<Fly>>>,
    /// What a flyout shows (the hidden icons' flyout can show the jobs').
    showing: RefCell<Vec<(&'static str, &'static str)>>,
    jobs_ring: Ring,
    streams_n: gtk::Label,
    net_icon: Icon,
    bt_icon: Icon,
    vol_icon: Icon,
    bell_badge: gtk::Box,
    hidden_badge: gtk::Box,
    clock_t: gtk::Label,
    clock_d: gtk::Label,
    hidden_icon: Icon,
    resized: RefCell<Option<Rc<dyn Fn()>>>,
    /// The sound and display flyouts (levels.rs).
    levels: Rc<super::levels::Levels>,
}

impl Tray {
    pub fn new(shell: &Rc<Shell>, shared: &Rc<Shared>, below: bool) -> Rc<Self> {
        if !CSS_ADDED.with(|c| c.replace(true))
            && let Some(d) = gtk::gdk::Display::default()
        {
            wad_gtk::kit::install(&d);
            wad_gtk::theme::add_css(&d, CSS);
            wad_gtk::theme::add_css(&d, radios::CSS);
        }
        let root = gtk::Box::builder().spacing(1).valign(gtk::Align::Center).build();
        let jobs_ring = Ring::new(18);
        let me = Rc::new(Self {
            root,
            shell: shell.clone(),
            shared: shared.clone(),
            below: Cell::new(below),
            flys: RefCell::default(),
            showing: RefCell::default(),
            jobs_ring,
            streams_n: gtk::Label::builder().css_classes(["st-tray-n"]).build(),
            net_icon: Icon::new("wifi", 17),
            bt_icon: Icon::new("bluetooth", 17),
            vol_icon: Icon::new("volume-2", 17),
            bell_badge: gtk::Box::builder()
                .css_classes(["wb-badge"])
                .halign(gtk::Align::End)
                .valign(gtk::Align::Start)
                .visible(false)
                .build(),
            hidden_badge: gtk::Box::builder()
                .css_classes(["wb-badge"])
                .halign(gtk::Align::End)
                .valign(gtk::Align::Start)
                .visible(false)
                .can_target(false)
                .build(),
            clock_t: gtk::Label::builder().css_classes(["wb-clock-t"]).xalign(1.0).build(),
            clock_d: gtk::Label::builder().css_classes(["wb-clock-d"]).xalign(1.0).build(),
            hidden_icon: Icon::new(if below { "chevron-down" } else { "chevron-up" }, 16),
            resized: RefCell::default(),
            levels: super::levels::Levels::new(shell),
        });

        let hidden = gtk::Overlay::new();
        hidden.set_child(Some(&me.hidden_icon));
        hidden.add_overlay(&me.hidden_badge);
        me.add("hidden", hidden.upcast(), "More: Tailscale, updates, activity, streams and the battery");
        let ring = gtk::Box::builder().halign(gtk::Align::Center).valign(gtk::Align::Center).build();
        ring.append(&me.jobs_ring.area);
        me.add("jobs", ring.upcast(), "Downloads and builds");
        let live = gtk::Box::builder().spacing(4).valign(gtk::Align::Center).build();
        live.append(&Icon::new("radio", 16));
        live.append(&me.streams_n);
        me.add("streams", live.upcast(), "Streams");
        if let Some(b) = me.button("streams") {
            b.add_css_class("st-tray");
            b.set_visible(false);
        }
        me.add("clipboard", Icon::new("clipboard-list", 17).upcast(), "Clipboard: what you've copied lately");
        me.add("bluetooth", me.bt_icon.clone().upcast(), "Bluetooth");
        me.add("brightness", Icon::new("sun-medium", 17).upcast(), "Brightness and night light");
        me.add("volume", me.vol_icon.clone().upcast(), "Sound");
        me.add("network", me.net_icon.clone().upcast(), "Network");
        let bot = gtk::Overlay::new();
        let bot_icon = Icon::new("bot", 17);
        bot_icon.set_size_request(18, 36);
        bot.set_child(Some(&bot_icon));
        let count = gtk::Label::builder()
            .css_classes(["wb-badge"])
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .visible(false)
            .build();
        bot.add_overlay(&count);
        me.add("agents", bot.upcast(), "AI agents: the last runs");
        let (w, c) = (Rc::downgrade(&me), count.clone());
        shell.store.watch(move |change| {
            if !matches!(change, crate::store::Change::Agents) {
                return;
            }
            let Some(m) = w.upgrade() else { return };
            let a = m.shell.store.agents();
            let (tip, need) = agents_tip(&a);
            c.set_label(&need.to_string());
            c.set_visible(need > 0);
            if let Some(b) = m.button("agents") {
                b.set_tooltip_text(Some(&tip));
            }
            m.refill_open("agents");
        });
        let bell = gtk::Overlay::new();
        let bell_icon = Icon::new("bell", 17);
        bell_icon.set_size_request(18, 36);
        bell.set_child(Some(&bell_icon));
        bell.add_overlay(&me.bell_badge);
        me.add("notifications", bell.upcast(), "Notifications");
        let clock = gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).build();
        clock.append(&me.clock_t);
        clock.append(&me.clock_d);
        me.add("clock", clock.upcast(), "");
        if let Some(b) = me.button("clock") {
            b.remove_css_class("wb-tb");
            b.add_css_class("wb-tb");
            b.add_css_class("wb-clock");
        }

        let w = Rc::downgrade(&me);
        shared.on(Topic::Jobs, move || {
            if let Some(m) = w.upgrade() {
                m.jobs_changed();
            }
        });
        let w = Rc::downgrade(&me);
        shared.on(Topic::System, move || {
            if let Some(m) = w.upgrade() {
                m.system_changed();
            }
        });
        let w = Rc::downgrade(&me);
        shared.on(Topic::Notices, move || {
            if let Some(m) = w.upgrade() {
                m.bell_badge.set_visible(m.shared.notices.borrow().unread() > 0);
                m.refill_open("notifications");
            }
        });
        let w = Rc::downgrade(&me);
        shell.store.watch(move |c| {
            let Some(m) = w.upgrade() else { return };
            if matches!(c, Change::Tailnet | Change::Peers | Change::Connected(_)) {
                m.tailnet_changed();
            }
            if matches!(c, Change::SignIn(_) | Change::PeerSpaces(_)) {
                m.refill_open("jobs");
                m.refill_open("hidden");
            }
            if matches!(c, Change::Streams | Change::PeerSpaces(_) | Change::Peers | Change::Library) {
                m.streams_changed();
            }
            // The Updates tile.
            if matches!(c, Change::Updates) {
                m.refill_open("hidden");
            }
            if matches!(c, Change::Network | Change::Bluetooth) {
                m.radios_changed();
            }
            // Focusing: the bell's countdown, and the bar's Focus button.
            if matches!(c, Change::Focus) {
                m.refill_open("notifications");
            }
        });
        me.jobs_changed();
        me.streams_changed();
        me.system_changed();
        me.tailnet_changed();
        let w = Rc::downgrade(&me);
        let tick = move || {
            let Some(m) = w.upgrade() else { return glib::ControlFlow::Break };
            m.tick();
            glib::ControlFlow::Continue
        };
        me.tick();
        glib::timeout_add_seconds_local(5, tick);
        me
    }

    fn add(self: &Rc<Self>, name: &'static str, child: gtk::Widget, tip: &str) {
        let button = gtk::Button::builder().child(&child).css_classes(["wb-tb"]).build();
        if !tip.is_empty() {
            button.set_tooltip_text(Some(tip));
        }
        let pop = ui::popover(&button, &[], self.below.get());
        let fly = Rc::new(Fly { name, button: button.clone(), pop: pop.clone(), closed_at: Cell::new(None) });
        let f = Rc::downgrade(&fly);
        pop.connect_closed(move |_| {
            if let Some(f) = f.upgrade() {
                f.closed_at.set(Some(Instant::now()));
                f.button.remove_css_class("wb-on");
            }
        });
        let (w, f) = (Rc::downgrade(self), Rc::downgrade(&fly));
        button.connect_clicked(move |_| {
            let (Some(me), Some(f)) = (w.upgrade(), f.upgrade()) else { return };
            // The click that closed it (outside it, on its button) doesn't open it again.
            if f.closed_at.get().is_some_and(|t| t.elapsed() < Duration::from_millis(250)) {
                return;
            }
            me.open(f.name);
        });
        self.root.append(&button);
        self.flys.borrow_mut().push(fly);
    }

    /// Closes whatever's open (a flyout's own button that leaves it).
    fn closer(self: &Rc<Self>) -> Rc<dyn Fn()> {
        let w = Rc::downgrade(self);
        Rc::new(move || {
            if let Some(m) = w.upgrade() {
                m.close_all();
            }
        })
    }

    fn fly(&self, name: &str) -> Option<Rc<Fly>> {
        self.flys.borrow().iter().find(|f| f.name == name).cloned()
    }

    fn button(&self, name: &str) -> Option<gtk::Button> {
        self.fly(name).map(|f| f.button.clone())
    }

    pub fn on_resize(&self, f: Rc<dyn Fn()>) {
        *self.resized.borrow_mut() = Some(f);
    }

    fn resized(&self) {
        if let Some(f) = self.resized.borrow().clone() {
            f();
        }
    }

    /// The dock (flyouts open upwards) or the top bar (downwards).
    pub fn set_below(&self, below: bool) {
        if self.below.replace(below) == below {
            return;
        }
        self.close_all();
        for f in self.flys.borrow().iter() {
            f.pop.set_position(if below { gtk::PositionType::Bottom } else { gtk::PositionType::Top });
            f.pop.set_offset(0, if below { 8 } else { -8 });
        }
        self.hidden_icon.set(if below { "chevron-down" } else { "chevron-up" });
    }

    pub fn close_all(&self) {
        for f in self.flys.borrow().iter() {
            f.pop.popdown();
        }
    }

    /// Opens a flyout (closing the others).
    pub fn open(self: &Rc<Self>, name: &str) -> bool {
        let Some(f) = self.fly(name) else { return false };
        if !f.button.is_visible() {
            // The jobs' ring (or the streams') isn't there with nothing running: the hidden icons show them.
            if name == "jobs" || name == "streams" {
                return self.open_as("hidden", name);
            }
            return false;
        }
        self.open_as(f.name, f.name)
    }

    fn open_as(self: &Rc<Self>, at: &str, what: &str) -> bool {
        let Some(f) = self.fly(at) else { return false };
        for o in self.flys.borrow().iter() {
            if o.name != at {
                o.pop.popdown();
            }
        }
        let what = FLYOUTS.into_iter().find(|n| *n == what).unwrap_or(f.name);
        {
            let mut s = self.showing.borrow_mut();
            s.retain(|(a, _)| *a != f.name);
            s.push((f.name, what));
        }
        f.pop.set_child(Some(&self.content(what)));
        f.button.add_css_class("wb-on");
        let below = self.below.get();
        ui::aim(&f.pop, &f.button, self.root.parent(), below, if below { 16 } else { 12 });
        f.pop.popup();
        if what == "notifications" {
            self.shared.notices.borrow_mut().read_all();
            self.shared.emit(Topic::Notices);
        }
        true
    }

    /// A flyout showing `what` is drawn again (its data changed).
    fn refill_open(self: &Rc<Self>, what: &str) {
        let open: Vec<&'static str> =
            self.showing.borrow().iter().filter(|(_, w)| *w == what).map(|(a, _)| *a).collect();
        for at in open {
            if let Some(f) = self.fly(at)
                && f.pop.is_visible()
            {
                f.pop.set_child(Some(&self.content(what)));
            }
        }
    }

    fn tick(self: &Rc<Self>) {
        let Ok(n) = glib::DateTime::now_local() else { return };
        let t = n.format("%-I:%M %P").map(|s| s.to_string()).unwrap_or_default();
        let d = n.format("%a, %-d %b").map(|s| s.to_string()).unwrap_or_default();
        let wider = t.len() != self.clock_t.label().len() || d.len() != self.clock_d.label().len();
        self.clock_t.set_label(&t);
        self.clock_d.set_label(&d);
        if let Some(b) = self.button("clock") {
            let long = n.format("%A, %-d %B").map(|s| s.to_string()).unwrap_or_default();
            b.set_tooltip_text(Some(&format!("{long}: the calendar")));
        }
        if wider {
            self.resized();
        }
    }

    fn jobs_changed(self: &Rc<Self>) {
        let jobs = self.shared.running_jobs();
        let Some(b) = self.button("jobs") else { return };
        let was = b.is_visible();
        b.set_visible(!jobs.is_empty());
        if !jobs.is_empty() {
            let p = jobs.iter().map(|j| j.job.progress).sum::<f64>() / jobs.len() as f64;
            self.jobs_ring.set(p);
            b.set_tooltip_text(Some(&format!("Downloads and builds: {} running", jobs.len())));
        }
        if was != b.is_visible() {
            if !b.is_visible()
                && let Some(f) = self.fly("jobs")
            {
                f.pop.popdown();
            }
            self.resized();
        }
        self.refill_open("jobs");
        self.refill_open("hidden");
    }

    /// This machine's streams: the tray's button (how many), and the flyouts.
    fn streams_changed(self: &Rc<Self>) {
        let mine = self.shell.store.streams().into_iter().filter(|s| s.peer.is_none()).count();
        if let Some(b) = self.button("streams") {
            let was = b.is_visible();
            b.set_visible(mine > 0);
            self.streams_n.set_label(&mine.to_string());
            b.set_tooltip_text(Some(&format!("Streams: {mine} from this machine")));
            if was
                && mine == 0
                && let Some(f) = self.fly("streams")
            {
                f.pop.popdown();
            }
            if was != b.is_visible() {
                self.resized();
            }
        }
        self.refill_open("streams");
        self.refill_open("hidden");
    }

    /// The asks: More's dot, and its flyout.
    fn tailnet_changed(self: &Rc<Self>) {
        let asks = self.asks().len();
        self.hidden_badge.set_visible(asks > 0);
        if let Some(b) = self.button("hidden") {
            b.set_tooltip_text(Some(&if asks > 0 {
                format!(
                    "More: {asks} device{} asking to be trusted, Tailscale, updates, activity, streams and the battery",
                    if asks == 1 { "" } else { "s" }
                )
            } else {
                "More: Tailscale, updates, activity, streams and the battery".into()
            }));
        }
        self.refill_open("hidden");
    }

    /// The devices asking to be trusted, while the tailnet runs.
    fn asks(&self) -> Vec<TailnetAsk> {
        self.shell.store.tailnet().filter(|t| t.state == TailnetState::Running).map(|t| t.asks).unwrap_or_default()
    }

    fn system_changed(self: &Rc<Self>) {
        let st = self.shared.system.borrow().clone();
        self.radio_icons(&st);
        self.vol_icon.set(system::volume_icon(st.volume.as_ref()));
        if let Some(b) = self.button("volume") {
            b.set_tooltip_text(Some(&system::volume_tip(st.volume.as_ref())));
        }
        // Sound's and Display's flyouts show it in place (a slider may be held).
        self.levels.update(&st);
        for n in ["network", "bluetooth", "hidden"] {
            self.refill_open(n);
        }
    }

    // ------------------------------------------------------------ what each shows

    fn content(self: &Rc<Self>, what: &str) -> gtk::Widget {
        let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
        col.set_size_request(if what == "notifications" { 392 } else { WIDTH }, -1);
        let st = self.shared.system.borrow().clone();
        match what {
            "network" => self.network_content(&col, &st),
            // levels.rs: the sliders, the outputs, night light.
            "volume" => self.levels.volume(&col, &st, self.closer()),
            "brightness" => self.levels.brightness(&col, &st, self.closer()),
            "bluetooth" => self.bluetooth_content(&col, &st),
            "clipboard" => {
                col.append(&head("Clipboard", None));
                col.append(&later("Your copied items aren't kept yet."));
            }
            "agents" => {
                self.agents_content(&col);
            }
            "jobs" => self.jobs_content(&col),
            "streams" => self.streams_content(&col),
            "notifications" => self.notices_content(&col),
            "clock" => self.clock_content(&col),
            _ => self.hidden_content(&col, &st),
        }
        col.upcast()
    }

    fn jobs_content(self: &Rc<Self>, col: &gtk::Box) {
        col.append(&head("Activity", None));
        let mut jobs: Vec<JobInfo> = self.shared.jobs.borrow().values().filter(|j| j.job.visible).cloned().collect();
        // Newest first, the last eight.
        jobs.sort_by_key(|j| std::cmp::Reverse(j.job.started_at));
        jobs.truncate(8);
        if jobs.is_empty() {
            col.append(&ui::label("Nothing's downloading or building.", &["wb-p"]));
            return;
        }
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
        for j in jobs {
            let job = &j.job;
            let b =
                gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).css_classes(["wb-job"]).build();
            let t = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
            let title = label(&crate::peers::job_title(&self.shell.store, &j), &["wb-job-t"]);
            title.set_wrap(true);
            t.append(&title);
            let (status, err) = match job.status {
                JobStatus::Queued => ("Waiting its turn".into(), false),
                JobStatus::Running => (job.stages.get(job.cursor).map(|s| s.label.clone()).unwrap_or_default(), false),
                JobStatus::Error => (job.error.clone().unwrap_or_else(|| "It didn't work".into()), true),
                JobStatus::Done => ("Done".into(), false),
                JobStatus::Cancelled => ("Cancelled".into(), false),
            };
            let s = label(&status, &["wb-small-t"]);
            s.set_wrap(true);
            if err {
                s.add_css_class("wb-err");
            }
            t.append(&s);
            b.append(&t);
            if job.status == JobStatus::Running {
                // Waiting for a sign-in in the browser: a way back to its window.
                if let Some(p) = waiting_sign_in(job) {
                    let s = self.shell.clone();
                    let w = Rc::downgrade(self);
                    let code = job.params.get("code").and_then(|c| c.as_str()).map(String::from);
                    let code = code.or_else(|| self.shell.store.sign_in(p).and_then(|s| s.code));
                    b.append(&wad_gtk::kit::in_browser(p.name(), code.as_deref(), move || {
                        if let Some(m) = w.upgrade() {
                            m.close_all();
                        }
                        crate::signin::show(&s, p);
                    }));
                }
                let p = gtk::ProgressBar::builder()
                    .fraction(job.progress.clamp(0.0, 1.0))
                    .css_classes(["wb-progress"])
                    .build();
                b.append(&p);
                if job.total_bytes > 0 {
                    b.append(&label(
                        &format!("{} of {}", bytes(job.done_bytes), bytes(job.total_bytes)),
                        &["wb-small-t"],
                    ));
                }
                let row = gtk::Box::builder().halign(gtk::Align::End).build();
                let cancel = gtk::Button::builder().label("Cancel").css_classes(["wb-ghost"]).build();
                let (s, id) = (self.shell.clone(), job.id.clone());
                cancel.connect_clicked(move |c| {
                    c.set_sensitive(false);
                    let id = id.clone();
                    let sh = s.clone();
                    s.link.call(
                        move |w| async move { w.cancel_job(&id).await },
                        move |r| {
                            if let Err(e) = r {
                                sh.toast(wad_proto::Level::Error, &format!("Couldn't cancel it: {}", e.message), None);
                            }
                        },
                    );
                });
                row.append(&cancel);
                b.append(&row);
            }
            list.append(&b);
        }
        col.append(&list);
    }

    /// The Streams flyout: each stream (here, and from my other machines), how it is, its Link and Stop.
    fn streams_content(self: &Rc<Self>, col: &gtk::Box) {
        col.append(&head("Streams", None));
        let store = &self.shell.store;
        let list = store.streams();
        if list.is_empty() {
            col.append(&ui::label(crate::streams::NOTHING, &["wb-p"]));
            return;
        }
        let rows = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
        for s in list {
            let key = crate::store::stream_key(&s);
            let r = gtk::Box::builder().spacing(10).css_classes(["st-row"]).build();
            r.set_widget_name(&format!("stream-{key}"));
            let dot = gtk::Box::builder().css_classes(["st-dot"]).valign(gtk::Align::Center).build();
            if !s.mode.is_public() {
                dot.add_css_class("priv");
            }
            r.append(&dot);
            let t = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
            t.append(&label(&crate::streams::name_of(store, &key), &["st-row-name"]));
            let peer = crate::streams::peer_name(store, &s);
            t.append(&label(&crate::streams::flyout_sub(&s, peer.as_deref()), &["wb-small-t"]));
            r.append(&t);
            if s.live {
                let link = wad_gtk::kit::button("Link", wad_gtk::kit::Variant::Ghost, Some("link"), true);
                link.set_valign(gtk::Align::Center);
                let (w, k) = (Rc::downgrade(self), key.clone());
                link.connect_clicked(move |_| {
                    if let Some(m) = w.upgrade() {
                        m.close_all();
                    }
                    crate::streams::share(&k);
                });
                r.append(&link);
            }
            let stop = wad_gtk::kit::button("Stop", wad_gtk::kit::Variant::Danger, None, true);
            stop.set_valign(gtk::Align::Center);
            stop.connect_clicked(move |b| {
                b.set_sensitive(false);
                crate::streams::stop(&key);
            });
            r.append(&stop);
            rows.append(&r);
        }
        col.append(&rows);
    }

    /// The last runs: what needs you first, with Review.
    fn agents_content(self: &Rc<Self>, col: &gtk::Box) {
        let a = self.shell.store.agents();
        col.append(&head("AI agents", None));
        let mut runs: Vec<_> = a.agents.iter().collect();
        runs.sort_by_key(|x| !x.needs_you());
        if runs.is_empty() {
            let l = label(
                "No runs yet. Send a bug, a feature or an update to a wadspace's Claude Code from Agents.",
                &["wb-muted"],
            );
            l.set_wrap(true);
            col.append(&l);
        }
        for r in runs.iter().take(6) {
            let b = gtk::Button::builder().css_classes(["wb-ghost"]).build();
            let c = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
            c.append(&label(&r.task, &[]));
            let s = a.sandboxes.iter().find(|s| Some(&s.id) == r.sandbox.as_ref());
            let line = wad_model::agents::wants(r, s).unwrap_or_else(|| r.now.clone());
            c.append(&label(&line, &["wb-muted"]));
            b.set_child(Some(&c));
            let (sh, id) = (self.shell.clone(), r.id.clone());
            b.connect_clicked(move |_| crate::peers::open_app(&sh, &["wados-agents", "--open", &id]));
            col.append(&b);
        }
        if runs.len() > 6 {
            col.append(&label(&format!("{} more in Agents", runs.len() - 6), &["wb-muted"]));
        }
        let open = gtk::Button::builder().label("Open Agents").css_classes(["wb-ghost"]).build();
        let sh = self.shell.clone();
        open.connect_clicked(move |_| crate::peers::open_app(&sh, &["wados-agents"]));
        col.append(&open);
    }

    fn notices_content(self: &Rc<Self>, col: &gtk::Box) {
        let header = gtk::Box::builder().spacing(10).css_classes(["wb-fly-head"]).build();
        let h = gtk::Box::builder().spacing(8).hexpand(true).build();
        h.append(&Icon::new("bell", 16));
        h.append(&label("Notifications", &["wb-h3"]));
        header.append(&h);
        // Focusing: how long is left, and what it is (the bar a full-screen
        // wadspace used to have is gone; this and the covers say it now).
        if let Some(f) = self.shell.store.focus() {
            let left = f.until - glib::real_time() / 1000;
            let timer = gtk::Button::builder()
                .label(format!("Focus · {} left", super::super::overlays::focus::left_words(left)))
                .css_classes(["wb-ghost"])
                .build();
            let s = self.shell.clone();
            timer.connect_clicked(move |_| s.focus_ui(|f| f.held()));
            header.append(&timer);
        }
        let empty = self.shared.notices.borrow().is_empty();
        if !empty {
            let clear = gtk::Button::builder().label("Clear").css_classes(["wb-ghost"]).build();
            let w = Rc::downgrade(self);
            clear.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.shared.notices.borrow_mut().clear();
                    m.shared.emit(Topic::Notices);
                }
            });
            header.append(&clear);
        }
        col.append(&header);
        let list = self.shared.notices.borrow().newest(14);
        if list.is_empty() {
            col.append(&ui::label("Nothing yet.", &["wb-p"]));
            return;
        }
        let t = now();
        for n in list {
            let ev = gtk::Box::builder().spacing(10).css_classes(["wb-ev", level_class(n.level)]).build();
            let ic = gtk::Box::builder().css_classes(["wb-eic"]).homogeneous(true).valign(gtk::Align::Start).build();
            ic.append(&Icon::new(n.icon(), 14));
            ev.append(&ic);
            let text = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(2)
                .hexpand(true)
                .valign(gtk::Align::Center)
                .build();
            let title = label(&n.title, &["wb-ev-title"]);
            title.set_wrap(true);
            title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            text.append(&title);
            if let Some(b) = &n.body {
                let body = label(b, &["wb-ev-body"]);
                body.set_wrap(true);
                body.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                text.append(&body);
            }
            ev.append(&text);
            let side =
                gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).halign(gtk::Align::End).build();
            let when = label(&ago(t, n.at), &["wb-ev-time"]);
            when.set_xalign(1.0);
            side.append(&when);
            if let Some(a) = n.action.clone().filter(|a| crate::verbs::known(&a.verb)) {
                let b = gtk::Button::builder().label(&a.label).css_classes(["wb-ghost"]).build();
                let (w, s) = (Rc::downgrade(self), self.shell.clone());
                b.connect_clicked(move |_| {
                    if let Some(m) = w.upgrade() {
                        m.close_all();
                    }
                    crate::verbs::run(&s, &a.verb, &a.args);
                });
                side.append(&b);
            }
            if let Some(undo) = n.undo.clone() {
                let b = gtk::Button::builder().label("Undo").css_classes(["wb-ghost"]).build();
                let (w, s) = (Rc::downgrade(self), self.shell.clone());
                b.connect_clicked(move |_| {
                    s.undo(&undo);
                    if let Some(m) = w.upgrade() {
                        m.shared.notices.borrow_mut().undone(&undo);
                        m.close_all();
                    }
                });
                side.append(&b);
            }
            ev.append(&side);
            col.append(&ev);
        }
    }

    fn clock_content(self: &Rc<Self>, col: &gtk::Box) {
        let Ok(n) = glib::DateTime::now_local() else { return };
        let head = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["wb-fly-head"]).build();
        head.append(&label(&n.format("%-I:%M %P").map(|s| s.to_string()).unwrap_or_default(), &["wb-h3"]));
        head.append(&label(&n.format("%A, %-d %B").map(|s| s.to_string()).unwrap_or_default(), &["wb-small-t"]));
        col.append(&head);
        let cal = gtk::Grid::builder()
            .column_homogeneous(true)
            .row_spacing(2)
            .column_spacing(2)
            .css_classes(["wb-cal"])
            .build();
        let name = gtk::Label::builder()
            .label(n.format("%B %Y").map(|s| s.to_string()).unwrap_or_default())
            .css_classes(["wb-cal-name"])
            .build();
        cal.attach(&name, 0, 0, 7, 1);
        for (i, d) in ["M", "T", "W", "T", "F", "S", "S"].iter().enumerate() {
            cal.attach(&gtk::Label::builder().label(*d).css_classes(["wb-dow"]).build(), i as i32, 1, 1, 1);
        }
        let (lead, days) = month_of(&n);
        for (i, c) in cells(lead, days).into_iter().enumerate() {
            if c == 0 {
                continue;
            }
            let l = gtk::Label::builder().label(c.to_string()).css_classes(["wb-day"]).build();
            if c == n.day_of_month() as u32 {
                l.add_css_class("wb-today");
            }
            cal.attach(&l, (i % 7) as i32, 2 + (i / 7) as i32, 1, 1);
        }
        col.append(&cal);
        let foot = gtk::Box::builder().halign(gtk::Align::End).css_classes(["wb-foot"]).build();
        let b = quiet("bell", "Notifications");
        let w = Rc::downgrade(self);
        b.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.open("notifications");
            }
        });
        foot.append(&b);
        col.append(&foot);
    }

    fn hidden_content(self: &Rc<Self>, col: &gtk::Box, st: &system::Status) {
        col.append(&head("More", None));
        let grid = gtk::Grid::builder().column_homogeneous(true).row_spacing(6).column_spacing(6).build();
        let tile = |icon: gtk::Widget, name: &str, sub: &str| {
            let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
            icon.set_halign(gtk::Align::Start);
            b.append(&icon);
            b.append(&label(name, &[]));
            b.append(&label(sub, &["wb-small-t"]));
            gtk::Button::builder().child(&b).css_classes(["wb-tile"]).build()
        };
        let jobs = self.shared.running_jobs().len();
        let unread = self.shared.notices.borrow().unread();
        let asks = self.asks();
        let online = self.shell.store.peers().iter().filter(|p| p.peer.online).count();
        let tailnet = self.shell.store.tailnet();
        let tailscale =
            tile(Svg::new(ui::TAILSCALE, 20, 20).upcast(), "Tailscale", &tailscale_sub(tailnet.as_ref(), online));
        let tailscale_tile = gtk::Overlay::new();
        tailscale_tile.set_child(Some(&tailscale));
        if !asks.is_empty() {
            let pip = label(&asks.len().to_string(), &["wb-pip"]);
            pip.set_halign(gtk::Align::End);
            pip.set_valign(gtk::Align::Start);
            pip.set_xalign(0.5);
            pip.set_can_target(false);
            tailscale_tile.add_overlay(&pip);
        }
        tailscale.set_tooltip_text(Some("Tailscale: your tailnet, its machines and who wants in"));
        let (w, s) = (Rc::downgrade(self), self.shell.clone());
        tailscale.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.close_all();
            }
            crate::peers::open_app(&s, &["wados-settings", "--page", "accounts"]);
        });
        grid.attach(&tailscale_tile, 0, 0, 1, 1);
        let tiles = [
            {
                // Updates (Settings' System ▸ Updates); a pip when one's staged.
                let u = self.shell.store.updates();
                let b = tile(Icon::new("download", 20).upcast(), "Updates", updates_sub(u.as_ref()));
                if u.is_some_and(|u| u.staged.is_some())
                    && let Some(inner) = b.child()
                {
                    b.set_child(None::<&gtk::Widget>);
                    let o = gtk::Overlay::new();
                    o.set_child(Some(&inner));
                    let pip = label("", &["wb-pip"]);
                    pip.set_halign(gtk::Align::End);
                    pip.set_valign(gtk::Align::Start);
                    pip.set_can_target(false);
                    o.add_overlay(&pip);
                    b.set_child(Some(&o));
                }
                (b, Some("updates"), UPDATES_TIP)
            },
            (
                tile(Icon::new("activity", 20).upcast(), "Activity", &format!("{jobs} running")),
                Some("jobs"),
                "Activity: what's downloading and building",
            ),
            (
                tile(
                    Icon::new("radio", 20).upcast(),
                    "Streams",
                    &crate::streams::tile_sub(&self.shell.store.streams()),
                ),
                Some("streams"),
                crate::streams::TILE_TIP,
            ),
            (
                tile(
                    Icon::new(st.battery.as_ref().map(system::battery_icon).unwrap_or("plug"), 20).upcast(),
                    "Battery",
                    &st.battery.as_ref().map(system::battery_text).unwrap_or_else(|| "No battery".into()),
                ),
                None,
                "The battery (UPower)",
            ),
            (
                tile(Icon::new("bell", 20).upcast(), "Notifications", &format!("{unread} new")),
                Some("notifications"),
                "Notifications",
            ),
        ];
        for (i, (b, goes, tip)) in tiles.into_iter().enumerate() {
            let i = i + 1;
            b.set_tooltip_text(Some(tip));
            match goes {
                Some("updates") => {
                    let (w, s) = (Rc::downgrade(self), self.shell.clone());
                    b.connect_clicked(move |_| {
                        if let Some(m) = w.upgrade() {
                            m.close_all();
                        }
                        crate::peers::open_app(&s, &["wados-settings", "--page", "updates"]);
                    });
                }
                Some(what @ ("jobs" | "streams")) => {
                    let w = Rc::downgrade(self);
                    b.connect_clicked(move |_| {
                        if let Some(m) = w.upgrade() {
                            m.open_as("hidden", what);
                        }
                    });
                }
                Some(other) => {
                    let (w, other) = (Rc::downgrade(self), other.to_string());
                    b.connect_clicked(move |_| {
                        if let Some(m) = w.upgrade() {
                            m.open(&other);
                        }
                    });
                }
                None => b.set_can_target(false),
            }
            grid.attach(&b, (i % 3) as i32, (i / 3) as i32, 1, 1);
        }
        col.append(&grid);
        // Who wants in: Deny, or Trust.
        for a in asks {
            col.append(&self.ask_row(&a));
        }
    }

    /// A device asking to be trusted (Flyout.svelte's `.ask`).
    fn ask_row(self: &Rc<Self>, a: &TailnetAsk) -> gtk::Box {
        let r = gtk::Box::builder().spacing(10).css_classes(["wb-ask"]).build();
        r.set_widget_name(&format!("ask-{}", a.id));
        let ic = Icon::new(crate::overlays::device_icon(a.kind), 18);
        ic.set_valign(gtk::Align::Center);
        r.append(&ic);
        let t = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        let what = gtk::Label::builder().xalign(0.0).wrap(true).build();
        what.set_markup(&format!("<b>{}</b> wants to manage this machine", glib::markup_escape_text(&a.device)));
        t.append(&what);
        t.append(&label(&format!("{} · {}", wad_gtk::kit::os_name(&a.os), a.ip), &["wb-small-t"]));
        r.append(&t);
        let deny = wad_gtk::kit::button("Deny", wad_gtk::kit::Variant::Ghost, None, true);
        let trust = wad_gtk::kit::button("Trust", wad_gtk::kit::Variant::Primary, None, true);
        for (b, yes) in [(&deny, false), (&trust, true)] {
            b.set_valign(gtk::Align::Center);
            let (s, id) = (self.shell.clone(), a.id.clone());
            b.connect_clicked(move |b| {
                b.set_sensitive(false);
                let (sh, id) = (s.clone(), id.clone());
                s.link.call(
                    move |w| async move { if yes { w.trust_device(&id).await } else { w.deny_ask(&id).await } },
                    move |r| {
                        if let Err(e) = r {
                            sh.toast(wad_proto::Level::Error, &e.message, None);
                        }
                    },
                );
            });
            r.append(b);
        }
        r
    }
}

fn head(title: &str, extra: Option<&gtk::Widget>) -> gtk::Box {
    let h = gtk::Box::builder().css_classes(["wb-fly-head"]).build();
    let t = label(title, &["wb-h3"]);
    t.set_hexpand(true);
    h.append(&t);
    if let Some(e) = extra {
        h.append(e);
    }
    h
}

/// A row of a flyout: its icon, a title and what's under it.
fn row(icon: &'static str, title: &str, sub: &str, on: bool) -> gtk::Box {
    let r = gtk::Box::builder().spacing(12).css_classes(["wb-row"]).build();
    r.append(&Icon::new(icon, 18));
    let t =
        gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).hexpand(true).build();
    t.append(&label(title, &[]));
    if !sub.is_empty() {
        t.append(&label(sub, &["wb-small-t"]));
    }
    r.append(&t);
    set_class(&r, "wb-on-row", on);
    r
}

fn quiet(icon: &'static str, text: &str) -> gtk::Button {
    let r = gtk::Box::builder().spacing(6).build();
    r.append(&Icon::new(icon, 14));
    r.append(&gtk::Label::new(Some(text)));
    gtk::Button::builder().child(&r).css_classes(["wb-quiet"]).build()
}

/// The first of the month's weekday (Monday 0) and how many days it has.
fn month_of(n: &glib::DateTime) -> (u32, u32) {
    let (y, m) = (n.year(), n.month());
    let first = glib::DateTime::from_local(y, m, 1, 0, 0, 0.0).map(|d| d.day_of_week() as u32 - 1).unwrap_or(0);
    let next = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    let days = glib::DateTime::from_local(next.0, next.1, 1, 0, 0, 0.0)
        .ok()
        .and_then(|d| d.add_days(-1).ok())
        .map(|d| d.day_of_month() as u32)
        .unwrap_or(30);
    (first, days)
}

/// A month's cells, Monday first: blanks (0) before the 1st, then each day.
pub fn cells(lead: u32, days: u32) -> Vec<u32> {
    std::iter::repeat_n(0, lead as usize).chain(1..=days).collect()
}

/// A size, as the spike says it ("12 KB", "3.4 MB", "1.20 GB").
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

/// A ring that fills as something goes (`ui/Ring.svelte`).
pub struct Ring {
    pub area: gtk::DrawingArea,
    value: Rc<Cell<f64>>,
}

impl Ring {
    pub fn new(size: i32) -> Self {
        let area = gtk::DrawingArea::builder().content_width(size).content_height(size).build();
        let value = Rc::new(Cell::new(0.0_f64));
        let v = value.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
            let r = cx.min(cy) - 2.0;
            cr.set_line_width(2.5);
            cr.set_source_rgba(240.0 / 255.0, 234.0 / 255.0, 250.0 / 255.0, 0.18);
            cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            let p = v.get().clamp(0.0, 1.0);
            if p > 0.0 {
                cr.set_source_rgb(198.0 / 255.0, 1.0, 31.0 / 255.0);
                cr.set_line_cap(gtk::cairo::LineCap::Round);
                let start = -std::f64::consts::FRAC_PI_2;
                cr.arc(cx, cy, r, start, start + p * std::f64::consts::TAU);
                let _ = cr.stroke();
            }
        });
        Self { area, value }
    }

    pub fn set(&self, p: f64) {
        if (self.value.replace(p) - p).abs() > f64::EPSILON {
            self.area.queue_draw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_month_starts_on_its_weekday() {
        // November 2026 starts on a Sunday: six blanks, then 1 to 30.
        let c = cells(6, 30);
        assert_eq!(c.len(), 36);
        assert_eq!(&c[..7], &[0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(c.last(), Some(&30));
        assert_eq!(cells(0, 28)[0], 1);
    }

    #[test]
    fn the_tailscale_tile_says_how_many_machines_or_where_it_is() {
        let mut t = Tailnet { state: TailnetState::Running, ..Tailnet::default() };
        assert_eq!(tailscale_sub(Some(&t), 1), "1 machine");
        assert_eq!(tailscale_sub(Some(&t), 2), "2 machines");
        t.state = TailnetState::NeedsLogin;
        assert_eq!(tailscale_sub(Some(&t), 2), "Not signed in");
        assert_eq!(tailscale_sub(None, 0), "Not answering");
        assert_eq!(state_word(TailnetState::LoggingIn), "Waiting for you to sign in");
    }

    #[test]
    fn the_updates_tile_says_whether_to_restart() {
        let mut u: wad_proto::Updates = serde_json::from_value(serde_json::json!({
            "online": true, "checking": true, "booted": {"version": "44.20261104.0", "image": "x"}
        }))
        .unwrap();
        assert_eq!(updates_sub(Some(&u)), "Checking…");
        u.staged = Some(u.booted.clone());
        assert_eq!(updates_sub(Some(&u)), "Restart to update");
        assert_eq!(updates_sub(None), "Up to date");
    }

    #[test]
    fn a_held_sign_in_stage_offers_the_browser() {
        use wad_model::{Job, JobStage, PartState};
        let stage = |id: &str, label: String, held: bool| JobStage {
            id: id.into(),
            label,
            state: PartState::Working,
            bytes: 0,
            done: 0,
            lines: vec![],
            held,
            error: None,
        };
        let mut j: Job = serde_json::from_value(serde_json::json!({
            "id": "j1", "kind": "github", "status": "running", "title": "Signing in to WadSpaces",
            "progress": 0.3, "cursor": 1, "startedAt": 1, "source": "machine", "visible": true
        }))
        .unwrap();
        j.stages = vec![
            stage("code", "Asking GitHub for a code".into(), false),
            stage("signin", SignInProvider::Github.held_label(), true),
        ];
        assert_eq!(waiting_sign_in(&j), Some(SignInProvider::Github));
        // Not held (yet, or any more), or the job's over: nothing to show.
        j.stages[1].held = false;
        assert_eq!(waiting_sign_in(&j), None);
        j.stages[1].held = true;
        j.status = JobStatus::Done;
        assert_eq!(waiting_sign_in(&j), None);
        // A Tailnet job's own words.
        j.status = JobStatus::Running;
        j.stages[1].label = SignInProvider::Tailscale.held_label();
        assert_eq!(waiting_sign_in(&j), Some(SignInProvider::Tailscale));
    }

    #[test]
    fn sizes() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(12 * 1024), "12 KB");
        assert_eq!(bytes(3_565_158), "3.4 MB");
        assert_eq!(bytes(52_428_800), "50 MB");
        assert_eq!(bytes(1_288_490_189), "1.20 GB");
    }
}
