//! A fake machine, and a mock of Cloudflare's API: nothing here touches a
//! disk or reaches Cloudflare.
use std::path::PathBuf;

use serde_json::json;
use wad_model::JobStatus;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::api_phase9::Installer;
use crate::events::Bus;

const KEYS: &str = r#"
[cloudflare.tunnel]
dns_edit_api_token = "dns-key"
tunnel_creator_api_token = "tunnel-key"
[cloudflare.turn]
token_id = "turn-id"
api_token = "turn-key"
"#;

struct Fake {
    /// What the job's folder held when the script started: name → bytes.
    seen: Mutex<Vec<(String, Vec<u8>)>>,
    ran: Mutex<u32>,
    result: &'static str,
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else {
            out.push((p.strip_prefix(base).unwrap().display().to_string(), std::fs::read(&p).unwrap()));
        }
    }
}

#[async_trait]
impl Hands for Fake {
    async fn disks(&self) -> Result<Vec<InstallDisk>, String> {
        Ok(vec![disk()])
    }
    async fn hash(&self, password: &str) -> Result<String, String> {
        assert_eq!(password, "account-password");
        Ok("$6$salt$hash".into())
    }
    async fn run(&self, job: &Path, on_line: &mut OnLine<'_>) -> Result<(), String> {
        *self.ran.lock().unwrap() += 1;
        walk(job, job, &mut self.seen.lock().unwrap());
        on_line("STEP 1/7: Checking the disk");
        on_line("/dev/nvme0n1: 512G");
        on_line("STEP 2/7: Preparing the system image");
        std::fs::write(job.join("result"), self.result).unwrap();
        Ok(())
    }
}

fn disk() -> InstallDisk {
    InstallDisk {
        by_id: "/dev/disk/by-id/nvme-x".into(),
        name: "nvme0n1".into(),
        model: "SSD".into(),
        serial: "S1".into(),
        size: 512,
        removable: false,
        holds: None,
    }
}

fn request() -> InstallRequest {
    InstallRequest {
        name: "studio".into(),
        username: "sam".into(),
        password: "account-password".into(),
        by_id: "/dev/disk/by-id/nvme-x".into(),
        size: 512,
        serial: "S1".into(),
        disk_password: Some("disk-password".into()),
    }
}

struct Rig {
    state: PathBuf,
    run: PathBuf,
    jobs: Arc<Jobs>,
    fake: Arc<Fake>,
    install: MachineInstall,
}

/// What host/stick-files writes: the web app's keys alone.
const FIREBASE: &str = "api_key = \"web-key\"\nproject_id = \"wad-test\"\n";

fn rig(name: &str, api: Option<&str>, keys: bool, result: &'static str) -> Rig {
    let base = std::env::temp_dir().join(format!("wadd-install-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (state, run) = (base.join("state"), base.join("run"));
    std::fs::create_dir_all(state.join("installer")).unwrap();
    let mut file = "[installer]\ntarget = \"ghcr.io/hunggod/wados-host:latest\"\n".to_string();
    if let Some(a) = api {
        file.push_str(&format!("api = \"{a}\"\n"));
    }
    std::fs::write(state.join("installer/installer.toml"), file).unwrap();
    if keys {
        std::fs::write(state.join("installer/cloudflare.toml"), KEYS).unwrap();
        std::fs::write(state.join("installer/firebase.toml"), FIREBASE).unwrap();
    }
    let bus = Bus::new(wad_proto::MachineInfo {
        id: "m".into(),
        name: "M".into(),
        version: "0".into(),
        profile: wad_proto::Profile::System,
    });
    let jobs = Jobs::new(bus, &state);
    let fake = Arc::new(Fake { seen: Mutex::default(), ran: Mutex::new(0), result });
    let install = MachineInstall::open(&state, &run, fake.clone(), jobs.clone()).unwrap();
    Rig { state, run, jobs, fake, install }
}

async fn ends(r: &Rig, id: &str) -> JobInfo {
    for _ in 0..200 {
        let j = r.jobs.info(id).unwrap();
        if crate::jobs::finished(j.job.status) {
            return j;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the job didn't end");
}

fn ok(result: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "result": result }))
}

async fn cloudflare(healthy: bool) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/zones"))
        .respond_with(ok(json!([{ "id": "z1", "account": { "id": "a1" } }])))
        .mount(&s)
        .await;
    let found = if healthy { json!([{ "id": "t1", "status": "healthy" }]) } else { json!([]) };
    Mock::given(method("GET")).and(path("/accounts/a1/cfd_tunnel")).respond_with(ok(found)).mount(&s).await;
    Mock::given(method("POST"))
        .and(path("/accounts/a1/cfd_tunnel"))
        .respond_with(ok(json!({ "id": "t1", "token": "machine-token" })))
        .mount(&s)
        .await;
    Mock::given(method("PUT"))
        .and(path("/accounts/a1/cfd_tunnel/t1/configurations"))
        .respond_with(ok(json!({})))
        .mount(&s)
        .await;
    Mock::given(method("GET")).and(path("/zones/z1/dns_records")).respond_with(ok(json!([]))).mount(&s).await;
    Mock::given(method("POST")).and(path("/zones/z1/dns_records")).respond_with(ok(json!({}))).mount(&s).await;
    s
}

