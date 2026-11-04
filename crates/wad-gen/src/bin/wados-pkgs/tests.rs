use super::*;
use wad_model::{AppId, AppIn, Ext};

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn ok(args: &[&str]) -> String {
    run(&s(args)).unwrap_or_else(|e| panic!("{args:?}: {e:?}"))
}

fn json_of(args: &[&str]) -> Value {
    serde_json::from_str(&ok(args)).unwrap_or_else(|e| panic!("{args:?}: {e}"))
}

fn is_usage(args: &[&str]) -> bool {
    matches!(run(&s(args)), Err(Fail::Usage(_)))
}

fn is_error(args: &[&str]) -> bool {
    matches!(run(&s(args)), Err(Fail::Error(_)))
}

/// A folder of the test's own, removed when it ends.
struct Temp(std::path::PathBuf);

impl Temp {
    fn new(name: &str) -> Temp {
        let dir = std::env::temp_dir().join(format!("wados-pkgs-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp folder");
        Temp(dir)
    }

    fn file(&self, name: &str, text: &str) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, text).expect("a temp file");
        path.to_string_lossy().into_owned()
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn packages_are_what_ci_builds_in_its_waves() {
    let all = json_of(&["packages"]);
    let all = all.as_array().expect("a list");
    let ids: Vec<&str> = all.iter().filter_map(|p| p["id"].as_str()).collect();
    assert_eq!(ids, p::all(), "every package, in WadOS's order");
    let waves = p::waves(&p::all());
    let wave_of = |id: &str| all.iter().find(|p| p["id"] == id).and_then(|p| p["wave"].as_u64());
    for pkg in all {
        let id = pkg["id"].as_str().expect("an id");
        let wave = pkg["wave"].as_u64().expect("a wave");
        assert!(waves[wave as usize].iter().any(|w| w == id), "{id} in wad-gen's wave {wave}");
        assert_eq!(pkg["needs"], json!(p::closure(&p::needs(id))), "{id}");
        assert_eq!(pkg["name"], p::name(id));
        assert_eq!(pkg["meta"], p::meta_path(id));
        for n in pkg["needs"].as_array().expect("needs") {
            assert!(wave_of(n.as_str().expect("an id")) < Some(wave), "{id} needs {n} from an earlier wave");
        }
    }
    let rust = all.iter().find(|p| p["id"] == "rust").expect("rust");
    assert_eq!(
        rust,
        &json!({"id": "rust", "name": "Rust", "needs": ["cpp"], "wave": 1, "meta": "/usr/share/wadspaces/pkg/rust/meta.json"})
    );

    // One wave: the matrix of one CI job.
    let one = json_of(&["packages", "--wave", "1"]);
    let one: Vec<&str> = one.as_array().expect("a list").iter().filter_map(|p| p["id"].as_str()).collect();
    assert_eq!(one, waves[1]);
    assert_eq!(json_of(&["packages", "--wave", "9"]), json!([]));

    // Some: those and what they need, in WadOS's order, in their own waves.
    let some = json_of(&["packages", "rust", "rpm-krita"]);
    let some: Vec<(&str, u64)> = some
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|p| Some((p["id"].as_str()?, p["wave"].as_u64()?)))
        .collect();
    assert_eq!(some, [("cpp", 0), ("rust", 1), ("rpm-krita", 0)]);

    assert!(is_error(&["packages", "rust", "obsidian"]), "not packaged");
    assert!(is_error(&["packages", "apt-krita"]), "v1's");
    assert!(is_usage(&["packages", "--wave"]));
    assert!(is_usage(&["packages", "--wave", "one"]));
    assert!(is_usage(&["packages", "--all"]));
}

#[test]
fn a_containerfile_is_wad_gens() {
    let base = "ghcr.io/hunggod/wados-base:44@sha256:b";
    let cpp = "ghcr.io/hunggod/wados-pkg-cpp:fedora44.1@sha256:c";
    let want = p::package_containerfile("rust", base, &[("cpp".into(), cpp.into())], true).expect("a Containerfile");
    assert_eq!(ok(&["containerfile", "rust", "--base", base, "--need", &format!("cpp={cpp}"), "--token"]), want);
    // The base by default: wadd's.
    assert_eq!(
        ok(&["containerfile", "rpm-krita"]),
        p::package_containerfile("rpm-krita", BASE_IMAGE, &[], false).expect("a Containerfile")
    );
    // What wad-gen refuses, it refuses.
    assert_eq!(
        run(&s(&["containerfile", "rust", "--base", base])),
        Err(Fail::Error(p::Error::Needs { id: "rust".into(), missing: "cpp".into() }.to_string()))
    );
    assert_eq!(run(&s(&["containerfile", "nope"])), Err(Fail::Error(p::Error::Unknown("nope".into()).to_string())));
    assert!(is_usage(&["containerfile"]));
    assert!(is_usage(&["containerfile", "--base", base]));
    assert!(is_usage(&["containerfile", "rust", "--need", "cpp"]));
    assert!(is_usage(&["containerfile", "rust", "--need"]));
    assert!(is_usage(&["containerfile", "rust", "--base"]));
    assert!(is_usage(&["containerfile", "rust", "--secret"]));
}

