//! Phase 8's sample data, for building Settings and the Updates app against
//! before the real network, Bluetooth, updates and storage are made
//! (`Parts::sample`: wadd's tests, and `wadd serve --sample`). It answers
//! every route of [`crate::api_phase8`], keeps what it's told for as long as
//! wadd runs, and tells each change as its event:
//!
//! - **The network:** Wi-Fi on, five networks: Home (on it, saved), Office
//!   (saved), Vanua Cafe (open), two secure ones in range; ethernet
//!   unplugged. A secure network not saved joins with any password of 8
//!   characters or more but `wrongpassword` (401). The exit node: the phase
//!   6 sample's tailnet's online computers (Desk).
//! - **Bluetooth:** on; Pixel Buds Pro (paired, connected, battery 80), an
//!   MX Master 3S mouse (paired), and a JBL Flip 6 nearby. Pairing that one
//!   asks to confirm a code (the `btPrompt` event); Yes pairs and connects it.
//! - **Updates:** 44.20261104.0 booted, 44.20261105.0 available,
//!   44.20261028.0 to roll back to. Check is an Update job that finds
//!   44.20261105.0 (unless it's staged); apply one that downloads and stages it.
//! - **Bases:** three images, the builder's not here; refresh is a Pull job.
//! - **Storage:** a 512 GB disk; each of this machine's wadspaces with a
//!   size (its build's, or a made-up one); Clean Up frees 6.5 GB once.
//!
//! Each stage of its jobs takes `step`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use wad_model::{JobKind, PartState};
use wad_proto as proto;
use wad_proto::network::exit_node_choices;
use wad_proto::{
    ApiError, Base, Bases, BtAnswer, BtAsk, BtDevice, BtKind, BtPrompt, ErrorCode, Ethernet, Event, JobInfo, Level,
    NoticeAction, UpdateProgress, Version, WadspaceStorage, WifiBand, WifiConnect, WifiNetwork,
};

use crate::api_phase6::Net;
use crate::api_phase8::{Bluetooth, Network, Storage, Updates};
use crate::builds::Builds;
use crate::events::Bus;
use crate::jobs::{Jobs, now_ms};
use crate::library::Docs;

const HOUR: i64 = 3_600_000;
const GB: u64 = 1_000_000_000;
const MB: u64 = 1_000_000;
const HOST: &str = "ghcr.io/hunggod/wados-host:44";
const BOOTED: &str = "44.20261104.0";
const NEXT: &str = "44.20261105.0";
const BEFORE: &str = "44.20261028.0";
/// The update's download, in all.
const UPDATE_BYTES: u64 = 1_200 * MB;
/// The made-up leftovers Clean Up frees.
const LEFTOVERS: u64 = 6_500 * MB;
/// The pairing code the nearby speaker shows.
const CODE: &str = "492817";

/// Cheap to clone: one sample, shared.
#[derive(Clone)]
pub struct Sample(Arc<Inner>);

struct Inner {
    st: Mutex<St>,
    jobs: Arc<Jobs>,
    bus: Bus,
    docs: Arc<Docs>,
    builds: Arc<Builds>,
    net: Arc<dyn Net>,
    step: Duration,
}

struct St {
    network: proto::Network,
    bluetooth: proto::Bluetooth,
    prompt: Option<BtPrompt>,
    prompts: u32,
    updates: proto::Updates,
    bases: Bases,
    bases_job: Option<String>,
    cleanup_job: Option<String>,
    leftovers: u64,
}

fn wifi(ssid: &str, signal: u8, secure: bool, saved: bool, band: WifiBand) -> WifiNetwork {
    let security = Some(if secure { "WPA2 Personal" } else { "None (open network)" }.into());
    WifiNetwork { ssid: ssid.into(), signal, secure, saved, active: false, band: Some(band), security }
}

fn bt(address: &str, name: &str, kind: BtKind, paired: bool, connected: bool, battery: Option<u8>) -> BtDevice {
    BtDevice { address: address.into(), name: name.into(), kind, paired, connected, battery }
}

