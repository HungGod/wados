//! Settings ▸ System (the spike's `panels/SystemPanel.svelte`): an overview
//! whose rows open its pages (System's `sub`):
//!
//! - **About**: the machine's name (wadd's) and hostname, its hardware
//!   (/proc, /sys, hwdata's pci.ids) and software (os-release; WadSpaces'
//!   version as wadd's updates say it's booted).
//! - **Software Updates**: the Updates agent's page (`updates.rs`'s
//!   [`updates_page`](super::updates::updates_page)), which System hands
//!   wadd's events to; its row says where the update is.
//! - **Date & Time**: timedated, over D-Bus (automatic time, the time
//!   zone; polkit may say no, and the page says so), and the clock's
//!   format (the settings).
//! - **Users**: who's signed in, read-only; the password's in Lock Screen & Privacy.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::{glib, prelude::*};
use wad_desk::settings::Settings;
use wad_gtk::form::{Choice, Segmented, Toggle, opt};
use wad_gtk::kit::{self, Group, Row, Tone, Variant};
use wad_proto::{Event, MachineInfo, Updates};
use wad_wadd_client::Incoming;

use super::updates::{UpdatesPage, updates_page};
use crate::panel::{Ctx, Panel, dump_labels};
use crate::parts;

const CSS: &str = r#"
.sys-hero { padding: 8px 0 0; }
.sys-logo { min-width: 76px; min-height: 76px; border-radius: 22px; background-image: linear-gradient(145deg, var(--surface-3), var(--bg-2)); box-shadow: inset 0 0 0 1px var(--line-strong), 0 0 40px -10px var(--accent); color: var(--accent); }
.sys-avatar { min-width: 84px; min-height: 84px; border-radius: 50%; background-color: var(--accent-soft); color: var(--accent); font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 700; font-size: 36px; box-shadow: inset 0 0 0 2px var(--accent); }
.sys-mono { font-family: monospace; color: var(--muted); }
"#;

const TIMEDATE: (&str, &str, &str) =
    ("org.freedesktop.timedate1", "/org/freedesktop/timedate1", "org.freedesktop.timedate1");

/// The time zones offered (the spike's), and the one set if it's another.
pub const ZONES: [&str; 12] = [
    "Australia/Melbourne",
    "Australia/Sydney",
    "Australia/Brisbane",
    "Australia/Perth",
    "Pacific/Auckland",
    "Asia/Tokyo",
    "Asia/Singapore",
    "Europe/London",
    "Europe/Berlin",
    "America/New_York",
    "America/Los_Angeles",
    "UTC",
];

/// "Australia · Melbourne", "America · New York".
pub fn zone_label(z: &str) -> String {
    z.replace('_', " ").replacen('/', " · ", 1)
}

/// "GMT+11", "GMT+5:30", "GMT" (as Intl's shortOffset).
pub fn offset_label(secs: i64) -> String {
    if secs == 0 {
        return "GMT".into();
    }
    let sign = if secs < 0 { '-' } else { '+' };
    let (h, m) = (secs.abs() / 3600, secs.abs() % 3600 / 60);
    if m == 0 { format!("GMT{sign}{h}") } else { format!("GMT{sign}{h}:{m:02}") }
}

fn zone_offset(z: &str) -> String {
    let tz = glib::TimeZone::from_identifier(Some(z)).unwrap_or_else(glib::TimeZone::utc);
    let off = glib::DateTime::now(&tz).map(|d| d.utc_offset().as_seconds()).unwrap_or(0);
    offset_label(off)
}

/// "4:05 pm", "16:05".
pub fn clock(d: &glib::DateTime, h24: bool) -> String {
    let f = if h24 { "%H:%M" } else { "%-I:%M %P" };
    d.format(f).map(|s| s.to_string()).unwrap_or_default()
}

/// The Software Updates row's line (the spike's `updateLine`).
pub fn update_line(u: &Updates, now_ms: i64) -> String {
    if u.checking {
        return "Checking for updates…".into();
    }
    if let Some(s) = &u.staged {
        return format!("{} is ready: restart to update", version_of(s));
    }
    if let Some(a) = u.available.as_ref().filter(|a| version_of(a) != version_of(&u.booted)) {
        return format!("{} is available", version_of(a));
    }
    match u.checked_at {
        Some(at) => format!("Up to date · checked {}", kit::ago(now_ms, at)),
        None => "Up to date".into(),
    }
}

