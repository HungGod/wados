// What wad-gen makes of the spike's populated sample (wad-model's
// tests/data/populated.json): its packages, sets, units and extra folders.
// Key lines, not whole files.

use wad_gen::extras;
use wad_gen::units::{self, Machine, Run, Sidecar};
use wad_gen::{BASE_IMAGE, Mounts, Set, SetBuild, packages};
use wad_model::{Built, Library, Wadspace, Wallpaper};

fn library() -> Library {
    serde_json::from_str(include_str!("../../wad-model/tests/data/populated.json"))
        .expect("populated.json is a library")
}

fn ws<'a>(lib: &'a Library, id: &str) -> &'a Wadspace {
    lib.wadspace(id).unwrap_or_else(|| panic!("no wadspace {id}"))
}

/// A machine: rootful podman in a user namespace, a screen, a GPU.
fn machine() -> Machine {
    Machine {
        state_dir: "/var/lib/wados".into(),
        signins_dir: "/var/lib/wados/keys".into(),
        tz: "Australia/Melbourne".into(),
        secrets: vec!["github_token".into()],
        userns_base: Some(units::USERNS_BASE),
        devices: vec!["/dev/dri".into()],
        device_rules: vec!["c 189:* rw".into()],
        screen_dir: Some("/run/wados/screen".into()),
        screens: 1,
        ..Machine::default()
    }
}

fn unit(w: &Wadspace, mounts: &Mounts, m: &Machine) -> String {
    let image = Set::of(w, "sha256:base").image(BASE_IMAGE);
    let run = Run { image: &image, mounts, wallpaper: None };
    let mut all = units::units(w, &run, m, None);
    assert_eq!(all.len(), 1);
    let (name, text) = all.remove(0);
    assert_eq!(name, format!("wad-{}.container", w.id));
    text
}

fn has(text: &str, line: &str) -> bool {
    text.lines().any(|l| l == line)
}

#[track_caller]
fn assert_lines(text: &str, lines: &[&str]) {
    for l in lines {
        assert!(has(text, l), "no {l:?} in:\n{text}");
    }
}

fn lines_with<'a>(text: &'a str, prefix: &str) -> Vec<&'a str> {
    text.lines().filter(|l| l.starts_with(prefix)).collect()
}

#[test]
fn what_each_wadspace_is_made_of() {
    let lib = library();
    let pk = |id: &str| packages::packages_of(ws(&lib, id));
    let un = |id: &str| packages::unpackaged(ws(&lib, id));
    assert_eq!(pk("wadspaces"), ["cpp", "rust", "vscode", "ext-anthropic.claude-code"]);
    assert!(un("wadspaces").is_empty());
    assert_eq!(
        pk("intelligencequest"),
        ["cpp", "rust", "vscode", "ext-anthropic.claude-code", "ext-rust-lang.rust-analyzer"]
    );
    assert_eq!(un("intelligencequest"), ["app:libresprite", "app:tiled"]);
    assert_eq!(pk("vanua-academy"), ["vscode", "ext-anthropic.claude-code"]);
    assert_eq!(un("vanua-academy"), ["terminal:firebase"]);
    assert!(pk("cosmic-bodybuilding").is_empty());
    assert_eq!(un("cosmic-bodybuilding"), ["app:obsidian"]);
    assert_eq!(un("couch-co-op"), ["app:steam"], "its games aren't packages");
    assert_eq!(un("retro"), ["app:dolphin"]);

    // The starter's set: its packages and its four commands.
    let set = Set::of(ws(&lib, "wadspaces"), "sha256:base");
    assert_eq!(set.commands.len(), 4);
    assert_eq!(set.state(None, true), Built::Never);
    assert_eq!(set.state(Some(&set.layer()), true), Built::Built);
    // A wadspace with nothing to add runs the base.
    let travel = Set::of(ws(&lib, "travel"), "sha256:base");
    assert_eq!(travel.image(BASE_IMAGE), BASE_IMAGE);
    // The same apps and add-ons, the same set (VanuaAcademy and its client demo).
    assert_eq!(Set::of(ws(&lib, "vanua-academy"), "b").key, Set::of(ws(&lib, "vanua-academy-client-demo"), "b").key);
}

