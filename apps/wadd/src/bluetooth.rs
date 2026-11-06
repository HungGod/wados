//! The machine's Bluetooth (wad-bt: BlueZ over D-Bus), for Settings ▸
//! Bluetooth and the tray: [`api_phase8::Bluetooth`]. It's looked at every
//! few seconds (every second while it looks for devices, pairs or asks
//! something) and goes out as the `bluetooth` event when it changes.
//!
//! Pairing doesn't wait: the call answers at once, and what BlueZ asks
//! through wad-bt's agent goes out as the `btPrompt` event (none once it's
//! answered, called off, or over); its answer comes back by the prompt's
//! id. It failing is a notice. Looking for devices stops by itself after
//! [`SCAN_FOR`] unless it's asked again (Settings does while its page is open).
//! Airplane mode ([`crate::network::Airplane`]) keeps it off.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{Notify, mpsc, oneshot};
use wad_bt::{AgentEvent, Answer, Bluez};
use wad_proto as proto;
use wad_proto::{ApiError, BtAnswer, BtAsk, BtPrompt, ErrorCode, Event, Level};

use crate::api_phase8;
use crate::events::Bus;
use crate::network::Airplane;

/// Looking for devices stops by itself after this.
pub const SCAN_FOR: Duration = Duration::from_secs(120);
const BUSY_EVERY: Duration = Duration::from_secs(1);
/// A connect gives up after this.
const CONNECT_FOR: Duration = Duration::from_secs(30);

/// wad-bt's error as the API says it.
/// What turning Bluetooth on says on a machine without it.
pub const NO_BLUETOOTH: &str = "This machine has no Bluetooth";

pub fn api_error(e: wad_bt::Error) -> ApiError {
    let code = match &e {
        wad_bt::Error::Bus(zbus::Error::MethodError(..)) => ErrorCode::Upstream,
        wad_bt::Error::Bus(_) => ErrorCode::Offline,
        wad_bt::Error::NoAdapter | wad_bt::Error::Off | wad_bt::Error::Refused(_) => ErrorCode::Conflict,
        wad_bt::Error::NotFound(_) => ErrorCode::NotFound,
        wad_bt::Error::BadAddress(_) => ErrorCode::BadRequest,
    };
    ApiError::new(code, e.to_string())
}

/// wad-bt's state as the API says it (`hostname`: the name when the adapter has none).
pub fn view(s: Option<wad_bt::State>, hostname: &str) -> proto::Bluetooth {
    match s {
        None => proto::Bluetooth { discoverable_name: hostname.into(), ..Default::default() },
        Some(s) => proto::Bluetooth {
            on: s.on,
            discoverable_name: if s.alias.is_empty() { hostname.into() } else { s.alias },
            devices: if s.on { s.devices } else { vec![] },
            scanning: s.on && s.discovering,
        },
    }
}

type Pending = (BtPrompt, Option<oneshot::Sender<Answer>>);

#[derive(Default)]
struct St {
    last: Option<proto::Bluetooth>,
    prompt: Option<Pending>,
    scan_until: Option<tokio::time::Instant>,
    /// Being paired or connected (BlueZ is busy with it).
    busy: bool,
}

pub struct MachineBluetooth {
    /// The bus BlueZ is on ("": the system's).
    address: String,
    hostname: String,
    bz: tokio::sync::Mutex<Option<Arc<Bluez>>>,
    tx: mpsc::UnboundedSender<AgentEvent>,
    rx: Mutex<Option<mpsc::UnboundedReceiver<AgentEvent>>>,
    bus: Bus,
    st: Mutex<St>,
    poke: Notify,
    every: Duration,
    airplane: Arc<Airplane>,
}

