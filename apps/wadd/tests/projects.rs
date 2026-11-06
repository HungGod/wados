//! Projects as a wadspace opens, against the fake podman: this machine's
//! folder mounted, a file copied in, a GitHub clone from a bare repository
//! standing in for github.com, another machine's folder and files over a
//! WebDAV server standing in for Taildrive (the sample tailnet's Desk,
//! online), an offline machine's left out, and what's taken out cleaned up.
//! rclone's mount itself is a stand-in here ([`FakeDrive`]): a real one
//! needs root and FUSE (the VM's run).

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use common::{Rig, read, rig};
use wad_model::edit::FileRef;
use wad_model::{AppId, JobKind, JobStatus, Op, PartState, Phase, Section, SessionDisplay};
use wad_proto::Event;
use wadd::fake_phase6::Sample;
use wadd::projects::{Projects, Settings};
use wadd::taildrive::{self, Dav, Drive, Kind, decode, encode};

const OFFLINE: &str =
    "Plans is on Laptop, which is offline. Folders come over Taildrive, so both machines need to be online.";

// ------------------------------------------------------------ a WebDAV server standing in for Taildrive

/// Serves `root` over WebDAV (PROPFIND, depth 0 or 1, and GET), as
/// golang.org/x/net/webdav (Taildrive's) answers: its URL.
async fn serve_dav(root: PathBuf) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().fallback(move |req: axum::extract::Request| {
        let root = root.clone();
        async move { dav(&root, req) }
    });
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn href(rel: &str, dir: bool) -> String {
    let mut h: String = rel.split('/').filter(|s| !s.is_empty()).map(|s| format!("/{}", encode(s))).collect();
    if dir || h.is_empty() {
        h.push('/');
    }
    h
}

fn response(rel: &str, path: &Path) -> String {
    let m = std::fs::metadata(path).unwrap();
    let props = if m.is_dir() {
        r#"<D:resourcetype><D:collection xmlns:D="DAV:"/></D:resourcetype>"#.to_string()
    } else {
        format!("<D:resourcetype></D:resourcetype><D:getcontentlength>{}</D:getcontentlength>", m.len())
    };
    format!(
        "<D:response><D:href>{}</D:href><D:propstat><D:prop>{props}</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
        href(rel, m.is_dir()).replace('&', "&amp;")
    )
}

fn dav(root: &Path, req: axum::extract::Request) -> axum::response::Response {
    use axum::body::Body;
    use axum::response::Response;
    let rel = decode(req.uri().path());
    let status = |code: u16| Response::builder().status(code).body(Body::empty()).unwrap();
    if rel.split('/').any(|s| s == "..") {
        return status(400);
    }
    if rel.contains("/locked/") {
        return status(403);
    }
    let path = root.join(rel.trim_start_matches('/'));
    if !path.exists() {
        return status(404);
    }
    match req.method().as_str() {
        "PROPFIND" => {
            let depth = req.headers().get("Depth").and_then(|v| v.to_str().ok()).unwrap_or("1").to_string();
            let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8"?><D:multistatus xmlns:D="DAV:">"#);
            xml.push_str(&response(&rel, &path));
            if depth == "1" && path.is_dir() {
                for e in std::fs::read_dir(&path).unwrap().flatten() {
                    let child = format!("{}/{}", rel.trim_end_matches('/'), e.file_name().to_string_lossy());
                    xml.push_str(&response(&child, &e.path()));
                }
            }
            xml.push_str("</D:multistatus>");
            Response::builder()
                .status(207)
                .header("Content-Type", "application/xml; charset=utf-8")
                .body(Body::from(xml))
                .unwrap()
        }
        "GET" if path.is_file() => {
            Response::builder().status(200).body(Body::from(std::fs::read(&path).unwrap())).unwrap()
        }
        _ => status(405),
    }
}

// ------------------------------------------------------------ rclone's stand-in

#[derive(Default)]
struct FakeDrive {
    mounts: Mutex<Vec<(String, String, PathBuf)>>,
    unmounts: Mutex<Vec<(String, PathBuf)>>,
}

#[async_trait]
impl Drive for FakeDrive {
    async fn mount(&self, unit: &str, url: &str, at: &Path) -> Result<(), String> {
        assert!(at.is_dir(), "its mount point is made first");
        self.mounts.lock().unwrap().push((unit.into(), url.into(), at.into()));
        Ok(())
    }
    async fn unmount(&self, unit: &str, at: &Path) -> Result<(), String> {
        self.unmounts.lock().unwrap().push((unit.into(), at.into()));
        Ok(())
    }
}

