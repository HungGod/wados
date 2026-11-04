//! gitimport.py's and test_gitops.py's tests: real git against a bare
//! repository standing in for GitHub, and a stand-in git for the clone.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;

const TOKEN: &str = "ghp_UUUUUUUUUUUUUUUUUUUUUUUUUUUUUUUUUUUU";

fn run(cwd: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn commit(repo: &Path, name: &str) {
    std::fs::write(repo.join(name), "x").unwrap();
    run(repo, &["add", name]);
    run(repo, &["commit", "-q", "-m", &format!("add {name}")]);
}

/// origin.git (bare), a "laptop" clone that pushes to it, and the project
/// folder under test (a clone, as a launch makes).
struct Remote {
    _d: tempfile::TempDir,
    bare: PathBuf,
    laptop: PathBuf,
    project: PathBuf,
}

impl Remote {
    fn new() -> Self {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("remote");
        std::fs::create_dir_all(&root).unwrap();
        let (bare, laptop, project) = (root.join("origin.git"), root.join("laptop"), d.path().join("projects/p1"));
        run(&root, &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()]);
        run(&root, &["clone", "-q", bare.to_str().unwrap(), laptop.to_str().unwrap()]);
        commit(&laptop, "README.md");
        run(&laptop, &["push", "-q", "origin", "main"]);
        std::fs::create_dir_all(project.parent().unwrap()).unwrap();
        run(&root, &["clone", "-q", bare.to_str().unwrap(), project.to_str().unwrap()]);
        Self { _d: d, bare, laptop, project }
    }

    fn push_new(&self, n: usize) {
        for i in 0..n {
            commit(&self.laptop, &format!("new{i}.md"));
        }
        run(&self.laptop, &["push", "-q", "origin", "main"]);
    }

    fn head(&self, repo: &Path) -> String {
        run(repo, &["rev-parse", "HEAD"]).trim().to_string()
    }
}

async fn update(path: &Path) -> Update {
    Git::default().update(path, None, None, FETCH_TIMEOUT).await
}

#[tokio::test]
async fn clean_and_behind_fast_forwards() {
    let r = Remote::new();
    r.push_new(3);
    assert_eq!(update(&r.project).await, Update::Updated(3));
    assert_eq!(r.head(&r.project), r.head(&r.laptop));
    assert!(r.project.join("new2.md").exists());
    assert_eq!(update(&r.project).await, Update::Current);
}

#[tokio::test]
async fn uncommitted_changes_are_left_alone() {
    let r = Remote::new();
    r.push_new(1);
    std::fs::write(r.project.join("README.md"), "my edit").unwrap();
    let before = r.head(&r.project);
    assert_eq!(update(&r.project).await, Update::Dirty);
    assert_eq!(r.head(&r.project), before);
    assert_eq!(std::fs::read_to_string(r.project.join("README.md")).unwrap(), "my edit");
    std::fs::write(r.project.join("README.md"), "x").unwrap();
    std::fs::write(r.project.join("draft.md"), "not added yet").unwrap(); // untracked counts
    assert_eq!(update(&r.project).await, Update::Dirty);
}

#[tokio::test]
async fn unpushed_commits_are_left_alone() {
    let r = Remote::new();
    r.push_new(2);
    commit(&r.project, "mine.md");
    let before = r.head(&r.project);
    assert_eq!(update(&r.project).await, Update::Ahead(1));
    assert_eq!(r.head(&r.project), before);
}

#[tokio::test]
async fn no_upstream_is_left_alone() {
    let r = Remote::new();
    r.push_new(1);
    run(&r.project, &["checkout", "-q", "-b", "local-only"]);
    assert_eq!(update(&r.project).await, Update::NoUpstream);
    run(&r.project, &["checkout", "-q", "--detach"]);
    assert_eq!(update(&r.project).await, Update::NoUpstream);
}

#[tokio::test]
async fn a_failed_fetch_is_only_a_warning() {
    let r = Remote::new();
    r.push_new(1);
    std::fs::remove_dir_all(&r.bare).unwrap();
    match update(&r.project).await {
        Update::Offline(e) => assert!(e.contains("fatal:"), "{e}"),
        other => panic!("{other:?}"),
    }
    assert!(!r.project.join("new0.md").exists());
}

