//! Packages: an app's, a tool's or a VS Code extension's files on their own
//! (v1's composable packages, from Debian to Fedora).
//!
//! Each is built once on the base by `wadspaces-pkg capture`, FROM scratch:
//! only the files its install added, plus fragments for what installs share
//! (the RPM headers it added, users, shells) under
//! `/usr/share/wadspaces/pkg/<id>/`. CI publishes them as
//! `ghcr.io/hunggod/wados-pkg-<id>:<abi>` ([`crate::index`] lists them);
//! wadd pulls them (or builds one here) and tags each by its image id
//! ([`local_ref`]). A wadspace's set is then the base with its packages
//! copied on and `wadspaces-pkg assemble`, then its own commands
//! ([`crate::sets`]): nothing it has is installed when its set is built.
//!
//! The copying goes through chain images: one per prefix of a set's
//! packages (the base, then each package copied on, one build each), named
//! by what's in it ([`chain_ref`]). A set starts FROM the longest chain the
//! machine has and copies only the rest ([`stack_order`]), so taking a
//! package out of the middle copies the ones above it again, and adding one
//! copies just that one.
//!
//! A package's id is its feature's name (`vscode`, `rust`…), `rpm-<app>` for
//! an app from Fedora's RPMs, or `ext-<marketplace id>` (lowercase) for a
//! VS Code extension.

use sha2::{Digest, Sha256};
use wad_model::{AppId, Wadspace, catalog};

use crate::labels;
use crate::recipes::{self, Feature, Recipe, Rpm};

/// Where CI publishes packages: `<REGISTRY>/wados-pkg-<id>:<abi>`.
pub const REGISTRY: &str = "ghcr.io/hunggod";
/// A package image's name, before its id ([`local_ref`], [`remote_ref`]).
pub const PREFIX: &str = "wados-pkg-";
/// Chain images: `<CHAIN>:<sha12 of the base and packages>`.
pub const CHAIN: &str = "localhost/wados-chain";
/// Where a package keeps its fragments and meta.json: `<PKG_DIR>/<id>/`.
pub const PKG_DIR: &str = "/usr/share/wadspaces/pkg";

/// What a package installs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A `wadspaces-feature` script.
    Feature(&'static Feature),
    /// A catalog app's RPMs (`wadspaces-dnf`).
    Rpm { app: AppId, rpm: &'static Rpm },
    /// A VS Code extension, by its marketplace id.
    Extension(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("WadOS doesn't know how to package {0:?}")]
    Unknown(String),
    #[error("{id} needs {missing} first")]
    Needs { id: String, missing: String },
}

/// A catalog app's package; none for one the base has (WadBrowser, the
/// terminal) or one not packaged for Fedora yet.
pub fn of_app(app: AppId) -> Option<String> {
    match recipes::app(app) {
        Recipe::Feature(f) => Some(f.name.to_string()),
        Recipe::Rpm(_) => Some(format!("rpm-{app}")),
        Recipe::Builtin { .. } | Recipe::Todo(_) => None,
    }
}

/// A command-line tool's package (tools are features).
pub fn of_tool(tool: &str) -> Option<String> {
    match recipes::tool(tool)? {
        Recipe::Feature(f) => Some(f.name.to_string()),
        _ => None,
    }
}

/// A VS Code extension's package.
pub fn of_extension(ext: &str) -> Option<String> {
    is_extension_id(ext).then(|| format!("ext-{}", ext.to_lowercase()))
}

/// A marketplace id: publisher.name, nothing a shell would read.
pub fn is_extension_id(id: &str) -> bool {
    let ok = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    id.len() <= 120 && id.split_once('.').is_some_and(|(a, b)| ok(a) && b.split('.').all(ok))
}

/// What package `id` installs, if it's one WadOS knows how to build.
pub fn source(id: &str) -> Option<Source> {
    if let Some(f) = recipes::feature_named(id) {
        return Some(Source::Feature(f));
    }
    if let Some(app) = id.strip_prefix("rpm-") {
        let app = AppId::parse(app)?;
        return match recipes::app(app) {
            Recipe::Rpm(rpm) => Some(Source::Rpm { app, rpm }),
            _ => None,
        };
    }
    let ext = id.strip_prefix("ext-")?;
    is_extension_id(ext).then(|| Source::Extension(ext.to_string()))
}

/// What a package is called: its app's, tool's or VS Code extension's name
/// in the catalog, else its id.
pub fn name(id: &str) -> String {
    if let Some(ext) = id.strip_prefix("ext-") {
        return catalog::extensions()
            .iter()
            .find(|e| e.id.to_lowercase() == ext)
            .map_or_else(|| ext.to_string(), |e| e.name.clone());
    }
    if let Some(app) = AppId::ALL.into_iter().find(|a| of_app(*a).as_deref() == Some(id)) {
        return catalog::app_name(app).to_string();
    }
    catalog::tool(id).map_or_else(|| id.to_string(), |t| t.name.clone())
}

/// Whether package `id`'s build asks GitHub's API: only then does it get
/// the github_token secret.
pub fn uses_github(id: &str) -> bool {
    recipes::feature_named(id).is_some_and(|f| f.github)
}

/// The packages `id` needs under it.
pub fn needs(id: &str) -> Vec<&'static str> {
    if id.starts_with("ext-") {
        return match recipes::app(AppId::Vscode) {
            Recipe::Feature(f) => vec![f.name],
            _ => vec![],
        };
    }
    recipes::feature_named(id).map(|f| f.needs.to_vec()).unwrap_or_default()
}