// ------------------------------------------------------------ the rig

struct Here {
    rig: Rig,
    drive: Arc<FakeDrive>,
    /// Desk's home, as Taildrive serves it.
    desk: PathBuf,
    /// Where the bare repositories standing in for GitHub are (`<owner>/<name>.git`).
    github: PathBuf,
    projects_dir: PathBuf,
}

async fn here() -> Here {
    let rig = rig();
    rig.builds.refresh_base().await;
    let root = rig.dir.path().join("taildrive");
    let desk = root.join("tail1234.ts.net/desk/home");
    std::fs::create_dir_all(&desk).unwrap();
    let url = serve_dav(root).await;
    let github = rig.dir.path().join("github");
    std::fs::create_dir_all(&github).unwrap();
    let projects_dir = rig.dir.path().join("projects");
    let sample =
        Sample::new("test", rig.jobs.clone(), rig.bus.clone(), rig.docs.clone(), rig.builds.clone(), Duration::ZERO);
    let settings = Settings {
        state_dir: rig.state_dir(),
        projects_dir: projects_dir.clone(),
        uid: nix::unistd::getuid().as_raw(),
        root: false,
        system: false,
        home: Some(rig.home.clone()),
        roots: vec![rig.home.clone()],
        machine_id: "m-test".into(),
        github: format!("file://{}", github.display()),
        taildrive: url,
        lookup: wadd::projects::user_of,
    };
    let drive = Arc::new(FakeDrive::default());
    let projects =
        Projects::new(settings, rig.fake.clone(), rig.jobs.clone(), Arc::new(sample)).with_drive(drive.clone());
    rig.runs.attach(Arc::new(projects));
    Here { rig, drive, desk, github, projects_dir }
}

fn git(cwd: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.com",
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

impl Here {
    /// A repository on the stand-in GitHub, with a README.
    fn repo(&self, owner: &str, name: &str) {
        let bare = self.github.join(owner).join(format!("{name}.git"));
        std::fs::create_dir_all(bare.parent().unwrap()).unwrap();
        git(&self.github, &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()]);
        let work = self.rig.dir.path().join(format!("work-{name}"));
        git(&self.github, &["clone", "-q", bare.to_str().unwrap(), work.to_str().unwrap()]);
        std::fs::write(work.join("README.md"), format!("# {name}\n")).unwrap();
        git(&work, &["add", "README.md"]);
        git(&work, &["commit", "-q", "-m", "first"]);
        git(&work, &["push", "-q", "origin", "main"]);
    }

    fn keep(&self, ws: &str, machine: &str, path: &str, name: &str, folder: bool) -> String {
        let files = vec![FileRef { machine: machine.into(), path: path.into(), name: name.into(), folder }];
        self.rig.edit(Op::KeepFiles { ws: ws.into(), files, wallpaper: None });
        self.project(ws, name)
    }

    fn project(&self, ws: &str, name: &str) -> String {
        let x = self.rig.docs.wadspace(ws).unwrap();
        x.look.projects.iter().find(|p| p.as_name == name).unwrap_or_else(|| panic!("no {name}")).id.clone()
    }

    async fn open(&self, ws: &str) {
        let st = self.rig.runs.open(ws, SessionDisplay::Window).unwrap();
        assert_eq!(st.state.phase, Phase::Starting);
        self.rig.runs.settled(ws).await;
        let st = self.rig.runs.state(ws);
        assert_eq!(st.state.phase, Phase::Ready, "{st:?}");
    }

    async fn stop(&self, ws: &str) {
        assert_eq!(self.rig.runs.stop(ws).await.unwrap().state.phase, Phase::Idle);
    }

    fn unit(&self, ws: &str) -> String {
        self.rig.fake.state.lock().unwrap().units[&format!("wad-{ws}.container")].clone()
    }

    fn projects_json(&self, ws: &str) -> serde_json::Value {
        serde_json::from_str(&read(&self.rig.state_dir().join("extra").join(ws).join("projects.json"))).unwrap()
    }

    /// Its newest open job.
    fn job(&self, ws: &str) -> wad_proto::JobInfo {
        let jobs = self.rig.jobs.list();
        jobs.into_iter()
            .find(|j| j.job.kind == JobKind::Launch && j.job.subject.as_deref() == Some(ws))
            .expect("an open job")
    }

    fn lines(&self, job: &str) -> Vec<String> {
        self.rig.jobs.get(job, 0).unwrap().lines
    }
}

