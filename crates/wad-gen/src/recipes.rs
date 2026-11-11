//! WadOS's own recipes: how each of the catalog's apps and command-line
//! tools gets into a Fedora 44 image. (The catalog's `install` is v1's,
//! Debian's packages and .debs, so nothing here reads it.) VS Code's
//! extensions need no table: any marketplace id is a package
//! ([`crate::packages::of_extension`]).
//!
//! A recipe is a feature script of the base's (`wadspaces-feature <name>`,
//! `/usr/local/lib/wadspaces/features/<name>.sh`, v1's scripts ported from
//! apt to dnf), or an app's RPMs from Fedora (`wadspaces-dnf`, v1's
//! `wadspaces-apt`), or in the base already. What isn't packaged for Fedora
//! yet says what it's meant to be ([`Recipe::Todo`]); a wadspace with one of
//! those is built without it ([`crate::packages::unpackaged`] says so).

use wad_model::AppId;

/// How an app or a tool gets into an image.
#[derive(Debug, PartialEq, Eq)]
pub enum Recipe {
    /// In the base image already: its launcher.
    Builtin { desktop: &'static str },
    /// A feature script of the base's; its package is the feature's name.
    Feature(Feature),
    /// Fedora's RPMs (an app's: its package is `rpm-<app>`).
    Rpm(Rpm),
    /// Not packaged for Fedora yet: what it's meant to be.
    Todo(&'static str),
}

/// A feature script.
#[derive(Debug, PartialEq, Eq)]
pub struct Feature {
    /// Its script's name (and its package's id).
    pub name: &'static str,
    /// The features its package needs under it: its script's `need` lines
    /// (a package carries only its own files, so these come along).
    pub needs: &'static [&'static str],
    /// The launcher it writes, for an app.
    pub desktop: Option<&'static str>,
    /// How to tell the version it installed, for its package's label.
    pub probe: Option<&'static str>,
    /// Its script asks GitHub's API for a release (`github_latest_asset`):
    /// only then does its build get the github_token secret.
    pub github: bool,
}

/// An app's RPMs.
#[derive(Debug, PartialEq, Eq)]
pub struct Rpm {
    /// Fedora's packages; the first one's version is the package's.
    pub packages: &'static [&'static str],
    /// The launcher they install.
    pub desktop: Option<&'static str>,
}

