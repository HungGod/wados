//! A drag source for tests (`wadsctl dnd-probe`): a small window holding
//! one pill that carries a `dnd::Held` (through `dnd::source`), so drags
//! onto the desktop, the bar and the Builder can be driven by a mouse that
//! isn't there before (or without) the Builder's stacks and sidebar. Under
//! it, two places to let go of any surface's drag, so a source of its own
//! (the desktop's) can be checked too: one a `dnd::target` (it says, as the
//! hint, what's held), one that reads the bytes the drag carries
//! ([`dnd::MIME`]).
//!
//!     wadsctl dnd-probe <held, as JSON>   opened (or its label replaced), carrying that
//!     wadsctl dnd-probe close
//!     wadsctl dnd-probe held              what's held now (`dnd::held`), as JSON, or -
//!     wadsctl dnd-probe got               what the last drag let go on its reader carried
//!     wadsctl dnd-probe landed            what the last drag let go on its target held
//!     wadsctl dnd-probe                   whether it's open, and what it carries
//!
//! Its window is an ordinary one, app id `io.wados.Probe`.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::{gdk, glib, prelude::*};

use crate::dnd::{self, Held};
use crate::shell::Shell;

pub const APP_ID: &str = "io.wados.Probe";

const CSS: &str = "
window.dnd-probe { background-color: var(--bg-2); color: var(--fg); }
.dnd-probe-about { margin: 12px 14px 0; color: var(--muted); font-size: 12px; }
.dnd-probe-pill {
  margin: 12px; padding: 6px 14px; border-radius: 999px;
  background-color: var(--surface-3); box-shadow: inset 0 0 0 1px var(--line-strong);
  font-weight: 600; font-size: 13px;
}
.dnd-probe-drop {
  margin: 0 12px 12px; padding: 14px 8px; border-radius: 12px;
  border: 1.5px dashed var(--line-strong); color: var(--muted); font-size: 12px;
}
.dnd-probe-drop.on { border-color: var(--accent); color: var(--fg); }
";

struct Win {
    window: gtk::ApplicationWindow,
    about: gtk::Label,
}

pub struct Probe {
    shell: Rc<Shell>,
    me: Weak<Probe>,
    held: RefCell<Option<Held>>,
    win: RefCell<Option<Win>>,
    /// What the last drag let go on its reader carried, and what was held then.
    got: RefCell<Option<(String, Option<Held>)>>,
    /// What was held when the last drag was let go on its target.
    landed: RefCell<Option<Held>>,
}

/// `wadsctl dnd-probe …` (it lives as long as the shell does).
pub fn install(shell: &Rc<Shell>) {
    if let Some(d) = gdk::Display::default() {
        wad_gtk::theme::add_css(&d, CSS);
        wad_gtk::theme::add_css(&d, dnd::CSS);
    }
    let me = Rc::new_cyclic(|me| Probe {
        shell: shell.clone(),
        me: me.clone(),
        held: RefCell::default(),
        win: RefCell::default(),
        got: RefCell::default(),
        landed: RefCell::default(),
    });
    shell.on_ctl("dnd-probe", move |args| me.ctl(args));
}

impl Probe {
    fn ctl(&self, args: &[&str]) -> String {
        let json = |h: Option<&Held>| h.and_then(|h| serde_json::to_string(h).ok()).unwrap_or_else(|| "-".into());
        match args {
            [] => format!("ok open={} carries={}", self.win.borrow().is_some(), json(self.held.borrow().as_ref())),
            ["close"] => {
                self.close();
                "ok".into()
            }
            ["held"] => format!("ok {}", json(dnd::held().as_ref())),
            ["landed"] => format!("ok {}", json(self.landed.borrow().as_ref())),
            ["got"] => match self.got.borrow().as_ref() {
                Some((bytes, held)) => format!("ok got={bytes} held={}", json(held.as_ref())),
                None => "ok none".into(),
            },
            _ => match serde_json::from_str::<Held>(&args.join(" ")) {
                Ok(h) => {
                    self.open(h);
                    "ok".into()
                }
                Err(e) => format!(
                    "error {e}: dnd-probe <held, as JSON (e.g. {{\"held\":\"part\",\"payload\":{{\"kind\":\"piece\",\"section\":\"addons\",\"item\":\"wadbrowser:gmail\",\"from\":\"vanua-academy\"}}}})> | close | held | got | landed"
                ),
            },
        }
    }