impl MachineBluetooth {
    pub fn new(address: &str, hostname: &str, bus: Bus, every: Duration, airplane: Arc<Airplane>) -> Arc<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            address: address.into(),
            hostname: hostname.into(),
            bz: tokio::sync::Mutex::default(),
            tx,
            rx: Mutex::new(Some(rx)),
            bus,
            st: Mutex::default(),
            poke: Notify::new(),
            every,
            airplane,
        })
    }

    /// BlueZ, connected to once (again if the bus went away).
    async fn bz(&self) -> Result<Arc<Bluez>, ApiError> {
        let mut g = self.bz.lock().await;
        if let Some(b) = g.as_ref() {
            return Ok(b.clone());
        }
        let r = if self.address.is_empty() {
            Bluez::system(self.tx.clone()).await
        } else {
            match zbus::connection::Builder::address(self.address.as_str()) {
                Ok(b) => match b.build().await {
                    Ok(c) => Bluez::on(c, self.tx.clone()).await,
                    Err(e) => Err(e.into()),
                },
                Err(e) => Err(e.into()),
            }
        };
        let b = Arc::new(r.map_err(|e| ApiError::new(ErrorCode::Offline, format!("BlueZ isn't answering: {e}")))?);
        *g = Some(b.clone());
        Ok(b)
    }

    /// The state now, told when it changed.
    pub async fn refresh(&self) -> Result<proto::Bluetooth, ApiError> {
        let bz = self.bz().await?;
        let s = match bz.state().await {
            Ok(s) => s,
            // No bluetoothd: no Bluetooth.
            Err(wad_bt::Error::Bus(zbus::Error::MethodError(name, ..)))
                if name.as_str().ends_with("ServiceUnknown") || name.as_str().ends_with("NameHasNoOwner") =>
            {
                None
            }
            Err(e @ wad_bt::Error::Bus(zbus::Error::InputOutput(_))) => {
                // The bus went: connect again next time.
                *self.bz.lock().await = None;
                return Err(api_error(e));
            }
            Err(e) => return Err(api_error(e)),
        };
        let busy = s.as_ref().is_some_and(|s| !s.busy.is_empty());
        let v = view(s, &self.hostname);
        let changed = {
            let mut st = self.st.lock().unwrap();
            st.busy = busy;
            let changed = st.last.as_ref() != Some(&v);
            st.last = Some(v.clone());
            changed
        };
        if changed {
            self.bus.publish(Event::Bluetooth(Box::new(v.clone())));
        }
        Ok(v)
    }

    fn set_prompt(&self, p: Option<Pending>) {
        let (was, now) = {
            let mut st = self.st.lock().unwrap();
            let was = st.prompt.as_ref().map(|(p, _)| p.id.clone());
            let now = p.as_ref().map(|(p, _)| p.clone());
            st.prompt = p;
            (was, now)
        };
        if was.is_some() || now.is_some() {
            self.bus.publish(Event::BtPrompt { prompt: now });
        }
    }

    /// Follows BlueZ and the agent for as long as wadd runs.
    pub async fn run(self: Arc<Self>) {
        let mut agent = self.rx.lock().unwrap().take();
        loop {
            if let Err(e) = self.refresh().await {
                tracing::debug!("bluetooth: {}", e.message);
            }
            let expired = {
                let mut st = self.st.lock().unwrap();
                let over = st.scan_until.is_some_and(|t| t <= tokio::time::Instant::now());
                if over {
                    st.scan_until = None;
                }
                over
            };
            if expired && let Ok(b) = self.bz().await {
                let _ = b.scan(false).await;
                continue;
            }
            let busy = {
                let st = self.st.lock().unwrap();
                st.prompt.is_some() || st.busy || st.last.as_ref().is_some_and(|b| b.scanning)
            };
            let wait = tokio::time::sleep(if busy { BUSY_EVERY } else { self.every });
            tokio::select! {
                _ = wait => {}
                _ = self.poke.notified() => {}
                Some(ev) = async { match agent.as_mut() { Some(a) => a.recv().await, None => std::future::pending().await } } => {
                    match ev {
                        AgentEvent::Ask(a) => self.set_prompt(Some((a.prompt, a.reply))),
                        AgentEvent::Done => self.set_prompt(None),
                    }
                }
            }
        }
    }

    /// Bluetooth off, for airplane mode (nothing to do without it).
    pub async fn off_for_airplane(&self) -> Option<bool> {
        let bz = self.bz().await.ok()?;
        let was = bz.state().await.ok().flatten().map(|s| s.on)?;
        if was {
            let _ = bz.power(false).await;
        }
        self.poke.notify_one();
        Some(was)
    }

    /// Back on after airplane mode (it was on before).
    pub async fn on_after_airplane(&self) {
        if let Ok(bz) = self.bz().await {
            let _ = bz.power(true).await;
        }
        self.poke.notify_one();
    }

    fn name_of(&self, address: &str) -> String {
        let st = self.st.lock().unwrap();
        st.last
            .as_ref()
            .and_then(|b| b.devices.iter().find(|d| d.address.eq_ignore_ascii_case(address)))
            .map(|d| d.name.clone())
            .unwrap_or_else(|| address.to_uppercase())
    }

    /// A device that's there now (Bluetooth on), or why not.
    async fn known(&self, address: &str) -> Result<(Arc<Bluez>, proto::Bluetooth), ApiError> {
        if !wad_bt::is_address(address) {
            return Err(api_error(wad_bt::Error::BadAddress(address.into())));
        }
        let bz = self.bz().await?;
        let b = self.refresh().await?;
        if !b.on {
            return Err(api_error(wad_bt::Error::Off));
        }
        if !b.devices.iter().any(|d| d.address.eq_ignore_ascii_case(address)) {
            return Err(api_error(wad_bt::Error::NotFound(address.to_uppercase())));
        }
        Ok((bz, b))
    }
}

#[async_trait]
impl api_phase8::Bluetooth for Arc<MachineBluetooth> {
    async fn bluetooth(&self) -> Result<proto::Bluetooth, ApiError> {
        self.refresh().await
    }

