//! Settings ▸ Power (the spike's `panels/PowerPanel.svelte`):
//!
//! - **Battery** (when there's one): UPower's display device, its charge
//!   and how long it has left (or until it's full).
//! - **Power Mode**: power-profiles-daemon's profile (performance, balanced,
//!   power saver), over D-Bus; polkit lets the person at the machine change it.
//! - **Power Saving**: when the screen blanks and the machine suspends
//!   (the settings: the shell's idle watch picks them up at once), and what
//!   closing the lid does (the shell holds logind's lid switch for Lock
//!   the screen and Do nothing: no root needed).
//! - **Server Mode** comes in phase 10: shown, off, saying so.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::{glib, prelude::*};
use wad_desk::settings::{self, Lid, Settings};
use wad_gtk::Icon;
use wad_gtk::form::{Choice, opt};
use wad_gtk::kit::{self, Group, Row, Tone, Variant};

use crate::panel::{Ctx, Panel};
use crate::parts;

const CSS: &str = r#"
.pw-battery { padding: 16px; }
.pw-bic { min-width: 48px; min-height: 48px; border-radius: 14px; background-color: var(--accent-soft); color: var(--accent); }
.pw-bic.low { background-color: var(--danger-soft); color: var(--danger); }
.pw-pct { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 22px; }
.pw-line { font-size: 13px; color: var(--muted); }
progressbar.pw-bar > trough { min-height: 8px; border-radius: 4px; background-color: var(--surface-3); border: none; }
progressbar.pw-bar > trough > progress { min-height: 8px; border-radius: 4px; background-image: none; background-color: var(--accent); border: none; }
progressbar.pw-bar.low > trough > progress { background-color: var(--danger); }
"#;

/// power-profiles-daemon's names, new and old.
const PPD: [(&str, &str, &str); 2] = [
    (
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
        "org.freedesktop.UPower.PowerProfiles",
    ),
    ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles", "net.hadess.PowerProfiles"),
];

/// The modes, as the spike has them: (profile, title, subtitle, icon).
pub const MODES: [(&str, &str, &str, &str); 3] = [
    ("performance", "Performance", "High performance and power usage", "gauge"),
    ("balanced", "Balanced", "Standard performance and power usage", "scale"),
    ("power-saver", "Power Saver", "Reduced performance and power usage", "leaf"),
];

pub const SERVER_LATER: &str = "Server mode (the screen goes off and this machine only streams) comes in phase 10.";

/// The battery as UPower says it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Battery {
    /// 0 to 1.
    pub level: f64,
    /// UPower's state: 1 charging, 2 discharging, 4 full, 5 plugged in but not charging…
    pub state: u32,
    pub to_empty: i64,
    pub to_full: i64,
}

/// The line under the battery's percent (the spike's).
pub fn battery_line(b: &Battery) -> String {
    match b.state {
        1 if b.to_full > 0 => format!("Charging · full in about {}", parts::duration(b.to_full as f64 / 60.0)),
        1 => "Charging".into(),
        4 => "Fully charged".into(),
        5 => "Plugged in, not charging".into(),
        _ if b.to_empty > 0 => format!("On battery · about {} left", parts::duration(b.to_empty as f64 / 60.0)),
        _ => "On battery".into(),
    }
}

pub fn battery_icon(b: &Battery) -> &'static str {
    match b.state {
        1 | 4 | 5 => "battery-charging",
        _ if b.level < 0.2 => "battery-low",
        _ => "battery",
    }
}

/// What the Power Mode group says when a switch is refused.
pub fn refused_words(e: &str) -> String {
    format!("Couldn't change the power mode: {e}")
}

