//! The network through NetworkManager over D-Bus (no nmcli): Wi-Fi's radio
//! on and off, the networks in range and the saved ones, joining, leaving
//! and forgetting one; and the wired network (a cable in, its speed, its
//! address). wadd makes it wad-proto's [`wad_proto::Network`].
//!
//! A network is joined with AddAndActivateConnection: the password goes to
//! NetworkManager inside the D-Bus call, so it's never on a command line
//! (other users can read /proc/*/cmdline) or in a file of ours, and
//! NetworkManager keeps it with the profile, which reconnects at boot. A
//! join that fails (a wrong password) deletes the profile it made, so it
//! doesn't sit there retrying. Enterprise (802.1X) and WEP networks can't be
//! joined here.
//!
//! wadd runs as root on a machine, so there are no polkit prompts.

use std::collections::HashMap;
use std::time::Duration;

use wad_proto::{Ethernet, WifiBand, WifiNetwork};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

#[cfg(feature = "fake")]
pub mod fake;
mod proxies;
use proxies::*;

/// NM80211ApFlags / NM80211ApSecurityFlags.
const AP_PRIVACY: u32 = 0x1;
const KEY_MGMT_PSK: u32 = 0x100;
const KEY_MGMT_802_1X: u32 = 0x200;
const KEY_MGMT_SAE: u32 = 0x400;
const KEY_MGMT_OWE: u32 = 0x800 | 0x1000;
/// NMDeviceType.
pub const DEVICE_ETHERNET: u32 = 1;
pub const DEVICE_WIFI: u32 = 2;
/// NMDeviceState: up, with its addresses.
pub const DEVICE_ACTIVATED: u32 = 100;
/// NMActiveConnectionState.
const ACTIVATED: u32 = 2;
const DEACTIVATED: u32 = 4;
/// NMDeviceStateReason: what a wrong password ends in (no secrets that
/// work; the supplicant giving up on the handshake, or on its config).
pub const REASON_NO_SECRETS: u32 = 7;
pub const REASON_SUPPLICANT_DISCONNECT: u32 = 8;
pub const REASON_SUPPLICANT_CONFIG_FAILED: u32 = 9;
pub const REASON_SUPPLICANT_TIMEOUT: u32 = 11;
const JOIN_TIMEOUT: Duration = Duration::from_secs(60);
/// A fresh scan, at most this long (NetworkManager's take a few seconds).
const SCAN_WAIT: Duration = Duration::from_secs(10);
/// Properties are read when they're asked for (some are polled).
const NO_CACHE: zbus::proxy::CacheProperties = zbus::proxy::CacheProperties::No;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("NetworkManager: {0}")]
    Bus(#[from] zbus::Error),
    #[error("This machine has no Wi-Fi")]
    NoWifi,
    #[error("Wi-Fi is off")]
    WifiOff,
    #[error("Wi-Fi is switched off on this machine (its key or switch)")]
    Blocked,
    #[error("{0} isn't in range")]
    NotInRange(String),
    #[error("{0} needs its password")]
    NeedsPassword(String),
    #[error("Wi-Fi passwords are 8 to 63 characters")]
    BadPassword,
    #[error("That isn't the password for {0}")]
    WrongPassword(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Failed(String),
    #[error("{0} isn't a saved network")]
    NotSaved(String),
}

impl From<zbus::fdo::Error> for Error {
    fn from(e: zbus::fdo::Error) -> Self {
        Error::Bus(e.into())
    }
}

/// What a network's security is, as nmcli shows it: "WPA2", "WPA1 WPA2",
/// "WPA3", "WPA2 802.1X", "WEP", "" (open).
pub fn security(flags: u32, wpa: u32, rsn: u32) -> String {
    let mut parts: Vec<&str> = vec![];
    if flags & AP_PRIVACY != 0 && wpa == 0 && rsn == 0 {
        parts.push("WEP");
    }
    if wpa != 0 {
        parts.push("WPA1");
    }
    if rsn & (KEY_MGMT_PSK | KEY_MGMT_802_1X) != 0 {
        parts.push("WPA2");
    }
    if rsn & KEY_MGMT_SAE != 0 {
        parts.push("WPA3");
    }
    if (wpa | rsn) & KEY_MGMT_OWE != 0 {
        parts.push("OWE");
    }
    if (wpa | rsn) & KEY_MGMT_802_1X != 0 {
        parts.push("802.1X");
    }
    parts.join(" ")
}

/// A network's security as Settings words it: "None (open network)",
/// "WPA2 Personal", "WPA3 Personal", "WPA2/WPA3 Personal", "WPA2 Enterprise"…
pub fn security_words(flags: u32, wpa: u32, rsn: u32) -> String {
    let both = wpa | rsn;
    if both & KEY_MGMT_802_1X != 0 {
        return if rsn != 0 { "WPA2 Enterprise".into() } else { "WPA Enterprise".into() };
    }
    let psk = both & KEY_MGMT_PSK != 0;
    let sae = rsn & KEY_MGMT_SAE != 0;
    match (psk, sae) {
        (true, true) => "WPA2/WPA3 Personal".into(),
        (false, true) => "WPA3 Personal".into(),
        (true, false) if rsn == 0 => "WPA Personal".into(),
        (true, false) => "WPA2 Personal".into(),
        _ if both & KEY_MGMT_OWE != 0 => "Enhanced Open (OWE)".into(),
        _ if flags & AP_PRIVACY != 0 => "WEP".into(),
        _ => "None (open network)".into(),
    }
}

/// The key management to join with: "" (open), "owe", "wpa-psk" or "sae";
/// None for what this can't join (enterprise, WEP).
pub fn key_mgmt(flags: u32, wpa: u32, rsn: u32) -> Option<&'static str> {
    let both = wpa | rsn;
    if both & KEY_MGMT_802_1X != 0 || (flags & AP_PRIVACY != 0 && both == 0) {
        return None;
    }
    if both & KEY_MGMT_PSK != 0 {
        return Some("wpa-psk");
    }
    if rsn & KEY_MGMT_SAE != 0 {
        return Some("sae");
    }
    if both & KEY_MGMT_OWE != 0 {
        return Some("owe");
    }
    Some("")
}

