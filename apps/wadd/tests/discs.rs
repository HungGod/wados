//! Discs on the fake machine, with a fake registry: nothing is pushed anywhere.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use common::{Rig, rig};
use wad_model::JobStatus;
use wad_proto::DiscAdd;
use wadd::discs::{MachineDiscs, NEEDS_GITHUB, Registry};

#[derive(Default)]
struct Fake {
    login: Option<(String, String)>,
    pushed: Mutex<Vec<(PathBuf, String)>>,
}

#[async_trait]
impl Registry for Fake {
    async fn login(&self) -> Result<Option<(String, String)>, String> {
        Ok(self.login.clone())
    }
    async fn push(&self, file: &Path, reference: &str, _: &str, _: &str) -> Result<(), String> {
        self.pushed.lock().unwrap().push((file.to_path_buf(), reference.to_string()));
        Ok(())
    }
}

fn discs(rig: &Rig, fake: Arc<Fake>) -> (Arc<MachineDiscs>, PathBuf) {
    let home = rig.dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let h = home.clone();
    let d = MachineDiscs::new(
        &rig.state_dir(),
        Box::new(move || Some(h.clone())),
        fake,
        rig.docs.clone(),
        rig.jobs.clone(),
        rig.bus.clone(),
    );
    (d, home)
}

fn add(path: &Path, push: bool) -> DiscAdd {
    DiscAdd { path: path.display().to_string(), push }
}

#[tokio::test]
async fn a_disc_stays_here_unless_its_asked_to_go_to_packages() {
    let rig = rig();
    let fake = Arc::new(Fake::default());
    let (d, home) = discs(&rig, fake.clone());
    let file = home.join("Mario-Kart_Double Dash.rvz");
    std::fs::write(&file, b"a disc").unwrap();
    let job = d.add(add(&file, false)).await.unwrap();
    assert_eq!(rig.finished(&job.job.id).await, JobStatus::Done);
    let kept = rig.state_dir().join("discs/mario-kart-double-dash/Mario-Kart_Double Dash.rvz");
    assert_eq!(std::fs::read(kept).unwrap(), b"a disc");
    let lib = rig.docs.snapshot();
    let disc = &lib.discs["mario-kart-double-dash"];
    assert_eq!((disc.name.as_str(), disc.reference.as_str()), ("Mario Kart Double Dash", ""));
    assert!(fake.pushed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn asked_it_goes_to_the_accounts_packages() {
    let rig = rig();
    let fake = Arc::new(Fake { login: Some(("hunggod".into(), "t".into())), ..Fake::default() });
    let (d, home) = discs(&rig, fake.clone());
    let file = home.join("mario-party-4.iso");
    std::fs::write(&file, b"a disc").unwrap();
    let job = d.add(add(&file, true)).await.unwrap();
    assert_eq!(rig.finished(&job.job.id).await, JobStatus::Done);
    let want = "ghcr.io/hunggod/wadspaces-disc-mario-party-4:latest";
    assert_eq!(fake.pushed.lock().unwrap()[0].1, want);
    assert_eq!(rig.docs.snapshot().discs["mario-party-4"].reference, want);
}

#[tokio::test]
async fn it_refuses_a_push_with_no_account_and_anything_outside_the_home() {
    let rig = rig();
    let (d, home) = discs(&rig, Arc::new(Fake::default()));
    let file = home.join("a.iso");
    std::fs::write(&file, b"x").unwrap();
    assert_eq!(d.add(add(&file, true)).await.unwrap_err().message, NEEDS_GITHUB);
    // Root reads what it's told to: only the person's own files are taken.
    let secret = rig.dir.path().join("elsewhere.iso");
    std::fs::write(&secret, b"x").unwrap();
    assert!(d.add(add(&secret, false)).await.unwrap_err().message.contains("your home"));
    let link = home.join("link.iso");
    std::os::unix::fs::symlink(&secret, &link).unwrap();
    assert!(d.add(add(&link, false)).await.unwrap_err().message.contains("your home"));
    let text = home.join("notes.txt");
    std::fs::write(&text, b"x").unwrap();
    assert_eq!(d.add(add(&text, false)).await.unwrap_err().message, "A disc is an .iso or an .rvz file");
}
