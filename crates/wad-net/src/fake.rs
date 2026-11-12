//! A fake NetworkManager on a private D-Bus (its own dbus-daemon: never
//! the machine's), for wad-net's tests and wadd's. It knows each network's
//! password, keeps profiles (without their secrets, as the real one shows
//! them), records what it was sent, and has a wire that can be plugged in.
//!
//! ```ignore
//! let bus = PrivateBus::start().await;
//! let world = World::with(vec![FakeAp::secure("Home", 70)], &[("Home", "correct horse")]);
//! let _nm = FakeNm::serve(&bus, world.clone()).await;
//! let nm = NetworkManager::on(bus.connect().await);
//! ```

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use zbus::object_server::ObjectServer;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

type Settings = HashMap<String, HashMap<String, OwnedValue>>;

const NM: &str = "/org/freedesktop/NetworkManager";
const WIFI_DEV: &str = "/org/freedesktop/NetworkManager/Devices/1";
const WIRED_DEV: &str = "/org/freedesktop/NetworkManager/Devices/2";
const WIFI_IP: &str = "/org/freedesktop/NetworkManager/IP4Config/1";
const WIRED_IP: &str = "/org/freedesktop/NetworkManager/IP4Config/2";

/// A dbus-daemon of its own, in a temporary folder; stopped when dropped.
pub struct PrivateBus {
    /// Its address (`unix:path=…`).
    pub address: String,
    _dir: tempfile::TempDir,
    _daemon: tokio::process::Child,
}