fn stage<'a>(j: &'a wad_proto::JobInfo, id: &str) -> &'a wad_model::JobStage {
    j.job.stages.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no stage {id} in {:?}", j.job.stages))
}

fn has(text: &str, line: &str) -> bool {
    text.lines().any(|l| l == line)
}

// ------------------------------------------------------------ this machine

#[tokio::test]
async fn this_machines_folder_is_mounted_and_its_file_copied_in() {
    let h = here().await;
    let ws = h.rig.wadspace("Studio", &[AppId::Terminal], &[]);
    std::fs::create_dir_all(h.rig.home.join("Thesis/ch1")).unwrap();
    std::fs::write(h.rig.home.join("Thesis/ch1/intro.md"), "intro").unwrap();
    std::fs::create_dir_all(h.rig.home.join("Desktop")).unwrap();
    std::fs::write(h.rig.home.join("Desktop/notes.txt"), "remember").unwrap();
    let thesis = h.keep(&ws, "m-test", "~/Thesis", "Thesis", true);
    let notes = h.keep(&ws, "m-test", "~/Desktop/notes.txt", "notes.txt", false);
    let (_, mut rx) = h.rig.bus.subscribe();
    h.open(&ws).await;

    let unit = h.unit(&ws);
    let home = std::fs::canonicalize(&h.rig.home).unwrap();
    assert!(has(&unit, &format!("Volume={}/Thesis:/config/Desktop/Thesis:rw", home.display())), "{unit}");
    assert!(has(&unit, "SecurityLabelDisable=true"), "the person's own folder isn't relabelled: {unit}");
    let copy = h.rig.state_dir().join("copies").join(&ws).join(&notes);
    assert!(has(&unit, &format!("Volume={}:/run/wadspaces-copies/{notes}:ro,z", copy.display())), "{unit}");
    assert_eq!(read(&copy.join("files/notes.txt")), "remember");
    // Its done says it's one file: the wadspace puts it at ~/Desktop/notes.txt.
    let done: serde_json::Value = serde_json::from_str(&read(&copy.join("done"))).unwrap();
    assert_eq!(done["file"], "notes.txt");
    let json = h.projects_json(&ws);
    let kinds: Vec<(&str, &str)> = json["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["id"].as_str().unwrap(), p["kind"].as_str().unwrap()))
        .collect();
    assert_eq!(kinds, [(thesis.as_str(), "mount"), (notes.as_str(), "copy")]);

    // Its open job: a stage each, in the spike's words, then its container and desktop.
    let j = h.job(&ws);
    assert_eq!(j.job.status, JobStatus::Done);
    assert_eq!(j.job.title, "Opening Studio");
    let labels: Vec<&str> = j.job.stages.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, ["Mounting Thesis", "Copying notes.txt", "Starting its container", "Waiting for its desktop"]);
    assert!(j.job.stages.iter().all(|s| s.state == PartState::Done), "{:?}", j.job.stages);
    // The run said what it was doing, and named its job while it opened.
    let messages: Vec<String> = common::drain(&mut rx)
        .into_iter()
        .filter_map(|e| if let Event::Run(r) = e { r.state.message } else { None })
        .collect();
    assert!(
        messages.iter().any(|m| m == "Mounting Thesis") && messages.iter().any(|m| m == "Copying notes.txt"),
        "{messages:?}"
    );
    assert_eq!(h.rig.runs.state(&ws).state.job, None, "ready: no job");

    // Opened again: the copy is here already (made once), the folder mounted again.
    h.stop(&ws).await;
    std::fs::write(h.rig.home.join("Desktop/notes.txt"), "changed").unwrap();
    h.open(&ws).await;
    let j = h.job(&ws);
    assert_eq!(stage(&j, &format!("project:{notes}")).state, PartState::Skipped);
    assert_eq!(read(&copy.join("files/notes.txt")), "remember");
}

