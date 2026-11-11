//! `wados-pkgs` as scripts run it (images/build.sh, CI's workflows): what
//! it prints on stdout, and its exit codes (0 printed, 1 refused, 2 not how
//! it's called), with nothing on stdout when it fails.

use std::process::{Command, Output};

fn wados_pkgs(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wados-pkgs")).args(args).output().expect("wados-pkgs runs")
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn it_prints_one_answer_or_fails_with_a_reason() {
    let o = wados_pkgs(&["remote-ref", "cpp", "fedora44.1"]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(text(&o.stdout), "ghcr.io/hunggod/wados-pkg-cpp:fedora44.1\n");

    let o = wados_pkgs(&["packages", "--wave", "0"]);
    assert_eq!(o.status.code(), Some(0));
    let wave: serde_json::Value = serde_json::from_slice(&o.stdout).expect("JSON");
    assert!(wave.as_array().is_some_and(|w| w.iter().any(|p| p["id"] == "cpp")), "{wave}");

    // Refused: why, on stderr.
    let o = wados_pkgs(&["containerfile", "obsidian"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(o.stdout.is_empty());
    assert!(text(&o.stderr).contains("doesn't know how to package \"obsidian\""), "{}", text(&o.stderr));

    // Not how it's called: the usage, on stderr.
    let o = wados_pkgs(&["chain-step", "only-one"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());
    assert!(text(&o.stderr).contains("usage: wados-pkgs"), "{}", text(&o.stderr));
}

#[test]
fn the_starters_set_is_what_a_stick_carries_for_wados() {
    let o = wados_pkgs(&["starter", "sha256:0123456789abcdef"]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let set: serde_json::Value = serde_json::from_slice(&o.stdout).expect("JSON");
    let ids: Vec<&str> = set["packages"].as_array().expect("packages").iter().filter_map(|p| p.as_str()).collect();
    // VS Code and its Claude Code extension (on the native claude), Rust (on
    // C/C++), Python, podman: what the WadOS wadspace has. Ori and Node.js
    // aren't in it.
    for id in ["vscode", "ext-anthropic.claude-code", "claude-code", "rust", "cpp", "python", "podman"] {
        assert!(ids.contains(&id), "{id}: {ids:?}");
    }
    assert!(!ids.contains(&"ori") && !ids.contains(&"nodejs"), "{ids:?}");
    let key = set["key"].as_str().expect("a key");
    assert_eq!(set["image"], format!("localhost/wados-set-{key}:latest"));

    let o = wados_pkgs(&["starter"]);
    assert_eq!(o.status.code(), Some(2));
}