#[test]
fn the_starters_set_runs_its_commands_on_its_packages() {
    let lib = library();
    let w = ws(&lib, "wadspaces");
    let set = Set::of(w, "sha256:base");
    let key = set.key.as_deref().expect("a set");
    let pkgs: Vec<(String, String)> =
        set.packages.iter().map(|p| (p.clone(), packages::local_ref(p, "0123456789abcdef"))).collect();
    let from = packages::chain_ref("sha256:base", &pkgs);
    let file = SetBuild { key, from: &from, base_id: "sha256:base", packages: &pkgs, commands: &set.commands }
        .containerfile()
        .expect("a Containerfile");
    let text = &file.containerfile;
    assert_lines(
        text,
        &[
            &format!("ARG BASE_IMAGE={from}"),
            "RUN wadspaces-pkg assemble",
            "RUN dnf install -y gcc pkg-config gtk4-devel gtk4-layer-shell-devel",
            "RUN rustup toolchain install 1.99 --component rustfmt clippy --target wasm32-unknown-unknown",
        ],
    );
    // In order, after the packages are put together, and its labels last.
    let at = |l: &str| text.find(l).unwrap_or_else(|| panic!("no {l}"));
    assert!(at("assemble") < at("RUN dnf install -y gcc") && at("RUN dnf install -y binaryen") < at("RUN rustup"));
    assert!(text.trim_end().lines().last().is_some_and(|l| l.starts_with("LABEL io.wadspaces.layer=\"set:")));
    assert!(file.stage.is_empty());
}

#[test]
fn a_wadspace_on_the_screen() {
    let lib = library();
    let m = machine();
    let w = ws(&lib, "wadspaces");
    let set = Set::of(w, "sha256:base");
    let u = unit(w, &Mounts::new(), &m);
    assert_lines(
        &u,
        &[
            "Description=WadSpaces wadspace: WadOS",
            &format!("Image={}", set.image(BASE_IMAGE)),
            "ContainerName=wad-wadspaces",
            "Label=wadspaces.id=wadspaces",
            "UIDMap=0:1000000:1000",
            "UIDMap=1000:1000:1",
            "GIDMap=1001:1001001:64535",
            "Environment=PUID=1000",
            "Environment=PGID=1000",
            "Environment=TZ=Australia/Melbourne",
            "Environment=TITLE=WadOS",
            "Environment=WALLPAPER=/run/wadspaces-extra/wallpaper.svg",
            "Environment=WALLPAPER_MODE=fill",
            "Environment=PULSE_SERVER=unix:/run/wadspaces-display/pulse/native",
            "Secret=github_token",
            // Its home keeps its files' owners under the machine's maps.
            "Volume=wad-wadspaces-config:/config:z,idmap",
            // A GitHub project: wadd's copy of it, read-only, to start from.
            "Volume=/var/lib/wados/copies/wadspaces:/run/wadspaces-copies/wadspaces:ro,z",
            "Volume=/var/lib/wados/extra/wadspaces:/run/wadspaces-extra:ro,z",
            // The screen only through its own folder.
            "Volume=/run/wados/screen/wadspaces:/run/wadspaces-display:ro,rslave",
            &format!("SecurityLabelLevel={}", units::mcs_level("wadspaces")),
            "AddDevice=/dev/dri",
            "PodmanArgs=\"--device-cgroup-rule=c 189:* rw\"",
            "ShmSize=1g",
            "Pull=never",
            "Restart=no",
            "TimeoutStartSec=900",
        ],
    );
    assert!(!u.contains("/run/user/") && !u.contains("SecurityLabelDisable") && !has(&u, "[Install]"), "{u}");
    assert!(!u.contains("BindsTo") && !u.contains("PublishPort") && !u.contains("WLR_WL_OUTPUTS"), "{u}");
    // Two screens: its compositor draws a window on each.
    let two = unit(w, &Mounts::new(), &Machine { screens: 2, ..machine() });
    assert!(has(&two, "Environment=WLR_WL_OUTPUTS=2"), "{two}");
}

