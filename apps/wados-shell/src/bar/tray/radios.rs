//! The tray's network and Bluetooth flyouts (`Flyout.svelte`'s), from
//! wadd's `network` and `bluetooth` events (the store's): the Wi-Fi switch
//! and the networks in range (a click joins one; a secure one not joined
//! before asks for its password in place), Disconnect, the wire; the
//! Bluetooth switch and its devices (a click connects or disconnects a
//! paired one; one not paired is paired in Settings ▸ Bluetooth, where what
//! pairing asks is shown). Each ends with "… settings", opening Settings
//! at its page. A wadd without them (a laptop's): what NetworkManager and
//! BlueZ say, read-only (system.rs).

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use wad_gtk::Icon;
use wad_gtk::form::{TextField, Toggle};
use wad_proto::{Bluetooth, BtDevice, BtKind, Network, WifiNetwork};

use super::{Tray, head, quiet, row};
use crate::bar::system::{self, NetState};
use crate::bar::ui::{self, label, set_class};

/// The spike's flyout bits: a network's join form, its error.
pub const CSS: &str = r#"
.wb-join { padding: 4px 10px 8px 42px; }
.wb-join .form-input { min-height: 32px; border-radius: 8px; }
.wb-join-err { font-size: 12px; color: var(--danger); }
.wb-row .wb-lock { color: var(--muted); }
"#;

/// The tray's network icon (Tray.svelte's `wifiIcon`).
pub fn net_icon(n: &Network) -> &'static str {
    if n.ethernet.plugged {
        "cable"
    } else if !n.wifi_on || n.airplane || n.connected.is_none() {
        "wifi-off"
    } else {
        "wifi"
    }
}

/// Its tooltip (`netTip`).
pub fn net_tip(n: &Network) -> String {
    if n.ethernet.plugged {
        "Network: wired".into()
    } else if n.airplane {
        "Network: airplane mode".into()
    } else if !n.wifi_on {
        "Network: Wi-Fi is off".into()
    } else if let Some(s) = &n.connected {
        format!("Network: on {s}")
    } else {
        "Network: not connected".into()
    }
}

pub fn bt_tip(b: &Bluetooth) -> String {
    if b.on {
        format!("Bluetooth: {} connected", b.devices.iter().filter(|d| d.connected).count())
    } else {
        "Bluetooth: off".into()
    }
}

/// A network's bars (the spike's thresholds).
pub fn signal_icon(s: u8) -> &'static str {
    if s > 66 {
        "wifi"
    } else if s > 33 {
        "wifi-high"
    } else {
        "wifi-low"
    }
}

/// What a device's row says under its name.
pub fn device_sub(d: &BtDevice) -> String {
    let mut s = String::from(if d.connected {
        "Connected"
    } else if d.paired {
        "Paired"
    } else {
        "Not paired"
    });
    if let (true, Some(b)) = (d.connected, d.battery) {
        s.push_str(&format!(" · {b}%"));
    }
    s
}

pub fn device_icon(k: BtKind) -> &'static str {
    match k {
        BtKind::Headphones => "headphones",
        BtKind::Mouse => "mouse",
        BtKind::Keyboard => "keyboard",
        BtKind::Phone => "smartphone",
        BtKind::Speaker => "speaker",
        BtKind::Controller => "gamepad-2",
        BtKind::Other => "bluetooth",
    }
}

/// The networks the flyout lists: in range (and the one it's on), eight at most.
pub fn listed(n: &Network) -> Vec<&WifiNetwork> {
    n.networks.iter().filter(|w| w.signal > 0 || w.active).take(8).collect()
}

/// The network asking for its password (kept across redraws: a network
/// event mustn't lose what's typed), and whether the last try was wrong.
struct Join {
    ssid: String,
    field: TextField,
    wrong: bool,
}

thread_local! {
    static JOIN: RefCell<Option<Join>> = const { RefCell::new(None) };
}