#[tokio::test]
async fn only_whats_in_the_home_goes_in() {
    let h = here().await;
    std::fs::create_dir_all(h.rig.home.join(".ssh")).unwrap();
    let outside = h.rig.dir.path().join("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, h.rig.home.join("sneaky")).unwrap();
    // A wadspace from elsewhere, with what keepFiles wouldn't make (`make`, a whole path).
    let base = h.rig.wadspace("Studio", &[AppId::Terminal], &[]);
    let mount = |id: &str, name: &str, path: &str, make: bool| wad_model::Project {
        id: id.into(),
        as_name: name.into(),
        kind: wad_model::ProjectKind::Mount {
            source: wad_model::ProjectSource { machine_id: "m-test".into(), paths: vec![path.into()] },
            make,
        },
    };
    let mut doc = h.rig.docs.wadspace(&base).unwrap();
    doc.look.projects = vec![
        mount("p-ssh", ".ssh", "~/.ssh", false),
        mount("p-link", "sneaky", "~/sneaky", false),
        mount("p-home", "Home", "~", false),
        mount("p-etc", "etc", "/etc", false),
        mount("p-gone", "Gone", "~/Gone", false),
        mount("p-fresh", "Fresh", "~/Fresh", true),
    ];
    let ws = h.rig.edit(Op::Import { doc: Box::new(doc), from: "m-test".into() }).created.unwrap();

    h.open(&ws).await;
    let j = h.job(&ws);
    for p in ["p-ssh", "p-link", "p-home", "p-etc"] {
        let s = stage(&j, &format!("project:{p}"));
        assert_eq!(s.state, PartState::Error, "{s:?}");
        assert!(s.error.as_deref().unwrap().contains("only what's in your home"), "{s:?}");
    }
    let gone = stage(&j, "project:p-gone");
    assert_eq!(gone.error.as_deref(), Some("Gone isn't on this machine any more (~/Gone isn't there)"));
    assert_eq!(stage(&j, "project:p-fresh").state, PartState::Done);
    assert!(h.rig.home.join("Fresh").is_dir(), "made, as it says");
    let unit = h.unit(&ws);
    assert!(!unit.contains(".ssh") && !unit.contains("elsewhere") && !unit.contains("Desktop/Home"), "{unit}");
    assert!(!unit.contains("Volume=/etc"), "{unit}");
    assert!(unit.contains("/config/Desktop/Fresh:rw"), "{unit}");
    assert_eq!(j.job.status, JobStatus::Done, "it opened without them");
}

// ------------------------------------------------------------ GitHub

#[tokio::test]
async fn a_repository_is_cloned_by_wadd_and_worked_in_where_it_is() {
    let h = here().await;
    h.repo("HungGod", "Writing");
    let ws = h.rig.wadspace("Writing", &[AppId::Terminal], &[]);
    h.rig.edit(Op::Add { ws: ws.clone(), section: Section::Look, item: "repo:HungGod/Writing".into(), from: None });
    let pid = h.project(&ws, "Writing");
    h.open(&ws).await;

    let clone = h.projects_dir.join(&ws).join(&pid);
    assert_eq!(read(&clone.join("README.md")), "# Writing\n");
    assert!(!h.projects_dir.join(&ws).join(format!("{pid}.part")).exists());
    let unit = h.unit(&ws);
    assert!(has(&unit, &format!("Volume={}:/config/Desktop/Writing:rw", clone.display())), "{unit}");
    assert!(!unit.contains("SecurityLabelDisable"), "a clone is labelled, SELinux stays on: {unit}");
    let json = h.projects_json(&ws);
    assert_eq!(json["projects"][0]["kind"], "github");
    assert_eq!(json["projects"][0]["repo"], "HungGod/Writing");
    let j = h.job(&ws);
    let s = stage(&j, &format!("project:{pid}"));
    assert_eq!((s.label.as_str(), s.state), ("Cloning HungGod/Writing", PartState::Done));
    assert!(h.lines(&j.job.id).iter().any(|l| l == "» git clone https://github.com/HungGod/Writing ~/Desktop/Writing"));

    // Opened again: it's here (the wadspace fetches it itself).
    h.stop(&ws).await;
    std::fs::write(clone.join("mine.md"), "work in progress").unwrap();
    h.open(&ws).await;
    let j = h.job(&ws);
    assert_eq!(stage(&j, &format!("project:{pid}")).state, PartState::Skipped);
    assert!(clone.join("mine.md").is_file());
}

#[tokio::test]
async fn a_private_repository_without_a_sign_in_is_left_out_and_says_so() {
    let h = here().await;
    let ws = h.rig.wadspace("Secret", &[AppId::Terminal], &[]);
    h.rig.edit(Op::Add { ws: ws.clone(), section: Section::Look, item: "repo:HungGod/Hidden".into(), from: None });
    let pid = h.project(&ws, "Hidden");
    let (_, mut rx) = h.rig.bus.subscribe();
    h.open(&ws).await;
    let j = h.job(&ws);
    let s = stage(&j, &format!("project:{pid}"));
    assert_eq!((s.state, s.error.as_deref()), (PartState::Error, Some("It's private: sign in to GitHub first")));
    assert_eq!(j.job.status, JobStatus::Done);
    assert!(!h.unit(&ws).contains("/config/Desktop/"));
    assert_eq!(h.projects_json(&ws)["projects"], serde_json::json!([]));
    assert!(!h.projects_dir.join(&ws).join(&pid).exists());
    let notices: Vec<String> = common::drain(&mut rx)
        .into_iter()
        .filter_map(|e| if let Event::Notice { title, .. } = e { Some(title) } else { None })
        .collect();
    assert_eq!(notices, ["Hidden: It's private: sign in to GitHub first"]);
}

#[tokio::test]
async fn taking_a_project_out_cleans_up_only_what_wadd_made() {
    let h = here().await;
    h.repo("HungGod", "Writing");
    h.repo("HungGod", "Drafts");
    // Nothing else in it: once its projects are out, it's let go (gone for good).
    let ws = h.rig.wadspace("Writing", &[], &[]);
    h.rig.edit(Op::Add { ws: ws.clone(), section: Section::Look, item: "repo:HungGod/Writing".into(), from: None });
    h.rig.edit(Op::Add { ws: ws.clone(), section: Section::Look, item: "repo:HungGod/Drafts".into(), from: None });
    std::fs::create_dir_all(h.rig.home.join("Thesis")).unwrap();
    std::fs::write(h.rig.home.join("Thesis/a.md"), "mine").unwrap();
    std::fs::write(h.rig.home.join("deck.pdf"), "%PDF").unwrap();
    let (writing, drafts) = (h.project(&ws, "Writing"), h.project(&ws, "Drafts"));
    let thesis = h.keep(&ws, "m-test", "~/Thesis", "Thesis", true);
    let deck = h.keep(&ws, "m-test", "~/deck.pdf", "deck.pdf", false);
    h.open(&ws).await;
    h.stop(&ws).await;
    let clones = h.projects_dir.join(&ws);
    // Drafts has work that isn't pushed.
    std::fs::write(clones.join(&drafts).join("draft.md"), "unsaved").unwrap();

    for p in [&writing, &drafts, &thesis, &deck] {
        let out = h.rig.edit(Op::RemoveProject { ws: ws.clone(), project: p.to_string() });
        h.rig.runs.edited(&out.changed).await;
    }
    assert!(!clones.join(&writing).exists(), "a clean clone goes");
    assert!(!clones.join(&drafts).exists());
    let kept: Vec<String> = std::fs::read_dir(&clones)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".kept-"))
        .collect();
    assert_eq!(kept.len(), 1, "work that isn't pushed is kept aside");
    assert!(clones.join(&kept[0]).join("draft.md").is_file());
    assert!(!h.rig.state_dir().join("copies").join(&ws).join(&deck).exists(), "its copy goes");
    assert_eq!(read(&h.rig.home.join("Thesis/a.md")), "mine", "the person's folder is never touched");
    assert!(h.rig.home.join("deck.pdf").is_file());

    // Gone for good: everything it had, what was kept aside too.
    let out = h.rig.edit(Op::DiscardIfEmpty { ws: ws.clone() });
    assert!(h.rig.docs.wadspace(&ws).is_none());
    h.rig.runs.edited(&out.changed).await;
    assert!(!clones.exists());
}

