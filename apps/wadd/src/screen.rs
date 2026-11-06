//! How a wadspace reaches the screen: only through a folder of its own,
//! `<screen_dir>/<id>`, mounted read-only at /run/wadspaces-display (never
//! the session's runtime dir). In it (v1's screen_access.rs, on labwc):
//!
//! - `wayland-1`: a socket the compositor serves it alone
//!   (`wad_wayland::security_context::listen`, app id `io.wadspaces.<id>`:
//!   how the shell tells its window), so its desktop gets none of the
//!   compositor's privileged protocols. Labelled for containers, so the
//!   wadspace runs with SELinux on. Made again when the compositor restarts
//!   (its socket's inode changes): its desktop sees its display go and come
//!   back. A desktop already connected stays connected.
//! - `pulse/`: PulseAudio's folder (the session's PipeWire). A machine's
//!   wadd binds it in read-only in PID 1's mounts (the wadspace's follow:
//!   rslave); a laptop's can't, so the unit mounts it from the start.
//!
//! Its clipboard: the bridge (clip.rs) between its labwc's and the
//! machine's, once its desktop is up; started again when it ends.
//!
//! **Finding the compositor.** The shell tells wadd when it starts
//! (`PUT /v1/screen/session`: its Wayland socket and whose session it is).
//! Until it has, or once that socket's gone: `[display] socket`, else the
//! newest `wayland-*` socket in `[display] runtime_dir`.

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use wad_gen::units::{PULSE, SANDBOX_ENGINE, SOCKET, container_name, sandbox_app_id};
use wad_proto::{ApiError, Event, ScreenFrom, ScreenInfo};

use crate::backend::{Backend, Res};
use crate::events::Bus;

/// The compositor wadd puts wadspaces on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compositor {
    pub socket: PathBuf,
    /// Which one it is: a restarted compositor's socket is new, at the same path.
    pub inode: u64,
    /// Whose session it is: the owner of the sockets made for wadspaces.
    pub uid: u32,
    pub from: ScreenFrom,
}

/// Keeps a socket served.
pub type Guard = Box<dyn std::any::Any + Send>;

/// A clipboard bridge.
pub trait Running: Send {
    /// Still going (it ends when its labwc does).
    fn alive(&mut self) -> bool;
}

impl Running for crate::clip::Bridge {
    fn alive(&mut self) -> bool {
        crate::clip::Bridge::alive(self)
    }
}

/// What screen access does on the machine (a fake in tests).
pub trait Hands: Send + Sync {
    /// A wadspace's folder, with PulseAudio's mountpoint in it, labelled for containers.
    fn prepare(&self, dir: &Path) -> Res<()>;
    /// A socket at `socket` the compositor serves `app_id` alone, until the guard drops.
    fn listen(&self, comp: &Compositor, socket: &Path, app_id: &str, instance: &str) -> Res<Guard>;
    fn remove(&self, socket: &Path);
    /// PulseAudio's folder bound read-only at `dst` (PID 1's mounts).
    fn bind_pulse(&self, src: &Path, dst: &Path) -> Res<()>;
    fn unbind_pulse(&self, dst: &Path);
    /// The clipboard bridge for `id`, whose container's first process is `pid`.
    fn bridge(&self, comp: &Compositor, id: &str, pid: u32) -> Res<Box<dyn Running>>;
}

pub struct Settings {
    pub enabled: bool,
    /// The screen folders' folder.
    pub root: PathBuf,
    pub runtime_dir: PathBuf,
    pub socket: Option<PathBuf>,
    /// wadd binds PulseAudio's folder in (a machine's); else the unit mounts it.
    pub bind_pulse: bool,
}

struct Served {
    inode: u64,
    _guard: Guard,
}

