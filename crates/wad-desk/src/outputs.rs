//! The displays through labwc, on a Wayland connection of its own in a
//! thread of its own (as the shell's wm.rs does foreign-toplevel):
//!
//! - **output management** (`zwlr_output_manager_v1`): each display as it
//!   is ([`Said::Heads`], every time labwc says it changed), and a new
//!   configuration for them all, applied or only tested ([`Outputs::apply`]);
//! - **night light** (`zwlr_gamma_control_manager_v1`): every display's
//!   gamma ramps warmed to a colour temperature, or given back. labwc gives
//!   them back itself when this connection goes, so only the shell (which
//!   lasts the session) asks for it;
//! - **screen blank** (`zwlr_output_power_manager_v1`): every display off, or on.
//!
//! A layout's moves go in two steps. labwc fixes a display where it's put
//! only when it's put somewhere other than where it is; one it doesn't
//! (one left where it was while another moves) stays in wlroots' automatic
//! layout and is moved again by it. So a configuration that moves anything
//! goes first with every display a pixel to the right of where it's wanted,
//! then where it's wanted: each is moved both times, so each is fixed.
//!
//! Nothing here touches GTK: what it hears goes to the closure given to
//! [`start`], in its thread.

use std::collections::HashMap;
use std::io::{Seek, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

use wayland_client::backend::ObjectId;
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, event_created_child};
use wayland_protocols_wlr::gamma_control::v1::client::{
    zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1,
    zwlr_gamma_control_v1::{self as gamma, ZwlrGammaControlV1},
};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::{self as config, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self as head, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self as manager, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self as mode, ZwlrOutputModeV1},
};
use wayland_protocols_wlr::output_power_management::v1::client::{
    zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1,
    zwlr_output_power_v1::{self as power, ZwlrOutputPowerV1},
};

use crate::layout::{Head, HeadConf, Mode, Transform};

/// What the thread hears.
#[derive(Debug, Clone, PartialEq)]
pub enum Said {
    /// The displays, as they are now.
    Heads(Vec<Head>),
    /// What became of [`Outputs::apply`]'s `id`.
    Applied(u64, Result<(), String>),
    /// Night light couldn't warm a display (why).
    NightFailed(String),
    /// The connection's gone (why): nothing more comes.
    Ended(String),
}

enum Cmd {
    Apply { id: u64, confs: Vec<HeadConf>, test: bool },
    Night(Option<u32>),
    Power(bool),
}

/// The thread's handle: dropping it ends the thread.
pub struct Outputs {
    tx: mpsc::Sender<Cmd>,
    wake: Arc<OwnedFd>,
    next: AtomicU64,
}

impl Outputs {
    /// A new configuration for the displays (every one, in `confs` or kept
    /// as it is): applied, or with `test` only checked. Its answer comes as
    /// `Said::Applied(id, …)`.
    pub fn apply(&self, confs: Vec<HeadConf>, test: bool) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.send(Cmd::Apply { id, confs, test });
        id
    }

    /// Night light at `kelvin`, or off.
    pub fn night(&self, kelvin: Option<u32>) {
        self.send(Cmd::Night(kelvin));
    }

    /// Every display on, or off (blanked).
    pub fn power(&self, on: bool) {
        self.send(Cmd::Power(on));
    }

    fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
        wake(&self.wake);
    }
}

impl Drop for Outputs {
    fn drop(&mut self) {
        wake(&self.wake);
    }
}

fn wake(fd: &OwnedFd) {
    let one: u64 = 1;
    // SAFETY: an eventfd takes 8 bytes.
    unsafe { libc::write(fd.as_raw_fd(), &one as *const u64 as *const libc::c_void, 8) };
}

