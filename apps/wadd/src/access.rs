//! Who may use wadd's API: the caller's credentials (SO_PEERCRED on the Unix
//! socket), checked on every request. Root, wadd's own user, the listed users
//! and members of the listed groups (`wados`, `wheel`: the account made at
//! first boot).
//! Everyone else gets 403.

/// The process on the other end of a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peer {
    pub uid: u32,
    pub gid: u32,
    pub pid: Option<i32>,
}

impl Peer {
    /// A connection whose credentials couldn't be read: allowed nowhere.
    pub const UNKNOWN: Peer = Peer { uid: u32::MAX, gid: u32::MAX, pid: None };
}

/// Looks users and groups up (the system's databases; fakes in tests).
pub trait Directory: Send + Sync {
    /// A user's name.
    fn user_name(&self, uid: u32) -> Option<String>;
    /// The names of the groups a user is in (primary and supplementary).
    fn user_groups(&self, name: &str, gid: u32) -> Vec<String>;
}

/// /etc/passwd and /etc/group (via NSS).
pub struct System;

impl Directory for System {
    fn user_name(&self, uid: u32) -> Option<String> {
        nix::unistd::User::from_uid(uid.into()).ok().flatten().map(|u| u.name)
    }

    fn user_groups(&self, name: &str, gid: u32) -> Vec<String> {
        let Ok(cname) = std::ffi::CString::new(name) else { return vec![] };
        nix::unistd::getgrouplist(&cname, gid.into())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|g| nix::unistd::Group::from_gid(g).ok().flatten().map(|g| g.name))
            .collect()
    }
}

pub struct Policy {
    /// wadd's own uid: always allowed (on a laptop, that's you).
    pub own_uid: u32,
    pub users: Vec<String>,
    pub groups: Vec<String>,
    pub directory: Box<dyn Directory>,
}

impl Policy {
    pub fn allows(&self, peer: &Peer) -> bool {
        if peer.uid == 0 || peer.uid == self.own_uid {
            return true;
        }
        if peer.uid == Peer::UNKNOWN.uid {
            return false;
        }
        let Some(name) = self.directory.user_name(peer.uid) else { return false };
        if self.users.contains(&name) {
            return true;
        }
        !self.groups.is_empty() && self.directory.user_groups(&name, peer.gid).iter().any(|g| self.groups.contains(g))
    }
}