#[test]
fn its_keys_are_mounted_and_listed() {
    let lib = library();
    let m = machine();
    let w = ws(&lib, "vanua-academy");
    let u = unit(w, &Mounts::new(), &m);
    let keys = lines_with(&u, "Volume=/var/lib/wados/keys/");
    assert_eq!(
        keys,
        [
            "Volume=/var/lib/wados/keys/vanua-claude:/run/wadspaces-signins/vanua-claude:rw,z",
            "Volume=/var/lib/wados/keys/work-web:/run/wadspaces-signins/work-web:rw,z",
            "Volume=/var/lib/wados/keys/vanua-google:/run/wadspaces-signins/vanua-google:rw,z",
            "Volume=/var/lib/wados/keys/hunggod:/run/wadspaces-signins/hunggod:rw,z",
        ]
    );
    let json: serde_json::Value =
        serde_json::from_str(&extras::signins_json(w, &lib.keys).expect("it has keys")).expect("json");
    assert_eq!(json["version"], 2);
    let s = &json["signins"];
    assert_eq!(s[0]["id"], "vanua-claude");
    assert_eq!(s[0]["web"], false);
    assert_eq!(s[0]["apps"], serde_json::json!(["vscode"]));
    assert_eq!(s[0]["dir"], "/run/wadspaces-signins/vanua-claude");
    assert_eq!(s[0]["paths"], serde_json::json!([".claude/", ".claude.json"]));
    assert_eq!(s[1]["id"], "work-web");
    assert_eq!(s[1]["web"], true);
    assert_eq!(s[1]["apps"], serde_json::json!(["gmail", "google-drive", "claude"]));
    assert_eq!(s[1]["paths"], serde_json::json!([]));
    // A web key's sites (its record's, this machine's); an app's key has none.
    assert_eq!(s[1]["sites"], serde_json::json!(["claude.ai", "mail.google.com", "drive.google.com"]));
    assert_eq!(s[0]["sites"], serde_json::json!([]));
    assert_eq!(s[3]["apps"], serde_json::json!(["github"]));
    assert_eq!(s[3]["sites"], serde_json::json!(["github.com"]));
    // WadBrowser's own windows, in IntelligenceQuest's group.
    let iq = extras::signins(ws(&lib, "intelligencequest"));
    assert_eq!(iq[1].key.id, "hunggod");
    assert_eq!(iq[1].apps, ["@browser", "github"]);
    // A key's own paths (this machine's record), else its kind's.
    let desk = extras::signins_json(ws(&lib, "desk-lab"), &lib.keys).expect("its key");
    assert!(desk.contains("\".config/obsidian/\""), "{desk}");
    let elsewhere = extras::signins_json(ws(&lib, "desk-lab"), &[]).expect("its key");
    assert!(elsewhere.contains("\".config/obsidian/Local Storage/\""), "{elsewhere}");
    // Without this machine's record of a web key, it has no sites.
    let bare: serde_json::Value = serde_json::from_str(&extras::signins_json(w, &[]).expect("its keys")).expect("json");
    assert_eq!(bare["signins"][1]["sites"], serde_json::json!([]));
    assert!(extras::signins_json(ws(&lib, "travel"), &lib.keys).is_none());
}

#[test]
fn a_mount_is_the_machines_own_folder_and_turns_selinux_off() {
    let lib = library();
    let m = machine();
    let w = ws(&lib, "cosmic-bodybuilding");
    let found = Mounts::from([("mount-thesis".to_string(), "/var/home/sam/Thesis".to_string())]);
    let u = unit(w, &found, &m);
    assert_lines(
        &u,
        &[
            "Volume=/var/home/sam/Thesis:/config/Desktop/Thesis:rw",
            "Volume=/var/lib/wados/copies/writing:/run/wadspaces-copies/writing:ro,z",
            "SecurityLabelDisable=true",
        ],
    );
    assert!(!u.contains("SecurityLabelLevel"), "{u}");
    let projects: serde_json::Value = serde_json::from_str(&extras::projects_json(w, &found)).expect("json");
    assert_eq!(
        projects,
        serde_json::json!({"version": 2, "projects": [
            {"id": "writing", "kind": "github", "mount": "Writing", "name": "Writing", "repo": "HungGod/Writing",
             "setup": "", "setupHash": extras::setup_hash("")},
            {"id": "mount-thesis", "kind": "mount", "mount": "Thesis", "name": "Thesis"},
        ]})
    );
    // Not on this machine: nothing mounted or listed, and SELinux stays on.
    let away = unit(w, &Mounts::new(), &m);
    assert!(!away.contains("/config/Desktop") && !away.contains("SecurityLabelDisable"), "{away}");
    assert!(!extras::projects_json(w, &Mounts::new()).contains("mount-thesis"));
    // A path podman's -v can't say isn't mounted either.
    let odd = Mounts::from([("mount-thesis".to_string(), "/var/home/sam/a:b".to_string())]);
    assert!(!unit(w, &odd, &m).contains("/config/Desktop"));
    // A copy is copied in, from wadd's copy of its files.
    let demo = unit(ws(&lib, "vanua-academy-client-demo"), &Mounts::new(), &m);
    assert!(has(&demo, "Volume=/var/lib/wados/copies/deck:/run/wadspaces-copies/deck:ro,z"), "{demo}");
}

