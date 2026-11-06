//! What Files knows, from wadd: this machine's wadspaces (the Trash's too)
//! and which are running, the tailnet (and who's asking), my other
//! machines, each one's wadspaces as it last sent them, and copies on their
//! way here. And where it is ([`Loc`]), with what each place is called.

use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use wad_model::{JobKind, JobStatus, Phase, Wadspace};
use wad_proto::{Event, JobInfo, MachineInfo, PeerInfo, Tailnet, TailnetState};

/// Where Files is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loc {
    /// A folder on this machine.
    Local(PathBuf),
    /// This machine's Trash (its files and its wadspaces).
    Trash,
    /// One of this machine's wadspaces (what's in it, read-only).
    Space(String),
    /// My other machines.
    Machines,
    /// One of them: its home and its wadspaces.
    Peer(String),
    /// A folder in its home, by its path's names from there (empty: the home).
    Folder { peer: String, path: Vec<String> },
}

impl Loc {
    /// One up (the Machines' is none).
    pub fn up(&self) -> Option<Loc> {
        match self {
            Loc::Local(p) => p.parent().map(|p| Loc::Local(p.to_path_buf())),
            Loc::Trash | Loc::Space(_) => None,
            Loc::Machines => None,
            Loc::Peer(_) => Some(Loc::Machines),
            Loc::Folder { peer, path } if path.is_empty() => Some(Loc::Peer(peer.clone())),
            Loc::Folder { peer, path } => {
                Some(Loc::Folder { peer: peer.clone(), path: path[..path.len() - 1].to_vec() })
            }
        }
    }

    pub fn peer(&self) -> Option<&str> {
        match self {
            Loc::Machines | Loc::Local(_) | Loc::Trash | Loc::Space(_) => None,
            Loc::Peer(p) | Loc::Folder { peer: p, .. } => Some(p),
        }
    }
}

/// A crumb: what it says, where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub label: String,
    pub icon: Option<&'static str>,
    pub at: Loc,
}

/// The way to `loc`: Home › Pictures (or / › tmp outside it), Trash, a
/// wadspace, or Machines › Desk › Home › Pictures. Deep down, the
/// middle folds (Files shows the first, "…" and the last two).
pub fn crumbs(loc: &Loc, home: &Path, name_of: impl Fn(&str) -> String) -> Vec<Crumb> {
    match loc {
        Loc::Local(p) => return local_crumbs(p, home),
        Loc::Trash => return vec![Crumb { label: "Trash".into(), icon: Some("trash"), at: Loc::Trash }],
        Loc::Space(id) => return vec![Crumb { label: name_of(id), icon: Some("monitor"), at: loc.clone() }],
        _ => {}
    }
    let mut out = vec![Crumb { label: "Machines".into(), icon: Some("server"), at: Loc::Machines }];
    let Some(peer) = loc.peer() else { return out };
    out.push(Crumb { label: name_of(peer), icon: None, at: Loc::Peer(peer.to_string()) });
    if let Loc::Folder { path, .. } = loc {
        out.push(Crumb { label: "Home".into(), icon: None, at: Loc::Folder { peer: peer.to_string(), path: vec![] } });
        for i in 0..path.len() {
            out.push(Crumb {
                label: path[i].clone(),
                icon: None,
                at: Loc::Folder { peer: peer.to_string(), path: path[..=i].to_vec() },
            });
        }
    }
    out
}

fn local_crumbs(p: &Path, home: &Path) -> Vec<Crumb> {
    let (mut out, rest, mut at) = match p.strip_prefix(home) {
        Ok(rest) => (
            vec![Crumb { label: "Home".into(), icon: Some("house"), at: Loc::Local(home.to_path_buf()) }],
            rest,
            home.to_path_buf(),
        ),
        Err(_) => (
            vec![Crumb { label: "/".into(), icon: Some("hard-drive"), at: Loc::Local(PathBuf::from("/")) }],
            p.strip_prefix("/").unwrap_or(p),
            PathBuf::from("/"),
        ),
    };
    for c in rest.components() {
        at.push(c);
        out.push(Crumb { label: c.as_os_str().to_string_lossy().into_owned(), icon: None, at: Loc::Local(at.clone()) });
    }
    out
}

