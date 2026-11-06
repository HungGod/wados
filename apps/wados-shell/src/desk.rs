//! The person's settings, as the shell follows them (wad-desk's, which
//! Settings writes): `~/.config/wados/settings.toml` read at the start and
//! again each time it changes, and the displays through labwc's output
//! management (wad-desk's outputs thread):
//!
//! - **the layouts kept** (`displays.toml`): when the set of displays is
//!   one a layout was kept for (at the start, or one plugged in or out),
//!   it's put back. Settings' own changes (and its 15 s trial) don't change
//!   the set, so they're left alone;
//! - **the primary**: the kept layout's (monitors.rs asks [`Desk::primary`]);
//! - **night light**: every display warmed while it's on (gamma ramps,
//!   which labwc gives back when the shell goes);
//! - **screen blank**: every display off, or on again (session's idle).
//!
//! `wadsctl desk` says what it knows: `ok night=… heads=[…] restored=…`.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::{gio, glib, prelude::*};
use wad_desk::layout::{self, Head, Layouts};
use wad_desk::outputs::{self, Outputs, Said};
use wad_desk::settings::{self, Settings};

type Sub<T> = Rc<dyn Fn(&T)>;

pub struct Desk {
    settings: RefCell<Settings>,
    subs: RefCell<Vec<Sub<Settings>>>,
    layout_subs: RefCell<Vec<Rc<dyn Fn()>>>,
    heads: RefCell<Vec<Head>>,
    outputs: Option<Outputs>,
    /// Why there's no output management (none: there is).
    no_outputs: Option<String>,
    /// The set of displays last seen (a change puts its kept layout back).
    set: RefCell<Vec<String>>,
    /// The night light asked of the outputs thread (kelvin), and why it failed.
    night: RefCell<(Option<u32>, Option<String>)>,
    /// What was put back last (for wadsctl).
    restored: RefCell<String>,
    _watch: RefCell<Option<gio::FileMonitor>>,
}

impl Desk {
    pub fn start() -> Rc<Self> {
        let (tx, rx) = async_channel::unbounded::<Said>();
        let (outputs, no_outputs) = match outputs::start(move |s| {
            let _ = tx.send_blocking(s);
        }) {
            Ok(o) => (Some(o), None),
            Err(e) => {
                tracing::warn!("displays: {e}");
                (None, Some(e))
            }
        };
        let me = Rc::new(Self {
            settings: RefCell::new(Settings::load()),
            subs: RefCell::default(),
            layout_subs: RefCell::default(),
            heads: RefCell::default(),
            outputs,
            no_outputs,
            set: RefCell::default(),
            night: RefCell::default(),
            restored: RefCell::new("nothing".into()),
            _watch: RefCell::default(),
        });
        // What it said as it connected (the displays now): known before
        // the shell picks its primary.
        while let Ok(s) = rx.try_recv() {
            me.heard(s);
        }
        let w = Rc::downgrade(&me);
        glib::spawn_future_local(async move {
            while let Ok(s) = rx.recv().await {
                let Some(me) = w.upgrade() else { return };
                me.heard(s);
            }
        });
        me.watch();
        me.night_light();
        me
    }

    pub fn settings(&self) -> Settings {
        self.settings.borrow().clone()
    }

    /// `f` each time the settings change (not at once).
    pub fn on_change(&self, f: impl Fn(&Settings) + 'static) {
        self.subs.borrow_mut().push(Rc::new(f));
    }

    /// `f` each time the displays or their kept layouts change.
    pub fn on_layout(&self, f: impl Fn() + 'static) {
        self.layout_subs.borrow_mut().push(Rc::new(f));
    }

    /// Changes the settings (the tray's night light), saved for Settings to see.
    pub fn update(&self, f: impl FnOnce(&mut Settings)) {
        match Settings::update(f) {
            Ok(s) => self.changed(s),
            Err(e) => tracing::warn!("settings: {e}"),
        }
    }

    /// The primary for the displays now (the kept one, `WADOS_PRIMARY`'s, the
    /// laptop's own, the leftmost): its connector.
    pub fn primary(&self) -> Option<String> {
        let env = std::env::var("WADOS_PRIMARY").ok();
        layout::primary_of(&self.heads.borrow(), &Layouts::load(), env.as_deref())
    }

    /// Every display off (blanked) or on.
    pub fn blank(&self, off: bool) {
        if let Some(o) = &self.outputs {
            o.power(!off);
        }
    }

    /// Why night light isn't working, if it isn't.
    pub fn night_failed(&self) -> Option<String> {
        self.night.borrow().1.clone()
    }