fn version(v: &str) -> Version {
    Version { version: v.into(), image: HOST.into(), digest: Some(format!("sha256:sample-{v}")) }
}

fn base(name: &str, image: &str, size: Option<u64>, at: Option<i64>) -> Base {
    Base { name: name.into(), image: image.into(), size, refreshed_at: at, missing: size.is_none() }
}

fn busy(what: &str, job: &str) -> ApiError {
    ApiError::conflict(format!("{what} is under way already ({job})"))
}

impl Sample {
    /// The sample, its present told to the watchers.
    pub fn new(
        hostname: &str,
        jobs: Arc<Jobs>,
        bus: Bus,
        docs: Arc<Docs>,
        builds: Arc<Builds>,
        net: Arc<dyn Net>,
        step: Duration,
    ) -> Self {
        let now = now_ms();
        let network = proto::Network {
            wifi_on: true,
            airplane: false,
            connected: Some("Home".into()),
            ethernet: Ethernet::default(),
            networks: vec![
                wifi("Home", 82, true, true, WifiBand::Ghz5),
                wifi("Office", 64, true, true, WifiBand::Ghz5),
                wifi("Vanua Cafe", 58, false, false, WifiBand::Ghz2),
                wifi("Neighbours", 41, true, false, WifiBand::Ghz2),
                wifi("TP-Link_4F2A", 23, true, false, WifiBand::Ghz2),
            ],
            ip: None,
            dns: vec![],
            exit_node: None,
            exit_nodes: vec![],
        };
        let bluetooth = proto::Bluetooth {
            on: true,
            discoverable_name: hostname.into(),
            devices: vec![
                bt("AA:BB:CC:00:00:01", "Pixel Buds Pro", BtKind::Headphones, true, true, Some(80)),
                bt("AA:BB:CC:00:00:02", "MX Master 3S", BtKind::Mouse, true, false, None),
                bt("AA:BB:CC:00:00:03", "JBL Flip 6", BtKind::Speaker, false, false, None),
            ],
            scanning: false,
        };
        let updates = proto::Updates {
            online: true,
            checking: false,
            booted: version(BOOTED),
            available: Some(version(NEXT)),
            staged: None,
            rollback: Some(version(BEFORE)),
            error: None,
            checked_at: Some(now - 2 * HOUR),
            progress: None,
            job: None,
        };
        let bases = Bases {
            bases: vec![
                base("Fedora 44 base", "ghcr.io/hunggod/wados-base:44", Some(1_900 * MB), Some(now - 3 * HOUR)),
                base("Stream sidecar", "ghcr.io/hunggod/wados-stream:44", Some(820 * MB), Some(now - 3 * HOUR)),
                base("Builder", "ghcr.io/hunggod/wados-builder:44", None, None),
            ],
            next_refresh_at: Some(now + 9 * HOUR),
        };
        let mut st = St {
            network,
            bluetooth,
            prompt: None,
            prompts: 0,
            updates,
            bases,
            bases_job: None,
            cleanup_job: None,
            leftovers: LEFTOVERS,
        };
        fix_network(&mut st.network);
        let me = Inner { st: Mutex::new(st), jobs, bus, docs, builds, net, step };
        let st = me.st.lock().unwrap();
        me.bus.publish(Event::Network(Box::new(st.network.clone())));
        me.bus.publish(Event::Bluetooth(Box::new(st.bluetooth.clone())));
        me.bus.publish(Event::Updates(Box::new(st.updates.clone())));
        drop(st);
        me.bus.publish(Event::Storage(Box::new(me.storage())));
        Self(Arc::new(me))
    }
}

