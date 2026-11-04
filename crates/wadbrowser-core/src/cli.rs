//! What a launch asks for: v1's grammar (`wadspaces-old/apps/wadbrowser/src/cli.rs`),
//! plus private windows and sign-in windows. Launchers pass `%U` (any number
//! of addresses), and other browsers' launchers pass flags of their own:
//! unknown flags are ignored rather than refused, so a link always opens.
//!
//! ```text
//! wadbrowser [URL...]                        a browser window (tabs, address bar)
//! wadbrowser --no-urlbar [URL...]            Focus: tabs, no address bar (also --focus)
//! wadbrowser --default [URL...]              what links open in: wadbrowser.conf's
//!                                            `default = full|focus` (the link handler)
//! wadbrowser --private [URL...]              a private window (nothing kept)
//! wadbrowser --app <id> --name <name> --url <start> [--icon <png>] [URL...]
//!                                            a web app's window (app_id wadspaces-webapp-<id>)
//! wadbrowser --signin <github|tailscale> --url <url> [--code <code>]
//! wadbrowser --signin <github|tailscale> --show | --done
//!                                            a sign-in window (crate::signin)
//! wadbrowser --instance <id> [...]           a WadBrowser of its own (crate::instance)
//! wadbrowser --ephemeral [...]               a run that keeps nothing (as it starts;
//!                                            with --instance, a browser of its own)
//!   --new-window  --profile <name>  --gpu auto|on|off  --config <kalebrowser.json>
//!   --restore | --no-restore  -v, --verbose  -V, --version  -h, --help
//! ```
//!
//! A second launch hands its request to the browser already running (one
//! per user, or per instance), over D-Bus: a new tab in its last active
//! window that fits, a new window with `--new-window`, or the web app's own
//! window. v1's `--setup <file>` isn't kept: wadd hands a wadspace its web
//! apps, sign-ins and `wadbrowser.conf` in `/run/wadspaces-extra` instead.

use crate::signin::{self, Provider};
use std::path::Path;

/// The app_id prefix of web apps' windows (`wadspaces-webapp-<id>`).
pub const WEBAPP_PREFIX: &str = "wadspaces-webapp-";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    /// A web app's window: its app_id (`wadspaces-webapp-<id>`, or a
    /// KaleBrowser app's old WM class).
    pub app_id: Option<String>,
    pub name: Option<String>,
    /// A web app's start page (`--url`); a sign-in's address.
    pub start: Option<String>,
    /// Pages to open (the arguments; existing files as file:// URIs).
    pub urls: Vec<String>,
    pub no_urlbar: bool,
    /// The kind of window the wadspace chose (the link handler).
    pub default: bool,
    pub new_window: bool,
    pub private: bool,
    pub profile: Option<String>,
    /// A picture for the web app's window (absolute path).
    pub icon: Option<String>,
    pub gpu: Option<String>,
    /// A WadBrowser of its own.
    pub instance: Option<String>,
    /// A run that keeps nothing: every profile but a key's is in memory, and
    /// no session is saved (v1's ephemeral setup). It's how the browser
    /// starts; a later launch can't make a running one ephemeral.
    pub ephemeral: bool,
    pub signin: Option<signin::Request>,
    /// Cactus's: restore the last session at start (or not), whatever the settings say.
    pub restore: Option<bool>,
    pub verbose: bool,
    pub version: bool,
    pub help: bool,
    /// Flags it ignored (other browsers', v1's `--setup`), for the log.
    pub ignored: Vec<String>,
}

impl Request {
    /// Pages to open, without launchers' unexpanded field codes (`%U`).
    pub fn pages(&self) -> Vec<String> {
        self.urls.iter().filter(|u| !u.starts_with('%')).cloned().collect()
    }
}