fn version_of(v: &wad_proto::Version) -> String {
    if v.version.is_empty() { v.image.clone() } else { v.version.clone() }
}

/// os-release's KEY.
pub fn os_release(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=').map(|v| v.trim().trim_matches('"').to_string()))
}

/// /proc/cpuinfo's model, "× threads".
pub fn processor(cpuinfo: &str) -> Option<String> {
    let model = cpuinfo
        .lines()
        .find_map(|l| l.strip_prefix("model name")?.split_once(':').map(|(_, v)| v.trim().to_string()))?;
    let n = cpuinfo.lines().filter(|l| l.starts_with("processor")).count();
    let model = model
        .replace("(R)", "")
        .replace("(TM)", "")
        .replace(" CPU ", " ")
        .split(" @ ")
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    Some(if n > 1 { format!("{model} × {n}") } else { model })
}

/// /proc/meminfo's total, in GB as it's sold ("16 GB").
pub fn memory(meminfo: &str) -> Option<String> {
    let kb: f64 = meminfo.lines().find_map(|l| l.strip_prefix("MemTotal:")?.split_whitespace().next()?.parse().ok())?;
    let gb = kb / 1024.0 / 1024.0;
    // What's usable is a little under what's fitted: round up to a size it'd be sold as.
    let sold = [2.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0, 96.0, 128.0].into_iter().find(|s| gb <= *s);
    Some(format!("{} GB", sold.unwrap_or(gb.round()) as i64))
}

/// A PCI device's name in hwdata's pci.ids ("8086", "5917").
pub fn pci_name(ids: &str, vendor: &str, device: &str) -> Option<String> {
    let mut in_vendor = false;
    let mut vname = None;
    for l in ids.lines() {
        if l.starts_with('#') || l.is_empty() {
            continue;
        }
        if !l.starts_with('\t') {
            if in_vendor {
                break;
            }
            if let Some(rest) = l.strip_prefix(vendor) {
                in_vendor = true;
                vname = Some(rest.trim().to_string());
            }
            continue;
        }
        if in_vendor && !l.starts_with("\t\t") && l[1..].starts_with(device) {
            let d = l[1 + device.len()..].trim();
            // "UHD Graphics 620 [Kaby Lake]": the part in brackets is the family's.
            let d = d.split(" [").next().unwrap_or(d).trim();
            let v = vname.clone().unwrap_or_default();
            let v = v.split_whitespace().next().unwrap_or("");
            return Some(format!("{v} {d}").trim().to_string());
        }
    }
    vname
}

fn graphics() -> String {
    let mut names = vec![];
    let Ok(dir) = std::fs::read_dir("/sys/class/drm") else { return "Unknown".into() };
    let ids = ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let mut cards: Vec<_> = dir
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("card") && !n.contains('-'))
        .collect();
    cards.sort();
    for c in cards {
        let dev = std::path::Path::new("/sys/class/drm").join(&c).join("device");
        let read =
            |f: &str| std::fs::read_to_string(dev.join(f)).ok().map(|s| s.trim().trim_start_matches("0x").to_string());
        let name = match (read("vendor"), read("device")) {
            (Some(v), Some(d)) => pci_name(&ids, &v, &d),
            _ => None,
        }
        .or_else(|| {
            std::fs::read_to_string(dev.join("uevent"))
                .ok()?
                .lines()
                .find_map(|l| l.strip_prefix("DRIVER=").map(str::to_string))
        });
        if let Some(n) = name
            && !names.contains(&n)
        {
            names.push(n);
        }
    }
    if names.is_empty() { "Unknown".into() } else { names.join(", ") }
}

/// The biggest disk that isn't removable (sectors of 512 bytes in /sys/block).
fn disk() -> String {
    let Ok(dir) = std::fs::read_dir("/sys/block") else { return "Unknown".into() };
    let size = dir
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            !["loop", "ram", "zram", "dm-", "sr", "md"].iter().any(|p| n.starts_with(p))
        })
        .filter(|e| std::fs::read_to_string(e.path().join("removable")).is_ok_and(|r| r.trim() == "0"))
        .filter_map(|e| std::fs::read_to_string(e.path().join("size")).ok()?.trim().parse::<u64>().ok())
        .max();
    match size {
        Some(s) => format!("{:.1} GB", s as f64 * 512.0 / 1e9),
        None => "Unknown".into(),
    }
}