impl PrivateBus {
    pub async fn start() -> Self {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let sock: PathBuf = dir.path().join("bus");
        let address = format!("unix:path={}", sock.display());
        let daemon = tokio::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--nopidfile", "--address", &address])
            .stdout(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("dbus-daemon");
        for _ in 0..250 {
            if sock.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        Self { address, _dir: dir, _daemon: daemon }
    }

    /// A client's connection to it.
    pub async fn connect(&self) -> zbus::Connection {
        zbus::connection::Builder::address(self.address.as_str()).expect("address").build().await.expect("the bus")
    }
}

/// An access point.
#[derive(Debug, Clone)]
pub struct FakeAp {
    pub ssid: String,
    pub strength: u8,
    pub flags: u32,
    pub rsn: u32,
    pub freq: u32,
}

impl FakeAp {
    /// A WPA2 network.
    pub fn secure(ssid: &str, strength: u8) -> Self {
        Self { ssid: ssid.into(), strength, flags: 1, rsn: 0x188, freq: 5180 }
    }

    pub fn open(ssid: &str, strength: u8) -> Self {
        Self { ssid: ssid.into(), strength, flags: 0, rsn: 0, freq: 2437 }
    }

    /// WPA2 Enterprise (802.1X).
    pub fn enterprise(ssid: &str, strength: u8) -> Self {
        Self { ssid: ssid.into(), strength, flags: 1, rsn: 0x288, freq: 5180 }
    }
}

/// What the fake knows, and what it was sent.
#[derive(Default)]
pub struct World {
    pub aps: Vec<FakeAp>,
    pub passwords: HashMap<String, String>,
    /// Profile number → its settings (with the psk, which GetSettings hides).
    pub profiles: BTreeMap<u32, Settings>,
    pub actives: HashMap<u32, u32>,
    pub next: u32,
    pub active_ap: Option<usize>,
    /// What AddAndActivateConnection was sent.
    pub added: Vec<Settings>,
    pub last_scan: i64,
    pub wifi_on: bool,
    pub hardware: bool,
    /// The wire: a cable in, its speed (Mb/s); none: no wired device.
    pub wired: Option<(bool, u32)>,
    /// Why the last join failed (NMDeviceStateReason).
    pub reason: u32,
    /// What the connectivity check found (`crate::PORTAL`: a sign-in page).
    pub connectivity: u32,
}

pub type W = Arc<Mutex<World>>;

impl World {
    pub fn with(aps: Vec<FakeAp>, passwords: &[(&str, &str)]) -> W {
        Arc::new(Mutex::new(World {
            aps,
            passwords: passwords.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            next: 100,
            wifi_on: true,
            hardware: true,
            ..Default::default()
        }))
    }
}

fn path(p: impl Into<String>) -> OwnedObjectPath {
    OwnedObjectPath::try_from(p.into()).expect("a path")
}

fn owned(v: Value<'_>) -> OwnedValue {
    OwnedValue::try_from(v).expect("a value")
}

fn copy(s: &Settings) -> Settings {
    s.iter()
        .map(|(k, v)| (k.clone(), v.iter().map(|(a, b)| (a.clone(), b.try_clone().expect("a value"))).collect()))
        .collect()
}

/// A setting's text (`802-11-wireless-security`'s `psk`).
pub fn get_str(s: &Settings, a: &str, k: &str) -> Option<String> {
    s.get(a)?.get(k).and_then(|v| String::try_from(v.try_clone().ok()?).ok())
}

/// A profile's SSID.
pub fn ssid_of(s: &Settings) -> String {
    let v = s.get("802-11-wireless").and_then(|w| w.get("ssid")).map(|v| v.try_clone().expect("a value"));
    String::from_utf8(v.map(|v| Vec::<u8>::try_from(v).expect("bytes")).unwrap_or_default()).expect("utf-8")
}

struct Nm(W);

impl Nm {
    /// Starts an "activation": activated if the password fits, else not.
    async fn activate(&self, settings: &Settings, server: &ObjectServer, ap: &OwnedObjectPath) -> OwnedObjectPath {
        let ssid = ssid_of(settings);
        let psk = get_str(settings, "802-11-wireless-security", "psk");
        let n = {
            let mut w = self.0.lock().unwrap();
            let ok = match w.passwords.get(&ssid) {
                Some(p) => psk.as_deref() == Some(p.as_str()),
                None => true,
            };
            w.next += 1;
            let n = w.next;
            w.actives.insert(n, if ok { 2 } else { 4 });
            if ok {
                w.active_ap = ap.as_str().rsplit('/').next().and_then(|i| i.parse().ok());
                w.reason = 0;
            } else {
                w.reason = crate::REASON_NO_SECRETS;
            }
            n
        };
        let p = path(format!("{NM}/ActiveConnection/{n}"));
        server.at(p.clone(), Active(self.0.clone(), n)).await.expect("served");
        p
    }
}

#[zbus::interface(name = "org.freedesktop.NetworkManager")]
impl Nm {
    fn get_devices(&self) -> Vec<OwnedObjectPath> {
        let mut d = vec![path(WIFI_DEV)];
        if self.0.lock().unwrap().wired.is_some() {
            d.push(path(WIRED_DEV));
        }
        d
    }

    async fn activate_connection(
        &self,
        connection: OwnedObjectPath,
        _device: OwnedObjectPath,
        specific_object: OwnedObjectPath,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let n: u32 = connection.as_str().rsplit('/').next().and_then(|n| n.parse().ok()).unwrap_or(0);
        let settings = self.0.lock().unwrap().profiles.get(&n).map(copy);
        let settings = settings.ok_or_else(|| zbus::fdo::Error::UnknownObject("no such profile".into()))?;
        Ok(self.activate(&settings, server, &specific_object).await)
    }

    async fn add_and_activate_connection(
        &self,
        connection: Settings,
        _device: OwnedObjectPath,
        specific_object: OwnedObjectPath,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<(OwnedObjectPath, OwnedObjectPath)> {
        let n = {
            let mut w = self.0.lock().unwrap();
            w.next += 1;
            let n = w.next;
            w.added.push(copy(&connection));
            w.profiles.insert(n, copy(&connection));
            n
        };
        let profile = path(format!("{NM}/Settings/{n}"));
        server.at(profile.clone(), Profile(self.0.clone(), n)).await.expect("served");
        let active = self.activate(&connection, server, &specific_object).await;
        Ok((profile, active))
    }

    #[zbus(property)]
    fn wireless_enabled(&self) -> bool {
        self.0.lock().unwrap().wifi_on
    }
    #[zbus(property)]
    fn set_wireless_enabled(&self, on: bool) {
        let mut w = self.0.lock().unwrap();
        w.wifi_on = on;
        if !on {
            w.active_ap = None;
        }
    }
    #[zbus(property)]
    fn wireless_hardware_enabled(&self) -> bool {
        self.0.lock().unwrap().hardware
    }
    #[zbus(property)]
    fn connectivity(&self) -> u32 {
        self.0.lock().unwrap().connectivity
    }
}

/// The Wi-Fi device (1) or the wired one (2).
struct Device(W, u32);

#[zbus::interface(name = "org.freedesktop.NetworkManager.Device")]
impl Device {
    fn disconnect(&self) {
        if self.1 == 1 {
            self.0.lock().unwrap().active_ap = None;
        }
    }
    #[zbus(property)]
    fn device_type(&self) -> u32 {
        if self.1 == 1 { crate::DEVICE_WIFI } else { crate::DEVICE_ETHERNET }
    }
    #[zbus(property)]
    fn interface(&self) -> String {
        if self.1 == 1 { "wlan0".into() } else { "enp1s0".into() }
    }
    #[zbus(property)]
    fn state(&self) -> u32 {
        let w = self.0.lock().unwrap();
        let up = if self.1 == 1 { w.active_ap.is_some() } else { w.wired.is_some_and(|(c, _)| c) };
        if up { crate::DEVICE_ACTIVATED } else { 30 }
    }
    #[zbus(property)]
    fn state_reason(&self) -> (u32, u32) {
        let w = self.0.lock().unwrap();
        (if w.active_ap.is_some() { 100 } else { 30 }, w.reason)
    }
    #[zbus(property)]
    fn ip4_config(&self) -> OwnedObjectPath {
        let w = self.0.lock().unwrap();
        match self.1 {
            1 if w.active_ap.is_some() => path(WIFI_IP),
            2 if w.wired.is_some_and(|(c, _)| c) => path(WIRED_IP),
            _ => path("/"),
        }
    }
    #[zbus(property)]
    fn ip6_config(&self) -> OwnedObjectPath {
        path("/")
    }
}

struct Wired(W);

#[zbus::interface(name = "org.freedesktop.NetworkManager.Device.Wired")]
impl Wired {
    #[zbus(property)]
    fn carrier(&self) -> bool {
        self.0.lock().unwrap().wired.is_some_and(|(c, _)| c)
    }
    #[zbus(property)]
    fn speed(&self) -> u32 {
        self.0.lock().unwrap().wired.map(|(_, s)| s).unwrap_or(0)
    }
}

struct Wireless(W);

#[zbus::interface(name = "org.freedesktop.NetworkManager.Device.Wireless")]
impl Wireless {
    fn get_all_access_points(&self) -> Vec<OwnedObjectPath> {
        let w = self.0.lock().unwrap();
        if !w.wifi_on {
            return vec![];
        }
        (0..w.aps.len()).map(|i| path(format!("{NM}/AccessPoint/{i}"))).collect()
    }
    fn request_scan(&self, _options: HashMap<String, OwnedValue>) {
        self.0.lock().unwrap().last_scan += 1;
    }
    #[zbus(property)]
    fn active_access_point(&self) -> OwnedObjectPath {
        match self.0.lock().unwrap().active_ap {
            Some(i) => path(format!("{NM}/AccessPoint/{i}")),
            None => path("/"),
        }
    }
    #[zbus(property)]
    fn last_scan(&self) -> i64 {
        self.0.lock().unwrap().last_scan
    }
}

struct Ap(W, usize);

#[zbus::interface(name = "org.freedesktop.NetworkManager.AccessPoint")]
impl Ap {
    #[zbus(property)]
    fn ssid(&self) -> Vec<u8> {
        self.0.lock().unwrap().aps[self.1].ssid.as_bytes().to_vec()
    }
    #[zbus(property)]
    fn strength(&self) -> u8 {
        self.0.lock().unwrap().aps[self.1].strength
    }
    #[zbus(property)]
    fn flags(&self) -> u32 {
        self.0.lock().unwrap().aps[self.1].flags
    }
    #[zbus(property)]
    fn wpa_flags(&self) -> u32 {
        0
    }
    #[zbus(property)]
    fn rsn_flags(&self) -> u32 {
        self.0.lock().unwrap().aps[self.1].rsn
    }
    #[zbus(property)]
    fn frequency(&self) -> u32 {
        self.0.lock().unwrap().aps[self.1].freq
    }
}

/// An address (`192.168.1.<host>`/24) and the router as DNS.
struct Ip4(u8);

#[zbus::interface(name = "org.freedesktop.NetworkManager.IP4Config")]
impl Ip4 {
    #[zbus(property)]
    fn address_data(&self) -> Vec<HashMap<String, OwnedValue>> {
        vec![HashMap::from([
            ("address".to_string(), owned(Value::from(format!("192.168.1.{}", self.0)))),
            ("prefix".to_string(), owned(Value::from(24u32))),
        ])]
    }
    #[zbus(property)]
    fn nameserver_data(&self) -> Vec<HashMap<String, OwnedValue>> {
        vec![HashMap::from([("address".to_string(), owned(Value::from("192.168.1.1")))])]
    }
}

struct SettingsIface(W);

#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings")]
impl SettingsIface {
    fn list_connections(&self) -> Vec<OwnedObjectPath> {
        self.0.lock().unwrap().profiles.keys().map(|n| path(format!("{NM}/Settings/{n}"))).collect()
    }
}

struct Profile(W, u32);

#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl Profile {
    fn get_settings(&self) -> zbus::fdo::Result<Settings> {
        let w = self.0.lock().unwrap();
        let s = w.profiles.get(&self.1).ok_or_else(|| zbus::fdo::Error::UnknownObject("deleted".into()))?;
        // Secrets aren't in GetSettings.
        let mut s = copy(s);
        for v in s.values_mut() {
            v.remove("psk");
        }
        Ok(s)
    }
    fn delete(&self) {
        let mut w = self.0.lock().unwrap();
        if let Some(s) = w.profiles.remove(&self.1) {
            let ssid = ssid_of(&s);
            let on = w.active_ap.and_then(|i| w.aps.get(i)).is_some_and(|a| a.ssid == ssid);
            if on {
                w.active_ap = None;
            }
        }
    }
}

struct Active(W, u32);

#[zbus::interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl Active {
    #[zbus(property)]
    fn state(&self) -> u32 {
        self.0.lock().unwrap().actives.get(&self.1).copied().unwrap_or(4)
    }
}

/// The fake NetworkManager, served on `bus` for as long as it's kept.
pub struct FakeNm {
    _conn: zbus::Connection,
}

impl FakeNm {
    pub async fn serve(bus: &PrivateBus, world: W) -> Self {
        let n = world.lock().unwrap().aps.len();
        let mut b = zbus::connection::Builder::address(bus.address.as_str())
            .expect("address")
            .name("org.freedesktop.NetworkManager")
            .expect("name")
            .serve_at(NM, Nm(world.clone()))
            .expect("served")
            .serve_at(WIFI_DEV, Device(world.clone(), 1))
            .expect("served")
            .serve_at(WIFI_DEV, Wireless(world.clone()))
            .expect("served")
            .serve_at(WIRED_DEV, Device(world.clone(), 2))
            .expect("served")
            .serve_at(WIRED_DEV, Wired(world.clone()))
            .expect("served")
            .serve_at(WIFI_IP, Ip4(42))
            .expect("served")
            .serve_at(WIRED_IP, Ip4(24))
            .expect("served")
            .serve_at(format!("{NM}/Settings"), SettingsIface(world.clone()))
            .expect("served");
        for i in 0..n {
            b = b.serve_at(format!("{NM}/AccessPoint/{i}"), Ap(world.clone(), i)).expect("served");
        }
        // Profiles saved before it started.
        let saved: Vec<u32> = world.lock().unwrap().profiles.keys().copied().collect();
        for p in saved {
            b = b.serve_at(format!("{NM}/Settings/{p}"), Profile(world.clone(), p)).expect("served");
        }
        Self { _conn: b.build().await.expect("the fake NetworkManager") }
    }
}

/// A saved profile for `ssid` (secure: WPA-PSK), as one joined before.
pub fn saved_profile(world: &W, ssid: &str, secure: bool) {
    let mut w = world.lock().unwrap();
    w.next += 1;
    let n = w.next;
    let mut s: Settings = HashMap::new();
    s.insert(
        "connection".into(),
        HashMap::from([
            ("id".to_string(), owned(Value::from(ssid))),
            ("type".to_string(), owned(Value::from("802-11-wireless"))),
        ]),
    );
    s.insert("802-11-wireless".into(), HashMap::from([("ssid".to_string(), owned(Value::from(ssid.as_bytes())))]));
    if secure {
        let psk = w.passwords.get(ssid).cloned().unwrap_or_default();
        s.insert(
            "802-11-wireless-security".into(),
            HashMap::from([
                ("key-mgmt".to_string(), owned(Value::from("wpa-psk"))),
                ("psk".to_string(), owned(Value::from(psk))),
            ]),
        );
    }
    w.profiles.insert(n, s);
}