#[derive(Default)]
struct State {
    served: HashMap<String, Served>,
    /// Wadspaces whose PulseAudio folder is bound in: which one.
    pulse: HashMap<String, u64>,
    /// What gets no clipboard bridge.
    sealed: HashSet<String>,
    bridges: HashMap<String, Box<dyn Running>>,
    /// The last error each had, said once.
    said: HashMap<String, String>,
    made: u64,
}

pub struct Screen {
    hands: Arc<dyn Hands>,
    backend: Arc<dyn Backend>,
    bus: Bus,
    settings: Settings,
    registered: Mutex<Option<(PathBuf, u32)>>,
    state: Mutex<State>,
    /// One socket made at a time (the loop and a start may both want one).
    serving: Mutex<()>,
    poke: tokio::sync::Notify,
}

/// A socket's inode and owner, if `p` is a socket.
fn socket_at(p: &Path) -> Option<(u64, u32)> {
    let m = std::fs::metadata(p).ok()?;
    m.file_type().is_socket().then(|| (m.ino(), m.uid()))
}

/// The newest `wayland-*` socket in `dir`.
pub fn newest_wayland(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("wayland-") && !n.ends_with(".lock") && e.file_type().is_ok_and(|t| t.is_socket())
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max()
        .map(|(_, p)| p)
}

impl Screen {
    pub fn new(hands: Arc<dyn Hands>, backend: Arc<dyn Backend>, bus: Bus, settings: Settings) -> Arc<Self> {
        Arc::new(Self {
            hands,
            backend,
            bus,
            settings,
            registered: Mutex::default(),
            state: Mutex::default(),
            serving: Mutex::default(),
            poke: tokio::sync::Notify::new(),
        })
    }

    /// A wadspace's folder.
    pub fn dir(&self, id: &str) -> PathBuf {
        self.settings.root.join(id)
    }

    pub fn root(&self) -> &Path {
        &self.settings.root
    }

    pub fn enabled(&self) -> bool {
        self.settings.enabled
    }

    /// The compositor, wherever it's found now.
    pub fn compositor(&self) -> Option<Compositor> {
        if !self.settings.enabled {
            return None;
        }
        let registered = self.registered.lock().unwrap().clone();
        if let Some((p, uid)) = registered
            && let Some((inode, _)) = socket_at(&p)
        {
            return Some(Compositor { socket: p, inode, uid, from: ScreenFrom::Shell });
        }
        if let Some(p) = &self.settings.socket {
            return socket_at(p).map(|(inode, uid)| Compositor {
                socket: p.clone(),
                inode,
                uid,
                from: ScreenFrom::Config,
            });
        }
        let p = newest_wayland(&self.settings.runtime_dir)?;
        socket_at(&p).map(|(inode, uid)| Compositor { socket: p, inode, uid, from: ScreenFrom::RuntimeDir })
    }

    /// `PUT /v1/screen/session`: the shell's compositor. Its socket must be
    /// there, a socket, and `uid`'s; and `uid` the caller's (root may say anyone's).
    pub fn register(&self, socket: &str, uid: u32, caller: u32) -> Result<ScreenInfo, ApiError> {
        if !self.settings.enabled {
            return Err(ApiError::conflict("this wadd has no screen ([display] enabled = false)"));
        }
        let p = PathBuf::from(socket);
        if !p.is_absolute() || socket.chars().any(char::is_control) {
            return Err(ApiError::bad_request(format!("{socket:?}: the compositor's socket, as an absolute path")));
        }
        if caller != 0 && caller != uid {
            return Err(ApiError::new(wad_proto::ErrorCode::Forbidden, "only your own session"));
        }
        let (_, owner) = socket_at(&p).ok_or_else(|| ApiError::bad_request(format!("{socket} isn't a socket")))?;
        if owner != uid {
            return Err(ApiError::bad_request(format!("{socket} isn't uid {uid}'s")));
        }
        *self.registered.lock().unwrap() = Some((p, uid));
        tracing::info!("the screen: {socket} (uid {uid}), as the shell says");
        self.poke.notify_one();
        Ok(self.info())
    }

