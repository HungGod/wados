//! What shows over everything, full-screen wadspaces included (OVERLAY
//! layer): toasts, Super+Tab's switcher, Alt+Space's search, the reveal bar
//! over a full-screen wadspace, and the dialogs (a device asking to be
//! trusted, the shell's yes-or-no: [`confirm`]). And, since they're per
//! display too, the other displays' own bars (bar::Displays, TOP layer).
//!
//! The interface main.rs uses: `Overlays::new(shell)`. Its `wadsctl`
//! commands, registered with the shell:
//!
//! - `switcher next|prev` (labwc's Super+Tab, Super+Shift+Tab), `switcher
//!   hold` (open until Enter, Esc, Super let go or `commit`: for tests,
//!   whose virtual keyboards labwc doesn't tell it about), `switcher
//!   commit|cancel`, `switcher` (what it shows);
//! - `search` (Alt+Space: open, or close), `search close|enter|state`,
//!   `search for <words>` (type them);
//! - `spaces`: a full-screen wadspace to the background, and the desktop
//!   (what Ctrl+Alt+↓ should run);
//! - `toast [info|ok|warn|error] <words>`, `toasts` (what's showing);
//! - `dialog` (what's up), `dialog later|deny|trust|ok|cancel|close`;
//! - `reveal [down|up] [display]`, `displays`;
//! - `log`: what it was asked to show.

mod dialog;
mod reveal;
mod search;
mod switcher;
mod toasts;

pub use dialog::{Confirm, confirm, device_icon};
pub use toasts::{expect_body, toast_with};

use std::cell::RefCell;
use std::rc::Rc;

use wad_model::SessionDisplay;
use wad_proto::Level;

use crate::bar::{Displays, in_front};
use crate::machine::Page;
use crate::shell::Shell;
use crate::store::Change;

const CSS: &str = r#"
window.wados-toasts, window.wados-switcher, window.wados-reveal, window.wados-dialog { background: transparent; }
window.wados-search { background-color: rgb(5 3 12 / 0.5); }

/* Toasts. */
.ov-toast {
  padding: 12px 10px 12px 12px;
  border-radius: 14px;
  border: 1px solid var(--line-strong);
  background-color: rgb(27 19 45 / 0.97);
  box-shadow: 0 10px 24px -14px var(--shadow);
  margin: 0 0 2px 0;
}
.ov-tic { min-width: 28px; min-height: 28px; border-radius: 9px; background-color: var(--surface-3); color: var(--muted); }
.ov-toast.wb-ok .ov-tic { background-color: var(--accent-soft); color: var(--accent); }
.ov-toast.wb-error .ov-tic { background-color: var(--danger-soft); color: var(--danger); }
.ov-toast.wb-warn .ov-tic { background-color: var(--warn-soft); color: var(--warn); }
.ov-toast-title { font-size: 13px; font-weight: 600; }
.ov-toast-body { font-size: 12.5px; color: var(--muted); }

/* A dialog (ui/Dialog.svelte): a card over the dimmed screen. */
.ov-scrim { background-color: var(--dim); }
.ov-dialog {
  border-radius: 24px;
  border: 1px solid var(--line-strong);
  background-color: rgb(20 13 34 / 0.97);
  box-shadow: 0 30px 80px -20px var(--shadow);
}
.ov-dialog-head { padding: 20px 20px 0; }
.ov-dialog-badge { border-radius: 12px; }
.ov-dialog-badge.warn { background-color: var(--warn-soft); color: var(--warn); }
.ov-dialog-title { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 18px; letter-spacing: -0.18px; }
button.ov-dialog-x { min-width: 30px; min-height: 30px; padding: 0; border: none; border-radius: 8px; background: none; box-shadow: none; color: var(--muted); }
button.ov-dialog-x:hover { background-color: var(--surface-2); color: var(--fg); }
.ov-dialog-body { padding: 16px 20px 20px; color: var(--muted); }
.ov-dialog-foot { padding: 0 20px 20px; }
button.ov-act, button.ov-x, button.ov-r, button.ov-fs-b {
  background: none; border: none; box-shadow: none; outline: none; text-shadow: none;
  color: var(--fg); min-height: 0; min-width: 0; padding: 0;
}
button.ov-act { border: 1px solid var(--line-strong); background-color: var(--surface-3); border-radius: 8px; min-height: 26px; padding: 0 10px; font-size: 12.5px; font-weight: 600; }
button.ov-act:hover { border-color: var(--accent); }
button.ov-x { min-width: 24px; min-height: 24px; border-radius: 6px; color: var(--faint); }
button.ov-x:hover { color: var(--fg); background-color: var(--surface-3); }

