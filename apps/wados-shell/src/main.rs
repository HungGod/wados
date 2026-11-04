//! wados-shell: the WadSpaces shell on labwc. The launcher and the desktop
//! (a scroll apart), the bar, the overlays (Super+Tab, Alt+Space, toasts),
//! the lock screen and the Builder, as layer-shell surfaces and windows,
//! on wadd (wadd.rs follows it into the store; surfaces read the store).
//!
//!     wados-shell [--builder]
//!
//! It answers `wadsctl` on `$XDG_RUNTIME_DIR/wados/shell.sock`. Run as the
//! machine's own session (`WADOS_SESSION=machine`, session.rs), it also
//! restarts and powers off the machine, locks when idle and before sleep,
//! and sets the brightness.

mod assets;
mod bar;
mod builder;
mod ctl;
mod desktop;
mod dnd;
mod dnd_probe;
mod home;
mod lock;
mod machine;
mod overlays;
mod pam;
mod placer;
mod session;
mod shell;
mod store;
mod surface;
mod wadd;
mod wallpaper;
mod wm;

use std::rc::Rc;

use gtk::{gdk, gio, glib, prelude::*};

use machine::{Machine, Page};
use shell::Shell;

const CSS: &str = include_str!("shell.css");

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        // Colours only on a terminal: the machine's journal stores escapes as binary (journalctl -g can't find them).
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();
    let with_builder = std::env::args().any(|a| a == "--builder");
    let app =
        gtk::Application::builder().application_id("io.wados.Shell").flags(gio::ApplicationFlags::NON_UNIQUE).build();
    app.connect_activate(move |app| activate(app, with_builder));
    // GTK mustn't see our flags.
    app.run_with_args::<&str>(&[])
}

fn activate(app: &gtk::Application, with_builder: bool) {
    let Some(display) = gdk::Display::default() else {
        tracing::error!("no display");
        return;
    };
    wad_gtk::theme::install(&display);
    wad_gtk::theme::add_css(&display, CSS);
    if !gtk4_layer_shell::is_supported() {
        tracing::error!("this compositor has no layer shell (wlr-layer-shell): run it on labwc");
        app.quit();
        return;
    }
    let monitors: Vec<gdk::Monitor> = display.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect();
    let Some((primary, others)) = pick_primary(&monitors) else {
        tracing::error!("no monitors");
        return;
    };

    // The lock comes first: every start lands on it.
    let lock = lock::Lock::new();
    if std::env::var_os("WADOS_NO_LOCK").is_none()
        && let Err(e) = lock.lock(app)
    {
        tracing::error!("{e}");
    }

    // wadd: followed into the store; told where the screen is.
    let store = store::Store::new();
    let link = wadd::Link::start(store.clone());
    link.register_screen();

    // The windows, and where wadspaces' go.
    let wm = match wm::start() {
        Ok((wm, updates)) => Some((Rc::new(wm), updates)),
        Err(e) => {
            tracing::error!("{e}");
            None
        }
    };
    let connector = primary.connector().map(|c| c.to_string()).unwrap_or_default();
    let placer = placer::Placer::new(wm.as_ref().map(|(w, _)| w.clone()), store.clone(), &connector);
    if let Some((_, updates)) = wm {
        let p = placer.clone();
        glib::spawn_future_local(async move {
            while let Ok(list) = updates.recv().await {
                p.windows_changed(list);
            }
        });
    }
    let p = placer.clone();
    store.watch(move |c| {
        if let store::Change::Run(id) = c {
            p.run_changed(id);
        }
    });

    let shell = Shell::new(app, store, link, placer, primary, others);
    let builder = builder::Builder::new(&shell);
    shell.set_builder(&builder);
    let machine = Machine::new(&shell);
    let _overlays = overlays::Overlays::new(&shell);
    let session = session::Session::new(&shell, &lock);
    dnd_probe::install(&shell);
    if with_builder {
        builder.present();
    }

    let (tx, rx) = async_channel::unbounded::<ctl::Request>();
    match ctl::serve(tx) {
        Ok(p) => tracing::info!("listening on {}", p.display()),
        Err(e) => tracing::error!("the shell's socket: {e}"),
    }
    let app = app.clone();
    glib::spawn_future_local(async move {
        let _keep = (_overlays, builder);
        while let Ok(req) = rx.recv().await {
            let words: Vec<&str> = req.line.split_whitespace().collect();
            let answer = match words.as_slice() {
                ["lock"] => match session.lock() {
                    Ok(()) => "ok".into(),
                    Err(e) => format!("error {e}"),
                },
                ["locked"] => format!("ok {}", lock.is_locked()),
                _ => handle(&app, &machine, &shell, &words),
            };
            let _ = req.reply.send(answer);
        }
    });
}