    /// Where the screen is, and what's on it.
    pub fn info(&self) -> ScreenInfo {
        let comp = self.compositor();
        let mut served: Vec<String> = self.state.lock().unwrap().served.keys().cloned().collect();
        served.sort();
        ScreenInfo {
            socket: comp.as_ref().map(|c| c.socket.to_string_lossy().into_owned()),
            uid: comp.as_ref().map(|c| c.uid),
            from: comp.as_ref().map_or(ScreenFrom::None, |c| c.from),
            served,
        }
    }

    fn publish(&self) {
        let info = self.info();
        if self.bus.latest("screen") != Some(Event::Screen(info.clone())) {
            self.bus.publish(Event::Screen(info));
        }
    }

    /// Something changed (a wadspace starting or stopping, the shell's session): the loop looks now.
    pub fn poke(&self) {
        self.poke.notify_one();
    }

    pub async fn poked(&self) {
        self.poke.notified().await;
    }

    /// PulseAudio's folder for a wadspace's unit to mount (a laptop's: wadd can't bind it in).
    pub fn pulse_dir(&self) -> Option<PathBuf> {
        if self.settings.bind_pulse {
            return None;
        }
        self.pulse_source().map(|(p, _)| p)
    }

    /// The session's PulseAudio folder: beside the compositor's socket, else in the runtime dir.
    fn pulse_source(&self) -> Option<(PathBuf, u64)> {
        let beside = self.compositor().and_then(|c| c.socket.parent().map(|d| d.join(PULSE)));
        [beside, Some(self.settings.runtime_dir.join(PULSE))]
            .into_iter()
            .flatten()
            .find_map(|p| std::fs::metadata(&p).ok().filter(|m| m.is_dir()).map(|m| (p, m.ino())))
    }

    /// Before a wadspace starts: its folder (its unit mounts it, so it must
    /// be there) and, with a compositor, its socket and sound. Blocking.
    pub fn prepare(&self, id: &str) -> Res<()> {
        self.hands.prepare(&self.dir(id)).map_err(|e| format!("its screen folder: {e}"))?;
        if let Some(comp) = self.compositor() {
            self.serve(id, &comp);
        }
        Ok(())
    }

    /// Serves what's on the screen (`want`: id, and whether its desktop is
    /// up) and takes away what isn't. Its bridges too.
    pub async fn tick(self: &Arc<Self>, want: Vec<(String, bool)>) {
        let me = self.clone();
        let w = want.clone();
        let _ = tokio::task::spawn_blocking(move || me.reconcile(&w)).await;
        self.bridges(&want).await;
        self.publish();
    }

    /// Blocking: a round trip to the compositor per socket made.
    fn reconcile(&self, want: &[(String, bool)]) {
        // One that runs already (taken on after wadd restarted) gets its
        // folder here; one starting gets it from its start.
        for (id, up) in want {
            if *up
                && !self.dir(id).is_dir()
                && let Err(e) = self.hands.prepare(&self.dir(id))
            {
                self.say(id, format!("{id}: its screen folder: {e}"));
            }
        }
        let want: Vec<String> = want.iter().map(|(id, _)| id.clone()).collect();
        let want = want.as_slice();
        let comp = self.compositor();
        let had: Vec<String> = {
            let s = self.state.lock().unwrap();
            s.served.keys().chain(s.pulse.keys()).cloned().collect::<HashSet<_>>().into_iter().collect()
        };
        for id in had.iter().filter(|id| !want.contains(id)) {
            self.take(id);
        }
        match comp {
            Some(c) => {
                for id in want {
                    self.serve(id, &c);
                }
            }
            // The compositor's gone, and the sockets it served with it.
            None => {
                for id in want {
                    self.take(id);
                }
            }
        }
    }