/// `wados-settings --power-mode <profile>`: the panel's switch without the
/// window (the VM's check): the same D-Bus call, the panel's words out.
pub fn mode_from_cli(profile: Option<&str>) -> glib::ExitCode {
    let Some(profile) = profile.filter(|p| MODES.iter().any(|m| m.0 == *p)) else {
        eprintln!("wados-settings --power-mode performance|balanced|power-saver");
        return glib::ExitCode::FAILURE;
    };
    let bus = match gtk::gio::bus_get_sync(gtk::gio::BusType::System, gtk::gio::Cancellable::NONE) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{}", refused_words(&parts::dbus_words(&e)));
            return glib::ExitCode::FAILURE;
        }
    };
    let mut last = String::new();
    for (name, path, iface) in PPD {
        let r = bus.call_sync(
            Some(name),
            path,
            "org.freedesktop.DBus.Properties",
            "Set",
            Some(&(iface, "ActiveProfile", profile.to_variant()).to_variant()),
            None,
            gtk::gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
            10_000,
            gtk::gio::Cancellable::NONE,
        );
        match r {
            Ok(_) => {
                println!("The power mode is {profile} now.");
                return glib::ExitCode::SUCCESS;
            }
            Err(e) => {
                last = parts::dbus_words(&e);
                // The old name's tried only when the new one isn't there.
                if last != "The service isn't running on this machine" {
                    break;
                }
            }
        }
    }
    eprintln!("{}", refused_words(&last));
    glib::ExitCode::FAILURE
}

pub fn lid_options() -> Vec<wad_gtk::form::Opt<Lid>> {
    vec![opt(Lid::Suspend, "Suspend"), opt(Lid::Lock, "Lock the screen"), opt(Lid::Nothing, "Do nothing")]
}

fn minutes(ns: &[u32]) -> Vec<wad_gtk::form::Opt<u32>> {
    settings::minute_options(ns).into_iter().map(|(l, n)| opt(n, &l)).collect()
}

pub fn make(_: &Ctx) -> Rc<dyn Panel> {
    Rc::new_cyclic(|me| Power {
        me: me.clone(),
        ui: RefCell::default(),
        ppd: RefCell::default(),
        refused: RefCell::default(),
    })
}

struct Ui {
    battery: Group,
    bic: gtk::Box,
    bicon: Icon,
    pct: gtk::Label,
    line: gtk::Label,
    bar: gtk::ProgressBar,
    modes: Vec<(&'static str, Row, gtk::Box)>,
    mode_note: gtk::Label,
}

pub struct Power {
    me: Weak<Power>,
    ui: RefCell<Option<Rc<Ui>>>,
    /// power-profiles-daemon's (name, path, interface) that answered.
    ppd: RefCell<Option<(&'static str, &'static str, &'static str)>>,
    /// Why the last switch was refused (shown until one goes through).
    refused: RefCell<Option<String>>,
}

impl Power {
    fn ui(&self) -> Option<Rc<Ui>> {
        self.ui.borrow().clone()
    }

    fn read_battery(&self) {
        let w = self.me.clone();
        parts::get_all(
            "org.freedesktop.UPower",
            "/org/freedesktop/UPower/devices/DisplayDevice",
            "org.freedesktop.UPower.Device",
            move |r| {
                let (Some(me), Some(ui)) = (w.upgrade(), w.upgrade().and_then(|m| m.ui())) else { return };
                let _ = me;
                let b = r.ok().and_then(|d| {
                    let present = d.lookup::<bool>("IsPresent").ok().flatten().unwrap_or(false);
                    let kind = d.lookup::<u32>("Type").ok().flatten().unwrap_or(0);
                    (present && kind == 2).then(|| Battery {
                        level: d.lookup::<f64>("Percentage").ok().flatten().unwrap_or(0.0) / 100.0,
                        state: d.lookup::<u32>("State").ok().flatten().unwrap_or(0),
                        to_empty: d.lookup::<i64>("TimeToEmpty").ok().flatten().unwrap_or(0),
                        to_full: d.lookup::<i64>("TimeToFull").ok().flatten().unwrap_or(0),
                    })
                });
                ui.battery.root.set_visible(b.is_some());
                let Some(b) = b else { return };
                let low = b.level < 0.2 && !matches!(b.state, 1 | 4 | 5);
                ui.bicon.set(battery_icon(&b));
                for w in [ui.bic.upcast_ref::<gtk::Widget>(), ui.bar.upcast_ref()] {
                    if low {
                        w.add_css_class("low");
                    } else {
                        w.remove_css_class("low");
                    }
                }
                ui.pct.set_label(&format!("{}%", (b.level * 100.0).round() as i64));
                ui.line.set_label(&battery_line(&b));
                ui.bar.set_fraction(b.level.clamp(0.0, 1.0));
            },
        );
    }

