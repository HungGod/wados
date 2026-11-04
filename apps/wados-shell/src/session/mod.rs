//! The shell as the machine's own session, rather than in a test or a
//! window on a laptop. greetd's session script sets `WADOS_SESSION=machine`
//! before it runs `labwc -s wados-shell`; [`is_machine`] is the one place
//! that reads it (`Shell::is_machine` for everyone else). Only then does the
//! shell reach past its own screen to the machine:
//!
//! - **Restart and Power off** (`logind.rs`: `Reboot`/`PowerOff`,
//!   interactive, so polkit asks if it must), from the start menu, the power
//!   key's menu (`menu.rs`) and the lock screen. Elsewhere they're off, and
//!   their tooltips say so. Server mode comes in phase 10.
//! - **Locking before sleep** (`sleep.rs`): a delay inhibitor while awake;
//!   at logind's `PrepareForSleep` (the lid, the suspend key, an idle
//!   suspend) the screen locks, then the inhibitor goes. logind's own `Lock`
//!   (`loginctl lock-session`) locks it too.
//! - **The brightness keys**: logind's `SetBrightness` (no root).
//! - **Locking when idle** (`idle.rs`, ext-idle-notify): after 5 minutes
//!   (Settings sets it in phase 8). `WADOS_IDLE_LOCK_SECS` sets it (0: never),
//!   and outside the machine's session only it turns it on.
//!
//! The volume keys (`media.rs`, `wpctl`) are the user's own sound, so they
//! work anywhere; the bar's tray shows what they did.
//!
//! `WADOS_LOGIND_DRY_RUN=1` logs logind's Reboot, PowerOff and SetBrightness
//! instead of calling them (`wadsctl session` lists them), so a machine
//! session tested on a laptop never turns the laptop off.
//!
//! Its `wadsctl` commands: `session [heard]`, `power` (the power key: its
//! menu, or on the lock screen the lock's Power off), `power
//! state|close|lock|restart|off`, `volume [up|down|mute]`, `brightness
//! [up|down]` and `server off` (Ctrl+Alt+Delete).

mod idle;
pub(crate) mod logind;
mod media;
mod menu;
mod sleep;

use std::cell::RefCell;
use std::os::fd::OwnedFd;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Duration;

use gtk::glib;
use wad_proto::Level;

use crate::bar::{self, Volume};
use crate::lock::Lock;
use crate::shell::Shell;
use logind::Logind;
use media::{Mixer, Vol};
use sleep::{Do, Sleep};

/// How long idle before the screen locks, in the machine's session.
const IDLE_LOCK: Duration = Duration::from_secs(5 * 60);
const SERVER_LATER: &str = "Server mode (the screen goes off and it only streams) comes in phase 10";

/// This is the machine's own session (`WADOS_SESSION=machine`).
pub fn is_machine() -> bool {
    static MACHINE: OnceLock<bool> = OnceLock::new();
    *MACHINE.get_or_init(|| machine_from(std::env::var("WADOS_SESSION").ok().as_deref()))
}

fn machine_from(v: Option<&str>) -> bool {
    v == Some("machine")
}

/// What a power item's tooltip says outside the machine's session.
pub fn off_here(what: &str) -> String {
    format!("{what}: off outside the machine's own session")
}

/// How long idle before it locks: `WADOS_IDLE_LOCK_SECS` (0: never), else
/// 5 minutes in the machine's session and never elsewhere.
fn idle_after(machine: bool, secs: Option<&str>) -> Option<Duration> {
    match secs.map(|s| s.trim().parse::<u64>()) {
        Some(Ok(0)) => None,
        Some(Ok(n)) => Some(Duration::from_secs(n)),
        _ => machine.then_some(IDLE_LOCK),
    }
}

/// The start menu's power items (and the power key's menu's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Power {
    Lock,
    Server,
    Restart,
    Off,
}