fn firmware() -> String {
    if !std::path::Path::new("/sys/firmware/efi").exists() {
        return "BIOS".into();
    }
    let sb = std::fs::read_dir("/sys/firmware/efi/efivars").ok().and_then(|d| {
        d.flatten()
            .find(|e| e.file_name().to_string_lossy().starts_with("SecureBoot-"))
            .and_then(|e| std::fs::read(e.path()).ok())
            .and_then(|b| b.get(4).copied())
    });
    match sb {
        Some(1) => "UEFI, Secure Boot on".into(),
        Some(_) => "UEFI, Secure Boot off".into(),
        None => "UEFI".into(),
    }
}

fn kata() -> String {
    for p in ["/usr/bin/kata-runtime", "/usr/local/bin/kata-runtime"] {
        if let Ok(o) = std::process::Command::new(p).arg("--version").output()
            && o.status.success()
        {
            let s = String::from_utf8_lossy(&o.stdout);
            return s
                .split_whitespace()
                .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))
                .unwrap_or("Installed")
                .to_string();
        }
    }
    "Not available".into()
}

/// Who's signed in: login, full name, an administrator (in wheel).
fn me() -> (String, String, bool) {
    let login = glib::user_name().to_string_lossy().into_owned();
    let full = glib::real_name().to_string_lossy().into_owned();
    let full = if full.is_empty() || full == "Unknown" { login.clone() } else { full };
    let admin = std::fs::read_to_string("/etc/group").ok().is_some_and(|g| {
        g.lines().any(|l| {
            let f: Vec<&str> = l.split(':').collect();
            f.first() == Some(&"wheel") && f.get(3).is_some_and(|m| m.split(',').any(|u| u == login))
        })
    });
    (login, full, admin)
}

/// The people with accounts here besides `login` (uid 1000 up, with a shell to sign in to).
pub fn others(passwd: &str, login: &str) -> Vec<(String, String)> {
    passwd
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            let uid: u32 = f.get(2)?.parse().ok()?;
            let shell = f.get(6).copied().unwrap_or("");
            let real = (1000..60000).contains(&uid)
                && !shell.ends_with("nologin")
                && !shell.ends_with("false")
                && f[0] != login;
            real.then(|| {
                (
                    f[0].to_string(),
                    f.get(4).and_then(|g| g.split(',').next()).filter(|g| !g.is_empty()).unwrap_or(f[0]).to_string(),
                )
            })
        })
        .collect()
}

pub fn make(ctx: &Ctx) -> Rc<dyn Panel> {
    let updates = updates_page(ctx);
    Rc::new_cyclic(|me| System { me: me.clone(), updates, machine: RefCell::default(), ui: RefCell::default() })
}

struct Ui {
    stack: gtk::Stack,
    about_row: Row,
    updates_row: Row,
    time_row: Row,
    device: gtk::Label,
    version: gtk::Label,
    version_hero: gtk::Label,
    big_time: gtk::Label,
    date: gtk::Label,
    auto: Toggle,
    zone_row: Row,
    zone: Choice<String>,
    time_note: gtk::Label,
}

pub struct System {
    me: Weak<System>,
    updates: Rc<UpdatesPage>,
    machine: RefCell<Option<MachineInfo>>,
    ui: RefCell<Option<Rc<Ui>>>,
}

impl System {
    fn ui(&self) -> Option<Rc<Ui>> {
        self.ui.borrow().clone()
    }

    fn go(&self, sub: &str) {
        if let Some(ui) = self.ui() {
            ui.stack.set_visible_child_name(sub);
            if sub == "datetime" {
                self.read_time();
            }
        }
    }

    fn version(&self) -> String {
        let u = self.updates.state.borrow().updates.clone();
        u.map(|u| version_of(&u.booted))
            .filter(|v| !v.is_empty())
            .or_else(|| self.machine.borrow().as_ref().map(|m| m.version.clone()))
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
    }

