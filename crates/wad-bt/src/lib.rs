//! Bluetooth through BlueZ over D-Bus: the adapter on and off, looking for
//! devices (and being seen meanwhile), pairing (then trusting and
//! connecting), connecting, disconnecting and forgetting. wadd makes it
//! wad-proto's [`wad_proto::Bluetooth`].
//!
//! Pairing goes through an agent of its own (org.bluez.Agent1, registered
//! as KeyboardDisplay and the default): what BlueZ asks (does the device
//! show this code? type this code on the keyboard; the PIN an old device
//! wants; may it connect?) goes out as an [`AgentEvent`] with a
//! [`wad_proto::BtPrompt`] for wadd to show, and the answer comes back
//! through the reply it carries. Once paired, a device is trusted, so it
//! connects again by itself.
//!
//! wadd runs as root on a machine, so BlueZ takes everything from it.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use wad_proto::{BtAsk, BtDevice, BtKind, BtPrompt};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

#[cfg(feature = "fake")]
pub mod fake;
mod proxies;
use proxies::*;

pub const AGENT_PATH: &str = "/io/wados/agent";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const BATTERY: &str = "org.bluez.Battery1";
/// How long a prompt waits for its answer (BlueZ gives up at about 30 s for some).
const ASK_FOR: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Bluetooth: {0}")]
    Bus(#[from] zbus::Error),
    #[error("This machine has no Bluetooth")]
    NoAdapter,
    #[error("Bluetooth is off")]
    Off,
    #[error("There's no Bluetooth device {0}")]
    NotFound(String),
    #[error("{0:?} isn't a Bluetooth address")]
    BadAddress(String),
    #[error("{0}")]
    Refused(String),
}

/// What pairing asks; `reply` (none: nothing to answer, only to show) takes the answer.
#[derive(Debug)]
pub struct Ask {
    pub prompt: BtPrompt,
    pub reply: Option<oneshot::Sender<Answer>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub accept: bool,
    pub pin: Option<String>,
}

/// From the agent to wadd.
#[derive(Debug)]
pub enum AgentEvent {
    Ask(Ask),
    /// It's over: BlueZ called it off, or pairing ended.
    Done,
}

/// What a device is, from BlueZ's icon name (its class of device).
pub fn kind_of(icon: &str) -> BtKind {
    match icon {
        "audio-headphones" | "audio-headset" => BtKind::Headphones,
        "audio-card" | "audio-speakers" | "multimedia-player" => BtKind::Speaker,
        "input-gaming" => BtKind::Controller,
        "input-keyboard" => BtKind::Keyboard,
        "input-mouse" | "input-tablet" => BtKind::Mouse,
        "phone" => BtKind::Phone,
        _ => BtKind::Other,
    }
}

pub fn is_address(s: &str) -> bool {
    s.len() == 17
        && s.split(':').count() == 6
        && s.split(':').all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// A device as BlueZ has it (`rssi`: how near, while it's being looked for).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub device: BtDevice,
    /// It has a name of its own (not just its address).
    pub named: bool,
    pub rssi: Option<i16>,
}

/// Paired ones first (connected first), then what's in range, nearest
/// first; nameless devices in range (beacons, phones' random addresses)
/// left out.
pub fn order(mut seen: Vec<Seen>) -> Vec<BtDevice> {
    seen.retain(|s| s.device.paired || s.named);
    seen.sort_by(|a, b| {
        let key = |s: &Seen| {
            (!s.device.paired, !s.device.connected, -(s.rssi.unwrap_or(-127) as i32), s.device.name.to_lowercase())
        };
        key(a).cmp(&key(b))
    });
    seen.into_iter().map(|s| s.device).collect()
}

fn prop<T: TryFrom<OwnedValue>>(props: &HashMap<String, OwnedValue>, k: &str) -> Option<T> {
    props.get(k).and_then(|v| v.try_clone().ok()).and_then(|v| T::try_from(v).ok())
}

/// The adapter as BlueZ has it now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    pub on: bool,
    /// Its name, as others see it.
    pub alias: String,
    pub discovering: bool,
    /// Paired first, then the ones in range ([`order`]).
    pub devices: Vec<BtDevice>,
    /// Being paired or connected now (by address).
    pub busy: Vec<String>,
}

pub struct Bluez {
    conn: zbus::Connection,
    /// Addresses being paired or connected.
    busy: Mutex<HashSet<String>>,
    agent_on: AtomicBool,
    agent: Agent,
    /// Soft-unblocks the radio (rfkill) to turn it on: the machine's only.
    rfkill: bool,
}

