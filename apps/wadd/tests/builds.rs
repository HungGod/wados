//! Building wadspaces' images against the fake podman: which packages are
//! here, pulled or built, in what order; chains and sets; what a wadspace
//! is (built, update, never) after builds and edits; COPY steps.

mod common;

use common::{ABI, INDEX, Options, rig, rig_with};
use wad_gen::{BASE_IMAGE, labels, packages};
use wad_model::{AppId, Built, CommandKind, JobStatus, Op};
use wad_proto::ErrorCode;

fn builds_of(rig: &common::Rig) -> Vec<String> {
    rig.fake.builds().into_iter().map(|b| b.tag).collect()
}

#[tokio::test]
async fn packages_here_are_used_and_the_rest_built_here_needs_first() {
    let rig = rig();
    let node = rig.package_here("nodejs");
    let id = rig.wadspace("Studio", &[AppId::Terminal], &["terminal:nodejs", "terminal:rust"]);
    assert_eq!(rig.builds.state_of(&rig.docs.wadspace(&id).unwrap()).state.built, Built::Never);

    let answer = rig.builds.request(&id).await.unwrap();
    let job = answer.job.expect("a build").job;
    assert_eq!(answer.state.job.as_deref(), Some(job.id.as_str()));
    let stages: Vec<&str> = job.stages.iter().map(|s| s.id.as_str()).collect();
    // WadOS's order: cpp (rust needs it), nodejs, rust.
    assert_eq!(stages, ["pkg:cpp", "pkg:nodejs", "pkg:rust", "stack", "set"]);
    assert_eq!(rig.finished(&job.id).await, JobStatus::Done);

    // cpp and rust built here (cpp first: rust's is built on it), nodejs as it was.
    let built = builds_of(&rig);
    assert_eq!(built[..2], ["localhost/wados-pkg-cpp:building", "localhost/wados-pkg-rust:building"]);
    let rust = rig.fake.builds().into_iter().find(|b| b.tag.contains("pkg-rust")).unwrap();
    let cpp = rig.fake.image("localhost/wados-pkg-cpp:latest").expect("cpp's package");
    assert!(
        rust.dockerfile.contains(&format!("COPY --from={} / /", packages::local_ref("cpp", &cpp.id))),
        "{}",
        rust.dockerfile
    );
    assert_eq!(rust.labels[labels::PKG_ABI], ABI);
    assert!(!rig.fake.calls().iter().any(|c| c.starts_with("pull ")), "nothing pulled: {:?}", rig.fake.calls());
    // Each package stacked on the chain under it, then the set on the top one.
    let chains: Vec<&String> = built.iter().filter(|t| t.starts_with(packages::CHAIN)).collect();
    assert_eq!(chains.len(), 3);
    let set = rig.fake.builds().pop().unwrap();
    let image = answer.state.layer.replace("set:", "localhost/wados-set-") + ":latest";
    assert_eq!(set.tag, image);
    assert!(set.dockerfile.contains("RUN wadspaces-pkg assemble"));
    assert!(set.dockerfile.contains(&format!("io.wadspaces.pkgs=\"cpp@{}", &cpp.id[7..19])), "{}", set.dockerfile);
    assert!(set.dockerfile.contains(&format!("nodejs@{}", &node[7..19])));
    assert_eq!(set.args["BASE_IMAGE"], chains[2].as_str());

    let st = rig.builds.state_of(&rig.docs.wadspace(&id).unwrap());
    assert_eq!((st.state.built, st.image.as_deref(), st.job), (Built::Built, Some(image.as_str()), None));
}

#[tokio::test]
async fn a_set_here_already_is_no_build_and_its_wadspaces_share_it() {
    let rig = rig();
    rig.package_here("nodejs");
    let a = rig.wadspace("A", &[AppId::Terminal], &["terminal:nodejs"]);
    let b = rig.wadspace("B", &[AppId::Terminal], &["terminal:nodejs"]);
    let job = rig.builds.request(&a).await.unwrap().job.unwrap().job.id;
    rig.finished(&job).await;
    let n = rig.fake.builds().len();
    // The same set: B is built with A's build.
    assert_eq!(rig.builds.state_of(&rig.docs.wadspace(&b).unwrap()).state.built, Built::Built);
    let again = rig.builds.request(&b).await.unwrap();
    assert!(again.job.is_none());
    assert_eq!(again.state.state.built, Built::Built);
    assert_eq!(rig.fake.builds().len(), n, "nothing built again");
}

