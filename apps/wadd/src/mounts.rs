//! A read-only bind made with the new mount API (open_tree, mount_setattr,
//! move_mount), whose copies (the mounts that follow it: a wadspace's
//! `rslave` screen folder) are read-only too, and in another process's
//! mount namespace ([`inside`]: PID 1's, where podman's containers get
//! theirs). A machine's wadd binds PulseAudio's folder into each screen
//! folder this way (v1's mounts.rs).

use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;

#[repr(C)]
struct MountAttr {
    attr_set: u64,
    attr_clr: u64,
    propagation: u64,
    userns_fd: u64,
}

const OPEN_TREE_CLONE: libc::c_uint = 1;
const MOVE_MOUNT_F_EMPTY_PATH: libc::c_uint = 0x4;
const MOUNT_ATTR_RDONLY: u64 = 0x1;

fn c(p: &Path) -> Result<CString, String> {
    CString::new(p.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())
}

fn err(what: &str, p: &Path) -> String {
    format!("{what} {}: {}", p.display(), std::io::Error::last_os_error())
}

/// `src` bound read-only at `dst`, in this thread's mount namespace.
pub fn bind_read_only(src: &Path, dst: &Path) -> Result<(), String> {
    let p = c(src)?;
    // SAFETY: open_tree(2) with a valid path.
    let r = unsafe {
        libc::syscall(
            libc::SYS_open_tree,
            libc::AT_FDCWD,
            p.as_ptr(),
            OPEN_TREE_CLONE | libc::O_CLOEXEC as libc::c_uint,
        )
    };
    if r < 0 {
        return Err(err("cloning", src));
    }
    // SAFETY: a fresh fd from the syscall just made.
    let tree = unsafe { OwnedFd::from_raw_fd(r as i32) };
    let attr = MountAttr { attr_set: MOUNT_ATTR_RDONLY, attr_clr: 0, propagation: 0, userns_fd: 0 };
    // SAFETY: mount_setattr(2) on the detached tree, with a mount_attr of its size.
    let r = unsafe {
        libc::syscall(
            libc::SYS_mount_setattr,
            tree.as_raw_fd(),
            c"".as_ptr(),
            libc::AT_EMPTY_PATH,
            &attr as *const MountAttr,
            std::mem::size_of::<MountAttr>(),
        )
    };
    if r < 0 {
        return Err(err("making read-only", src));
    }
    let d = c(dst)?;
    // SAFETY: move_mount(2) of a detached tree onto a valid path.
    let r = unsafe {
        libc::syscall(
            libc::SYS_move_mount,
            tree.as_raw_fd(),
            c"".as_ptr(),
            libc::AT_FDCWD,
            d.as_ptr(),
            MOVE_MOUNT_F_EMPTY_PATH,
        )
    };
    if r < 0 {
        return Err(err("mounting at", dst));
    }
    Ok(())
}

pub fn unmount(dst: &Path) -> Result<(), String> {
    let d = c(dst)?;
    // SAFETY: umount2(2) with a valid C string.
    if unsafe { libc::umount2(d.as_ptr(), libc::MNT_DETACH | libc::UMOUNT_NOFOLLOW) } < 0 {
        let e = std::io::Error::last_os_error();
        if !matches!(e.raw_os_error(), Some(libc::EINVAL) | Some(libc::ENOENT)) {
            return Err(format!("{}: {e}", dst.display()));
        }
    }
    Ok(())
}

/// Runs `f` in `pid`'s mount namespace (a thread of its own, which leaves
/// wadd's other threads where they are).
pub fn inside<T: Send + 'static>(
    pid: u32,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let ns = std::fs::File::open(format!("/proc/{pid}/ns/mnt")).map_err(|e| format!("its namespace: {e}"))?;
    std::thread::spawn(move || {
        // SAFETY: unshare/setns affect this short-lived thread only (it
        // stops sharing its root and cwd with wadd's other threads first,
        // which setns into a mount namespace needs).
        unsafe {
            if libc::unshare(libc::CLONE_FS) < 0 {
                return Err(format!("unshare: {}", std::io::Error::last_os_error()));
            }
            if libc::setns(ns.as_raw_fd(), libc::CLONE_NEWNS) < 0 {
                return Err(format!("setns: {}", std::io::Error::last_os_error()));
            }
        }
        f()
    })
    .join()
    .map_err(|_| "the namespace thread panicked".to_string())?
}
