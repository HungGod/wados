//! wados-files: WadOS's Files (the spike's `apps/files`). Phase 6 has
//! Machines (ui.rs): my other machines on the tailnet, a machine's home over
//! Taildrive (drive.rs) and its wadspaces (with Copy here). The rest of
//! Files (this machine's places, the Trash, Open with) comes in phase 8.
//!
//!     wados-files [--machines]          Machines
//!     wados-files --machine <id> [--home [<path>]]
//!                                       one of them (its home, a folder in it)
//!
//! One window: a second start goes where it says in the one open. `--dump`
//! prints what it shows once it's there, and quits (tests).

mod drive;
mod link;
mod state;
mod ui;

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
}

fn parse(args: &[String]) -> Result<Args, String> {
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
            println!("usage: wados-files [--machines | --machine <id> [--home [<path>]]] [--dump]");
            return glib::ExitCode::SUCCESS;
        }
        _ => {}
    }
    if let Err(e) = parse(&argv) {
        eprintln!("wados-files: {e}");
        return glib::ExitCode::FAILURE;
    }
    let app = gtk::Application::builder().application_id(APP_ID).flags(flags(&argv)).build();
    let files: Rc<std::cell::OnceCell<Rc<ui::Files>>> = Rc::default();
    let f = files.clone();
    app.connect_command_line(move |app, cmd| {
        let args: Vec<String> = cmd.arguments().iter().map(|a| a.to_string_lossy().into_owned()).collect();
        let a = match parse(&args) {
            Ok(a) => a,
            Err(e) => {
                cmd.printerr_literal(&format!("wados-files: {e}\n"));
                return glib::ExitCode::FAILURE;
            }
        };
        let files = f.get_or_init(|| {
            if let Some(d) = gtk::gdk::Display::default() {
                wad_gtk::theme::install(&d);
                wad_gtk::kit::install(&d);
                wad_gtk::theme::add_css(&d, ui::CSS);
            }
            ui::Files::new(app)
        });
        if let Some(l) = a.loc {
            files.go(l);
        }
        if a.dump {
            dump_when_ready(files, app);
        }
        files.window.present();
        glib::ExitCode::SUCCESS
    });
    app.run_with_args(&argv)
}

/// `--dump`: what it shows once it's settled (or after a while), then quit.
fn dump_when_ready(files: &Rc<ui::Files>, app: &gtk::Application) {
    let (f, app) = (files.clone(), app.clone());
    let tries = Cell::new(0);
    glib::timeout_add_local(Duration::from_millis(200), move || {
        tries.set(tries.get() + 1);
        if !f.settled() && tries.get() < 40 {
            return glib::ControlFlow::Continue;
        }
        f.draw();
        println!("{}", f.dump());
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
                dump: true
            }
        );
        assert!(parse(&args(&["--machine"])).is_err());
        assert!(parse(&args(&["--home"])).is_err());
    }
}
