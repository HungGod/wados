//! My other machines (peers), as the shell uses them: which ones the
//! launcher's machine picker offers (this one, and mine that are online),
//! their wadspaces (asked of each over the tailnet, through wadd: `GET
//! /v1/peers/{id}/wadspaces`, kept in the store under keys of their
//! machine's, `<peer>:<id>`: see store.rs), and Copy here (a CopyWadspace
//! job: `POST /v1/peers/{id}/wadspaces/{ws}/copy`, sent the id the peer
//! knows it by), with what its toast says.
//!
//! Without a cloud a copy comes from that machine, over the tailnet, so it
//! has to be online; its folders stay there, and come over Taildrive.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use wad_model::{JobKind, JobStatus, PeerKind, ProjectKind, Wadspace};
use wad_proto::{JobInfo, Level, PeerInfo};

use crate::shell::Shell;
use crate::store::{Change, Store, split_key};

/// A peer's wadspaces are asked for again when they're older than this
/// (and each time its machine is picked).
const FRESH: Duration = Duration::from_secs(30);

/// One of the machines the launcher can show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// Its wadd machine id.
    pub id: String,
    pub name: String,
    /// Its glyph: `house` (this one), `laptop` or `monitor`.
    pub icon: &'static str,
    /// This machine.
    pub here: bool,
}

/// A peer's glyph (the spike's: a laptop, else a monitor; a server's own).
pub fn kind_icon(kind: PeerKind) -> &'static str {
    match kind {
        PeerKind::Laptop => "laptop",
        PeerKind::Server => "server",
        PeerKind::Desktop => "monitor",
    }
}

/// The machines the launcher offers: this one, then my others that are
/// online (a copy or a stream comes from there, so it has to answer).
pub fn machines(this_id: &str, this_name: &str, peers: &[PeerInfo]) -> Vec<Choice> {
    let mut out = vec![Choice { id: this_id.into(), name: this_name.into(), icon: "house", here: true }];
    out.extend(peers.iter().filter(|p| p.peer.trusted && p.peer.online && p.peer.id != this_id).map(|p| Choice {
        id: p.peer.id.clone(),
        name: p.peer.name.clone(),
        icon: kind_icon(p.peer.kind),
        here: false,
    }));
    out
}

/// What the picker's list says for one (`{name} (this one)` for this machine).
pub fn choice_label(c: &Choice) -> String {
    if c.here { format!("{} (this one)", c.name) } else { c.name.clone() }
}

/// The machines `x`'s folders stay on: the ones its mounted or copied
/// folders name that aren't `here` (in the order they first come).
pub fn away_machines(x: &Wadspace, here: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for p in &x.look.projects {
        let m = match &p.kind {
            ProjectKind::Mount { source, .. } | ProjectKind::Copy { source } => &source.machine_id,
            ProjectKind::Github { .. } => continue,
        };
        if m != here && !m.is_empty() && !out.contains(m) {
            out.push(m.clone());
        }
    }
    out
}

/// The toast when a copy is here: its title and body. `from` is the machine
/// it came from; `away` the machines (their names) its folders stay on.
pub fn copied(name: &str, from: &str, away: &[String]) -> (String, String) {
    let title = format!("{name} is on this machine");
    let mut body = format!("Copied from {from}. It builds when it first opens.");
    if !away.is_empty() {
        body.push_str(&format!(" Its folders stay on {}, over Taildrive.", away.join(" and ")));
    }
    (title, body)
}

/// Copy here's job, as the Activity flyout names it.
pub fn copy_title(name: &str) -> String {
    format!("Copying {name} to this machine")
}

/// A job's title as the shell shows it: a copy's is the spike's ("Copying
/// {name} to this machine"), when its wadspace is known; any other's is wadd's.
pub fn job_title(store: &Store, j: &JobInfo) -> String {
    if let Some((_, x)) = store.copy_key(j).and_then(|k| store.peer_wadspace(&k)) {
        return copy_title(&x.name);
    }
    j.job.title.clone()
}

/// A copy on its way here (the shell's own, or one Files started).
#[derive(Debug, Clone)]
struct Copy {
    job: Option<String>,
    /// The peer's wadspace's key.
    ws: String,
    name: String,
    /// The machine it comes from (its name).
    from: String,
}

#[derive(Default)]
struct Copies {
    list: RefCell<Vec<Copy>>,
    /// Jobs seen going (so a finished one replayed on a reconnect isn't news).
    going: RefCell<HashSet<String>>,
}

thread_local! {
    static COPIES: Copies = Copies::default();
}

