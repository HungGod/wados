//! A wadspace's desktop moving between the screen and its stream, which
//! wadd does without restarting the wadspace (its terminals' jobs, in tmux,
//! carry on): labwc ends on one display and svc-de starts it on the other.
//!
//!   wadspaces-session switch [--timeout <seconds>] [--force]
//!       (wadd, through podman exec as abc) Notes which apps are open, asks
//!       each window to close as its × would (apps ask about unsaved work
//!       themselves), and ends labwc once they all have. Prints
//!       {"still_open": [...]}: what didn't close in time, and then labwc
//!       carries on as it was, unless --force (it ends anyway).
//!   wadspaces-session restore
//!       (labwc's autostart, or wadd when a switch is called off) Opens
//!       again what switch noted, and a terminal for each tmux session that
//!       was in one. Nothing noted (labwc started for any other reason):
//!       nothing to do.
//!
//! Terminals aren't noted: their jobs are in tmux sessions (wadspaces-term,
//! /etc/tmux.conf), which outlive them; a session's `destroy-unattached`
//! is turned off for the switch, and on again when it's reattached.

use std::collections::{BTreeSet, HashMap};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self as handle, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self as manager, ZwlrForeignToplevelManagerV1},
};

/// What switch noted, and that a switch is on (WadBrowser keeps its tabs
/// when it's closed then): in the wadspace's runtime dir, which starts
/// empty with the container.
const NOTED: &str = "wadspaces-session.json";
const SWITCHING: &str = "wadspaces-switching";
/// svc-de's note of labwc's pid.
const LABWC_PID: &str = "/run/wadspaces-labwc.pid";
/// Apps whose windows aren't apps to open again: terminals (their tmux
/// sessions are) and the desktop's own.
const NOT_APPS: &[&str] = &["foot", "footclient", "selkies-desktop", ""];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!("wadspaces-session {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("switch") => {
            let force = args.iter().any(|a| a == "--force");
            let timeout = args
                .iter()
                .position(|a| a == "--timeout")
                .and_then(|i| args.get(i + 1))
                .and_then(|t| t.parse().ok())
                .unwrap_or(20);
            switch(Duration::from_secs(timeout), force)
        }
        Some("restore") => restore(),
        _ => {
            eprintln!("usage: wadspaces-session switch [--timeout <seconds>] [--force] | restore | --version");
            ExitCode::from(2)
        }
    }
}

/// The wadspace's runtime dir: $XDG_RUNTIME_DIR in its desktop, /config/.XDG
/// (init-desktop-config's) from podman exec.
fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()).map_or_else(|| "/config/.XDG".into(), PathBuf::from)
}

/// labwc's socket: $WAYLAND_DISPLAY, else the newest in the runtime dir.
fn labwc_socket() -> Option<PathBuf> {
    use std::os::unix::fs::FileTypeExt;
    let dir = runtime_dir();
    if let Some(d) = std::env::var_os("WAYLAND_DISPLAY").filter(|d| !d.is_empty()) {
        return Some(dir.join(d));
    }
    std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix("wayland-").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                && e.file_type().is_ok_and(|t| t.is_socket())
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max()
        .map(|(_, p)| p)
}

#[derive(Default, Clone)]
struct Window {
    title: String,
    app_id: String,
}

