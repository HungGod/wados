//! The tray, at the bar's right end (`Tray.svelte`, `Flyout.svelte`,
//! `Notifications.svelte`): "show hidden icons", the jobs' ring while
//! something builds, the clipboard, Bluetooth, brightness, sound, the
//! network, the agents, the bell and the clock. Each opens its flyout by it:
//! over the dock on the launcher, under the bar on the desktop.
//!
//! Real where the machine says: wadd's jobs (with Cancel), this session's
//! notices (with Undo), the time, read-only the network, Bluetooth and the
//! battery, and the volume and the brightness as their keys leave them.
//! The rest says which phase brings it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::{glib, prelude::*};
use wad_gtk::{Icon, Svg};
use wad_model::JobStatus;
use wad_proto::JobInfo;

use super::notices::{ago, level_class, level_icon};
use super::shared::{Shared, Topic, now};
use super::system::{self, NetState};
use super::ui::{self, label, later, set_class};
use crate::shell::Shell;

const WIDTH: i32 = 332;

/// The flyouts, by name (what `wadsctl bar flyout <name>` opens).
pub const FLYOUTS: [&str; 10] =
    ["hidden", "jobs", "clipboard", "bluetooth", "brightness", "volume", "network", "agents", "notifications", "clock"];

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
    net_icon: Icon,
    bt_icon: Icon,
    vol_icon: Icon,
    bell_badge: gtk::Box,
    clock_t: gtk::Label,
    clock_d: gtk::Label,
    hidden_icon: Icon,
    resized: RefCell<Option<Rc<dyn Fn()>>>,
}

