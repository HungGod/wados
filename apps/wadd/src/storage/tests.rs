//! The disk measured, and Clean Up, against a fake podman.

use wad_model::{Built, JobStatus};
use wad_podman::{DfImage, DfVolume};
use wad_proto::MachineInfo;

use super::*;
use crate::api_phase8::Storage as _;

const MB: u64 = 1_000_000;
const BASE: &str = "localhost/wados-base:44";
const STREAM: &str = "localhost/wados-stream:44";

fn image(id: &str, tags: &[&str], size: u64, labels: &[(&str, &str)]) -> ImageInfo {
    ImageInfo {
        id: id.into(),
        created: 0.0,
        size,
        labels: labels.iter().map(|(k, v)| (k.to_string(), Value::String(v.to_string()))).collect(),
        tags: tags.iter().map(|t| t.to_string()).collect(),
        parent: String::new(),
    }
}

fn set(id: &str, key: &str, size: u64) -> ImageInfo {
    image(id, &[&format!("localhost/wados-set-{key}:latest")], size, &[(wad_gen::labels::LAYER, &format!("set:{key}"))])
}

fn df(volumes: &[(&str, u64)]) -> Df {
    Df {
        volumes: volumes.iter().map(|(n, s)| DfVolume { name: n.to_string(), size: *s, links: 0 }).collect(),
        images: vec![DfImage::default()],
        ..Df::default()
    }
}

#[test]
fn the_disk_as_bases_wadspaces_other_and_free() {
    let images = vec![
        image("b", &[BASE], 1_450 * MB, &[]),
        image("s", &[STREAM], 865 * MB, &[]),
        set("x", "studio", 2_450 * MB),
        set("y", "retro", 1_950 * MB),
    ];
    let bases = vec![BASE.to_string(), STREAM.to_string()];
    let ws = vec![
        ("studio".to_string(), Some("localhost/wados-set-studio:latest".to_string()), Built::Built),
        // Shares studio's image: counted once in all.
        ("copy".to_string(), Some("localhost/wados-set-studio:latest".to_string()), Built::Update),
        ("retro".to_string(), Some("localhost/wados-set-retro:latest".to_string()), Built::Built),
        // On the base itself: nothing of its own but its home.
        ("wados".to_string(), Some(BASE.to_string()), Built::Built),
        ("new".to_string(), None, Built::Never),
    ];
    let d = df(&[("wad-studio-config", 300 * MB), ("wad-wados-config", 20 * MB), ("other", 9_999 * MB)]);
    let s = measure((100_000 * MB, 60_000 * MB), &d, &images, &bases, &ws);
    assert_eq!((s.total, s.free), (100_000 * MB, 60_000 * MB));
    assert_eq!(s.bases, 2_315 * MB);
    assert_eq!(s.wadspaces, (1_000 + 500 + 300 + 20) * MB);
    assert_eq!(s.other, 40_000 * MB - s.bases - s.wadspaces);
    assert_eq!(s.used(), s.bases + s.wadspaces + s.other);
    let per: Vec<_> = s.per_wadspace.iter().map(|w| (w.id.as_str(), w.bytes / MB, w.built)).collect();
    assert_eq!(
        per,
        [
            ("studio", 1_300, Built::Built),
            ("copy", 1_000, Built::Update),
            ("retro", 500, Built::Built),
            ("wados", 20, Built::Built),
            ("new", 0, Built::Never),
        ]
    );
    // A nearly empty disk can't hold more than it uses.
    let tiny = measure((3_000 * MB, 2_000 * MB), &d, &images, &bases, &ws);
    assert_eq!(tiny.bases + tiny.wadspaces + tiny.other, 1_000 * MB);
}

#[test]
fn a_cleanup_says_what_it_freed() {
    assert_eq!(freed_words(0, 0), ("Nothing to clean up".into(), "There are no leftovers on this machine.", "check"));
    let (t, b, i) = freed_words(6_500 * MB, 3);
    assert_eq!(
        (t.as_str(), b, i),
        ("Freed 6.5 GB", "Old build layers, stopped containers and caches are gone.", "sparkles")
    );
}

// ------------------------------------------------------------ a fake podman

#[derive(Default)]
struct FakeDisk {
    images: Mutex<Vec<ImageInfo>>,
    dangling: Mutex<Vec<(String, u64)>>,
    stopped: Mutex<Vec<(String, u64, bool)>>,
    leftovers: Mutex<Vec<(String, u64)>>,
    free: Mutex<u64>,
}