    /// Its socket (made again for a new compositor) and sound.
    fn serve(&self, id: &str, comp: &Compositor) {
        let _one = self.serving.lock().unwrap();
        let dir = self.dir(id);
        // Not prepared yet (it's just starting): its start serves it.
        if !dir.is_dir() {
            return;
        }
        let fresh = self.state.lock().unwrap().served.get(id).is_some_and(|s| s.inode == comp.inode);
        if !fresh {
            let instance = {
                let mut s = self.state.lock().unwrap();
                if s.served.remove(id).is_some() {
                    tracing::info!("{id}: the compositor restarted; its socket is made again");
                }
                s.made += 1;
                format!("{id}-{}", s.made)
            };
            let socket = dir.join(SOCKET);
            match self.hands.listen(comp, &socket, &sandbox_app_id(id), &instance) {
                Ok(guard) => {
                    tracing::info!("{id}: on the screen through its own socket ({})", socket.display());
                    let mut s = self.state.lock().unwrap();
                    s.served.insert(id.into(), Served { inode: comp.inode, _guard: guard });
                    s.said.remove(id);
                }
                Err(e) => self.say(id, format!("{id}: no socket on the screen: {e}")),
            }
        }
        if !self.settings.bind_pulse {
            return;
        }
        let Some((src, src_id)) = self.pulse_source() else { return };
        if self.state.lock().unwrap().pulse.get(id) == Some(&src_id) {
            return;
        }
        let dst = dir.join(PULSE);
        if self.state.lock().unwrap().pulse.remove(id).is_some() {
            self.hands.unbind_pulse(&dst);
        }
        match self.hands.bind_pulse(&src, &dst) {
            Ok(()) => {
                self.state.lock().unwrap().pulse.insert(id.into(), src_id);
            }
            Err(e) => self.say(&format!("{id}/pulse"), format!("{id}: no sound on the screen: {e}")),
        }
    }

    /// What gets no clipboard bridge (sandboxes).
    pub fn seal(&self, ids: HashSet<String>) {
        self.state.lock().unwrap().sealed = ids;
    }

    /// Clipboard bridges for what's served with its desktop up; ended ones
    /// started again once it is.
    async fn bridges(&self, want: &[(String, bool)]) {
        let start: Vec<String> = {
            let mut s = self.state.lock().unwrap();
            let served: HashSet<String> = s.served.keys().cloned().collect();
            let sealed = &s.sealed;
            let up: HashSet<&String> =
                want.iter().filter(|(id, up)| *up && !sealed.contains(id)).map(|(id, _)| id).collect();
            s.bridges.retain(|id, b| served.contains(id) && up.contains(id) && b.alive());
            up.into_iter().filter(|id| served.contains(*id) && !s.bridges.contains_key(*id)).cloned().collect()
        };
        if start.is_empty() {
            return;
        }
        let Some(comp) = self.compositor() else { return };
        for id in start {
            let Ok(Some(pid)) = self.backend.container_pid(&container_name(&id)).await else { continue };
            let (hands, c, i) = (self.hands.clone(), comp.clone(), id.clone());
            match tokio::task::spawn_blocking(move || hands.bridge(&c, &i, pid)).await {
                Ok(Ok(b)) => {
                    tracing::info!("{id}: its clipboard is the machine's");
                    let mut s = self.state.lock().unwrap();
                    s.bridges.insert(id.clone(), b);
                    s.said.remove(&format!("{id}/clipboard"));
                }
                Ok(Err(e)) => {
                    self.say(&format!("{id}/clipboard"), format!("{id}: its clipboard isn't the machine's: {e}"))
                }
                Err(_) => {}
            }
        }
    }