#[tokio::test]
async fn not_a_repository() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(update(d.path()).await, Update::NotGit);
    assert_eq!(Git::default().status(d.path(), None).await, None);
}

#[tokio::test]
async fn status_from_the_refs_here() {
    let r = Remote::new();
    let main = |dirty, ahead, behind| GitStatus {
        branch: Some("main".into()),
        dirty,
        ahead,
        behind,
        upstream: Some("origin/main".into()),
    };
    assert_eq!(Git::default().status(&r.project, None).await, Some(main(false, 0, 0)));
    r.push_new(2);
    // No fetch in status: it doesn't know about the new commits yet.
    assert_eq!(Git::default().status(&r.project, None).await.unwrap().behind, 0);
    run(&r.project, &["fetch", "-q"]);
    commit(&r.project, "mine.md");
    std::fs::write(r.project.join("mine.md"), "changed").unwrap();
    assert_eq!(Git::default().status(&r.project, None).await, Some(main(true, 1, 2)));
}

#[test]
fn parsing_status() {
    assert_eq!(parse_status("# branch.oid abc\n# branch.head (detached)\n"), GitStatus::default());
    // An upstream that's gone has no ab line: as good as none.
    assert_eq!(parse_status("# branch.head main\n# branch.upstream origin/gone\n").upstream, None);
    assert_eq!(
        parse_status("# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -5\n? x\n"),
        GitStatus {
            branch: Some("main".into()),
            dirty: true,
            ahead: 2,
            behind: 5,
            upstream: Some("origin/main".into())
        }
    );
}

#[test]
fn progress_and_hosts() {
    assert_eq!(parse_progress("Receiving objects:  42% (42/100), 1.2 MiB | 3 MiB/s"), Some(0.42));
    assert_eq!(parse_progress("Resolving deltas: 100% (5/5), done."), None);
    assert!(is_github("https://github.com/o/r.git") && is_github("https://GitHub.com/o/r"));
    assert!(
        !is_github("https://gitlab.com/o/r")
            && !is_github("http://github.com/o/r")
            && !is_github("https://github.com.evil.example/o")
    );
}

/// A stand-in git that records its arguments and environment, prints clone
/// progress and makes a README; `mode` (a file) makes it fail or hang.
struct FakeGit {
    _d: tempfile::TempDir,
    dir: PathBuf,
    git: Git,
}

impl FakeGit {
    fn new() -> Self {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_path_buf();
        let rec = dir.join("rec");
        std::fs::create_dir(&rec).unwrap();
        let script = format!(
            r#"#!/bin/bash
[ "$1" = --probe ] && exit 0
printf '%s\n' "$@" > {rec}/argv
env > {rec}/env
dest="${{@: -1}}"
printf 'Receiving objects:   0%% (0/100)\r' >&2
printf 'Receiving objects:  50%% (50/100)\r' >&2
printf 'Receiving objects: 100%% (100/100), done.\n' >&2
echo hi > "$dest/README.md"
mode=$(cat {rec}/mode 2>/dev/null)
[ "$mode" = fail ] && {{ echo "fatal: repository 'https://github.com/o/r.git/' not found" >&2; exit 128; }}
[ "$mode" = slow ] && sleep 30
exit 0
"#,
            rec = rec.display()
        );
        let prog = dir.join("git");
        std::fs::write(&prog, script).unwrap();
        std::fs::set_permissions(&prog, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Another test's child, forked while the script was open for writing,
        // keeps it busy until it execs: run it once it's free.
        for _ in 0..100 {
            match std::process::Command::new(&prog).arg("--probe").status() {
                Err(e) if e.raw_os_error() == Some(26) => std::thread::sleep(std::time::Duration::from_millis(10)),
                _ => break,
            }
        }
        Self { git: Git::with_program(&prog), _d: d, dir }
    }

    fn argv(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("rec/argv")).unwrap().lines().map(String::from).collect()
    }

    fn env(&self) -> String {
        std::fs::read_to_string(self.dir.join("rec/env")).unwrap()
    }

    fn mode(&self, m: &str) {
        std::fs::write(self.dir.join("rec/mode"), m).unwrap();
    }
}

