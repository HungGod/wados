//! The projects button's menu, "A folder, file or repository" (the spike's
//! `apps/builder/projects.ts` `filesMenu`), as data: under "Keep in {name}",
//! this machine's folders (those in your home, its Desktop and its
//! Documents), your GitHub repositories (`/v1/github/repos`, the last pushed
//! first), and each of your other machines (`/v1/peers`): an online one's
//! home folders, an offline one off ("{name} is offline": its folders come
//! over Taildrive, so it has to be online).
//!
//! Each choice is one edit: a folder is `space.keepFiles` (mounted when the
//! wadspace opens), a repository the `repo:<owner/name>` piece (cloned).
//! What's in it already is off.
//!
//! Another machine's folders aren't listed from here (that's Taildrive's,
//! and wadd's): its home's usual ones are offered, [`PEER_FOLDERS`].

use std::path::Path;

use wad_model::edit::{FileRef, Op};
use wad_model::{ProjectKind, Section, Wadspace};
use wad_proto::{PeerInfo, Repo};

use crate::builder::side::keys::{Act, Choice, Pick};

/// Folders of this machine the menu lists, at most.
pub const MOST_FOLDERS: usize = 60;
/// What another machine's part of the menu offers: its home's usual folders.
pub const PEER_FOLDERS: [&str; 6] = ["Desktop", "Documents", "Downloads", "Music", "Pictures", "Videos"];

/// A folder of this machine: its name, and its path as a project names it ('~/Documents/Thesis').
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub name: String,
    pub path: String,
}

/// The folders in `home`, its Desktop and its Documents (none hidden),
/// each part by name; at most `most`.
pub fn folders(home: &Path, most: usize) -> Vec<Folder> {
    let mut out = vec![];
    for sub in ["", "Desktop", "Documents"] {
        let dir = if sub.is_empty() { home.to_path_buf() } else { home.join(sub) };
        let mut here: Vec<Folder> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            // The Desktop and Documents are listed for what's in them.
            .filter(|n| !(sub.is_empty() && (n == "Desktop" || n == "Documents")))
            .map(|n| Folder { path: if sub.is_empty() { format!("~/{n}") } else { format!("~/{sub}/{n}") }, name: n })
            .collect();
        here.sort_by_key(|f| f.name.to_lowercase());
        out.extend(here);
    }
    out.truncate(most);
    out
}

/// Whether `x` keeps the folder or file at `path` of `machine` already.
fn keeps(x: &Wadspace, machine: &str, path: &str) -> bool {
    x.look.projects.iter().any(|p| match &p.kind {
        ProjectKind::Mount { source, .. } | ProjectKind::Copy { source } => {
            source.machine_id == machine && source.paths.iter().any(|q| q == path)
        }
        ProjectKind::Github { .. } => false,
    })
}

fn has_repo(x: &Wadspace, full_name: &str) -> bool {
    x.look
        .projects
        .iter()
        .any(|p| matches!(&p.kind, ProjectKind::Github { repo, .. } if repo.eq_ignore_ascii_case(full_name)))
}

fn keep_folder(x: &Wadspace, machine: &str, name: &str, path: &str) -> Act {
    Act::Ops(vec![Op::KeepFiles {
        ws: x.id.clone(),
        files: vec![FileRef { machine: machine.into(), path: path.into(), name: name.into(), folder: true }],
        wallpaper: None,
    }])
}

fn pick(id: String, label: impl Into<String>, icon: &'static str, off: Option<String>, act: Act) -> Choice {
    Choice::Item(Pick { id, label: label.into(), icon, checked: false, off, act })
}