const fn feature(name: &'static str, needs: &'static [&'static str], probe: &'static str) -> Recipe {
    Recipe::Feature(Feature { name, needs, desktop: None, probe: Some(probe), github: false })
}

/// Every app's recipe, in the catalog's order.
pub static APPS: [(AppId, Recipe); 13] = [
    // The base's (images/base): its launcher is named after its app id, as
    // its windows' Wayland app_id is, so desktops match them by name.
    (AppId::Wadbrowser, Recipe::Builtin { desktop: "io.wados.Browser.desktop" }),
    // Microsoft's dnf repo (packages.microsoft.com/yumrepos/vscode): its RPM
    // brings the libraries Electron needs. The script wraps it as v1's did
    // (--no-sandbox, --password-store=basic) and writes this launcher.
    (
        AppId::Vscode,
        Recipe::Feature(Feature {
            name: "vscode",
            needs: &[],
            desktop: Some("wadspaces-vscode.desktop"),
            probe: Some("code --version --no-sandbox --user-data-dir /tmp/wadspaces-probe"),
            github: false,
        }),
    ),
    (
        AppId::Obsidian,
        Recipe::Todo("its AppImage from GitHub's releases, as v1's feature (github, Electron's libraries)"),
    ),
    (AppId::Tiled, Recipe::Todo("Fedora's tiled (rpm-tiled), or v1's AppImage")),
    (AppId::Libresprite, Recipe::Todo("its AppImage from GitHub's releases, as v1's feature (github)")),
    (AppId::Audacity, Recipe::Todo("Fedora's audacity (rpm-audacity), or v1's AppImage")),
    (AppId::Lmms, Recipe::Todo("Fedora's lmms (rpm-lmms)")),
    // Fedora's: newer than Debian's, so not v1's AppImage.
    (AppId::Krita, Recipe::Rpm(Rpm { packages: &["krita"], desktop: Some("org.kde.krita.desktop") })),
    (AppId::Openshot, Recipe::Todo("Fedora's openshot (rpm-openshot)")),
    (
        AppId::Steam,
        Recipe::Feature(Feature {
            name: "steam",
            needs: &[],
            desktop: Some("steam.desktop"),
            probe: Some("rpm -q --qf '%{VERSION}' steam"),
            github: false,
        }),
    ),
    (
        AppId::Dolphin,
        Recipe::Feature(Feature {
            name: "dolphin",
            needs: &[],
            desktop: Some("dolphin-emu.desktop"),
            probe: Some("rpm -q --qf '%{VERSION}' dolphin-emu"),
            github: false,
        }),
    ),
    (AppId::Obs, Recipe::Todo("Fedora's obs-studio (rpm-obs; RPM Fusion's obs-studio-freeworld for its codecs)")),
    (AppId::Terminal, Recipe::Builtin { desktop: "foot.desktop" }),
];

/// The command line's tools: each a feature script (its package is its id).
/// What each script installs on Fedora 44 is noted beside it.
pub static TOOLS: [(&str, Recipe); 11] = [
    // dnf: gcc gcc-c++ make cmake ninja-build gdb pkgconf-pkg-config clang-tools-extra (clang-format)
    ("cpp", feature("cpp", &[], "cmake --version")),
    // dnf: python3 python3-pip python3-devel (venv is python3's own)
    ("python", feature("python", &[], "python3 --version")),
    // dnf: nodejs22 nodejs22-bin nodejs22-npm nodejs22-npm-bin (Node 22 LTS; `node` and `npm`)
    ("nodejs", feature("nodejs", &[], "node --version")),
    ("firebase", Recipe::Todo("npm's firebase-tools, as v1's feature (needs nodejs)")),
    ("gcloud", Recipe::Todo("Google's dnf repo: google-cloud-cli (a feature: it adds the repo)")),
    // rustup and the stable toolchain into /usr/local/{rustup,cargo}, with
    // wrappers on PATH: v1's script as it was (it needs a C toolchain).
    ("rust", feature("rust", &["cpp"], "rustc --version")),
    ("android", Recipe::Todo("Java, Gradle, adb and the Android SDK's command-line tools, as v1's feature")),
    // Anthropic's native build (claude.ai/install.sh): no Node.js under it.
    ("claude-code", feature("claude-code", &[], "claude --version")),
    // A standalone binary from OpenRouter's GitHub releases (downloaded, not
    // the API: no token). It signs in with `ori login`.
    (
        "ori",
        Recipe::Feature(Feature {
            name: "ori",
            needs: &[],
            desktop: None,
            // `ori --version` answers JSON: its version line, as it prints it.
            probe: Some(
                "ORI_NO_UPDATE_CHECK=1 ori --version | sed -n 's/.*\"version\": *\"\\([^\"]*\\)\".*/\\1/p' | head -n1",
            ),
            github: false,
        }),
    ),
    // The nested container tools: podman, buildah and skopeo, rootless with
    // vfs, so a wadspace builds and pushes images (the WadOS one the host
    // image). Its unit gets what nested containers need (container_engine_t,
    // an unshared /proc, /dev/fuse): wad-gen's `units::nested`.
    ("podman", feature("podman", &[], "podman --version && buildah --version && skopeo --version")),
    ("hplip", Recipe::Todo("Fedora's hplip and cups-client (a feature)")),
];

/// Features in the order they stack: what others need first, the big,
/// common ones low. (Each one once: a test checks.)
pub const ORDER: &[&str] =
    &["cpp", "python", "nodejs", "rust", "claude-code", "ori", "podman", "dolphin", "steam", "vscode"];

/// An app's recipe (every app has one, if only a `Todo`).
pub fn app(id: AppId) -> &'static Recipe {
    &APPS.iter().find(|(a, _)| *a == id).expect("APPS has every app (a test checks)").1
}