/// The crumbs shown: deep down, the first, "…" (none: its menu of the
/// middle ones) and the last two.
pub fn shown_crumbs(all: &[Crumb]) -> (Vec<Option<Crumb>>, Vec<Crumb>) {
    if all.len() <= 3 {
        return (all.iter().cloned().map(Some).collect(), vec![]);
    }
    let n = all.len();
    let shown = vec![Some(all[0].clone()), None, Some(all[n - 2].clone()), Some(all[n - 1].clone())];
    (shown, all[1..n - 2].iter().rev().cloned().collect())
}

/// What it says of a machine's state: Online, or when it was last seen.
pub fn status(p: &PeerInfo, now_ms: i64) -> String {
    if p.peer.online {
        "Online".into()
    } else {
        format!("Offline · last seen {}", wad_gtk::kit::ago(now_ms, p.peer.last_seen))
    }
}

/// "{n} wadspaces".
pub fn count(n: usize) -> String {
    format!("{n} wadspace{}", if n == 1 { "" } else { "s" })
}

/// A peer's wadspaces, as it last sent them.
#[derive(Debug, Clone, Default)]
pub struct Spaces {
    pub list: Vec<Wadspace>,
    pub loading: bool,
    pub error: Option<String>,
    /// When they came (ms; 0 never).
    pub at: i64,
}

#[derive(Debug, Default)]
pub struct State {
    pub connected: bool,
    /// This machine's wadspaces and the Trash's (wadd's library); none yet: not heard.
    pub library: Option<Vec<Wadspace>>,
    /// The wadspaces running here.
    pub running: std::collections::HashSet<String>,
    pub machine: Option<MachineInfo>,
    pub tailnet: Option<Tailnet>,
    pub peers: Vec<PeerInfo>,
    /// wadd has said who they are.
    pub peers_known: bool,
    pub spaces: IndexMap<String, Spaces>,
    /// Copies here (CopyWadspace jobs), by job.
    pub copies: IndexMap<String, JobInfo>,
    /// The copies Files started: each job's machine and wadspace (its id there).
    pub started: IndexMap<String, (String, String)>,
}

impl State {
    /// One of wadd's events: true if Files shows what it changed.
    pub fn apply(&mut self, e: Event) -> bool {
        match e {
            Event::Machine(m) => self.machine = Some(m),
            Event::Library(l) => self.library = Some(l.wadspaces),
            Event::Wadspace(w) => {
                let list = self.library.get_or_insert_default();
                match list.iter_mut().find(|x| x.id == w.id) {
                    Some(x) => *x = *w,
                    None => list.push(*w),
                }
            }
            Event::WadspaceGone { id } => {
                if let Some(l) = &mut self.library {
                    l.retain(|x| x.id != id);
                }
            }
            Event::Run(r) => {
                if matches!(r.state.phase, Phase::Starting | Phase::Waiting | Phase::Ready) {
                    self.running.insert(r.id.clone());
                } else {
                    self.running.remove(&r.id);
                }
            }
            Event::Tailnet(t) => self.tailnet = Some(t),
            Event::Peers { peers } => {
                self.spaces.retain(|id, _| peers.iter().any(|p| &p.peer.id == id));
                self.peers = peers;
                self.peers_known = true;
            }
            Event::Job { job } if job.job.kind == JobKind::CopyWadspace => {
                self.copies.insert(job.job.id.clone(), *job);
            }
            _ => return false,
        }
        true
    }

    /// The tailnet and the machines are known (a machine's home can be found).
    pub fn ready(&self) -> bool {
        self.tailnet.is_some() && self.peers_known
    }