/// The menu for `x`, on machine `here`: its folders, the repositories
/// (or why there are none), and the other machines.
pub fn choices(
    x: &Wadspace,
    here: &str,
    folders: &[Folder],
    repos: &Result<Vec<Repo>, String>,
    peers: &[PeerInfo],
) -> Vec<Choice> {
    const IN_IT: &str = "It's in it already";
    let mut out = vec![Choice::Heading(format!("Keep in {}", x.name))];

    out.push(Choice::Heading("Folders".into()));
    if folders.is_empty() {
        out.push(pick(
            "files-here".into(),
            "No folders here",
            "folder",
            Some("Your home has none yet".into()),
            Act::Ops(vec![]),
        ));
    }
    for f in folders {
        let off = keeps(x, here, &f.path).then(|| IN_IT.to_string());
        out.push(pick(format!("keep:{}", f.path), &f.name, "folder", off, keep_folder(x, here, &f.name, &f.path)));
    }

    out.push(Choice::Heading("GitHub".into()));
    match repos {
        Err(why) => {
            out.push(pick("files-repos".into(), "Your repositories", "git-branch", Some(why.clone()), Act::Ops(vec![])))
        }
        Ok(list) if list.is_empty() => out.push(pick(
            "files-repos".into(),
            "No repositories",
            "git-branch",
            Some("Your GitHub account has none".into()),
            Act::Ops(vec![]),
        )),
        Ok(list) => {
            let mut sorted: Vec<&Repo> = list.iter().collect();
            sorted.sort_by(|a, b| b.pushed_at.cmp(&a.pushed_at).then_with(|| a.full_name.cmp(&b.full_name)));
            for r in sorted {
                let off = has_repo(x, &r.full_name).then(|| IN_IT.to_string());
                let act = Act::Ops(vec![Op::Add {
                    ws: x.id.clone(),
                    section: Section::Look,
                    item: format!("repo:{}", r.full_name),
                    from: None,
                }]);
                out.push(pick(format!("repo:{}", r.full_name), &r.full_name, "git-branch", off, act));
            }
        }
    }

    for p in peers.iter().filter(|p| p.peer.trusted && p.peer.id != here) {
        let icon = crate::peers::kind_icon(p.peer.kind);
        out.push(Choice::Heading(p.peer.name.clone()));
        if !p.peer.online {
            let off = Some(format!("{} is offline", p.peer.name));
            out.push(pick(format!("files-peer:{}", p.peer.id), &p.peer.name, icon, off, Act::Ops(vec![])));
            continue;
        }
        for name in PEER_FOLDERS {
            let path = format!("~/{name}");
            let off = keeps(x, &p.peer.id, &path).then(|| IN_IT.to_string());
            let act = keep_folder(x, &p.peer.id, name, &path);
            out.push(pick(format!("keep:{}:{path}", p.peer.id), name, "folder", off, act));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_model::{Peer, PeerKind, Project, ProjectSource};

    fn repo(full: &str, pushed: Option<i64>) -> Repo {
        let name = full.split('/').nth(1).unwrap().to_string();
        Repo {
            full_name: full.into(),
            name,
            private: false,
            url: format!("https://github.com/{full}.git"),
            default_branch: "main".into(),
            pushed_at: pushed,
            description: String::new(),
        }
    }

    fn peer(id: &str, name: &str, online: bool) -> PeerInfo {
        PeerInfo {
            peer: Peer {
                id: id.into(),
                name: name.into(),
                hostname: name.to_lowercase(),
                kind: PeerKind::Laptop,
                trusted: true,
                online,
                last_seen: 0,
                os: String::new(),
                gpu: None,
            },
            node_id: format!("n{name}"),
            dns_name: format!("{}.tail1234.ts.net", name.to_lowercase()),
        }
    }

    fn studio() -> Wadspace {
        let mut x = wad_model::ops::blank("studio", "Studio", "m-here", 0, 1);
        x.look.projects = vec![
            Project {
                id: "p-1".into(),
                as_name: "Thesis".into(),
                kind: ProjectKind::Mount {
                    source: ProjectSource { machine_id: "m-here".into(), paths: vec!["~/Documents/Thesis".into()] },
                    make: false,
                },
            },
            Project {
                id: "gh-writing".into(),
                as_name: "Writing".into(),
                kind: ProjectKind::Github { repo: "HungGod/Writing".into(), setup: None },
            },
        ];
        x
    }

    fn items(c: &[Choice]) -> Vec<(String, Option<String>)> {
        c.iter()
            .filter_map(|c| if let Choice::Item(p) = c { Some((p.id.clone(), p.off.clone())) } else { None })
            .collect()
    }

    fn headings(c: &[Choice]) -> Vec<String> {
        c.iter().filter_map(|c| if let Choice::Heading(h) = c { Some(h.clone()) } else { None }).collect()
    }

    #[test]
    fn this_machines_folders_in_the_home_its_desktop_and_documents() {
        let d = std::env::temp_dir().join(format!("wados-keep-menu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for f in ["Pictures", ".config", "Desktop/Sprites", "Documents/Thesis", "Documents/apple"] {
            std::fs::create_dir_all(d.join(f)).unwrap();
        }
        std::fs::write(d.join("Desktop/notes.txt"), "a file").unwrap();
        let f = folders(&d, MOST_FOLDERS);
        let paths: Vec<&str> = f.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["~/Pictures", "~/Desktop/Sprites", "~/Documents/apple", "~/Documents/Thesis"]);
        assert_eq!(f[3].name, "Thesis");
        assert_eq!(folders(&d, 2).len(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_menu_keeps_folders_clones_repos_and_offers_my_machines() {
        let x = studio();
        let folders = vec![
            Folder { name: "Pictures".into(), path: "~/Pictures".into() },
            Folder { name: "Thesis".into(), path: "~/Documents/Thesis".into() },
        ];
        let repos = Ok(vec![
            repo("HungGod/Old", Some(10)),
            repo("HungGod/Writing", Some(30)),
            repo("HungGod/WadSpaces", Some(20)),
            repo("HungGod/Never", None),
        ]);
        let peers = vec![peer("m-desk", "Desk", true), peer("m-laptop", "Laptop", false), peer("m-here", "Here", true)];
        let c = choices(&x, "m-here", &folders, &repos, &peers);
        assert_eq!(headings(&c), ["Keep in Studio", "Folders", "GitHub", "Desk", "Laptop"]);
        let it = items(&c);
        let ids: Vec<&str> = it.iter().map(|(i, _)| i.as_str()).collect();
        // The last pushed first; what's in it already is off.
        assert_eq!(
            ids[..6],
            [
                "keep:~/Pictures",
                "keep:~/Documents/Thesis",
                "repo:HungGod/Writing",
                "repo:HungGod/WadSpaces",
                "repo:HungGod/Old",
                "repo:HungGod/Never"
            ]
        );
        assert_eq!(it[1].1.as_deref(), Some("It's in it already"));
        assert_eq!(it[2].1.as_deref(), Some("It's in it already"));
        assert_eq!(it[0].1, None);
        // An online machine: its home's folders; an offline one, off, saying so.
        assert!(ids.contains(&"keep:m-desk:~/Documents") && ids.contains(&"keep:m-desk:~/Pictures"));
        let laptop = it.iter().find(|(i, _)| i == "files-peer:m-laptop").unwrap();
        assert_eq!(laptop.1.as_deref(), Some("Laptop is offline"));
        assert!(!ids.iter().any(|i| i.starts_with("keep:m-laptop:")));
        assert!(!ids.iter().any(|i| i.contains("m-here:")), "this machine isn't one of the others");

        // What each does: a folder kept (mounted), a repository's piece (cloned).
        let act = |id: &str| {
            c.iter()
                .find_map(|c| if let Choice::Item(p) = c { (p.id == id).then(|| p.act.clone()) } else { None })
                .unwrap()
        };
        assert_eq!(
            act("keep:m-desk:~/Music"),
            Act::Ops(vec![Op::KeepFiles {
                ws: "studio".into(),
                files: vec![FileRef {
                    machine: "m-desk".into(),
                    path: "~/Music".into(),
                    name: "Music".into(),
                    folder: true
                }],
                wallpaper: None,
            }])
        );
        assert_eq!(
            act("repo:HungGod/Old"),
            Act::Ops(vec![Op::Add {
                ws: "studio".into(),
                section: Section::Look,
                item: "repo:HungGod/Old".into(),
                from: None
            }])
        );
    }

    #[test]
    fn without_github_or_folders_it_says_why() {
        let x = studio();
        let c = choices(&x, "m-here", &[], &Err("Sign in to WadSpaces first".into()), &[]);
        let it = items(&c);
        assert_eq!(it[0], ("files-here".to_string(), Some("Your home has none yet".to_string())));
        assert_eq!(it[1], ("files-repos".to_string(), Some("Sign in to WadSpaces first".to_string())));
        let none = choices(&x, "m-here", &[], &Ok(vec![]), &[]);
        assert_eq!(items(&none)[1].1.as_deref(), Some("Your GitHub account has none"));
    }
}