    /// Opened (or brought up), carrying `held`.
    fn open(&self, held: Held) {
        let text = format!("It carries {}", describe(&held));
        *self.held.borrow_mut() = Some(held);
        if let Some(w) = self.win.borrow().as_ref() {
            w.about.set_label(&text);
            w.window.present();
            return;
        }
        let window = gtk::ApplicationWindow::builder()
            .application(&self.shell.app)
            .title("Drag probe")
            .default_width(380)
            .default_height(200)
            .resizable(false)
            .css_classes(["dnd-probe"])
            .build();
        // Its own app id, set as the Builder's is (after GtkApplication's own).
        let own_id = |w: &gtk::ApplicationWindow| {
            if let Some(t) = w.surface().and_downcast::<gdk4_wayland::WaylandToplevel>() {
                t.set_application_id(APP_ID);
            }
        };
        window.connect_realize(own_id);
        window.connect_map(own_id);
        let w = self.me.clone();
        window.connect_close_request(move |_| {
            if let Some(p) = w.upgrade() {
                p.win.borrow_mut().take();
            }
            glib::Propagation::Proceed
        });

        let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        // Its labels wrap (and its size stays), so the pill stays where it is.
        let about = gtk::Label::builder()
            .label(&text)
            .css_classes(["dnd-probe-about"])
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(40)
            .build();
        col.append(&about);
        // Small, so its ghost (and the hint under it) stay by the pointer.
        let pill = gtk::Label::builder()
            .label("Drag me")
            .css_classes(["dnd-probe-pill"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .vexpand(true)
            .build();
        col.append(&pill);
        let w = self.me.clone();
        let weak_pill = pill.downgrade();
        dnd::source(&pill, move |_, _| {
            let held = w.upgrade()?.held.borrow().clone()?;
            Some((held, weak_pill.upgrade()?.upcast()))
        });
        let zones = gtk::Box::builder().homogeneous(true).build();
        let target = gtk::Label::builder()
            .label("A target: it says what's held")
            .css_classes(["dnd-probe-drop"])
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(16)
            .build();
        zones.append(&target);
        self.take_held(&target);
        let zone = gtk::Label::builder()
            .label("Let go here to see what it carries")
            .css_classes(["dnd-probe-drop"])
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(16)
            .build();
        zones.append(&zone);
        self.take_drops(&zone);
        col.append(&zones);
        window.set_child(Some(&col));
        window.present();
        *self.win.borrow_mut() = Some(Win { window, about });
    }

    fn close(&self) {
        let win = self.win.borrow_mut().take();
        if let Some(w) = win {
            w.window.destroy();
        }
    }

    /// `zone` is a target for anything held: its hint says what.
    fn take_held(&self, zone: &gtk::Label) {
        let offer = |held: &Held, _: f64, _: f64| dnd::Verdict::yes(format!("The probe: {}", describe(held)));
        let w = self.me.clone();
        let land = move |held: &Held, _: f64, _: f64| {
            if let Some(p) = w.upgrade() {
                *p.landed.borrow_mut() = Some(held.clone());
            }
        };
        let z = zone.downgrade();
        let hover = move |on: bool| {
            if let Some(z) = z.upgrade() {
                if on {
                    z.add_css_class("on");
                } else {
                    z.remove_css_class("on");
                }
            }
        };
        dnd::target(zone, offer, land, hover);
    }

    /// `zone` takes any surface's drag (ours), and keeps what it carried.
    fn take_drops(&self, zone: &gtk::Label) {
        let t = gtk::DropTargetAsync::new(Some(dnd::formats()), gdk::DragAction::COPY | gdk::DragAction::MOVE);
        let z = zone.downgrade();
        let lit = move |on: bool| {
            if let Some(z) = z.upgrade() {
                if on {
                    z.add_css_class("on");
                } else {
                    z.remove_css_class("on");
                }
            }
        };
        let l = lit.clone();
        t.connect_drag_enter(move |_, _, _, _| {
            l(true);
            gdk::DragAction::COPY
        });
        t.connect_drag_motion(|_, _, _, _| gdk::DragAction::COPY);
        let l = lit.clone();
        t.connect_drag_leave(move |_, _| l(false));
        let (w, z) = (self.me.clone(), zone.downgrade());
        t.connect_drop(move |_, drop, _, _| {
            lit(false);
            let held = dnd::held();
            let (w, z, drop) = (w.clone(), z.clone(), drop.clone());
            glib::spawn_future_local(async move {
                let bytes = read_all(&drop).await;
                drop.finish(if bytes.is_some() { gdk::DragAction::COPY } else { gdk::DragAction::empty() });
                let text = bytes.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_else(|| "-".into());
                if let Some(z) = z.upgrade() {
                    let what =
                        serde_json::from_str::<Held>(&text).map(|h| describe(&h)).unwrap_or_else(|_| text.clone());
                    z.set_label(&format!("It carried {what}"));
                }
                if let Some(p) = w.upgrade() {
                    *p.got.borrow_mut() = Some((text, held));
                }
            });
            true
        });
        zone.add_controller(t);
    }
}

/// Everything a drop carries as [`dnd::MIME`].
async fn read_all(drop: &gdk::Drop) -> Option<Vec<u8>> {
    let (stream, _) = drop.read_future(&[dnd::MIME], glib::Priority::DEFAULT).await.ok()?;
    let mut out = Vec::new();
    loop {
        let chunk = stream.read_bytes_future(64 * 1024, glib::Priority::DEFAULT).await.ok()?;
        if chunk.is_empty() {
            break;
        }
        out.extend_from_slice(&chunk);
    }
    let _ = stream.close_future(glib::Priority::DEFAULT).await;
    Some(out)
}

/// What it carries, in a few words.
fn describe(h: &Held) -> String {
    use wad_model::edit::Payload;
    match h {
        Held::Space { ws, from } => {
            format!("{ws} (a whole wadspace{})", if from.is_some() { ", its icon" } else { "" })
        }
        Held::Part { payload: Payload::Section { ws, section } } => format!("{ws}'s {}", section.label()),
        Held::Part { payload: Payload::Piece { item, from, .. } } => match from {
            Some(f) => format!("{item}, from {f}"),
            None => item.clone(),
        },
        Held::Part { payload: Payload::Folder { ws, app, key } } => {
            format!("{ws}'s {app:?} signed in with {}", key.as_deref().unwrap_or("no key"))
        }
        Held::Pieces { items, from, .. } => format!("{} of {from}'s", items.len()),
        Held::WebApp { id } => format!("the browser's web app {id}"),
    }
}