fn file<'a>(seen: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a str> {
    seen.iter().find(|(n, _)| n == name).map(|(_, b)| std::str::from_utf8(b).unwrap())
}

#[tokio::test]
async fn the_machine_gets_its_files_and_never_the_sticks_keys() {
    let cf = cloudflare(false).await;
    let r = rig("files", Some(&cf.uri()), true, "ok\n");
    let job = r.install.start(request()).await.unwrap();
    let j = ends(&r, &job.job.id).await;
    assert_eq!(j.job.status, JobStatus::Done, "{:?}", j.job.error);

    let seen = r.fake.seen.lock().unwrap();
    assert_eq!(file(&seen, "passphrase"), Some("disk-password"));
    assert!(file(&seen, "request").unwrap().contains("ENCRYPT=1\n"));
    assert!(file(&seen, "request").unwrap().contains("TARGET=ghcr.io/hunggod/wados-host:latest\n"));
    assert_eq!(file(&seen, "provision/hostname"), Some("studio\n"));
    assert_eq!(file(&seen, "provision/account"), Some("sam\n$6$salt$hash\n"));
    assert_eq!(file(&seen, "provision/cloudflared-token"), Some("machine-token"));
    assert_eq!(file(&seen, "provision/setup.json"), Some(r#"{"step":"welcome"}"#));
    let wadd = file(&seen, "provision/wadd.toml").unwrap();
    assert!(wadd.contains("id = \"studio\"") && wadd.contains("host = \"studio.wadspaces.com\""));
    assert!(file(&seen, "provision/turn.toml").unwrap().contains("turn-key"));
    // The account's web app: from the stick, never the image.
    assert_eq!(file(&seen, "provision/firebase.toml"), Some(FIREBASE));
    for (name, bytes) in seen.iter() {
        let t = String::from_utf8_lossy(bytes);
        assert!(!t.contains("dns-key") && !t.contains("tunnel-key"), "{name} has a stick's key");
        assert!(!t.contains("account-password"), "{name} has the password");
    }
    // Its folder doesn't outlast the job.
    assert_eq!(std::fs::read_dir(&r.run).unwrap().count(), 0);
}

#[tokio::test]
async fn a_name_thats_taken_stops_before_the_disk() {
    let cf = cloudflare(true).await;
    let r = rig("taken", Some(&cf.uri()), true, "ok\n");
    let job = r.install.start(request()).await.unwrap();
    let j = ends(&r, &job.job.id).await;
    assert_eq!(j.job.status, JobStatus::Error);
    assert!(j.job.error.unwrap().contains("connected already"));
    assert_eq!(*r.fake.ran.lock().unwrap(), 0);
}

#[tokio::test]
async fn a_stick_without_keys_installs_a_machine_without_an_address() {
    let r = rig("plain", None, false, "ok\n");
    assert!(!r.install.status().await.unwrap().public);
    let mut req = request();
    req.disk_password = None;
    let job = r.install.start(req).await.unwrap();
    assert_eq!(ends(&r, &job.job.id).await.job.status, JobStatus::Done);
    let seen = r.fake.seen.lock().unwrap();
    assert!(file(&seen, "passphrase").is_none());
    assert!(file(&seen, "request").unwrap().contains("ENCRYPT=0\n"));
    assert!(file(&seen, "provision/cloudflared-token").is_none());
    assert!(file(&seen, "provision/turn.toml").is_none());
    assert!(file(&seen, "provision/firebase.toml").is_none());
    assert!(!file(&seen, "provision/wadd.toml").unwrap().contains("streaming"));
}

#[tokio::test]
async fn the_scripts_failure_is_the_jobs_and_the_folder_still_goes() {
    let r = rig("fail", None, false, "failed: something on that disk is mounted\n");
    let job = r.install.start(request()).await.unwrap();
    let j = ends(&r, &job.job.id).await;
    assert_eq!(j.job.status, JobStatus::Error);
    assert_eq!(j.job.error.as_deref(), Some("something on that disk is mounted"));
    assert_eq!(std::fs::read_dir(&r.run).unwrap().count(), 0);
}

#[tokio::test]
async fn wadd_has_the_last_word_on_what_it_erases() {
    let r = rig("refuse", None, false, "ok\n");
    for (change, says) in [
        (Box::new(|q: &mut InstallRequest| q.name = "www".into()) as Box<dyn Fn(&mut InstallRequest)>, "kept for"),
        (Box::new(|q| q.username = "root".into()), "taken by the system"),
        (Box::new(|q| q.password = "short".into()), "At least 8"),
        (Box::new(|q| q.disk_password = Some("short".into())), "At least 8"),
        (Box::new(|q| q.serial = "another".into()), "isn't here any more"),
        (Box::new(|q| q.size = 1), "isn't here any more"),
    ] {
        let mut q = request();
        change(&mut q);
        let e = r.install.start(q).await.unwrap_err();
        assert!(e.message.contains(says), "{}", e.message);
    }
    assert_eq!(*r.fake.ran.lock().unwrap(), 0);
    let _ = &r.state;
}

#[test]
fn only_a_stick_has_an_installer() {
    let d = std::env::temp_dir().join(format!("wadd-install-none-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let bus = Bus::new(wad_proto::MachineInfo {
        id: "m".into(),
        name: "M".into(),
        version: "0".into(),
        profile: wad_proto::Profile::System,
    });
    let hands = Arc::new(SampleHands { step: Duration::ZERO });
    assert!(MachineInstall::open(&d, &d, hands, Jobs::new(bus, &d)).is_none());
}

#[test]
fn the_stick_and_what_isnt_a_disk_arent_offered() {
    let v = json!({ "blockdevices": [
        { "name": "sda", "type": "disk", "size": 64, "tran": "usb", "model": "Stick ", "serial": "U1",
          "children": [{ "name": "sda3", "mountpoints": ["/sysroot", "/var"] }] },
        { "name": "zram0", "type": "disk", "size": 8 },
        { "name": "sr0", "type": "rom", "size": 1024 },
        { "name": "sdb", "type": "disk", "size": 0 },
        { "name": "sdc", "type": "disk", "size": 32, "model": null },
        { "name": "sdd", "type": "disk", "size": 32, "tran": "usb", "serial": "U2" },
        { "name": "nvme0n1", "type": "disk", "size": 512, "model": "SSD", "serial": " S1 ",
          "children": [{ "name": "p1", "fstype": "vfat", "label": "EFI" }, { "name": "p2", "fstype": "ntfs" }] },
    ]});
    let link = |n: &str| (n != "sdc").then(|| format!("/dev/disk/by-id/{n}"));
    let d = parse_disks(&v, &link);
    assert_eq!(d.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["sdd", "nvme0n1"]);
    assert!(d[0].removable);
    assert_eq!(d[0].model, "sdd");
    assert_eq!(d[1].serial, "S1");
    assert_eq!(d[1].holds.as_deref(), Some("EFI, ntfs"));
}

#[test]
fn a_steps_line_is_a_stage() {
    assert_eq!(step_of("STEP 1/7: Checking the disk"), Some(0));
    assert_eq!(step_of("STEP 7/7: Finishing"), Some(6));
    assert_eq!(step_of("STEP x"), None);
    assert_eq!(step_of("Installing"), None);
}