#[tokio::test]
async fn a_clone_lands_in_place_with_progress() {
    let g = FakeGit::new();
    let dest = g.dir.join("projects/p1");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    let mut seen = vec![];
    g.git
        .clone_repo(
            "https://github.com/o/r.git",
            Some("main"),
            &dest,
            Some(TOKEN),
            |t, f| seen.push((t.to_string(), f)),
            None,
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(dest.join("README.md")).unwrap(), "hi\n");
    assert!(!g.dir.join("projects/p1.part").exists());
    assert_eq!(seen.iter().map(|(_, f)| *f).collect::<Vec<_>>(), [Some(0.0), Some(0.5), Some(1.0)]);
    assert_eq!(seen.last().unwrap().0, "Receiving objects: 100% (100/100), done.");
    let argv = g.argv();
    let part = format!("{}.part", dest.display());
    assert_eq!(argv[argv.len() - 5..], ["--branch", "main", "--", "https://github.com/o/r.git", part.as_str()]);
    assert!(g.env().contains("GIT_TERMINAL_PROMPT=0"));
}

#[tokio::test]
async fn the_token_only_travels_in_the_environment() {
    let g = FakeGit::new();
    g.git
        .clone_repo("https://github.com/o/r.git", None, &g.dir.join("p1"), Some(TOKEN), |_, _| {}, None, |_| {})
        .await
        .unwrap();
    let argv = g.argv();
    assert!(!argv.iter().any(|a| a.contains(TOKEN)));
    assert!(g.env().contains(&format!("{TOKEN_ENV}={TOKEN}")));
    // The previous helpers are reset, then the inline one reads the variable.
    assert_eq!(argv[..4], ["-c", "credential.helper=", "-c", &format!("credential.helper={HELPER}")]);
    assert!(!argv.contains(&"--branch".to_string()));
    // Nothing else of wadd's environment goes along.
    assert!(!g.env().contains("CARGO"));
}

#[tokio::test]
async fn no_token_for_other_hosts() {
    let g = FakeGit::new();
    g.git
        .clone_repo("https://gitlab.com/o/r.git", None, &g.dir.join("p1"), Some(TOKEN), |_, _| {}, None, |_| {})
        .await
        .unwrap();
    assert!(!g.env().contains(TOKEN_ENV));
    assert_eq!(g.argv()[0], "clone");
}

#[tokio::test]
async fn a_failed_clone_leaves_nothing() {
    let g = FakeGit::new();
    g.mode("fail");
    let e = g
        .git
        .clone_repo("https://github.com/o/r.git", None, &g.dir.join("p1"), Some(TOKEN), |_, _| {}, None, |_| {})
        .await
        .unwrap_err();
    assert!(matches!(&e, Error::Failed(m) if m.contains("fatal: repository") && m.contains("not found")), "{e}");
    assert!(!g.dir.join("p1").exists() && !g.dir.join("p1.part").exists());
}