#[test]
fn labels_are_v1s_from_the_captures_meta() {
    let t = Temp::new("labels");
    let meta = t.file(
        "meta.json",
        r#"{"id": "rpm-krita", "version": "6.0.4.1-1.fc44", "needs": [], "abi": "fedora44.1", "content": "abc123", "files": 3}"#,
    );
    let ci = "LABEL io.wadspaces.pkg.version=\"6.0.4.1-1.fc44\" io.wadspaces.pkg.abi=\"fedora44.1\" \
              io.wadspaces.pkg.content=\"abc123\"\n";
    assert_eq!(ok(&["labels", "rpm-krita", "--meta", &meta, "--abi", "fedora44.1"]), ci);
    // The ABI the capture saw, by default; built here, said so.
    assert_eq!(
        ok(&["labels", "rpm-krita", "--meta", &meta, "--built-here"]),
        ci.replace('\n', " io.wadspaces.pkg.built-here=\"true\"\n")
    );
    // Captured on a base of another ABI, another package's meta.json, no content hash: no.
    assert!(is_error(&["labels", "rpm-krita", "--meta", &meta, "--abi", "fedora44.2"]));
    assert!(is_error(&["labels", "nodejs", "--meta", &meta]));
    let none = t.file("none.json", r#"{"id": "cpp", "version": "x", "abi": "a"}"#);
    assert!(is_error(&["labels", "cpp", "--meta", &none]));
    assert!(is_error(&["labels", "cpp", "--meta", &t.path("missing.json")]));
    assert!(is_error(&["labels", "cpp", "--meta", &t.file("bad.json", "not json")]));
    // A version is a value: quotes escaped, no variable read in it. No ABI in the meta: --abi.
    let odd = t.file("odd.json", r#"{"id": "cpp", "version": "cmake \"3\" $HOME \\", "content": "c"}"#);
    assert!(is_error(&["labels", "cpp", "--meta", &odd]));
    assert_eq!(
        ok(&["labels", "cpp", "--meta", &odd, "--abi", "a"]),
        "LABEL io.wadspaces.pkg.version=\"cmake \\\"3\\\" \\$HOME \\\\\" io.wadspaces.pkg.abi=\"a\" \
         io.wadspaces.pkg.content=\"c\"\n"
    );
    assert!(is_usage(&["labels", "cpp"]));
    assert!(is_usage(&["labels", "--meta", &meta]));
    assert!(is_usage(&["labels", "cpp", "--meta"]));
    assert!(is_usage(&["labels", "cpp", "--meta", &meta, "--version", "1"]));
}

#[test]
fn refs_and_chains_are_wad_gens() {
    assert_eq!(ok(&["local-ref", "cpp", "sha256:41fc6e02a4d9aaaa"]), "localhost/wados-pkg-cpp:41fc6e02a4d9\n");
    assert_eq!(ok(&["remote-ref", "cpp", "fedora44.1"]), "ghcr.io/hunggod/wados-pkg-cpp:fedora44.1\n");
    assert!(is_usage(&["local-ref", "cpp"]));
    assert!(is_usage(&["remote-ref", "cpp", "a", "b"]));

    let chain = vec![
        ("cpp".to_string(), "localhost/wados-pkg-cpp:41fc6e02a4d9".to_string()),
        ("rust".to_string(), "localhost/wados-pkg-rust:448760a5be45".to_string()),
    ];
    let args: Vec<String> = chain.iter().map(|(i, r)| format!("{i}={r}")).collect();
    let with = |head: &[&str]| -> Vec<String> { s(head).into_iter().chain(args.iter().cloned()).collect() };
    assert_eq!(run(&with(&["chain-ref", "sha256:base"])), Ok(p::chain_ref("sha256:base", &chain) + "\n"));
    assert_eq!(ok(&["chain-ref", "base", &args[0]]), p::chain_ref("base", &chain[..1]) + "\n");
    assert_eq!(ok(&["chain-ref", "base"]), p::chain_ref("base", &[]) + "\n");
    // A step copies the last one on, and is labelled with them all.
    let top = p::chain_ref("b", &chain[..1]);
    assert_eq!(run(&with(&["chain-step", &top, "sha256:t"])), Ok(p::chain_step(&top, "sha256:t", &chain[1].1, &chain)));
    assert!(is_usage(&["chain-step", &top, "sha256:t"]), "nothing to copy on");
    assert!(is_usage(&["chain-step", &top, "sha256:t", "rust"]), "not <id>=<ref>");
    assert!(is_usage(&["chain-ref", "b", "=r"]));
    assert!(is_usage(&["chain-ref"]));
}

#[test]
fn a_set_is_keyed_as_wadd_keys_a_wadspaces() {
    // Krita, and the command line with Rust: rpm-krita, and rust on cpp.
    let mut ws = wad_model::ops::blank("w", "W", "m", 0, 0);
    ws.apps = vec![AppIn::new(AppId::Krita), AppIn::new(AppId::Terminal)];
    ws.exts = vec![Ext::new(AppId::Terminal, "rust")];
    let want = wad_gen::Set::of(&ws, "sha256:b1");
    let got = json_of(&["set", "b1", "rust", "rpm-krita"]);
    assert_eq!(got["packages"], json!(want.packages));
    assert_eq!(got["packages"], json!(["cpp", "rust", "rpm-krita"]));
    assert_eq!(got["key"].as_str(), want.key.as_deref());
    assert_eq!(got["image"], want.image(BASE_IMAGE));
    // Whatever order, with what they need or not, sha256: or not: the same set.
    assert_eq!(json_of(&["set", "sha256:b1", "rpm-krita", "cpp", "rust", "rust"]), got);
    // Another base: another key.
    assert_ne!(json_of(&["set", "b2", "rust", "rpm-krita"])["key"], got["key"]);
    assert!(is_usage(&["set", "b1"]), "no packages: it runs the base");
    assert!(is_error(&["set", "b1", "rust", "nope"]));

    // Its Containerfile is SetBuild's, FROM its top chain.
    let pairs = vec![
        ("cpp".to_string(), "localhost/wados-pkg-cpp:41fc6e02a4d9".to_string()),
        ("rust".to_string(), "localhost/wados-pkg-rust:448760a5be45".to_string()),
    ];
    let from = p::chain_ref("b1", &pairs);
    let build = SetBuild { key: "0123456789ab", from: &from, base_id: "b1", packages: &pairs, commands: &[] };
    let want = build.containerfile().expect("a Containerfile").containerfile;
    let mut args = s(&["set-containerfile", "0123456789ab", &from, "b1"]);
    args.extend(pairs.iter().map(|(i, r)| format!("{i}={r}")));
    assert_eq!(run(&args), Ok(want));
    // No packages: the base's, nothing to put together.
    let plain = ok(&["set-containerfile", "k", BASE_IMAGE, "b1"]);
    assert!(plain.contains(&format!("ARG BASE_IMAGE={BASE_IMAGE}\n")) && !plain.contains("assemble"), "{plain}");
    assert!(is_usage(&["set-containerfile", "k", BASE_IMAGE]));
}

#[test]
fn the_index_takes_this_runs_packages() {
    let t = Temp::new("index");
    let krita = t.file(
        "result-rpm-krita.json",
        r#"{"id": "rpm-krita", "digest": "sha256:d", "imageId": "sha256:i", "version": "6.0.4.1-1.fc44",
            "size": 1, "needs": [], "content": "c", "built": "2026-11-04T00:00:00Z"}"#,
    );
    // None yet (no file, or CI's `{}`): a new one.
    let fresh = ok(&["index", &t.path("missing.json"), "fedora44.1", "sha256:base", &krita]);
    assert_eq!(fresh, ok(&["index", &t.file("empty.json", "{}"), "fedora44.1", "sha256:base", &krita]));
    let fresh: Index = serde_json::from_str(&fresh).expect("an index");
    assert_eq!(
        fresh,
        Index::merge(
            None,
            "fedora44.1",
            "sha256:base",
            &[serde_json::from_str(&std::fs::read_to_string(&krita).expect("read")).expect("a result")]
        )
    );
    assert_eq!(fresh.get("fedora44.1", "rpm-krita").map(|p| p.image_id.as_str()), Some("sha256:i"));
    // The next run's: other ABIs' kept.
    let old = t.file("old.json", &serde_json::to_string(&fresh).expect("JSON"));
    let next: Index = serde_json::from_str(&ok(&["index", &old, "fedora44.2", "sha256:b2"])).expect("an index");
    assert_eq!(next.abis["fedora44.1"], fresh.abis["fedora44.1"]);
    assert_eq!(next.abis["fedora44.2"].base, "sha256:b2");
    // A result that isn't one: no.
    assert!(is_error(&["index", &old, "a", "b", &t.file("bad.json", "[]")]));
    assert!(is_error(&["index", &old, "a", "b", &t.path("gone.json")]));
    assert!(is_usage(&["index", &old, "a"]));
}

#[test]
fn how_its_called() {
    assert!(is_usage(&[]));
    assert!(is_usage(&["build"]));
    assert_eq!(ok(&["--help"]), USAGE);
    // Every command is in the usage.
    for c in [
        "packages",
        "containerfile",
        "labels",
        "local-ref",
        "remote-ref",
        "chain-ref",
        "chain-step",
        "set ",
        "set-containerfile",
        "index",
    ] {
        assert!(USAGE.contains(&format!("\n  {c}")), "{c}");
    }
}