    /// Its socket, sound and clipboard taken away (its folder stays: its unit mounts it).
    pub fn take(&self, id: &str) {
        let _one = self.serving.lock().unwrap();
        let dir = self.dir(id);
        let (served, bound, bridge) = {
            let mut s = self.state.lock().unwrap();
            (s.served.remove(id), s.pulse.remove(id), s.bridges.remove(id))
        };
        drop(bridge);
        if served.is_some() {
            drop(served);
            self.hands.remove(&dir.join(SOCKET));
            tracing::info!("{id}: its screen socket closed");
        }
        if bound.is_some() {
            self.hands.unbind_pulse(&dir.join(PULSE));
        }
    }

    /// A wadspace gone for good: its folder too.
    pub fn forget(&self, id: &str) {
        self.take(id);
        if crate::store::safe_id(id) {
            let _ = std::fs::remove_dir(self.dir(id).join(PULSE));
            let _ = std::fs::remove_dir(self.dir(id));
        }
    }

    pub fn served(&self, id: &str) -> bool {
        self.state.lock().unwrap().served.contains_key(id)
    }

    pub fn bridged(&self, id: &str) -> bool {
        self.state.lock().unwrap().bridges.contains_key(id)
    }

    /// Says `text` once (until something else happens to `key`).
    fn say(&self, key: &str, text: String) {
        let mut s = self.state.lock().unwrap();
        if s.said.get(key) != Some(&text) {
            tracing::warn!("{text}");
            s.said.insert(key.into(), text);
        }
    }
}

/// The machine's hands: sockets made here, PulseAudio bound in PID 1's mounts.
pub struct HostHands;

impl Hands for HostHands {
    fn prepare(&self, dir: &Path) -> Res<()> {
        use std::os::unix::fs::PermissionsExt;
        for d in [dir.to_path_buf(), dir.join(PULSE)] {
            if d.exists() {
                continue;
            }
            std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
            std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("{}: {e}", d.display()))?;
            crate::selinux::label(&d, crate::selinux::CONTAINER_FILE)?;
        }
        Ok(())
    }

    fn listen(&self, comp: &Compositor, socket: &Path, app_id: &str, instance: &str) -> Res<Guard> {
        use wad_wayland::security_context::{Labels, Sandbox, listen};
        let sandbox = Sandbox { engine: SANDBOX_ENGINE, app_id, instance_id: instance };
        let labels =
            Labels { socket: Some(crate::selinux::CONTAINER_SOCKET), file: Some(crate::selinux::CONTAINER_FILE) };
        // Its owner's: who may connect (wadd's own, unless it's root).
        let owner = (comp.uid != nix::unistd::geteuid().as_raw()).then_some(comp.uid);
        Ok(Box::new(listen(&comp.socket, socket, &sandbox, owner, &labels)?))
    }

    fn remove(&self, socket: &Path) {
        let _ = std::fs::remove_file(socket);
    }

    fn bind_pulse(&self, src: &Path, dst: &Path) -> Res<()> {
        let (src, dst) = (src.to_path_buf(), dst.to_path_buf());
        crate::mounts::inside(1, move || {
            // Its socket labelled as the host's SELinux module lets
            // containers use it (the module's type transition does that for
            // a socket made after it's loaded).
            if crate::selinux::pulse_allowed() {
                let _ = crate::selinux::label(&src.join("native"), crate::selinux::PULSE_SOCKET);
            }
            crate::mounts::bind_read_only(&src, &dst)
        })
    }

    fn unbind_pulse(&self, dst: &Path) {
        let dst = dst.to_path_buf();
        let _ = crate::mounts::inside(1, move || crate::mounts::unmount(&dst));
    }

    fn bridge(&self, comp: &Compositor, id: &str, pid: u32) -> Res<Box<dyn Running>> {
        let program = crate::clip::program().ok_or("no clipboard bridge (wadspaces-clipboard) on this machine")?;
        let host = std::os::unix::net::UnixStream::connect(&comp.socket)
            .map_err(|e| format!("the compositor ({}): {e}", comp.socket.display()))?;
        let here = crate::clip::connect_inside(pid)?;
        Ok(Box::new(crate::clip::spawn(&program, id, here, host)?))
    }
}
