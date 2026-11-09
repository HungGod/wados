//! The updates against a fake bootc (canned `bootc status --json`, as v1's
//! tests and bootc 1.17 print it) and fake images.

use std::collections::HashMap;

use serde_json::json;
use wad_model::JobStatus;
use wad_proto::MachineInfo;

use super::*;
use crate::api_phase8::Updates as _;

// ------------------------------------------------------------ parsing

/// v1's canned status (its tests'): a staged update, no version on the booted one.
const V1: &str = r#"{"apiVersion":"org.containers.bootc/v1","kind":"BootcHost",
  "spec":{"image":{"image":"ghcr.io/hunggod/wadspaces-host-drivers:latest","transport":"registry"}},
  "status":{"staged":{"image":{"image":{"image":"ghcr.io/hunggod/wadspaces-host-drivers:latest","transport":"registry"},
              "version":"43.20261024.0","timestamp":"2026-10-24T01:00:00Z","imageDigest":"sha256:bbb"}},
            "booted":{"image":{"image":{"image":"ghcr.io/hunggod/wadspaces-host:latest","transport":"registry"},
              "imageDigest":"sha256:aaa"}},"rollback":null}}"#;

#[test]
fn what_bootc_says_v1s() {
    let h = parse_status(V1).unwrap();
    assert_eq!((h.image.as_str(), h.transport.as_str()), ("ghcr.io/hunggod/wadspaces-host-drivers:latest", "registry"));
    let booted = h.booted.unwrap();
    assert_eq!(booted.digest.as_deref(), Some("sha256:aaa"));
    // No version label: the digest, short.
    assert_eq!(booted.version, "aaa");
    let staged = h.staged.unwrap();
    assert_eq!((staged.version.as_str(), staged.digest.as_deref()), ("43.20261024.0", Some("sha256:bbb")));
    assert!(h.rollback.is_none() && !h.rollback_queued && h.cached.is_none());
    // A stick's copy: nothing booted from a registry.
    let stick = parse_status(
        r#"{"spec":{"image":{"image":"/var/lib/wadspaces/incoming:host","transport":"oci"}},"status":{"booted":null,"staged":null}}"#,
    )
    .unwrap();
    assert!(stick.booted.is_none() && stick.transport == "oci");
    // In a container: no image at all.
    let none = parse_status(
        r#"{"spec":{"bootOrder":"default","image":null},"status":{"booted":null,"rollbackQueued":false}}"#,
    )
    .unwrap();
    assert_eq!(none, Host::default());
    assert!(parse_status("").is_err());
}

#[test]
fn what_bootc_1_17_says() {
    let h = parse_status(&status(&Machine {
        booted: v("44.20261104.0", "a"),
        rollback: Some(v("44.20261028.0", "r")),
        cached: Some(v("44.20261105.0", "n")),
        ..Machine::default()
    }))
    .unwrap();
    assert_eq!(h.booted.unwrap().version, "44.20261104.0");
    assert_eq!(h.rollback.unwrap().version, "44.20261028.0");
    assert_eq!(h.cached.unwrap().digest.as_deref(), Some("sha256:n"));
}

#[test]
fn what_a_check_found() {
    let found = parse_check(
        "Update available for: docker://10.0.2.2:5077/wados-host:latest\n  Version: 44.20261105.2\n  Digest: sha256:fe84\nTotal new layers: 66    Size: 1.7 GB\n",
    )
    .unwrap();
    assert_eq!(found.version, "44.20261105.2");
    assert_eq!(found.image, "10.0.2.2:5077/wados-host:latest");
    assert_eq!(found.digest.as_deref(), Some("sha256:fe84"));
    assert_eq!(parse_check("No changes in: docker://ghcr.io/hunggod/wados-host:latest\n"), None);
    assert_eq!(
        parse_check("Update available for: x\n  Digest: sha256:abcdef1234567890\n").unwrap().version,
        "abcdef123456"
    );
}