impl Tray {
    /// The tray's icons and tips for the network and Bluetooth: wadd's
    /// when it says, else what system.rs reads.
    pub(super) fn radio_icons(&self, st: &system::Status) {
        match self.shell.store.network() {
            Some(n) => {
                self.net_icon.set(net_icon(&n));
                if let Some(b) = self.button("network") {
                    b.set_tooltip_text(Some(&net_tip(&n)));
                }
            }
            None => {
                self.net_icon.set(system::net_icon(st.network.as_ref()));
                if let Some(b) = self.button("network") {
                    b.set_tooltip_text(Some(&system::net_tip(st.network.as_ref())));
                }
            }
        }
        match self.shell.store.bluetooth() {
            Some(b) => {
                self.bt_icon.set(if b.on { "bluetooth" } else { "bluetooth-off" });
                if let Some(btn) = self.button("bluetooth") {
                    btn.set_tooltip_text(Some(&bt_tip(&b)));
                }
            }
            None => {
                self.bt_icon.set(if st.bluetooth.as_ref().is_some_and(|b| !b.on) {
                    "bluetooth-off"
                } else {
                    "bluetooth"
                });
                if let Some(b) = self.button("bluetooth") {
                    b.set_tooltip_text(Some(&system::bluetooth_tip(st.bluetooth.as_ref())));
                }
            }
        }
    }

    /// wadd said something new of the network or Bluetooth.
    pub(super) fn radios_changed(self: &Rc<Self>) {
        let st = self.shared.system.borrow().clone();
        self.radio_icons(&st);
        self.refill_open("network");
        self.refill_open("bluetooth");
    }

