//! A wadspace's clipboard on the screen: the compositor serves its socket
//! no clipboard control (docs/decisions/0001-labwc.md), so the bridge
//! between its labwc's clipboard and the machine's (`wadspaces-clipboard`,
//! crates/wad-clip) runs here, on the machine, with a connection to each
//! that wadd makes it: to the compositor, and to the labwc inside the
//! wadspace. On a machine (wadd as root) it runs as a user of its own.
//!
//! The labwc inside is reached through `/proc/<pid>/root` with every path
//! resolved inside the wadspace's own root (openat2's RESOLVE_IN_ROOT, no
//! links followed at the end): nothing it planted in its folders leads
//! anywhere on the machine. It ends with its labwc, and is started again
//! with the next.

use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Who a machine's clipboard bridges run as (host/usr/lib/sysusers.d).
pub const BRIDGE_USER: &str = "wadspaces-clip";
/// Where a wadspace's labwc makes its sockets (its XDG_RUNTIME_DIR), from its root.
const XDG: &str = "config/.XDG";

/// The bridge: a machine's, else one built beside wadd (a laptop's checkout).
pub fn program() -> Option<PathBuf> {
    let machine = PathBuf::from("/usr/libexec/wados/wadspaces-clipboard");
    if machine.exists() {
        return Some(machine);
    }
    let here = std::env::current_exe().ok()?.parent()?.join("wadspaces-clipboard");
    here.exists().then_some(here)
}

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
const RESOLVE_IN_ROOT: u64 = 0x10;

fn cstr(s: &str) -> Result<CString, String> {
    CString::new(s).map_err(|e| e.to_string())
}

fn owned(r: libc::c_long, what: &str) -> Result<OwnedFd, String> {
    if r < 0 {
        return Err(format!("{what}: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: a fresh fd from the syscall just made.
    Ok(unsafe { OwnedFd::from_raw_fd(r as i32) })
}

/// Connects to the newest `wayland-<n>` socket in the wadspace's runtime
/// dir: its labwc's. `pid`: the container's first process, on the machine.
pub fn connect_inside(pid: u32) -> Result<UnixStream, String> {
    let root_path = cstr(&format!("/proc/{pid}/root"))?;
    // SAFETY: open(2) with a valid C string.
    let root = owned(
        unsafe { libc::open(root_path.as_ptr(), libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC) } as libc::c_long,
        "its root",
    )?;
    let how = OpenHow {
        flags: (libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) as u64,
        mode: 0,
        resolve: RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS,
    };
    let xdg_name = cstr(XDG)?;
    // SAFETY: openat2(2) with a valid dir fd, path and open_how of its size.
    let xdg = owned(
        unsafe {
            libc::syscall(
                libc::SYS_openat2,
                root.as_raw_fd(),
                xdg_name.as_ptr(),
                &how as *const OpenHow,
                std::mem::size_of::<OpenHow>(),
            )
        },
        "its runtime dir",
    )?;
    let listing = Path::new("/proc/self/fd").join(xdg.as_raw_fd().to_string());
    let mut newest: Option<(i64, String)> = None;
    for e in std::fs::read_dir(&listing).map_err(|e| format!("its runtime dir: {e}"))?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let numbered =
            name.strip_prefix("wayland-").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if !numbered {
            continue;
        }
        let Ok(st) = nix::sys::stat::fstatat(&xdg, name.as_str(), nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW) else {
            continue;
        };
        let socket = st.st_mode & libc::S_IFMT == libc::S_IFSOCK;
        if socket && newest.as_ref().is_none_or(|(t, _)| st.st_mtime > *t) {
            newest = Some((st.st_mtime, name));
        }
    }
    let (_, name) = newest.ok_or("its labwc has no socket yet")?;
    let n = cstr(&name)?;
    // SAFETY: openat(2) of one name in a dir fd, not following a link.
    let sock = owned(
        unsafe { libc::openat(xdg.as_raw_fd(), n.as_ptr(), libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC) }
            as libc::c_long,
        "its labwc's socket",
    )?;
    let st = nix::sys::stat::fstat(&sock).map_err(|e| e.to_string())?;
    if st.st_mode & libc::S_IFMT != libc::S_IFSOCK {
        return Err(format!("{name} isn't a socket"));
    }
    // The socket itself, as opened: its fd's link resolves to it alone.
    UnixStream::connect(format!("/proc/self/fd/{}", sock.as_raw_fd())).map_err(|e| format!("its labwc ({name}): {e}"))
}

/// A clipboard bridge running.
pub struct Bridge(Child);

impl Bridge {
    /// Still going (it ends when either side does).
    pub fn alive(&mut self) -> bool {
        matches!(self.0.try_wait(), Ok(None))
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Runs the bridge with its two connections as fds 3 (the wadspace's labwc) and 4 (the compositor).
pub fn spawn(program: &Path, id: &str, here: UnixStream, host: UnixStream) -> Result<Bridge, String> {
    // Never as root: a machine's wadd runs it as a user of its own.
    let user = if nix::unistd::geteuid().is_root() {
        let u = nix::unistd::User::from_name(BRIDGE_USER)
            .ok()
            .flatten()
            .ok_or_else(|| format!("no {BRIDGE_USER} user on this machine"))?;
        Some((u.uid.as_raw(), u.gid.as_raw()))
    } else {
        None
    };
    let (here, host): (OwnedFd, OwnedFd) = (here.into(), host.into());
    let (h, m) = (here.as_raw_fd(), host.as_raw_fd());
    let mut cmd = Command::new(program);
    cmd.args(["--here-fd", "3", "--host-fd", "4", "--name", &format!("ws-{id}")])
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    if let Some((uid, gid)) = user {
        use std::os::unix::process::CommandExt;
        cmd.uid(uid).gid(gid);
    }
    // SAFETY: only async-signal-safe calls (fcntl, dup2) between fork and
    // exec. Copied above 10 first, so 3 and 4 can't overwrite each other;
    // the copies close at exec, 3 and 4 stay open.
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(move || {
            let a = libc::fcntl(h, libc::F_DUPFD_CLOEXEC, 10);
            let b = libc::fcntl(m, libc::F_DUPFD_CLOEXEC, 10);
            if a < 0 || b < 0 || libc::dup2(a, 3) < 0 || libc::dup2(b, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd.spawn().map_err(|e| format!("{}: {e}", program.display()))?;
    drop((here, host));
    Ok(Bridge(child))
}
