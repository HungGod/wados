//! wados-files: WadOS's Files (the spike's `apps/files`): this machine's
//! places (Home, Desktop, Documents, Downloads, Pictures), the Trash (its
//! files and its wadspaces), its wadspaces (read-only), Open with (into a
//! wadspace with an app for it) and the viewers (places.rs, viewer.rs); and
//! Machines (machines.rs): my other machines on the tailnet, a machine's
//! home over Taildrive (drive.rs) and its wadspaces (with Copy here).
//!
//!     wados-files [<folder>]            a folder here (the home's, by default)
//!     wados-files --trash               the Trash
//!     wados-files --wadspace <id>       one of this machine's wadspaces
//!     wados-files --view <file>         a picture or text file, in a viewer
//!     wados-files --select <file>       its folder, with it picked (a browser's "Show in folder")
//!     wados-files --machines            Machines
//!     wados-files --machine <id> [--home [<path>]]
//!                                       one of them (its home, a folder in it)
//!
//! One window: a second start goes where it says in the one open. `--dump`
//! prints what it shows once it's there, and quits (tests); with `--menu
//! <name>` (or `background`), what that thing's menu has, too.

mod drive;
mod link;
mod local;
mod machines;
mod menu;
mod places;
mod state;
mod ui;
mod viewer;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::{glib, prelude::*};

use state::Loc;

const APP_ID: &str = "io.wados.Files";

/// What it was started with.
#[derive(Debug, Default, PartialEq)]
struct Args {
    loc: Option<Loc>,
    dump: bool,
    /// A file to show in a viewer.
    view: Option<std::path::PathBuf>,
    /// A thing to pick in the folder it opens (`--select`).
    select: Option<String>,
    /// `--dump`: this thing's menu too.
    menu: Option<String>,
    /// `--dump`: these done first, as clicks would (tests).
    acts: Vec<String>,
}

/// A path as given: `~/…`, a `file://` URI, relative (to `cwd`) or absolute.
fn path_of(s: &str, cwd: &std::path::Path) -> std::path::PathBuf {
    if let Some(u) = s.strip_prefix("file://") {
        return glib::Uri::unescape_string(u, None::<&str>).map(|p| p.as_str().into()).unwrap_or_else(|| u.into());
    }
    if s == "~" {
        return glib::home_dir();
    }
    if let Some(r) = s.strip_prefix("~/") {
        return glib::home_dir().join(r);
    }
    let p = std::path::Path::new(s);
    if p.is_absolute() { p.to_path_buf() } else { cwd.join(p) }
}

fn parse(args: &[String]) -> Result<Args, String> {
    parse_in(args, &std::env::current_dir().unwrap_or_else(|_| glib::home_dir()))
}

