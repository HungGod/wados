//! The little of systemd's protocols wadd needs, without libsystemd: a socket
//! it was handed (socket activation, LISTEN_FDS) and telling it we're ready
//! (sd_notify).

use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::net::{UnixDatagram, UnixListener};

const SD_LISTEN_FDS_START: RawFd = 3;

/// The first socket systemd passed us, if it did. Clears the variables so
/// children don't think they were passed it too.
pub fn listener_from_systemd() -> Option<UnixListener> {
    let pid_ok = std::env::var("LISTEN_PID").ok().and_then(|p| p.parse::<u32>().ok()) == Some(std::process::id());
    let n = std::env::var("LISTEN_FDS").ok().and_then(|n| n.parse::<i32>().ok()).unwrap_or(0);
    // SAFETY: only touched at startup, before any threads read the environment.
    unsafe {
        std::env::remove_var("LISTEN_PID");
        std::env::remove_var("LISTEN_FDS");
        std::env::remove_var("LISTEN_FDNAMES");
    }
    if !pid_ok || n < 1 {
        return None;
    }
    // SAFETY: systemd hands over fd 3 for our process (LISTEN_PID matched).
    Some(unsafe { UnixListener::from_raw_fd(SD_LISTEN_FDS_START) })
}

/// Tells systemd something (e.g. "READY=1"); a no-op outside a Type=notify unit.
pub fn notify(state: &str) {
    let Some(path) = std::env::var_os("NOTIFY_SOCKET") else { return };
    let Ok(sock) = UnixDatagram::unbound() else { return };
    let bytes = path.as_encoded_bytes();
    let sent = if let Some(name) = bytes.strip_prefix(b"@") {
        use std::os::linux::net::SocketAddrExt;
        std::os::unix::net::SocketAddr::from_abstract_name(name).and_then(|a| sock.send_to_addr(state.as_bytes(), &a))
    } else {
        sock.send_to(state.as_bytes(), std::path::Path::new(&path))
    };
    if let Err(e) = sent {
        tracing::debug!("sd_notify {state}: {e}");
    }
}