/// A network asks for a password (OWE's encrypted but open).
pub fn is_secure(flags: u32, wpa: u32, rsn: u32) -> bool {
    let s = security(flags, wpa, rsn);
    !s.is_empty() && s != "OWE"
}

/// A WPA password: 8 to 63 characters, or 64 hex digits (the key itself).
pub fn valid_psk(p: &str) -> bool {
    (8..=63).contains(&p.chars().count()) || (p.len() == 64 && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The band a frequency (MHz) is in.
pub fn band(mhz: u32) -> Option<WifiBand> {
    match mhz {
        2400..=2500 => Some(WifiBand::Ghz2),
        4900..=5900 => Some(WifiBand::Ghz5),
        5925..=7125 => Some(WifiBand::Ghz6),
        _ => None,
    }
}

/// Why a join failed, from the Wi-Fi device's state reason: a wrong
/// password (for a secure network), it not answering, or anything else.
pub fn join_failure(ssid: &str, secure: bool, reason: u32) -> Error {
    match reason {
        REASON_NO_SECRETS | REASON_SUPPLICANT_DISCONNECT | REASON_SUPPLICANT_CONFIG_FAILED if secure => {
            Error::WrongPassword(ssid.into())
        }
        REASON_SUPPLICANT_TIMEOUT => Error::Failed(format!("{ssid} didn't answer: is it still in range?")),
        _ => Error::Failed(format!("Couldn't join {ssid}")),
    }
}

/// The networks in range and saved, in the order Settings shows them: the
/// one it's on, then the saved ones, then by signal (then by name).
pub fn order(nets: &mut [WifiNetwork]) {
    nets.sort_by(|a, b| {
        (!a.active, !a.saved, std::cmp::Reverse(a.signal), a.ssid.to_lowercase()).cmp(&(
            !b.active,
            !b.saved,
            std::cmp::Reverse(b.signal),
            b.ssid.to_lowercase(),
        ))
    });
}

/// The network as NetworkManager has it now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    /// There's a Wi-Fi device.
    pub has_wifi: bool,
    pub wifi_on: bool,
    /// Its switch (a key, the BIOS) lets it be on.
    pub wifi_hardware: bool,
    /// The SSID it's on.
    pub connected: Option<String>,
    pub networks: Vec<WifiNetwork>,
    pub ethernet: Ethernet,
    /// The address on the network it's on (Wi-Fi's, else the wire's).
    pub ip: Option<String>,
    pub dns: Vec<String>,
}

/// A device's address and its DNS servers.
type Addresses = (Option<String>, Vec<String>);

struct Ap {
    path: OwnedObjectPath,
    ssid: String,
    signal: u8,
    flags: u32,
    wpa: u32,
    rsn: u32,
    freq: u32,
}

/// A saved Wi-Fi profile.
struct Profile {
    path: OwnedObjectPath,
    ssid: String,
    secure: bool,
}

fn get<T: TryFrom<OwnedValue>>(m: &HashMap<String, OwnedValue>, k: &str) -> Option<T> {
    m.get(k).and_then(|v| v.try_clone().ok()).and_then(|v| T::try_from(v).ok())
}

pub struct NetworkManager {
    conn: zbus::Connection,
    /// One change at a time.
    lock: tokio::sync::Mutex<()>,
}

impl NetworkManager {
    /// The system's NetworkManager.
    pub async fn system() -> Result<Self, Error> {
        Ok(Self::on(zbus::Connection::system().await?))
    }

    /// On this bus (tests: a private one).
    pub fn on(conn: zbus::Connection) -> Self {
        Self { conn, lock: tokio::sync::Mutex::new(()) }
    }

    async fn nm(&self) -> Result<NmProxy<'_>, Error> {
        Ok(NmProxy::builder(&self.conn).cache_properties(NO_CACHE).build().await?)
    }

    async fn device(&self, path: &OwnedObjectPath) -> Result<DeviceProxy<'_>, Error> {
        Ok(DeviceProxy::builder(&self.conn).path(path.clone())?.cache_properties(NO_CACHE).build().await?)
    }

    async fn wireless(&self, path: &OwnedObjectPath) -> Result<WirelessProxy<'_>, Error> {
        Ok(WirelessProxy::builder(&self.conn).path(path.clone())?.cache_properties(NO_CACHE).build().await?)
    }

    async fn profile(&self, path: &OwnedObjectPath) -> Result<ConnectionProxy<'_>, Error> {
        Ok(ConnectionProxy::builder(&self.conn).path(path.clone())?.cache_properties(NO_CACHE).build().await?)
    }

    /// The devices of `kind`.
    async fn devices(&self, kind: u32) -> Result<Vec<OwnedObjectPath>, Error> {
        let mut out = vec![];
        for path in self.nm().await?.get_devices().await? {
            if self.device(&path).await?.device_type().await? == kind {
                out.push(path);
            }
        }
        Ok(out)
    }

    async fn wifi_device(&self) -> Result<Option<OwnedObjectPath>, Error> {
        Ok(self.devices(DEVICE_WIFI).await?.into_iter().next())
    }

    async fn ap(&self, path: OwnedObjectPath) -> Result<Ap, Error> {
        let p = AccessPointProxy::builder(&self.conn).path(path.clone())?.cache_properties(NO_CACHE).build().await?;
        Ok(Ap {
            ssid: String::from_utf8_lossy(&p.ssid().await?).into_owned(),
            signal: p.strength().await?.min(100),
            flags: p.flags().await?,
            wpa: p.wpa_flags().await?,
            rsn: p.rsn_flags().await?,
            freq: p.frequency().await.unwrap_or(0),
            path,
        })
    }

    /// The saved Wi-Fi profiles.
    async fn profiles(&self) -> Result<Vec<Profile>, Error> {
        let settings = SettingsProxy::builder(&self.conn).cache_properties(NO_CACHE).build().await?;
        let mut out = vec![];
        for path in settings.list_connections().await? {
            let Ok(s) = self.profile(&path).await?.get_settings().await else { continue };
            let Some(ssid) = s.get("802-11-wireless").and_then(|w| get::<Vec<u8>>(w, "ssid")) else { continue };
            let mgmt: Option<String> = s.get("802-11-wireless-security").and_then(|w| get(w, "key-mgmt"));
            out.push(Profile {
                path,
                ssid: String::from_utf8_lossy(&ssid).into_owned(),
                secure: mgmt.is_some_and(|m| m != "none" && m != "owe"),
            });
        }
        Ok(out)
    }

    /// The access points the Wi-Fi device sees (`rescan`: after a fresh scan).
    async fn access_points(&self, dev: &OwnedObjectPath, rescan: bool) -> Result<Vec<Ap>, Error> {
        let w = self.wireless(dev).await?;
        if rescan {
            let before = w.last_scan().await.unwrap_or(0);
            // Refused when it scanned a moment ago: the list it has is fresh.
            if w.request_scan(HashMap::new()).await.is_ok() {
                let end = tokio::time::Instant::now() + SCAN_WAIT;
                while tokio::time::Instant::now() < end && w.last_scan().await.unwrap_or(0) == before {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
        let mut aps = vec![];
        for p in w.get_all_access_points().await? {
            if let Ok(a) = self.ap(p).await {
                aps.push(a);
            }
        }
        Ok(aps)
    }

    /// Asks for a fresh scan (NetworkManager's list is a few minutes old
    /// otherwise), without waiting for it.
    pub async fn rescan(&self) -> Result<(), Error> {
        if let Some(dev) = self.wifi_device().await? {
            let _ = self.wireless(&dev).await?.request_scan(HashMap::new()).await;
        }
        Ok(())
    }

    /// A device's address (IPv4, else IPv6) and its DNS servers.
    async fn addresses(&self, dev: &DeviceProxy<'_>) -> Addresses {
        let mut ip = None;
        let mut dns = vec![];
        if let Ok(p) = dev.ip4_config().await
            && p.as_str() != "/"
            && let Ok(c) = Ip4ConfigProxy::builder(&self.conn).path(p).map(|b| b.cache_properties(NO_CACHE))
            && let Ok(c) = c.build().await
        {
            ip = c.address_data().await.ok().and_then(|a| a.first().and_then(|a| get::<String>(a, "address")));
            dns = c
                .nameserver_data()
                .await
                .map(|d| d.iter().filter_map(|n| get::<String>(n, "address")).collect())
                .unwrap_or_default();
        }
        if ip.is_none()
            && let Ok(p) = dev.ip6_config().await
            && p.as_str() != "/"
            && let Ok(c) = Ip6ConfigProxy::builder(&self.conn).path(p).map(|b| b.cache_properties(NO_CACHE))
            && let Ok(c) = c.build().await
        {
            ip = c.address_data().await.ok().and_then(|a| a.first().and_then(|a| get::<String>(a, "address")));
        }
        (ip, dns)
    }

    /// The network now: Wi-Fi (its radio, what it's on, the networks in
    /// range and saved: one per name, the strongest; hidden ones left out)
    /// and the wire.
    pub async fn state(&self) -> Result<State, Error> {
        let nm = self.nm().await?;
        let mut s = State {
            wifi_on: nm.wireless_enabled().await?,
            wifi_hardware: nm.wireless_hardware_enabled().await.unwrap_or(true),
            ..State::default()
        };
        let saved = self.profiles().await?;
        let mut wifi_addr = None;
        if let Some(dev) = self.wifi_device().await? {
            s.has_wifi = true;
            let d = self.device(&dev).await?;
            let active = self.wireless(&dev).await?.active_access_point().await.ok().filter(|p| p.as_str() != "/");
            let up = d.state().await.unwrap_or(0) == DEVICE_ACTIVATED;
            let mut nets: Vec<WifiNetwork> = vec![];
            for a in self.access_points(&dev, false).await? {
                if a.ssid.is_empty() {
                    continue;
                }
                let is_active = up && active.as_ref() == Some(&a.path);
                let net = WifiNetwork {
                    signal: a.signal,
                    secure: is_secure(a.flags, a.wpa, a.rsn),
                    saved: saved.iter().any(|p| p.ssid == a.ssid),
                    active: is_active,
                    band: band(a.freq),
                    security: Some(security_words(a.flags, a.wpa, a.rsn)),
                    ssid: a.ssid,
                };
                match nets.iter_mut().find(|b| b.ssid == net.ssid) {
                    Some(b) if b.signal >= net.signal => b.active |= is_active,
                    Some(b) => {
                        let was = b.active;
                        *b = net;
                        b.active |= was;
                    }
                    None => nets.push(net),
                }
            }
            s.connected = nets.iter().find(|n| n.active).map(|n| n.ssid.clone());
            for p in &saved {
                if !nets.iter().any(|n| n.ssid == p.ssid) {
                    nets.push(WifiNetwork {
                        ssid: p.ssid.clone(),
                        signal: 0,
                        secure: p.secure,
                        saved: true,
                        active: false,
                        band: None,
                        security: None,
                    });
                }
            }
            order(&mut nets);
            s.networks = nets;
            if s.connected.is_some() {
                wifi_addr = Some(self.addresses(&d).await);
            }
        }
        // The wire: one with a cable in, if any.
        let mut wired: Option<(Ethernet, Addresses)> = None;
        for dev in self.devices(DEVICE_ETHERNET).await? {
            let w = WiredProxy::builder(&self.conn).path(dev.clone())?.cache_properties(NO_CACHE).build().await?;
            if !w.carrier().await.unwrap_or(false) {
                continue;
            }
            let d = self.device(&dev).await?;
            let up = d.state().await.unwrap_or(0) == DEVICE_ACTIVATED;
            let addr = if up { self.addresses(&d).await } else { (None, vec![]) };
            let speed = w.speed().await.ok().filter(|s| *s > 0);
            wired = Some((Ethernet { plugged: true, speed_mbps: speed, ip: addr.0.clone() }, addr));
            break;
        }
        let wire_addr = wired.as_ref().map(|(_, a)| a.clone());
        s.ethernet = wired.map(|(e, _)| e).unwrap_or_default();
        if let Some((ip, dns)) = wifi_addr.filter(|(ip, _)| ip.is_some()).or(wire_addr) {
            s.ip = ip;
            s.dns = dns;
        }
        Ok(s)
    }

    /// Wi-Fi's radio on or off.
    pub async fn set_wifi(&self, on: bool) -> Result<(), Error> {
        let _one = self.lock.lock().await;
        let nm = self.nm().await?;
        if on && !nm.wireless_hardware_enabled().await.unwrap_or(true) {
            return Err(Error::Blocked);
        }
        nm.set_wireless_enabled(on).await?;
        Ok(())
    }

    /// Joins a network: a saved one with its saved password, a new one (or
    /// a saved one again) with `password` (none for an open network).
    pub async fn connect(&self, ssid: &str, password: Option<&str>) -> Result<(), Error> {
        let _one = self.lock.lock().await;
        let password = password.filter(|p| !p.is_empty());
        let nm = self.nm().await?;
        let dev = self.wifi_device().await?.ok_or(Error::NoWifi)?;
        if !nm.wireless_enabled().await? {
            return Err(Error::WifiOff);
        }
        let mut aps = self.access_points(&dev, false).await?;
        if !aps.iter().any(|a| a.ssid == ssid) {
            aps = self.access_points(&dev, true).await?;
        }
        let ap = aps
            .iter()
            .filter(|a| a.ssid == ssid)
            .max_by_key(|a| a.signal)
            .ok_or_else(|| Error::NotInRange(ssid.into()))?;
        let Some(mgmt) = key_mgmt(ap.flags, ap.wpa, ap.rsn) else {
            return Err(Error::Unsupported(format!(
                "{ssid} is a {} network: those can't be joined here",
                security(ap.flags, ap.wpa, ap.rsn)
            )));
        };
        let needs_password = mgmt == "wpa-psk" || mgmt == "sae";
        let saved: Vec<OwnedObjectPath> =
            self.profiles().await?.into_iter().filter(|p| p.ssid == ssid).map(|p| p.path).collect();
        if password.is_none() || !needs_password {
            if let Some(profile) = saved.first() {
                let active = nm.activate_connection(profile, &dev, &ap.path).await?;
                return self.wait_activated(&active, &dev, ssid, needs_password).await;
            }
            if needs_password {
                return Err(Error::NeedsPassword(ssid.into()));
            }
        }
        if let Some(p) = password
            && needs_password
            && !valid_psk(p)
        {
            return Err(Error::BadPassword);
        }
        // A new password (or an open network): a fresh profile.
        for p in &saved {
            self.profile(p).await?.delete().await?;
        }
        let mut connection: HashMap<&str, Value> = HashMap::new();
        connection.insert("id", Value::from(ssid));
        connection.insert("type", Value::from("802-11-wireless"));
        connection.insert("autoconnect", Value::from(true));
        let mut wireless: HashMap<&str, Value> = HashMap::new();
        wireless.insert("ssid", Value::from(ssid.as_bytes().to_vec()));
        wireless.insert("mode", Value::from("infrastructure"));
        let mut settings: HashMap<&str, HashMap<&str, Value>> = HashMap::new();
        settings.insert("connection", connection);
        settings.insert("802-11-wireless", wireless);
        if !mgmt.is_empty() {
            let mut sec: HashMap<&str, Value> = HashMap::new();
            sec.insert("key-mgmt", Value::from(mgmt));
            if let Some(p) = password.filter(|_| needs_password) {
                sec.insert("psk", Value::from(p));
                // Kept with the profile (system-owned), so it reconnects at boot.
                sec.insert("psk-flags", Value::from(0u32));
            }
            settings.insert("802-11-wireless-security", sec);
        }
        let (profile, active) = nm.add_and_activate_connection(settings, &dev, &ap.path).await?;
        if let Err(e) = self.wait_activated(&active, &dev, ssid, needs_password).await {
            // Don't leave a profile with a wrong password retrying.
            if let Ok(c) = self.profile(&profile).await {
                let _ = c.delete().await;
            }
            return Err(e);
        }
        Ok(())
    }

    async fn wait_activated(
        &self,
        active: &OwnedObjectPath,
        dev: &OwnedObjectPath,
        ssid: &str,
        secure: bool,
    ) -> Result<(), Error> {
        let a = ActiveProxy::builder(&self.conn).path(active.clone())?.cache_properties(NO_CACHE).build().await?;
        let end = tokio::time::Instant::now() + JOIN_TIMEOUT;
        loop {
            match a.state().await {
                Ok(ACTIVATED) => return Ok(()),
                Ok(DEACTIVATED) | Err(_) => {
                    let reason = self.device(dev).await?.state_reason().await.map(|(_, r)| r).unwrap_or(0);
                    return Err(join_failure(ssid, secure, reason));
                }
                Ok(_) => {}
            }
            if tokio::time::Instant::now() > end {
                return Err(Error::Failed(format!("{ssid} didn't answer within {} s", JOIN_TIMEOUT.as_secs())));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Leaves the Wi-Fi network it's on (the profile stays: it rejoins at boot).
    pub async fn disconnect(&self) -> Result<(), Error> {
        let _one = self.lock.lock().await;
        let dev = self.wifi_device().await?.ok_or(Error::NoWifi)?;
        self.device(&dev).await?.disconnect().await?;
        Ok(())
    }

    /// Forgets a saved network (leaving it, if it's on it).
    pub async fn forget(&self, ssid: &str) -> Result<(), Error> {
        let _one = self.lock.lock().await;
        let mut any = false;
        for p in self.profiles().await? {
            if p.ssid == ssid {
                self.profile(&p.path).await?.delete().await?;
                any = true;
            }
        }
        if any { Ok(()) } else { Err(Error::NotSaved(ssid.into())) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_like_nmcli() {
        assert_eq!(security(1, 0, 0x188), "WPA2"); // a home router
        assert_eq!(security(1, 0x148, 0x188), "WPA1 WPA2");
        assert_eq!(security(1, 0, 0x588), "WPA2 WPA3"); // PSK + SAE
        assert_eq!(security(1, 0, 0x408), "WPA3");
        assert_eq!(security(1, 0, 0x288), "WPA2 802.1X");
        assert_eq!(security(1, 0, 0), "WEP");
        assert_eq!(security(0, 0, 0), "");
        assert!(is_secure(1, 0, 0x188) && !is_secure(0, 0, 0) && !is_secure(0, 0, 0x808));
        assert_eq!(security_words(1, 0, 0x188), "WPA2 Personal");
        assert_eq!(security_words(1, 0, 0x588), "WPA2/WPA3 Personal");
        assert_eq!(security_words(1, 0, 0x408), "WPA3 Personal");
        assert_eq!(security_words(1, 0, 0x288), "WPA2 Enterprise");
        assert_eq!(security_words(0, 0, 0), "None (open network)");
        assert_eq!(security_words(0, 0, 0x808), "Enhanced Open (OWE)");
    }

    #[test]
    fn what_can_be_joined() {
        assert_eq!(key_mgmt(1, 0, 0x188), Some("wpa-psk"));
        assert_eq!(key_mgmt(1, 0, 0x588), Some("wpa-psk")); // PSK and SAE: PSK works for both
        assert_eq!(key_mgmt(1, 0, 0x408), Some("sae"));
        assert_eq!(key_mgmt(0, 0, 0), Some(""));
        assert_eq!(key_mgmt(0, 0, 0x808), Some("owe"));
        assert_eq!(key_mgmt(1, 0, 0x288), None); // enterprise
        assert_eq!(key_mgmt(1, 0, 0), None); // WEP
        assert!(valid_psk("12345678") && valid_psk(&"a".repeat(63)) && valid_psk(&"0f".repeat(32)));
        assert!(!valid_psk("short") && !valid_psk(&"z".repeat(64)));
    }

    #[test]
    fn bands_and_why_a_join_failed() {
        assert_eq!(band(2437), Some(WifiBand::Ghz2));
        assert_eq!(band(5180), Some(WifiBand::Ghz5));
        assert_eq!(band(5955), Some(WifiBand::Ghz6));
        assert_eq!(band(0), None);
        assert!(matches!(join_failure("Home", true, REASON_NO_SECRETS), Error::WrongPassword(s) if s == "Home"));
        assert!(matches!(join_failure("Home", true, REASON_SUPPLICANT_DISCONNECT), Error::WrongPassword(_)));
        // An open one has no password to be wrong.
        assert!(matches!(join_failure("Cafe", false, REASON_NO_SECRETS), Error::Failed(_)));
        assert_eq!(
            join_failure("Home", true, REASON_SUPPLICANT_TIMEOUT).to_string(),
            "Home didn't answer: is it still in range?"
        );
    }

    #[test]
    fn the_one_its_on_then_the_saved_ones_then_by_signal() {
        let n = |ssid: &str, signal, saved, active| WifiNetwork {
            ssid: ssid.into(),
            signal,
            secure: true,
            saved,
            active,
            band: None,
            security: None,
        };
        let mut v = vec![n("Far", 20, false, false), n("Office", 0, true, false), n("Near", 90, false, false)];
        v.push(n("Home", 60, true, true));
        v.push(n("Cafe", 70, true, false));
        order(&mut v);
        let names: Vec<&str> = v.iter().map(|w| w.ssid.as_str()).collect();
        assert_eq!(names, ["Home", "Cafe", "Office", "Near", "Far"]);
    }
}

/// Reads the real NetworkManager (changes nothing):
/// `cargo test -p wad-net -- --ignored --nocapture`.
#[cfg(test)]
mod real {
    #[tokio::test]
    #[ignore = "reads this machine's NetworkManager"]
    async fn reads_the_real_network_manager() {
        let nm = super::NetworkManager::system().await.unwrap();
        let s = nm.state().await.unwrap();
        eprintln!(
            "wifi {} (hw {}), on {:?}, ip {:?}, dns {:?}, ethernet {:?}",
            s.wifi_on, s.wifi_hardware, s.connected, s.ip, s.dns, s.ethernet
        );
        for n in s.networks.iter().take(10) {
            eprintln!(
                "{:>3} {:<6} {:<5} {:<5} {:?} {}",
                n.signal,
                if n.secure { "secure" } else { "" },
                if n.active { "*" } else { "" },
                if n.saved { "saved" } else { "" },
                n.band,
                n.ssid
            );
        }
    }
}