/// The primary: WADOS_PRIMARY's connector if set, else the first.
fn pick_primary(all: &[gdk::Monitor]) -> Option<(gdk::Monitor, Vec<gdk::Monitor>)> {
    let wanted = std::env::var("WADOS_PRIMARY").ok();
    let i = wanted.and_then(|c| all.iter().position(|m| m.connector().as_deref() == Some(c.as_str()))).unwrap_or(0);
    let primary = all.get(i)?.clone();
    let others = all.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, m)| m.clone()).collect();
    Some((primary, others))
}

/// One line from wadsctl: the shell's own commands, then each module's.
fn handle(app: &gtk::Application, m: &Rc<Machine>, shell: &Rc<Shell>, words: &[&str]) -> String {
    match words {
        ["ping"] => return "ok pong".into(),
        ["page"] => return format!("ok {:?} {:.2}", m.page(), m.progress()),
        ["page", "desktop"] => {
            m.go(Page::Desktop);
            return "ok".into();
        }
        ["page", "home" | "launcher"] => {
            m.go(Page::Launcher);
            return "ok".into();
        }
        ["page", "toggle"] => {
            m.toggle();
            return "ok".into();
        }
        ["wadd"] => {
            let screen = shell.store.screen().map(|s| format!("{:?}", s.from)).unwrap_or_else(|| "?".into());
            return format!(
                "ok connected={} machine={} wadspaces={} running={} screen={screen}",
                shell.store.connected(),
                shell.store.machine_id(),
                shell.store.live().len(),
                shell.store.running().len()
            );
        }
        ["wm", rest @ ..] => return handle_wm(shell, rest),
        ["quit"] => {
            app.quit();
            return "ok".into();
        }
        _ => {}
    }
    if let Some(answer) = shell.ctl(words) {
        return answer;
    }
    match words {
        ["help"] => "ok ping | page [desktop|home|toggle] | wadd | wm list|activate|fullscreen|unfullscreen|minimize|unminimize|close <#id|app-id> [output] | lock | locked | session [heard] | power [state|close|lock|restart|off] | volume [up|down|mute] | brightness [up|down] | server off | quit (and each module's own)".into(),
        _ => format!("error unknown: {}", words.join(" ")),
    }
}

/// `wadsctl wm …`: the windows, and moving them.
fn handle_wm(shell: &Rc<Shell>, words: &[&str]) -> String {
    let Some(wm) = shell.placer.wm() else { return "error no window management (foreign-toplevel)".into() };
    let which = |w: &str| wm::Which(w.to_string());
    let cmd = match words {
        ["list"] => {
            let rows: Vec<String> = shell
                .placer
                .windows()
                .iter()
                .map(|t| {
                    let flags: Vec<&str> = [
                        (t.activated, "activated"),
                        (t.maximized, "maximized"),
                        (t.minimized, "minimized"),
                        (t.fullscreen, "fullscreen"),
                    ]
                    .into_iter()
                    .filter_map(|(on, f)| on.then_some(f))
                    .collect();
                    match t.wadspace() {
                        Some(ws) => format!("#{} wadspace {ws} on [{}] {}", t.id, t.outputs.join(","), flags.join(",")),
                        None => format!(
                            "#{} {} {:?} on [{}] {}",
                            t.id,
                            t.app_id,
                            t.title,
                            t.outputs.join(","),
                            flags.join(",")
                        ),
                    }
                })
                .collect();
            return format!("ok {}", rows.join(" ; "));
        }
        ["activate", w] => wm::Cmd::Activate(which(w)),
        ["fullscreen", w] => wm::Cmd::Fullscreen(which(w), None),
        ["fullscreen", w, output] => wm::Cmd::Fullscreen(which(w), Some(output.to_string())),
        ["unfullscreen", w] => wm::Cmd::Unfullscreen(which(w)),
        ["minimize", w] => wm::Cmd::Minimize(which(w)),
        ["unminimize", w] => wm::Cmd::Unminimize(which(w)),
        ["close", w] => wm::Cmd::Close(which(w)),
        _ => return format!("error unknown: wm {}", words.join(" ")),
    };
    wm.send(cmd);
    "ok".into()
}