#[test]
fn rootless_on_a_laptop() {
    let lib = library();
    let m = Machine {
        state_dir: "/home/sam/.local/state/wados".into(),
        signins_dir: "/home/sam/.local/state/wados/keys".into(),
        rootless_uid: Some(1000),
        screen_dir: Some("/run/user/1000/wados/screen".into()),
        pulse_dir: Some("/run/user/1000/pulse".into()),
        ..Machine::default()
    };
    let u = unit(ws(&lib, "desk-lab"), &Mounts::new(), &m);
    assert_lines(
        &u,
        &[
            "UIDMap=0:1:1000",
            "UIDMap=1000:0:1",
            "GIDMap=1001:1001:64535",
            "Volume=wad-desk-lab-config:/config:z",
            "Volume=/run/user/1000/wados/screen/desk-lab:/run/wadspaces-display:ro,rslave",
            // Where wadd can't bind PulseAudio's folder in, it's mounted from the start.
            "Volume=/run/user/1000/pulse:/run/wadspaces-display/pulse:ro",
            "Environment=TZ=Etc/UTC",
        ],
    );
    assert!(!u.contains("Secret=") && !u.contains("AddDevice") && !u.contains("idmap"), "{u}");
}

#[test]
fn its_own_wallpaper_and_title_bars() {
    let lib = library();
    let mut w = ws(&lib, "travel").clone();
    w.look.title_bars = Some(true);
    let colour = Wallpaper::color("#17112a");
    let run = Run { image: BASE_IMAGE, mounts: &Mounts::new(), wallpaper: Some(&colour) };
    let u = &units::units(&w, &run, &machine(), None)[0].1;
    assert_lines(u, &["Environment=WALLPAPER_COLOR=#17112a", "Environment=WADSPACES_TITLEBARS=1"]);
    assert!(!u.contains("WALLPAPER="), "{u}");
    assert_eq!(extras::wallpaper_file(&Wallpaper::image("~/Pictures/Lagoon.JPEG")).as_deref(), Some("wallpaper.jpg"));
    assert_eq!(extras::wallpaper_file(&colour), None);
}

#[test]
fn streamed_it_draws_on_its_sidecar() {
    let lib = library();
    let m = machine();
    let w = ws(&lib, "retro");
    let run = Run { image: BASE_IMAGE, mounts: &Mounts::new(), wallpaper: None };
    let s = Sidecar { image: "localhost/wados-stream:44", guid: "g1", web_port: 48101, mux_port: Some(48201) };
    let all = units::units(w, &run, &m, Some(&s));
    let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["wad-retro.container", "wadstream-retro.container", "wadstream-retro.volume"]);
    let (unit, sidecar, volume) = (&all[0].1, &all[1].1, &all[2].1);
    assert_lines(
        unit,
        &[
            "BindsTo=wadstream-retro.service",
            "After=wadstream-retro.service",
            "Volume=wadstream-retro.volume:/run/wadspaces-display:z",
            "Environment=PULSE_SERVER=unix:/run/wadspaces-display/pulse/native",
            "AddDevice=/dev/dri",
        ],
    );
    // Its devices and the screen stay on this machine.
    assert!(!unit.contains("/run/wados/screen") && !unit.contains("device-cgroup-rule"), "{unit}");
    assert_lines(
        sidecar,
        &[
            "Image=localhost/wados-stream:44",
            "ContainerName=wadstream-retro",
            "Label=wadspaces.id=retro",
            "Label=wadspaces.role=display",
            "Label=wadspaces.stream=g1",
            "StopWhenUnneeded=yes",
            "UIDMap=0:1000000:1000",
            "Volume=wadstream-retro.volume:/run/wadspaces-display:z",
            "PublishPort=127.0.0.1:48101:3000",
            "PublishPort=48201:48201/udp",
            "PublishPort=48201:48201/tcp",
            "Environment=SELKIES_WEBRTC_UDP_MUX_PORT=48201",
            "Secret=wad_turn_retro,type=mount,target=wadspaces-turn",
        ],
    );
    // The two share their categories.
    let level = format!("SecurityLabelLevel={}", units::mcs_level("retro"));
    assert!(has(unit, &level) && has(sidecar, &level));
    assert_lines(volume, &["[Volume]", "VolumeName=wadstream-retro", "User=1000", "Group=1000"]);
    // WebSockets only: no WebRTC, no TURN.
    let private = Sidecar { mux_port: None, ..s };
    let all = units::units(w, &run, &m, Some(&private));
    assert!(!all[1].1.contains("Secret=") && !all[1].1.contains("/udp"), "{}", all[1].1);
}