#[async_trait]
impl Disk for FakeDisk {
    fn space(&self, _: &Path) -> Res<(u64, u64)> {
        Ok((512_000 * MB, *self.free.lock().unwrap()))
    }
    async fn df(&self) -> Res<Df> {
        Ok(df(&[]))
    }
    async fn images(&self) -> Res<Vec<ImageInfo>> {
        Ok(self.images.lock().unwrap().clone())
    }
    async fn remove_image(&self, id: &str) -> Res<bool> {
        let mut images = self.images.lock().unwrap();
        let Some(at) = images.iter().position(|i| i.id == id) else { return Ok(false) };
        *self.free.lock().unwrap() += images[at].size;
        images.remove(at);
        Ok(true)
    }
    async fn prune_images(&self, external: bool) -> Res<Vec<(String, u64)>> {
        let list = if external { &self.leftovers } else { &self.dangling };
        Ok(std::mem::take(&mut *list.lock().unwrap()))
    }
    async fn prune_containers(&self, keep_label: &str) -> Res<Vec<(String, u64)>> {
        assert_eq!(keep_label, wad_gen::labels::WADSPACE);
        let mut c = self.stopped.lock().unwrap();
        let (gone, kept): (Vec<_>, Vec<_>) = c.drain(..).partition(|(_, _, wadspace)| !wadspace);
        *c = kept;
        Ok(gone.into_iter().map(|(id, s, _)| (id, s)).collect())
    }
}

#[tokio::test]
async fn clean_up_removes_what_belongs_to_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let bus = Bus::new(MachineInfo {
        id: "m-test".into(),
        name: "Test".into(),
        version: "0".into(),
        profile: wad_proto::Profile::User,
    });
    let fake = Arc::new(crate::fake::Fake::new());
    fake.add_image(BASE, &[(wad_gen::labels::ABI, "fedora44.1")]);
    let docs = Arc::new(Docs::open(crate::store::Store::new(&state).unwrap(), bus.clone(), "m-test").unwrap());
    let jobs = Jobs::new(bus.clone(), &state);
    let packages = crate::packages::Packages::new(
        fake.clone(),
        crate::packages::Settings {
            pull: false,
            index_url: String::new(),
            index_max_age: Duration::from_secs(60),
            state_dir: state.clone(),
            min_free_gb: 1,
        },
    );
    let builds = Builds::new(
        fake.clone(),
        packages,
        jobs.clone(),
        docs.clone(),
        bus.clone(),
        &state,
        crate::builds::Settings { min_free_gb: 1, copy_max_bytes: 1 << 20 },
        None,
    );
    builds.refresh_base().await;
    let disk = Arc::new(FakeDisk::default());
    *disk.images.lock().unwrap() = vec![
        image("b", &[BASE], 1_450 * MB, &[]),
        image("s", &[STREAM], 865 * MB, &[]),
        // A set nothing has any more.
        set("old", "gone", 2_000 * MB),
        // Someone else's image, and a package builds restack from: kept.
        image("f", &["docker.io/library/fedora:44"], 200 * MB, &[]),
        image("p", &["localhost/wados-pkg-nodejs:latest"], 90 * MB, &[(wad_gen::labels::PKG, "nodejs")]),
    ];
    *disk.dangling.lock().unwrap() = vec![("sha256:d".into(), 300 * MB)];
    *disk.stopped.lock().unwrap() = vec![("c1".into(), 10 * MB, false), ("ws".into(), 5 * MB, true)];
    *disk.leftovers.lock().unwrap() = vec![("sha256:l".into(), 50 * MB)];
    *disk.free.lock().unwrap() = 100_000 * MB;
    let st = MachineStorage::new(
        Settings { disk: "/var".into(), bases: vec![BASE.into(), STREAM.into()] },
        disk.clone(),
        docs.clone(),
        builds.clone(),
        jobs.clone(),
        bus.clone(),
    );
    let s = st.storage().await.unwrap();
    assert_eq!(s.bases, 2_315 * MB);
    let starter = docs.live()[0].id.clone();
    assert_eq!(s.per_wadspace.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(), [starter.as_str()]);

    let (_, mut rx) = bus.subscribe();
    let job = st.cleanup().await.unwrap();
    assert_eq!(job.job.kind, JobKind::Cleanup);
    assert_eq!(st.cleanup().await.unwrap_err().code, wad_proto::ErrorCode::Conflict, "one at a time");
    let j = loop {
        let j = jobs.info(&job.job.id).unwrap();
        if crate::jobs::finished(j.job.status) {
            break j;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_eq!(j.job.status, JobStatus::Done, "{:?}", j.job.error);
    let left: Vec<_> = disk.images.lock().unwrap().iter().map(|i| i.id.clone()).collect();
    assert_eq!(left, ["b", "s", "f", "p"], "only the set nothing has");
    assert_eq!(disk.stopped.lock().unwrap().len(), 1, "a wadspace's container stays");
    assert!(disk.dangling.lock().unwrap().is_empty() && disk.leftovers.lock().unwrap().is_empty());
    let mut notice = None;
    while let Ok(e) = rx.try_recv() {
        if let Event::Notice { title, .. } = e {
            notice = Some(title);
        }
    }
    // The space it made (the set's), the fake's statvfs says.
    assert_eq!(notice.as_deref(), Some("Freed 2.0 GB"));
    // Again: nothing left.
    let job = st.cleanup().await.unwrap();
    loop {
        if crate::jobs::finished(jobs.info(&job.job.id).unwrap().job.status) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let mut notice = None;
    while let Ok(e) = rx.try_recv() {
        if let Event::Notice { title, .. } = e {
            notice = Some(title);
        }
    }
    assert_eq!(notice.as_deref(), Some("Nothing to clean up"));
}