impl Power {
    /// Its word in `wadsctl power …`.
    pub fn word(self) -> &'static str {
        match self {
            Power::Lock => "lock",
            Power::Server => "server",
            Power::Restart => "restart",
            Power::Off => "off",
        }
    }
}

pub struct Item {
    pub power: Power,
    pub icon: &'static str,
    pub label: &'static str,
    pub tip: String,
    /// It works here.
    pub on: bool,
}

/// The power items, worded as the spike's start menu has them.
pub fn items(machine: bool) -> Vec<Item> {
    let item = |power, icon, label: &'static str, on: bool, tip: Option<&str>| Item {
        power,
        icon,
        label,
        tip: tip.map(str::to_string).unwrap_or_else(|| if on { label.to_string() } else { off_here(label) }),
        on,
    };
    vec![
        item(Power::Lock, "lock", "Lock", true, None),
        item(Power::Server, "server", "Server mode", false, Some(SERVER_LATER)),
        item(Power::Restart, "rotate-ccw", "Restart", machine, None),
        item(Power::Off, "power", "Power off", machine, None),
    ]
}

pub struct Session {
    shell: Rc<Shell>,
    lock: Rc<Lock>,
    machine: bool,
    idle_after: Option<Duration>,
    /// The idle watch: watching, idle, or why it can't.
    idle: RefCell<String>,
    logind: Option<Rc<Logind>>,
    /// Why there's no logind, in the machine's session.
    no_logind: Option<String>,
    sleep: RefCell<Sleep<OwnedFd>>,
    menu: Rc<menu::Menu>,
    mixer: Mixer,
    volume: RefCell<Result<Volume, String>>,
    /// The keys' commands, as they came (for tests: `wadsctl session heard`).
    heard: RefCell<Vec<String>>,
}

impl Session {
    pub fn new(shell: &Rc<Shell>, lock: &Rc<Lock>) -> Rc<Self> {
        let machine = is_machine();
        let dry_run = std::env::var("WADOS_LOGIND_DRY_RUN").is_ok_and(|v| v == "1");
        let (logind, no_logind) = match machine.then(|| Logind::connect(dry_run)) {
            Some(Ok(l)) => (Some(l), None),
            Some(Err(e)) => {
                tracing::error!("logind: {e}");
                (None, Some(e))
            }
            None => (None, None),
        };
        let idle_after = idle_after(machine, std::env::var("WADOS_IDLE_LOCK_SECS").ok().as_deref());
        let (mixer, volumes) = Mixer::start();
        let me = Rc::new_cyclic(|w: &std::rc::Weak<Self>| {
            let w = w.clone();
            Self {
                shell: shell.clone(),
                lock: lock.clone(),
                machine,
                idle_after,
                idle: RefCell::new("off".into()),
                logind,
                no_logind,
                sleep: RefCell::default(),
                menu: menu::Menu::new(shell, machine, move |p| {
                    if let Some(me) = w.upgrade() {
                        me.chose(p);
                    }
                }),
                mixer,
                volume: RefCell::new(Err("not read yet".into())),
                heard: RefCell::default(),
            }
        });
        tracing::info!(
            "session: machine={machine} idle lock={} logind={}",
            idle_after.map(|d| format!("{}s", d.as_secs())).unwrap_or_else(|| "off".into()),
            me.logind_state()
        );
        let w = Rc::downgrade(&me);
        lock.set_power_off(move || {
            if let Some(me) = w.upgrade() {
                me.chose(Power::Off);
            }
        });
        me.watch_idle();
        me.watch_sleep();
        me.watch_volume(volumes);
        me.ctl();
        shell.set_session(&me);
        me
    }

    // ------------------------------------------------------------ verbs

    pub fn lock(&self) -> Result<(), String> {
        self.menu.close();
        self.lock.lock(&self.shell.app)
    }