fn parse_in(args: &[String], cwd: &std::path::Path) -> Result<Args, String> {
    let mut a = Args::default();
    let mut machine: Option<String> = None;
    let mut home: Option<Vec<String>> = None;
    let mut it = args.iter().skip(1).peekable();
    while let Some(x) = it.next() {
        match x.as_str() {
            "--machines" => a.loc = Some(Loc::Machines),
            "--machine" => machine = Some(it.next().ok_or("--machine needs a machine's id")?.clone()),
            "--home" => {
                let path = match it.peek() {
                    Some(p) if !p.starts_with("--") => it.next().cloned().unwrap_or_default(),
                    _ => String::new(),
                };
                home = Some(path.split('/').filter(|s| !s.is_empty()).map(String::from).collect());
            }
            "--dump" => a.dump = true,
            "--trash" => a.loc = Some(Loc::Trash),
            "--wadspace" => a.loc = Some(Loc::Space(it.next().ok_or("--wadspace needs a wadspace's id")?.clone())),
            "--view" => a.view = Some(path_of(it.next().ok_or("--view needs a file")?, cwd)),
            "--select" => {
                let p = path_of(it.next().ok_or("--select needs a file")?, cwd);
                let (Some(dir), Some(name)) = (p.parent(), p.file_name()) else {
                    return Err(format!("--select: {} isn't in a folder", p.display()));
                };
                a.loc = Some(Loc::Local(dir.to_path_buf()));
                a.select = Some(name.to_string_lossy().into_owned());
            }
            "--act" => a.acts.push(it.next().ok_or("--act needs what to do")?.clone()),
            "--menu" => a.menu = Some(it.next().ok_or("--menu needs a thing's name (or background)")?.clone()),
            other if !other.starts_with("--") => {
                let p = path_of(other, cwd);
                if p.is_dir() {
                    a.loc = Some(Loc::Local(p));
                } else {
                    a.view = Some(p);
                }
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    match (machine, home) {
        (Some(peer), Some(path)) => a.loc = Some(Loc::Folder { peer, path }),
        (Some(peer), None) => a.loc = Some(Loc::Peer(peer)),
        (None, Some(_)) => return Err("--home is a machine's: --machine <id> --home".into()),
        (None, None) => {}
    }
    Ok(a)
}

/// One window (a second start hands it its arguments), but `--dump` runs on its own.
fn flags(argv: &[String]) -> gio::ApplicationFlags {
    let mut f = gio::ApplicationFlags::HANDLES_COMMAND_LINE;
    if argv.iter().any(|a| a == "--dump") {
        f |= gio::ApplicationFlags::NON_UNIQUE;
    }
    f
}

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();
    let argv: Vec<String> = std::env::args().collect();
    match argv.get(1).map(String::as_str) {
        Some("--version") => {
            println!("wados-files {}", env!("CARGO_PKG_VERSION"));
            return glib::ExitCode::SUCCESS;
        }
        Some("--help" | "-h") => {
            println!(
                "usage: wados-files [<folder> | --trash | --wadspace <id> | --view <file> | --select <file> | --machines | --machine <id> [--home [<path>]]] [--dump [--menu <name>]]"
            );
            return glib::ExitCode::SUCCESS;
        }
        _ => {}
    }
    if let Err(e) = parse(&argv) {
        eprintln!("wados-files: {e}");
        return glib::ExitCode::FAILURE;
    }
    let app = gtk::Application::builder().application_id(APP_ID).flags(flags(&argv)).build();
    app.connect_startup(|_| {
        if let Some(d) = gtk::gdk::Display::default() {
            wad_gtk::theme::install(&d);
            wad_gtk::kit::install(&d);
            wad_gtk::theme::add_css(&d, ui::CSS);
            wad_gtk::theme::add_css(&d, menu::CSS);
            wad_gtk::theme::add_css(&d, viewer::CSS);
        }
    });
    let files: Rc<std::cell::OnceCell<Rc<ui::Files>>> = Rc::default();
    let f = files.clone();
    app.connect_command_line(move |app, cmd| {
        let args: Vec<String> = cmd.arguments().iter().map(|a| a.to_string_lossy().into_owned()).collect();
        let cwd = cmd.cwd().unwrap_or_else(glib::home_dir);
        let a = match parse_in(&args, &cwd) {
            Ok(a) => a,
            Err(e) => {
                cmd.printerr_literal(&format!("wados-files: {e}\n"));
                return glib::ExitCode::FAILURE;
            }
        };
        // A file to view, alone: its viewer, not Files' window.
        if let Some(p) = &a.view
            && a.loc.is_none()
        {
            let v = viewer::open(app, p);
            v.present();
            if a.dump {
                let (app, v) = (app.clone(), v.clone());
                glib::timeout_add_local_once(Duration::from_millis(600), move || {
                    println!("{}", dump_widget(&v));
                    app.quit();
                });
            }
            return glib::ExitCode::SUCCESS;
        }
        let files = f.get_or_init(|| ui::Files::new(app));
        if let Some(l) = a.loc {
            files.go(l);
        }
        if let Some(name) = &a.select {
            files.pick_only(name);
        }
        if a.dump {
            dump_when_ready(files, app, a.menu.clone(), a.acts.clone());
        }
        files.window.present();
        glib::ExitCode::SUCCESS
    });
    app.run_with_args(&argv)
}

/// `--dump`: what it shows once it's settled (or after a while), then quit.
/// A window's labels and texts, in order (`--dump` of a viewer).
fn dump_widget(w: &impl IsA<gtk::Widget>) -> String {
    fn walk(w: &gtk::Widget, out: &mut Vec<String>) {
        if let Some(l) = w.downcast_ref::<gtk::Label>() {
            out.push(l.label().to_string());
        }
        if let Some(t) = w.downcast_ref::<gtk::TextView>() {
            let b = t.buffer();
            out.push(format!("text: {}", b.text(&b.start_iter(), &b.end_iter(), false)));
        }
        if w.downcast_ref::<gtk::Picture>().is_some() {
            out.push(format!("picture: {}", w.widget_name()));
        }
        let mut c = w.first_child();
        while let Some(x) = c {
            walk(&x, out);
            c = x.next_sibling();
        }
    }
    let mut out = vec![];
    walk(w.upcast_ref(), &mut out);
    out.retain(|s| !s.is_empty());
    out.join(" | ")
}

fn dump_when_ready(files: &Rc<ui::Files>, app: &gtk::Application, menu: Option<String>, acts: Vec<String>) {
    let (f, app) = (files.clone(), app.clone());
    let tries = Cell::new(0);
    glib::timeout_add_local(Duration::from_millis(200), move || {
        tries.set(tries.get() + 1);
        if !f.settled() && tries.get() < 40 {
            return glib::ControlFlow::Continue;
        }
        if !acts.is_empty() {
            for a in &acts {
                if let Err(e) = f.act(a) {
                    eprintln!("wados-files: {e}");
                }
            }
            // What they asked of wadd answers first (a moment), then it's shown.
            let (f, app, menu) = (f.clone(), app.clone(), menu.clone());
            glib::timeout_add_local_once(Duration::from_millis(800), move || {
                f.reload();
                f.draw();
                println!("{}", f.dump());
                if let Some(m) = &menu {
                    println!("menu: {}", f.menu_words(m));
                }
                app.quit();
            });
            return glib::ControlFlow::Break;
        }
        f.draw();
        println!("{}", f.dump());
        if let Some(m) = &menu {
            println!("menu: {}", f.menu_words(m));
        }
        app.quit();
        glib::ControlFlow::Break
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        std::iter::once("wados-files").chain(a.iter().copied()).map(String::from).collect()
    }

    #[test]
    fn it_opens_where_it_is_told() {
        assert_eq!(parse(&args(&[])).unwrap().loc, None);
        assert_eq!(parse(&args(&["--trash"])).unwrap().loc, Some(Loc::Trash));
        assert_eq!(parse(&args(&["--wadspace", "art"])).unwrap().loc, Some(Loc::Space("art".into())));
        let cwd = std::path::Path::new("/");
        assert_eq!(parse_in(&args(&["tmp"]), cwd).unwrap().loc, Some(Loc::Local("/tmp".into())));
        assert_eq!(parse_in(&args(&["file:///tmp"]), cwd).unwrap().loc, Some(Loc::Local("/tmp".into())));
        let v = parse_in(&args(&["--view", "/etc/hostname-not-a-dir"]), cwd).unwrap();
        assert_eq!(v.view.as_deref(), Some(std::path::Path::new("/etc/hostname-not-a-dir")));
        assert_eq!(parse(&args(&["--machines"])).unwrap().loc, Some(Loc::Machines));
        assert_eq!(parse(&args(&["--machine", "m-desk"])).unwrap().loc, Some(Loc::Peer("m-desk".into())));
        assert_eq!(
            parse(&args(&["--machine", "m-desk", "--home"])).unwrap().loc,
            Some(Loc::Folder { peer: "m-desk".into(), path: vec![] })
        );
        assert_eq!(
            parse(&args(&["--machine", "m-desk", "--home", "Pictures/Sprites", "--dump"])).unwrap(),
            Args {
                loc: Some(Loc::Folder { peer: "m-desk".into(), path: vec!["Pictures".into(), "Sprites".into()] }),
                dump: true,
                ..Args::default()
            }
        );
        let s = parse_in(&args(&["--select", "/home/live/Downloads/a b.zip"]), cwd).unwrap();
        assert_eq!(s.loc, Some(Loc::Local("/home/live/Downloads".into())));
        assert_eq!(s.select.as_deref(), Some("a b.zip"));
        assert!(parse(&args(&["--select"])).is_err());
        assert!(parse(&args(&["--machine"])).is_err());
        assert!(parse(&args(&["--home"])).is_err());
    }
}