/* Super+Tab. */
.ov-switcher {
  padding: 14px;
  border-radius: 22px;
  border: 1px solid var(--line-strong);
  background-color: rgb(14 9 25 / 0.94);
}
.ov-sw-item { padding: 12px 8px 10px; border-radius: 16px; color: var(--muted); }
.ov-sw-item.ov-on { background-color: rgb(255 255 255 / 0.1); box-shadow: inset 0 0 0 2px rgb(255 255 255 / 0.85); color: var(--fg); }
.ov-sw-name { font-size: 12.5px; font-weight: 600; }

/* Alt+Space. */
.ov-search {
  border-radius: 18px;
  border: 1px solid var(--line-strong);
  background-color: rgb(14 9 25 / 0.97);
  box-shadow: 0 30px 80px -20px rgb(0 0 0 / 0.85);
}
.ov-field { padding: 16px 18px; border-bottom: 1px solid var(--line); color: var(--muted); }
text.ov-q, .ov-q { font-size: 18px; font-weight: 500; color: var(--fg); background: none; }
.ov-kbd { padding: 3px 7px; border-radius: 6px; border: 1px solid var(--line-strong); font-size: 11px; color: var(--faint); }
.ov-results { padding: 6px; }
button.ov-r { padding: 8px 10px; border-radius: 11px; }
button.ov-r.ov-on { background-color: rgb(127 211 255 / 0.16); }
.ov-r-name { font-size: 14px; font-weight: 600; }
.ov-r-kind { font-size: 12px; color: var(--muted); }
.ov-r-ws { font-size: 12px; color: var(--muted); }
.ov-dot { min-width: 7px; min-height: 7px; border-radius: 99px; background-color: var(--accent); }
.ov-none { padding: 18px; color: var(--muted); }
.ov-foot { padding: 9px 16px; border-top: 1px solid var(--line); font-size: 11.5px; color: var(--faint); }

/* A full-screen wadspace's bar: tucked up out of sight until it's wanted. */
.ov-fs {
  padding: 6px 6px 6px 14px;
  border-radius: 14px;
  border: 1px solid var(--line-strong);
  background-color: rgb(27 19 45 / 0.96);
  box-shadow: 0 10px 24px -12px var(--shadow);
  transform: translateY(-64px);
  opacity: 0;
  transition: transform 250ms cubic-bezier(0.2, 0.85, 0.25, 1), opacity 200ms ease;
}
.ov-fs.ov-down { transform: none; opacity: 1; }
.ov-fs-name { font-weight: 600; }
.ov-fs-look { color: var(--muted); font-size: 13px; }
button.ov-fs-b { min-height: 32px; padding: 0 10px; border-radius: 9px; font-size: 13px; }
button.ov-fs-b:hover { background-color: var(--surface-3); }
button.ov-fs-b:disabled { color: var(--faint); }
button.ov-fs-b.ov-stop { color: var(--danger); }
"#;

pub struct Overlays {
    toasts: Rc<toasts::Toasts>,
    dialogs: Rc<dialog::Dialogs>,
    switcher: Rc<switcher::Switcher>,
    search: Rc<search::Search>,
    reveal: Rc<reveal::Reveal>,
    displays: Displays,
    /// What it was asked to show (tests read it back: `wadsctl log`).
    log: RefCell<Vec<String>>,
    /// The wadspace in front last (to tell the switcher).
    front: RefCell<Option<String>>,
}