/// Follows the peers for the shell: their wadspaces asked for when they
/// come online (and kept fresh), and copies' jobs, for their toasts.
pub fn start(shell: &Rc<Shell>) {
    let s = Rc::downgrade(shell);
    shell.store.watch(move |c| {
        let Some(shell) = s.upgrade() else { return };
        match c {
            Change::Peers => {
                for p in shell.store.peers().into_iter().filter(|p| p.peer.online) {
                    let stale = shell
                        .store
                        .peer_spaces(&p.peer.id)
                        .is_none_or(|s| !s.loading && now_ms() - s.at > FRESH.as_millis() as i64);
                    if stale {
                        let (sh, id) = (shell.clone(), p.peer.id.clone());
                        // Not inside the store's own telling.
                        glib::idle_add_local_once(move || fetch(&sh, &id));
                    }
                }
            }
            Change::Job(id) => {
                if let Some(j) = shell.store.job(id).filter(|j| j.job.kind == JobKind::CopyWadspace) {
                    let sh = shell.clone();
                    glib::idle_add_local_once(move || job_changed(&sh, &j));
                }
            }
            _ => {}
        }
    });
}

/// Asks `peer` for its wadspaces (they're in the store when they come).
pub fn fetch(shell: &Rc<Shell>, peer: &str) {
    if shell.store.peer_spaces(peer).is_some_and(|s| s.loading) {
        return;
    }
    shell.store.peer_spaces_asked(peer);
    let (s, id) = (shell.clone(), peer.to_string());
    let p = peer.to_string();
    shell.link.call(
        move |w| async move { w.peer_wadspaces(&p).await },
        move |r| s.store.set_peer_spaces(&id, r.map_err(|e| e.message), now_ms()),
    );
}

/// Asks again if what's kept of `peer` is older than [`FRESH`] (or never came).
pub fn refresh(shell: &Rc<Shell>, peer: &str) {
    let stale = shell.store.peer_spaces(peer).is_none_or(|s| now_ms() - s.at > FRESH.as_millis() as i64);
    if stale {
        fetch(shell, peer);
    }
}

/// Copy here: a peer's wadspace (by its key, `<peer>:<id>`) copied to this
/// machine, from it.
pub fn copy_here(shell: &Rc<Shell>, key: &str) {
    let Some((peer, ws)) = split_key(key) else {
        shell.toast(Level::Info, "It's this machine's already", None);
        return;
    };
    let from = shell.store.machine_name(peer).unwrap_or_else(|| "another machine".into());
    let Some((_, x)) = shell.store.peer_wadspace(key) else {
        shell.toast(Level::Warn, &format!("{from} didn't say what that is: open its wadspaces again"), None);
        return;
    };
    expect(shell, &x, &from, None);
    // wadd knows it by the peer's own id.
    let (s, p, w, k) = (shell.clone(), peer.to_string(), ws.to_string(), key.to_string());
    let name = x.name.clone();
    shell.link.call(
        move |wadd| async move { wadd.copy_from_peer(&p, &w).await },
        move |r| match r {
            Ok(job) => {
                s.store.note_copy(&job.job.id, &k);
                COPIES.with(|c| {
                    if let Some(k) = c.list.borrow_mut().iter_mut().find(|k| k.ws == x.id && k.job.is_none()) {
                        k.job = Some(job.job.id.clone());
                    }
                });
                job_changed(&s, &job);
            }
            Err(e) => {
                COPIES.with(|c| c.list.borrow_mut().retain(|k| !(k.ws == x.id && k.job.is_none())));
                crate::overlays::toast_with(&s, Level::Error, &format!("{name} didn’t copy"), &e.message);
            }
        },
    );
}

/// A copy of `x` (from machine `from`) is on its way: its toast will say so.
fn expect(shell: &Shell, x: &Wadspace, from: &str, job: Option<String>) {
    let away: Vec<String> = away_machines(x, &shell.store.machine_id())
        .iter()
        .map(|m| shell.store.machine_name(m).unwrap_or_else(|| "another machine".into()))
        .collect();
    let (title, body) = copied(&x.name, from, &away);
    crate::overlays::expect_body(&title, &body);
    COPIES.with(|c| {
        let mut list = c.list.borrow_mut();
        if !list.iter().any(|k| k.ws == x.id && (k.job.is_none() || k.job == job)) {
            list.push(Copy { job, ws: x.id.clone(), name: x.name.clone(), from: from.to_string() });
        }
    });
}

/// A copy's job moved: one Files started is expected too; one that failed says why.
fn job_changed(shell: &Rc<Shell>, j: &JobInfo) {
    let job = &j.job;
    let going = matches!(job.status, JobStatus::Queued | JobStatus::Running);
    let known = COPIES.with(|c| c.list.borrow().iter().any(|k| k.job.as_deref() == Some(&job.id)));
    if going {
        let first = COPIES.with(|c| c.going.borrow_mut().insert(job.id.clone()));
        // Started elsewhere (Files): what it copies, if the store has it.
        if first
            && !known
            && let Some((peer, x)) = shell.store.copy_key(j).and_then(|k| shell.store.peer_wadspace(&k))
        {
            let from = shell.store.machine_name(&peer).unwrap_or_else(|| "another machine".into());
            expect(shell, &x, &from, Some(job.id.clone()));
        }
        return;
    }
    // Finished: only news if it was seen going.
    if !COPIES.with(|c| c.going.borrow_mut().remove(&job.id)) {
        return;
    }
    let copy = COPIES.with(|c| {
        let mut list = c.list.borrow_mut();
        let i = list.iter().position(|k| k.job.as_deref() == Some(&job.id))?;
        Some(list.remove(i))
    });
    if job.status == JobStatus::Error {
        let name = copy.as_ref().map(|k| k.name.clone()).unwrap_or_else(|| "It".into());
        let why = job.error.clone().unwrap_or_else(|| "It didn't work".into());
        crate::overlays::toast_with(shell, Level::Error, &format!("{name} didn’t copy"), &why);
    }
    // Done: wadd's own notice ("{name} is on this machine") is the toast,
    // with the body expected for it. A copy that made no notice says so here.
    if job.status == JobStatus::Done
        && let Some(k) = copy
    {
        tracing::info!("{} copied here from {}", k.name, k.from);
    }
}