#[tokio::test]
async fn an_edit_shows_as_an_update_until_its_built_again() {
    let rig = rig();
    rig.package_here("nodejs");
    let id = rig.wadspace("Studio", &[AppId::Terminal], &["terminal:nodejs"]);
    let job = rig.builds.request(&id).await.unwrap().job.unwrap().job.id;
    rig.finished(&job).await;
    let first = rig.builds.state_of(&rig.docs.wadspace(&id).unwrap());
    let (_, mut rx) = rig.bus.subscribe();

    // A step of its own: another set; meanwhile it runs the one it has.
    let out = rig.edit(Op::AddCommand { ws: id.clone(), kind: CommandKind::Run, text: "dnf install -y tree".into() });
    assert!(out.needs_build);
    let now = rig.builds.state_of(&rig.docs.wadspace(&id).unwrap());
    assert_eq!(now.state.built, Built::Update);
    assert_eq!(now.image, first.image);
    assert_ne!(now.layer, first.layer);
    let heard = common::drain(&mut rx);
    assert!(
        heard.iter().any(|e| matches!(e, wad_proto::Event::Build(b) if b.id == id && b.state.built == Built::Update))
    );

    // Built again: only its own steps on the chain it has.
    let before = rig.fake.builds().len();
    let job = rig.builds.request(&id).await.unwrap().job.unwrap().job.id;
    assert_eq!(rig.finished(&job).await, JobStatus::Done);
    let new: Vec<String> = builds_of(&rig)[before..].to_vec();
    assert_eq!(new.len(), 1, "{new:?}");
    let set = rig.fake.builds().pop().unwrap();
    assert!(set.dockerfile.contains("RUN dnf install -y tree"));
    assert!(rig.jobs.get(&job, 0).unwrap().lines.iter().any(|l| l.contains("stacked already")));
    assert_eq!(rig.builds.state_of(&rig.docs.wadspace(&id).unwrap()).state.built, Built::Built);

    // Back as it was: the first set, still here.
    let cmd = rig.docs.wadspace(&id).unwrap().commands[0].id.clone();
    rig.edit(Op::RemoveCommand { ws: id.clone(), id: cmd });
    let back = rig.builds.request(&id).await.unwrap();
    assert!(back.job.is_none());
    assert_eq!(back.state.image, first.image);
}

#[tokio::test]
async fn a_newer_base_is_an_update() {
    let rig = rig();
    let id = rig.wadspace("Tools", &[], &[]);
    rig.edit(Op::AddCommand { ws: id.clone(), kind: CommandKind::Run, text: "true".into() });
    rig.builds.refresh_base().await;
    let job = rig.builds.request(&id).await.unwrap().job.unwrap().job.id;
    rig.finished(&job).await;
    assert_eq!(rig.builds.state_of(&rig.docs.wadspace(&id).unwrap()).state.built, Built::Built);
    rig.fake.add_image(BASE_IMAGE, &[(labels::ABI, ABI), (labels::EXTRAS, "1")]);
    assert!(rig.builds.refresh_base().await);
    assert_eq!(rig.builds.state_of(&rig.docs.wadspace(&id).unwrap()).state.built, Built::Update);
}