impl Bluez {
    /// On the system bus, with its agent (`events` gets what it asks).
    pub async fn system(events: mpsc::UnboundedSender<AgentEvent>) -> Result<Self, Error> {
        let mut b = Self::on(zbus::Connection::system().await?, events).await?;
        b.rfkill = true;
        Ok(b)
    }

    /// On this bus (tests: a private one), with its agent.
    pub async fn on(conn: zbus::Connection, events: mpsc::UnboundedSender<AgentEvent>) -> Result<Self, Error> {
        let agent = Agent { events };
        conn.object_server().at(AGENT_PATH, agent.clone()).await?;
        let me = Self { conn, busy: Mutex::default(), agent_on: AtomicBool::new(false), agent, rfkill: false };
        me.ensure_agent().await;
        Ok(me)
    }

    /// Registers the agent with BlueZ (again after bluetoothd restarted).
    async fn ensure_agent(&self) {
        if self.agent_on.load(Ordering::Relaxed) {
            return;
        }
        let r = async {
            let m = AgentManagerProxy::new(&self.conn).await?;
            let path = OwnedObjectPath::try_from(AGENT_PATH).expect("path");
            m.register_agent(&path, "KeyboardDisplay").await?;
            m.request_default_agent(&path).await
        }
        .await;
        match r {
            Ok(()) => self.agent_on.store(true, Ordering::Relaxed),
            Err(e) if e.to_string().contains("AlreadyExists") => self.agent_on.store(true, Ordering::Relaxed),
            Err(e) => tracing::debug!("bluetooth agent: {e}"),
        }
    }

    async fn objects(&self) -> Result<Objects, Error> {
        match ObjectManagerProxy::new(&self.conn).await?.get_managed_objects().await {
            Ok(o) => Ok(o),
            Err(e) => {
                // bluetoothd went away: its agent with it.
                self.agent_on.store(false, Ordering::Relaxed);
                Err(e.into())
            }
        }
    }

    /// The first adapter's path.
    fn adapter_path(objects: &Objects) -> Option<OwnedObjectPath> {
        objects
            .iter()
            .filter(|(_, i)| i.contains_key(ADAPTER))
            .map(|(p, _)| p.clone())
            .min_by(|a, b| a.as_str().cmp(b.as_str()))
    }

