//! What wadd and CI generate for a wadspace (a v6 [`wad_model::Wadspace`])
//! on a machine: its quadlet units, its packages and set, and its extra
//! folder. Text only: no podman, no files (wadd does those), no clocks.
//!
//! It's the port of v1's `wad-core` generators (`generator.rs`,
//! `packages.rs`, `spec.rs`, and what they used of `layers.rs` and
//! `v5.rs`), from Debian trixie to Fedora 44:
//!
//! - [`recipes`]: how each of the catalog's apps and tools gets into an
//!   image on Fedora (a feature script of the base's, or an app's RPMs),
//!   what isn't packaged yet, and the features' order and needs.
//! - [`packages`]: package ids (`<feature>`, `rpm-<app>`, `ext-<id>`), what
//!   needs what, closures and CI's waves, the capture Containerfile
//!   (`wadspaces-pkg capture`), chain images, and which packages a
//!   wadspace's set is made of.
//! - [`index`]: CI's index.json of what it published.
//! - [`sets`]: a wadspace's set: its key (packages, base, commands), its
//!   image, whether it's built here (built, update, never), and its
//!   Containerfile (its packages put together, then its RUN and COPY steps,
//!   and what wadd stages for its COPYs).
//! - [`extras`]: its extra folder, `/run/wadspaces-extra` (web apps, the
//!   desktop's layout, keys, projects, WadBrowser's mode, the wallpaper).
//! - [`units`]: its quadlet units (SELinux categories, id maps, the screen
//!   folder, sound, keys, projects, devices) and its stream sidecar's.
//! - [`labels`]: the labels images and containers carry.
//!
//! Where it isn't v1:
//!
//! - **Fedora.** `apt-<app>` packages are `rpm-<app>` (`wadspaces-dnf`, v1's
//!   `wadspaces-apt` command line); a package's version is `rpm -q`'s.
//!   Images are `localhost/wados-*:44` and packages
//!   `ghcr.io/hunggod/wados-pkg-*`. The catalog's `install` (Debian's) isn't
//!   read: [`recipes`] is WadOS's own, and what's not in it yet is built
//!   without ([`packages::unpackaged`]). VS Code is Microsoft's RPM, which
//!   brings its own libraries: no `electron-deps`, and no `git` feature (the
//!   base has git).
//! - **A set has the wadspace's commands on top**, so its key covers them
//!   and the base's id too: a set on an older base is another key, which
//!   [`sets::Set::state`] calls an update (v1 named sets by their packages
//!   alone, and compared the base it was built on).
//! - **Version 6 wadspaces**, typed: one look, keys by slot and WadBrowser's
//!   groups, no owner, descriptions, passwords, themes or textures. The
//!   extra folder has no `theme` (dark only) and the wallpaper is a picture
//!   copied in or a colour passed as it is (nothing drawn).
//! - **Units:** no fallback to the session's whole runtime dir (no screen
//!   folder, no display), no ~/Drives, `Pull=never` (sets are built here),
//!   values with `%` escaped, and a mount whose path or name has a `:` (which
//!   podman's `-v` can't say) isn't mounted. The stream sidecar has no login
//!   (wadd's proxy and Selkies 2.x's tokens, phase 7) and its TURN
//!   credentials are a podman secret wadd mints, never in the unit. A
//!   wadspace that moves between the screen and a stream (v1's
//!   `moving_quadlets`) is phase 7's.

use std::collections::BTreeMap;

pub mod extras;
pub mod index;
pub mod labels;
pub mod packages;
pub mod recipes;
pub mod sets;
pub mod units;

pub use sets::{BASE_IMAGE, Set, SetBuild, SetFile, Stage};
pub use units::{Machine, Run, Sidecar};

/// A wadspace's projects found or made on this machine: project id → its
/// folder or file here (a mount's own folder or file, or its Taildrive
/// mount; a GitHub project's clone; a copy's folder, `files/` and `done`).
/// One not here isn't mounted (nor listed inside).
pub type Mounts = BTreeMap<String, String>;

/// Whether a mount can be: podman's `-v` reads a `:` in its path or its
/// name inside as a separator, and a line break would end the unit's line.
pub fn mountable(path: &str, name: &str) -> bool {
    let ok = |s: &str| !s.is_empty() && !s.contains(':') && !s.chars().any(char::is_control);
    ok(path) && ok(name)
}
