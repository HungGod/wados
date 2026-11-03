//! What changed, for everyone watching `/v1/events`: a broadcast of events,
//! plus the latest of each kind that has one ([`Event::latest_key`]: the
//! machine, each wadspace's state…), so a new watcher starts from the present.

use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use tokio::sync::broadcast;
use wad_proto::{Event, MachineInfo};

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
        if let Some(k) = e.latest_key() {
            self.latest.lock().unwrap().insert(k, e.clone());
        }
        let _ = self.tx.send(e);
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