    /// power-profiles-daemon's profile now (trying its new name, then its old).
    fn read_mode(&self, i: usize) {
        let Some(&(name, path, iface)) = PPD.get(i) else {
            if let Some(ui) = self.ui() {
                ui.mode_note.set_label("Power modes need power-profiles-daemon, which isn't running on this machine.");
                ui.mode_note.set_visible(true);
                for (_, r, _) in &ui.modes {
                    r.root.set_sensitive(false);
                }
            }
            return;
        };
        let w = self.me.clone();
        parts::get_all(name, path, iface, move |r| {
            let Some(me) = w.upgrade() else { return };
            let Ok(d) = r else {
                me.read_mode(i + 1);
                return;
            };
            *me.ppd.borrow_mut() = Some((name, path, iface));
            let active = d.lookup::<String>("ActiveProfile").ok().flatten().unwrap_or_default();
            let offered: Vec<String> = d
                .lookup_value("Profiles", None)
                .map(|v| {
                    v.iter()
                        .filter_map(|p| glib::VariantDict::new(Some(&p)).lookup::<String>("Profile").ok().flatten())
                        .collect()
                })
                .unwrap_or_default();
            me.show_mode(&active, &offered);
        });
    }

    fn show_mode(&self, active: &str, offered: &[String]) {
        let Some(ui) = self.ui() else { return };
        for (p, row, radio) in &ui.modes {
            let on = *p == active;
            parts::set_radio(radio, on);
            parts::set_active(&row.root, on);
            if let Some(t) = row.root.first_child() {
                t.remove_css_class(if on { "plain" } else { "accent" });
                t.add_css_class(if on { "accent" } else { "plain" });
            }
            let there = offered.is_empty() || offered.iter().any(|o| o == p);
            row.root.set_sensitive(there);
            if !there {
                parts::set_subtitle(row, "Not available on this machine");
            }
        }
        match &*self.refused.borrow() {
            Some(e) => {
                ui.mode_note.set_label(e);
                ui.mode_note.set_visible(true);
            }
            None => ui.mode_note.set_visible(false),
        }
    }