// ------------------------------------------------------------ another machine

#[tokio::test]
async fn an_offline_machines_folder_is_left_out_with_the_spikes_words() {
    let h = here().await;
    let ws = h.rig.wadspace("Plans", &[AppId::Terminal], &[]);
    let pid = h.keep(&ws, "m-laptop", "~/Documents/Plans", "Plans", true);
    h.open(&ws).await;
    let j = h.job(&ws);
    let s = stage(&j, &format!("project:{pid}"));
    assert_eq!(s.label, "Mounting Plans from Laptop over Taildrive");
    assert_eq!((s.state, s.error.as_deref()), (PartState::Error, Some(OFFLINE)));
    assert!(h.lines(&j.job.id).contains(&format!("✗ {OFFLINE}")));
    assert_eq!(j.job.status, JobStatus::Done);
    assert!(h.drive.mounts.lock().unwrap().is_empty());
    assert!(!h.unit(&ws).contains("/config/Desktop/"));
    // A copy from it, too.
    let ws2 = h.rig.wadspace("Copy", &[AppId::Terminal], &[]);
    let pid2 = h.keep(&ws2, "m-laptop", "~/Documents/Plans", "Plans", false);
    h.open(&ws2).await;
    let j = h.job(&ws2);
    let s = stage(&j, &format!("project:{pid2}"));
    assert_eq!((s.label.as_str(), s.error.as_deref()), ("Copying Plans from Laptop over Taildrive", Some(OFFLINE)));
}

