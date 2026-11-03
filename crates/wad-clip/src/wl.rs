//! A compositor's clipboard through wlr-data-control: told of every copy (no
//! window or focus needed, which is why the protocol exists), able to read
//! it and to make copies of its own. sway and labwc both speak it.
//!
//! Each connection runs on its own thread: the Wayland socket and a wake-up
//! pipe polled together. Copies are read on a thread each (so the loop can
//! still serve this side's own copies meanwhile), and served on a thread
//! each, whatever their size.

use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, event_created_child};
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_device_v1::{
    self as device, ZwlrDataControlDeviceV1,
};
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_manager_v1::ZwlrDataControlManagerV1;
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_offer_v1::{
    self as offer, ZwlrDataControlOfferV1,
};
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_source_v1::{
    self as source, ZwlrDataControlSourceV1,
};

use crate::content::{self, Content, Item};

/// What happened to the clipboard.
#[derive(Debug)]
pub enum Change {
    /// Someone else copied this (read, as far as it's worth carrying).
    Copied(Content),
    /// Nothing is copied now (whoever had copied went away).
    Cleared,
    /// The connection ended, and why.
    Closed(String),
}

type OnChange = Arc<dyn Fn(Change) + Send + Sync>;

enum Cmd {
    Set(Arc<Content>),
    Clear,
}

/// A compositor's clipboard, followed on its own thread.
#[derive(Clone)]
pub struct Clipboard {
    tx: mpsc::Sender<Cmd>,
    wake: Arc<OwnedFd>,
}

impl Clipboard {
    fn send(&self, cmd: Cmd) {
        if self.tx.send(cmd).is_ok() {
            let _ = nix::unistd::write(&*self.wake, &[1]);
        }
    }

    /// Copies `c` (as `me`, so it's known for ours when it comes back).
    pub fn set(&self, c: Arc<Content>) {
        self.send(Cmd::Set(c));
    }

    /// Nothing copied any more (a password manager cleared its copy).
    pub fn clear(&self) {
        self.send(Cmd::Clear);
    }
}

/// A Wayland display's socket: a path, or a name in $XDG_RUNTIME_DIR.
pub fn socket_path(display: &str) -> PathBuf {
    let p = Path::new(display);
    if p.is_absolute() {
        return p.into();
    }
    PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_default()).join(display)
}

/// $WAYLAND_DISPLAY's socket (wayland-0 if unset).
pub fn env_socket() -> PathBuf {
    socket_path(&std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into()))
}

/// Follows the clipboard of the compositor at `socket`, as `me`. `on_change`
/// runs on the clipboard's threads, never for `me`'s own copies.
pub fn watch(socket: &Path, me: &str, on_change: impl Fn(Change) + Send + Sync + 'static) -> Result<Clipboard, String> {
    let stream = UnixStream::connect(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    watch_stream(stream, me, on_change)
}

/// The same, over a connection someone else made (wadd, for a bridge on
/// the machine that can't reach either socket itself).
pub fn watch_stream(
    stream: UnixStream,
    me: &str,
    on_change: impl Fn(Change) + Send + Sync + 'static,
) -> Result<Clipboard, String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let (globals, queue) = registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let manager: ZwlrDataControlManagerV1 =
        globals.bind(&qh, 1..=2, ()).map_err(|_| "the compositor has no wlr-data-control".to_string())?;
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=8, ()).map_err(|_| "the compositor has no seat".to_string())?;
    let device = manager.get_data_device(&seat, &qh, ());
    let (wake_r, wake_w) = nix::unistd::pipe2(OFlag::O_CLOEXEC | OFlag::O_NONBLOCK).map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::channel();
    let on_change: OnChange = Arc::new(on_change);
    let state = State {
        me: me.into(),
        conn,
        manager,
        device,
        generation: Arc::new(AtomicU64::new(0)),
        on_change: on_change.clone(),
        offer: None,
        finished: false,
    };
    std::thread::Builder::new()
        .name("wad-clip".into())
        .spawn(move || {
            let why = run(queue, state, rx, wake_r);
            on_change(Change::Closed(why));
        })
        .map_err(|e| e.to_string())?;
    Ok(Clipboard { tx, wake: Arc::new(wake_w) })
}

struct State {
    me: String,
    conn: Connection,
    manager: ZwlrDataControlManagerV1,
    device: ZwlrDataControlDeviceV1,
    /// Bumped by each new selection: a slow read of an older one is dropped.
    generation: Arc<AtomicU64>,
    on_change: OnChange,
    /// The selection now (destroyed when the next arrives).
    offer: Option<ZwlrDataControlOfferV1>,
    finished: bool,
}

fn run(mut queue: EventQueue<State>, mut st: State, rx: mpsc::Receiver<Cmd>, wake: OwnedFd) -> String {
    let qh = queue.handle();
    loop {
        if let Err(e) = queue.dispatch_pending(&mut st) {
            return e.to_string();
        }
        if st.finished {
            return "the compositor took the clipboard away".into();
        }
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                Cmd::Set(c) => st.copy(&qh, c),
                Cmd::Clear => st.device.set_selection(None),
            }
        }
        if let Err(e) = queue.flush()
            && !matches!(&e, wayland_client::backend::WaylandError::Io(io) if io.kind() == std::io::ErrorKind::WouldBlock)
        {
            return e.to_string();
        }
        let Some(guard) = queue.prepare_read() else { continue };
        let (socket_ready, woken) = {
            let mut fds =
                [PollFd::new(guard.connection_fd(), PollFlags::POLLIN), PollFd::new(wake.as_fd(), PollFlags::POLLIN)];
            match poll(&mut fds, PollTimeout::NONE) {
                Ok(_) => {}
                Err(Errno::EINTR) => continue,
                Err(e) => return e.to_string(),
            }
            let ready = |f: &PollFd| f.revents().is_some_and(|r| !r.is_empty());
            (ready(&fds[0]), ready(&fds[1]))
        };
        if socket_ready {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return e.to_string(),
            }
        } else {
            drop(guard);
        }
        if woken {
            let mut buf = [0u8; 64];
            while nix::unistd::read(&wake, &mut buf).is_ok_and(|n| n > 0) {}
        }
    }
}