pub const USAGE: &str = "\
usage: wadbrowser [--no-urlbar | --default] [--private] [--new-window] [URL...]
       wadbrowser --app <id> --name <name> --url <start> [--icon <png>] [URL...]
       wadbrowser --signin <github|tailscale> --url <url> [--code <code>]
       wadbrowser --signin <github|tailscale> --show | --done
       wadbrowser --instance <id> [--ephemeral] [...]
options: --profile <name>  --gpu auto|on|off  --config <kalebrowser.json>
         --restore | --no-restore  -v, --verbose  -V, --version  -h, --help";

pub fn help_text() -> String {
    format!(
        "WadBrowser, WadOS's web browser\n\n{USAGE}\n\nBased on Cactus ({}, {}).\n",
        crate::CACTUS_REPO,
        crate::CACTUS_LICENSE
    )
}

/// Reads `args` (without the program name); relative paths are `cwd`'s.
pub fn parse<I, S>(args: I, cwd: &Path) -> Result<Request, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut req = Request::default();
    let mut args = args.into_iter().map(Into::into).peekable();
    let mut only_urls = false;
    let (mut signin, mut show, mut done, mut code) = (None::<String>, false, false, None::<String>);
    while let Some(arg) = args.next() {
        if only_urls || !arg.starts_with('-') || arg == "-" {
            push_url(&mut req.urls, &arg, cwd);
            continue;
        }
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_owned(), Some(v.to_owned())),
            _ => (arg.clone(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            match inline.clone().or_else(|| args.next()) {
                Some(v) => Ok(v),
                None => Err(format!("{name} needs a value")),
            }
        };
        match flag.as_str() {
            "--" => only_urls = true,
            "--app" => {
                let id = value("--app")?;
                if !web_app_id(&id) {
                    return Err(format!("--app: {id:?} isn't an app id (a-z, 0-9, -)"));
                }
                req.app_id = Some(format!("{WEBAPP_PREFIX}{id}"));
            }
            "--name" => req.name = Some(value("--name")?),
            "--url" | "-u" => req.start = Some(value("--url")?),
            "--no-urlbar" | "--focus" => req.no_urlbar = true,
            "--default" => req.default = true,
            "--new-window" => req.new_window = true,
            "--private" | "-p" => req.private = true,
            "--profile" => req.profile = Some(value("--profile")?).filter(|p| web_app_id(p)),
            "--gpu" => req.gpu = Some(value("--gpu")?).filter(|g| matches!(g.as_str(), "auto" | "on" | "off")),
            "--icon" | "-I" => req.icon = Some(absolute(&value("--icon")?, cwd)),
            "--config" | "-c" => legacy_config(&absolute(&value("--config")?, cwd), &mut req)?,
            "--instance" => {
                let id = value("--instance")?;
                if !web_app_id(&id) {
                    return Err(format!("--instance: {id:?} isn't an id (a-z, 0-9, -)"));
                }
                req.instance = Some(id);
            }
            "--ephemeral" => req.ephemeral = true,
            "--signin" => signin = Some(value("--signin")?),
            "--show" => show = true,
            "--done" => done = true,
            "--code" => code = Some(value("--code")?),
            // v1's, from a wadd that wrote a setup file: its value isn't a page.
            "--setup" => {
                let file = value("--setup")?;
                req.ignored.push(format!("--setup {file}"));
            }
            "--restore" => req.restore = Some(true),
            "--no-restore" => req.restore = Some(false),
            "--verbose" | "-v" => req.verbose = true,
            "--version" | "-V" => req.version = true,
            "--help" | "-h" => req.help = true,
            // Other browsers' (--no-sandbox, --ozone-platform=…) and anything new.
            _ => req.ignored.push(arg),
        }
    }
    req.signin = sign_in(signin, show, done, code, &mut req)?;
    Ok(req)
}

