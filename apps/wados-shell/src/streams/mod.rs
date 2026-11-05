//! Streams, as the shell draws them (phase 7): the stream sheet (Private
//! or Public, Secure when it can, the confirm when it's on the screen, then
//! its job's stages), the share sheet (its QR code, link and sign-in, who's
//! watching, Stop), the Stream menu's rows, and the toasts (live, didn't
//! start, stopped). The tray's Streams (bar/tray.rs), the cards' badges and
//! buttons (home), the reveal bar's Stream (overlays/reveal.rs) and the
//! desktop's menu call the verbs here.
//!
//! - **This machine's:** `POST /v1/wadspaces/{id}/stream` starts a Stream
//!   job, followed by its `job` events and the `streams` event; `DELETE`
//!   stops it; `GET /v1/streams/{id}/share` is its share, fetched when the
//!   share sheet opens and dropped when it closes (never logged).
//! - **Another machine's** ("Stream from {machine}", Launch on its card):
//!   `POST /v1/peers/{peer}/wadspaces/{ws}/stream` (a private stream there,
//!   answering its share once it's live; asked again, its share again: the
//!   share sheet's), opened here in a WadBrowser `--app` window by its
//!   one-tap link (full screen on the primary display once it shows, as a
//!   wadspace opens; Stream in a window leaves it a window); `DELETE` stops it.
//!
//! `wadsctl stream` lists them; `stream sheet <key> [mode]`, `stream share
//! <key>`, `stream stop <key>`, `stream from <key> [window]` and `stream
//! menu <key>` (its Stream menu, as text) drive it; the sheets' buttons are
//! `wadsctl dialog <name>`.

mod model;
mod qr;
mod share;
mod sheet;

pub use model::{Button, MenuItem, Pick};

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use gtk::{gdk, glib, prelude::*};
use wad_model::{JobKind, JobStatus, StreamMode};
use wad_proto::{Level, StreamInfo, StreamRequest, StreamShare};

use crate::shell::Shell;
use crate::store::{Change, Store, split_key, stream_key};

pub use model::{NOTHING, TILE_TIP, card_badge, card_button, flyout_sub, stream_menu, tile_sub};

/// The sheets', the flyout's and the badges' look (the spike's).
const CSS: &str = r#"
/* ---- the stream sheet */
.st-look { margin-bottom: 14px; }
.st-thumb { border-radius: 12px; box-shadow: inset 0 0 0 1px var(--line-strong); }
.st-name { font-weight: 600; font-size: 14px; color: var(--fg); }
.st-chip { padding: 3px 9px; border-radius: 99px; background-color: var(--surface-3); color: var(--fg); font-size: 12px; font-weight: 500; }
.st-seg { padding: 3px; border-radius: 12px; background-color: var(--surface); border: 1px solid var(--line); }
button.st-seg-b { min-height: 30px; padding: 0 12px; border-radius: 9px; border: none; background: none; box-shadow: none; color: var(--muted); font-size: 13px; }
button.st-seg-b:hover { color: var(--fg); }
button.st-seg-b.on { background-color: var(--surface-3); color: var(--fg); box-shadow: inset 0 0 0 1px var(--line-strong); }
.st-what { font-size: 13px; margin: 12px 0; color: var(--muted); }
.st-secure { padding: 12px; border-radius: 14px; border: 1px solid var(--line); background-color: var(--surface-2); }
.st-secure-t { color: var(--fg); font-weight: 600; }
.st-secure-s { font-size: 12px; color: var(--muted); }
switch.st-toggle { min-width: 38px; min-height: 22px; border-radius: 999px; border: 1px solid var(--line-strong); background-color: var(--surface-3); padding: 0; }
switch.st-toggle:checked { background-color: var(--accent); border-color: var(--accent); }
switch.st-toggle > slider { min-width: 18px; min-height: 18px; margin: 1px; border-radius: 999px; background-color: var(--fg); border: none; box-shadow: none; }
switch.st-toggle:checked > slider { background-color: var(--accent-fg); }
switch.st-toggle:disabled { opacity: 0.4; }
.st-box { margin-top: 12px; padding: 10px 12px; border-radius: 12px; font-size: 13px; }
.st-box.blocked { background-color: var(--danger-soft); color: var(--danger); }
.st-box.warn { background-color: var(--warn-soft); color: var(--warn); }
.st-box.move { background-color: var(--surface-3); color: var(--fg); }