    fn settings_footer(self: &Rc<Self>, col: &gtk::Box, text: &str, page: &'static str) {
        let foot = gtk::Box::builder().css_classes(["wb-foot"]).build();
        let b = quiet("settings", text);
        let (w, s) = (Rc::downgrade(self), self.shell.clone());
        b.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.close_all();
            }
            crate::peers::open_app(&s, &["wados-settings", "--page", page]);
        });
        foot.append(&b);
        col.append(&foot);
    }

    /// A call to wadd from a flyout; a refusal is a toast.
    fn radio_call<T: Send + 'static, F>(&self, call: impl FnOnce(wad_wadd_client::Wadd) -> F)
    where
        F: std::future::Future<Output = Result<T, wad_wadd_client::Failure>> + Send + 'static,
    {
        let s = self.shell.clone();
        self.shell.link.call(call, move |r| {
            if let Err(e) = r {
                s.toast(wad_proto::Level::Error, &e.message, None);
            }
        });
    }

    pub(super) fn network_content(self: &Rc<Self>, col: &gtk::Box, st: &system::Status) {
        let Some(n) = self.shell.store.network() else {
            // A wadd without the network: what NetworkManager says, read-only.
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
            self.settings_footer(col, "Network settings", "wifi");
            return;
        };
        let w = Rc::downgrade(self);
        let t = Toggle::new(n.wifi_on && !n.airplane, move |on| {
            if let Some(m) = w.upgrade() {
                m.radio_call(move |wadd| async move { wadd.set_wifi(on).await });
            }
        })
        .named("fly-wifi");
        t.root.set_sensitive(!n.airplane);
        col.append(&head("Network", Some(t.root.upcast_ref())));
        if n.ethernet.plugged {
            col.append(&row("cable", "Wired · connected", "", true));
        }
        if n.airplane {
            col.append(&ui::label("Airplane mode is on.", &["wb-p"]));
        } else if n.wifi_on {
            let joining = JOIN.with(|j| j.borrow().as_ref().map(|j| j.ssid.clone()));
            for net in listed(&n) {
                col.append(&self.network_row(net));
                if joining.as_deref() == Some(net.ssid.as_str()) {
                    col.append(&self.join_form(&net.ssid));
                }
            }
            if n.connected.is_some() {
                let b = gtk::Button::builder().label("Disconnect").css_classes(["wb-ghost"]).build();
                b.set_halign(gtk::Align::Start);
                b.set_margin_start(6);
                let w = Rc::downgrade(self);
                b.connect_clicked(move |b| {
                    b.set_sensitive(false);
                    if let Some(m) = w.upgrade() {
                        m.radio_call(|wadd| async move { wadd.wifi_disconnect().await });
                    }
                });
                col.append(&b);
            }
        } else {
            col.append(&ui::label("Wi-Fi is off.", &["wb-p"]));
        }
        self.settings_footer(col, "Network settings", "wifi");
    }

    fn network_row(self: &Rc<Self>, net: &WifiNetwork) -> gtk::Button {
        let r = gtk::Box::builder().spacing(12).build();
        r.append(&Icon::new(signal_icon(net.signal), 18));
        let t = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        t.append(&label(&net.ssid, &[]));
        if net.active {
            t.append(&label("Connected", &["wb-small-t"]));
        } else if net.saved {
            t.append(&label("Saved", &["wb-small-t"]));
        }
        r.append(&t);
        if net.secure {
            let l = Icon::new("lock", 13);
            l.add_css_class("wb-lock");
            r.append(&l);
        }
        let b = gtk::Button::builder().child(&r).css_classes(["wb-row"]).build();
        b.set_widget_name(&format!("fly-net-{}", net.ssid));
        set_class(&b, "wb-on-row", net.active);
        let (w, net) = (Rc::downgrade(self), net.clone());
        b.connect_clicked(move |_| {
            let Some(m) = w.upgrade() else { return };
            if net.active {
                return;
            }
            if net.secure && !net.saved {
                JOIN.with(|j| {
                    let mut j = j.borrow_mut();
                    let again = j.as_ref().is_some_and(|x| x.ssid == net.ssid);
                    *j = (!again).then(|| Join {
                        ssid: net.ssid.clone(),
                        field: TextField::password("", "Password").named("fly-password"),
                        wrong: false,
                    });
                });
                m.refill_open("network");
                JOIN.with(|j| {
                    if let Some(j) = &*j.borrow() {
                        j.field.focus();
                    }
                });
                return;
            }
            JOIN.with(|j| *j.borrow_mut() = None);
            let s = net.ssid.clone();
            m.radio_call(move |wadd| async move { wadd.wifi_connect(&s, None).await });
        });
        b
    }

    fn join_form(self: &Rc<Self>, ssid: &str) -> gtk::Box {
        let form =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).css_classes(["wb-join"]).build();
        let line = gtk::Box::builder().spacing(6).build();
        let (field, wrong) = JOIN.with(|j| {
            let j = j.borrow();
            let j = j.as_ref().expect("joining");
            (j.field.clone(), j.wrong)
        });
        if let Some(p) = field.root.parent().and_then(|p| p.downcast::<gtk::Box>().ok()) {
            p.remove(&field.root);
        }
        field.root.set_hexpand(true);
        line.append(&field.root);
        let join = gtk::Button::builder().label("Join").css_classes(["wb-ghost"]).valign(gtk::Align::Start).build();
        join.set_widget_name("fly-join");
        line.append(&join);
        form.append(&line);
        if wrong {
            form.append(&label("That password didn’t work", &["wb-join-err"]));
        }
        let go = {
            let (w, s, f) = (Rc::downgrade(self), ssid.to_string(), field.clone());
            move || {
                let Some(m) = w.upgrade() else { return };
                let p = f.value();
                if p.is_empty() {
                    return;
                }
                let (s2, w2) = (s.clone(), Rc::downgrade(&m));
                m.shell.link.call(
                    move |wadd| async move { wadd.wifi_connect(&s2, Some(&p)).await },
                    move |r| {
                        let Some(m) = w2.upgrade() else { return };
                        match r {
                            Ok(_) => JOIN.with(|j| *j.borrow_mut() = None),
                            Err(e) if e.status == 401 || e.status == 400 => JOIN.with(|j| {
                                if let Some(j) = j.borrow_mut().as_mut() {
                                    j.wrong = true;
                                }
                            }),
                            Err(e) => m.shell.toast(wad_proto::Level::Error, &e.message, None),
                        }
                        m.refill_open("network");
                    },
                );
            }
        };
        let g = go.clone();
        join.connect_clicked(move |_| g());
        field.on_enter(move |_| go());
        form
    }

    pub(super) fn bluetooth_content(self: &Rc<Self>, col: &gtk::Box, st: &system::Status) {
        let Some(b) = self.shell.store.bluetooth() else {
            col.append(&head("Bluetooth", None));
            match &st.bluetooth {
                Some(b) if b.on => {
                    col.append(&row("bluetooth", "On", &format!("{} connected", b.connected), b.connected > 0))
                }
                Some(_) => col.append(&row("bluetooth-off", "Off", "", false)),
                None => col.append(&ui::label("No Bluetooth on this machine.", &["wb-p"])),
            }
            self.settings_footer(col, "Bluetooth settings", "bluetooth");
            return;
        };
        let w = Rc::downgrade(self);
        let t = Toggle::new(b.on, move |on| {
            if let Some(m) = w.upgrade() {
                m.radio_call(move |wadd| async move { wadd.set_bluetooth(on).await });
            }
        })
        .named("fly-bluetooth");
        col.append(&head("Bluetooth", Some(t.root.upcast_ref())));
        if b.on {
            for d in &b.devices {
                col.append(&self.device_row(d));
            }
            if b.devices.is_empty() {
                col.append(&ui::label("No devices yet: pair one in Settings.", &["wb-p"]));
            }
        } else {
            col.append(&ui::label("Bluetooth is off.", &["wb-p"]));
        }
        self.settings_footer(col, "Bluetooth settings", "bluetooth");
    }

    fn device_row(self: &Rc<Self>, d: &BtDevice) -> gtk::Button {
        let r = gtk::Box::builder().spacing(12).build();
        r.append(&Icon::new(device_icon(d.kind), 18));
        let t = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        t.append(&label(&d.name, &[]));
        t.append(&label(&device_sub(d), &["wb-small-t"]));
        r.append(&t);
        let b = gtk::Button::builder().child(&r).css_classes(["wb-row"]).build();
        b.set_widget_name(&format!("fly-bt-{}", d.address));
        set_class(&b, "wb-on-row", d.connected);
        let (w, d) = (Rc::downgrade(self), d.clone());
        b.connect_clicked(move |b| {
            let Some(m) = w.upgrade() else { return };
            let a = d.address.clone();
            if !d.paired {
                // Pairing asks things: Settings ▸ Bluetooth shows them.
                m.close_all();
                crate::peers::open_app(&m.shell, &["wados-settings", "--page", "bluetooth"]);
                m.radio_call(move |wadd| async move { wadd.bt_pair(&a).await });
                return;
            }
            b.set_sensitive(false);
            if d.connected {
                m.radio_call(move |wadd| async move { wadd.bt_disconnect(&a).await });
            } else {
                m.radio_call(move |wadd| async move { wadd.bt_connect(&a).await });
            }
        });
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_proto::Ethernet;

    #[test]
    fn the_tray_says_what_the_spike_does() {
        let mut n = Network { wifi_on: true, connected: Some("Home".into()), ..Default::default() };
        assert_eq!((net_icon(&n), net_tip(&n).as_str()), ("wifi", "Network: on Home"));
        n.connected = None;
        assert_eq!((net_icon(&n), net_tip(&n).as_str()), ("wifi-off", "Network: not connected"));
        n.airplane = true;
        assert_eq!(net_tip(&n), "Network: airplane mode");
        n.ethernet = Ethernet { plugged: true, ..Default::default() };
        assert_eq!((net_icon(&n), net_tip(&n).as_str()), ("cable", "Network: wired"));
        let d = BtDevice {
            address: "AA".into(),
            name: "Buds".into(),
            kind: BtKind::Headphones,
            paired: true,
            connected: true,
            battery: Some(80),
        };
        assert_eq!(device_sub(&d), "Connected · 80%");
        let b = Bluetooth { on: true, devices: vec![d.clone()], ..Default::default() };
        assert_eq!(bt_tip(&b), "Bluetooth: 1 connected");
        assert_eq!(device_sub(&BtDevice { connected: false, ..d.clone() }), "Paired");
        assert_eq!(device_sub(&BtDevice { connected: false, paired: false, ..d }), "Not paired");
    }
}
