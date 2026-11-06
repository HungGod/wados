//! A fake BlueZ on a bus the test gives (wad-net's `fake::PrivateBus`:
//! never the machine's): an adapter (hci0) and the devices the test
//! describes. Pairing one that asks calls the registered agent back over
//! the bus, as bluetoothd does, and pairs it if the agent says yes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const ADAPTER: &str = "/org/bluez/hci0";

/// What a device asks when it's paired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asks {
    /// Nothing (it just pairs).
    Nothing,
    /// RequestConfirmation with this passkey.
    Confirm(u32),
    /// RequestPinCode: it pairs with this PIN only.
    Pin(&'static str),
}

#[derive(Debug, Clone)]
pub struct FakeDevice {
    pub address: String,
    /// None: a nameless one (a beacon).
    pub name: Option<String>,
    pub icon: String,
    pub paired: bool,
    pub trusted: bool,
    pub connected: bool,
    pub battery: Option<u8>,
    pub rssi: Option<i16>,
    pub asks: Asks,
    /// Connecting fails (it's off, or out of range).
    pub unreachable: bool,
}

impl FakeDevice {
    pub fn new(address: &str, name: &str, icon: &str) -> Self {
        Self {
            address: address.into(),
            name: Some(name.into()),
            icon: icon.into(),
            paired: false,
            trusted: false,
            connected: false,
            battery: None,
            rssi: Some(-60),
            asks: Asks::Nothing,
            unreachable: false,
        }
    }

    pub fn paired(mut self, connected: bool) -> Self {
        self.paired = true;
        self.trusted = true;
        self.connected = connected;
        self
    }

    pub fn battery(mut self, b: u8) -> Self {
        self.battery = Some(b);
        self
    }

    pub fn asks(mut self, a: Asks) -> Self {
        self.asks = a;
        self
    }
}

#[derive(Debug, Default)]
pub struct World {
    pub powered: bool,
    pub alias: String,
    pub discovering: bool,
    pub discoverable: bool,
    pub devices: Vec<FakeDevice>,
    /// The agent registered: (its bus name, its path), and whether it's the default.
    pub agent: Option<(String, String)>,
    pub default_agent: bool,
}

pub type W = Arc<Mutex<World>>;

impl World {
    pub fn with(alias: &str, devices: Vec<FakeDevice>) -> W {
        Arc::new(Mutex::new(World { powered: true, alias: alias.into(), devices, ..Default::default() }))
    }
}

fn dev_path(address: &str) -> String {
    format!("{ADAPTER}/dev_{}", address.replace(':', "_"))
}

fn ov(v: Value<'_>) -> OwnedValue {
    OwnedValue::try_from(v).expect("a value")
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum BzError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NotReady(String),
    AlreadyExists(String),
    AuthenticationRejected(String),
    Failed(String),
    DoesNotExist(String),
}

struct ObjectManager(W);

#[zbus::interface(name = "org.freedesktop.DBus.ObjectManager")]
impl ObjectManager {
    fn get_managed_objects(&self) -> HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> {
        let w = self.0.lock().unwrap();
        let mut out = HashMap::new();
        let adapter = HashMap::from([
            ("Powered".to_string(), ov(Value::from(w.powered))),
            ("Alias".to_string(), ov(Value::from(w.alias.clone()))),
            ("Discovering".to_string(), ov(Value::from(w.discovering))),
            ("Discoverable".to_string(), ov(Value::from(w.discoverable))),
        ]);
        out.insert(
            OwnedObjectPath::try_from(ADAPTER).expect("path"),
            HashMap::from([("org.bluez.Adapter1".to_string(), adapter)]),
        );
        if w.powered {
            for d in &w.devices {
                let mut p = HashMap::from([
                    ("Address".to_string(), ov(Value::from(d.address.clone()))),
                    (
                        "Alias".to_string(),
                        ov(Value::from(d.name.clone().unwrap_or_else(|| d.address.replace(':', "-")))),
                    ),
                    ("Icon".to_string(), ov(Value::from(d.icon.clone()))),
                    ("Paired".to_string(), ov(Value::from(d.paired))),
                    ("Trusted".to_string(), ov(Value::from(d.trusted))),
                    ("Connected".to_string(), ov(Value::from(d.connected))),
                ]);
                if let Some(n) = &d.name {
                    p.insert("Name".into(), ov(Value::from(n.clone())));
                }
                if let (Some(r), true) = (d.rssi, w.discovering) {
                    p.insert("RSSI".into(), ov(Value::from(r)));
                }
                let mut ifaces = HashMap::from([("org.bluez.Device1".to_string(), p)]);
                if let Some(b) = d.battery {
                    ifaces.insert(
                        "org.bluez.Battery1".into(),
                        HashMap::from([("Percentage".to_string(), ov(Value::from(b)))]),
                    );
                }
                out.insert(OwnedObjectPath::try_from(dev_path(&d.address)).expect("path"), ifaces);
            }
        }
        out
    }
}

struct AgentManager(W);

#[zbus::interface(name = "org.bluez.AgentManager1")]
impl AgentManager {
    fn register_agent(
        &self,
        agent: OwnedObjectPath,
        _capability: String,
        #[zbus(header)] h: zbus::message::Header<'_>,
    ) -> Result<(), BzError> {
        let mut w = self.0.lock().unwrap();
        if w.agent.is_some() {
            return Err(BzError::AlreadyExists("an agent is registered".into()));
        }
        let sender = h.sender().map(|s| s.to_string()).unwrap_or_default();
        w.agent = Some((sender, agent.to_string()));
        Ok(())
    }