/* ---- the share sheet */
.st-qr { padding: 6px; border-radius: 14px; background-color: #f5f2fa; }
.st-qr-wait { color: #0a0614; }
.st-pill { padding: 3px 10px; border-radius: 99px; font-size: 12px; font-weight: 600; background-color: var(--accent-soft); color: var(--accent); }
.st-pill.pub { background-color: var(--accent-2-soft); color: var(--accent-2); }
.st-p { font-size: 13px; color: var(--muted); }
.st-field { padding: 6px 6px 6px 10px; border-radius: 10px; background-color: var(--bg-2); border: 1px solid var(--line); }
.st-field-l { font-size: 11.5px; color: var(--muted); }
.st-code { font-family: monospace; font-size: 12px; color: var(--fg); }
button.st-fb { min-width: 28px; min-height: 28px; padding: 0; border-radius: 7px; border: none; box-shadow: none; background-color: var(--surface-3); color: var(--muted); }
button.st-fb:hover { color: var(--fg); }
.st-status { margin-top: 14px; font-size: 13px; color: var(--muted); }
.st-live-dot { min-width: 9px; min-height: 9px; border-radius: 99px; background-color: var(--accent-2); box-shadow: 0 0 10px var(--accent-2); animation: st-pulse 1.6s infinite; }
.st-live-dot.priv { background-color: var(--accent); box-shadow: 0 0 10px var(--accent); }
@keyframes st-pulse { 50% { opacity: 0.4; } }

/* ---- the Streams flyout */
.st-row { padding: 10px; border-radius: 12px; background-color: var(--surface-2); }
.st-row-name { font-weight: 600; font-size: 13px; }
.st-dot { min-width: 9px; min-height: 9px; border-radius: 99px; background-color: var(--accent-2); box-shadow: 0 0 8px var(--accent-2); }
.st-dot.priv { background-color: var(--accent); box-shadow: 0 0 8px var(--accent); }
button.wb-tb.st-tray { color: var(--accent-2); }
.st-tray-n { font-size: 12px; font-weight: 700; }
"#;

pub struct Streams {
    shell: Rc<Shell>,
    me: Weak<Streams>,
    sheet: Rc<sheet::StreamSheet>,
    share: Rc<share::ShareSheet>,
    /// The streams as wadd last said them (none: not yet, since connecting).
    last: RefCell<Option<Vec<StreamInfo>>>,
    /// What this shell last asked to start, by wadspace key: Retry asks it again.
    asked: RefCell<HashMap<String, StreamRequest>>,
    /// Stream jobs seen going this session (only theirs are told when they fail, not old ones').
    going: RefCell<HashSet<String>>,
    /// Stream jobs whose failure was told.
    told: RefCell<HashSet<String>>,
    /// The peers asked for their wadspaces' names (once each).
    named: RefCell<HashSet<String>>,
    /// Viewer windows to put full screen when they show (their app id), and since when.
    full: RefCell<Vec<(String, Instant)>>,
}

/// WadBrowser's web-app windows' app id prefix (wadbrowser-core's `WEBAPP_PREFIX`).
const VIEWER_APP_PREFIX: &str = "wadspaces-webapp-";

/// How long a viewer's window is waited for, to put it full screen.
const VIEWER_WAIT: Duration = Duration::from_secs(90);

thread_local! {
    static STREAMS: RefCell<Weak<Streams>> = RefCell::default();
}

fn get() -> Option<Rc<Streams>> {
    STREAMS.with(|s| s.borrow().upgrade())
}

/// Starts following wadd's streams for the shell (once, after the overlays).
pub fn start(shell: &Rc<Shell>) -> Rc<Streams> {
    if let Some(d) = gdk::Display::default() {
        wad_gtk::kit::install(&d);
        wad_gtk::theme::add_css(&d, CSS);
    }
    let me = Rc::new_cyclic(|me: &Weak<Streams>| Streams {
        shell: shell.clone(),
        me: me.clone(),
        sheet: sheet::StreamSheet::new(shell),
        share: share::ShareSheet::new(shell),
        last: RefCell::default(),
        asked: RefCell::default(),
        going: RefCell::default(),
        told: RefCell::default(),
        named: RefCell::default(),
        full: RefCell::default(),
    });
    STREAMS.with(|s| *s.borrow_mut() = Rc::downgrade(&me));
    let w = me.me.clone();
    shell.store.watch(move |c| {
        let Some(m) = w.upgrade() else { return };
        let c = c.clone();
        // Never from inside a store watcher (toasts are the store's too): next turn.
        glib::idle_add_local_once(move || m.changed(&c));
    });
    let w = me.me.clone();
    shell.placer.on_windows(move |list| {
        if let Some(m) = w.upgrade() {
            m.windows_changed(list);
        }
    });
    let w = me.me.clone();
    shell.on_ctl("stream", move |args| match w.upgrade() {
        Some(m) => m.ctl(args),
        None => "error gone".into(),
    });
    me
}

// ------------------------------------------------------------ verbs

/// A wadspace's name: this machine's by its id, a peer's by its key.
pub fn name_of(store: &Store, key: &str) -> String {
    store.any_wadspace(key).map(|x| x.name).unwrap_or_else(|| split_key(key).map(|(_, w)| w).unwrap_or(key).to_string())
}

/// The machine a stream comes from, by name (none: this one).
pub fn peer_name(store: &Store, s: &StreamInfo) -> Option<String> {
    s.peer.as_ref().map(|p| store.machine_name(p).unwrap_or_else(|| p.clone()))
}

/// The Stream menu of a wadspace (its key).
pub fn menu(store: &Store, key: &str) -> Vec<MenuItem> {
    stream_menu(store.stream(key).as_ref(), store.stream_modes().as_ref())
}

/// What a menu row or a card's button picked, for a wadspace (its key).
pub fn pick(key: &str, p: Pick) {
    if let Some(m) = get() {
        match p {
            Pick::Sheet(mode) => m.open_sheet(key, mode),
            Pick::Share => m.open_share(key),
        }
    }
}

/// The stream sheet for this machine's wadspace, at `mode` (its link, if it streams).
pub fn sheet(key: &str, mode: StreamMode) {
    if let Some(m) = get() {
        m.open_sheet(key, mode);
    }
}

/// A wadspace's stream's link (the stream sheet while it starts here).
pub fn share(key: &str) {
    if let Some(m) = get() {
        m.open_share(key);
    }
}

/// Stops a wadspace's stream (this machine's, or one from a peer).
pub fn stop(key: &str) {
    if let Some(m) = get() {
        m.stop(key);
    }
}

/// Another machine's wadspace (its key), streamed from there privately and
/// opened here in a window of WadBrowser's.
pub fn stream_from(key: &str, window: bool) {
    if let Some(m) = get() {
        m.stream_from(key, window);
    }
}

/// A stream's viewer's web-app id (its window's app id is WadBrowser's prefix and it).
pub fn viewer_id(key: &str) -> String {
    format!("stream-{key}")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect()
}

/// The program that opens a stream's viewer (WadBrowser; `WADOS_WADBROWSER` for tests).
pub fn viewer_argv(program: &str, key: &str, title: &str, open_link: &str) -> Vec<String> {
    let id = viewer_id(key);
    vec![program.into(), "--app".into(), id, "--name".into(), title.into(), "--url".into(), open_link.into()]
}

/// Opens a stream's one-tap link in a WadBrowser window of its own (never
/// logged: the key's in it). True if it started.
pub fn open_viewer(shell: &Shell, key: &str, title: &str, open_link: &str) -> bool {
    let prog = crate::signin::program();
    let argv = viewer_argv(&prog, key, title, open_link);
    let started = glib::find_program_in_path(&prog).is_some() && {
        let args: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
        gtk::gio::Subprocess::newv(&args, gtk::gio::SubprocessFlags::NONE)
            .inspect_err(|e| tracing::warn!("{prog} didn't start: {}", e.message()))
            .is_ok()
    };
    if !started {
        shell.toast(Level::Warn, "WadBrowser isn't on this machine: the stream's link can't open here", None);
    }
    started
}

pub fn now_ms() -> i64 {
    glib::real_time() / 1000
}

impl Streams {
    /// A viewer waited for showed: full screen on the primary display, in front.
    fn windows_changed(&self, list: &[crate::wm::Top]) {
        let Some(wm) = self.shell.placer.wm() else { return };
        let primary = self.shell.primary.connector().map(|c| c.to_string());
        self.full.borrow_mut().retain(|(app, at)| {
            if at.elapsed() > VIEWER_WAIT {
                return false;
            }
            let Some(t) = list.iter().find(|t| &t.app_id == app) else { return true };
            let which = || crate::wm::Which(format!("#{}", t.id));
            wm.send(crate::wm::Cmd::Fullscreen(which(), primary.clone()));
            wm.send(crate::wm::Cmd::Activate(which()));
            false
        });
    }

    fn changed(&self, c: &Change) {
        match c {
            Change::Streams => {
                self.streams_changed();
                self.sheet.refresh();
                self.share.refresh();
            }
            Change::Job(id) => {
                self.job_changed(id);
                self.sheet.refresh();
            }
            Change::Run(_) | Change::PeerSpaces(_) | Change::Peers => self.sheet.refresh(),
            Change::Connected(false) => *self.last.borrow_mut() = None,
            // (Re)connected: the streams and what can start, now (not at wadd's next change).
            Change::Connected(true) => self.ask_list(),
            _ => {}
        }
    }

    /// `GET /v1/streams` into the store (unless a `streams` event says it first).
    fn ask_list(&self) {
        let (store, asked) = (self.shell.store.clone(), self.shell.store.streams_said());
        self.shell.link.call(
            |w| async move { w.streams().await },
            move |r| match r {
                Ok(list) => {
                    store.set_streams(list, asked);
                }
                Err(e) => tracing::info!("the streams didn't come: {}", e.message),
            },
        );
    }

    /// The toasts for what changed (none for what was so when it first said).
    fn streams_changed(&self) {
        let store = &self.shell.store;
        let now = store.streams();
        let before = self.last.replace(Some(now.clone()));
        // A peer's stream, whose wadspace's name isn't known here yet: ask it.
        for s in &now {
            if let Some(p) = &s.peer
                && store.peer_wadspace(&stream_key(s)).is_none()
                && self.named.borrow_mut().insert(p.clone())
            {
                crate::peers::refresh(&self.shell, p);
            }
        }
        let Some(before) = before else { return };
        for (key, turn, s) in model::turns(&before, &now, stream_key) {
            let name = name_of(store, &key);
            match turn {
                model::Turn::Live => {
                    let (title, body) = model::live_toast(&name, &s);
                    let k = key.clone();
                    crate::overlays::toast_doing(
                        &self.shell,
                        Level::Ok,
                        &title,
                        Some(&body),
                        "Show link",
                        Rc::new(move || share(&k)),
                    );
                }
                model::Turn::Ended { was_live: true } => {
                    self.shell.toast(Level::Info, &model::stopped_title(&name), None);
                }
                // Stopped (or failed) while it started: its job says which.
                model::Turn::Ended { was_live: false } => {}
            }
        }
    }

    /// A Stream job that failed: why, and Retry.
    fn job_changed(&self, id: &str) {
        let Some(j) = self.shell.store.job(id) else { return };
        if j.job.kind != JobKind::Stream {
            return;
        }
        if matches!(j.job.status, JobStatus::Queued | JobStatus::Running) {
            self.going.borrow_mut().insert(id.to_string());
        }
        if j.job.status != JobStatus::Error || !self.going.borrow().contains(id) {
            return;
        }
        if !self.told.borrow_mut().insert(id.to_string()) {
            return;
        }
        let Some(ws) = j.job.subject.clone() else { return };
        let why = j.job.error.clone().unwrap_or_else(|| "It didn’t work".into());
        self.failed(&ws, &why);
    }

    /// `{name} didn't start streaming`, why, and Retry.
    fn failed(&self, key: &str, why: &str) {
        let name = name_of(&self.shell.store, key);
        let k = key.to_string();
        crate::overlays::toast_doing(
            &self.shell,
            Level::Error,
            &model::failed_title(&name),
            Some(why),
            "Retry",
            Rc::new(move || {
                if let Some(m) = get() {
                    m.retry(&k);
                }
            }),
        );
    }

    fn retry(&self, key: &str) {
        if split_key(key).is_some() {
            return self.stream_from(key, true);
        }
        let req = self.asked.borrow().get(key).copied();
        match req {
            Some(r) => self.sheet.start_now(key, r),
            None => self.sheet.open(key, StreamMode::Private),
        }
    }

    fn open_sheet(&self, key: &str, mode: StreamMode) {
        if split_key(key).is_some() {
            return self.stream_from(key, true);
        }
        match self.shell.store.stream(key) {
            Some(_) => self.open_share(key),
            None => self.sheet.open(key, mode),
        }
    }

    fn open_share(&self, key: &str) {
        match self.shell.store.stream(key) {
            // Starting here: its job, as it goes.
            Some(s) if !s.live && s.peer.is_none() => self.sheet.open_starting(key, &s),
            Some(_) => self.share.open(key),
            None => {
                self.shell.toast(Level::Info, &format!("{} isn’t streaming", name_of(&self.shell.store, key)), None)
            }
        }
    }

    /// Asks wadd to stream this machine's wadspace `ws`; `done` with its job, or why not.
    fn start(
        &self,
        ws: &str,
        req: StreamRequest,
        done: impl FnOnce(Result<wad_proto::JobInfo, wad_wadd_client::Failure>) + 'static,
    ) {
        self.asked.borrow_mut().insert(ws.to_string(), req);
        let id = ws.to_string();
        self.shell.link.call(move |w| async move { w.start_stream(&id, &req).await }, done);
    }

    fn stop(&self, key: &str) {
        let shell = self.shell.clone();
        let name = name_of(&shell.store, key);
        let fail = move |r: Result<StreamInfo, wad_wadd_client::Failure>| {
            if let Err(e) = r {
                shell.toast(Level::Error, &format!("{name} didn’t stop streaming: {}", e.message), None);
            }
        };
        let (peer, ws) = match split_key(key) {
            Some((p, w)) => (Some(p.to_string()), w.to_string()),
            None => (None, key.to_string()),
        };
        self.shell.link.call(
            move |w| async move {
                match peer {
                    Some(p) => w.stop_peer_stream(&p, &ws).await,
                    None => w.stop_stream(&ws).await,
                }
            },
            fail,
        );
    }

    /// "Stream from {machine}": a private stream on that machine, opened here.
    fn stream_from(&self, key: &str, window: bool) {
        let store = &self.shell.store;
        let Some((peer, ws)) = split_key(key) else {
            // This machine's opens as itself.
            let display = if window { wad_model::SessionDisplay::Window } else { wad_model::SessionDisplay::Screen };
            return self.shell.open(key, display);
        };
        let name = name_of(store, key);
        let from = store.machine_name(peer).unwrap_or_else(|| peer.to_string());
        if store.peer(peer).is_some_and(|p| !p.peer.online) {
            return crate::overlays::toast_with(
                &self.shell,
                Level::Warn,
                &format!("{from} is offline"),
                &format!(
                    "{name} streams from {from}, so it waits for it. Copy it to this machine meanwhile: right-click ▸ Copy to this machine."
                ),
            );
        }
        if let Some(why) = model::why_not(store.stream_modes().as_ref(), StreamMode::Private) {
            return crate::overlays::toast_with(
                &self.shell,
                Level::Warn,
                &format!("{name} lives on {from}"),
                &format!("{why}: streaming it here goes over your tailnet."),
            );
        }
        let req = StreamRequest { mode: StreamMode::Private, takeover: false };
        self.asked.borrow_mut().insert(key.to_string(), req);
        let title = format!("{name} · from {from}");
        let (shell, k, p, w) = (self.shell.clone(), key.to_string(), peer.to_string(), ws.to_string());
        self.shell.link.call(
            // It answers once it's live there (streaming already: its share again).
            move |wadd| async move { wadd.start_peer_stream(&p, &w, &req).await },
            move |r: Result<StreamShare, wad_wadd_client::Failure>| match r {
                Ok(s) => {
                    if open_viewer(&shell, &k, &title, &s.open_link)
                        && !window
                        && let Some(m) = get()
                    {
                        let app = format!("{}{}", VIEWER_APP_PREFIX, viewer_id(&k));
                        m.full.borrow_mut().push((app, Instant::now()));
                    }
                }
                Err(e) => {
                    if let Some(m) = get() {
                        m.failed(&k, &e.message);
                    }
                }
            },
        );
    }

    // ------------------------------------------------------------ wadsctl

    fn ctl(&self, args: &[&str]) -> String {
        let store = &self.shell.store;
        match args {
            [] => {
                let rows: Vec<String> = store
                    .streams()
                    .iter()
                    .map(|s| {
                        format!(
                            "{} {} {} live={} viewers={}",
                            stream_key(s),
                            model::mode_id(s.mode),
                            s.stage.as_str(),
                            s.live,
                            s.viewers
                        )
                    })
                    .collect();
                let can = match store.stream_modes() {
                    // Not said yet.
                    None => "?".to_string(),
                    Some(a) => {
                        let modes: Vec<&str> = model::MODES
                            .iter()
                            .filter(|m| model::why_not(Some(&a), **m).is_none())
                            .map(|m| model::mode_id(*m))
                            .collect();
                        modes.join(",")
                    }
                };
                format!("ok can={can} | {}", rows.join(" | "))
            }
            ["sheet", key, rest @ ..] => {
                let mode = rest.first().and_then(|m| model::parse_mode(m)).unwrap_or(StreamMode::Private);
                self.open_sheet(key, mode);
                "ok".into()
            }
            ["share", key] => {
                self.open_share(key);
                "ok".into()
            }
            ["stop", key] => {
                self.stop(key);
                "ok".into()
            }
            ["from", key, rest @ ..] => {
                self.stream_from(key, rest.first() == Some(&"window"));
                "ok".into()
            }
            ["menu", key] => {
                let rows: Vec<String> = menu(store, key)
                    .iter()
                    .map(|i| {
                        let mut s = format!("{} {:?}", i.id, i.hint);
                        if i.checked {
                            s.push_str(" ✓");
                        }
                        if let Some(why) = &i.disabled {
                            s.push_str(&format!(" off({why})"));
                        }
                        s
                    })
                    .collect();
                format!("ok {}", rows.join(" | "))
            }
            _ => "error stream [sheet <key> [private|public|private-secure|public-secure] | share <key> | stop <key> | from <key> [window] | menu <key>]".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_viewer_is_wadbrowsers_app_window_on_the_one_tap_link() {
        let v = viewer_argv("wadbrowser", "m-desk:notes", "Notes · from Desk", "https://d.invalid/ws-1/#token=k");
        assert_eq!(
            v,
            [
                "wadbrowser",
                "--app",
                "stream-m-desk-notes",
                "--name",
                "Notes · from Desk",
                "--url",
                "https://d.invalid/ws-1/#token=k"
            ]
        );
        // An app id is a-z, 0-9 and dashes.
        let v = viewer_argv("wadbrowser", "Studio_2", "x", "u");
        assert_eq!(v[2], "stream-studio-2");
    }
}