    fn name(&self) -> String {
        self.machine.borrow().as_ref().map(|m| m.name.clone()).unwrap_or_else(|| glib::host_name().to_string())
    }

    /// The overview's lines, from what's known now.
    fn lines(&self) {
        let Some(ui) = self.ui() else { return };
        let v = self.version();
        parts::set_subtitle(&ui.about_row, &format!("{} · WadSpaces {v}", self.name()));
        ui.device.set_label(&self.name());
        ui.version.set_label(&v);
        ui.version_hero.set_label(&format!("WadSpaces {v}"));
        let line = match self.updates.state.borrow().updates.clone() {
            Some(u) => format!("WadSpaces {v} · {}", update_line(&u, glib::real_time() / 1000)),
            None => format!("WadSpaces {v}"),
        };
        parts::set_subtitle(&ui.updates_row, &line);
        let s = Settings::load();
        let now = glib::DateTime::now_local().ok();
        let tz = std::fs::read_link("/etc/localtime")
            .ok()
            .and_then(|p| p.to_string_lossy().split("zoneinfo/").nth(1).map(str::to_string))
            .unwrap_or_else(|| "UTC".into());
        if let Some(n) = &now {
            parts::set_subtitle(&ui.time_row, &format!("{} · {}", clock(n, s.clock.h24), zone_label(&tz)));
            ui.big_time.set_label(&clock(n, s.clock.h24));
            ui.date.set_label(&n.format("%A %-d %B %Y").map(|s| s.to_string()).unwrap_or_default());
        }
    }

    fn read_time(&self) {
        let w = self.me.clone();
        let (n, p, i) = TIMEDATE;
        parts::get_all(n, p, i, move |r| {
            let Some(me) = w.upgrade() else { return };
            let Some(ui) = me.ui() else { return };
            match r {
                Ok(d) => {
                    let zone = d.lookup::<String>("Timezone").ok().flatten().unwrap_or_else(|| "UTC".into());
                    ui.auto.set(d.lookup::<bool>("NTP").ok().flatten().unwrap_or(false));
                    ui.auto.root.set_sensitive(d.lookup::<bool>("CanNTP").ok().flatten().unwrap_or(true));
                    let mut zones: Vec<String> = ZONES.iter().map(|z| z.to_string()).collect();
                    if !zones.contains(&zone) {
                        zones.insert(0, zone.clone());
                    }
                    ui.zone.set_options(
                        zones.iter().map(|z| opt(z.clone(), &zone_label(z)).hint(&zone_offset(z))).collect::<Vec<_>>(),
                    );
                    ui.zone.set(zone.clone());
                    parts::set_subtitle(&ui.zone_row, &format!("{} ({})", zone_label(&zone), zone_offset(&zone)));
                }
                Err(e) => {
                    ui.time_note.set_label(&format!("timedated isn't answering: {e}"));
                    ui.time_note.set_visible(true);
                }
            }
            me.lines();
        });
    }

    fn time_call(&self, method: &'static str, args: glib::Variant) {
        let w = self.me.clone();
        let (n, p, i) = TIMEDATE;
        parts::call(n, p, i, method, args, true, move |r| {
            let Some(me) = w.upgrade() else { return };
            if let (Err(e), Some(ui)) = (r, me.ui()) {
                ui.time_note.set_label(&format!("Couldn't change it: {e}"));
                ui.time_note.set_visible(true);
            }
            me.read_time();
        });
    }
}

fn hero(icon_or_letter: gtk::Widget, title: &gtk::Label, sub: &str, mono: bool) -> gtk::Box {
    let h = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .halign(gtk::Align::Center)
        .css_classes(["sys-hero"])
        .build();
    icon_or_letter.set_halign(gtk::Align::Center);
    h.append(&icon_or_letter);
    title.set_halign(gtk::Align::Center);
    title.add_css_class("set-hero-title");
    h.append(title);
    let s = gtk::Label::builder()
        .label(sub)
        .halign(gtk::Align::Center)
        .css_classes([if mono { "sys-mono" } else { "set-hero-sub" }])
        .build();
    h.append(&s);
    h
}

fn value_row(title: &str, value: &str, mono: bool) -> Row {
    Row::new(None, Tone::Plain, title, "").value(value, mono)
}