#[test]
fn no_unit_or_extra_file_has_a_secret_in_it() {
    let lib = library();
    let m = machine();
    let s = Sidecar { image: "localhost/wados-stream:44", guid: "g", web_port: 1, mux_port: Some(2) };
    for w in &lib.wadspaces {
        let run = Run { image: BASE_IMAGE, mounts: &Mounts::new(), wallpaper: None };
        let mut text: String = units::units(w, &run, &m, Some(&s)).into_iter().map(|(_, t)| t).collect();
        for (_, f) in extras::files(w, &lib.keys, &Mounts::new()) {
            text.push_str(&f.unwrap_or_default());
        }
        let lower = text.to_lowercase();
        for word in ["password", "api_token", "token_id", "turn.toml", "cloudflare_turn"] {
            assert!(!lower.contains(word), "{word} in {}'s files", w.id);
        }
    }
}

#[test]
fn its_extra_folder() {
    let lib = library();
    // The desktop in its look's order, each app with its recipe's launcher.
    let layout: serde_json::Value = serde_json::from_str(&extras::layout_json(ws(&lib, "wadspaces"))).expect("json");
    assert_eq!(
        layout,
        serde_json::json!({"version": 1, "icons": [
            {"app": "vscode", "desktop": "wadspaces-vscode.desktop", "label": "VS Code", "autostart": true},
            {"app": "wadbrowser", "desktop": "io.wados.Browser.desktop", "label": "WadBrowser"},
            {"app": "terminal", "desktop": "foot.desktop", "label": "Command line"},
            {"project": "wadspaces", "label": "WadSpaces"},
        ]})
    );
    // Web apps are WadBrowser's launchers; an app not packaged yet has no icon.
    let iq: serde_json::Value =
        serde_json::from_str(&extras::layout_json(ws(&lib, "intelligencequest"))).expect("json");
    let apps: Vec<&str> = iq["icons"].as_array().into_iter().flatten().filter_map(|i| i["app"].as_str()).collect();
    assert_eq!(apps, ["wadbrowser", "vscode", "webapp-github", "terminal"]);
    assert_eq!(iq["icons"][2]["desktop"], "wadspaces-webapp-github.desktop");
    let demo = extras::layout_json(ws(&lib, "vanua-academy-client-demo"));
    assert!(demo.contains("\"app\": \"webapp-claude\",\n      \"desktop\": \"wadspaces-webapp-claude.desktop\",\n      \"label\": \"Claude\",\n      \"autostart\": true"), "{demo}");
    // An icon its look's order doesn't name still comes (after the rest).
    let mut w = ws(&lib, "desk-lab").clone();
    w.look.layout.icons.clear();
    w.apps.push(wad_model::AppIn::new(wad_model::AppId::Krita));
    let l: serde_json::Value = serde_json::from_str(&extras::layout_json(&w)).expect("json");
    assert_eq!(l["icons"][0]["desktop"], "org.kde.krita.desktop");

    let webapps: serde_json::Value = serde_json::from_str(&extras::webapps_json(ws(&lib, "travel"))).expect("json");
    assert_eq!(webapps, serde_json::json!([{"id": "gmail", "name": "Gmail", "url": "https://mail.google.com"}]));
    assert_eq!(extras::webapps_json(ws(&lib, "desk-lab")), "[]\n");

    // What links open in: Focus in IntelligenceQuest; none without WadBrowser.
    assert!(extras::wadbrowser_conf(ws(&lib, "intelligencequest")).is_some_and(|c| c.ends_with("default = focus\n")));
    assert!(extras::wadbrowser_conf(ws(&lib, "travel")).is_some_and(|c| c.ends_with("default = full\n")));
    assert_eq!(extras::wadbrowser_conf(ws(&lib, "desk-lab")), None);

    let files = extras::files(ws(&lib, "desk-lab"), &lib.keys, &Mounts::new());
    let names: Vec<(&str, bool)> = files.iter().map(|(n, f)| (*n, f.is_some())).collect();
    assert_eq!(
        names,
        [
            ("webapps.json", true),
            ("layout.json", true),
            ("projects.json", true),
            ("signins.json", true),
            ("wadbrowser.conf", false)
        ]
    );
    assert_eq!(extras::dir("/var/lib/wados", "desk-lab"), "/var/lib/wados/extra/desk-lab");
}

#[test]
fn a_streams_sidecar_is_never_another_wadspaces_container() {
    use wad_gen::units::{container_name, display_name};
    for id in ["retro", "retro-display", "d", "wad", "x.y_z"] {
        for other in ["retro", "retro-display", "stream-retro", "d", "wad", "x.y_z"] {
            assert_ne!(display_name(id), container_name(other), "{id} vs {other}");
        }
    }
}