impl Overlays {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        if let Some(d) = gtk::gdk::Display::default() {
            wad_gtk::kit::install(&d);
            wad_gtk::theme::add_css(&d, CSS);
        }
        let me = Rc::new(Self {
            toasts: toasts::Toasts::new(shell),
            dialogs: dialog::Dialogs::new(shell),
            switcher: switcher::Switcher::new(shell),
            search: search::Search::new(shell),
            reveal: Rc::new(reveal::Reveal::new(shell)),
            displays: Displays::new(shell),
            log: RefCell::default(),
            front: RefCell::default(),
        });
        let w = Rc::downgrade(&me);
        me.switcher.on_pick(move |id| {
            if let Some(m) = w.upgrade() {
                m.note(&format!("switcher: {id}"));
            }
        });
        let w = Rc::downgrade(&me);
        me.search.on_open(move |r| {
            if let Some(m) = w.upgrade() {
                m.note(&format!("search: {}/{}", r.ws, r.variant));
            }
        });

        let (w, s) = (Rc::downgrade(&me), shell.clone());
        shell.store.watch(move |c| {
            let Some(m) = w.upgrade() else { return };
            match c {
                Change::Notice { level, title, undo } => {
                    m.note(title);
                    m.toasts.push(*level, title, undo.clone());
                }
                Change::Run(_) => {
                    let running: Vec<String> = s.store.running().into_iter().map(|w| w.id).collect();
                    m.switcher.mru.borrow_mut().keep(&running);
                }
                _ => {}
            }
        });
        let (w, s) = (Rc::downgrade(&me), shell.clone());
        shell.placer.on_windows(move |list| {
            let Some(m) = w.upgrade() else { return };
            m.reveal.windows_changed();
            // The wadspace in front is the one used most lately.
            let front = list.iter().find(|t| in_front(t)).and_then(|t| t.wadspace()).map(str::to_string);
            if let Some(id) = &front
                && m.front.borrow().as_ref() != Some(id)
                && s.store.is_running(id)
            {
                m.switcher.mru.borrow_mut().touch(id);
            }
            *m.front.borrow_mut() = front;
        });
        me.ctl(shell);
        me
    }

    fn note(&self, what: &str) {
        let mut log = self.log.borrow_mut();
        log.push(what.to_string());
        let n = log.len();
        if n > 200 {
            log.drain(..n - 200);
        }
    }

    fn ctl(self: &Rc<Self>, shell: &Rc<Shell>) {
        let w = Rc::downgrade(self);
        shell.on_ctl("switcher", move |args| {
            let Some(m) = w.upgrade() else { return "error gone".into() };
            match args {
                [] => m.switcher.state(),
                // Noted as asked (Super+Tab's keybind; tests read it back).
                ["next"] => {
                    m.note("switcher next");
                    m.switcher.step(false)
                }
                ["prev"] => {
                    m.note("switcher prev");
                    m.switcher.step(true)
                }
                ["hold"] => m.switcher.hold(),
                ["commit"] => {
                    m.switcher.commit();
                    "ok".into()
                }
                ["cancel"] => {
                    m.switcher.cancel();
                    "ok".into()
                }
                _ => "error switcher [next|prev|hold|commit|cancel]".into(),
            }
        });
        let w = Rc::downgrade(self);
        shell.on_ctl("search", move |args| {
            let Some(m) = w.upgrade() else { return "error gone".into() };
            match args {
                [] => {
                    m.search.toggle();
                    if m.search.is_open() {
                        m.note("search");
                    }
                    "ok".into()
                }
                ["state"] => m.search.state(),
                ["close"] => {
                    m.search.close();
                    "ok".into()
                }
                ["enter"] => {
                    m.search.enter();
                    "ok".into()
                }
                ["for", words @ ..] => {
                    m.search.set_query(&words.join(" "));
                    m.search.state()
                }
                _ => "error search [state|close|enter|for <words>]".into(),
            }
        });
        let w = Rc::downgrade(self);
        let s = shell.clone();
        shell.on_ctl("spaces", move |_| {
            let Some(m) = w.upgrade() else { return "error gone".into() };
            // Off a full-screen wadspace (it keeps running) to the desktop.
            let id = m.reveal.primary_ws().or_else(|| {
                s.store
                    .running()
                    .into_iter()
                    .map(|w| w.id)
                    .find(|id| s.store.display(id) == Some(SessionDisplay::Screen))
            });
            if let Some(id) = &id {
                s.open(id, SessionDisplay::Background);
            }
            s.go(Page::Desktop);
            format!("ok {}", id.unwrap_or_default())
        });
        let (w, s) = (Rc::downgrade(self), shell.clone());
        shell.on_ctl("windowed", move |_| {
            let Some(m) = w.upgrade() else { return "error gone".into() };
            // Ctrl+Alt+↓: the full-screen wadspace into a window (none: nothing).
            let id = m.reveal.primary_ws().or_else(|| {
                s.store
                    .running()
                    .into_iter()
                    .map(|w| w.id)
                    .find(|id| s.store.display(id) == Some(SessionDisplay::Screen))
            });
            match id {
                Some(id) => {
                    s.open(&id, SessionDisplay::Window);
                    format!("ok {id}")
                }
                None => "ok".into(),
            }
        });
        let (w, s) = (Rc::downgrade(self), shell.clone());
        shell.on_ctl("toast", move |args| {
            if w.upgrade().is_none() {
                return "error gone".into();
            }
            let (level, words) = match args.split_first() {
                Some((&"info", rest)) => (Level::Info, rest),
                Some((&"ok", rest)) => (Level::Ok, rest),
                Some((&"warn", rest)) => (Level::Warn, rest),
                Some((&"error", rest)) => (Level::Error, rest),
                _ => (Level::Info, args),
            };
            if words.is_empty() {
                return "error toast [info|ok|warn|error] <words>".into();
            }
            s.toast(level, &words.join(" "), None);
            "ok".into()
        });
        let w = Rc::downgrade(self);
        shell.on_ctl("dialog", move |args| {
            let Some(m) = w.upgrade() else { return "error gone".into() };
            match args {
                [] => m.dialogs.state(),
                [what @ ("later" | "deny" | "trust" | "ok" | "cancel" | "close")] => {
                    if m.dialogs.press(what) {
                        m.note(&format!("dialog {what}"));
                        "ok".into()
                    } else {
                        format!("error no {what} here: {}", m.dialogs.state())
                    }
                }
                _ => "error dialog [later|deny|trust|ok|cancel|close]".into(),
            }
        });
        let w = Rc::downgrade(self);
        shell.on_ctl("toasts", move |_| match w.upgrade() {
            Some(m) => {
                let t: Vec<String> = m
                    .toasts
                    .shown()
                    .into_iter()
                    .map(|t| match t.body {
                        Some(b) => format!("{}: {b}", t.title),
                        None => t.title,
                    })
                    .collect();
                let size = m.toasts.size().map(|(w, h)| format!("{w}x{h}")).unwrap_or_else(|| "hidden".into());
                format!("ok {size} {}", t.join(" | "))
            }
            None => "error gone".into(),
        });
        let w = Rc::downgrade(self);
        shell.on_ctl("reveal", move |args| {
            let Some(m) = w.upgrade() else { return "error gone".into() };
            let n = |s: Option<&&str>| s.and_then(|s| s.parse::<usize>().ok()).unwrap_or(1);
            match args {
                [] => m.reveal.state(),
                ["down", rest @ ..] | ["up", rest @ ..] => {
                    let down = args[0] == "down";
                    if m.reveal.show(n(rest.first()), down) {
                        "ok".into()
                    } else {
                        "error nothing full screen there".into()
                    }
                }
                _ => "error reveal [down|up] [display]".into(),
            }
        });
        let w = Rc::downgrade(self);
        shell.on_ctl("displays", move |_| match w.upgrade() {
            Some(m) => format!("ok {}", m.displays.describe()),
            None => "error gone".into(),
        });
        let w = Rc::downgrade(self);
        shell.on_ctl("log", move |_| match w.upgrade() {
            Some(m) => format!("ok {}", m.log.borrow().join(" | ")),
            None => "error gone".into(),
        });
    }
}