impl Panel for System {
    fn build(&self, ctx: &Ctx) -> gtk::Widget {
        parts::install();
        if let Some(d) = gtk::gdk::Display::default() {
            wad_gtk::theme::add_css(&d, CSS);
        }
        let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::None).hexpand(true).build();

        // The overview.
        let main = kit::Page::new("System", false);
        let g = Group::new("", "");
        let about_row = Row::new(Some("info"), Tone::Plain, "About", "").named("sys-about");
        let updates_row = Row::new(Some("download"), Tone::Plain, "Software Updates", "").named("sys-updates");
        let time_row = Row::new(Some("clock"), Tone::Plain, "Date & Time", "").named("sys-datetime");
        let (login, full, admin) = me();
        let users_row = Row::new(
            Some("users"),
            Tone::Plain,
            "Users",
            &format!("{full} ({login}){}", if admin { ", administrator" } else { "" }),
        )
        .named("sys-users");
        for (r, sub) in
            [(&about_row, "about"), (&updates_row, "updates"), (&time_row, "datetime"), (&users_row, "users")]
        {
            parts::nav(r, false);
            let w = self.me.clone();
            parts::clickable(r, move || {
                if let Some(me) = w.upgrade() {
                    me.go(sub);
                }
            });
            g.add(&r.root);
        }
        main.add(&g.root);
        stack.add_named(&main.root, Some("main"));