/// The `--signin` flags, checked: one provider, one thing to do.
fn sign_in(
    provider: Option<String>,
    show: bool,
    done: bool,
    code: Option<String>,
    req: &mut Request,
) -> Result<Option<signin::Request>, String> {
    let Some(p) = provider else {
        if show || done || code.is_some() {
            return Err("--show, --done and --code go with --signin <provider>".into());
        }
        return Ok(None);
    };
    let provider = Provider::parse(&p).ok_or_else(|| format!("--signin: {p:?} isn't github or tailscale"))?;
    if req.app_id.is_some() {
        return Err("--signin and --app are different windows".into());
    }
    let action = match (show, done, req.start.take()) {
        (true, true, _) => return Err("--show or --done, not both".into()),
        (false, true, _) => signin::Action::Done,
        (true, false, _) => signin::Action::Show,
        (false, false, Some(url)) if !url.trim().is_empty() => {
            signin::Action::Open { url: url.trim().to_owned(), code: code.filter(|c| !c.trim().is_empty()) }
        }
        (false, false, _) => return Err(format!("--signin {p} needs --url <url>, --show or --done")),
    };
    Ok(Some(signin::Request { provider, action }))
}

/// An argument that's a page: as it is, or an existing file's file:// URI.
fn push_url(urls: &mut Vec<String>, raw: &str, cwd: &Path) {
    let arg = raw.trim();
    if arg.is_empty() {
        return;
    }
    let has_scheme = arg.split_once(':').is_some_and(|(s, _)| {
        s.len() > 1 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
    });
    if !has_scheme && !arg.starts_with('%') {
        let path = cwd.join(arg);
        if path.is_file()
            && let Ok(url) = url::Url::from_file_path(path.canonicalize().unwrap_or(path))
        {
            urls.push(url.into());
            return;
        }
    }
    urls.push(arg.to_owned());
}

/// A KaleBrowser app's config.json: {app_name, app_url, icon_path, wm_class}.
/// Its window keeps the old WM class, so its old launcher still matches it.
fn legacy_config(path: &str, req: &mut Request) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("--config {path}: {e}"))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("--config {path}: {e}"))?;
    let dir = Path::new(path).parent().unwrap_or(Path::new("/"));
    req.name = v["app_name"].as_str().map(str::to_owned).or(req.name.take());
    req.start = v["app_url"].as_str().map(str::to_owned).or(req.start.take());
    req.icon = v["icon_path"].as_str().map(|p| absolute(p, dir)).or(req.icon.take());
    req.app_id = v["wm_class"].as_str().filter(|c| valid_app_id(c)).map(str::to_owned).or(req.app_id.take());
    if req.app_id.is_none() {
        req.app_id = Some(format!("{WEBAPP_PREFIX}{}", slug(req.name.as_deref().unwrap_or("app"))));
    }
    Ok(())
}

fn absolute(p: &str, cwd: &Path) -> String {
    let p = Path::new(p);
    if p.is_absolute() { p.to_string_lossy().into() } else { cwd.join(p).to_string_lossy().into() }
}