    /// A power item: Lock, or (in the machine's session) Restart and Power off.
    pub fn power(&self, p: Power) -> Result<(), String> {
        let reboot = match p {
            Power::Lock => return self.lock(),
            Power::Server => return Err(SERVER_LATER.into()),
            Power::Restart => true,
            Power::Off => false,
        };
        let label = if reboot { "Restart" } else { "Power off" };
        if !self.machine {
            return Err(off_here(label));
        }
        let Some(l) = &self.logind else {
            return Err(format!("{label}: logind isn't there ({})", self.no_logind.as_deref().unwrap_or("?")));
        };
        self.menu.close();
        let s = self.shell.clone();
        l.power(reboot, move |r| match r {
            Ok(true) => s.toast(Level::Info, &format!("{label}: a dry run, so logind wasn't asked"), None),
            Ok(false) => {}
            Err(e) => s.toast(Level::Error, &format!("Couldn't {}: {e}", label.to_lowercase()), None),
        });
        Ok(())
    }

    /// Chosen in a menu: done, or toasted why not.
    fn chose(&self, p: Power) {
        if let Err(e) = self.power(p) {
            self.shell.toast(Level::Warn, &e, None);
        }
    }

    fn brightness(&self, up: bool) -> String {
        if !self.machine {
            return format!("error {}", off_here("Brightness"));
        }
        let Some(l) = &self.logind else { return "error logind isn't there".into() };
        let Some(b) = media::backlight() else { return "error this screen has no backlight to set".into() };
        let to = media::step(b.now, b.max, up);
        let s = self.shell.clone();
        let (now, max) = (b.now, b.max);
        l.set_brightness(&b.name, to, move |r| match r {
            Ok(false) => bar::set_brightness(&s, to as f64 / max as f64),
            Ok(true) => {}
            Err(e) => {
                tracing::warn!("brightness: {e}");
                s.toast(Level::Warn, &format!("Couldn't change the brightness: {e}"), None);
            }
        });
        let pct = |v: u32| (v as f64 * 100.0 / max as f64).round() as i64;
        format!("ok {}% {}", pct(now), pct(to))
    }

    // ------------------------------------------------------------ watching

