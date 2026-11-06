//! Idle, as labwc sees it (ext-idle-notify-v1), on a Wayland connection of
//! the shell's own in a thread of its own (as wm.rs does foreign-toplevel).
//! A notification each for locking, blanking the screen and suspending,
//! after their own times (Settings ▸ Lock Screen & Privacy and ▸ Power);
//! [`Idle::set`] gives it new times, and it starts again with them. The
//! notifications take idle inhibitors into account, so a wadspace playing
//! a video full screen (`zwp_idle_inhibit_manager_v1`, which a wadspace's
//! socket keeps) holds them off.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use wayland_client::{
    Connection, Dispatch, QueueHandle,
    protocol::{wl_registry, wl_seat},
};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self as notification, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};

/// What idle does after a while.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Lock,
    Blank,
    Suspend,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Lock => "lock",
            Kind::Blank => "blank",
            Kind::Suspend => "suspend",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Said {
    /// Watching these.
    Watching,
    /// No input for `Kind`'s time.
    Idled(Kind),
    /// Input again (after `Kind`'s time had gone by).
    Back(Kind),
    /// It can't watch (why).
    Failed(String),
}

/// The watch's handle: dropping it ends the thread.
pub struct Idle {
    tx: mpsc::Sender<Vec<(Kind, Duration)>>,
    wake: Arc<OwnedFd>,
}

impl Idle {
    /// Watches for these times instead.
    pub fn set(&self, times: Vec<(Kind, Duration)>) {
        let _ = self.tx.send(times);
        poke(&self.wake);
    }
}

impl Drop for Idle {
    fn drop(&mut self) {
        poke(&self.wake);
    }
}

fn poke(fd: &OwnedFd) {
    let one: u64 = 1;
    // SAFETY: an eventfd takes 8 bytes.
    unsafe { libc::write(fd.as_raw_fd(), &one as *const u64 as *const libc::c_void, 8) };
}

/// Starts watching for `times` without input (none: watching for nothing, yet).
pub fn start(times: Vec<(Kind, Duration)>) -> (Option<Idle>, async_channel::Receiver<Said>) {
    let (out, rx) = async_channel::unbounded();
    // SAFETY: a new eventfd, ours alone.
    let raw = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if raw < 0 {
        let _ = out.send_blocking(Said::Failed(format!("eventfd: {}", std::io::Error::last_os_error())));
        return (None, rx);
    }
    // SAFETY: raw is a valid fd we own.
    let wake = Arc::new(unsafe { OwnedFd::from_raw_fd(raw) });
    let (tx, cmds) = mpsc::channel();
    let _ = tx.send(times);
    let w = wake.clone();
    let spawned = std::thread::Builder::new().name("idle".into()).spawn({
        let out = out.clone();
        move || {
            if let Err(e) = run(&out, &cmds, &w) {
                let _ = out.send_blocking(Said::Failed(e));
            }
        }
    });
    if let Err(e) = spawned {
        let _ = out.send_blocking(Said::Failed(format!("idle: {e}")));
        return (None, rx);
    }
    (Some(Idle { tx, wake }), rx)
}

struct State {
    notifier: Option<ExtIdleNotifierV1>,
    seat: Option<wl_seat::WlSeat>,
    tx: async_channel::Sender<Said>,
}

fn run(
    tx: &async_channel::Sender<Said>,
    cmds: &mpsc::Receiver<Vec<(Kind, Duration)>>,
    wake: &OwnedFd,
) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut st = State { notifier: None, seat: None, tx: tx.clone() };
    queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    let (Some(notifier), Some(seat)) = (st.notifier.clone(), st.seat.clone()) else {
        return Err("the compositor offers no ext_idle_notifier_v1 (or no seat)".into());
    };
    let mut watching: Vec<ExtIdleNotificationV1> = vec![];
    loop {
        loop {
            match cmds.try_recv() {
                Ok(times) => {
                    for n in watching.drain(..) {
                        n.destroy();
                    }
                    for (k, after) in times {
                        let ms = u32::try_from(after.as_millis()).unwrap_or(u32::MAX).max(1000);
                        watching.push(notifier.get_idle_notification(ms, &seat, &qh, k));
                    }
                    let _ = tx.send_blocking(Said::Watching);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
        if tx.is_closed() {
            return Ok(());
        }
        conn.flush().map_err(|e| e.to_string())?;
        queue.dispatch_pending(&mut st).map_err(|e| e.to_string())?;
        let Some(guard) = queue.prepare_read() else { continue };
        let mut fds = [
            libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: wake.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        ];
        // SAFETY: two valid pollfds.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("poll: {}", std::io::Error::last_os_error()));
        }
        if fds[0].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            return Err("the compositor hung up".into());
        }
        if fds[0].revents & libc::POLLIN != 0 {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.to_string()),
            }
        } else {
            drop(guard);
        }
        if fds[1].revents & libc::POLLIN != 0 {
            let mut buf = 0u64;
            // SAFETY: an eventfd gives 8 bytes.
            unsafe { libc::read(wake.as_raw_fd(), &mut buf as *mut u64 as *mut libc::c_void, 8) };
        }
        queue.dispatch_pending(&mut st).map_err(|e| e.to_string())?;
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

impl Dispatch<ExtIdleNotificationV1, Kind> for State {
    fn event(
        st: &mut Self,
        _: &ExtIdleNotificationV1,
        e: notification::Event,
        k: &Kind,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let said = match e {
            notification::Event::Idled => Said::Idled(*k),
            notification::Event::Resumed => Said::Back(*k),
            _ => return,
        };
        let _ = st.tx.send_blocking(said);
    }
}

wayland_client::delegate_noop!(State: ExtIdleNotifierV1);
wayland_client::delegate_noop!(State: ignore wl_seat::WlSeat);