/// Each network's `active`, its address and DNS, from what it's on.
fn fix_network(n: &mut proto::Network) {
    if !n.wifi_on {
        n.connected = None;
    }
    for w in &mut n.networks {
        w.active = n.connected.as_deref() == Some(w.ssid.as_str());
    }
    n.networks.sort_by_key(|w| (!w.active, !w.saved, u8::MAX - w.signal));
    let on = n.connected.is_some() || n.ethernet.plugged;
    n.ip = on.then(|| "192.168.1.42".into());
    n.dns = if on { vec!["192.168.1.1".into()] } else { vec![] };
}

impl Inner {
    /// The network as it is (its exit nodes from the tailnet now), told.
    async fn network(&self) -> Result<proto::Network, ApiError> {
        let tailnet = self.net.tailnet().await?;
        let mut n = self.st.lock().unwrap().network.clone();
        n.exit_nodes = exit_node_choices(&tailnet);
        Ok(n)
    }

    async fn change_network(
        &self,
        f: impl FnOnce(&mut St) -> Result<(), ApiError>,
    ) -> Result<proto::Network, ApiError> {
        let bt = {
            let mut st = self.st.lock().unwrap();
            let was = st.bluetooth.clone();
            f(&mut st)?;
            fix_network(&mut st.network);
            (st.bluetooth != was).then(|| st.bluetooth.clone())
        };
        if let Some(b) = bt {
            self.bus.publish(Event::Bluetooth(Box::new(b)));
        }
        let n = self.network().await?;
        self.bus.publish(Event::Network(Box::new(n.clone())));
        Ok(n)
    }

    fn change_bt(&self, f: impl FnOnce(&mut St) -> Result<(), ApiError>) -> Result<proto::Bluetooth, ApiError> {
        let b = {
            let mut st = self.st.lock().unwrap();
            f(&mut st)?;
            st.bluetooth.clone()
        };
        self.bus.publish(Event::Bluetooth(Box::new(b.clone())));
        Ok(b)
    }

    fn change_updates(&self, f: impl FnOnce(&mut proto::Updates)) -> proto::Updates {
        let u = {
            let mut st = self.st.lock().unwrap();
            f(&mut st.updates);
            st.updates.clone()
        };
        self.bus.publish(Event::Updates(Box::new(u.clone())));
        u
    }

    fn notice(&self, level: Level, title: String, body: &str, icon: &str, action: Option<NoticeAction>, toast: bool) {
        self.bus.publish(Event::Notice {
            level,
            title,
            undo: None,
            body: Some(body.into()),
            icon: Some(icon.into()),
            action,
            toast,
        });
    }

    fn storage(&self) -> proto::Storage {
        let st = self.st.lock().unwrap();
        let total = 512 * GB;
        let bases: u64 = st.bases.bases.iter().filter_map(|b| b.size).sum();
        let mut per: Vec<WadspaceStorage> = self
            .docs
            .live()
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let b = self.builds.state_of(w).state;
                let made_up = (3_200 * MB).saturating_sub(i as u64 * 600 * MB).max(400 * MB);
                WadspaceStorage { id: w.id.clone(), bytes: (b.size_mb * MB).max(made_up), built: b.built }
            })
            .collect();
        per.sort_by_key(|w| std::cmp::Reverse(w.bytes));
        let wadspaces = per.iter().map(|w| w.bytes).sum();
        let other = 38 * GB + st.leftovers;
        proto::Storage {
            total,
            free: total.saturating_sub(bases + wadspaces + other),
            bases,
            wadspaces,
            other,
            per_wadspace: per,
        }
    }

    fn device<'a>(st: &'a mut St, address: &str) -> Result<&'a mut BtDevice, ApiError> {
        if !st.bluetooth.on {
            return Err(ApiError::conflict("Bluetooth is off"));
        }
        st.bluetooth
            .devices
            .iter_mut()
            .find(|d| d.address == address)
            .ok_or_else(|| ApiError::not_found(format!("There's no Bluetooth device {address}")))
    }

    /// Runs `stages` of `job`, a step each (`on` at each), then `done`; what
    /// `gone` clears happens however it ends (cancelled too).
    fn run(
        self: &Arc<Self>,
        job: &str,
        stages: Vec<String>,
        on: impl Fn(&Arc<Inner>, &str) + Send + 'static,
        done: impl FnOnce(&Arc<Inner>) + Send + 'static,
        gone: impl FnOnce(&Arc<Inner>) + Send + 'static,
    ) {
        struct Gone<F: FnOnce(&Arc<Inner>)>(Arc<Inner>, Option<F>);
        impl<F: FnOnce(&Arc<Inner>)> Drop for Gone<F> {
            fn drop(&mut self) {
                if let Some(f) = self.1.take() {
                    f(&self.0);
                }
            }
        }
        let (me, id) = (self.clone(), job.to_string());
        let task = tokio::spawn(async move {
            let guard = Gone(me.clone(), Some(gone));
            me.jobs.running(&id);
            for s in &stages {
                me.jobs.stage(&id, s, PartState::Working, None);
                on(&me, s);
                tokio::time::sleep(me.step).await;
                me.jobs.stage(&id, s, PartState::Done, None);
            }
            done(&me);
            drop(guard);
            me.jobs.finish(&id, Ok(()));
        });
        self.jobs.attach(job, task.abort_handle());
    }

    fn info(&self, job: &str) -> Result<JobInfo, ApiError> {
        self.jobs.info(job).ok_or_else(|| ApiError::internal("the job went"))
    }
}