        // About.
        let about = kit::Page::new("About", true);
        let os = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let base = os_release(&os, "PRETTY_NAME").unwrap_or_else(|| "Linux".into());
        let logo = kit::icon_tile("layout-grid", Tone::Accent, 76, 34);
        logo.remove_css_class("kit-ic");
        logo.add_css_class("sys-logo");
        let version_hero = gtk::Label::new(None);
        about.add(&hero(logo.upcast(), &version_hero, &base, false));
        let id = Group::new("", "");
        let dev = value_row("Device Name", "", false);
        let device = dev.ctl.first_child().and_downcast::<gtk::Label>().expect("a value");
        id.add(&dev.root);
        id.add(&value_row("Hostname", &glib::host_name(), true).root);
        about.add(&id.root);
        let hw = Group::new("Hardware", "");
        let cpu = std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|c| processor(&c))
            .unwrap_or_else(|| "Unknown".into());
        let mem =
            std::fs::read_to_string("/proc/meminfo").ok().and_then(|m| memory(&m)).unwrap_or_else(|| "Unknown".into());
        hw.add(&value_row("Processor", &cpu, false).root);
        hw.add(&value_row("Memory", &mem, false).root);
        hw.add(&value_row("Graphics", &graphics(), false).root);
        hw.add(&value_row("Disk Capacity", &disk(), false).root);
        about.add(&hw.root);
        let sw = Group::new("Software", "");
        let ver = value_row("WadSpaces", "", true);
        let version = ver.ctl.first_child().and_downcast::<gtk::Label>().expect("a value");
        sw.add(&ver.root);
        sw.add(&value_row("Base", &base, false).root);
        sw.add(&value_row("Kata Containers", &kata(), false).root);
        sw.add(&value_row("Firmware", &firmware(), false).root);
        about.add(&sw.root);
        stack.add_named(&about.root, Some("about"));

        // Date & Time.
        let dt = kit::Page::new("Date & Time", true);
        let s = Settings::load();
        let big_time = gtk::Label::builder().css_classes(["set-big-time"]).build();
        big_time.set_widget_name("clock");
        let dh = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .halign(gtk::Align::Center)
            .css_classes(["sys-hero"])
            .build();
        dh.append(&big_time);
        let date = gtk::Label::builder().css_classes(["set-hero-sub"]).build();
        dh.append(&date);
        dt.add(&dh);
        let tg = Group::new("", "");
        let auto_row = Row::new(Some("globe"), Tone::Plain, "Automatic Date & Time", "Set from the internet");
        let w = self.me.clone();
        let auto = Toggle::new(false, move |on| {
            if let Some(me) = w.upgrade() {
                me.time_call("SetNTP", (on, true).to_variant());
            }
        })
        .named("auto-time");
        auto_row.ctl.append(&auto.root);
        tg.add(&auto_row.root);
        let zone_row = Row::new(Some("compass"), Tone::Plain, "Time Zone", "");
        let w = self.me.clone();
        let zone = Choice::new(Vec::<wad_gtk::form::Opt<String>>::new(), String::new(), move |z: &String| {
            if let Some(me) = w.upgrade() {
                me.time_call("SetTimezone", (z.as_str(), true).to_variant());
            }
        })
        .named("timezone");
        zone_row.ctl.append(&zone.root);
        tg.add(&zone_row.root);
        let fmt = Row::new(Some("clock"), Tone::Plain, "Time Format", "");
        let w = self.me.clone();
        fmt.ctl.append(
            &Segmented::new([opt(false, "AM / PM"), opt(true, "24-hour")], s.clock.h24, true, move |h: &bool| {
                let h = *h;
                if let Err(e) = Settings::update(|s| s.clock.h24 = h) {
                    tracing::warn!("settings: {e}");
                }
                if let Some(me) = w.upgrade() {
                    me.lines();
                }
            })
            .named("time-format")
            .root,
        );
        tg.add(&fmt.root);
        dt.add(&tg.root);
        let time_note = parts::note("", "set-note");
        dt.add(&time_note);
        stack.add_named(&dt.root, Some("datetime"));

        // Users.
        let users = kit::Page::new("Users", true);
        let avatar = gtk::Label::builder()
            .label(full.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default())
            .css_classes(["sys-avatar"])
            .build();
        users.add(&hero(avatar.upcast(), &gtk::Label::new(Some(&full)), &login, true));
        let ug = Group::new("", "");
        ug.add(
            &Row::new(
                Some(if admin { "shield-check" } else { "user" }),
                if admin { Tone::Accent } else { Tone::Plain },
                if admin { "Administrator" } else { "Standard Account" },
                if admin {
                    "Created at first boot (there’s no admin baked into the image any more)"
                } else {
                    "It can't change the machine's settings"
                },
            )
            .root,
        );
        let pw = Row::new(Some("key-round"), Tone::Plain, "Password", "Change it in Lock Screen & Privacy");
        parts::nav(&pw, false);
        let c = ctx.clone();
        parts::clickable(&pw, move || c.open("lock"));
        ug.add(&pw.root);
        users.add(&ug.root);
        let og = Group::new("Other Users", "");
        let rest = others(&std::fs::read_to_string("/etc/passwd").unwrap_or_default(), &login);
        if rest.is_empty() {
            let r = Row::new(
                None,
                Tone::Plain,
                "Nobody else yet",
                "Adding users comes later: there's one account, made at first boot",
            );
            let b = kit::button("Add User…", Variant::Secondary, Some("user-round-plus"), true);
            b.set_sensitive(false);
            b.set_tooltip_text(Some("Adding users comes later"));
            r.ctl.append(&b);
            og.add(&r.root);
        }
        for (l, n) in rest {
            og.add(&Row::new(Some("user"), Tone::Plain, &n, &l).root);
        }
        users.add(&og.root);
        stack.add_named(&users.root, Some("users"));

        // Software Updates: the Updates agent's page.
        stack.add_named(&self.updates.root, Some("updates"));

        for p in [&about, &dt, &users] {
            if let Some(b) = &p.back {
                let st = stack.clone();
                b.connect_clicked(move |_| st.set_visible_child_name("main"));
            }
        }
        if let Some(b) = &self.updates.back {
            let w = self.me.clone();
            b.connect_clicked(move |_| {
                if let Some(me) = w.upgrade() {
                    me.go("main");
                    me.lines();
                }
            });
        }

        *self.ui.borrow_mut() = Some(Rc::new(Ui {
            stack: stack.clone(),
            about_row,
            updates_row,
            time_row,
            device,
            version,
            version_hero,
            big_time,
            date,
            auto,
            zone_row,
            zone,
            time_note,
        }));
        self.lines();
        let w = self.me.clone();
        glib::timeout_add_seconds_local(5, move || {
            let Some(me) = w.upgrade() else { return glib::ControlFlow::Break };
            if me.ui().is_some_and(|u| u.stack.is_mapped()) {
                me.lines();
            }
            glib::ControlFlow::Continue
        });
        stack.upcast()
    }

    fn on_event(&self, _: &Ctx, event: &Incoming) {
        self.updates.on_event(event);
        if let Incoming::Event(Event::Machine(m)) = event {
            *self.machine.borrow_mut() = Some(m.clone());
        }
        if matches!(event, Incoming::Event(Event::Machine(_) | Event::Updates(_))) {
            self.lines();
        }
    }

    fn show(&self, sub: Option<&str>) {
        self.go(sub.unwrap_or("main"));
    }

    fn ready(&self) -> bool {
        match self.ui().and_then(|u| u.stack.visible_child_name()).as_deref() {
            Some("updates") => self.updates.ready(),
            _ => true,
        }
    }

    fn dump(&self, page: &gtk::Widget) -> String {
        match self.ui().and_then(|u| u.stack.visible_child_name()).as_deref() {
            Some("updates") => self.updates.dump(),
            _ => self
                .ui()
                .and_then(|u| u.stack.visible_child())
                .map(|c| dump_labels(&c))
                .unwrap_or_else(|| dump_labels(page)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_zones_in_words() {
        assert_eq!(zone_label("America/New_York"), "America · New York");
        assert_eq!(zone_label("UTC"), "UTC");
        assert_eq!(offset_label(11 * 3600), "GMT+11");
        assert_eq!(offset_label(-(5 * 3600 + 1800)), "GMT-5:30");
        assert_eq!(offset_label(0), "GMT");
        let d = glib::DateTime::from_utc(2026, 11, 5, 16, 5, 0.0).unwrap();
        assert_eq!(clock(&d, true), "16:05");
        assert_eq!(clock(&d, false), "4:05 pm");
    }

    #[test]
    fn the_machine_in_words() {
        assert_eq!(
            os_release("NAME=\"Fedora Linux\"\nPRETTY_NAME=\"Fedora Linux 44 (bootc)\"\n", "PRETTY_NAME"),
            Some("Fedora Linux 44 (bootc)".into())
        );
        let cpu =
            "processor\t: 0\nmodel name\t: Intel(R) Core(TM) i7-8650U CPU @ 1.90GHz\nprocessor\t: 1\nmodel name\t: x\n";
        assert_eq!(processor(cpu), Some("Intel Core i7-8650U × 2".into()));
        assert_eq!(memory("MemTotal:       16169468 kB\n"), Some("16 GB".into()));
        assert_eq!(memory("MemTotal:        8038232 kB\n"), Some("8 GB".into()));
        let ids = "# x\n8086  Intel Corporation\n\t5917  UHD Graphics 620 [Kaby Lake]\n\t\t1414 0020  sub\n10de  NVIDIA Corporation\n\t1c8d  GP107M [GeForce GTX 1050 Mobile]\n";
        assert_eq!(pci_name(ids, "8086", "5917"), Some("Intel UHD Graphics 620".into()));
        assert_eq!(pci_name(ids, "10de", "1c8d"), Some("NVIDIA GP107M".into()));
        assert_eq!(pci_name(ids, "8086", "ffff"), Some("Intel Corporation".into()));
        let pw = "root:x:0:0:root:/root:/bin/bash\nsam:x:1000:1000:Sam Wad:/home/sam:/bin/bash\nkid:x:1001:1001::/home/kid:/bin/bash\nsvc:x:1002:1002::/:/sbin/nologin\n";
        assert_eq!(others(pw, "sam"), [("kid".to_string(), "kid".to_string())]);
    }

    #[test]
    fn the_update_line() {
        let v = |s: &str| wad_proto::Version { version: s.into(), ..Default::default() };
        let mut u = Updates { booted: v("44.1"), ..Default::default() };
        assert_eq!(update_line(&u, 0), "Up to date");
        u.checked_at = Some(0);
        assert!(update_line(&u, 120_000).starts_with("Up to date · checked "));
        u.available = Some(v("44.2"));
        assert_eq!(update_line(&u, 0), "44.2 is available");
        u.staged = Some(v("44.2"));
        assert_eq!(update_line(&u, 0), "44.2 is ready: restart to update");
        u.checking = true;
        assert_eq!(update_line(&u, 0), "Checking for updates…");
    }
}