    fn request_default_agent(&self, _agent: OwnedObjectPath) {
        self.0.lock().unwrap().default_agent = true;
    }
}

struct Adapter(W);

#[zbus::interface(name = "org.bluez.Adapter1")]
impl Adapter {
    fn start_discovery(&self) -> Result<(), BzError> {
        let mut w = self.0.lock().unwrap();
        if !w.powered {
            return Err(BzError::NotReady("Resource Not Ready".into()));
        }
        w.discovering = true;
        Ok(())
    }

    fn stop_discovery(&self) -> Result<(), BzError> {
        let mut w = self.0.lock().unwrap();
        if !w.discovering {
            return Err(BzError::Failed("No discovery started".into()));
        }
        w.discovering = false;
        Ok(())
    }

    fn remove_device(&self, device: OwnedObjectPath) -> Result<(), BzError> {
        let mut w = self.0.lock().unwrap();
        let d = w
            .devices
            .iter_mut()
            .find(|d| dev_path(&d.address) == device.as_str())
            .ok_or_else(|| BzError::DoesNotExist("no such device".into()))?;
        // Gone from the list of paired ones; still in range.
        d.paired = false;
        d.trusted = false;
        d.connected = false;
        Ok(())
    }

    #[zbus(property)]
    fn powered(&self) -> bool {
        self.0.lock().unwrap().powered
    }
    #[zbus(property)]
    fn set_powered(&self, on: bool) {
        let mut w = self.0.lock().unwrap();
        w.powered = on;
        if !on {
            w.discovering = false;
            w.discoverable = false;
            for d in &mut w.devices {
                d.connected = false;
            }
        }
    }
    #[zbus(property)]
    fn discoverable(&self) -> bool {
        self.0.lock().unwrap().discoverable
    }
    #[zbus(property)]
    fn set_discoverable(&self, on: bool) {
        self.0.lock().unwrap().discoverable = on;
    }
}

struct Device(W, String);

impl Device {
    fn with<T>(&self, f: impl FnOnce(&mut FakeDevice) -> T) -> T {
        let mut w = self.0.lock().unwrap();
        let d = w.devices.iter_mut().find(|d| d.address == self.1).expect("the device");
        f(d)
    }
}

#[zbus::interface(name = "org.bluez.Device1")]
impl Device {
    async fn pair(&self, #[zbus(connection)] conn: &zbus::Connection) -> Result<(), BzError> {
        let (asks, paired) = self.with(|d| (d.asks, d.paired));
        if paired {
            return Err(BzError::AlreadyExists("Already Paired".into()));
        }
        let agent = self.0.lock().unwrap().agent.clone();
        let path = OwnedObjectPath::try_from(dev_path(&self.1)).expect("path");
        let ok = match (asks, agent) {
            (Asks::Nothing, _) => true,
            (_, None) => false,
            (Asks::Confirm(passkey), Some((name, at))) => conn
                .call_method(
                    Some(name.as_str()),
                    at.as_str(),
                    Some("org.bluez.Agent1"),
                    "RequestConfirmation",
                    &(path, passkey),
                )
                .await
                .is_ok(),
            (Asks::Pin(pin), Some((name, at))) => {
                match conn
                    .call_method(Some(name.as_str()), at.as_str(), Some("org.bluez.Agent1"), "RequestPinCode", &(path,))
                    .await
                {
                    Ok(m) => m.body().deserialize::<String>().is_ok_and(|p| p == pin),
                    Err(_) => false,
                }
            }
        };
        if !ok {
            return Err(BzError::AuthenticationRejected("Authentication Rejected".into()));
        }
        self.with(|d| d.paired = true);
        Ok(())
    }

    fn cancel_pairing(&self) {}

    fn connect(&self) -> Result<(), BzError> {
        self.with(|d| {
            if d.unreachable {
                return Err(BzError::Failed("br-connection-page-timeout".into()));
            }
            d.connected = true;
            Ok(())
        })
    }

    fn disconnect(&self) {
        self.with(|d| d.connected = false);
    }

    #[zbus(property)]
    fn address(&self) -> String {
        self.1.clone()
    }
    #[zbus(property)]
    fn alias(&self) -> String {
        self.with(|d| d.name.clone().unwrap_or_else(|| d.address.replace(':', "-")))
    }
    #[zbus(property)]
    fn paired(&self) -> bool {
        self.with(|d| d.paired)
    }
    #[zbus(property)]
    fn trusted(&self) -> bool {
        self.with(|d| d.trusted)
    }
    #[zbus(property)]
    fn set_trusted(&self, yes: bool) {
        self.with(|d| d.trusted = yes);
    }
}

/// The fake BlueZ, served on the bus at `address` for as long as it's kept.
pub struct FakeBluez {
    _conn: zbus::Connection,
}

impl FakeBluez {
    pub async fn serve(address: &str, world: W) -> Self {
        let mut b = zbus::connection::Builder::address(address)
            .expect("address")
            .name("org.bluez")
            .expect("name")
            .serve_at("/", ObjectManager(world.clone()))
            .expect("served")
            .serve_at("/org/bluez", AgentManager(world.clone()))
            .expect("served")
            .serve_at(ADAPTER, Adapter(world.clone()))
            .expect("served");
        let addresses: Vec<String> = world.lock().unwrap().devices.iter().map(|d| d.address.clone()).collect();
        for a in addresses {
            b = b.serve_at(dev_path(&a), Device(world.clone(), a.clone())).expect("served");
        }
        Self { _conn: b.build().await.expect("the fake BlueZ") }
    }
}