    async fn power(&self, on: bool) -> Result<proto::Bluetooth, ApiError> {
        if on && self.airplane.is_on() {
            return Err(ApiError::conflict("Airplane mode is on"));
        }
        // No adapter (or no bluetoothd at all): nothing to turn on, said plainly.
        let bz = match self.bz().await {
            Ok(b) => b,
            Err(_) if on => return Err(ApiError::conflict(NO_BLUETOOTH)),
            Err(e) => return Err(e),
        };
        if on && !matches!(bz.state().await, Ok(Some(_))) {
            return Err(ApiError::conflict(NO_BLUETOOTH));
        }
        bz.power(on).await.map_err(api_error)?;
        if !on {
            self.st.lock().unwrap().scan_until = None;
        }
        self.poke.notify_one();
        self.refresh().await
    }

    async fn scan(&self, on: bool) -> Result<proto::Bluetooth, ApiError> {
        self.bz().await?.scan(on).await.map_err(api_error)?;
        self.st.lock().unwrap().scan_until = on.then(|| tokio::time::Instant::now() + SCAN_FOR);
        self.poke.notify_one();
        self.refresh().await
    }

    async fn pair(&self, address: &str) -> Result<proto::Bluetooth, ApiError> {
        let (bz, b) = self.known(address).await?;
        if b.devices.iter().any(|d| d.address.eq_ignore_ascii_case(address) && d.paired) {
            return Err(ApiError::conflict(format!("{} is paired already", self.name_of(address))));
        }
        let (me, address) = (self.clone(), address.to_uppercase());
        tokio::spawn(async move {
            let name = me.name_of(&address);
            let r = bz.pair(&address).await;
            me.set_prompt(None);
            if let Err(e) = r {
                tracing::info!("bluetooth: pairing {name}: {e}");
                me.bus.publish(Event::Notice {
                    level: Level::Error,
                    title: format!("Couldn't pair {name}"),
                    undo: None,
                    body: Some(e.to_string()),
                    icon: Some("bluetooth".into()),
                    action: None,
                    toast: true,
                });
            }
            let _ = me.refresh().await;
        });
        self.poke.notify_one();
        Ok(b)
    }

    async fn connect(&self, address: &str) -> Result<proto::Bluetooth, ApiError> {
        let (bz, _) = self.known(address).await?;
        match tokio::time::timeout(CONNECT_FOR, bz.connect(address)).await {
            Ok(r) => r.map_err(api_error)?,
            Err(_) => {
                return Err(ApiError::new(
                    ErrorCode::Timeout,
                    format!("{} didn't answer: is it on and close by?", self.name_of(address)),
                ));
            }
        }
        self.refresh().await
    }

    async fn disconnect(&self, address: &str) -> Result<proto::Bluetooth, ApiError> {
        let (bz, _) = self.known(address).await?;
        bz.disconnect(address).await.map_err(api_error)?;
        self.refresh().await
    }

    async fn forget(&self, address: &str) -> Result<proto::Bluetooth, ApiError> {
        let (bz, _) = self.known(address).await?;
        bz.forget(address).await.map_err(api_error)?;
        self.refresh().await
    }

    async fn answer(&self, id: &str, a: BtAnswer) -> Result<proto::Bluetooth, ApiError> {
        let pending = {
            let mut st = self.st.lock().unwrap();
            match st.prompt.as_ref() {
                Some((p, _)) if p.id == id => st.prompt.take(),
                _ => return Err(ApiError::not_found(format!("There's no prompt {id:?} (it's gone)"))),
            }
        };
        self.bus.publish(Event::BtPrompt { prompt: None });
        if let Some((p, reply)) = pending {
            match reply {
                Some(r) => {
                    let _ = r.send(Answer { accept: a.accept, pin: a.pin });
                }
                // A code shown: No calls pairing off.
                None if !a.accept && matches!(p.kind, BtAsk::Show { .. }) => {
                    if let Ok(bz) = self.bz().await {
                        let _ = bz.cancel_pairing(&p.device).await;
                    }
                }
                None => {}
            }
        }
        self.poke.notify_one();
        self.refresh().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_proto::{BtDevice, BtKind};

    #[test]
    fn off_shows_no_devices_and_the_name_falls_back_to_the_hostname() {
        let d = BtDevice {
            address: "AA:BB:CC:DD:EE:FF".into(),
            name: "Buds".into(),
            kind: BtKind::Headphones,
            paired: true,
            connected: false,
            battery: None,
        };
        let s = wad_bt::State { on: false, alias: String::new(), discovering: true, devices: vec![d], busy: vec![] };
        let v = view(Some(s), "wados");
        assert!(!v.on && v.devices.is_empty() && !v.scanning);
        assert_eq!(v.discoverable_name, "wados");
        assert_eq!(view(None, "wados"), proto::Bluetooth { discoverable_name: "wados".into(), ..Default::default() });
        assert_eq!(api_error(wad_bt::Error::Off).code, ErrorCode::Conflict);
        assert_eq!(api_error(wad_bt::Error::NotFound("x".into())).code, ErrorCode::NotFound);
    }
}
