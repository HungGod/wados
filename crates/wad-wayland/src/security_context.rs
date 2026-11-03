//! A Wayland socket of its own that the compositor (labwc) serves to one
//! sandboxed client (wp_security_context_v1), which then gets none of its
//! privileged protocols: no screen capture, clipboard control, window lists,
//! overlays, virtual keyboard or pointer, gamma, output or power control,
//! input method or lock screen (labwc blocks them for sandboxed clients).
//! What a desktop needs is still there.
//!
//! The caller makes the listening socket (here, so it can be labelled for
//! containers when it's made) and hands it to the compositor with a pipe's
//! read end: it accepts on it until the pipe's other end, which [`Listener`]
//! keeps, closes. Clients that already connected stay connected.

use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

use wayland_client::{Connection, Dispatch, QueueHandle, protocol::wl_registry};
use wayland_protocols::wp::security_context::v1::client::{
    wp_security_context_manager_v1::WpSecurityContextManagerV1, wp_security_context_v1::WpSecurityContextV1,
};

/// Keeps the compositor accepting on a socket: dropping it hangs up, and it
/// stops (the socket's file stays, for the caller to remove).
#[derive(Debug)]
pub struct Listener {
    _close: OwnedFd,
}

/// Who the sandboxed client is, as labwc's window rules match it
/// (`sandboxEngine`, `sandboxAppId`; and the instance id).
#[derive(Debug, Clone)]
pub struct Sandbox<'a> {
    pub engine: &'a str,
    pub app_id: &'a str,
    pub instance_id: &'a str,
}

/// SELinux labels for the socket: the listening socket's own (a process
/// context, as `setsockcreatecon` takes) and its file's.
#[derive(Debug, Clone, Default)]
pub struct Labels<'a> {
    pub socket: Option<&'a str>,
    pub file: Option<&'a str>,
}

#[derive(Default)]
struct Globals {
    manager: Option<WpSecurityContextManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Globals {
    fn event(
        g: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, .. } = event
            && interface == "wp_security_context_manager_v1"
        {
            g.manager = Some(registry.bind(name, 1, qh, ()));
        }
    }
}

wayland_client::delegate_noop!(Globals: ignore WpSecurityContextManagerV1);
wayland_client::delegate_noop!(Globals: ignore WpSecurityContextV1);

/// Makes a socket at `path` (replacing what's there) that the compositor,
/// reached at `compositor` (its own Wayland socket; root may connect to
/// anyone's), serves to `sandbox` alone. `owner`: whose the socket's file is
/// (a client needs write access to connect). Blocking: a round trip to the
/// compositor.
pub fn listen(
    compositor: &Path,
    path: &Path,
    sandbox: &Sandbox,
    owner: Option<u32>,
    labels: &Labels,
) -> Result<Listener, String> {
    let stream = UnixStream::connect(compositor).map_err(|e| format!("{}: {e}", compositor.display()))?;
    let conn = Connection::from_socket(stream).map_err(|e| format!("the compositor: {e}"))?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut globals = Globals::default();
    queue.roundtrip(&mut globals).map_err(|e| format!("the compositor: {e}"))?;
    let manager = globals
        .manager
        .take()
        .ok_or("the compositor doesn't offer wp_security_context_manager_v1 (labwc 0.8 or newer has it)")?;

    let listener = bind(path, labels)?;
    if let Some(uid) = owner {
        std::os::unix::fs::lchown(path, Some(uid), Some(uid)).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let mut fds = [0; 2];
    // SAFETY: pipe2 fills two fds, which are then owned here.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(format!("pipe: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: both fresh from pipe2 above.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let context = manager.create_listener(listener.as_fd(), read.as_fd(), &qh, ());
    context.set_sandbox_engine(sandbox.engine.into());
    context.set_app_id(sandbox.app_id.into());
    context.set_instance_id(sandbox.instance_id.into());
    context.commit();
    context.destroy();
    manager.destroy();
    // A refused one is a protocol error, which ends the connection here.
    queue.roundtrip(&mut globals).map_err(|e| format!("the compositor refused the socket: {e}"))?;
    Ok(Listener { _close: write })
}

/// The listening socket, labelled (when SELinux is on) as it's made.
fn bind(path: &Path, labels: &Labels) -> Result<UnixListener, String> {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", path.display())),
    }
    let selinux = Path::new("/sys/fs/selinux/enforce").exists();
    let socket_label = labels.socket.filter(|_| selinux);
    if let Some(l) = socket_label {
        set_sockcreate(l)?;
    }
    let made = UnixListener::bind(path).map_err(|e| format!("{}: {e}", path.display()));
    if socket_label.is_some() {
        set_sockcreate("").map_err(|e| format!("resetting the socket label: {e}"))?;
    }
    let listener = made?;
    if let Some(l) = labels.file.filter(|_| selinux) {
        label_file(path, l)?;
    }
    Ok(listener)
}

/// The label of the sockets this thread makes from now on ("": the usual
/// one again). One write(2), even of nothing: that's what clears it (as
/// libselinux's setsockcreatecon(NULL) does), and write_all would skip it.
fn set_sockcreate(label: &str) -> Result<(), String> {
    let wrote = std::fs::OpenOptions::new()
        .write(true)
        .open("/proc/thread-self/attr/sockcreate")
        .and_then(|mut f| f.write(label.as_bytes()));
    match wrote {
        Ok(n) if n == label.len() => Ok(()),
        Ok(_) => Err(format!("labelling the socket {label}: a short write")),
        Err(e) => Err(format!("labelling the socket {label}: {e}")),
    }
}

/// A file's SELinux label (not following a symlink).
pub fn label_file(path: &Path, label: &str) -> Result<(), String> {
    let p = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
    // SAFETY: lsetxattr with valid C strings and a buffer of the given length.
    let r = unsafe { libc::lsetxattr(p.as_ptr(), c"security.selinux".as_ptr(), label.as_ptr().cast(), label.len(), 0) };
    if r < 0 {
        return Err(format!("labelling {} {label}: {}", path.display(), std::io::Error::last_os_error()));
    }
    Ok(())
}