    async fn adapter(&self, objects: &Objects) -> Result<AdapterProxy<'static>, Error> {
        let path = Self::adapter_path(objects).ok_or(Error::NoAdapter)?;
        Ok(AdapterProxy::builder(&self.conn)
            .path(path)?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?)
    }

    async fn powered(&self, objects: &Objects) -> Result<bool, Error> {
        Ok(self.adapter(objects).await?.powered().await?)
    }

    /// A device by its address (any case), while Bluetooth is on.
    async fn device(&self, address: &str) -> Result<(Objects, OwnedObjectPath, DeviceProxy<'static>), Error> {
        if !is_address(address) {
            return Err(Error::BadAddress(address.into()));
        }
        let objects = self.objects().await?;
        if !self.powered(&objects).await? {
            return Err(Error::Off);
        }
        let path = objects
            .iter()
            .find(|(_, i)| {
                i.get(DEVICE)
                    .and_then(|p| prop::<String>(p, "Address"))
                    .is_some_and(|a| a.eq_ignore_ascii_case(address))
            })
            .map(|(p, _)| p.clone())
            .ok_or_else(|| Error::NotFound(address.to_uppercase()))?;
        let proxy = DeviceProxy::builder(&self.conn)
            .path(path.clone())?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        Ok((objects, path, proxy))
    }

    /// The adapter and its devices; none: there's no adapter.
    pub async fn state(&self) -> Result<Option<State>, Error> {
        self.ensure_agent().await;
        let objects = self.objects().await?;
        let Some(path) = Self::adapter_path(&objects) else { return Ok(None) };
        let a = &objects[&path][ADAPTER];
        let under = format!("{}/", path.as_str());
        let seen = objects
            .iter()
            .filter(|(p, _)| p.as_str().starts_with(&under))
            .filter_map(|(_, i)| {
                let d = i.get(DEVICE)?;
                let address: String = prop(d, "Address")?;
                let name: Option<String> = prop(d, "Name");
                let alias: Option<String> = prop(d, "Alias");
                let icon: String = prop(d, "Icon").unwrap_or_default();
                let connected = prop(d, "Connected").unwrap_or(false);
                let battery = i.get(BATTERY).and_then(|b| prop::<u8>(b, "Percentage")).filter(|_| connected);
                let named = name.is_some() || alias.as_ref().is_some_and(|a| a.replace('-', ":") != address);
                Some(Seen {
                    device: BtDevice {
                        name: alias.or(name).unwrap_or_else(|| address.clone()),
                        kind: kind_of(&icon),
                        paired: prop(d, "Paired").unwrap_or(false),
                        connected,
                        battery,
                        address,
                    },
                    named,
                    rssi: prop(d, "RSSI"),
                })
            })
            .collect();
        let mut busy: Vec<String> = self.busy.lock().unwrap().iter().cloned().collect();
        busy.sort();
        Ok(Some(State {
            on: prop(a, "Powered").unwrap_or(false),
            alias: prop(a, "Alias").unwrap_or_default(),
            discovering: prop(a, "Discovering").unwrap_or(false),
            devices: order(seen),
            busy,
        }))
    }

    /// Turns the adapter on or off (on: unblocking its radio first, when
    /// it's soft-blocked).
    pub async fn power(&self, on: bool) -> Result<(), Error> {
        let adapter = self.adapter(&self.objects().await?).await?;
        if on && self.rfkill {
            unblock();
        }
        match adapter.set_powered(on).await {
            Err(e) if on && self.rfkill && e.to_string().contains("Blocked") => {
                unblock();
                tokio::time::sleep(Duration::from_millis(300)).await;
                adapter.set_powered(true).await?;
            }
            r => r?,
        }
        Ok(())
    }

    /// Looks for devices to pair, and is seen by them (on), or stops.
    pub async fn scan(&self, on: bool) -> Result<(), Error> {
        let objects = self.objects().await?;
        let adapter = self.adapter(&objects).await?;
        if on && !adapter.powered().await? {
            return Err(Error::Off);
        }
        let _ = adapter.set_discoverable(on).await;
        let r = if on { adapter.start_discovery().await } else { adapter.stop_discovery().await };
        match r {
            // Already as asked.
            Err(e) if e.to_string().contains("InProgress") || (!on && e.to_string().contains("NotReady")) => Ok(()),
            Err(e) if !on && e.to_string().contains("Failed") => Ok(()),
            r => r.map_err(Error::from),
        }
    }

    fn busy(&self, address: &str, yes: bool) {
        let mut b = self.busy.lock().unwrap();
        if yes {
            b.insert(address.to_uppercase());
        } else {
            b.remove(&address.to_uppercase());
        }
    }

    /// Pairs, trusts and connects a device (the agent asks what it must),
    /// once it's done.
    pub async fn pair(&self, address: &str) -> Result<(), Error> {
        let (_, _, d) = self.device(address).await?;
        self.busy(address, true);
        let r = async {
            match d.pair().await {
                Err(e) if e.to_string().contains("AlreadyExists") => {}
                r => r?,
            }
            d.set_trusted(true).await?;
            d.connect().await?;
            Ok::<_, Error>(())
        }
        .await;
        self.busy(address, false);
        self.agent.done();
        r.map_err(friendly)
    }

    /// Connects a paired device, once it's connected.
    pub async fn connect(&self, address: &str) -> Result<(), Error> {
        let (_, _, d) = self.device(address).await?;
        if !d.paired().await.unwrap_or(false) {
            return Err(Error::Refused("It isn't paired: pair it first".into()));
        }
        self.busy(address, true);
        let r = d.connect().await;
        self.busy(address, false);
        r.map_err(|e| friendly(e.into()))
    }

    /// Calls off a pairing under way (a code shown that won't be typed).
    pub async fn cancel_pairing(&self, address: &str) -> Result<(), Error> {
        let (_, _, d) = self.device(address).await?;
        let _ = d.cancel_pairing().await;
        Ok(())
    }

    pub async fn disconnect(&self, address: &str) -> Result<(), Error> {
        let (_, _, d) = self.device(address).await?;
        d.disconnect().await?;
        Ok(())
    }

    /// Unpairs and forgets a device.
    pub async fn forget(&self, address: &str) -> Result<(), Error> {
        let (objects, path, _) = self.device(address).await?;
        self.adapter(&objects).await?.remove_device(&path).await?;
        Ok(())
    }
}