    pub fn running(&self) -> bool {
        self.tailnet.as_ref().is_some_and(|t| t.state == TailnetState::Running)
    }

    pub fn peer(&self, id: &str) -> Option<&PeerInfo> {
        self.peers.iter().find(|p| p.peer.id == id)
    }

    /// A copy Files started: it copies `peer`'s `ws`.
    pub fn started(&mut self, job: JobInfo, peer: &str, ws: &str) {
        self.started.insert(job.job.id.clone(), (peer.to_string(), ws.to_string()));
        self.copies.insert(job.job.id.clone(), job);
    }

    /// The machine a copy's job copies from. An id is only unique on its
    /// machine, so it's the one Files asked; else the one wadd names (its
    /// `peer`); else the only machine with a wadspace by that id (none when
    /// several have one).
    pub fn copy_from(&self, j: &JobInfo) -> Option<String> {
        if let Some((p, _)) = self.started.get(&j.job.id) {
            return Some(p.clone());
        }
        if let Some(p) = j.job.params.get("peer").and_then(|p| p.as_str()) {
            return Some(p.to_string());
        }
        let ws = j.job.subject.as_deref()?;
        let mut has = self.spaces.iter().filter(|(_, s)| s.list.iter().any(|x| x.id == ws));
        match (has.next(), has.next()) {
            (Some((p, _)), None) => Some(p.clone()),
            _ => None,
        }
    }

    /// A copy of `peer`'s `ws` on its way here.
    pub fn copying(&self, peer: &str, ws: &str) -> bool {
        self.copies.values().any(|j| {
            matches!(j.job.status, JobStatus::Queued | JobStatus::Running)
                && j.job.subject.as_deref() == Some(ws)
                && self.copy_from(j).as_deref() == Some(peer)
        })
    }

    fn me(&self) -> &str {
        self.machine.as_ref().map(|m| m.id.as_str()).unwrap_or("")
    }

    /// This machine's wadspaces not in the Trash, in order.
    pub fn live(&self) -> Vec<Wadspace> {
        let me = self.me();
        let mut v: Vec<Wadspace> = self
            .library
            .iter()
            .flatten()
            .filter(|x| !x.is_trashed() && (x.machine == me || x.machine.is_empty()))
            .cloned()
            .collect();
        v.sort_by_key(|x| x.order);
        v
    }

    /// This machine's wadspaces in the Trash.
    pub fn trashed(&self) -> Vec<Wadspace> {
        let me = self.me();
        self.library
            .iter()
            .flatten()
            .filter(|x| x.is_trashed() && (x.machine == me || x.machine.is_empty()))
            .cloned()
            .collect()
    }

    pub fn wadspace(&self, id: &str) -> Option<&Wadspace> {
        self.library.iter().flatten().find(|x| x.id == id)
    }