    fn set_mode(&self, profile: &'static str) {
        let Some((name, path, iface)) = *self.ppd.borrow() else { return };
        let w = self.me.clone();
        parts::set_prop(name, path, iface, "ActiveProfile", profile.to_variant(), move |r| {
            let Some(me) = w.upgrade() else { return };
            *me.refused.borrow_mut() = r.err().map(|e| refused_words(&e));
            me.read_mode(0);
        });
    }
}

impl Panel for Power {
    fn build(&self, ctx: &Ctx) -> gtk::Widget {
        parts::install();
        if let Some(d) = gtk::gdk::Display::default() {
            wad_gtk::theme::add_css(&d, CSS);
        }
        let s = Settings::load();
        let page = kit::Page::new("Power", false);

        let battery = Group::new("Battery", "");
        let row = gtk::Box::builder().spacing(14).css_classes(["pw-battery"]).build();
        row.set_widget_name("battery");
        let bic = gtk::Box::builder().css_classes(["pw-bic"]).valign(gtk::Align::Center).hexpand(false).build();
        bic.set_size_request(48, 48);
        let bicon = Icon::new("battery", 26);
        bicon.set_hexpand(true);
        bicon.set_halign(gtk::Align::Center);
        bicon.set_valign(gtk::Align::Center);
        bic.append(&bicon);
        bic.set_hexpand(false);
        row.append(&bic);
        let bt = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        let line_box = gtk::Box::builder().spacing(10).build();
        let pct = gtk::Label::builder().css_classes(["pw-pct"]).build();
        let line = gtk::Label::builder().css_classes(["pw-line"]).valign(gtk::Align::End).build();
        line_box.append(&pct);
        line_box.append(&line);
        bt.append(&line_box);
        let bar = gtk::ProgressBar::builder().css_classes(["pw-bar"]).build();
        bt.append(&bar);
        row.append(&bt);
        battery.add(&row);
        battery.root.set_visible(false);
        page.add(&battery.root);

        let modes_g = Group::new("Power Mode", "Affects how fast this machine is, and how long the battery lasts.");
        let mut modes = vec![];
        for (p, title, sub, icon) in MODES {
            let r = Row::new(Some(icon), Tone::Plain, title, sub).named(&format!("mode-{p}"));
            let radio = parts::radio(false);
            r.ctl.append(&radio);
            let w = self.me.clone();
            parts::clickable(&r, move || {
                if let Some(me) = w.upgrade() {
                    me.set_mode(p);
                }
            });
            modes_g.add(&r.root);
            modes.push((p, r, radio));
        }
        let mode_note = parts::note("", "set-note");
        modes_g.root.append(&mode_note);
        page.add(&modes_g.root);

        let saving = Group::new("Power Saving", "");
        let blank = Row::new(
            Some("monitor"),
            Tone::Plain,
            "Automatic Screen Blank",
            "Turns the screen off after this long idle",
        );
        blank.ctl.append(
            &Choice::new(minutes(&[1, 2, 5, 10, 15]), s.power.blank_min, |v: &u32| {
                let v = *v;
                save(move |s| s.power.blank_min = v);
            })
            .named("blank")
            .root,
        );
        saving.add(&blank.root);
        let susp = Row::new(Some("moon"), Tone::Plain, "Automatic Suspend", "Pauses the machine after this long idle");
        susp.ctl.append(
            &Choice::new(minutes(&[15, 20, 30, 60]), s.power.suspend_min, |v: &u32| {
                let v = *v;
                save(move |s| s.power.suspend_min = v);
            })
            .named("suspend")
            .root,
        );
        saving.add(&susp.root);
        let lid = Row::new(Some("laptop"), Tone::Plain, "When the Lid Is Closed", "");
        lid.ctl.append(
            &Choice::new(lid_options(), s.power.lid, |v: &Lid| {
                let v = *v;
                save(move |s| s.power.lid = v);
            })
            .named("lid")
            .root,
        );
        saving.add(&lid.root);
        page.add(&saving.root);

        let server = Group::new("Server Mode", "");
        let (srow, sctl) = parts::stacked(
            Some("server"),
            Tone::Magenta,
            "Server Mode",
            "The screen goes off and this machine only streams: Ctrl+Alt+Delete brings it back.",
        );
        let b = kit::button("Turn On Server Mode…", Variant::Secondary, Some("power"), false);
        b.set_widget_name("server-mode");
        b.set_sensitive(false);
        b.set_tooltip_text(Some(SERVER_LATER));
        sctl.append(&b);
        sctl.append(&parts::note(SERVER_LATER, "set-note"));
        server.add(&srow);
        page.add(&server.root);
        let _ = ctx;

        *self.ui.borrow_mut() = Some(Rc::new(Ui { battery, bic, bicon, pct, line, bar, modes, mode_note }));
        self.read_battery();
        self.read_mode(0);
        let w = self.me.clone();
        glib::timeout_add_seconds_local(30, move || {
            let Some(me) = w.upgrade() else { return glib::ControlFlow::Break };
            me.read_battery();
            glib::ControlFlow::Continue
        });
        page.root.upcast()
    }
}

/// Changes a setting (the shell picks it up).
fn save(f: impl FnOnce(&mut Settings)) {
    if let Err(e) = Settings::update(f) {
        tracing::warn!("settings: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_battery_in_words() {
        let b = |state, level, to_empty, to_full| Battery { level, state, to_empty, to_full };
        assert_eq!(battery_line(&b(1, 0.5, 0, 3300)), "Charging · full in about 55 min");
        assert_eq!(battery_line(&b(2, 0.81, 4 * 3600 + 12 * 60, 0)), "On battery · about 4 h 12 min left");
        assert_eq!(battery_line(&b(2, 0.81, 0, 0)), "On battery");
        assert_eq!(battery_line(&b(4, 1.0, 0, 0)), "Fully charged");
        assert_eq!(battery_icon(&b(2, 0.1, 0, 0)), "battery-low");
        assert_eq!(battery_icon(&b(1, 0.1, 0, 0)), "battery-charging");
        assert_eq!(battery_icon(&b(2, 0.5, 0, 0)), "battery");
        assert_eq!(lid_options().len(), 3);
        assert_eq!(minutes(&[15, 20, 30, 60]).last().unwrap().label, "Never");
    }
}