/// BlueZ's errors, in words.
fn friendly(e: Error) -> Error {
    let text = e.to_string();
    let says = |s: &str| text.contains(s);
    if says("AuthenticationFailed") || says("AuthenticationRejected") || says("Rejected") || says("Canceled") {
        Error::Refused("Pairing was turned down: the codes didn't match, or it was cancelled".into())
    } else if says("AuthenticationTimeout") || says("Timeout") {
        Error::Refused("It didn't answer in time: put it in pairing mode and try again".into())
    } else if says("ConnectionAttemptFailed") || says("page-timeout") || says("Host is down") {
        Error::Refused("It didn't answer: is it on and close by?".into())
    } else {
        e
    }
}

/// Soft-unblocks the machine's Bluetooth radios (rfkill).
fn unblock() {
    for e in std::fs::read_dir("/sys/class/rfkill").into_iter().flatten().flatten() {
        let p = e.path();
        if std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "bluetooth")
            && std::fs::read_to_string(p.join("soft")).is_ok_and(|s| s.trim() == "1")
        {
            let _ = std::fs::write(p.join("soft"), "0");
        }
    }
}

// ------------------------------------------------------------------ the agent

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected(String),
    Canceled(String),
}

/// The pairing agent: what BlueZ asks goes out as an event, and the answer
/// comes back (or nobody answers in a minute: cancelled).
#[derive(Clone)]
struct Agent {
    events: mpsc::UnboundedSender<AgentEvent>,
}

/// Prompts, numbered.
static PROMPTS: AtomicU64 = AtomicU64::new(1);

/// A PIN an old device can take: 1 to 16 letters and digits.
pub fn valid_pin(p: &str) -> bool {
    !p.is_empty() && p.len() <= 16 && p.chars().all(|c| c.is_ascii_alphanumeric())
}

/// A passkey: up to six digits.
pub fn passkey(p: &str) -> Option<u32> {
    p.parse::<u32>().ok().filter(|p| *p <= 999_999)
}

impl Agent {
    async fn who(conn: &zbus::Connection, device: &OwnedObjectPath) -> (String, String) {
        let all = async {
            let p = zbus::fdo::PropertiesProxy::builder(conn)
                .destination("org.bluez")?
                .path(device.clone())?
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()
                .await?;
            Ok::<_, zbus::Error>(p.get_all(DEVICE.try_into().expect("name")).await?)
        }
        .await
        .unwrap_or_default();
        let address: String = prop(&all, "Address").unwrap_or_default();
        let name: String = prop(&all, "Alias").or_else(|| prop(&all, "Name")).unwrap_or_else(|| address.clone());
        (address, name)
    }

    /// Asks wadd (`answer`: and waits for it; else only shows it).
    async fn ask(
        &self,
        conn: &zbus::Connection,
        device: &OwnedObjectPath,
        kind: BtAsk,
        answer: bool,
    ) -> Result<Answer, AgentError> {
        let (device, name) = Self::who(conn, device).await;
        let id = format!("bt-{}", PROMPTS.fetch_add(1, Ordering::Relaxed));
        let prompt = BtPrompt { id, device, name, kind };
        if !answer {
            let _ = self.events.send(AgentEvent::Ask(Ask { prompt, reply: None }));
            return Ok(Answer { accept: true, pin: None });
        }
        let (tx, rx) = oneshot::channel();
        self.events
            .send(AgentEvent::Ask(Ask { prompt, reply: Some(tx) }))
            .map_err(|_| AgentError::Rejected("nobody to ask".into()))?;
        match tokio::time::timeout(ASK_FOR, rx).await {
            Ok(Ok(a)) if a.accept => Ok(a),
            Ok(Ok(_)) => Err(AgentError::Rejected("turned down".into())),
            _ => Err(AgentError::Canceled("not answered".into())),
        }
    }

    fn done(&self) {
        let _ = self.events.send(AgentEvent::Done);
    }
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    async fn release(&self) {}

    async fn request_pin_code(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        device: OwnedObjectPath,
    ) -> Result<String, AgentError> {
        let a = self.ask(conn, &device, BtAsk::Enter, true).await?;
        a.pin.filter(|p| valid_pin(p)).ok_or_else(|| AgentError::Rejected("no PIN".into()))
    }

    async fn display_pin_code(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        device: OwnedObjectPath,
        pincode: String,
    ) -> Result<(), AgentError> {
        self.ask(conn, &device, BtAsk::Show { code: pincode }, false).await.map(drop)
    }

