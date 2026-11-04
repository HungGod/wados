//! What changed, for everyone watching `/v1/events`: a broadcast of events,
//! plus the latest of each kind that has one ([`Event::latest_key`]: the
//! machine, each wadspace's state…), so a new watcher starts from the present.
//! An event can end a present ([`Event::ends_key`]: a sign-in's, when it's done).

use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use tokio::sync::broadcast;
use wad_proto::{Event, MachineInfo, SignInProvider};

#[derive(Clone)]
pub struct Bus {
    tx: broadcast::Sender<Event>,
    latest: Arc<Mutex<IndexMap<String, Event>>>,
}

impl Bus {
    pub fn new(machine: MachineInfo) -> Self {
        let bus = Self { tx: broadcast::channel(512).0, latest: Arc::default() };
        bus.publish(Event::Machine(machine));
        bus
    }

    pub fn publish(&self, e: Event) {
        {
            let mut latest = self.latest.lock().unwrap();
            if let Some(k) = e.ends_key() {
                latest.shift_remove(&k);
            }
            if let Some(k) = e.latest_key() {
                latest.insert(k, e.clone());
            }
        }
        let _ = self.tx.send(e);
    }

    /// A sign-in in the browser: `signIn` now, `signInDone` when the
    /// [`SignInWindow`] goes (signed in, failed, or its job cancelled: its
    /// task dropped), so the shell's window never outlives it.
    pub fn sign_in(&self, provider: SignInProvider, url: &str, code: Option<&str>) -> SignInWindow {
        self.publish(Event::SignIn { provider, url: url.to_string(), code: code.map(String::from) });
        SignInWindow { bus: self.clone(), provider }
    }

    /// A kind's latest is no longer the present (a job forgotten, a wadspace gone).
    pub fn forget(&self, key: &str) {
        self.latest.lock().unwrap().shift_remove(key);
    }

    /// The latest event of a kind, if there's one.
    pub fn latest(&self, key: &str) -> Option<Event> {
        self.latest.lock().unwrap().get(key).cloned()
    }

    pub fn machine(&self) -> Option<MachineInfo> {
        match self.latest.lock().unwrap().get("machine") {
            Some(Event::Machine(m)) => Some(m.clone()),
            _ => None,
        }
    }

    /// The latest of each kind, then whatever happens next.
    pub fn subscribe(&self) -> (Vec<Event>, broadcast::Receiver<Event>) {
        let rx = self.tx.subscribe();
        let now = self.latest.lock().unwrap().values().cloned().collect();
        (now, rx)
    }
}

/// A sign-in window open in the shell ([`Bus::sign_in`]): it closes when this is dropped.
pub struct SignInWindow {
    bus: Bus,
    provider: SignInProvider,
}

impl Drop for SignInWindow {
    fn drop(&mut self) {
        self.bus.publish(Event::SignInDone { provider: self.provider });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_proto::Profile;

    #[test]
    fn a_sign_in_is_the_present_until_its_window_goes() {
        let bus =
            Bus::new(MachineInfo { id: "m".into(), name: "M".into(), version: "0".into(), profile: Profile::User });
        let (_, mut rx) = bus.subscribe();
        let window = bus.sign_in(SignInProvider::Github, "https://github.com/login/device", Some("WADS-0601"));
        let (now, _) = bus.subscribe();
        assert!(now.iter().any(|e| matches!(e, Event::SignIn { code: Some(c), .. } if c == "WADS-0601")));
        drop(window);
        let (now, _) = bus.subscribe();
        assert!(!now.iter().any(|e| matches!(e, Event::SignIn { .. } | Event::SignInDone { .. })), "{now:?}");
        assert!(matches!(rx.try_recv(), Ok(Event::SignIn { .. })));
        assert!(matches!(rx.try_recv(), Ok(Event::SignInDone { provider: SignInProvider::Github })));
    }
}