/// Where a package goes in WadOS's order: features in their order (what
/// they need first, the big, common ones low), then Fedora's apps, then
/// extensions, each by id.
fn rank(id: &str) -> (u8, usize, &str) {
    match recipes::ORDER.iter().position(|f| *f == id) {
        Some(i) => (0, i, id),
        None if id.starts_with("rpm-") => (1, 0, id),
        None if id.starts_with("ext-") => (2, 0, id),
        None => (3, 0, id),
    }
}

/// `ids` in WadOS's order, once each.
pub fn canonical<S: AsRef<str>>(ids: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        if !out.iter().any(|o| o == id.as_ref()) {
            out.push(id.as_ref().to_string());
        }
    }
    out.sort_by(|a, b| rank(a).cmp(&rank(b)));
    out
}

/// `ids` and everything they need, in WadOS's order.
pub fn closure<S: AsRef<str>>(ids: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut todo: Vec<String> = ids.iter().map(|s| s.as_ref().to_string()).collect();
    while let Some(id) = todo.pop() {
        if out.contains(&id) {
            continue;
        }
        todo.extend(needs(&id).into_iter().map(String::from));
        out.push(id);
    }
    canonical(&out)
}

/// The order a set stacks `wanted` in: the longest chain the machine
/// already stores (`stored`: its chains' packages, bottom first, all on the
/// current base) made only of wanted packages, then the rest in WadOS's
/// order. Any order gives the same files; this one copies the least.
pub fn stack_order(wanted: &[String], stored: &[Vec<String>]) -> Vec<String> {
    let mut best: &[String] = &[];
    for chain in stored {
        let n = chain.iter().take_while(|id| wanted.contains(id)).count();
        if n > best.len() {
            best = &chain[..n];
        }
    }
    let rest: Vec<&String> = wanted.iter().filter(|id| !best.contains(id)).collect();
    best.iter().cloned().chain(canonical(&rest)).collect()
}

/// A package's image on this machine, by its image id (sha256 hex): what a
/// set copies from. Ids survive the stick's re-compression; registry
/// digests don't.
pub fn local_ref(id: &str, image_id: &str) -> String {
    let short: String = image_id.trim_start_matches("sha256:").chars().take(12).collect();
    format!("localhost/{PREFIX}{id}:{short}")
}

/// A package as CI publishes it, for the base's ABI.
pub fn remote_ref(id: &str, abi: &str) -> String {
    format!("{REGISTRY}/{PREFIX}{id}:{abi}")
}

/// A package's meta.json, in it (what CI reads after a capture).
pub fn meta_path(id: &str) -> String {
    format!("{PKG_DIR}/{id}/meta.json")
}