    async fn request_passkey(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        device: OwnedObjectPath,
    ) -> Result<u32, AgentError> {
        let a = self.ask(conn, &device, BtAsk::Enter, true).await?;
        a.pin.as_deref().and_then(passkey).ok_or_else(|| AgentError::Rejected("no passkey".into()))
    }

    async fn display_passkey(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        device: OwnedObjectPath,
        passkey: u32,
        _entered: u16,
    ) -> Result<(), AgentError> {
        self.ask(conn, &device, BtAsk::Show { code: format!("{passkey:06}") }, false).await.map(drop)
    }

    async fn request_confirmation(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        device: OwnedObjectPath,
        passkey: u32,
    ) -> Result<(), AgentError> {
        self.ask(conn, &device, BtAsk::Confirm { code: format!("{passkey:06}") }, true).await.map(drop)
    }

    async fn request_authorization(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        device: OwnedObjectPath,
    ) -> Result<(), AgentError> {
        self.ask(conn, &device, BtAsk::Authorize, true).await.map(drop)
    }

    /// A paired device using a service: fine (it's trusted once paired).
    async fn authorize_service(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        device: OwnedObjectPath,
        _uuid: String,
    ) -> Result<(), AgentError> {
        let paired = async {
            DeviceProxy::builder(conn)
                .path(device)?
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()
                .await?
                .paired()
                .await
        }
        .await
        .unwrap_or(false);
        if paired { Ok(()) } else { Err(AgentError::Rejected("not paired".into())) }
    }

    async fn cancel(&self) {
        self.done();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(name: &str, paired: bool, connected: bool, rssi: Option<i16>, named: bool) -> Seen {
        Seen {
            device: BtDevice {
                address: "AA:BB:CC:DD:EE:FF".into(),
                name: name.into(),
                kind: BtKind::Other,
                paired,
                connected,
                battery: None,
            },
            named,
            rssi,
        }
    }

    #[test]
    fn paired_first_then_the_nearest_in_range() {
        let list = order(vec![
            seen("Far", false, false, Some(-90), true),
            seen("Pad", true, false, None, true),
            seen("AA-BB-CC-DD-EE-FF", false, false, Some(-40), false),
            seen("Near", false, false, Some(-50), true),
            seen("Buds", true, true, None, true),
        ]);
        let names: Vec<&str> = list.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Buds", "Pad", "Near", "Far"]);
    }

    #[test]
    fn kinds_come_from_the_class_icon() {
        assert_eq!(kind_of("input-gaming"), BtKind::Controller);
        assert_eq!(kind_of("audio-headphones"), BtKind::Headphones);
        assert_eq!(kind_of("audio-headset"), BtKind::Headphones);
        assert_eq!(kind_of("audio-card"), BtKind::Speaker);
        assert_eq!(kind_of("input-keyboard"), BtKind::Keyboard);
        assert_eq!(kind_of("input-tablet"), BtKind::Mouse);
        assert_eq!(kind_of("phone"), BtKind::Phone);
        assert_eq!(kind_of("computer"), BtKind::Other);
        assert!(is_address("A0:AB:51:12:34:56") && !is_address("A0:AB:51:12:34") && !is_address("../x"));
        assert!(valid_pin("0000") && !valid_pin("") && !valid_pin("12 34"));
        assert_eq!((passkey("012345"), passkey("1000000"), passkey("x")), (Some(12345), None, None));
    }
}

/// `cargo test -p wad-bt this_machines_bluetooth -- --ignored --nocapture`
/// (reads it; changes nothing, but registers the agent while it runs).
#[cfg(test)]
#[tokio::test]
#[ignore = "reads this machine's BlueZ"]
async fn this_machines_bluetooth() {
    // No agent: the desktop's stays the one that's asked.
    let conn = zbus::Connection::system().await.unwrap();
    let objects = ObjectManagerProxy::new(&conn).await.unwrap().get_managed_objects().await.unwrap();
    for (p, i) in &objects {
        if let Some(d) = i.get(DEVICE) {
            let name: Option<String> = prop(d, "Alias");
            let icon: Option<String> = prop(d, "Icon");
            println!("{p}: {name:?} {:?} paired={:?}", icon.as_deref().map(kind_of), prop::<bool>(d, "Paired"));
        } else if let Some(a) = i.get(ADAPTER) {
            println!("{p}: powered={:?} alias={:?}", prop::<bool>(a, "Powered"), prop::<String>(a, "Alias"));
        }
    }
}
