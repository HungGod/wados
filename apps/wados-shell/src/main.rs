//! wados-shell: the WadSpaces shell on labwc. The launcher and the desktop
//! (a scroll apart), the bar, the overlays (Super+Tab, Alt+Space, toasts),
//! the lock screen and the Builder, as layer-shell surfaces and windows.
//!
//!     wados-shell [--builder]
//!
//! It answers `wadsctl` on `$XDG_RUNTIME_DIR/wados/shell.sock`.

mod assets;
mod builder;
mod ctl;
mod dock;
mod lock;
mod machine;
mod pam;
mod surface;
mod wallpaper;
mod wm;

use std::cell::RefCell;
use std::rc::Rc;

use gtk::{gdk, gio, glib, prelude::*};

use machine::{Machine, Page};

const CSS: &str = include_str!("shell.css");

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
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
    let machine = Machine::new(app, &primary, &others);
    let lock = lock::Lock::new();
    if std::env::var_os("WADOS_NO_LOCK").is_none()
        && let Err(e) = lock.lock(app)
    {
        tracing::error!("{e}");
    }
    if with_builder {
        builder::open(app);
    }

    let windows: Rc<RefCell<Vec<wm::Top>>> = Rc::default();
    let wm = match wm::start() {
        Ok((wm, updates)) => {
            let windows = windows.clone();
            glib::spawn_future_local(async move {
                while let Ok(list) = updates.recv().await {
                    *windows.borrow_mut() = list;
                }
            });
            Some(Rc::new(wm))
        }
        Err(e) => {
            tracing::error!("{e}");
            None
        }
    };

    let (tx, rx) = async_channel::unbounded::<ctl::Request>();
    match ctl::serve(tx) {
        Ok(p) => tracing::info!("listening on {}", p.display()),
        Err(e) => tracing::error!("the shell's socket: {e}"),
    }
    let app = app.clone();
    glib::spawn_future_local(async move {
        while let Ok(req) = rx.recv().await {
            let answer = if req.line == "lock" {
                match lock.lock(&app) {
                    Ok(()) => "ok".into(),
                    Err(e) => format!("error {e}"),
                }
            } else if req.line == "locked" {
                format!("ok {}", lock.is_locked())
            } else {
                handle(&app, &machine, wm.as_deref(), &windows.borrow(), &req.line)
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

/// One line from wadsctl.
fn handle(app: &gtk::Application, m: &Rc<Machine>, wm: Option<&wm::Wm>, windows: &[wm::Top], line: &str) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    if let ["wm", rest @ ..] = words.as_slice() {
        return handle_wm(wm, windows, rest);
    }
    match words.as_slice() {
        ["ping"] => "ok pong".into(),
        ["page"] => format!("ok {:?} {:.2}", m.page(), m.progress()),
        ["page", "desktop"] => {
            m.go(Page::Desktop);
            "ok".into()
        }
        ["page", "home" | "launcher"] => {
            m.go(Page::Launcher);
            "ok".into()
        }
        ["page", "toggle"] => {
            m.toggle();
            "ok".into()
        }
        ["switcher", dir @ ("next" | "prev")] => {
            m.flash(&format!("Super+Tab: switcher {dir}"));
            "ok".into()
        }
        ["search"] => {
            m.flash("Alt+Space: search");
            "ok".into()
        }
        ["log"] => format!("ok {}", m.log.borrow().join(" | ")),
        ["quit"] => {
            app.quit();
            "ok".into()
        }
        ["help"] => "ok ping | page [desktop|home|toggle] | switcher next|prev | search | log | wm list|activate|fullscreen|unfullscreen|minimize|unminimize|close <#id|app-id> [output] | quit".into(),
        _ => format!("error unknown: {line}"),
    }
}

/// `wadsctl wm …`: the windows, and moving them.
fn handle_wm(wm: Option<&wm::Wm>, windows: &[wm::Top], words: &[&str]) -> String {
    let Some(wm) = wm else { return "error no window management (foreign-toplevel)".into() };
    let which = |w: &str| wm::Which(w.to_string());
    let cmd = match words {
        ["list"] => {
            let rows: Vec<String> = windows
                .iter()
                .map(|t| {
                    let mut flags = Vec::new();
                    for (on, f) in [
                        (t.activated, "activated"),
                        (t.maximized, "maximized"),
                        (t.minimized, "minimized"),
                        (t.fullscreen, "fullscreen"),
                    ] {
                        if on {
                            flags.push(f);
                        }
                    }
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