#[tokio::test]
async fn an_online_machines_files_come_over_taildrive() {
    let h = here().await;
    std::fs::create_dir_all(h.desk.join("Pictures/Sprites/walk cycle")).unwrap();
    std::fs::write(h.desk.join("Pictures/Sprites/hero.png"), "PNG1").unwrap();
    std::fs::write(h.desk.join("Pictures/Sprites/walk cycle/frame 1.png"), "PNG22").unwrap();
    std::fs::write(h.desk.join("Pictures/a&b.txt"), "and").unwrap();
    std::fs::create_dir_all(h.desk.join("Music")).unwrap();
    let ws = h.rig.wadspace("Art", &[AppId::Terminal], &[]);
    let sprites = h.keep(&ws, "m-desk", "~/Pictures/Sprites", "Sprites", false);
    let file = h.keep(&ws, "m-desk", "~/Pictures/a&b.txt", "a&b.txt", false);
    let music = h.keep(&ws, "m-desk", "~/Music", "Music", true);
    let gone = h.keep(&ws, "m-desk", "~/Videos", "Videos", true);
    h.open(&ws).await;

    // Copies fetched over WebDAV, folders and all.
    let copies = h.rig.state_dir().join("copies").join(&ws);
    assert_eq!(read(&copies.join(&sprites).join("files/hero.png")), "PNG1");
    assert_eq!(read(&copies.join(&sprites).join("files/walk cycle/frame 1.png")), "PNG22");
    assert_eq!(read(&copies.join(&file).join("files/a&b.txt")), "and");
    let done = |p: &str| -> serde_json::Value { serde_json::from_str(&read(&copies.join(p).join("done"))).unwrap() };
    assert_eq!(done(&file)["file"], "a&b.txt");
    assert!(done(&sprites).get("file").is_none(), "a folder's copy is its contents");
    let j = h.job(&ws);
    let s = stage(&j, &format!("project:{sprites}"));
    assert_eq!((s.label.as_str(), s.state), ("Copying Sprites from Desk over Taildrive", PartState::Done));
    assert_eq!((s.done, s.bytes), (9, 9));
    // A folder mounted: rclone's stand-in got Desk's folder on Taildrive.
    let mounts = h.drive.mounts.lock().unwrap().clone();
    assert_eq!(mounts.len(), 1);
    let (unit_name, url, at) = &mounts[0];
    assert!(url.ends_with("/tail1234.ts.net/desk/home/Music"), "{url}");
    assert_eq!(*at, h.rig.state_dir().join("taildrive").join(&ws).join(&music));
    assert_eq!(*unit_name, taildrive::unit_name(&ws, &music));
    let unit = h.unit(&ws);
    assert!(has(&unit, &format!("Volume={}:/config/Desktop/Music:rw", at.display())), "{unit}");
    assert!(has(&unit, "SecurityLabelDisable=true"), "{unit}");
    let s = stage(&j, &format!("project:{music}"));
    assert_eq!((s.label.as_str(), s.state), ("Mounting Music from Desk over Taildrive", PartState::Done));
    // One that isn't on Desk any more is left out.
    assert_eq!(stage(&j, &format!("project:{gone}")).error.as_deref(), Some("Videos isn't on Desk any more"));

    // Stopped: its mounts go, their folders with them.
    h.drive.unmounts.lock().unwrap().clear();
    h.stop(&ws).await;
    assert_eq!(h.drive.unmounts.lock().unwrap().clone(), [(unit_name.clone(), at.clone())]);
    assert!(!at.exists());
}

