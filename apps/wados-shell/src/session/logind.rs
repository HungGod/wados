//! logind, on the system bus (only in the machine's own session):
//! `Reboot` and `PowerOff` (interactive: polkit asks if it must),
//! `SetBrightness` on this session (no root), the sleep delay inhibitor,
//! `PrepareForSleep`, and this session's `Lock`.
//!
//! `WADOS_LOGIND_DRY_RUN=1`: the calls that change the machine (Reboot,
//! PowerOff, SetBrightness) are logged and kept for `wadsctl session`
//! instead of made, so a test on a laptop can check the call without making
//! it. Reading, the inhibitor and the signals stay real.

use std::cell::RefCell;
use std::os::fd::OwnedFd;
use std::rc::Rc;

use gtk::{gio, glib, prelude::*};

const LOGIN1: &str = "org.freedesktop.login1";
const PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION: &str = "org.freedesktop.login1.Session";
/// This session (or, for a process outside one, the user's display session).
const AUTO: &str = "/org/freedesktop/login1/session/auto";

pub struct Logind {
    bus: gio::DBusConnection,
    dry_run: bool,
    /// A dry run's calls, as they'd have been made.
    asked: RefCell<Vec<String>>,
    subs: RefCell<Vec<gio::SignalSubscription>>,
}

/// `PrepareForSleep`'s argument: true going to sleep, false woken.
pub fn prepare_for_sleep(args: &glib::Variant) -> Option<bool> {
    args.get::<(bool,)>().map(|(going,)| going)
}

impl Logind {
    pub fn connect(dry_run: bool) -> Result<Rc<Self>, String> {
        let bus = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)
            .map_err(|e| format!("the system bus: {}", e.message()))?;
        Ok(Rc::new(Self { bus, dry_run, asked: RefCell::default(), subs: RefCell::default() }))
    }

    pub fn dry_run(&self) -> bool {
        self.dry_run
    }

    /// A dry run's calls so far.
    pub fn asked(&self) -> Vec<String> {
        self.asked.borrow().clone()
    }

    /// A call that changes the machine: made, or in a dry run noted.
    /// `done(Ok(true))`: it was a dry run.
    #[allow(clippy::too_many_arguments)]
    fn change(
        &self,
        path: &str,
        iface: &str,
        method: &str,
        args: glib::Variant,
        said: String,
        timeout_ms: i32,
        done: impl FnOnce(Result<bool, String>) + 'static,
    ) {
        if self.dry_run {
            tracing::info!("logind, a dry run (not asked): {said}");
            self.asked.borrow_mut().push(said);
            done(Ok(true));
            return;
        }
        tracing::info!("logind: {said}");
        let call = self.bus.call_future(
            Some(LOGIN1),
            path,
            iface,
            method,
            Some(&args),
            None,
            gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
            timeout_ms,
        );
        glib::spawn_future_local(async move {
            done(call.await.map(|_| false).map_err(|e| e.message().to_string()));
        });
    }

    /// Restart (`Reboot`) or power off (`PowerOff`), interactive: polkit asks if it must.
    pub fn power(&self, reboot: bool, done: impl FnOnce(Result<bool, String>) + 'static) {
        let method = if reboot { "Reboot" } else { "PowerOff" };
        // polkit may ask for a password: no timeout.
        self.change(PATH, MANAGER, method, (true,).to_variant(), format!("{method}(true)"), i32::MAX, done);
    }

    /// The backlight `name` to `value` (of its max_brightness).
    pub fn set_brightness(&self, name: &str, value: u32, done: impl FnOnce(Result<bool, String>) + 'static) {
        let said = format!("SetBrightness(\"backlight\", \"{name}\", {value})");
        self.change(AUTO, SESSION, "SetBrightness", ("backlight", name, value).to_variant(), said, 5000, done);
    }

    /// A "sleep" delay inhibitor: sleep waits (up to logind's
    /// InhibitDelayMaxSec) until its fd is closed.
    pub fn inhibit_sleep(&self, done: impl FnOnce(Result<OwnedFd, String>) + 'static) {
        let args = ("sleep", "WadSpaces", "Locks the screen before the machine sleeps", "delay").to_variant();
        let call = self.bus.call_with_unix_fd_list_future(
            Some(LOGIN1),
            PATH,
            MANAGER,
            "Inhibit",
            Some(&args),
            None,
            gio::DBusCallFlags::NONE,
            5000,
            None::<&gio::UnixFDList>,
        );
        glib::spawn_future_local(async move {
            let fd = call.await.map_err(|e| e.message().to_string()).and_then(|(v, fds)| {
                let i = v.get::<(glib::variant::Handle,)>().ok_or_else(|| format!("Inhibit said {v}"))?.0.0;
                fds.ok_or_else(|| "Inhibit sent no fd".to_string())?.get(i).map_err(|e| e.message().to_string())
            });
            done(fd);
        });
    }

    /// `f` at each `PrepareForSleep`.
    pub fn on_prepare_for_sleep(&self, f: impl Fn(bool) + 'static) {
        let sub = self.bus.subscribe_to_signal(
            Some(LOGIN1),
            Some(MANAGER),
            Some("PrepareForSleep"),
            Some(PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |s| match prepare_for_sleep(s.parameters) {
                Some(going) => f(going),
                None => tracing::warn!("logind: PrepareForSleep{} isn't (b)", s.parameters),
            },
        );
        self.subs.borrow_mut().push(sub);
    }

    /// `f` when logind asks this session to lock (`loginctl lock-session`,
    /// a lid set to lock).
    pub fn on_lock(self: &Rc<Self>, f: impl Fn() + 'static) {
        let me = Rc::downgrade(self);
        let call = self.bus.call_future(
            Some(LOGIN1),
            PATH,
            MANAGER,
            "GetSession",
            Some(&("auto",).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            5000,
        );
        glib::spawn_future_local(async move {
            let path = match call.await {
                Ok(v) => v.child_value(0).str().map(str::to_string),
                Err(e) => {
                    tracing::warn!("logind: no session of ours ({}): its Lock won't reach the shell", e.message());
                    return;
                }
            };
            let (Some(me), Some(path)) = (me.upgrade(), path) else { return };
            tracing::info!("logind: this session is {path}");
            let sub = me.bus.subscribe_to_signal(
                Some(LOGIN1),
                Some(SESSION),
                Some("Lock"),
                Some(&path),
                None,
                gio::DBusSignalFlags::NONE,
                move |_| f(),
            );
            me.subs.borrow_mut().push(sub);
        });
    }
}