fn labelled(stages: &[(&str, &str)]) -> Vec<(String, String)> {
    stages.iter().map(|(s, l)| (s.to_string(), l.to_string())).collect()
}

#[async_trait]
impl Network for Sample {
    async fn network(&self) -> Result<proto::Network, ApiError> {
        self.0.network().await
    }

    async fn wifi(&self, on: bool) -> Result<proto::Network, ApiError> {
        self.0
            .change_network(|st| {
                let n = &mut st.network;
                if on && n.airplane {
                    return Err(ApiError::conflict("Airplane mode is on"));
                }
                n.wifi_on = on;
                if on && n.connected.is_none() {
                    n.connected = n.networks.iter().find(|w| w.saved && w.signal > 0).map(|w| w.ssid.clone());
                }
                Ok(())
            })
            .await
    }

    async fn airplane(&self, on: bool) -> Result<proto::Network, ApiError> {
        self.0
            .change_network(|st| {
                st.network.airplane = on;
                if on {
                    st.network.wifi_on = false;
                    st.bluetooth.on = false;
                    st.bluetooth.scanning = false;
                    for d in &mut st.bluetooth.devices {
                        d.connected = false;
                    }
                } else {
                    let n = &mut st.network;
                    n.wifi_on = true;
                    n.connected = n.networks.iter().find(|w| w.saved && w.signal > 0).map(|w| w.ssid.clone());
                }
                Ok(())
            })
            .await
    }

    async fn connect(&self, req: WifiConnect) -> Result<proto::Network, ApiError> {
        self.0
            .change_network(|st| {
                let n = &mut st.network;
                if n.airplane {
                    return Err(ApiError::conflict("Airplane mode is on"));
                }
                let w = n
                    .networks
                    .iter_mut()
                    .find(|w| w.ssid == req.ssid)
                    .ok_or_else(|| ApiError::not_found(format!("{:?} isn't in range", req.ssid)))?;
                if w.secure && !w.saved {
                    match req.password.as_deref() {
                        None => return Err(ApiError::bad_request(format!("{} needs its password", w.ssid))),
                        Some(p) if p.len() < 8 || p == "wrongpassword" => {
                            return Err(ApiError::new(
                                ErrorCode::Unauthorized,
                                format!("That isn't the password for {}", w.ssid),
                            ));
                        }
                        Some(_) => {}
                    }
                }
                w.saved = true;
                n.wifi_on = true;
                n.connected = Some(req.ssid.clone());
                Ok(())
            })
            .await
    }

    async fn disconnect(&self) -> Result<proto::Network, ApiError> {
        self.0
            .change_network(|st| {
                st.network.connected = None;
                Ok(())
            })
            .await
    }