/// A command-line tool's recipe.
pub fn tool(id: &str) -> Option<&'static Recipe> {
    TOOLS.iter().find(|(t, _)| *t == id).map(|(_, r)| r)
}

/// A feature script, the apps' or the tools'.
pub fn feature_named(name: &str) -> Option<&'static Feature> {
    features().find(|f| f.name == name)
}

/// Every feature script WadOS has a recipe for.
pub fn features() -> impl Iterator<Item = &'static Feature> {
    APPS.iter().map(|(_, r)| r).chain(TOOLS.iter().map(|(_, r)| r)).filter_map(|r| match r {
        Recipe::Feature(f) => Some(f),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_app_has_a_recipe_and_every_feature_its_place() {
        assert_eq!(APPS.iter().map(|(a, _)| *a).collect::<Vec<_>>(), AppId::ALL);
        let mut names: Vec<&str> = features().map(|f| f.name).collect();
        names.sort_unstable();
        let mut order = ORDER.to_vec();
        order.sort_unstable();
        assert_eq!(names, order, "ORDER has every feature once");
        // What a feature needs is a feature, lower in the order.
        for f in features() {
            let at = ORDER.iter().position(|o| *o == f.name);
            for n in f.needs {
                assert!(ORDER.iter().position(|o| o == n) < at, "{} needs {n}", f.name);
            }
        }
        // A tool's feature is named for it.
        for (t, r) in &TOOLS {
            if let Recipe::Feature(f) = r {
                assert_eq!(f.name, *t);
            }
        }
    }

    #[test]
    fn which_catalog_ids_still_lack_a_recipe() {
        let apps: Vec<&str> = wad_model::catalog::apps()
            .iter()
            .filter(|a| matches!(app(a.id), Recipe::Todo(_)))
            .map(|a| a.id.as_str())
            .collect();
        assert_eq!(apps, ["obsidian", "tiled", "libresprite", "audacity", "lmms", "openshot", "obs"]);
        let tools: Vec<&str> = wad_model::catalog::tools()
            .iter()
            .filter(|t| !matches!(tool(&t.id), Some(Recipe::Feature(_) | Recipe::Rpm(_))))
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(tools, ["firebase", "gcloud", "android", "hplip"]);
        // Every VS Code extension the catalog offers is a package.
        for e in wad_model::catalog::extensions() {
            assert!(crate::packages::of_extension(&e.id).is_some(), "{}", e.id);
        }
        // Phase 1's: the terminal tools, VS Code (and its extensions), Krita.
        for t in ["rust", "nodejs", "python", "cpp"] {
            assert!(matches!(tool(t), Some(Recipe::Feature(_))), "{t}");
        }
        assert!(matches!(app(AppId::Vscode), Recipe::Feature(_)));
        assert!(matches!(app(AppId::Krita), Recipe::Rpm(_)));
    }

    /// The base's feature scripts (images/base, ported from v1) and this
    /// table agree on what each needs: a script's `need` lines are its
    /// package's needs. Only the scripts that are there yet are read.
    #[test]
    fn feature_needs_are_the_scripts_need_lines() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../images/base/root/usr/local/lib/wadspaces/features");
        for f in features() {
            let Ok(text) = std::fs::read_to_string(dir.join(format!("{}.sh", f.name))) else { continue };
            let mut found: Vec<&str> =
                text.lines().filter_map(|l| l.trim().strip_prefix("need ")).map(str::trim).collect();
            found.sort_unstable();
            let mut want = f.needs.to_vec();
            want.sort_unstable();
            assert_eq!(found, want, "{}.sh's need lines and its recipe's needs differ", f.name);
            assert_eq!(text.contains("github_latest_asset"), f.github, "{}.sh and its recipe's github", f.name);
        }
    }
}