#[derive(Default)]
struct Windows {
    manager: Option<ZwlrForeignToplevelManagerV1>,
    open: HashMap<u32, (ZwlrForeignToplevelHandleV1, Window)>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Windows {
    fn event(
        w: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, .. } = event
            && interface == "zwlr_foreign_toplevel_manager_v1"
        {
            w.manager = Some(registry.bind(name, 1, qh, ()));
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Windows {
    fn event(
        w: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let manager::Event::Toplevel { toplevel } = event {
            w.open.insert(toplevel.id().protocol_id(), (toplevel, Window::default()));
        }
    }

    event_created_child!(Windows, ZwlrForeignToplevelManagerV1, [
        manager::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for Windows {
    fn event(
        w: &mut Self,
        h: &ZwlrForeignToplevelHandleV1,
        event: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = h.id().protocol_id();
        match event {
            handle::Event::Title { title } => {
                if let Some((_, win)) = w.open.get_mut(&id) {
                    win.title = title;
                }
            }
            handle::Event::AppId { app_id } => {
                if let Some((_, win)) = w.open.get_mut(&id) {
                    win.app_id = app_id;
                }
            }
            handle::Event::Closed => {
                if let Some((h, _)) = w.open.remove(&id) {
                    h.destroy();
                }
            }
            _ => {}
        }
    }
}

/// What's open in labwc: its windows, and the connection to close them on.
fn connect() -> Result<(Connection, wayland_client::EventQueue<Windows>, Windows), String> {
    let socket = labwc_socket().ok_or("no desktop (labwc) running")?;
    let stream = UnixStream::connect(&socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut w = Windows::default();
    queue.roundtrip(&mut w).map_err(|e| e.to_string())?;
    if w.manager.is_none() {
        return Err("the desktop doesn't list its windows (wlr-foreign-toplevel)".into());
    }
    // The windows, then each one's title and app id.
    queue.roundtrip(&mut w).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut w).map_err(|e| e.to_string())?;
    Ok((conn, queue, w))
}

/// Apps to open again: each one once, in the order they're listed.
fn apps_of(windows: &[Window]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    windows
        .iter()
        .map(|w| w.app_id.clone())
        .filter(|a| !NOT_APPS.contains(&a.as_str()) && seen.insert(a.clone()))
        .collect()
}

fn switch(timeout: Duration, force: bool) -> ExitCode {
    // Terminals' sessions outlive their windows from here on.
    for (name, _) in tmux_sessions() {
        tmux(&["set-option", "-t", &name, "destroy-unattached", "off"]);
    }
    let dir = runtime_dir();
    let (conn, mut queue, mut w) = match connect() {
        Ok(c) => c,
        // No desktop: nothing open, nothing to end.
        Err(e) => {
            eprintln!("wadspaces-session: {e}");
            println!("{}", json!({"still_open": []}));
            return ExitCode::SUCCESS;
        }
    };
    let windows: Vec<Window> = w.open.values().map(|(_, win)| win.clone()).collect();
    let noted = json!({"apps": apps_of(&windows)});
    let _ = std::fs::write(dir.join(NOTED), noted.to_string());
    let _ = std::fs::write(dir.join(SWITCHING), "");
    if !force {
        for (h, _) in w.open.values() {
            h.close();
        }
        let _ = conn.flush();
        let deadline = Instant::now() + timeout;
        while !w.open.is_empty() && Instant::now() < deadline {
            if wait(&conn, &mut queue, &mut w, deadline - Instant::now()).is_err() {
                break;
            }
        }
        if !w.open.is_empty() {
            let still: Vec<Value> =
                w.open.values().map(|(_, win)| json!({"title": win.title, "app_id": win.app_id})).collect();
            println!("{}", json!({"still_open": still}));
            return ExitCode::from(3);
        }
    }
    end_labwc();
    println!("{}", json!({"still_open": []}));
    ExitCode::SUCCESS
}

/// Reads labwc's events for up to `left`.
fn wait(
    conn: &Connection,
    queue: &mut wayland_client::EventQueue<Windows>,
    w: &mut Windows,
    left: Duration,
) -> Result<(), String> {
    queue.dispatch_pending(w).map_err(|e| e.to_string())?;
    conn.flush().map_err(|e| e.to_string())?;
    let Some(guard) = conn.prepare_read() else { return Ok(()) };
    let mut fds = [libc::pollfd {
        fd: std::os::fd::AsRawFd::as_raw_fd(&guard.connection_fd().as_fd()),
        events: libc::POLLIN,
        revents: 0,
    }];
    let ms = left.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: poll on one valid fd for up to `ms`.
    let n = unsafe { libc::poll(fds.as_mut_ptr(), 1, ms) };
    if n > 0 {
        guard.read().map_err(|e| e.to_string())?;
    }
    queue.dispatch_pending(w).map_err(|e| e.to_string())?;
    Ok(())
}

/// Ends labwc (svc-de then starts it where the wadspace is now).
fn end_labwc() {
    let pid: Option<i32> = std::fs::read_to_string(LABWC_PID).ok().and_then(|p| p.trim().parse().ok());
    if let Some(pid) = pid.filter(|p| *p > 1) {
        // SAFETY: a signal to the pid svc-de noted (labwc, abc's like us).
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
}

fn restore() -> ExitCode {
    let dir = runtime_dir();
    let Ok(text) = std::fs::read_to_string(dir.join(NOTED)) else { return ExitCode::SUCCESS };
    let _ = std::fs::remove_file(dir.join(NOTED));
    let noted: Value = serde_json::from_str(&text).unwrap_or_default();
    let env = desktop_env(&dir);
    let entries = desktop_entries();
    let autostarted = autostarted(&entries);
    for app in noted["apps"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        match entry_for(&entries, app) {
            Some(e) if autostarted.contains(&e) => {}
            Some(e) => {
                let ok = Command::new("gio")
                    .args(["launch", &e.to_string_lossy()])
                    .envs(env.iter().map(|(k, v)| (k, v)))
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success());
                eprintln!("wadspaces-session: {app} {}", if ok { "opened again" } else { "didn't open" });
            }
            None => eprintln!("wadspaces-session: {app}: nothing here opens it again"),
        }
    }
    // A terminal for each session that was in one (VS Code brings back its own).
    for (name, attached) in tmux_sessions() {
        if attached == 0 && name.starts_with("term-") {
            let _ = Command::new("foot")
                .args(["-e", "tmux", "attach-session", "-t", &name, ";", "set-option", "destroy-unattached", "on"])
                .envs(env.iter().map(|(k, v)| (k, v)))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
    }
    let _ = std::fs::remove_file(dir.join(SWITCHING));
    ExitCode::SUCCESS
}

/// What an app opened from here needs of the desktop: labwc's own env when
/// run from its autostart; from podman exec, its socket and the bus.
fn desktop_env(dir: &Path) -> Vec<(String, String)> {
    let mut env = vec![("XDG_RUNTIME_DIR".to_string(), dir.to_string_lossy().into_owned())];
    if std::env::var_os("WAYLAND_DISPLAY").is_none()
        && let Some(s) = labwc_socket()
    {
        env.push(("WAYLAND_DISPLAY".into(), s.to_string_lossy().into_owned()));
    }
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        env.push(("DBUS_SESSION_BUS_ADDRESS".into(), format!("unix:path={}", dir.join("bus").display())));
    }
    env
}

/// The desktop entries apps can be opened from, the first of a name winning
/// (as XDG_DATA_DIRS orders them).
fn desktop_entries() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/config"), PathBuf::from);
    let dirs = [
        home.join(".local/share/applications"),
        "/usr/local/share/applications".into(),
        "/usr/share/applications".into(),
    ];
    let mut seen = BTreeSet::new();
    let mut out = vec![];
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut here: Vec<PathBuf> =
            rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "desktop")).collect();
        here.sort();
        for p in here {
            if seen.insert(p.file_name().map(|n| n.to_os_string())) {
                out.push(p);
            }
        }
    }
    out
}

/// A desktop entry's key (in its [Desktop Entry] group).
fn key(text: &str, k: &str) -> Option<String> {
    let mut in_group = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
        } else if in_group && let Some(v) = line.strip_prefix(k).and_then(|r| r.strip_prefix('=')) {
            return Some(v.trim().to_string());
        }
    }
    None
}