    async fn forget(&self, ssid: &str) -> Result<proto::Network, ApiError> {
        self.0
            .change_network(|st| {
                let n = &mut st.network;
                let w = n
                    .networks
                    .iter_mut()
                    .find(|w| w.ssid == ssid && w.saved)
                    .ok_or_else(|| ApiError::not_found(format!("{ssid:?} isn't saved")))?;
                w.saved = false;
                if n.connected.as_deref() == Some(ssid) {
                    n.connected = None;
                }
                Ok(())
            })
            .await
    }

    async fn exit_node(&self, node: Option<String>) -> Result<proto::Network, ApiError> {
        if let Some(id) = &node {
            let t = self.0.net.tailnet().await?;
            if t.state != proto::TailnetState::Running {
                return Err(ApiError::conflict("Join Tailscale first"));
            }
            if !exit_node_choices(&t).iter().any(|c| &c.id == id) {
                return Err(ApiError::not_found(format!("{id:?} can't be an exit node now")));
            }
        }
        self.0
            .change_network(|st| {
                st.network.exit_node = node;
                Ok(())
            })
            .await
    }
}

#[async_trait]
impl Bluetooth for Sample {
    async fn bluetooth(&self) -> Result<proto::Bluetooth, ApiError> {
        Ok(self.0.st.lock().unwrap().bluetooth.clone())
    }

    async fn power(&self, on: bool) -> Result<proto::Bluetooth, ApiError> {
        self.0.change_bt(|st| {
            if on && st.network.airplane {
                return Err(ApiError::conflict("Airplane mode is on"));
            }
            let b = &mut st.bluetooth;
            b.on = on;
            if !on {
                b.scanning = false;
                for d in &mut b.devices {
                    d.connected = false;
                }
            }
            Ok(())
        })
    }

    async fn scan(&self, on: bool) -> Result<proto::Bluetooth, ApiError> {
        self.0.change_bt(|st| {
            if !st.bluetooth.on {
                return Err(ApiError::conflict("Bluetooth is off"));
            }
            st.bluetooth.scanning = on;
            Ok(())
        })
    }

    async fn pair(&self, address: &str) -> Result<proto::Bluetooth, ApiError> {
        let me = &self.0;
        let prompt = {
            let mut st = me.st.lock().unwrap();
            let d = Inner::device(&mut st, address)?;
            if d.paired {
                None
            } else {
                let name = d.name.clone();
                st.prompts += 1;
                let p = BtPrompt {
                    id: format!("bt-prompt-{}", st.prompts),
                    device: address.into(),
                    name,
                    kind: BtAsk::Confirm { code: CODE.into() },
                };
                st.prompt = Some(p.clone());
                Some(p)
            }
        };
        if let Some(p) = prompt {
            me.bus.publish(Event::BtPrompt { prompt: Some(p) });
        }
        self.bluetooth().await
    }

    async fn connect(&self, address: &str) -> Result<proto::Bluetooth, ApiError> {
        self.0.change_bt(|st| {
            let d = Inner::device(st, address)?;
            if !d.paired {
                return Err(ApiError::conflict(format!("{} isn't paired: pair it first", d.name)));
            }
            d.connected = true;
            Ok(())
        })
    }

    async fn disconnect(&self, address: &str) -> Result<proto::Bluetooth, ApiError> {
        self.0.change_bt(|st| {
            Inner::device(st, address)?.connected = false;
            Ok(())
        })
    }

    async fn forget(&self, address: &str) -> Result<proto::Bluetooth, ApiError> {
        self.0.change_bt(|st| {
            let d = Inner::device(st, address)?;
            d.paired = false;
            d.connected = false;
            d.battery = None;
            Ok(())
        })
    }