    /// `wadsctl desk`.
    pub fn describe(&self) -> String {
        let heads: Vec<String> = self
            .heads
            .borrow()
            .iter()
            .map(|h| {
                let (w, hh) = h.logical();
                format!("{}{}@{},{}:{}x{}", h.name, if h.enabled { "" } else { "(off)" }, h.x, h.y, w, hh)
            })
            .collect();
        let (k, failed) = self.night.borrow().clone();
        format!(
            "ok night={}{} heads=[{}] restored={}{}",
            k.map(|k| format!("{k}K")).unwrap_or_else(|| "off".into()),
            failed.map(|e| format!("(failed: {e})")).unwrap_or_default(),
            heads.join(","),
            self.restored.borrow(),
            self.no_outputs.as_ref().map(|e| format!(" outputs=failed({e})")).unwrap_or_default()
        )
    }

    fn heard(self: &Rc<Self>, s: Said) {
        match s {
            Said::Heads(heads) => {
                let set = layout::set_of(&heads);
                let fresh = *self.set.borrow() != set;
                *self.heads.borrow_mut() = heads.clone();
                if fresh {
                    *self.set.borrow_mut() = set;
                    self.restore(&heads);
                }
                self.layout_changed();
            }
            Said::Applied(_, r) => {
                if let Err(e) = r {
                    tracing::warn!("displays: the kept layout couldn't be put back: {e}");
                    *self.restored.borrow_mut() = format!("failed({e})");
                }
            }
            Said::NightFailed(e) => {
                tracing::warn!("night light: {e}");
                self.night.borrow_mut().1 = Some(e);
            }
            Said::Ended(e) => tracing::warn!("displays: the outputs thread ended: {e}"),
        }
    }

    /// The layout kept for `heads`' set, put back if they aren't in it.
    fn restore(&self, heads: &[Head]) {
        let Some(o) = &self.outputs else { return };
        let all = Layouts::load();
        let Some(l) = all.for_heads(heads) else {
            *self.restored.borrow_mut() = "none-kept".into();
            return;
        };
        let confs = l.confs(heads);
        if layout::already(heads, &confs) {
            *self.restored.borrow_mut() = "as-kept".into();
            return;
        }
        tracing::info!("displays: putting back the layout kept for {}", layout::set_of(heads).join(" + "));
        *self.restored.borrow_mut() = "applied".into();
        o.apply(confs, false);
    }

    fn layout_changed(&self) {
        let subs = self.layout_subs.borrow().clone();
        for f in subs {
            f();
        }
    }

    /// Watches `~/.config/wados` (a file renamed over another is a change there).
    fn watch(self: &Rc<Self>) {
        let dir = settings::dir();
        let _ = std::fs::create_dir_all(&dir);
        let m = match gio::File::for_path(&dir)
            .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("{}: can't watch it ({e}): settings changes need a restart", dir.display());
                return;
            }
        };
        let pending = Rc::new(RefCell::new(None::<glib::SourceId>));
        let w = Rc::downgrade(self);
        m.connect_changed(move |_, f, other, _| {
            let name = |x: Option<&gio::File>| x.and_then(|f| f.basename()).map(|b| b.to_string_lossy().into_owned());
            let names = [name(Some(f)), name(other)];
            let ours = |n: &str| names.iter().flatten().any(|x| x == n);
            let (s, d) = (ours("settings.toml"), ours("displays.toml"));
            if !s && !d {
                return;
            }
            // A burst of events for one write: read it once it's settled.
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let (w, p) = (w.clone(), pending.clone());
            let id = glib::timeout_add_local_once(Duration::from_millis(120), move || {
                p.borrow_mut().take();
                let Some(me) = w.upgrade() else { return };
                me.changed(Settings::load());
                me.layout_changed();
            });
            *pending.borrow_mut() = Some(id);
        });
        *self._watch.borrow_mut() = Some(m);
    }

    fn changed(&self, s: Settings) {
        if *self.settings.borrow() == s {
            return;
        }
        *self.settings.borrow_mut() = s.clone();
        self.night_light();
        let subs = self.subs.borrow().clone();
        for f in subs {
            f(&s);
        }
    }

    fn night_light(&self) {
        let d = &self.settings.borrow().display;
        let k = d.night_light.then(|| outputs::kelvin_for(d.night_temp));
        let mut n = self.night.borrow_mut();
        if n.0 == k {
            return;
        }
        *n = (k, None);
        if let Some(o) = &self.outputs {
            o.night(k);
        } else {
            n.1 = self.no_outputs.clone();
        }
    }
}