#[tokio::test]
async fn webdav_as_taildrive_answers() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("t/desk/home");
    std::fs::create_dir_all(home.join("A folder/inner")).unwrap();
    std::fs::write(home.join("A folder/one.txt"), "1").unwrap();
    std::fs::write(home.join("A folder/inner/two.txt"), "22").unwrap();
    std::fs::create_dir_all(home.join("locked")).unwrap();
    let base = serve_dav(d.path().to_path_buf()).await;
    let url = taildrive::home_url(&base, "t", "desk.t", "desk");
    let dav = Dav::new();
    let folder = taildrive::url_in(&url, "~/A folder").unwrap();
    assert_eq!(dav.stat(&folder).await.unwrap(), Kind::Folder);
    assert_eq!(dav.stat(&format!("{folder}/one.txt")).await.unwrap(), Kind::File(1));
    let all = dav.walk(&folder, 100).await.unwrap();
    let mut names: Vec<(String, Kind)> = all.clone();
    names.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        names,
        [
            ("inner".to_string(), Kind::Folder),
            ("inner/two.txt".into(), Kind::File(2)),
            ("one.txt".into(), Kind::File(1))
        ]
    );
    assert!(dav.walk(&folder, 1).await.is_err(), "too many");
    assert_eq!(dav.stat(&format!("{url}/nothing")).await.unwrap_err(), taildrive::DavError::NotFound);
    assert!(matches!(dav.stat(&format!("{url}/locked/")).await.unwrap_err(), taildrive::DavError::Refused(403)));
    let to = d.path().join("got.txt");
    assert_eq!(dav.download(&format!("{folder}/inner/two.txt"), &to, |_| {}).await.unwrap(), 2);
    assert_eq!(read(&to), "22");
    // Nobody there.
    let e = Dav::new().stat("http://127.0.0.1:9/x").await.unwrap_err();
    assert!(matches!(e, taildrive::DavError::Unreachable(_)), "{e:?}");
}

// ------------------------------------------------------------ Open with

#[tokio::test]
async fn a_file_opened_with_a_wadspace_is_mounted_for_that_run_only() {
    let h = here().await;
    let ws = h.rig.wadspace("Terminal", &[AppId::Terminal], &[]);
    std::fs::create_dir_all(h.rig.home.join("Documents")).unwrap();
    std::fs::write(h.rig.home.join("Documents/notes.txt"), "remember").unwrap();
    let st = h.rig.runs.open_with(&ws, SessionDisplay::Screen, "~/Documents/notes.txt", None).await.unwrap();
    assert_eq!(st.state.phase, Phase::Starting);
    h.rig.runs.settled(&ws).await;
    assert_eq!(h.rig.runs.state(&ws).state.phase, Phase::Ready);
    let home = std::fs::canonicalize(&h.rig.home).unwrap();
    let mount = format!("Volume={}/Documents/notes.txt:/config/Desktop/notes.txt:rw", home.display());
    assert!(has(&h.unit(&ws), &mount), "{}", h.unit(&ws));
    // Its document keeps nothing of it.
    assert!(h.rig.docs.wadspace(&ws).unwrap().look.projects.is_empty());

    // Running: another file is copied onto its Desktop, inside it.
    std::fs::write(h.rig.home.join("Documents/more.txt"), "more").unwrap();
    h.rig.runs.open_with(&ws, SessionDisplay::Screen, "~/Documents/more.txt", None).await.unwrap();
    let calls = h.rig.fake.calls();
    assert!(
        calls
            .iter()
            .any(|c| c.starts_with(&format!("exec wad-{ws} abc sh -c")) && c.contains("/config/Desktop/more.txt")),
        "{calls:?}"
    );
    // Not what's outside the home, nor hidden.
    let e = h.rig.runs.open_with(&ws, SessionDisplay::Screen, "/etc/passwd", None).await.unwrap_err();
    assert_eq!(e.code, wad_proto::ErrorCode::Conflict, "{}", e.message);

    // Opened again without it: it's gone.
    h.stop(&ws).await;
    h.open(&ws).await;
    assert!(!h.unit(&ws).contains("notes.txt"), "{}", h.unit(&ws));
    // And the empty file its mount stood on is cleared from its home.
    let calls = h.rig.fake.calls();
    assert!(
        calls
            .iter()
            .any(|c| c.starts_with(&format!("exec wad-{ws} root sh -c")) && c.ends_with("/config/Desktop/notes.txt")),
        "{calls:?}"
    );
}

