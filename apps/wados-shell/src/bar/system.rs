//! What the tray can say about the machine without Settings (phase 8 does
//! the changing): the network (NetworkManager), the battery (UPower),
//! Bluetooth (BlueZ) and the screen's brightness (sysfs), read-only; and the
//! volume and the brightness as the session's keys left them (session.rs
//! tells it, through [`start`]'s setter). What isn't there stays `None`, and
//! the tray says it doesn't know rather than making a value up.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::{gio, glib, prelude::*};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub network: Option<Net>,
    pub battery: Option<Battery>,
    pub bluetooth: Option<Bluetooth>,
    /// 0 to 1.
    pub brightness: Option<f64>,
    pub volume: Option<Volume>,
}

/// The default output's volume (PipeWire's, through wpctl).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Volume {
    /// 0 to 1 (1.5 at most, as PipeWire allows).
    pub level: f64,
    pub muted: bool,
}

/// The spike's (`Tray.svelte`): muted or silent, low, or more.
pub fn volume_icon(v: Option<&Volume>) -> &'static str {
    match v {
        Some(v) if v.muted || v.level <= 0.0 => "volume-x",
        Some(v) if v.level < 0.4 => "volume-1",
        _ => "volume-2",
    }
}

pub fn volume_tip(v: Option<&Volume>) -> String {
    match v {
        None => "Sound".into(),
        Some(v) if v.muted => "Sound: muted".into(),
        Some(v) => format!("Sound: {}%", (v.level * 100.0).round() as i64),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetState {
    Connected,
    Connecting,
    Offline,
    Asleep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetKind {
    Wired,
    Wifi,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Net {
    pub state: NetState,
    pub kind: NetKind,
    /// The connection's name (the Wi-Fi's, a wired profile's).
    pub name: Option<String>,
    pub wifi_on: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Battery {
    /// 0 to 100.
    pub percent: f64,
    pub charging: bool,
    pub full: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bluetooth {
    pub on: bool,
    pub connected: u32,
}

/// NetworkManager's `State`.
pub fn nm_state(n: u32) -> NetState {
    match n {
        70 | 60 | 50 => NetState::Connected,
        40 => NetState::Connecting,
        10 => NetState::Asleep,
        _ => NetState::Offline,
    }
}

/// NetworkManager's `PrimaryConnectionType`.
pub fn nm_kind(t: &str) -> NetKind {
    match t {
        "802-3-ethernet" => NetKind::Wired,
        "802-11-wireless" => NetKind::Wifi,
        _ => NetKind::Other,
    }
}

/// UPower's device `State`: (charging, full).
pub fn upower_state(n: u32) -> (bool, bool) {
    match n {
        1 | 5 => (true, false),
        4 => (false, true),
        _ => (false, false),
    }
}

pub fn net_icon(n: Option<&Net>) -> &'static str {
    match n {
        None => "wifi",
        Some(n) if n.state != NetState::Connected => "wifi-off",
        Some(n) if n.kind == NetKind::Wired => "cable",
        Some(_) => "wifi",
    }
}

pub fn net_tip(n: Option<&Net>) -> String {
    match n {
        None => "Network".into(),
        Some(n) => match n.state {
            NetState::Asleep => "Network: asleep".into(),
            NetState::Offline if !n.wifi_on => "Network: Wi-Fi is off".into(),
            NetState::Offline => "Network: not connected".into(),
            NetState::Connecting => "Network: connecting…".into(),
            NetState::Connected => match (&n.kind, &n.name) {
                (NetKind::Wired, _) => "Network: wired".into(),
                (_, Some(name)) => format!("Network: on {name}"),
                _ => "Network: connected".into(),
            },
        },
    }
}

pub fn battery_icon(b: &Battery) -> &'static str {
    if b.charging {
        "battery-charging"
    } else if b.percent < 20.0 {
        "battery-low"
    } else {
        "battery"
    }
}

pub fn battery_text(b: &Battery) -> String {
    let how = if b.full {
        "full"
    } else if b.charging {
        "charging"
    } else {
        "on battery"
    };
    format!("{}% · {how}", b.percent.round() as i64)
}

pub fn bluetooth_tip(b: Option<&Bluetooth>) -> String {
    match b {
        None => "Bluetooth".into(),
        Some(b) if !b.on => "Bluetooth: off".into(),
        Some(b) => format!("Bluetooth: {} connected", b.connected),
    }
}

/// Changes the status (`out` hears it if that changed it).
pub type Set = Rc<dyn Fn(&dyn Fn(&mut Status))>;

/// Starts following the machine; `out` gets the whole status at each
/// change. What it gives back changes it from elsewhere.
pub fn start(out: Rc<dyn Fn(&Status)>) -> Set {
    let st = Rc::new(RefCell::new(Status::default()));
    let set = {
        let (st, out) = (st.clone(), out.clone());
        Rc::new(move |f: &dyn Fn(&mut Status)| {
            let changed = {
                let mut s = st.borrow_mut();
                let before = s.clone();
                f(&mut s);
                *s != before
            };
            if changed {
                out(&st.borrow());
            }
        })
    };

    // The network.
    let s = set.clone();
    glib::spawn_future_local(async move {
        let Some(nm) = proxy(
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
        )
        .await
        else {
            return;
        };
        let rx = changes(&nm);
        loop {
            let net = read_network(&nm).await;
            s(&|st| st.network = net.clone());
            if rx.recv().await.is_err() {
                break;
            }
        }
    });

    // The battery.
    let s = set.clone();
    glib::spawn_future_local(async move {
        let Some(up) = proxy(
            "org.freedesktop.UPower",
            "/org/freedesktop/UPower/devices/DisplayDevice",
            "org.freedesktop.UPower.Device",
        )
        .await
        else {
            return;
        };
        let rx = changes(&up);
        loop {
            let b = read_battery(&up);
            s(&|st| st.battery = b.clone());
            if rx.recv().await.is_err() {
                break;
            }
        }
    });

    // Bluetooth and the brightness: looked at now and then.
    let s: Set = set.clone();
    let look = move || {
        let s2 = s.clone();
        glib::spawn_future_local(async move {
            let bt = read_bluetooth().await;
            s2(&|st| st.bluetooth = bt.clone());
        });
        let b = read_brightness();
        s(&|st| st.brightness = b);
    };
    look();
    glib::timeout_add_seconds_local(10, move || {
        look();
        glib::ControlFlow::Continue
    });
    set
}

async fn proxy(name: &str, path: &str, iface: &str) -> Option<gio::DBusProxy> {
    let p = gio::DBusProxy::for_bus_future(gio::BusType::System, gio::DBusProxyFlags::NONE, None, name, path, iface)
        .await
        .ok()?;
    p.name_owner().map(|_| p)
}

/// A wake-up each time the proxy's properties change.
fn changes(p: &gio::DBusProxy) -> async_channel::Receiver<()> {
    let (tx, rx) = async_channel::unbounded();
    p.connect_g_properties_changed(move |_, _, _| {
        let _ = tx.try_send(());
    });
    rx
}

fn prop<T: glib::variant::FromVariant>(p: &gio::DBusProxy, name: &str) -> Option<T> {
    p.cached_property(name).and_then(|v| v.get::<T>())
}

async fn read_network(nm: &gio::DBusProxy) -> Option<Net> {
    let state = nm_state(prop::<u32>(nm, "State")?);
    let kind = nm_kind(&prop::<String>(nm, "PrimaryConnectionType").unwrap_or_default());
    let wifi_on = prop::<bool>(nm, "WirelessEnabled").unwrap_or(true);
    let path = nm.cached_property("PrimaryConnection").and_then(|v| v.str().map(str::to_string));
    let mut name = None;
    if let Some(path) = path.filter(|p| p != "/")
        && let Some(active) =
            proxy("org.freedesktop.NetworkManager", &path, "org.freedesktop.NetworkManager.Connection.Active").await
    {
        name = prop::<String>(&active, "Id");
    }
    Some(Net { state, kind, name, wifi_on })
}

fn read_battery(up: &gio::DBusProxy) -> Option<Battery> {
    // Type 2 is a battery; a desktop's display device is nothing (0).
    if !prop::<bool>(up, "IsPresent").unwrap_or(false) || prop::<u32>(up, "Type") != Some(2) {
        return None;
    }
    let (charging, full) = upower_state(prop::<u32>(up, "State").unwrap_or(0));
    Some(Battery { percent: prop::<f64>(up, "Percentage")?, charging, full })
}

async fn read_bluetooth() -> Option<Bluetooth> {
    let om = proxy("org.bluez", "/", "org.freedesktop.DBus.ObjectManager").await?;
    let v = om.call_future("GetManagedObjects", None, gio::DBusCallFlags::NONE, 2000).await.ok()?;
    let objects = v.try_child_value(0)?;
    let (mut adapter, mut on, mut connected) = (false, false, 0);
    for i in 0..objects.n_children() {
        let Some(ifaces) = objects.child_value(i).try_child_value(1) else { continue };
        for j in 0..ifaces.n_children() {
            let e = ifaces.child_value(j);
            let (Some(name), Some(props)) = (e.try_child_value(0), e.try_child_value(1)) else { continue };
            let flag = |k: &str| {
                (0..props.n_children()).map(|i| props.child_value(i)).any(|kv| {
                    kv.try_child_value(0).is_some_and(|n| n.str() == Some(k))
                        && kv
                            .try_child_value(1)
                            .and_then(|v| v.as_variant())
                            .and_then(|v| v.get::<bool>())
                            .unwrap_or(false)
                })
            };
            match name.str() {
                Some("org.bluez.Adapter1") => {
                    adapter = true;
                    on |= flag("Powered");
                }
                Some("org.bluez.Device1") if flag("Connected") => connected += 1,
                _ => {}
            }
        }
    }
    adapter.then_some(Bluetooth { on, connected })
}

fn read_brightness() -> Option<f64> {
    let dir = std::fs::read_dir("/sys/class/backlight").ok()?.flatten().next()?.path();
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).ok()?.trim().parse::<f64>().ok();
    let (now, max) = (read("brightness")?, read("max_brightness")?);
    (max > 0.0).then(|| (now / max).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_says_what_it_knows() {
        assert_eq!(net_icon(None), "wifi");
        assert_eq!(net_tip(None), "Network");
        let wifi = Net {
            state: nm_state(70),
            kind: nm_kind("802-11-wireless"),
            name: Some("Kowhai 5G".into()),
            wifi_on: true,
        };
        assert_eq!(net_icon(Some(&wifi)), "wifi");
        assert_eq!(net_tip(Some(&wifi)), "Network: on Kowhai 5G");
        let wired = Net { kind: nm_kind("802-3-ethernet"), ..wifi.clone() };
        assert_eq!(net_icon(Some(&wired)), "cable");
        assert_eq!(net_tip(Some(&wired)), "Network: wired");
        let off = Net { state: nm_state(20), wifi_on: false, ..wifi };
        assert_eq!(net_icon(Some(&off)), "wifi-off");
        assert_eq!(net_tip(Some(&off)), "Network: Wi-Fi is off");
    }

    #[test]
    fn the_battery() {
        let (charging, full) = upower_state(1);
        let b = Battery { percent: 83.4, charging, full };
        assert_eq!(battery_icon(&b), "battery-charging");
        assert_eq!(battery_text(&b), "83% · charging");
        let low = Battery { percent: 12.0, charging: false, full: false };
        assert_eq!(battery_icon(&low), "battery-low");
        assert_eq!(battery_text(&low), "12% · on battery");
        assert_eq!(upower_state(4), (false, true));
    }

    #[test]
    fn the_volume() {
        assert_eq!(volume_icon(None), "volume-2");
        assert_eq!(volume_tip(None), "Sound");
        let v = |level, muted| Volume { level, muted };
        assert_eq!(volume_icon(Some(&v(0.45, false))), "volume-2");
        assert_eq!(volume_tip(Some(&v(0.45, false))), "Sound: 45%");
        assert_eq!(volume_icon(Some(&v(0.2, false))), "volume-1");
        assert_eq!(volume_icon(Some(&v(0.0, false))), "volume-x");
        assert_eq!(volume_icon(Some(&v(0.8, true))), "volume-x");
        assert_eq!(volume_tip(Some(&v(0.8, true))), "Sound: muted");
    }

    #[test]
    fn bluetooth() {
        assert_eq!(bluetooth_tip(None), "Bluetooth");
        assert_eq!(bluetooth_tip(Some(&Bluetooth { on: false, connected: 0 })), "Bluetooth: off");
        assert_eq!(bluetooth_tip(Some(&Bluetooth { on: true, connected: 2 })), "Bluetooth: 2 connected");
    }
}