/// The command that installs it, as `wadspaces-pkg capture` runs it.
fn install_command(src: &Source) -> Vec<String> {
    match src {
        Source::Feature(f) => vec!["wadspaces-feature".into(), f.name.into()],
        Source::Rpm { app, rpm } => {
            let mut dnf = vec!["wadspaces-dnf".to_string(), app.to_string()];
            if let Some(d) = rpm.desktop {
                dnf.extend(["--desktop".into(), d.to_string()]);
            }
            dnf.extend(rpm.packages.iter().map(|p| p.to_string()));
            vec!["sh".into(), "-c".into(), format!("{} && wadspaces-dnf --clean", dnf.join(" "))]
        }
        Source::Extension(e) => vec!["wadspaces-vscode-extensions".into(), e.clone()],
    }
}

/// How to tell the installed version, for the package's label (run with `sh -c`).
fn probe(id: &str, src: &Source) -> Option<String> {
    match src {
        Source::Feature(f) => f.probe.map(String::from),
        Source::Rpm { rpm, .. } => rpm.packages.first().map(|p| format!("rpm -q --qf '%{{VERSION}}-%{{RELEASE}}' {p}")),
        Source::Extension(_) => Some(format!(
            "python3 -c 'import json; print(json.load(open(\"/usr/share/wadspaces/vscode-extensions.d/{}.json\"))[\"version\"])'",
            &id[4..]
        )),
    }
}

/// JSON exec form, so nothing in it is read by a shell.
fn exec(words: &[String]) -> String {
    serde_json::to_string(words).unwrap_or_default()
}

pub(crate) fn quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// A package's Containerfile: the base, the packages under it (`needs`:
/// (id, image ref) for everything it needs, theirs too, in WadOS's order),
/// and the capture. `token`: a github_token secret is at hand; only a build
/// that asks GitHub's API ([`uses_github`]) gets it.
pub fn package_containerfile(id: &str, base: &str, needs: &[(String, String)], token: bool) -> Result<String, Error> {
    let src = source(id).ok_or_else(|| Error::Unknown(id.to_string()))?;
    let want = closure(&self::needs(id));
    let missing: Vec<&str> = want.iter().filter(|n| !needs.iter().any(|(i, _)| i == *n)).map(String::as_str).collect();
    if !missing.is_empty() {
        return Err(Error::Needs { id: id.to_string(), missing: missing.join(", ") });
    }
    Ok(capture_file(id, &src, base, needs, token && uses_github(id)))
}

fn capture_file(id: &str, src: &Source, base: &str, needs: &[(String, String)], token: bool) -> String {
    let what = match src {
        Source::Feature(f) => format!("wadspaces-feature {}", f.name),
        Source::Rpm { app, .. } => format!("{app} from Fedora"),
        Source::Extension(e) => format!("the VS Code extension {e}"),
    };
    let mut lines = vec![
        format!("# The {id} package: what {what} adds to the base. Generated by wad-gen."),
        format!("ARG BASE_IMAGE={base}"),
        "FROM ${BASE_IMAGE} AS build".into(),
    ];
    for (_, r) in needs {
        lines.push(format!("COPY --from={r} / /"));
    }
    if !needs.is_empty() {
        lines.push("RUN wadspaces-pkg assemble".into());
    }
    let mut capture = vec!["wadspaces-pkg".to_string(), "capture".into(), id.into(), "--out".into(), "/out".into()];
    let names: Vec<&str> = needs.iter().map(|(i, _)| i.as_str()).collect();
    if !names.is_empty() {
        capture.extend(["--needs".into(), names.join(",")]);
    }
    if let Some(p) = probe(id, src) {
        capture.extend(["--probe".into(), p]);
    }
    capture.push("--".into());
    capture.extend(install_command(src));
    let mount = if token { "--mount=type=secret,id=github_token " } else { "" };
    lines.push(format!("RUN {mount}{}", exec(&capture)));
    lines.extend([
        String::new(),
        "FROM scratch".into(),
        "COPY --from=build /out/ /".into(),
        format!("LABEL {}={} {}={}", labels::PKG, quote(id), labels::PKG_NEEDS, quote(&names.join(","))),
    ]);
    lines.join("\n") + "\n"
}

/// The chain image for the base (its image id) with `pkgs` ((id, image
/// ref), bottom first) copied on: the same packages on the same base are
/// the same chain.
pub fn chain_ref(base_id: &str, pkgs: &[(String, String)]) -> String {
    let mut h = Sha256::new();
    h.update(base_id.trim_start_matches("sha256:").as_bytes());
    for (_, r) in pkgs {
        h.update(b"|");
        h.update(r.as_bytes());
    }
    let hex: String = h.finalize().iter().take(6).map(|b| format!("{b:02x}")).collect();
    format!("{CHAIN}:{hex}")
}