/// Starts one of the machine's apps (Settings at Accounts, Files at
/// Machines): a second start hands its arguments to the window open.
pub fn open_app(shell: &Shell, argv: &[&str]) {
    let Some(prog) = argv.first() else { return };
    let started = glib::find_program_in_path(prog).is_some() && {
        let args: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
        gtk::gio::Subprocess::newv(&args, gtk::gio::SubprocessFlags::NONE)
            .inspect_err(|e| tracing::warn!("{prog}: {e}"))
            .is_ok()
    };
    if !started {
        let name = if *prog == "wados-settings" { "Settings" } else { "Files" };
        shell.toast(Level::Info, &format!("{name} isn't on this machine"), None);
    }
}

pub fn now_ms() -> i64 {
    glib::real_time() / 1000
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_model::{Peer, Project, ProjectSource};

    fn peer(id: &str, name: &str, kind: PeerKind, online: bool) -> PeerInfo {
        PeerInfo {
            peer: Peer {
                id: id.into(),
                name: name.into(),
                hostname: name.to_lowercase(),
                kind,
                trusted: true,
                online,
                last_seen: 0,
                os: "WadOS 44".into(),
                gpu: None,
            },
            node_id: format!("n{name}"),
            dns_name: format!("{}.tail1234.ts.net", name.to_lowercase()),
        }
    }

    #[test]
    fn the_picker_offers_this_machine_and_my_online_ones() {
        let peers = [
            peer("m-desk", "Desk", PeerKind::Desktop, true),
            peer("m-laptop", "Laptop", PeerKind::Laptop, false),
            peer("m-tv", "Living room", PeerKind::Laptop, true),
        ];
        let m = machines("m-surface", "Surface", &peers);
        let names: Vec<String> = m.iter().map(choice_label).collect();
        assert_eq!(names, ["Surface (this one)", "Desk", "Living room"]);
        assert_eq!(m.iter().map(|c| c.icon).collect::<Vec<_>>(), ["house", "monitor", "laptop"]);
        assert!(m[0].here && !m[1].here);
        // Off the tailnet (no peers), or all of them offline: only this one.
        assert_eq!(machines("m-surface", "Surface", &[]).len(), 1);
        assert_eq!(machines("m-surface", "Surface", &peers[1..2]).len(), 1);
        // A peer that's this machine (a stale list) isn't offered twice.
        assert_eq!(machines("m-desk", "Desk", &peers[..1]).len(), 1);
        assert_eq!(kind_icon(PeerKind::Server), "server");
    }

    fn with_projects(sources: &[&str]) -> Wadspace {
        let mut x = wad_model::ops::blank("pixel-art", "Pixel Art", "m-desk", 0, 1);
        x.look.projects = sources
            .iter()
            .enumerate()
            .map(|(i, m)| Project {
                id: format!("p{i}"),
                as_name: format!("P{i}"),
                kind: if m.is_empty() {
                    ProjectKind::Github { repo: "a/b".into(), setup: None }
                } else {
                    ProjectKind::Mount {
                        source: ProjectSource { machine_id: m.to_string(), paths: vec!["~/x".into()] },
                        make: false,
                    }
                },
            })
            .collect();
        x
    }

    #[test]
    fn the_copy_toast_says_where_it_came_from_and_where_its_folders_stay() {
        let (t, b) = copied("Pixel Art", "Desk", &["Desk".into()]);
        assert_eq!(t, "Pixel Art is on this machine");
        assert_eq!(b, "Copied from Desk. It builds when it first opens. Its folders stay on Desk, over Taildrive.");
        // No folders elsewhere: no word of them.
        let (_, b) = copied("Notes", "Desk", &[]);
        assert_eq!(b, "Copied from Desk. It builds when it first opens.");
        let (_, b) = copied("X", "Desk", &["Desk".into(), "Laptop".into()]);
        assert!(b.ends_with("Its folders stay on Desk and Laptop, over Taildrive."));
        assert_eq!(copy_title("Pixel Art"), "Copying Pixel Art to this machine");
    }

    #[test]
    fn a_copys_folders_stay_on_the_machines_they_name() {
        // A clone (GitHub) is anywhere; this machine's own folders are here.
        let x = with_projects(&["m-desk", "", "m-surface", "m-desk", "m-laptop"]);
        assert_eq!(away_machines(&x, "m-surface"), ["m-desk", "m-laptop"]);
        assert!(away_machines(&with_projects(&["", "m-surface"]), "m-surface").is_empty());
    }
}