/// WadSpaces' app ids: lower-case letters, digits and dashes.
pub fn web_app_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Whether `id` is fit to be a Wayland app_id (and a .desktop file's name).
pub fn valid_app_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() { "app".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;
    use signin::Action;

    fn p(args: &[&str]) -> Request {
        parse(args.iter().copied(), Path::new("/home/abc")).unwrap()
    }

    fn bad(args: &[&str]) -> String {
        parse(args.iter().copied(), Path::new("/")).unwrap_err()
    }

    /// A folder of its own for a test, removed when it's dropped.
    struct Dir(std::path::PathBuf);
    impl Dir {
        fn new(name: &str) -> Dir {
            let d = std::env::temp_dir().join(format!("wadbrowser-cli-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Dir(d)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn browser_launches() {
        assert_eq!(p(&[]), Request::default());
        let r = p(&["https://a.example", "b.example", "rust lang"]);
        assert_eq!(r.urls, ["https://a.example", "b.example", "rust lang"]);
        assert!(p(&["--no-urlbar"]).no_urlbar);
        assert!(p(&["--focus"]).no_urlbar);
        let r = p(&["--default", "--new-window", "https://x.example"]);
        assert!(r.default && r.new_window);
        let r = p(&["--private", "https://x.example"]);
        assert!(r.private);
        assert!(p(&["-p"]).private);
        // Any number of addresses, as %U gives them.
        assert_eq!(p(&["a.example", "b.example", "c.example"]).urls.len(), 3);
    }

    #[test]
    fn values_inline_or_next() {
        assert_eq!(p(&["--profile=work"]).profile.as_deref(), Some("work"));
        assert_eq!(p(&["--profile", "work"]).profile.as_deref(), Some("work"));
        // Not a profile name: as if it weren't given.
        assert_eq!(p(&["--profile", "Not Ok"]).profile, None);
        assert_eq!(p(&["--gpu", "off"]).gpu.as_deref(), Some("off"));
        assert_eq!(p(&["--gpu", "maybe"]).gpu, None);
        assert_eq!(bad(&["--name"]), "--name needs a value");
    }

    #[test]
    fn web_apps() {
        let r = p(&["--app", "claude", "--name=Claude", "--url", "https://claude.ai", "--icon", "c.png", "%U"]);
        assert_eq!(r.app_id.as_deref(), Some("wadspaces-webapp-claude"));
        assert_eq!(r.name.as_deref(), Some("Claude"));
        assert_eq!(r.start.as_deref(), Some("https://claude.ai"));
        assert_eq!(r.icon.as_deref(), Some("/home/abc/c.png"));
        // An unexpanded %U is just a word; it isn't a page.
        assert_eq!(r.urls, ["%U"]);
        assert!(r.pages().is_empty());
        assert!(bad(&["--app", "Not An Id"]).contains("isn't an app id"));
        assert!(bad(&["--app"]).contains("needs a value"));
    }

    #[test]
    fn other_browsers_flags_are_ignored() {
        let r = p(&["--no-sandbox", "--ozone-platform=wayland", "--incognito-ish", "https://x.example"]);
        assert_eq!(r.urls, ["https://x.example"]);
        assert_eq!(r.ignored, ["--no-sandbox", "--ozone-platform=wayland", "--incognito-ish"]);
        // --setup's file isn't a page.
        let r = p(&["--setup", "/run/x.json", "https://x.example"]);
        assert_eq!(r.urls, ["https://x.example"]);
        assert_eq!(r.ignored, ["--setup /run/x.json"]);
        // After --, everything is a page.
        assert_eq!(p(&["--", "--weird", "x"]).urls, ["--weird", "x"]);
    }

    #[test]
    fn files_become_file_urls() {
        let d = Dir::new("files");
        std::fs::write(d.0.join("page.html"), "<p>hi").unwrap();
        let r = parse(["page.html", "missing.html", "https://x.example/page.html"], &d.0).unwrap();
        let canon = d.0.canonicalize().unwrap();
        assert_eq!(r.urls[0], format!("file://{}/page.html", canon.display()));
        assert_eq!(r.urls[1], "missing.html");
        assert_eq!(r.urls[2], "https://x.example/page.html");
        let abs = d.0.join("page.html");
        let r = parse([abs.to_str().unwrap()], Path::new("/")).unwrap();
        assert!(r.urls[0].starts_with("file:///"));
    }

    #[test]
    fn sign_ins() {
        let r = p(&["--signin", "github", "--url", "https://github.com/login/device", "--code", "WDJB-MJHT"]);
        assert_eq!(
            r.signin,
            Some(signin::Request {
                provider: Provider::Github,
                action: Action::Open { url: "https://github.com/login/device".into(), code: Some("WDJB-MJHT".into()) }
            })
        );
        // --url is the sign-in's, not a web app's start.
        assert_eq!(r.start, None);
        let r = p(&["--signin=tailscale", "--url=https://login.tailscale.com/a/abc"]);
        assert_eq!(
            r.signin.unwrap().action,
            Action::Open { url: "https://login.tailscale.com/a/abc".into(), code: None }
        );
        assert_eq!(p(&["--signin", "github", "--show"]).signin.unwrap().action, Action::Show);
        assert_eq!(p(&["--signin", "github", "--done"]).signin.unwrap().action, Action::Done);
    }

    #[test]
    fn bad_sign_ins_are_refused() {
        assert!(bad(&["--signin", "steam", "--show"]).contains("isn't github or tailscale"));
        assert!(bad(&["--signin", "github"]).contains("needs --url"));
        assert!(bad(&["--signin", "github", "--show", "--done"]).contains("not both"));
        assert!(bad(&["--show"]).contains("go with --signin"));
        assert!(bad(&["--code", "X"]).contains("go with --signin"));
        assert!(bad(&["--signin", "github", "--app", "x", "--url", "https://x"]).contains("different windows"));
    }

    #[test]
    fn instances() {
        assert_eq!(p(&["--instance", "browser-mail"]).instance.as_deref(), Some("browser-mail"));
        let r = p(&["--instance", "try", "--ephemeral", "https://x.example"]);
        assert!(r.ephemeral && r.urls == ["https://x.example"]);
        assert!(!p(&["--instance", "try"]).ephemeral);
        assert_eq!(p(&["--instance=focus", "--no-urlbar"]).instance.as_deref(), Some("focus"));
        assert!(bad(&["--instance", "Not An Id"]).contains("isn't an id"));
        assert!(bad(&["--instance", "../x"]).contains("isn't an id"));
    }

    #[test]
    fn version_help_and_cactus_flags() {
        assert!(p(&["--version"]).version);
        assert!(p(&["-V"]).version);
        assert!(p(&["--help"]).help);
        assert!(p(&["-h"]).help);
        assert!(p(&["-v"]).verbose);
        assert_eq!(p(&["--no-restore"]).restore, Some(false));
        assert_eq!(p(&["--restore"]).restore, Some(true));
        assert!(help_text().contains("--signin"));
    }

    #[test]
    fn kalebrowser_configs() {
        let d = Dir::new("kale");
        let cfg = d.0.join("config.json");
        std::fs::write(
            &cfg,
            r#"{"app_name":"Open Router","app_url":"https://openrouter.ai","icon_path":"open-router.png","wm_class":"WADspaces-open-router"}"#,
        )
        .unwrap();
        let r = p(&["--config", cfg.to_str().unwrap()]);
        assert_eq!(r.app_id.as_deref(), Some("WADspaces-open-router"));
        assert_eq!(r.start.as_deref(), Some("https://openrouter.ai"));
        assert_eq!(r.icon, Some(d.0.join("open-router.png").to_string_lossy().into()));
        std::fs::write(&cfg, r#"{"app_name":"Open Router!","app_url":"https://openrouter.ai"}"#).unwrap();
        assert_eq!(p(&["-c", cfg.to_str().unwrap()]).app_id.as_deref(), Some("wadspaces-webapp-open-router"));
        assert!(bad(&["--config", "/nonexistent/config.json"]).contains("--config"));
    }

    #[test]
    fn ids_and_slugs() {
        assert!(web_app_id("claude") && web_app_id("webapp-1a2b") && web_app_id("9lives"));
        assert!(!web_app_id("") && !web_app_id("-x") && !web_app_id("A") && !web_app_id("a_b"));
        assert!(valid_app_id("wadspaces-webapp-claude") && valid_app_id("io.wadspaces.x_1"));
        assert!(!valid_app_id("two words") && !valid_app_id("a/b"));
        assert_eq!(slug("Open Router!"), "open-router");
        assert_eq!(slug("!!!"), "app");
        assert_eq!(p(&["-I", "icons/x.png"]).icon.as_deref(), Some("/home/abc/icons/x.png"));
    }
}