#[test]
fn bootcs_progress_lines() {
    assert_eq!(
        progress_of(
            r#"{"type":"ProgressBytes","task":"pulling","description":"Pulling","id":"x","bytesCached":0,"bytes":50,"bytesTotal":200,"stepsCached":0,"steps":1,"stepsTotal":9,"subtasks":[]}"#
        ),
        Some(Progress::Bytes { done: 50, total: 200 })
    );
    assert_eq!(
        progress_of(r#"{"type":"ProgressSteps","task":"staging","steps":1,"stepsTotal":4,"subtasks":[]}"#),
        Some(Progress::Steps { task: "staging".into(), done: 1, total: 4 })
    );
    assert_eq!(progress_of(r#"{"type":"Start","version":"org.containers.bootc.progress/v1"}"#), None);
    assert_eq!(progress_of("Queued for next boot: ghcr.io/hunggod/wados-host:latest"), None);
}

#[test]
fn failures_in_their_words() {
    assert_eq!(
        failure("bootc", "warning: x\nerror: Upgrading: reading manifest latest: manifest unknown\n\n", Some(1)),
        "Upgrading: reading manifest latest: manifest unknown"
    );
    assert_eq!(failure("bootc", "", Some(2)), "bootc failed (exit 2)");
    assert!(offline("Creating importer: pinging container registry ghcr.io: dial tcp: lookup ghcr.io: no such host"));
    assert!(!offline("manifest unknown"));
    assert!(refused("reading manifest latest in ghcr.io/x/y: unauthorized: authentication required"));
}

#[test]
fn the_login_is_the_github_token_and_root_s_alone() {
    let j = auth_json("gho_abc");
    let v: Value = serde_json::from_str(&j).unwrap();
    let auth = v["auths"]["ghcr.io"]["auth"].as_str().unwrap();
    assert_eq!(base64::engine::general_purpose::STANDARD.decode(auth).unwrap(), b"wadspaces:gho_abc");
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("ostree/auth.json");
    write_private(&f, &j).unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn the_bases_published_names() {
    assert_eq!(published("localhost/wados-base:44"), "ghcr.io/hunggod/wados-base:44");
    assert_eq!(published("quay.io/x/y:1"), "quay.io/x/y:1");
}

// ------------------------------------------------------------ a fake bootc

fn v(version: &str, digest: &str) -> Version {
    Version {
        version: version.into(),
        image: "ghcr.io/hunggod/wados-host:latest".into(),
        digest: Some(format!("sha256:{digest}")),
    }
}

/// The machine as the fake bootc keeps it.
#[derive(Debug, Clone, Default)]
struct Machine {
    booted: Version,
    staged: Option<Version>,
    rollback: Option<Version>,
    rollback_queued: bool,
    cached: Option<Version>,
    /// What the registry has.
    remote: Option<Version>,
    /// What the registry answers with instead (once each), by verb.
    fail: HashMap<&'static str, String>,
    /// The registry wants a login.
    private: bool,
    /// A login is refused (a GitHub token without read:packages: ghcr says so even for a public image).
    login_refused: bool,
}

fn entry(v: &Version) -> Value {
    json!({"image": {"image": {"image": v.image, "transport": "registry"}, "version": v.version,
        "timestamp": "2026-11-05T00:00:00Z", "imageDigest": v.digest, "architecture": "amd64"},
        "cachedUpdate": null, "incompatible": false, "pinned": false, "store": "ostreeContainer"})
}

fn status(m: &Machine) -> String {
    let mut booted = entry(&m.booted);
    booted["cachedUpdate"] = m.cached.as_ref().map_or(Value::Null, |c| entry(c)["image"].clone());
    json!({"apiVersion": "org.containers.bootc/v1", "kind": "BootcHost", "metadata": {"name": "host"},
        "spec": {"bootOrder": "default", "image": {"image": m.booted.image, "transport": "registry"}},
        "status": {"booted": booted, "staged": m.staged.as_ref().map(entry), "rollback": m.rollback.as_ref().map(entry),
            "rollbackQueued": m.rollback_queued, "type": "bootcHost"}})
    .to_string()
}

#[derive(Default)]
struct FakeBootc {
    m: Mutex<Machine>,
    calls: Mutex<Vec<String>>,
    auth: PathBuf,
    /// How long a download takes.
    slow: Duration,
    not_here: bool,
}

#[async_trait]
impl Bootc for FakeBootc {
    fn here(&self) -> bool {
        !self.not_here
    }

    async fn run(&self, argv: &[&str], mut on_line: OnLine) -> Res<String> {
        let line = argv.join(" ");
        self.calls.lock().unwrap().push(line.clone());
        let verb = match argv {
            ["bootc", "status", "--json"] => "status",
            ["bootc", "upgrade", "--check"] => "check",
            a if a == UPGRADE => "upgrade",
            ["bootc", "rollback"] => "rollback",
            a if a == UNDEPLOY_STAGED => "undeploy",
            _ => return Err(format!("no such command: {line}")),
        };
        if let Some(e) = self.m.lock().unwrap().fail.remove(verb) {
            return Err(e);
        }
        let signed_in = self.auth.exists();
        let mut m = self.m.lock().unwrap().clone();
        if matches!(verb, "check" | "upgrade") && m.login_refused && signed_in {
            return Err("initializing source docker://ghcr.io/hunggod/wados-host:latest: Requesting bearer token: received unexpected HTTP status: 403 Forbidden".into());
        }
        if matches!(verb, "check" | "upgrade") && m.private && !signed_in {
            return Err(
                "reading manifest latest in ghcr.io/hunggod/wados-host: unauthorized: authentication required".into()
            );
        }
        match verb {
            "status" => return Ok(status(&m)),
            // As bootc 1.17: it says what it found, and keeps no cachedUpdate.
            "check" => {
                return Ok(match m.remote.clone().filter(|r| r.digest != m.booted.digest) {
                    Some(r) => format!(
                        "Update available for: docker://{}\n  Version: {}\n  Digest: {}\nTotal new layers: 66    Size: 1.7 GB\n",
                        r.image,
                        r.version,
                        r.digest.unwrap_or_default()
                    ),
                    None => format!("No changes in: docker://{}\n", m.booted.image),
                });
            }
            "upgrade" => {
                let Some(r) = m.remote.clone().filter(|r| r.digest != m.booted.digest) else {
                    return Ok("No update available.\n".into());
                };
                on_line(r#"{"type":"Start","version":"org.containers.bootc.progress/v1"}"#);
                for done in [0, 600, 1200] {
                    on_line(&format!(
                        r#"{{"type":"ProgressBytes","task":"pulling","bytes":{done},"bytesTotal":1200,"steps":1,"stepsTotal":3}}"#
                    ));
                    tokio::time::sleep(self.slow).await;
                }
                on_line(r#"{"type":"ProgressSteps","task":"staging","steps":1,"stepsTotal":2}"#);
                on_line("Queued for next boot: ghcr.io/hunggod/wados-host:latest");
                m.staged = Some(r);
                m.rollback_queued = false;
            }
            "rollback" => {
                // A staged one is dropped; the order swaps.
                m.staged = None;
                m.rollback_queued = !m.rollback_queued;
            }
            "undeploy" => m.staged = None,
            _ => unreachable!(),
        }
        *self.m.lock().unwrap() = m;
        Ok(String::new())
    }
}

#[derive(Default)]
struct FakeImages {
    /// What's here, by name: (id, size, created).
    here: Mutex<HashMap<String, (String, u64, f64)>>,
    /// What the registry has, by reference: (id, size).
    remote: Mutex<HashMap<String, (String, u64)>>,
    token: Option<String>,
    pulls: Mutex<Vec<(String, bool)>>,
    /// Images this machine built (a WadOS wadspace's): reference → (id, version).
    built: Mutex<Vec<(String, String, String)>>,
}

#[async_trait]
impl Images for FakeImages {
    async fn info(&self, image: &str) -> Res<Option<ImageInfo>> {
        Ok(self.here.lock().unwrap().get(image).map(|(id, size, created)| ImageInfo {
            id: id.clone(),
            created: *created,
            size: *size,
            labels: Default::default(),
            tags: vec![image.into()],
            parent: String::new(),
        }))
    }

    async fn pull_newer(&self, reference: &str, auth: Option<String>, mut on_bytes: OnBytes) -> Res<String> {
        self.pulls.lock().unwrap().push((reference.into(), auth.is_some()));
        let (id, size) =
            self.remote.lock().unwrap().get(reference).cloned().ok_or(format!("{reference}: manifest unknown"))?;
        on_bytes(size / 2, size);
        on_bytes(size, size);
        self.here.lock().unwrap().insert(reference.into(), (id.clone(), size, 1.0));
        Ok(id)
    }

    async fn tag(&self, image: &str, target: &str) -> Res<()> {
        let mut here = self.here.lock().unwrap();
        let i = here.values().find(|(id, ..)| id == image).cloned().ok_or("no such image")?;
        here.insert(target.into(), i);
        Ok(())
    }

    async fn token(&self) -> Option<String> {
        self.token.clone()
    }

    async fn all(&self) -> Res<Vec<ImageInfo>> {
        let mut out: Vec<ImageInfo> = self
            .here
            .lock()
            .unwrap()
            .iter()
            .map(|(tag, (id, size, created))| ImageInfo {
                id: id.clone(),
                created: *created,
                size: *size,
                labels: Default::default(),
                tags: vec![tag.clone()],
                parent: String::new(),
            })
            .collect();
        for (reference, id, version) in self.built.lock().unwrap().iter() {
            out.push(ImageInfo {
                id: id.clone(),
                created: 1.0,
                size: 100,
                labels: [("org.opencontainers.image.version".to_string(), serde_json::json!(version))]
                    .into_iter()
                    .collect(),
                tags: vec![reference.clone()],
                parent: String::new(),
            });
        }
        Ok(out)
    }
}

struct Up {
    _dir: tempfile::TempDir,
    u: HostUpdates,
    bootc: Arc<FakeBootc>,
    images: Arc<FakeImages>,
    jobs: Arc<Jobs>,
    bus: Bus,
    changed: Arc<std::sync::atomic::AtomicUsize>,
}

fn machine() -> Machine {
    Machine {
        booted: v("44.20261104.0", "booted"),
        rollback: Some(v("44.20261028.0", "before")),
        remote: Some(v("44.20261105.0", "next")),
        ..Machine::default()
    }
}

fn up_with(m: Machine, images: FakeImages, f: impl FnOnce(&mut FakeBootc, &mut Settings)) -> Up {
    let dir = tempfile::tempdir().unwrap();
    let bus = Bus::new(MachineInfo {
        id: "m".into(),
        name: "M".into(),
        version: "0".into(),
        profile: wad_proto::Profile::System,
    });
    let jobs = Jobs::new(bus.clone(), dir.path());
    let mut settings = Settings {
        state_dir: dir.path().join("state"),
        auth_file: dir.path().join("run/ostree/auth.json"),
        // Not the machine's cadence: these tests drive `due()` themselves,
        // and a scheduler running behind them would check again mid-test.
        every: Duration::from_secs(3600),
        bases_every: Duration::from_secs(3600),
        // These tests drive `due()` themselves: a scheduler starting a
        // check of its own would race them (an hour's the same as never).
        first: Duration::from_secs(3600),
        tick: Duration::from_millis(5),
        schedule: true,
        bases: vec![
            BaseImage::new("Fedora 44 base", "localhost/wados-base:44"),
            BaseImage::new("Stream sidecar", "localhost/wados-stream:44"),
        ],
    };
    let mut bootc = FakeBootc { m: Mutex::new(m), auth: settings.auth_file.clone(), ..FakeBootc::default() };
    f(&mut bootc, &mut settings);
    let (bootc, images) = (Arc::new(bootc), Arc::new(images));
    let changed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = changed.clone();
    let hooks = Hooks {
        bases_changed: Some(Box::new(move || {
            c.fetch_add(1, Ordering::Relaxed);
        })),
    };
    let u = HostUpdates::new(settings, bootc.clone(), images.clone(), jobs.clone(), bus.clone(), hooks);
    Up { _dir: dir, u, bootc, images, jobs, bus, changed }
}

fn up() -> Up {
    up_with(machine(), FakeImages::default(), |_, _| {})
}

impl Up {
    async fn ended(&self, job: &JobInfo) -> JobInfo {
        for _ in 0..500 {
            let j = self.jobs.info(&job.job.id).unwrap();
            if crate::jobs::finished(j.job.status) {
                return j;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("{} didn't end", job.job.id)
    }

    /// Until `f` holds of the status (a job chained after another).
    async fn until(&self, f: impl Fn(&proto::Updates) -> bool) -> proto::Updates {
        for _ in 0..500 {
            let u = self.u.status().await.unwrap();
            if f(&u) {
                return u;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("it never came: {:?}", self.u.status().await.unwrap())
    }

    fn calls(&self) -> Vec<String> {
        self.bootc.calls.lock().unwrap().clone()
    }

    fn notices(&self, rx: &mut tokio::sync::broadcast::Receiver<Event>) -> Vec<(String, Option<String>)> {
        let mut out = vec![];
        while let Ok(e) = rx.try_recv() {
            if let Event::Notice { title, action, .. } = e {
                out.push((title, action.map(|a| a.verb)));
            }
        }
        out
    }
}

// ------------------------------------------------------------ the host

#[tokio::test]
async fn a_check_finds_one_downloads_it_and_stages_it() {
    let up = up();
    let (_, mut rx) = up.bus.subscribe();
    let u = up.u.status().await.unwrap();
    assert!(u.online && u.available.is_none() && u.staged.is_none());
    assert_eq!(u.booted.version, "44.20261104.0");
    assert_eq!(u.rollback.as_ref().unwrap().version, "44.20261028.0");

    let job = up.u.check().await.unwrap();
    assert_eq!(job.job.kind, JobKind::Update);
    assert_eq!(job.job.stages.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["check"]);
    assert_eq!(up.ended(&job).await.job.status, JobStatus::Done);
    // Found, so downloaded and staged straight after (an apply job of its own).
    let u = up.until(|u| u.staged.is_some() && u.job.is_none()).await;
    assert_eq!(u.staged.unwrap().version, "44.20261105.0");
    assert!(u.available.is_none() && u.progress.is_none() && u.checked_at.is_some());
    let calls = up.calls();
    assert!(calls.contains(&"bootc upgrade --check".into()) && calls.contains(&UPGRADE.join(" ")));
    let apply = up.jobs.list().into_iter().find(|j| j.job.title == "Updating WadSpaces to 44.20261105.0").unwrap();
    assert_eq!(apply.job.status, JobStatus::Done);
    assert_eq!(apply.job.total_bytes, 1200, "bootc's progress, in bytes");
    // Ready: a notice to restart, the shell's verb.
    let n = up.notices(&mut rx);
    assert!(n.contains(&("WadSpaces 44.20261105.0 is ready".into(), Some("power.restart".into()))), "{n:?}");
    // Up to date now: nothing more to apply.
    assert_eq!(up.u.apply().await.unwrap_err().code, ErrorCode::Conflict);
}

#[tokio::test]
async fn one_at_a_time_and_a_check_that_finds_nothing() {
    let up =
        up_with(Machine { remote: Some(v("44.20261104.0", "booted")), ..machine() }, FakeImages::default(), |b, _| {
            b.slow = Duration::from_millis(20)
        });
    let job = up.u.check().await.unwrap();
    assert_eq!(up.u.check().await.unwrap_err().code, ErrorCode::Conflict, "one at a time");
    up.ended(&job).await;
    let u = up.until(|u| u.job.is_none()).await;
    assert!(u.available.is_none() && u.staged.is_none() && u.error.is_none() && !u.checking);
    assert!(!up.calls().iter().any(|c| c.contains("progress-fd")), "nothing to download");
}

#[tokio::test]
async fn offline_and_failures_are_said() {
    let mut m = machine();
    m.fail.insert("check", "pinging container registry ghcr.io: dial tcp: lookup ghcr.io: no such host".into());
    let up = up_with(m, FakeImages::default(), |_, _| {});
    let job = up.u.check().await.unwrap();
    let j = up.ended(&job).await;
    assert_eq!(j.job.status, JobStatus::Error);
    let u = up.u.status().await.unwrap();
    assert!(!u.online, "the registry wasn't reached");
    assert!(u.error.unwrap().contains("no such host"));
    // Back online: the next check clears it.
    ended_ok(&up).await;
    let u = up.until(|u| u.staged.is_some()).await;
    assert!(u.online && u.error.is_none());
}

async fn ended_ok(up: &Up) {
    let job = up.u.check().await.unwrap();
    assert_eq!(up.ended(&job).await.job.status, JobStatus::Done);
    // A check that finds one downloads and stages it straight after: that
    // apply is under way, and the next check would be the second one at once.
    up.until(|u| u.job.is_none()).await;
}

#[tokio::test]
async fn a_private_image_signs_in_with_the_github_token_once_refused() {
    let images = FakeImages { token: Some("gho_secret".into()), ..FakeImages::default() };
    let up = up_with(Machine { private: true, ..machine() }, images, |_, _| {});
    ended_ok(&up).await;
    let auth = std::fs::read_to_string(&up.bootc.auth).unwrap();
    assert_eq!(auth, auth_json("gho_secret"));
    let checks = up.calls().iter().filter(|c| *c == "bootc upgrade --check").count();
    assert_eq!(checks, 2, "refused, then signed in");

    // A public one never writes it.
    let up = up_with(machine(), FakeImages { token: Some("gho_secret".into()), ..FakeImages::default() }, |_, _| {});
    ended_ok(&up).await;
    assert!(!up.bootc.auth.exists());
}

#[tokio::test]
async fn dont_apply_it_undeploys_and_the_next_check_leaves_it() {
    let up = up();
    let (_, mut rx) = up.bus.subscribe();
    ended_ok(&up).await;
    up.until(|u| u.staged.is_some() && u.job.is_none()).await;
    let u = up.u.cancel().await.unwrap();
    assert!(u.staged.is_none());
    assert!(up.calls().contains(&UNDEPLOY_STAGED.join(" ")));
    // What's on the registry is still there to download, by hand.
    assert_eq!(u.available.as_ref().unwrap().version, "44.20261105.0");
    assert!(up.notices(&mut rx).contains(&("WadSpaces 44.20261105.0 won't be applied".into(), None)));
    assert_eq!(up.u.cancel().await.unwrap_err().code, ErrorCode::Conflict, "nothing staged");
    // The next check doesn't fetch it again.
    ended_ok(&up).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let u = up.u.status().await.unwrap();
    assert!(u.staged.is_none() && u.available.is_some());
    // Download does.
    let job = up.u.apply().await.unwrap();
    up.ended(&job).await;
    assert!(up.u.status().await.unwrap().staged.is_some());
}

#[tokio::test]
async fn rolling_back_stages_the_one_before_and_stops_a_download() {
    let up = up_with(machine(), FakeImages::default(), |b, _| b.slow = Duration::from_millis(50));
    let (_, mut rx) = up.bus.subscribe();
    let check = up.u.check().await.unwrap();
    up.ended(&check).await;
    let downloading = up.until(|u| u.job.is_some()).await;
    let u = up.u.rollback().await.unwrap();
    assert_eq!(up.jobs.info(downloading.job.as_ref().unwrap()).unwrap().job.status, JobStatus::Cancelled);
    assert_eq!(u.staged.as_ref().unwrap().version, "44.20261028.0");
    assert_eq!(u.staged, u.rollback, "the page shows it as Staged");
    assert!(u.available.is_none());
    let n = up.notices(&mut rx);
    assert!(n.contains(&("WadSpaces 44.20261028.0 is staged".into(), Some("power.restart".into()))), "{n:?}");
    assert_eq!(up.u.rollback().await.unwrap_err().code, ErrorCode::Conflict, "staged already");
    // Don't apply it: the order swaps back.
    let u = up.u.cancel().await.unwrap();
    assert!(u.staged.is_none());
    assert_eq!(up.calls().iter().filter(|c| *c == "bootc rollback").count(), 2);
    // Nothing to go back to.
    let none = up_with(Machine { rollback: None, ..machine() }, FakeImages::default(), |_, _| {});
    assert_eq!(none.u.rollback().await.unwrap_err().message, "There's nothing to go back to");
}

#[tokio::test]
async fn a_laptop_doesnt_update_through_bootc() {
    let up = up_with(machine(), FakeImages::default(), |b, _| b.not_here = true);
    let u = up.u.status().await.unwrap();
    assert!(!u.online);
    assert_eq!(u.error.as_deref(), Some("This machine doesn't update through bootc"));
    assert_eq!(up.u.check().await.unwrap_err().code, ErrorCode::Conflict);
    assert!(up.calls().is_empty());
}

#[tokio::test]
async fn the_schedule_checks_when_its_due_and_remembers() {
    // On the real cadence, for the one test that's about the schedule.
    let up = up_with(machine(), FakeImages::default(), |_, s| {
        s.every = EVERY;
        s.bases_every = BASES_EVERY;
        s.schedule = true;
    });
    up.u.due().await;
    up.until(|u| u.staged.is_some() && u.job.is_none()).await;
    let checks = || up.calls().iter().filter(|c| *c == "bootc upgrade --check").count();
    assert_eq!(checks(), 1);
    // Not due again for hours.
    up.u.due().await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(checks(), 1);
    // Remembered after a restart.
    let saved: Saved =
        serde_json::from_slice(&std::fs::read(up._dir.path().join("state/updates.json")).unwrap()).unwrap();
    assert!(saved.checked_at.is_some() && saved.bases_at.is_some());
}

// ------------------------------------------------------------ where from

#[tokio::test]
async fn this_machines_own_build_is_an_update_from_here() {
    // The WadOS wadspace's host/build.sh image, in podman's store.
    let images = FakeImages::default();
    {
        let mut built = images.built.lock().unwrap();
        built.push(("localhost/wados-host:latest".into(), "sha256:built".into(), "44.20261106.0".into()));
        built.push(("localhost/wados-host:44.20261101.0".into(), "sha256:old".into(), "44.20261101.0".into()));
    }
    let up = up_with(Machine { remote: Some(v("44.20261104.0", "booted")), ..machine() }, images, |_, _| {});
    // Chosen: this machine's builds.
    let u = up.u.source(UpdateSource::Local).await.unwrap();
    assert_eq!(u.source, UpdateSource::Local);
    // The newest by version is offered, and no registry is asked.
    let u = up.u.status().await.unwrap();
    assert_eq!(u.local.as_ref().map(|l| l.version.as_str()), Some("44.20261106.0"));
    let job = up.u.check().await.unwrap();
    assert_eq!(up.ended(&job).await.job.status, JobStatus::Done);
    let u = up.until(|u| u.available.is_some()).await;
    assert_eq!(u.available.map(|a| a.version), Some("44.20261106.0".into()));
    assert!(!up.calls().iter().any(|c| c.contains("--check")), "no registry: {:?}", up.calls());
    // Found, so switched to in podman's store straight after (a check
    // downloads what it finds): nothing from a registry.
    up.until(|u| u.job.is_none()).await;
    assert!(
        up.calls().iter().any(|c| c == "bootc switch --transport containers-storage localhost/wados-host@sha256:built"),
        "{:?}",
        up.calls()
    );
}

#[tokio::test]
async fn a_machines_build_that_is_what_it_boots_isnt_an_update() {
    let images = FakeImages::default();
    images.built.lock().unwrap().push((
        "localhost/wados-host:latest".into(),
        "sha256:same".into(),
        "44.20261104.0".into(),
    ));
    let up = up_with(Machine { remote: Some(v("44.20261104.0", "booted")), ..machine() }, images, |_, _| {});
    up.u.source(UpdateSource::Local).await.unwrap();
    let job = up.u.check().await.unwrap();
    up.ended(&job).await;
    assert!(up.until(|u| u.job.is_none()).await.available.is_none());
}

#[tokio::test]
async fn versions_compare_by_their_parts() {
    use std::cmp::Ordering::*;
    assert_eq!(compare_versions("44.20261105.0", "44.20261104.0"), Greater);
    assert_eq!(compare_versions("44.20261105.10", "44.20261105.9"), Greater);
    assert_eq!(compare_versions("44.20261105.0", "44.20261105.0"), Equal);
    assert_eq!(compare_versions("45.20260101.0", "44.20261231.99"), Greater);
    assert_eq!(compare_versions("44.20261105", "44.20261105.0"), Less);
}

// ------------------------------------------------------------ the bases

#[tokio::test]
async fn the_bases_listed_and_refreshed() {
    let images = FakeImages::default();
    images.here.lock().unwrap().insert("localhost/wados-base:44".into(), ("sha256:old".into(), 1_450_000_000, 1000.0));
    images.remote.lock().unwrap().insert("ghcr.io/hunggod/wados-base:44".into(), ("sha256:new".into(), 1_500_000_000));
    let up = up_with(machine(), images, |_, _| {});
    let b = up.u.bases().await.unwrap();
    assert_eq!(b.bases.len(), 2);
    assert_eq!(b.bases[0].image, "ghcr.io/hunggod/wados-base:44");
    assert_eq!(
        (b.bases[0].size, b.bases[0].refreshed_at, b.bases[0].missing),
        (Some(1_450_000_000), Some(1_000_000), false)
    );
    assert!(b.bases[1].missing && b.bases[1].size.is_none());
    assert!(b.next_refresh_at.is_some());

    let job = up.u.refresh_bases().await.unwrap();
    assert_eq!(job.job.kind, JobKind::Pull);
    assert_eq!(
        job.job.stages.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(),
        ["Fedora 44 base", "Stream sidecar"]
    );
    assert_eq!(up.u.refresh_bases().await.unwrap_err().code, ErrorCode::Conflict, "one at a time");
    let j = up.ended(&job).await;
    // The sidecar isn't published: that stage failed, the base's didn't.
    assert_eq!(j.job.status, JobStatus::Error);
    assert_eq!(j.job.stages[0].state, PartState::Done);
    assert_eq!(j.job.stages[1].state, PartState::Error);
    assert!(j.job.error.unwrap().starts_with("Stream sidecar: "));
    // The base is the new one, under the name wadspaces use.
    let b = up.u.bases().await.unwrap();
    assert_eq!(b.bases[0].size, Some(1_500_000_000));
    assert!(b.bases[0].refreshed_at.unwrap() > 1_000_000);
    assert_eq!(up.images.here.lock().unwrap()["localhost/wados-base:44"].0, "sha256:new");
    assert_eq!(up.changed.load(Ordering::Relaxed), 1, "wadspaces on the old base are told");
    // No token: pulled without a login.
    assert_eq!(up.images.pulls.lock().unwrap()[0], ("ghcr.io/hunggod/wados-base:44".into(), false));
}

#[test]
fn the_nvidia_image_is_beside_the_plain_one() {
    assert_eq!(
        nvidia_image("ghcr.io/hunggod/wados-host:latest").as_deref(),
        Some("ghcr.io/hunggod/wados-host-nvidia:latest")
    );
    // On it already, or not one of ours: nothing to move to.
    assert_eq!(nvidia_image("ghcr.io/hunggod/wados-host-nvidia:latest"), None);
    assert_eq!(nvidia_image("quay.io/fedora/fedora-bootc:44"), None);
}

/// The machine's case (phase 13): the image was private, so a check signed in; then it was made
/// public, and the GitHub token can't read packages, so ghcr refuses the login even for it. Every
/// check starts without a login, and one that didn't help isn't left behind.
#[tokio::test]
async fn a_login_that_doesnt_help_is_never_kept() {
    let images = FakeImages { token: Some("gho_no_packages".into()), ..FakeImages::default() };
    let up = up_with(Machine { private: true, ..machine() }, images, |_, _| {});
    ended_ok(&up).await;
    assert!(up.bootc.auth.exists(), "signed in while it was private");

    // Public now, and the token can't read packages.
    {
        let mut m = up.bootc.m.lock().unwrap();
        m.private = false;
        m.login_refused = true;
    }
    ended_ok(&up).await;
    assert!(!up.bootc.auth.exists(), "a public image is checked without a login");

    // Private, and the login refused: it fails, and the login doesn't stay.
    up.bootc.m.lock().unwrap().private = true;
    let job = up.u.check().await.unwrap();
    assert_eq!(up.ended(&job).await.job.status, JobStatus::Error);
    assert!(!up.bootc.auth.exists(), "a refused login isn't kept");
}
