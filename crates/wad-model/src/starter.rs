// The prebuilt wadspace Set up offers: WadOS, for working on WadSpaces
// itself (your own branch of its repo). It comes built (it's part of the
// image), with VS Code, the Cactus browser, the command line with Rust (the
// Rust tool: rustup and cargo) and Claude Code, the WadSpaces repo cloned
// in, what compiling it needs (its RUN commands), the container tools its
// host image is built and pushed with (podman, buildah, skopeo), and its
// agents allowed to message the desktop. It's an ordinary wadspace
// otherwise: add to it in the Builder, rename it, trash it.

use crate::commands::named_commands;
use crate::edit::{Ctx, Library};
use crate::ops::blank;
use crate::types::{AppId, AppIn, BuildCommand, CommandKind, Ext, Project, ProjectKind};
use crate::wallpapers::art_of;

pub const STARTER_ID: &str = "wadspaces";
/// The wados repo (WadSpaces v2, renamed from v1's WadSpaces).
pub const STARTER_REPO: &str = "HungGod/wados";

/// What compiling wados needs on Fedora 44 (its README): the C toolchain
/// and pkg-config; GTK 4, gtk4-layer-shell, libadwaita and PAM (the shell and
/// the apps); WebKitGTK 6.0 (WadBrowser, the Cactus fork); labwc and the
/// tools its headless probes and screenshots use; and, on the Rust tool's
/// rustup, the Rust it pins (1.99, rustfmt, clippy).
pub const WADOS_COMMANDS: [(CommandKind, &str); 4] = [
    // The C toolchain, GTK and PAM (the shell and the apps), and the
    // container tools the host image is built and pushed with (podman,
    // buildah, skopeo: host/build.sh image, then push after
    // wadspaces-ghcr-login).
    (
        CommandKind::Run,
        "dnf install -y gcc pkg-config gtk4-devel gtk4-layer-shell-devel libadwaita-devel pam-devel podman buildah skopeo fuse-overlayfs",
    ),
    (CommandKind::Run, "dnf install -y webkitgtk6.0-devel"),
    (CommandKind::Run, "dnf install -y labwc grim wlr-randr wtype foot wayland-utils"),
    (CommandKind::Run, "rustup toolchain install 1.99 --component rustfmt,clippy"),
];

/// RUN_4 as WadOS once had it: rustup reads the second component as a
/// toolchain's name (`invalid toolchain name: 'clippy'`), so it never built.
pub const BROKEN_RUSTUP: &str = "rustup toolchain install 1.99 --component rustfmt clippy";

/// A wadspace's commands with WadOS's broken one put right (it was ours, so
/// only that exact line is): true if it changed.
pub fn fix_commands(w: &mut crate::types::Wadspace) -> bool {
    let mut fixed = false;
    for c in w.commands.iter_mut().filter(|c| c.kind == CommandKind::Run && c.text == BROKEN_RUSTUP) {
        c.text = WADOS_COMMANDS[3].1.to_string();
        fixed = true;
    }
    fixed
}

/// WadOS's commands, named (RUN_1…).
pub fn wados_commands() -> Vec<BuildCommand> {
    named_commands(&WADOS_COMMANDS, |i| format!("cmd-wados-{}", i + 1))
}

/// The WadOS wadspace, on `machine`.
pub fn starter_wadspace(machine: &str, now: i64) -> crate::types::Wadspace {
    let mut x = blank(STARTER_ID, "WadOS", machine, now, 0);
    x.apps = vec![AppIn::new(AppId::Vscode), AppIn::new(AppId::Wadbrowser), AppIn::new(AppId::Terminal)];
    x.exts = vec![
        Ext::new(AppId::Vscode, "anthropic.claude-code"),
        Ext::new(AppId::Terminal, "rust"),
        // Nested containers: the host image is built and pushed from here.
        Ext::new(AppId::Terminal, "podman"),
    ];
    x.look.wallpaper = art_of("aurora");
    x.look.projects = vec![Project {
        id: "wadspaces".to_string(),
        as_name: "WadSpaces".to_string(),
        kind: ProjectKind::Github { repo: STARTER_REPO.to_string(), setup: Some("npm ci".to_string()) },
    }];
    x.look.autostart = vec!["app:vscode".to_string()];
    x.look.layout.icons =
        ["app:vscode", "app:wadbrowser", "app:terminal", "project:wadspaces"].map(String::from).to_vec();
    x.agents = true;
    x.starter = true;
    x.commands = wados_commands();
    x
}

/// Give this machine its WadOS wadspace, if the library hasn't one (Set up's prebuilt wadspace). True if it was added.
pub fn add_starter(lib: &mut Library, ctx: &Ctx) -> bool {
    if lib.wadspaces.iter().any(|x| x.id == STARTER_ID) {
        return false;
    }
    lib.wadspaces.insert(0, starter_wadspace(&ctx.machine, ctx.now_ms));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wados_comes_with_what_compiling_wados_needs() {
        let os = starter_wadspace("m-surface", 0);
        assert_eq!(os.name, "WadOS");
        assert_eq!(
            os.commands.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["RUN_1", "RUN_2", "RUN_3", "RUN_4"]
        );
        let all: String = os.commands.iter().map(|c| format!(" {}", c.text)).collect();
        for pkg in [
            "gcc",
            "pkg-config",
            "gtk4-devel",
            "gtk4-layer-shell-devel",
            "libadwaita-devel",
            "pam-devel",
            "webkitgtk6.0-devel",
            "labwc",
            "wtype",
            // The host image is built and pushed here (podman, buildah, skopeo).
            "podman",
            "buildah",
            "skopeo",
        ] {
            assert!(all.contains(&format!(" {pkg}")), "{pkg}");
        }
        // Rust itself is its Rust tool (rustup, cargo); RUN_4 pins the Rust it compiles with on it.
        assert!(os.exts.contains(&Ext::new(AppId::Terminal, "rust")));
        assert!(os.exts.contains(&Ext::new(AppId::Terminal, "podman")));
        assert_eq!(os.commands[3].text, "rustup toolchain install 1.99 --component rustfmt,clippy");
        assert_eq!(crate::check::wadspace(&os), Ok(()));

        // WadOS as it once was: only its broken line is put right, once.
        let mut old = os.clone();
        old.commands[3].text = BROKEN_RUSTUP.to_string();
        old.commands.push(old.commands[0].clone());
        old.commands[4].text = "rustup toolchain install nightly --component miri clippy".into();
        assert!(fix_commands(&mut old));
        assert_eq!(old.commands[3].text, os.commands[3].text);
        assert_eq!(old.commands[4].text, "rustup toolchain install nightly --component miri clippy", "yours stays");
        assert!(!fix_commands(&mut old));
    }
}
