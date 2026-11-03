//! The windows on the screen, and moving them (SHELL-REQUIREMENTS'
//! "Windows and wadspaces on the screen"): labwc's wlr-foreign-toplevel, on
//! a Wayland connection of the shell's own, in a thread of its own.
//!
//! A wadspace's window is the one whose app id is `io.wadspaces.<id>`: labwc
//! (with host/labwc/0001-…) reports a sandboxed client's security-context
//! app id, which wadd set and the wadspace can't change
//! (docs/decisions/0001-labwc.md). The machine's own apps keep theirs
//! (`io.wados.*`).

use std::collections::BTreeMap;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::mpsc;

use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, event_created_child,
    protocol::{wl_output, wl_registry, wl_seat},
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self as handle, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self as manager, ZwlrForeignToplevelManagerV1},
};

/// A window, as the shell knows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Top {
    pub id: u32,
    pub app_id: String,
    pub title: String,
    /// The outputs it's on, by connector name ("HEADLESS-1", "eDP-1").
    pub outputs: Vec<String>,
    pub activated: bool,
    pub maximized: bool,
    pub minimized: bool,
    pub fullscreen: bool,
}

impl Top {
    /// The wadspace it is, if it's one.
    pub fn wadspace(&self) -> Option<&str> {
        self.app_id.strip_prefix("io.wadspaces.")
    }
}

/// Which window a command is for: `#<id>`, or the first with that app id.
#[derive(Clone, Debug)]
pub struct Which(pub String);

#[derive(Clone, Debug)]
pub enum Cmd {
    Activate(Which),
    /// Full screen, on that output (by connector) or wherever it is.
    Fullscreen(Which, Option<String>),
    Unfullscreen(Which),
    Minimize(Which),
    Unminimize(Which),
    Close(Which),
}

/// The shell's end: send commands, read the windows.
pub struct Wm {
    tx: mpsc::Sender<Cmd>,
    wake: OwnedFd,
}