    fn watch_idle(self: &Rc<Self>) {
        let Some(after) = self.idle_after else { return };
        let rx = idle::start(after);
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(ev) = rx.recv().await {
                let Some(me) = w.upgrade() else { return };
                let said = match ev {
                    idle::Said::Watching | idle::Said::Back => "watching".to_string(),
                    idle::Said::Idled => {
                        tracing::info!("idle for {}s: locking", after.as_secs());
                        if let Err(e) = me.lock() {
                            tracing::error!("the idle lock: {e}");
                        }
                        "idle".to_string()
                    }
                    idle::Said::Failed(e) => {
                        tracing::error!("idle: {e}");
                        format!("failed({e})")
                    }
                };
                *me.idle.borrow_mut() = said;
            }
        });
    }

    fn watch_sleep(self: &Rc<Self>) {
        let Some(l) = &self.logind else { return };
        let w = Rc::downgrade(self);
        l.on_prepare_for_sleep(move |going| {
            if let Some(me) = w.upgrade() {
                me.prepare_for_sleep(going);
            }
        });
        let w = Rc::downgrade(self);
        l.on_lock(move || {
            if let Some(me) = w.upgrade() {
                tracing::info!("logind asked this session to lock");
                if let Err(e) = me.lock() {
                    tracing::error!("{e}");
                }
            }
        });
        self.take_inhibitor();
    }

    /// logind's `PrepareForSleep`: lock, then let sleep go on; or, woken,
    /// hold the inhibitor again.
    fn prepare_for_sleep(self: &Rc<Self>, going: bool) {
        tracing::info!("logind: {}", if going { "going to sleep, so locking first" } else { "awake" });
        let next = self.sleep.borrow_mut().prepare(going);
        match next {
            Do::Lock => {
                self.menu.close();
                let w = Rc::downgrade(self);
                self.lock.lock_then(&self.shell.app, move |ok| {
                    let Some(me) = w.upgrade() else { return };
                    if !ok {
                        tracing::error!("the screen didn't lock before sleep");
                    }
                    me.sleep.borrow_mut().locked();
                    tracing::info!("locked: sleep can go on (the inhibitor's let go)");
                });
            }
            Do::Inhibit => self.take_inhibitor(),
        }
    }

    fn take_inhibitor(self: &Rc<Self>) {
        let Some(l) = &self.logind else { return };
        if !self.sleep.borrow_mut().ask() {
            return;
        }
        let w = Rc::downgrade(self);
        l.inhibit_sleep(move |r| {
            let Some(me) = w.upgrade() else { return };
            match r {
                Ok(fd) => {
                    tracing::info!("logind: holding a sleep delay inhibitor");
                    me.sleep.borrow_mut().inhibited(fd);
                }
                Err(e) => {
                    tracing::warn!("logind: no sleep inhibitor ({e}): sleep won't wait for the lock");
                    me.sleep.borrow_mut().refused(e);
                }
            }
        });
    }

    fn watch_volume(self: &Rc<Self>, rx: async_channel::Receiver<Result<Volume, String>>) {
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(v) = rx.recv().await {
                let Some(me) = w.upgrade() else { return };
                if let Err(e) = &v {
                    tracing::warn!("volume: {e}");
                }
                bar::set_volume(&me.shell, v.as_ref().ok().cloned());
                *me.volume.borrow_mut() = v;
            }
        });
        self.mixer.ask(Vol::Read);
    }

    // ------------------------------------------------------------ wadsctl

    fn logind_state(&self) -> String {
        match (&self.logind, &self.no_logind) {
            (Some(l), _) if l.dry_run() => "dry-run".into(),
            (Some(_), _) => "on".into(),
            (None, Some(e)) => format!("failed({e})"),
            (None, None) => "off".into(),
        }
    }

    /// `wadsctl session`.
    fn describe(&self) -> String {
        let asked = self.logind.as_ref().map(|l| l.asked()).unwrap_or_default();
        format!(
            "ok machine={} idle_lock={} idle={} inhibitor={} logind={} calls=[{}] locked={}",
            self.machine,
            self.idle_after.map(|d| format!("{}s", d.as_secs())).unwrap_or_else(|| "off".into()),
            self.idle.borrow(),
            if self.logind.is_some() { self.sleep.borrow().state() } else { "none".into() },
            self.logind_state(),
            asked.join(", "),
            self.lock.is_locked(),
        )
    }

    fn heard(&self, words: &str) {
        tracing::info!("heard: {words}");
        let mut h = self.heard.borrow_mut();
        h.push(words.to_string());
        let n = h.len();
        if n > 100 {
            h.drain(..n - 100);
        }
    }

    fn ctl(self: &Rc<Self>) {
        let answer = |r: Result<(), String>| match r {
            Ok(()) => "ok".to_string(),
            Err(e) => format!("error {e}"),
        };
        let w = Rc::downgrade(self);
        self.shell.on_ctl("session", move |args| {
            let Some(m) = w.upgrade() else { return "error gone".into() };
            match args {
                [] => m.describe(),
                ["heard"] => format!("ok {}", m.heard.borrow().join(" | ")),
                _ => "error session [heard]".into(),
            }
        });
        let w = Rc::downgrade(self);
        self.shell.on_ctl("power", move |args| {
            let Some(m) = w.upgrade() else { return "error gone".into() };
            match args {
                // The power key: its menu, or the lock screen's Power off.
                [] => {
                    m.heard("power");
                    if m.lock.is_locked() {
                        let focused = m.lock.focus_power();
                        return format!("ok locked{}", if focused { ": the keys are on its Power off" } else { "" });
                    }
                    m.menu.toggle();
                    format!("ok {}", if m.menu.is_open() { "open" } else { "closed" })
                }
                ["state"] => m.menu.state(),
                ["close"] => {
                    m.menu.close();
                    "ok".into()
                }
                ["lock"] => answer(m.power(Power::Lock)),
                ["restart"] => answer(m.power(Power::Restart)),
                ["off"] => answer(m.power(Power::Off)),
                _ => "error power [state|close|lock|restart|off]".into(),
            }
        });
        let w = Rc::downgrade(self);
        self.shell.on_ctl("volume", move |args| {
            let Some(m) = w.upgrade() else { return "error gone".into() };
            let v = match args {
                [] => {
                    return match &*m.volume.borrow() {
                        Ok(v) => format!("ok {}", media::describe(v)),
                        Err(e) => format!("error {e}"),
                    };
                }
                ["up"] => Vol::Up,
                ["down"] => Vol::Down,
                ["mute"] => Vol::Mute,
                _ => return "error volume [up|down|mute]".into(),
            };
            m.heard(&format!("volume {}", args[0]));
            m.mixer.ask(v);
            "ok".into()
        });
        let w = Rc::downgrade(self);
        self.shell.on_ctl("brightness", move |args| {
            let Some(m) = w.upgrade() else { return "error gone".into() };
            match args {
                [] => match media::backlight() {
                    Some(b) => format!("ok {} {}%", b.name, (b.now as f64 * 100.0 / b.max as f64).round() as i64),
                    None => "error this screen has no backlight".into(),
                },
                [dir @ ("up" | "down")] => {
                    m.heard(&format!("brightness {dir}"));
                    m.brightness(*dir == "up")
                }
                _ => "error brightness [up|down]".into(),
            }
        });
        let w = Rc::downgrade(self);
        self.shell.on_ctl("server", move |args| {
            let Some(m) = w.upgrade() else { return "error gone".into() };
            match args {
                // Ctrl+Alt+Delete: back from server mode.
                ["off"] => {
                    m.heard("server off");
                    "ok not in server mode (it comes in phase 10)".into()
                }
                _ => "error server off".into(),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_machine_s_own_session() {
        assert!(machine_from(Some("machine")));
        assert!(!machine_from(None));
        assert!(!machine_from(Some("")));
        assert!(!machine_from(Some("Machine")));
        assert!(!machine_from(Some("try")));
    }

    #[test]
    fn the_idle_lock() {
        assert_eq!(idle_after(true, None), Some(Duration::from_secs(300)));
        assert_eq!(idle_after(false, None), None, "a test or a laptop's window doesn't lock by itself");
        assert_eq!(idle_after(false, Some("5")), Some(Duration::from_secs(5)));
        assert_eq!(idle_after(true, Some(" 30 ")), Some(Duration::from_secs(30)));
        assert_eq!(idle_after(true, Some("0")), None);
        assert_eq!(idle_after(true, Some("soon")), Some(Duration::from_secs(300)));
        assert_eq!(idle_after(false, Some("soon")), None);
    }

    #[test]
    fn power_items_say_why_they_re_off() {
        let here: Vec<(Power, bool, String)> = items(false).into_iter().map(|i| (i.power, i.on, i.tip)).collect();
        assert_eq!(
            here,
            [
                (Power::Lock, true, "Lock".to_string()),
                (Power::Server, false, SERVER_LATER.to_string()),
                (Power::Restart, false, "Restart: off outside the machine's own session".to_string()),
                (Power::Off, false, "Power off: off outside the machine's own session".to_string()),
            ]
        );
        let machine: Vec<(Power, bool, String)> = items(true).into_iter().map(|i| (i.power, i.on, i.tip)).collect();
        assert_eq!(machine[2], (Power::Restart, true, "Restart".to_string()));
        assert_eq!(machine[3], (Power::Off, true, "Power off".to_string()));
        assert!(!machine[1].1, "server mode is phase 10's");
    }
}