impl State {
    fn copy(&mut self, qh: &QueueHandle<State>, c: Arc<Content>) {
        let me = content::marker(&self.me);
        let src = self.manager.create_data_source(qh, Served { content: c.clone(), me: self.me.clone() });
        for item in &c.items {
            src.offer(item.mime.clone());
        }
        src.offer(me);
        self.device.set_selection(Some(&src));
    }

    fn selection(&mut self, id: Option<ZwlrDataControlOfferV1>) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(old) = self.offer.take() {
            old.destroy();
        }
        let Some(offer) = id else {
            (self.on_change)(Change::Cleared);
            return;
        };
        self.offer = Some(offer.clone());
        let mimes = offer.data::<Mutex<Vec<String>>>().map(|m| m.lock().unwrap().clone()).unwrap_or_default();
        if content::origin(&mimes) == Some(self.me.as_str()) {
            return;
        }
        let wanted = content::wanted(&mimes);
        if wanted.is_empty() {
            return;
        }
        let (conn, current, on_change) = (self.conn.clone(), self.generation.clone(), self.on_change.clone());
        std::thread::spawn(move || {
            let mut items = Vec::new();
            let mut total = 0;
            for mime in wanted {
                if current.load(Ordering::SeqCst) != generation {
                    return;
                }
                if let Some(data) = read(&conn, &offer, &mime, content::MAX_ITEM.min(content::MAX_TOTAL - total)) {
                    total += data.len();
                    items.push(Item { mime, data });
                }
            }
            if !items.is_empty() && current.load(Ordering::SeqCst) == generation {
                on_change(Change::Copied(Content { items }));
            }
        });
    }
}

/// One type of an offer's data (None: too big, too slow, or refused).
fn read(conn: &Connection, offer: &ZwlrDataControlOfferV1, mime: &str, limit: usize) -> Option<Vec<u8>> {
    // Blocking for the writer (it's their end); only ours is non-blocking.
    let (r, w) = nix::unistd::pipe2(OFlag::O_CLOEXEC).ok()?;
    offer.receive(mime.into(), w.as_fd());
    conn.flush().ok()?;
    drop(w);
    fcntl(&r, FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 << 10];
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        let mut fds = [PollFd::new(r.as_fd(), PollFlags::POLLIN)];
        match poll(&mut fds, PollTimeout::try_from(left.as_millis().min(60_000) as i32).ok()?) {
            Ok(0) => return None,
            Ok(_) | Err(Errno::EINTR) => {}
            Err(_) => return None,
        }
        match nix::unistd::read(&r, &mut buf) {
            Ok(0) => return Some(out),
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if out.len() > limit {
                    return None;
                }
            }
            Err(Errno::EAGAIN | Errno::EINTR) => {}
            Err(_) => return None,
        }
    }
}

/// Writes `data` to a reader's pipe, then closes it.
fn serve(fd: OwnedFd, data: &[u8]) {
    let mut rest = data;
    while !rest.is_empty() {
        match nix::unistd::write(&fd, rest) {
            Ok(n) => rest = &rest[n..],
            Err(Errno::EINTR) => {}
            Err(Errno::EAGAIN) => {
                let mut fds = [PollFd::new(fd.as_fd(), PollFlags::POLLOUT)];
                if !matches!(poll(&mut fds, PollTimeout::from(5000u16)), Ok(n) if n > 0) {
                    return;
                }
            }
            // The reader went away (Rust ignores SIGPIPE, so this is EPIPE).
            Err(_) => return,
        }
    }
}

/// A copy of ours, as served.
struct Served {
    content: Arc<Content>,
    me: String,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwlrDataControlManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlManagerV1,
        _: <ZwlrDataControlManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for State {
    fn event(
        st: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            // Its types arrive on it, before the selection names it.
            device::Event::DataOffer { .. } => {}
            device::Event::Selection { id } => st.selection(id),
            // Middle-click paste stays each wadspace's own.
            device::Event::PrimarySelection { id: Some(o) } => o.destroy(),
            device::Event::Finished => st.finished = true,
            _ => {}
        }
    }

    event_created_child!(State, ZwlrDataControlDeviceV1, [
        device::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, Mutex::new(Vec::<String>::new())),
    ]);
}

impl Dispatch<ZwlrDataControlOfferV1, Mutex<Vec<String>>> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlOfferV1,
        event: offer::Event,
        mimes: &Mutex<Vec<String>>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let offer::Event::Offer { mime_type } = event {
            mimes.lock().unwrap().push(mime_type);
        }
    }
}

impl Dispatch<ZwlrDataControlSourceV1, Served> for State {
    fn event(
        _: &mut Self,
        src: &ZwlrDataControlSourceV1,
        event: source::Event,
        served: &Served,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            source::Event::Send { mime_type, fd } => {
                let c = served.content.clone();
                let me = served.me.clone();
                std::thread::spawn(move || {
                    if mime_type == content::marker(&me) {
                        serve(fd, me.as_bytes());
                    } else if let Some(data) = c.get(&mime_type) {
                        serve(fd, data);
                    }
                });
            }
            source::Event::Cancelled => src.destroy(),
            _ => {}
        }
    }
}
