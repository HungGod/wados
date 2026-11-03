// What a wadspace's document may have in it: ids of a few safe characters,
// names of 1 to 80 characters on one line, a project's name that's a name
// (never a path out of its folder), commands of one line, and no more of
// anything than a person would make. Edits check the wadspaces they change;
// a machine checks a document it's sent before it keeps it.

use crate::catalog::url_host;
use crate::edit::Library;
use crate::types::{KeyRef, Project, ProjectKind, ProjectSource, VERSION, Wadspace, Wallpaper};

/// The longest a name can be, in characters.
pub const MAX_NAME: usize = 80;
/// The longest an id can be.
pub const MAX_ID: usize = 128;
/// The longest a command (or a project's setup) can be, in bytes.
pub const MAX_COMMAND: usize = 4096;
/// The longest a path, a picture or a web address can be, in bytes.
pub const MAX_VALUE: usize = 4096;
/// The longest a project's name (its folder or file inside the wadspace) can be.
pub const MAX_FILE_NAME: usize = 255;

pub const MAX_APPS: usize = 32;
pub const MAX_EXTS: usize = 256;
pub const MAX_KEYS: usize = 64;
pub const MAX_MEMBERS: usize = 256;
pub const MAX_PROJECTS: usize = 256;
pub const MAX_PATHS: usize = 64;
pub const MAX_ICONS: usize = 1024;
pub const MAX_COMMANDS: usize = 64;
pub const MAX_TAGS: usize = 256;
pub const MAX_WADSPACES: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckError {
    #[error("A name is 1 to {MAX_NAME} characters, on one line")]
    Name,
    #[error("{what} {id:?} isn’t an id")]
    Id { what: &'static str, id: String },
    #[error("Two {what} have the id {id:?}")]
    Twice { what: &'static str, id: String },
    #[error("Too many {what} (at most {max})")]
    TooMany { what: &'static str, max: usize },
    #[error("{0}")]
    Bad(String),
}

fn bad(s: impl Into<String>) -> CheckError {
    CheckError::Bad(s.into())
}

fn most(what: &'static str, n: usize, max: usize) -> Result<(), CheckError> {
    if n > max { Err(CheckError::TooMany { what, max }) } else { Ok(()) }
}

/// A name, trimmed: 1 to 80 characters, nothing that isn't printable.
pub fn name(s: &str) -> Result<String, CheckError> {
    let t = s.trim();
    let n = t.chars().count();
    if n == 0 || n > MAX_NAME || t.chars().any(char::is_control) {
        return Err(CheckError::Name);
    }
    Ok(t.to_string())
}

/// An id: 1 to 128 of a-z, A-Z, 0-9, '-', '_' and '.', starting with a letter or a digit.
pub fn id(what: &'static str, s: &str) -> Result<(), CheckError> {
    let ok = !s.is_empty()
        && s.len() <= MAX_ID
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if ok { Ok(()) } else { Err(CheckError::Id { what, id: s.to_string() }) }
}

/// A line of text: at most `max` bytes, no line breaks or other control characters.
fn line(what: &str, s: &str, max: usize) -> Result<(), CheckError> {
    if s.len() > max {
        return Err(bad(format!("{what} is too long")));
    }
    if s.chars().any(char::is_control) {
        return Err(bad(format!("{what} is more than one line")));
    }
    Ok(())
}

/// A key ('icon', 'autostart', 'launch'): 'app:<id>', 'web:<id>', 'variant:<id>' or 'project:<id>'.
fn ref_key(what: &'static str, s: &str) -> Result<(), CheckError> {
    match s.split_once(':') {
        Some(("app" | "web" | "variant" | "project", rest)) => id(what, rest),
        _ => Err(CheckError::Id { what, id: s.to_string() }),
    }
}

pub fn wallpaper(w: &Wallpaper) -> Result<(), CheckError> {
    match w {
        Wallpaper::Empty => Ok(()),
        Wallpaper::Color { value } if value.is_empty() || value.len() > 64 => Err(bad("That isn’t a colour")),
        Wallpaper::Image { value } if value.is_empty() => Err(bad("That isn’t a picture")),
        Wallpaper::Color { value } | Wallpaper::Image { value } => line("A background", value, MAX_VALUE),
    }
}

/// What a project is called inside its wadspace: a folder's or a file's name, never a path.
pub fn file_name(s: &str) -> Result<(), CheckError> {
    let ok = !s.is_empty()
        && s.len() <= MAX_FILE_NAME
        && s != "."
        && s != ".."
        && !s.contains(['/', '\\'])
        && !s.chars().any(char::is_control);
    if ok { Ok(()) } else { Err(bad(format!("{s:?} isn’t a file’s name"))) }
}

fn source(s: &ProjectSource) -> Result<(), CheckError> {
    id("machine", &s.machine_id)?;
    if s.paths.is_empty() {
        return Err(bad("A folder or file comes from somewhere"));
    }
    most("paths", s.paths.len(), MAX_PATHS)?;
    for p in &s.paths {
        if p.is_empty() {
            return Err(bad("A path is empty"));
        }
        line("A path", p, MAX_VALUE)?;
    }
    Ok(())
}

pub fn project(p: &Project) -> Result<(), CheckError> {
    id("project", &p.id)?;
    file_name(&p.as_name)?;
    match &p.kind {
        ProjectKind::Github { repo, setup } => {
            let ok =
                repo.split_once('/').is_some_and(|(o, n)| crate::ops::is_repo_part(o) && crate::ops::is_repo_part(n));
            if !ok {
                return Err(bad(format!("{repo:?} isn’t a repository")));
            }
            if let Some(s) = setup {
                line("Its setup", s, MAX_COMMAND)?;
            }
            Ok(())
        }
        ProjectKind::Mount { source: s, .. } | ProjectKind::Copy { source: s } => source(s),
    }
}

fn key_ref(k: &KeyRef) -> Result<(), CheckError> {
    id("key", &k.id)?;
    name(&k.name).map(|_| ())
}

fn unique<'a>(what: &'static str, ids: impl IntoIterator<Item = &'a str>) -> Result<(), CheckError> {
    let mut seen: Vec<&str> = vec![];
    for i in ids {
        if seen.contains(&i) {
            return Err(CheckError::Twice { what, id: i.to_string() });
        }
        seen.push(i);
    }
    Ok(())
}

/// A wadspace's document, all of it.
pub fn wadspace(x: &Wadspace) -> Result<(), CheckError> {
    if x.version != VERSION {
        return Err(bad(format!("A wadspace is version {VERSION}, not {}", x.version)));
    }
    id("wadspace", &x.id)?;
    id("machine", &x.machine)?;
    name(&x.name)?;
    if let Some(t) = &x.tag {
        id("tag", t)?;
    }

    most("apps", x.apps.len(), MAX_APPS)?;
    let apps: Vec<&str> = x.apps.iter().map(|a| a.app.as_str()).collect();
    unique("apps", apps.iter().copied())?;
    for a in &x.apps {
        if let Some(w) = &a.wallpaper {
            wallpaper(w)?;
        }
    }

    most("add-ons", x.exts.len(), MAX_EXTS)?;
    let keys: Vec<String> = x.exts.iter().map(crate::ops::ext_key).collect();
    unique("add-ons", keys.iter().map(String::as_str))?;
    for e in &x.exts {
        id("add-on", &e.id)?;
        if !x.has_app(e.app) {
            return Err(bad(format!("{} is an add-on of {}, and it isn’t in it", e.id, e.app)));
        }
        if let Some(n) = &e.name {
            name(n)?;
        }
        if let Some(u) = &e.url {
            line("A web address", u, MAX_VALUE)?;
            if url_host(u).is_none() {
                return Err(bad(format!("{u:?} isn’t a web address")));
            }
        }
        if let Some(u) = &e.icon_url {
            line("An icon", u, MAX_VALUE)?;
        }
        if let Some(w) = &e.wallpaper {
            wallpaper(w)?;
        }
    }

    most("sign-ins", x.secrets.by_slot.len(), MAX_KEYS)?;
    for (slot, k) in &x.secrets.by_slot {
        match slot.split_once(':') {
            Some(("app" | "ext", rest)) => id("slot", rest)?,
            _ => return Err(CheckError::Id { what: "slot", id: slot.clone() }),
        }
        key_ref(k)?;
    }
    most("sign-in groups", x.secrets.groups.len(), MAX_KEYS)?;
    for g in &x.secrets.groups {
        if let Some(gid) = &g.id {
            id("sign-in group", gid)?;
        }
        most("members", g.members.len(), MAX_MEMBERS)?;
        for m in &g.members {
            if m != "browser" {
                match m.strip_prefix("web:") {
                    Some(rest) => id("member", rest)?,
                    None => return Err(CheckError::Id { what: "member", id: m.clone() }),
                }
            }
        }
        key_ref(&g.key)?;
    }

    let l = &x.look;
    wallpaper(&l.wallpaper)?;
    most("projects", l.projects.len(), MAX_PROJECTS)?;
    unique("projects", l.projects.iter().map(|p| p.id.as_str()))?;
    for p in &l.projects {
        project(p)?;
    }
    most("autostart", l.autostart.len(), MAX_ICONS)?;
    for k in &l.autostart {
        ref_key("autostart", k)?;
    }
    most("icons", l.layout.icons.len(), MAX_ICONS)?;
    for k in &l.layout.icons {
        ref_key("icon", k)?;
    }
    if let Some(k) = &l.launch {
        ref_key("launch", k)?;
    }
    if let Some(i) = &l.icon {
        if !(i.zoom.is_finite() && i.x.is_finite() && i.y.is_finite()) {
            return Err(bad("An icon’s square is numbers"));
        }
        if let Some(w) = &i.picture {
            wallpaper(w)?;
        }
    }

    most("commands", x.commands.len(), MAX_COMMANDS)?;
    unique("commands", x.commands.iter().map(|c| c.id.as_str()))?;
    for c in &x.commands {
        id("command", &c.id)?;
        let numbered = c
            .name
            .strip_prefix(c.kind.word())
            .and_then(|r| r.strip_prefix('_'))
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if !numbered {
            return Err(bad(format!("{:?} isn’t a {} command’s name", c.name, c.kind.word())));
        }
        crate::commands::parse_command(c.kind, &c.text).map_err(|e| bad(e.to_string()))?;
    }
    Ok(())
}

/// A library: its wadspaces, tags, keys and web apps, each id once.
pub fn library(lib: &Library) -> Result<(), CheckError> {
    most("wadspaces", lib.wadspaces.len(), MAX_WADSPACES)?;
    unique("wadspaces", lib.wadspaces.iter().map(|x| x.id.as_str()))?;
    for x in &lib.wadspaces {
        wadspace(x)?;
    }
    most("tags", lib.tags.len(), MAX_TAGS)?;
    unique("tags", lib.tags.iter().map(|t| t.id.as_str()))?;
    for t in &lib.tags {
        id("tag", &t.id)?;
        name(&t.name)?;
        if t.color.is_empty() || t.color.len() > 64 || t.color.chars().any(char::is_control) {
            return Err(bad("That isn’t a colour"));
        }
    }
    unique("keys", lib.keys.iter().map(|k| k.id.as_str()))?;
    for k in &lib.keys {
        id("key", &k.id)?;
        name(&k.name)?;
    }
    unique("web apps", lib.web_apps.iter().map(|a| a.id.as_str()))?;
    for a in &lib.web_apps {
        id("web app", &a.id)?;
        name(&a.name)?;
        if url_host(&a.url).is_none() {
            return Err(bad(format!("{:?} isn’t a web address", a.url)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::blank;
    use crate::types::{AppId, AppIn, BuildCommand, CommandKind, Ext};

    #[test]
    fn names_and_ids() {
        assert_eq!(name("  Art studio "), Ok("Art studio".to_string()));
        assert_eq!(name(""), Err(CheckError::Name));
        assert_eq!(name(&"x".repeat(81)), Err(CheckError::Name));
        assert!(name(&"é".repeat(80)).is_ok());
        assert_eq!(name("two\nlines"), Err(CheckError::Name));
        assert!(id("wadspace", "cosmic-bodybuilding").is_ok());
        assert!(id("add-on", "anthropic.claude-code").is_ok());
        assert!(id("wadspace", "../etc").is_err());
        assert!(id("wadspace", "").is_err());
        assert!(id("wadspace", "a b").is_err());
    }

    #[test]
    fn a_projects_name_is_never_a_path() {
        assert!(file_name("Pitch deck.pdf").is_ok());
        for s in ["", ".", "..", "../x", "a/b", "a\\b", "a\0b"] {
            assert!(file_name(s).is_err(), "{s:?}");
        }
    }

    #[test]
    fn a_wadspace_is_checked_all_through() {
        let mut x = blank("a", "A", "m-surface", 0, 0);
        assert_eq!(wadspace(&x), Ok(()));
        x.exts.push(Ext::new(AppId::Vscode, "esbenp.prettier-vscode"));
        assert!(wadspace(&x).is_err(), "an add-on without its app");
        x.apps.push(AppIn::new(AppId::Vscode));
        assert_eq!(wadspace(&x), Ok(()));
        x.commands.push(BuildCommand {
            id: "c1".into(),
            name: "RUN_1".into(),
            kind: CommandKind::Run,
            text: "a\nb".into(),
        });
        assert!(wadspace(&x).is_err(), "a command of two lines");
        x.commands[0].text = "cargo install just".into();
        assert_eq!(wadspace(&x), Ok(()));
        x.commands[0].name = "COPY_1".into();
        assert!(wadspace(&x).is_err());
        x.commands.clear();
        x.version = 5;
        assert!(wadspace(&x).is_err());
    }
}