/// Connects (to WAYLAND_DISPLAY) and starts the thread; `on` gets what it
/// hears, there. An error when there's no compositor, or it has no output
/// management.
pub fn start(on: impl FnMut(Said) + Send + 'static) -> Result<Outputs, String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("Wayland: {e}"))?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut st = St::new(Box::new(on));
    queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    if st.manager.is_none() {
        return Err("the compositor offers no zwlr_output_manager_v1".into());
    }
    // SAFETY: a new eventfd; ours alone.
    let raw = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if raw < 0 {
        return Err(format!("eventfd: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: raw is a valid fd we own.
    let wake = Arc::new(unsafe { OwnedFd::from_raw_fd(raw) });
    let (tx, rx) = mpsc::channel();
    let w = wake.clone();
    std::thread::Builder::new()
        .name("outputs".into())
        .spawn(move || {
            let why = run(&conn, &mut queue, &mut st, &rx, &w).err().unwrap_or_else(|| "stopped".into());
            (st.on)(Said::Ended(why));
        })
        .map_err(|e| e.to_string())?;
    Ok(Outputs { tx, wake, next: AtomicU64::new(0) })
}

fn run(
    conn: &Connection,
    queue: &mut EventQueue<St>,
    st: &mut St,
    rx: &mpsc::Receiver<Cmd>,
    wake: &OwnedFd,
) -> Result<(), String> {
    let qh = queue.handle();
    loop {
        loop {
            match rx.try_recv() {
                Ok(c) => st.command(c, &qh),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
        conn.flush().map_err(|e| e.to_string())?;
        queue.dispatch_pending(st).map_err(|e| e.to_string())?;
        let Some(guard) = queue.prepare_read() else { continue };
        let mut fds = [
            libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: wake.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        ];
        // SAFETY: two valid pollfds.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("poll: {e}"));
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
        queue.dispatch_pending(st).map_err(|e| e.to_string())?;
    }
}

struct HeadObj {
    proxy: ZwlrOutputHeadV1,
    head: Head,
    modes: Vec<ObjectId>,
    current: Option<ObjectId>,
}

struct OutObj {
    global: u32,
    proxy: wl_output::WlOutput,
    name: String,
    gamma: Option<(ZwlrGammaControlV1, u32)>,
    power: Option<ZwlrOutputPowerV1>,
}

struct St {
    on: Box<dyn FnMut(Said) + Send>,
    manager: Option<ZwlrOutputManagerV1>,
    serial: u32,
    heads: Vec<HeadObj>,
    modes: HashMap<ObjectId, (ZwlrOutputModeV1, Mode)>,
    configs: HashMap<ObjectId, Step>,
    /// The second step of a move, once labwc's said the first is done.
    second: Option<(u64, Vec<HeadConf>, u32)>,
    gamma: Option<ZwlrGammaControlManagerV1>,
    power: Option<ZwlrOutputPowerManagerV1>,
    outputs: Vec<OutObj>,
    night: Option<u32>,
    off: bool,
}

impl St {
    fn new(on: Box<dyn FnMut(Said) + Send>) -> Self {
        Self {
            on,
            manager: None,
            serial: 0,
            heads: vec![],
            modes: HashMap::new(),
            configs: HashMap::new(),
            second: None,
            gamma: None,
            power: None,
            outputs: vec![],
            night: None,
            off: false,
        }
    }

    fn snapshot(&self) -> Vec<Head> {
        self.heads
            .iter()
            .map(|h| {
                let mut x = h.head.clone();
                x.modes = h.modes.iter().filter_map(|id| self.modes.get(id).map(|(_, m)| *m)).collect();
                x.mode = h.current.as_ref().and_then(|id| self.modes.get(id).map(|(_, m)| *m));
                if !x.enabled {
                    x.mode = None;
                }
                x
            })
            .collect()
    }

    fn command(&mut self, c: Cmd, qh: &QueueHandle<St>) {
        match c {
            Cmd::Apply { id, confs, test } => {
                let r = if !test && self.moves(&confs) {
                    let first = nudged(&confs);
                    let serial = self.serial;
                    self.configure(Step::First(id, confs, serial), &first, false, qh)
                } else {
                    self.configure(Step::Last(id), &confs, test, qh)
                };
                if let Err(e) = r {
                    (self.on)(Said::Applied(id, Err(e)));
                }
            }
            Cmd::Night(k) => {
                self.night = k;
                if k.is_none() {
                    for o in &mut self.outputs {
                        if let Some((g, _)) = o.gamma.take() {
                            g.destroy();
                        }
                    }
                } else if self.gamma.is_none() {
                    (self.on)(Said::NightFailed("the compositor offers no gamma control".into()));
                } else {
                    for i in 0..self.outputs.len() {
                        self.warm(i, qh);
                    }
                }
            }
            Cmd::Power(on) => {
                self.off = !on;
                for i in 0..self.outputs.len() {
                    self.set_power(i, qh);
                }
            }
        }
    }

    /// `confs` move a display (and there's more than one on).
    fn moves(&self, confs: &[HeadConf]) -> bool {
        confs.iter().filter(|c| c.enabled).count() > 1
            && confs.iter().any(|c| {
                c.enabled
                    && self
                        .heads
                        .iter()
                        .any(|h| h.head.name == c.name && (!h.head.enabled || (h.head.x, h.head.y) != (c.x, c.y)))
            })
    }

    /// The second step of a move, if the first's done and labwc's said so.
    fn go_on(&mut self, qh: &QueueHandle<St>) {
        let Some((id, confs, serial)) = self.second.take() else { return };
        if self.serial == serial {
            self.second = Some((id, confs, serial));
            return;
        }
        if let Err(e) = self.configure(Step::Last(id), &confs, false, qh) {
            (self.on)(Said::Applied(id, Err(e)));
        }
    }

    fn configure(&mut self, step: Step, confs: &[HeadConf], test: bool, qh: &QueueHandle<St>) -> Result<(), String> {
        let Some(m) = &self.manager else { return Err("no output management".into()) };
        if let Some(c) = confs.iter().find(|c| !self.heads.iter().any(|h| h.head.name == c.name)) {
            return Err(format!("there's no display {} now", c.name));
        }
        if !confs.iter().any(|c| c.enabled)
            && !self.heads.iter().any(|h| h.head.enabled && !confs.iter().any(|c| c.name == h.head.name))
        {
            return Err("at least one display has to stay on".into());
        }
        let cfg = m.create_configuration(self.serial, qh, ());
        for h in &self.heads {
            let c = confs.iter().find(|c| c.name == h.head.name).cloned().unwrap_or_else(|| {
                let mut x = h.head.clone();
                x.mode = h.current.as_ref().and_then(|id| self.modes.get(id).map(|(_, m)| *m));
                x.conf()
            });
            if !c.enabled {
                cfg.disable_head(&h.proxy);
                continue;
            }
            let ch = cfg.enable_head(&h.proxy, qh, ());
            // Always a mode: turned on without one, wlroots may have none
            // to say it's in (and asserts, ending labwc). Its own preferred
            // one, else its first.
            let fallback = || {
                let ms: Vec<Mode> = h.modes.iter().filter_map(|id| self.modes.get(id).map(|(_, m)| *m)).collect();
                ms.iter().find(|m| m.preferred).or(ms.first()).copied()
            };
            if let Some(want) = c.mode.or_else(fallback) {
                let known = h.modes.iter().find_map(|id| self.modes.get(id).filter(|(_, m)| m.same(&want)));
                match known {
                    Some((p, _)) => ch.set_mode(p),
                    None => ch.set_custom_mode(want.width, want.height, want.refresh),
                }
            }
            ch.set_position(c.x, c.y);
            if let Ok(t) = wl_output::Transform::try_from(c.transform.wl()) {
                ch.set_transform(t);
            }
            ch.set_scale(c.scale);
        }
        if test {
            cfg.test();
        } else {
            cfg.apply();
        }
        self.configs.insert(cfg.id(), step);
        Ok(())
    }

    /// Output `i`'s gamma, warmed to the night light (asked for first, if it must be).
    fn warm(&mut self, i: usize, qh: &QueueHandle<St>) {
        let (Some(k), Some(gm)) = (self.night, &self.gamma) else { return };
        let o = &mut self.outputs[i];
        match &o.gamma {
            None => {
                let g = gm.get_gamma_control(&o.proxy, qh, o.global);
                o.gamma = Some((g, 0));
            }
            Some((g, size)) if *size > 0 => {
                if let Err(e) = set_gamma(g, *size, k) {
                    (self.on)(Said::NightFailed(format!("{}: {e}", o.name)));
                }
            }
            // Its size comes first.
            Some(_) => {}
        }
    }

    fn set_power(&mut self, i: usize, qh: &QueueHandle<St>) {
        let Some(pm) = &self.power else { return };
        let o = &mut self.outputs[i];
        if o.power.is_none() {
            if !self.off {
                // Never asked to go off: nothing to put back.
                return;
            }
            o.power = Some(pm.get_output_power(&o.proxy, qh, ()));
        }
        if let Some(p) = &o.power {
            p.set_mode(if self.off { power::Mode::Off } else { power::Mode::On });
        }
    }
}

/// A memfd with `size` entries of each of red, green and blue warmed to `kelvin`.
fn set_gamma(g: &ZwlrGammaControlV1, size: u32, kelvin: u32) -> Result<(), String> {
    let ramps = ramp(size as usize, kelvin);
    // SAFETY: a new memfd, ours alone.
    let raw = unsafe { libc::memfd_create(c"wados-gamma".as_ptr(), libc::MFD_CLOEXEC) };
    if raw < 0 {
        return Err(format!("memfd: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: raw is a valid fd we own.
    let mut f = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(raw) });
    let bytes: Vec<u8> = ramps.iter().flat_map(|v| v.to_ne_bytes()).collect();
    f.write_all(&bytes).map_err(|e| e.to_string())?;
    f.rewind().map_err(|e| e.to_string())?;
    g.set_gamma(f.as_fd());
    Ok(())
}

/// What a colour temperature does to white: red, green and blue, 0 to 1
/// (Tanner Helland's fit to the blackbody), relative to 6500 K.
pub fn whitepoint(kelvin: u32) -> (f64, f64, f64) {
    fn raw(k: f64) -> (f64, f64, f64) {
        let t = k / 100.0;
        let r = if t <= 66.0 { 255.0 } else { 329.698_727_446 * (t - 60.0).powf(-0.133_204_759_2) };
        let g = if t <= 66.0 {
            99.470_802_586_1 * t.ln() - 161.119_568_166_1
        } else {
            288.122_169_528_3 * (t - 60.0).powf(-0.075_514_849_2)
        };
        let b = if t >= 66.0 {
            255.0
        } else if t <= 19.0 {
            0.0
        } else {
            138.517_731_223_1 * (t - 10.0).ln() - 305.044_792_730_7
        };
        let c = |v: f64| (v / 255.0).clamp(0.0, 1.0);
        (c(r), c(g), c(b))
    }
    let (r, g, b) = raw(kelvin.clamp(1000, 10000) as f64);
    let (wr, wg, wb) = raw(6500.0);
    ((r / wr).min(1.0), (g / wg).min(1.0), (b / wb).min(1.0))
}

/// The gamma ramps for `kelvin`: `size` reds, then greens, then blues.
pub fn ramp(size: usize, kelvin: u32) -> Vec<u16> {
    let (r, g, b) = whitepoint(kelvin);
    let one = |k: f64| {
        (0..size).map(move |i| {
            let v = if size > 1 { i as f64 / (size - 1) as f64 } else { 1.0 };
            (v * k * 65535.0).round() as u16
        })
    };
    one(r).chain(one(g)).chain(one(b)).collect()
}

/// Night light's colour temperature for the Color Temperature slider (0 cooler, 1 warmer).
pub fn kelvin_for(warmth: f64) -> u32 {
    (5500.0 - 3000.0 * warmth.clamp(0.0, 1.0)).round() as u32
}

impl Dispatch<wl_registry::WlRegistry, ()> for St {
    fn event(
        st: &mut Self,
        r: &wl_registry::WlRegistry,
        e: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match e {
            wl_registry::Event::Global { name, interface, version } => match interface.as_str() {
                "zwlr_output_manager_v1" => st.manager = Some(r.bind(name, version.min(4), qh, ())),
                "zwlr_gamma_control_manager_v1" => st.gamma = Some(r.bind(name, 1, qh, ())),
                "zwlr_output_power_manager_v1" => st.power = Some(r.bind(name, 1, qh, ())),
                "wl_output" => {
                    let proxy = r.bind(name, version.min(4), qh, name);
                    st.outputs.push(OutObj { global: name, proxy, name: String::new(), gamma: None, power: None });
                    let i = st.outputs.len() - 1;
                    st.warm(i, qh);
                    st.set_power(i, qh);
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(i) = st.outputs.iter().position(|o| o.global == name) {
                    let o = st.outputs.remove(i);
                    if let Some((g, _)) = o.gamma {
                        g.destroy();
                    }
                    if let Some(p) = o.power {
                        p.destroy();
                    }
                    if o.proxy.version() >= 3 {
                        o.proxy.release();
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for St {
    fn event(
        st: &mut Self,
        _: &wl_output::WlOutput,
        e: wl_output::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = e
            && let Some(o) = st.outputs.iter_mut().find(|o| o.global == *global)
        {
            o.name = name;
        }
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for St {
    fn event(
        st: &mut Self,
        _: &ZwlrOutputManagerV1,
        e: manager::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match e {
            manager::Event::Head { head } => st.heads.push(HeadObj {
                proxy: head,
                head: Head { scale: 1.0, ..Default::default() },
                modes: vec![],
                current: None,
            }),
            manager::Event::Done { serial } => {
                st.serial = serial;
                if st.second.is_some() {
                    // A move's first step: it's where it's wanted after the second.
                    st.go_on(qh);
                    return;
                }
                let heads = st.snapshot();
                (st.on)(Said::Heads(heads));
            }
            manager::Event::Finished => {
                st.manager = None;
            }
            _ => {}
        }
    }

    event_created_child!(St, ZwlrOutputManagerV1, [
        manager::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for St {
    fn event(st: &mut Self, p: &ZwlrOutputHeadV1, e: head::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let id = p.id();
        if let head::Event::Finished = e {
            st.heads.retain(|h| h.proxy.id() != id);
            if p.version() >= 3 {
                p.release();
            }
            return;
        }
        let modes = &mut st.modes;
        let Some(h) = st.heads.iter_mut().find(|h| h.proxy.id() == id) else { return };
        match e {
            head::Event::Name { name } => h.head.name = name,
            head::Event::Description { description } => h.head.description = description,
            head::Event::Make { make } => h.head.make = make,
            head::Event::Model { model } => h.head.model = model,
            head::Event::SerialNumber { serial_number } => h.head.serial = serial_number,
            head::Event::PhysicalSize { width, height } => h.head.physical = (width, height),
            head::Event::Enabled { enabled } => h.head.enabled = enabled != 0,
            head::Event::Mode { mode } => {
                h.modes.push(mode.id());
                modes.insert(mode.id(), (mode, Mode { width: 0, height: 0, refresh: 0, preferred: false }));
            }
            head::Event::CurrentMode { mode } => h.current = Some(mode.id()),
            head::Event::Position { x, y } => (h.head.x, h.head.y) = (x, y),
            head::Event::Transform { transform } => {
                h.head.transform = match transform {
                    WEnum::Value(t) => Transform::from_wl(t.into()),
                    WEnum::Unknown(n) => Transform::from_wl(n),
                }
            }
            head::Event::Scale { scale } => h.head.scale = scale,
            _ => {}
        }
    }

    event_created_child!(St, ZwlrOutputHeadV1, [
        head::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputModeV1, ()> for St {
    fn event(st: &mut Self, p: &ZwlrOutputModeV1, e: mode::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let id = p.id();
        if let mode::Event::Finished = e {
            st.modes.remove(&id);
            for h in &mut st.heads {
                h.modes.retain(|m| *m != id);
            }
            if p.version() >= 3 {
                p.release();
            }
            return;
        }
        let Some((_, m)) = st.modes.get_mut(&id) else { return };
        match e {
            mode::Event::Size { width, height } => (m.width, m.height) = (width, height),
            mode::Event::Refresh { refresh } => m.refresh = refresh,
            mode::Event::Preferred => m.preferred = true,
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, ()> for St {
    fn event(
        st: &mut Self,
        p: &ZwlrOutputConfigurationV1,
        e: config::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let r = match e {
            config::Event::Succeeded => Ok(()),
            config::Event::Failed => Err("the displays can't be set that way".to_string()),
            config::Event::Cancelled => Err("the displays changed meanwhile: try again".to_string()),
            _ => return,
        };
        p.destroy();
        match st.configs.remove(&p.id()) {
            Some(Step::Last(id)) => (st.on)(Said::Applied(id, r)),
            Some(Step::First(id, confs, serial)) => match r {
                // Then where they're wanted, once labwc's said where they
                // are (a serial newer than the first's: it may have already).
                Ok(()) => {
                    st.second = Some((id, confs, serial));
                    st.go_on(qh);
                }
                Err(e) => (st.on)(Said::Applied(id, Err(e))),
            },
            None => {}
        }
    }
}

/// Which step of an apply a configuration is.
enum Step {
    /// The first of a move's two (the rest's to do after it), and the
    /// serial it was made with.
    First(u64, Vec<HeadConf>, u32),
    /// The last (or only): its answer is the apply's.
    Last(u64),
}

/// `confs` a pixel to the right (a move's first step).
fn nudged(confs: &[HeadConf]) -> Vec<HeadConf> {
    confs.iter().map(|c| HeadConf { x: c.x + 1, ..c.clone() }).collect()
}

impl Dispatch<ZwlrGammaControlV1, u32> for St {
    fn event(
        st: &mut Self,
        p: &ZwlrGammaControlV1,
        e: gamma::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(o) = st.outputs.iter_mut().find(|o| o.global == *global) else { return };
        match e {
            gamma::Event::GammaSize { size } => {
                if let Some(g) = &mut o.gamma {
                    g.1 = size;
                }
                if let Some(k) = st.night
                    && let Err(e) = set_gamma(p, size, k)
                {
                    let name = o.name.clone();
                    (st.on)(Said::NightFailed(format!("{name}: {e}")));
                }
            }
            gamma::Event::Failed => {
                p.destroy();
                o.gamma = None;
                let name = if o.name.is_empty() { "a display".to_string() } else { o.name.clone() };
                (st.on)(Said::NightFailed(format!("{name} can't change its colours (no gamma ramps)")));
            }
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(St: ignore ZwlrOutputConfigurationHeadV1);
wayland_client::delegate_noop!(St: ignore ZwlrGammaControlManagerV1);
wayland_client::delegate_noop!(St: ignore ZwlrOutputPowerManagerV1);
wayland_client::delegate_noop!(St: ignore ZwlrOutputPowerV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn night_light_warms_white() {
        let (r, g, b) = whitepoint(6500);
        assert!((r - 1.0).abs() < 1e-9 && (g - 1.0).abs() < 1e-9 && (b - 1.0).abs() < 1e-9);
        let (r, g, b) = whitepoint(3000);
        assert_eq!(r, 1.0);
        assert!(g < 0.8 && g > 0.6, "{g}");
        assert!(b < g && b > 0.3, "{b}");
        // Warmer is less blue.
        assert!(whitepoint(2500).2 < whitepoint(4000).2);
        assert_eq!(kelvin_for(0.0), 5500);
        assert_eq!(kelvin_for(0.5), 4000);
        assert_eq!(kelvin_for(1.0), 2500);
        assert_eq!(kelvin_for(7.0), 2500);
    }

    #[test]
    fn its_ramps() {
        let r = ramp(4, 6500);
        assert_eq!(r.len(), 12);
        assert_eq!(&r[..4], &[0, 21845, 43690, 65535]);
        let warm = ramp(256, 2500);
        assert_eq!(warm[255], 65535, "red stays");
        assert!(warm[511] < 50000, "green's top comes down");
        assert!(warm[767] < warm[511], "blue more");
        assert_eq!(warm[256], 0);
    }
}