    async fn answer(&self, id: &str, answer: BtAnswer) -> Result<proto::Bluetooth, ApiError> {
        let me = &self.0;
        let p = {
            let mut st = me.st.lock().unwrap();
            if st.prompt.as_ref().is_none_or(|p| p.id != id) {
                return Err(ApiError::not_found(format!("There's no prompt {id:?} (it's gone)")));
            }
            st.prompt.take().expect("found")
        };
        me.bus.publish(Event::BtPrompt { prompt: None });
        if !answer.accept {
            me.notice(Level::Info, format!("{} wasn't paired", p.name), "You said no.", "bluetooth", None, true);
            return self.bluetooth().await;
        }
        me.change_bt(|st| {
            let d = Inner::device(st, &p.device)?;
            d.paired = true;
            d.connected = true;
            Ok(())
        })
    }
}

#[async_trait]
impl Updates for Sample {
    async fn status(&self) -> Result<proto::Updates, ApiError> {
        Ok(self.0.st.lock().unwrap().updates.clone())
    }

    async fn check(&self) -> Result<JobInfo, ApiError> {
        let me = &self.0;
        if let Some(j) = &me.st.lock().unwrap().updates.job {
            return Err(busy("An update", j));
        }
        let job = me.jobs.create(JobKind::Update, "Checking for updates", "wados", &labelled(proto::updates::CHECK));
        me.change_updates(|u| {
            u.checking = true;
            u.job = Some(job.clone());
        });
        let stages = proto::updates::CHECK.iter().map(|(s, _)| s.to_string()).collect();
        me.run(
            &job,
            stages,
            |_, _| {},
            |me| {
                me.change_updates(|u| {
                    let staged = u.staged.as_ref().is_some_and(|s| s.version == NEXT);
                    u.available = (!staged).then(|| version(NEXT));
                    u.checked_at = Some(now_ms());
                    u.error = None;
                });
            },
            |me| {
                me.change_updates(|u| {
                    u.checking = false;
                    u.job = None;
                });
            },
        );
        me.info(&job)
    }

    async fn apply(&self) -> Result<JobInfo, ApiError> {
        let me = &self.0;
        let next = {
            let st = me.st.lock().unwrap();
            if let Some(j) = &st.updates.job {
                return Err(busy("An update", j));
            }
            st.updates.available.clone().ok_or_else(|| ApiError::conflict("Nothing to apply: it's up to date"))?
        };
        let title = format!("Updating WadSpaces to {}", next.version);
        let job = me.jobs.create(JobKind::Update, &title, "wados", &labelled(proto::updates::APPLY));
        me.change_updates(|u| u.job = Some(job.clone()));
        let stages = proto::updates::APPLY.iter().map(|(s, _)| s.to_string()).collect();
        let id = job.clone();
        me.run(
            &job,
            stages,
            move |me, stage| {
                if stage == "download" {
                    me.jobs.bytes(&id, stage, UPDATE_BYTES / 2, UPDATE_BYTES);
                    me.change_updates(|u| {
                        u.progress = Some(UpdateProgress { done_bytes: UPDATE_BYTES / 2, total_bytes: UPDATE_BYTES })
                    });
                }
            },
            move |me| {
                me.change_updates(|u| {
                    u.staged = Some(next.clone());
                    u.available = None;
                });
                me.notice(
                    Level::Ok,
                    format!("WadSpaces {} is ready", next.version),
                    "Restart to start it.",
                    "download",
                    Some(NoticeAction::new("Restart", "power.restart", &[])),
                    true,
                );
            },
            |me| {
                me.change_updates(|u| {
                    u.progress = None;
                    u.job = None;
                });
            },
        );
        me.info(&job)
    }

    async fn cancel(&self) -> Result<proto::Updates, ApiError> {
        let me = &self.0;
        let (was, booted) = {
            let st = me.st.lock().unwrap();
            let s = st.updates.staged.clone().ok_or_else(|| ApiError::conflict("Nothing is staged"))?;
            (s, st.updates.booted.version.clone())
        };
        let u = me.change_updates(|u| {
            // Dropping the newer one: it's available again.
            if u.rollback.as_ref() != Some(&was) {
                u.available = Some(was.clone());
            }
            u.staged = None;
        });
        let title = format!("WadSpaces {} won't be applied", was.version);
        me.notice(Level::Info, title, &format!("This machine stays on {booted}."), "x", None, false);
        Ok(u)
    }