#[tokio::test]
async fn the_base_as_it_is_needs_no_build() {
    let rig = rig();
    let id = rig.wadspace("Plain", &[AppId::Terminal], &[]);
    let a = rig.builds.request(&id).await.unwrap();
    assert!(a.job.is_none());
    assert_eq!(
        (a.state.state.built, a.state.layer.as_str(), a.state.image.as_deref()),
        (Built::Built, "base", Some(BASE_IMAGE))
    );
    assert!(rig.fake.builds().is_empty());

    // Without the base, nothing is built and nothing opens.
    let bare = rig_with(Options { base: false, ..Default::default() });
    let id = bare.wadspace("Plain", &[AppId::Terminal], &[]);
    assert_eq!(bare.builds.request(&id).await.unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(bare.builds.state_of(&bare.docs.wadspace(&id).unwrap()).state.built, Built::Never);
}

#[tokio::test]
async fn packages_are_pulled_when_ci_published_them_and_built_when_offline() {
    let rig = rig_with(Options { pull: true, ..Default::default() });
    let remote = format!("ghcr.io/hunggod/wados-pkg-nodejs@sha256:{}", "d".repeat(64));
    let image = rig.fake.publish(&remote, &[(labels::PKG, "nodejs"), (labels::PKG_ABI, ABI)]);
    let index = serde_json::json!({
        "version": 1, "registry": "ghcr.io/hunggod",
        "abis": {ABI: {"base": "sha256:b", "packages": {"nodejs": {"digest": format!("sha256:{}", "d".repeat(64)), "imageId": image, "size": 50_000_000}}}}
    });
    rig.fake.state.lock().unwrap().documents.insert(INDEX.into(), serde_json::to_vec(&index).unwrap());
    let id = rig.wadspace("Node", &[AppId::Terminal], &["terminal:nodejs"]);
    let job = rig.builds.request(&id).await.unwrap().job.unwrap().job.id;
    assert_eq!(rig.finished(&job).await, JobStatus::Done);
    let calls = rig.fake.calls();
    assert!(calls.contains(&format!("pull {remote}")), "{calls:?}");
    assert!(!calls.iter().any(|c| c.contains("pkg-nodejs:building")));
    let info = rig.jobs.info(&job).unwrap();
    assert!(info.job.stages[0].bytes > 0, "the download's bytes: {:?}", info.job.stages[0]);
    // Saved, for when the network's out.
    assert!(rig.state_dir().join("packages/index.json").exists());

    // Offline, with nothing saved: built here.
    let off = rig_with(Options { pull: true, ..Default::default() });
    off.fake.state.lock().unwrap().offline = true;
    let id = off.wadspace("Node", &[AppId::Terminal], &["terminal:nodejs"]);
    let job = off.builds.request(&id).await.unwrap().job.unwrap().job.id;
    assert_eq!(off.finished(&job).await, JobStatus::Done);
    assert!(builds_of(&off).contains(&"localhost/wados-pkg-nodejs:building".to_string()));
    assert!(
        off.jobs
            .get(&job, 0)
            .unwrap()
            .lines
            .iter()
            .any(|l| l.contains("Node.js: built here") || l.contains("built here"))
    );
}

#[tokio::test]
async fn a_pull_that_fails_is_built_here_instead() {
    let rig = rig_with(Options { pull: true, ..Default::default() });
    let index = serde_json::json!({
        "version": 1, "registry": "ghcr.io/hunggod",
        "abis": {ABI: {"packages": {"nodejs": {"digest": format!("sha256:{}", "e".repeat(64)), "imageId": "sha256:nothere"}}}}
    });
    rig.fake.state.lock().unwrap().documents.insert(INDEX.into(), serde_json::to_vec(&index).unwrap());
    let id = rig.wadspace("Node", &[AppId::Terminal], &["terminal:nodejs"]);
    let job = rig.builds.request(&id).await.unwrap().job.unwrap().job.id;
    assert_eq!(rig.finished(&job).await, JobStatus::Done);
    let lines = rig.jobs.get(&job, 0).unwrap().lines;
    assert!(lines.iter().any(|l| l.contains("couldn't download it")), "{lines:?}");
    assert!(builds_of(&rig).contains(&"localhost/wados-pkg-nodejs:building".to_string()));
}

#[tokio::test]
async fn copy_steps_take_only_from_home_within_the_cap() {
    let rig = rig();
    std::fs::create_dir_all(rig.home.join("notes/sub")).unwrap();
    std::fs::write(rig.home.join("notes/a.md"), "# a").unwrap();
    std::fs::write(rig.home.join("notes/sub/b.md"), "# b").unwrap();
    let id = rig.wadspace("Notes", &[], &[]);
    rig.edit(Op::AddCommand { ws: id.clone(), kind: CommandKind::Copy, text: "~/notes /config/notes".into() });
    let job = rig.builds.request(&id).await.unwrap().job.unwrap().job.id;
    assert_eq!(rig.finished(&job).await, JobStatus::Done);
    let set = rig.fake.builds().pop().unwrap();
    assert!(set.dockerfile.contains(r#"COPY ["copy/1/notes","/config/notes"]"#), "{}", set.dockerfile);
    assert_eq!(set.files["copy/1/notes/sub/b.md"], b"# b");
    // No packages: the set is the base and its steps.
    assert_eq!(set.args["BASE_IMAGE"], BASE_IMAGE);
    assert!(!set.dockerfile.contains("assemble"));

    // Nothing from outside your home, nor through a link out of it.
    let out = rig.wadspace("Out", &[], &[]);
    rig.edit(Op::AddCommand { ws: out.clone(), kind: CommandKind::Copy, text: "/etc/hostname /etc/x".into() });
    let e = rig.builds.request(&out).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::BadRequest);
    assert!(e.message.contains("isn't in your home folder"), "{}", e.message);
    std::os::unix::fs::symlink("/etc", rig.home.join("etc")).unwrap();
    let link = rig.wadspace("Link", &[], &[]);
    rig.edit(Op::AddCommand { ws: link.clone(), kind: CommandKind::Copy, text: "~/etc/hostname /x".into() });
    assert!(rig.builds.request(&link).await.unwrap_err().message.contains("isn't in your home folder"));
    // Nor more than the cap (1 MB here).
    std::fs::write(rig.home.join("big"), vec![0u8; 2 << 20]).unwrap();
    let big = rig.wadspace("Big", &[], &[]);
    rig.edit(Op::AddCommand { ws: big.clone(), kind: CommandKind::Copy, text: "~/big /big".into() });
    let job = rig.builds.request(&big).await.unwrap().job.unwrap().job.id;
    assert_eq!(rig.finished(&job).await, JobStatus::Error);
    assert!(rig.jobs.info(&job).unwrap().job.error.unwrap().contains("more than the 1 MB"));
}

#[tokio::test]
async fn a_build_can_be_cancelled_and_a_failed_one_says_why() {
    let rig = rig();
    rig.fake.hold_builds();
    let id = rig.wadspace("Slow", &[AppId::Terminal], &["terminal:python"]);
    let job = rig.builds.request(&id).await.unwrap().job.unwrap().job.id;
    // Asked again while it builds: the same build.
    assert_eq!(rig.builds.request(&id).await.unwrap().job.unwrap().job.id, job);
    for _ in 0..100 {
        if !rig.fake.builds().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let info = rig.jobs.cancel(&job).unwrap();
    assert_eq!(info.job.status, JobStatus::Cancelled);
    rig.builds.cancelled(info.job.subject.as_deref());
    assert_eq!(rig.builds.state_of(&rig.docs.wadspace(&id).unwrap()).job, None);
    rig.fake.release_builds();

    rig.fake.state.lock().unwrap().fail_builds.push("wadspaces-feature python".into());
    let job = rig.builds.request(&id).await.unwrap().job.unwrap().job.id;
    assert_eq!(rig.finished(&job).await, JobStatus::Error);
    let info = rig.jobs.info(&job).unwrap();
    assert!(info.job.error.as_deref().unwrap().contains("building the python package"), "{:?}", info.job.error);
    assert_eq!(info.job.stages[0].state, wad_model::PartState::Error);
    assert_eq!(rig.builds.state_of(&rig.docs.wadspace(&id).unwrap()).state.built, Built::Never);
}

#[tokio::test]
async fn a_step_that_cant_be_one_is_refused_before_anything_builds() {
    let rig = rig();
    let id = rig.wadspace("Bad", &[], &[]);
    rig.edit(Op::AddCommand { ws: id.clone(), kind: CommandKind::Copy, text: "~/*.txt /x/".into() });
    let e = rig.builds.request(&id).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::BadRequest);
    assert!(rig.fake.builds().is_empty());
}
