//! SELinux, for what wadd makes for wadspaces (v1's): labelling a file, and
//! asking the loaded policy what it allows (security_compute_av, as
//! libselinux does it: /sys/fs/selinux/access).

use std::io::{Read, Seek, Write};
use std::path::Path;

/// What containers may use, as a file's label.
pub const CONTAINER_FILE: &str = "system_u:object_r:container_file_t:s0";
/// A screen socket's own label: a container may connect to it.
pub const CONTAINER_SOCKET: &str = "system_u:system_r:container_t:s0";
/// PulseAudio's socket, as the host's SELinux module lets containers use it.
pub const PULSE_SOCKET: &str = "system_u:object_r:wadspaces_sock_t:s0";

/// Is SELinux on (enforcing or not)?
pub fn enabled() -> bool {
    Path::new("/sys/fs/selinux/enforce").exists()
}

/// Labels a file (not following a symlink), when SELinux is on.
pub fn label(path: &Path, label: &str) -> Result<(), String> {
    if !enabled() {
        return Ok(());
    }
    wad_wayland::security_context::label_file(path, label)
}

/// Does the policy let `scon` do `perm` on `class` objects labelled `tcon`?
/// None when it can't be asked (no SELinux, an unknown class or
/// permission); false for a context the policy hasn't (an unknown type).
pub fn allows(scon: &str, tcon: &str, class: &str, perm: &str) -> Option<bool> {
    let base = Path::new("/sys/fs/selinux/class").join(class);
    let index: u16 = std::fs::read_to_string(base.join("index")).ok()?.trim().parse().ok()?;
    let bit: u32 = std::fs::read_to_string(base.join("perms").join(perm)).ok()?.trim().parse().ok()?;
    let mut f = std::fs::OpenOptions::new().read(true).write(true).open("/sys/fs/selinux/access").ok()?;
    if f.write_all(format!("{scon} {tcon} {index}").as_bytes()).is_err() {
        // EINVAL: a context the policy doesn't know.
        return Some(false);
    }
    f.rewind().ok()?;
    let mut answer = String::new();
    f.read_to_string(&mut answer).ok()?;
    let allowed = u32::from_str_radix(answer.split_whitespace().next()?, 16).ok()?;
    Some(allowed & (1 << (bit - 1)) != 0)
}

/// Can wadspaces use PulseAudio's socket on the screen (the host's module)?
pub fn pulse_allowed() -> bool {
    allows(CONTAINER_SOCKET, PULSE_SOCKET, "sock_file", "write") == Some(true)
}

#[cfg(test)]
mod tests {
    /// Against this machine's policy (skipped without SELinux).
    #[test]
    fn asks_the_loaded_policy() {
        if !super::enabled() {
            return;
        }
        let container = super::CONTAINER_SOCKET;
        assert_eq!(super::allows(container, super::CONTAINER_FILE, "sock_file", "write"), Some(true));
        assert_eq!(super::allows(container, "system_u:object_r:no_such_type_t:s0", "sock_file", "write"), Some(false));
        assert_eq!(super::allows(container, super::CONTAINER_FILE, "no_such_class", "write"), None);
    }
}