    async fn rollback(&self) -> Result<proto::Updates, ApiError> {
        let me = &self.0;
        let back = {
            let st = me.st.lock().unwrap();
            let u = &st.updates;
            if let Some(j) = &u.job {
                return Err(busy("An update", j));
            }
            match &u.rollback {
                Some(r) if u.staged.as_ref() != Some(r) => r.clone(),
                Some(_) => return Err(ApiError::conflict("It's staged already")),
                None => return Err(ApiError::conflict("There's nothing to go back to")),
            }
        };
        let u = me.change_updates(|u| u.staged = Some(back.clone()));
        me.notice(
            Level::Info,
            format!("WadSpaces {} is staged", back.version),
            "Restart to go back to it.",
            "rotate-ccw",
            Some(NoticeAction::new("Restart", "power.restart", &[])),
            true,
        );
        Ok(u)
    }

    async fn bases(&self) -> Result<Bases, ApiError> {
        Ok(self.0.st.lock().unwrap().bases.clone())
    }

    async fn refresh_bases(&self) -> Result<JobInfo, ApiError> {
        let me = &self.0;
        let stages: Vec<(String, String)> = {
            let st = me.st.lock().unwrap();
            if let Some(j) = &st.bases_job {
                return Err(busy("A refresh", j));
            }
            st.bases.bases.iter().enumerate().map(|(i, b)| (format!("base-{i}"), b.name.clone())).collect()
        };
        let job = me.jobs.create(JobKind::Pull, "Refreshing the base images", "bases", &stages);
        me.st.lock().unwrap().bases_job = Some(job.clone());
        me.run(
            &job,
            stages.into_iter().map(|(s, _)| s).collect(),
            |_, _| {},
            |me| {
                let now = now_ms();
                {
                    let mut st = me.st.lock().unwrap();
                    for b in &mut st.bases.bases {
                        b.size = b.size.or(Some(640 * MB));
                        b.refreshed_at = Some(now);
                        b.missing = false;
                    }
                    st.bases.next_refresh_at = Some(now + 12 * HOUR);
                }
                me.bus.publish(Event::Storage(Box::new(me.storage())));
            },
            |me| me.st.lock().unwrap().bases_job = None,
        );
        me.info(&job)
    }
}

#[async_trait]
impl Storage for Sample {
    async fn storage(&self) -> Result<proto::Storage, ApiError> {
        Ok(self.0.storage())
    }

    async fn cleanup(&self) -> Result<JobInfo, ApiError> {
        let me = &self.0;
        if let Some(j) = &me.st.lock().unwrap().cleanup_job {
            return Err(busy("A cleanup", j));
        }
        let job = me.jobs.create(JobKind::Cleanup, "Cleaning up", "storage", &labelled(proto::storage::CLEANUP));
        me.st.lock().unwrap().cleanup_job = Some(job.clone());
        me.run(
            &job,
            proto::storage::CLEANUP.iter().map(|(s, _)| s.to_string()).collect(),
            |_, _| {},
            |me| {
                let freed = std::mem::take(&mut me.st.lock().unwrap().leftovers);
                me.bus.publish(Event::Storage(Box::new(me.storage())));
                let (title, body) = match freed {
                    0 => ("Nothing to clean up".to_string(), "There are no leftovers on this machine."),
                    n => (
                        format!("Freed {:.1} GB", n as f64 / GB as f64),
                        "Old build layers, stopped containers and caches are gone.",
                    ),
                };
                me.notice(Level::Ok, title, body, "sparkles", None, true);
            },
            |me| me.st.lock().unwrap().cleanup_job = None,
        );
        me.info(&job)
    }
}