#[tokio::test]
async fn a_cancelled_clone_leaves_nothing() {
    let g = FakeGit::new();
    g.mode("slow");
    let (git, dest) = (g.git.clone(), g.dir.join("p1"));
    let task = tokio::spawn(async move {
        git.clone_repo("https://github.com/o/r.git", None, &dest, None, |_, _| {}, None, |_| {}).await
    });
    for _ in 0..200 {
        if g.dir.join("p1.part/README.md").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(!g.dir.join("p1.part").exists() && !g.dir.join("p1").exists());
}

#[tokio::test]
async fn an_existing_folder_and_a_leftover_part() {
    let g = FakeGit::new();
    std::fs::create_dir(g.dir.join("p1")).unwrap();
    let e = g
        .git
        .clone_repo("https://github.com/o/r.git", None, &g.dir.join("p1"), None, |_, _| {}, None, |_| {})
        .await
        .unwrap_err();
    assert!(e.to_string().contains("already exists"));
    std::fs::create_dir(g.dir.join("p2.part")).unwrap();
    std::fs::write(g.dir.join("p2.part/junk"), "from a crash").unwrap();
    g.git
        .clone_repo("https://github.com/o/r.git", None, &g.dir.join("p2"), None, |_, _| {}, None, |_| {})
        .await
        .unwrap();
    let names: Vec<String> = std::fs::read_dir(g.dir.join("p2"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["README.md"]);
}

#[tokio::test]
async fn the_fetch_alone_gets_the_token() {
    let r = Remote::new();
    // A git that logs each call's arguments and environment, then runs the real one.
    let d = tempfile::tempdir().unwrap();
    let log = d.path().join("log");
    let real =
        String::from_utf8(std::process::Command::new("sh").args(["-c", "command -v git"]).output().unwrap().stdout)
            .unwrap();
    let prog = d.path().join("git");
    std::fs::write(
        &prog,
        format!(
            "#!/bin/bash\n{{ echo \"ARGS $*\"; env | grep -c {TOKEN_ENV}= ; }} >> {}\nexec {} \"$@\"\n",
            log.display(),
            real.trim()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&prog, std::fs::Permissions::from_mode(0o755)).unwrap();
    Git::with_program(&prog).update(&r.project, Some(TOKEN), None, FETCH_TIMEOUT).await;
    let text = std::fs::read_to_string(&log).unwrap();
    let calls: Vec<(&str, &str)> = text.lines().collect::<Vec<_>>().chunks(2).map(|c| (c[0], c[1])).collect();
    let fetch = calls.iter().find(|(a, _)| a.contains(" fetch ")).unwrap();
    assert!(!fetch.0.contains(TOKEN) && fetch.0.contains(&format!("credential.https://github.com.helper={HELPER}")));
    assert_eq!(fetch.1, "1");
    assert!(calls.iter().filter(|(a, _)| !a.contains(" fetch ")).all(|(_, n)| *n == "0"));
}

/// As root (wadd on a machine), git runs as the projects user: a clone and a
/// fast-forward work, and the clone is theirs. Skipped unless root; on a
/// laptop, root in a container is enough:
///   podman run --rm -v ./target/debug/deps:/t:ro,z localhost/wadspaces-host \
///     /t/wad_git-<hash> as_root
#[tokio::test]
async fn as_root_git_runs_as_the_projects_user() {
    if !nix::unistd::geteuid().is_root() {
        return;
    }
    const UID: u32 = 1000;
    let r = Remote::new();
    // The projects user reads the remote and writes its own folder.
    std::fs::set_permissions(r._d.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    // A local remote must be theirs too (git won't read another user's
    // repository); GitHub's over HTTPS isn't a folder at all.
    let own = |who: &str| {
        let ok = std::process::Command::new("chown").args(["-R", who]).arg(&r.bare).status().unwrap();
        assert!(ok.success());
    };
    own("1000:1000");
    let clones = r._d.path().join("clones");
    std::fs::create_dir_all(&clones).unwrap();
    nix::unistd::chown(&clones, Some(UID.into()), Some(UID.into())).unwrap();
    let dest = clones.join("p2");
    let git = Git::default();
    git.clone_repo(r.bare.to_str().unwrap(), None, &dest, None, |_, _| {}, Some(UID), |_| {}).await.unwrap();
    use std::os::unix::fs::MetadataExt;
    assert_eq!(std::fs::metadata(dest.join("README.md")).unwrap().uid(), UID);
    own("0:0"); // the test pushes as root
    r.push_new(1);
    own("1000:1000");
    let u = git.update(&dest, None, Some(UID), FETCH_TIMEOUT).await;
    assert_eq!(u, Update::Updated(1));
}

#[test]
fn the_token_goes_only_to_the_configured_github() {
    let real = Git::default();
    assert!(real.is_github("https://github.com/o/r.git") && !real.is_github("http://10.0.2.2:8399/o/r.git"));
    let mock = Git::default().with_github("http://10.0.2.2:8399/");
    assert!(mock.is_github("http://10.0.2.2:8399/o/r.git"));
    assert!(!mock.is_github("http://10.0.2.2:83990/o/r.git") && !mock.is_github("https://github.com/o/r.git"));
    assert!(!mock.is_github("http://10.0.2.2:8399.evil.example/o/r.git"));
    // An empty origin keeps github.com.
    assert!(Git::default().with_github("").is_github("https://github.com/o/r"));
}