impl Tray {
    pub fn new(shell: &Rc<Shell>, shared: &Rc<Shared>, below: bool) -> Rc<Self> {
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
            net_icon: Icon::new("wifi", 17),
            bt_icon: Icon::new("bluetooth", 17),
            vol_icon: Icon::new("volume-2", 17),
            bell_badge: gtk::Box::builder()
                .css_classes(["wb-badge"])
                .halign(gtk::Align::End)
                .valign(gtk::Align::Start)
                .visible(false)
                .build(),
            clock_t: gtk::Label::builder().css_classes(["wb-clock-t"]).xalign(1.0).build(),
            clock_d: gtk::Label::builder().css_classes(["wb-clock-d"]).xalign(1.0).build(),
            hidden_icon: Icon::new(if below { "chevron-down" } else { "chevron-up" }, 16),
            resized: RefCell::default(),
        });

        me.add(
            "hidden",
            me.hidden_icon.clone().upcast(),
            "More: Tailscale, updates, activity, streams and the battery",
        );
        let ring = gtk::Box::builder().halign(gtk::Align::Center).valign(gtk::Align::Center).build();
        ring.append(&me.jobs_ring.area);
        me.add("jobs", ring.upcast(), "Downloads and builds");
        me.add("clipboard", Icon::new("clipboard-list", 17).upcast(), "Clipboard: what you've copied lately");
        me.add("bluetooth", me.bt_icon.clone().upcast(), "Bluetooth");
        me.add("brightness", Icon::new("sun-medium", 17).upcast(), "Brightness and night light");
        me.add("volume", me.vol_icon.clone().upcast(), "Sound");
        me.add("network", me.net_icon.clone().upcast(), "Network");
        me.add("agents", Icon::new("bot", 17).upcast(), "AI agents: the last runs");
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
        me.jobs_changed();
        me.system_changed();
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
            // The jobs' ring isn't there with nothing running: the hidden icons show the jobs.
            if name == "jobs" {
                return self.open_as("hidden", "jobs");
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

    fn system_changed(self: &Rc<Self>) {
        let st = self.shared.system.borrow().clone();
        self.net_icon.set(system::net_icon(st.network.as_ref()));
        if let Some(b) = self.button("network") {
            b.set_tooltip_text(Some(&system::net_tip(st.network.as_ref())));
        }
        self.bt_icon.set(if st.bluetooth.as_ref().is_some_and(|b| !b.on) { "bluetooth-off" } else { "bluetooth" });
        if let Some(b) = self.button("bluetooth") {
            b.set_tooltip_text(Some(&system::bluetooth_tip(st.bluetooth.as_ref())));
        }
        self.vol_icon.set(system::volume_icon(st.volume.as_ref()));
        if let Some(b) = self.button("volume") {
            b.set_tooltip_text(Some(&system::volume_tip(st.volume.as_ref())));
        }
        for n in ["network", "bluetooth", "brightness", "volume", "hidden"] {
            self.refill_open(n);
        }
    }

    // ------------------------------------------------------------ what each shows

    fn content(self: &Rc<Self>, what: &str) -> gtk::Widget {
        let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
        col.set_size_request(if what == "notifications" { 392 } else { WIDTH }, -1);
        let st = self.shared.system.borrow().clone();
        match what {
            "network" => {
                col.append(&head("Network", None));
                match &st.network {
                    Some(n) => {
                        let (icon, title, sub) = match n.state {
                            NetState::Connected => (
                                system::net_icon(Some(n)),
                                n.name.clone().unwrap_or_else(|| "Connected".into()),
                                match n.kind {
                                    system::NetKind::Wired => "Wired · connected",
                                    system::NetKind::Wifi => "Wi-Fi · connected",
                                    system::NetKind::Other => "Connected",
                                },
                            ),
                            NetState::Connecting => ("wifi", "Connecting…".into(), ""),
                            NetState::Asleep => ("wifi-off", "Asleep".into(), ""),
                            NetState::Offline => {
                                ("wifi-off", "Not connected".into(), if n.wifi_on { "" } else { "Wi-Fi is off" })
                            }
                        };
                        col.append(&row(icon, &title, sub, n.state == NetState::Connected));
                    }
                    None => col.append(&ui::label("NetworkManager isn't answering.", &["wb-p"])),
                }
                col.append(&later(
                    "Choosing a network, joining one and turning Wi-Fi off come with Settings (phase 8).",
                ));
            }
            "volume" => {
                col.append(&head("Sound", None));
                match &st.volume {
                    Some(v) => col.append(&row(
                        system::volume_icon(Some(v)),
                        if v.muted { "Muted" } else { "Volume" },
                        &format!("{}%", (v.level * 100.0).round() as i64),
                        !v.muted,
                    )),
                    None => col.append(&ui::label("The sound server (PipeWire) isn't answering.", &["wb-p"])),
                }
                col.append(&later("The volume keys change it. A slider and the outputs come with Settings (phase 8)."));
            }
            "brightness" => {
                col.append(&head("Display", None));
                match st.brightness {
                    Some(b) => {
                        col.append(&row("sun", "Brightness", &format!("{}%", (b * 100.0).round() as i64), false))
                    }
                    None => col.append(&ui::label("This screen has no brightness to read here.", &["wb-p"])),
                }
                col.append(&later(
                    "The brightness keys change it. A slider and night light come with Settings (phase 8).",
                ));
            }
            "bluetooth" => {
                col.append(&head("Bluetooth", None));
                match &st.bluetooth {
                    Some(b) if b.on => {
                        col.append(&row("bluetooth", "On", &format!("{} connected", b.connected), b.connected > 0))
                    }
                    Some(_) => col.append(&row("bluetooth-off", "Off", "", false)),
                    None => col.append(&ui::label("No Bluetooth on this machine.", &["wb-p"])),
                }
                col.append(&later("Pairing and connecting devices come with Settings (phase 8)."));
            }
            "clipboard" => {
                col.append(&head("Clipboard", None));
                col.append(&later("What you copy, kept here (the last 25), comes in phase 8."));
            }
            "agents" => {
                col.append(&head("AI agents", None));
                col.append(&later(
                    "Agents (Claude Code at work in your wadspaces, and what wants you) come in phase 11.",
                ));
            }
            "jobs" => self.jobs_content(&col),
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
            let title = label(&job.title, &["wb-job-t"]);
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

    fn notices_content(self: &Rc<Self>, col: &gtk::Box) {
        let header = gtk::Box::builder().spacing(10).css_classes(["wb-fly-head"]).build();
        let h = gtk::Box::builder().spacing(8).hexpand(true).build();
        h.append(&Icon::new("bell", 16));
        h.append(&label("Notifications", &["wb-h3"]));
        header.append(&h);
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
            ic.append(&Icon::new(level_icon(n.level), 14));
            ev.append(&ic);
            let title = label(&n.title, &["wb-ev-title"]);
            title.set_wrap(true);
            title.set_hexpand(true);
            title.set_valign(gtk::Align::Center);
            ev.append(&title);
            let side =
                gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).halign(gtk::Align::End).build();
            let when = label(&ago(t, n.at), &["wb-ev-time"]);
            when.set_xalign(1.0);
            side.append(&when);
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
        let tiles = [
            (
                tile(Svg::new(ui::TAILSCALE, 20, 20).upcast(), "Tailscale", "Phase 6"),
                None,
                "Tailscale: your tailnet comes in phase 6",
            ),
            (tile(Icon::new("download", 20).upcast(), "Updates", "Phase 8"), None, "Updates (bootc) come in phase 8"),
            (
                tile(Icon::new("activity", 20).upcast(), "Activity", &format!("{jobs} running")),
                Some("jobs"),
                "Activity: what's downloading and building",
            ),
            (tile(Icon::new("radio", 20).upcast(), "Streams", "Phase 7"), None, "Streams come in phase 7"),
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
            b.set_tooltip_text(Some(tip));
            match goes {
                Some("jobs") => {
                    let w = Rc::downgrade(self);
                    b.connect_clicked(move |_| {
                        if let Some(m) = w.upgrade() {
                            m.open_as("hidden", "jobs");
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
    fn sizes() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(12 * 1024), "12 KB");
        assert_eq!(bytes(3_565_158), "3.4 MB");
        assert_eq!(bytes(52_428_800), "50 MB");
        assert_eq!(bytes(1_288_490_189), "1.20 GB");
    }
}