impl Wm {
    pub fn send(&self, cmd: Cmd) {
        if self.tx.send(cmd).is_ok() {
            // SAFETY: one byte from a valid buffer to our own pipe.
            unsafe { libc::write(self.wake.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
        }
    }
}

/// Starts the thread. Every change to the windows comes on the channel, as
/// the whole list.
pub fn start() -> Result<(Wm, async_channel::Receiver<Vec<Top>>), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("wm: {e}"))?;
    let (tx, rx) = mpsc::channel();
    let (out, tops) = async_channel::unbounded();
    let mut fds = [0; 2];
    // SAFETY: pipe2 fills two fds, owned from here on.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } < 0 {
        return Err(format!("wm: pipe: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: both fresh from pipe2.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    std::thread::Builder::new()
        .name("wm".into())
        .spawn(move || {
            if let Err(e) = run(conn, rx, read, out) {
                tracing::error!("wm: {e}");
            }
        })
        .map_err(|e| format!("wm: {e}"))?;
    Ok((Wm { tx, wake: write }, tops))
}

#[derive(Default)]
struct State {
    manager: Option<ZwlrForeignToplevelManagerV1>,
    seat: Option<wl_seat::WlSeat>,
    outputs: BTreeMap<u32, (wl_output::WlOutput, String)>,
    handles: BTreeMap<u32, (ZwlrForeignToplevelHandleV1, Top)>,
    changed: bool,
}

impl State {
    fn find(&self, which: &Which) -> Option<&ZwlrForeignToplevelHandleV1> {
        if let Some(n) = which.0.strip_prefix('#').and_then(|n| n.parse::<u32>().ok()) {
            return self.handles.get(&n).map(|(h, _)| h);
        }
        self.handles.values().find(|(_, t)| t.app_id == which.0).map(|(h, _)| h)
    }

    fn output(&self, name: &str) -> Option<&wl_output::WlOutput> {
        self.outputs.values().find(|(_, n)| n == name).map(|(o, _)| o)
    }

    fn apply(&self, cmd: Cmd) {
        let who = match &cmd {
            Cmd::Activate(w)
            | Cmd::Fullscreen(w, _)
            | Cmd::Unfullscreen(w)
            | Cmd::Minimize(w)
            | Cmd::Unminimize(w)
            | Cmd::Close(w) => w,
        };
        let Some(h) = self.find(who) else {
            tracing::warn!("wm: no window {}", who.0);
            return;
        };
        match cmd {
            Cmd::Activate(_) => {
                if let Some(seat) = &self.seat {
                    h.unset_minimized();
                    h.activate(seat);
                }
            }
            Cmd::Fullscreen(_, output) => h.set_fullscreen(output.as_deref().and_then(|n| self.output(n))),
            Cmd::Unfullscreen(_) => h.unset_fullscreen(),
            Cmd::Minimize(_) => h.set_minimized(),
            Cmd::Unminimize(_) => h.unset_minimized(),
            Cmd::Close(_) => h.close(),
        }
    }

    fn list(&self) -> Vec<Top> {
        self.handles.values().map(|(_, t)| t.clone()).collect()
    }
}

fn run(
    conn: Connection,
    rx: mpsc::Receiver<Cmd>,
    wake: OwnedFd,
    out: async_channel::Sender<Vec<Top>>,
) -> Result<(), String> {
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut st = State::default();
    queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    if st.manager.is_none() {
        return Err("the compositor offers no zwlr_foreign_toplevel_manager_v1".into());
    }
    queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    let _ = out.send_blocking(st.list());
    loop {
        queue.dispatch_pending(&mut st).map_err(|e| e.to_string())?;
        if std::mem::take(&mut st.changed) && out.send_blocking(st.list()).is_err() {
            return Ok(());
        }
        conn.flush().map_err(|e| e.to_string())?;
        let Some(guard) = queue.prepare_read() else { continue };
        let mut fds = [
            libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: wake.as_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 },
        ];
        // SAFETY: two valid pollfds.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e.to_string());
        }
        if fds[0].revents & libc::POLLIN != 0 {
            guard.read().map_err(|e| e.to_string())?;
        } else {
            drop(guard);
        }
        if fds[1].revents & libc::POLLIN != 0 {
            let mut buf = [0u8; 64];
            // SAFETY: reading into a local buffer from our own pipe.
            while unsafe { libc::read(wake.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
            while let Ok(cmd) = rx.try_recv() {
                st.apply(cmd);
            }
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
                "zwlr_foreign_toplevel_manager_v1" => st.manager = Some(r.bind(name, version.min(3), qh, ())),
                "wl_seat" if st.seat.is_none() => st.seat = Some(r.bind(name, version.min(7), qh, ())),
                "wl_output" if version >= 4 => {
                    let o: wl_output::WlOutput = r.bind(name, 4, qh, ());
                    st.outputs.insert(o.id().protocol_id(), (o, String::new()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        st: &mut Self,
        o: &wl_output::WlOutput,
        e: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = e
            && let Some(entry) = st.outputs.get_mut(&o.id().protocol_id())
        {
            entry.1 = name;
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(
        st: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        e: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let manager::Event::Toplevel { toplevel } = e {
            let id = toplevel.id().protocol_id();
            st.handles.insert(id, (toplevel, Top { id, ..Top::default() }));
        }
    }
    event_created_child!(State, ZwlrForeignToplevelManagerV1, [
        manager::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(
        st: &mut Self,
        h: &ZwlrForeignToplevelHandleV1,
        e: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = h.id().protocol_id();
        let names: BTreeMap<u32, String> = st.outputs.iter().map(|(k, (_, n))| (*k, n.clone())).collect();
        let Some((_, t)) = st.handles.get_mut(&id) else { return };
        match e {
            handle::Event::AppId { app_id } => t.app_id = app_id,
            handle::Event::Title { title } => t.title = title,
            handle::Event::OutputEnter { output } => {
                if let Some(n) = names.get(&output.id().protocol_id())
                    && !t.outputs.contains(n)
                {
                    t.outputs.push(n.clone());
                }
            }
            handle::Event::OutputLeave { output } => {
                if let Some(n) = names.get(&output.id().protocol_id()) {
                    t.outputs.retain(|x| x != n);
                }
            }
            handle::Event::State { state } => {
                let s: Vec<u32> = state.as_chunks::<4>().0.iter().map(|c| u32::from_ne_bytes(*c)).collect();
                let has = |v: handle::State| s.contains(&(v as u32));
                t.maximized = has(handle::State::Maximized);
                t.minimized = has(handle::State::Minimized);
                t.activated = has(handle::State::Activated);
                t.fullscreen = has(handle::State::Fullscreen);
            }
            handle::Event::Done => st.changed = true,
            handle::Event::Closed => {
                st.handles.remove(&id);
                st.changed = true;
            }
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(State: ignore wl_seat::WlSeat);
