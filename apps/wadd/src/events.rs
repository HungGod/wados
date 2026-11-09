//! What changed, for everyone watching `/v1/events`: a broadcast of events,
//! plus the latest of each kind that has one ([`Event::latest_key`]: the
//! machine, each wadspace's state…), so a new watcher starts from the present.
//! An event can end a present ([`Event::ends_key`]: a sign-in's, when it's done).

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use tokio::sync::broadcast;
use wad_proto::{Event, MachineInfo, RemoteNotice, SignInProvider};

/// How many notices are kept for a phone's Activity.
pub const RECENT: usize = 50;

/// Told every notice, to pass on to my other machines (peers.rs's): an
/// agent asking a question on one machine reaches the bell on every screen.
pub trait Relay: Send + Sync {
    fn notice(&self, n: &RemoteNotice);
}

#[derive(Clone)]
pub struct Bus {
    tx: broadcast::Sender<Event>,
    latest: Arc<Mutex<IndexMap<String, Event>>>,
    /// The last notices, newest last (a phone's Activity: the shell keeps its own).
    recent: Arc<Mutex<VecDeque<RemoteNotice>>>,
    /// Where notices go beside here (none: this machine alone).
    relay: Arc<Mutex<Option<Arc<dyn Relay>>>>,
    /// The ids of the notices that came from another machine: passed on
    /// no further (or two machines would tell each other for ever).
    abroad: Arc<Mutex<HashSet<String>>>,
}

impl Bus {
    pub fn new(machine: MachineInfo) -> Self {
        let bus = Self {
            tx: broadcast::channel(512).0,
            latest: Arc::default(),
            recent: Arc::default(),
            relay: Arc::default(),
            abroad: Arc::default(),
        };
        bus.publish(Event::Machine(machine));
        bus
    }

    /// Notices told here are passed on to my other machines from now on.
    pub fn relay_through(&self, relay: Arc<dyn Relay>) {
        *self.relay.lock().unwrap() = Some(relay);
    }

    pub fn publish(&self, e: Event) {
        if let Event::Notice { level, title, body, icon, .. } = &e {
            let at = crate::jobs::now_ms();
            let listed = RemoteNotice {
                at,
                machine: None,
                id: Some(notice_id(at, title, body.as_deref())),
                level: *level,
                title: title.clone(),
                body: body.clone(),
                icon: icon.clone(),
            };
            self.keep(&listed);
            // On to my other machines, unless it came from one of them.
            let from_abroad = self.abroad.lock().unwrap().remove(listed.id.as_deref().unwrap_or_default());
            if !from_abroad && let Some(r) = self.relay.lock().unwrap().clone() {
                r.notice(&listed);
            }
        }
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

    /// One notice in the list (not the newest out of it): the same one told
    /// twice is there once.
    fn keep(&self, n: &RemoteNotice) {
        let mut recent = self.recent.lock().unwrap();
        if let Some(id) = &n.id
            && recent.iter().any(|x| x.id.as_ref() == Some(id))
        {
            return;
        }
        recent.push_back(n.clone());
        while recent.len() > RECENT {
            recent.pop_front();
        }
    }

    /// A notice from another of my machines: kept and shown here as its own
    /// (the shell's bell, a toast), and never passed on again.
    pub fn adopt(&self, n: RemoteNotice) {
        let id = n.id.clone().unwrap_or_else(|| notice_id(n.at, &n.title, n.body.as_deref()));
        // Its own notice, or one of this machine's that already went round.
        if self.recent.lock().unwrap().iter().any(|x| x.id.as_deref() == Some(&id)) {
            return;
        }
        self.abroad.lock().unwrap().insert(id.clone());
        let title = match &n.machine {
            Some(m) if !m.is_empty() => format!("{m}: {}", n.title),
            _ => n.title.clone(),
        };
        let listed = RemoteNotice {
            at: n.at,
            machine: n.machine.clone(),
            id: Some(id),
            level: n.level,
            title: title.clone(),
            body: n.body.clone(),
            icon: n.icon.clone(),
        };
        self.keep(&listed);
        // The shell hears it (its bell, and a toast: it's news here too).
        let e = Event::Notice {
            level: n.level,
            title,
            undo: None,
            body: n.body,
            icon: n.icon,
            action: None,
            toast: true,
        };
        {
            let mut latest = self.latest.lock().unwrap();
            if let Some(k) = e.latest_key() {
                latest.insert(k, e.clone());
            }
        }
        let _ = self.tx.send(e);
    }

    /// The last notices, newest first.
    pub fn recent_notices(&self) -> Vec<RemoteNotice> {
        self.recent.lock().unwrap().iter().rev().cloned().collect()
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

/// What a notice is, in one string: the same one told twice (here and from
/// another machine) has the same id.
fn notice_id(at: i64, title: &str, body: Option<&str>) -> String {
    // FNV-1a over the second it was told in (two machines' clocks differ), the
    // title and the body.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in (at / 1000).to_string().bytes().chain(title.bytes()).chain(body.unwrap_or("").bytes()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    format!("{h:016x}")
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

    #[derive(Default)]
    struct Passed(std::sync::Mutex<Vec<RemoteNotice>>);
    impl Relay for Passed {
        fn notice(&self, n: &RemoteNotice) {
            self.0.lock().unwrap().push(n.clone());
        }
    }

    fn bus() -> Bus {
        Bus::new(MachineInfo { id: "m".into(), name: "M".into(), version: "0".into(), profile: Profile::User })
    }

    #[test]
    fn a_notice_here_goes_to_my_other_machines_once() {
        let b = bus();
        let passed = Arc::new(Passed::default());
        b.relay_through(passed.clone());
        b.publish(Event::notice(wad_proto::Level::Info, "Claude Code wants a sandbox", None));
        let n = passed.0.lock().unwrap().clone();
        assert_eq!(n.len(), 1, "it went on");
        assert!(n[0].id.is_some(), "with an id to tell it by");
        // One told from another machine isn't passed on again (or two
        // machines would tell each other for ever).
        b.adopt(n[0].clone());
        assert_eq!(passed.0.lock().unwrap().len(), 1, "not twice");
        // And it's shown here, as that machine's.
        let recent = b.recent_notices();
        assert!(recent.iter().any(|r| r.title.contains("Claude Code wants a sandbox")), "{recent:?}");
    }

    #[test]
    fn the_same_notice_twice_is_shown_once() {
        let b = bus();
        let n = RemoteNotice {
            at: 1_000,
            machine: Some("Studio".into()),
            id: Some("abc".into()),
            level: wad_proto::Level::Info,
            title: "An agent is done".into(),
            body: None,
            icon: None,
        };
        b.adopt(n.clone());
        b.adopt(n.clone());
        assert_eq!(b.recent_notices().len(), 1);
        // Its title says where it came from.
        assert_eq!(b.recent_notices()[0].title, "Studio: An agent is done");
    }
}