/// A machine's person (`sam`, uid 1000), made after wadd started: their home
/// at `/var/home/sam` reached through `/home` (a link, as on Fedora's atomic
/// images), found when a file's opened.
static PERSON: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

fn sam(uid: u32) -> Option<(PathBuf, String)> {
    if uid != 1000 {
        return None;
    }
    Some((PERSON.get()?.clone(), "sam".to_string()))
}

#[tokio::test]
async fn a_machines_person_made_after_wadd_started_has_their_home_found() {
    let rig = rig();
    rig.builds.refresh_base().await;
    let var = rig.dir.path().join("var/home/sam");
    std::fs::create_dir_all(var.join("Documents")).unwrap();
    std::fs::write(var.join("Documents/notes.txt"), "remember").unwrap();
    std::os::unix::fs::symlink(rig.dir.path().join("var/home"), rig.dir.path().join("homes")).unwrap();
    let sample =
        Sample::new("test", rig.jobs.clone(), rig.bus.clone(), rig.docs.clone(), rig.builds.clone(), Duration::ZERO);
    // As wadd started on a machine with no person yet: no home, nothing allowed.
    let settings = Settings {
        state_dir: rig.state_dir(),
        projects_dir: rig.dir.path().join("projects"),
        uid: 1000,
        root: false,
        system: true,
        home: None,
        roots: vec![],
        machine_id: "m-test".into(),
        github: "file:///nowhere".into(),
        taildrive: "http://127.0.0.1:9".into(),
        lookup: sam,
    };
    let projects = Projects::new(settings, rig.fake.clone(), rig.jobs.clone(), Arc::new(sample));
    rig.runs.attach(Arc::new(projects));
    let ws = rig.wadspace("Terminal", &[AppId::Terminal], &[]);
    // Not yet made: said so.
    let e = rig.runs.open_with(&ws, SessionDisplay::Screen, "~/Documents/notes.txt", None).await.unwrap_err();
    assert!(e.message.contains("no home"), "{}", e.message);
    // sam made now (useradd): their home, through /home's link.
    PERSON.set(rig.dir.path().join("homes/sam")).unwrap();
    let st = rig.runs.open_with(&ws, SessionDisplay::Screen, "~/Documents/notes.txt", None).await.unwrap();
    assert_eq!(st.state.phase, Phase::Starting);
    rig.runs.settled(&ws).await;
    assert_eq!(rig.runs.state(&ws).state.phase, Phase::Ready, "{:?}", rig.runs.state(&ws));
    let unit = rig.fake.state.lock().unwrap().units[&format!("wad-{ws}.container")].clone();
    let real = std::fs::canonicalize(&var).unwrap();
    let mount = format!("Volume={}/Documents/notes.txt:/config/Desktop/notes.txt:rw", real.display());
    assert!(has(&unit, &mount), "{unit}");
    // Outside their home: no.
    let outside = rig.dir.path().join("elsewhere.txt");
    std::fs::write(&outside, "x").unwrap();
    assert!(rig.runs.open_with(&ws, SessionDisplay::Screen, outside.to_str().unwrap(), None).await.is_err());
}

#[tokio::test]
async fn a_wadspace_deleted_for_good_leaves_no_volume() {
    let h = here().await;
    let ws = h.rig.wadspace("Terminal", &[AppId::Terminal], &[]);
    let other = h.rig.wadspace("Other", &[AppId::Terminal], &[]);
    h.open(&ws).await;
    h.stop(&ws).await;
    {
        let mut st = h.rig.fake.state.lock().unwrap();
        for v in [
            format!("wad-{ws}-config"),
            format!("wadstream-{ws}"),
            format!("wadsecure-{ws}"),
            format!("wad-{other}-config"),
        ] {
            st.volumes.insert(v);
        }
    }
    h.rig.edit(Op::Trash { ws: ws.clone() });
    let out = h.rig.docs.purge(&ws).unwrap();
    tokio::time::timeout(Duration::from_secs(20), h.rig.runs.edited(&out.changed)).await.expect("it didn't wait long");
    let left: Vec<String> = h.rig.fake.state.lock().unwrap().volumes.iter().cloned().collect();
    assert_eq!(left, [format!("wad-{other}-config")], "only the other wadspace's stays");
}