    /// The machines to show: mine on the tailnet (trusted: always).
    pub fn machines(&self) -> Vec<&PeerInfo> {
        let me = self.machine.as_ref().map(|m| m.id.as_str()).unwrap_or("");
        self.peers.iter().filter(|p| p.peer.trusted && p.peer.id != me).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_model::{Peer, PeerKind};

    fn peer(online: bool, last_seen: i64) -> PeerInfo {
        PeerInfo {
            peer: Peer {
                id: "m-laptop".into(),
                name: "Laptop".into(),
                hostname: "laptop".into(),
                kind: PeerKind::Laptop,
                trusted: true,
                online,
                last_seen,
                os: "WadOS 44".into(),
                gpu: None,
            },
            node_id: "nLaptop1".into(),
            dns_name: "laptop.tail1234.ts.net".into(),
        }
    }

    #[test]
    fn a_machine_is_online_or_says_when_it_was_seen() {
        let now = 1_000_000_000_000;
        assert_eq!(status(&peer(true, now), now), "Online");
        assert_eq!(status(&peer(false, now - 51 * 3_600_000), now), "Offline · last seen 2 days ago");
        assert_eq!(count(1), "1 wadspace");
        assert_eq!(count(2), "2 wadspaces");
    }

    #[test]
    fn a_copy_is_its_machines_and_no_others() {
        let job = |id: &str, subject: &str| -> JobInfo {
            serde_json::from_value(serde_json::json!({
                "id": id, "kind": "copyWadspace", "status": "running", "title": "Copying WadOS to this machine",
                "subject": subject, "startedAt": 1, "source": "machine", "lineCount": 0
            }))
            .unwrap()
        };
        let mut st = State::default();
        let ws = |id: &str, m: &str| wad_model::ops::blank(id, "WadOS", m, 0, 1);
        st.spaces.insert("m-desk".into(), Spaces { list: vec![ws("wadspaces", "m-desk")], at: 1, ..Spaces::default() });
        st.spaces
            .insert("m-laptop".into(), Spaces { list: vec![ws("wadspaces", "m-laptop")], at: 1, ..Spaces::default() });
        // Started here, from Desk: Desk's starter is copying, the Laptop's (the same id) isn't.
        st.started(job("j1", "wadspaces"), "m-desk", "wadspaces");
        assert!(st.copying("m-desk", "wadspaces"));
        assert!(!st.copying("m-laptop", "wadspaces"));
        // Started elsewhere, two machines with that id, wadd naming none: neither.
        st.apply(Event::Job { job: Box::new(job("j2", "wadspaces")) });
        assert_eq!(st.copy_from(&job("j2", "wadspaces")), None);
        assert!(!st.copying("m-laptop", "wadspaces"));
    }

    #[test]
    fn the_crumbs_go_from_machines_to_the_folder() {
        let loc = Loc::Folder { peer: "m-desk".into(), path: vec!["Pictures".into(), "Sprites".into()] };
        let c = crumbs(&loc, Path::new("/home/sam"), |_| "Desk".into());
        let labels: Vec<&str> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["Machines", "Desk", "Home", "Pictures", "Sprites"]);
        assert_eq!(c[3].at, Loc::Folder { peer: "m-desk".into(), path: vec!["Pictures".into()] });
        // Up: the folder above, the home, the machine, Machines, then nothing.
        let up = loc.up().unwrap();
        assert_eq!(up, Loc::Folder { peer: "m-desk".into(), path: vec!["Pictures".into()] });
        let home = Loc::Folder { peer: "m-desk".into(), path: vec![] };
        assert_eq!(home.up(), Some(Loc::Peer("m-desk".into())));
        assert_eq!(Loc::Peer("m-desk".into()).up(), Some(Loc::Machines));
        assert_eq!(Loc::Machines.up(), None);
        assert_eq!(crumbs(&Loc::Machines, Path::new("/h"), |_| String::new()).len(), 1);
    }

    #[test]
    fn this_machines_crumbs_start_at_home() {
        let home = Path::new("/home/sam");
        let c = crumbs(&Loc::Local(home.join("Pictures/Sprites/old")), home, |_| String::new());
        let labels: Vec<&str> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["Home", "Pictures", "Sprites", "old"]);
        assert_eq!(c[0].icon, Some("house"));
        let (shown, folded) = shown_crumbs(&c);
        let shown: Vec<String> = shown.iter().map(|c| c.as_ref().map_or("…".into(), |c| c.label.clone())).collect();
        assert_eq!(shown, ["Home", "…", "Sprites", "old"]);
        assert_eq!(folded.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["Pictures"]);
        let out = crumbs(&Loc::Local(PathBuf::from("/tmp/x")), home, |_| String::new());
        assert_eq!(out.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["/", "tmp", "x"]);
        assert_eq!(Loc::Local(home.join("Pictures")).up(), Some(Loc::Local(home.to_path_buf())));
        assert_eq!(crumbs(&Loc::Trash, home, |_| String::new())[0].label, "Trash");
    }
}