/// One step of a chain: `from` (the base or the chain under it; `from_id`
/// its image id) with a package copied on. Labelled with the chain so far,
/// so wadd can tell what chains it stores, and on what.
pub fn chain_step(from: &str, from_id: &str, pkg_ref: &str, chain: &[(String, String)]) -> String {
    format!(
        "# A chain image. Generated by wad-gen.\nFROM {from}\nCOPY --from={pkg_ref} / /\nLABEL {}={} {}={}\n",
        labels::CHAIN,
        quote(&pkgs_label(chain)),
        labels::FROM,
        quote(from_id)
    )
}

/// The `io.wadspaces.pkgs` (and `.chain`) label: `id@image-id-12,…` in stack order.
pub fn pkgs_label(pkgs: &[(String, String)]) -> String {
    pkgs.iter()
        .map(|(id, r)| format!("{id}@{}", r.rsplit_once(':').map(|(_, t)| t).unwrap_or("")))
        .collect::<Vec<_>>()
        .join(",")
}

/// Reads an `io.wadspaces.pkgs` label back: package ids, bottom first.
pub fn pkgs_of_label(label: &str) -> Vec<String> {
    label.split(',').filter_map(|p| p.split('@').next()).filter(|p| !p.is_empty()).map(String::from).collect()
}

/// Every package the catalog can ask for that WadOS can build (its apps',
/// tools' and extensions', and what they need), in WadOS's order: what CI
/// builds.
pub fn all() -> Vec<String> {
    let mut want: Vec<String> = catalog::apps().iter().filter_map(|a| of_app(a.id)).collect();
    want.extend(catalog::tools().iter().filter_map(|t| of_tool(&t.id)));
    want.extend(catalog::extensions().iter().filter_map(|e| of_extension(&e.id)));
    closure(&want)
}

/// `ids` (and what they need) in waves for CI: each wave's packages need
/// only earlier waves'.
pub fn waves<S: AsRef<str>>(ids: &[S]) -> Vec<Vec<String>> {
    let all = closure(ids);
    let mut done: Vec<String> = Vec::new();
    let mut out = Vec::new();
    while done.len() < all.len() {
        let wave: Vec<String> = all
            .iter()
            .filter(|id| !done.contains(id) && needs(id).iter().all(|n| done.iter().any(|d| d == n)))
            .cloned()
            .collect();
        if wave.is_empty() {
            break;
        }
        done.extend(wave.iter().cloned());
        out.push(wave);
    }
    out
}

/// The packages a wadspace's set is made of: each app's, tool's and
/// extension's, with what they need, in WadOS's order. (An add-on whose app
/// isn't in it is left out; web apps, games and variants aren't packages.)
pub fn packages_of(ws: &Wadspace) -> Vec<String> {
    let mut ids: Vec<String> = ws.apps.iter().filter_map(|a| of_app(a.app)).collect();
    for e in ws.exts.iter().filter(|e| ws.has_app(e.app)) {
        ids.extend(match e.app {
            AppId::Vscode => of_extension(&e.id),
            AppId::Terminal => of_tool(&e.id),
            _ => None,
        });
    }
    closure(&ids)
}

/// What a wadspace has that WadOS can't package yet, so its set is built
/// without it: apps ('app:obsidian') and add-ons ('terminal:firebase').
pub fn unpackaged(ws: &Wadspace) -> Vec<String> {
    let mut out: Vec<String> = ws
        .apps
        .iter()
        .filter(|a| matches!(recipes::app(a.app), Recipe::Todo(_)))
        .map(|a| format!("app:{}", a.app))
        .collect();
    for e in ws.exts.iter().filter(|e| ws.has_app(e.app)) {
        let missing = match e.app {
            AppId::Vscode => of_extension(&e.id).is_none(),
            AppId::Terminal => of_tool(&e.id).is_none(),
            _ => false,
        };
        if missing {
            out.push(format!("{}:{}", e.app, e.id));
        }
    }
    out
}

#[cfg(test)]
mod tests;
