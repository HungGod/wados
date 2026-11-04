//! Idle, as labwc sees it (ext-idle-notify-v1), on a Wayland connection of
//! the shell's own in a thread of its own (as wm.rs does foreign-toplevel).
//! The notification takes idle inhibitors into account, so a wadspace
//! playing a video full screen (`zwp_idle_inhibit_manager_v1`, which a
//! wadspace's socket keeps) holds the lock off.

use std::time::Duration;

use wayland_client::{
    Connection, Dispatch, QueueHandle,
    protocol::{wl_registry, wl_seat},
};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self as notification, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Said {
    /// Watching: no input for the timeout will say so.
    Watching,
    /// No input for the timeout.
    Idled,
    /// Input again.
    Back,
    /// It can't watch (why).
    Failed(String),
}

/// Starts watching for `after` without input.
pub fn start(after: Duration) -> async_channel::Receiver<Said> {
    let (tx, rx) = async_channel::unbounded();
    let spawned = std::thread::Builder::new().name("idle".into()).spawn({
        let tx = tx.clone();
        move || {
            if let Err(e) = run(after, &tx) {
                let _ = tx.send_blocking(Said::Failed(e));
            }
        }
    });
    if let Err(e) = spawned {
        let _ = tx.send_blocking(Said::Failed(format!("idle: {e}")));
    }
    rx
}

struct State {
    notifier: Option<ExtIdleNotifierV1>,
    seat: Option<wl_seat::WlSeat>,
    tx: async_channel::Sender<Said>,
}

fn run(after: Duration, tx: &async_channel::Sender<Said>) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut st = State { notifier: None, seat: None, tx: tx.clone() };
    queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    let (Some(notifier), Some(seat)) = (&st.notifier, &st.seat) else {
        return Err("the compositor offers no ext_idle_notifier_v1 (or no seat)".into());
    };
    let ms = u32::try_from(after.as_millis()).unwrap_or(u32::MAX);
    let _notification = notifier.get_idle_notification(ms, seat, &qh, ());
    let _ = tx.send_blocking(Said::Watching);
    loop {
        queue.blocking_dispatch(&mut st).map_err(|e| e.to_string())?;
        if tx.is_closed() {
            return Ok(());
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        st: &mut Self,
        r: &wl_registry::WlRegistry,
        e: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = e {
            match interface.as_str() {
                // v1's get_idle_notification is all it needs (v2 adds one that ignores inhibitors).
                "ext_idle_notifier_v1" => st.notifier = Some(r.bind(name, version.min(1), qh, ())),
                "wl_seat" if st.seat.is_none() => st.seat = Some(r.bind(name, version.min(7), qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for State {
    fn event(
        st: &mut Self,
        _: &ExtIdleNotificationV1,
        e: notification::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let said = match e {
            notification::Event::Idled => Said::Idled,
            notification::Event::Resumed => Said::Back,
            _ => return,
        };
        let _ = st.tx.send_blocking(said);
    }
}

wayland_client::delegate_noop!(State: ExtIdleNotifierV1);
wayland_client::delegate_noop!(State: ignore wl_seat::WlSeat);