/// The entry that opens an app whose windows say `app_id`: its
/// StartupWMClass, else its own name, else its command's.
fn entry_for(entries: &[PathBuf], app_id: &str) -> Option<PathBuf> {
    let read: Vec<(PathBuf, String)> =
        entries.iter().filter_map(|p| Some((p.clone(), std::fs::read_to_string(p).ok()?))).collect();
    let usable = |t: &str| key(t, "NoDisplay").is_none_or(|v| v != "true") && key(t, "Exec").is_some();
    let same = |a: &str| a.eq_ignore_ascii_case(app_id);
    let by = |f: &dyn Fn(&Path, &str) -> bool| read.iter().find(|(p, t)| usable(t) && f(p, t)).map(|(p, _)| p.clone());
    by(&|_, t| key(t, "StartupWMClass").is_some_and(|c| same(&c)))
        .or_else(|| by(&|p, _| p.file_stem().is_some_and(|s| same(&s.to_string_lossy()))))
        .or_else(|| {
            by(&|_, t| {
                key(t, "Exec")
                    .and_then(|e| e.split_whitespace().next().map(String::from))
                    .is_some_and(|cmd| Path::new(&cmd).file_name().is_some_and(|n| same(&n.to_string_lossy())))
            })
        })
}

/// The entries the design's autostart opens anyway (`gio launch <entry>`
/// in /etc/wadspaces/autostart.d): not opened twice.
fn autostarted(entries: &[PathBuf]) -> BTreeSet<PathBuf> {
    let mut out = BTreeSet::new();
    let Ok(rd) = std::fs::read_dir("/etc/wadspaces/autostart.d") else { return out };
    for f in rd.flatten() {
        let Ok(text) = std::fs::read_to_string(f.path()) else { continue };
        for word in text.split_whitespace().filter(|w| w.ends_with(".desktop")) {
            let name = Path::new(word).file_name();
            out.extend(entries.iter().filter(|e| e.file_name() == name).cloned());
        }
    }
    out
}

/// tmux's sessions: (name, clients attached). None without a tmux server.
fn tmux_sessions() -> Vec<(String, u32)> {
    let Ok(out) = Command::new("tmux").args(["list-sessions", "-F", "#{session_attached} #{session_name}"]).output()
    else {
        return vec![];
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (n, name) = l.split_once(' ')?;
            Some((name.to_string(), n.parse().ok()?))
        })
        .collect()
}

fn tmux(args: &[&str]) {
    let _ = Command::new("tmux").args(args).stdout(Stdio::null()).stderr(Stdio::null()).status();
}
