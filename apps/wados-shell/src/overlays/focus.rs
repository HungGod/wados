//! Focus (`shell/focus`): its setup dialog, the countdown dialog, and a
//! cover on every other display. wadd holds the rule (`apps/wadd/src/focus.rs`);
//! here are the keys (main.rs's `handle` drops the ones that would leave),
//! the bell's countdown and these.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::{glib, prelude::*};
use gtk4_layer_shell::{KeyboardMode, Layer};
use wad_gtk::Icon;
use wad_gtk::kit::{self, Variant};
use wad_model::Focus;
use wad_proto::Level;

use super::{Frame, Sheet, Tone, frame, hide_sheet, show_sheet};
use crate::session::Power;
use crate::shell::Shell;
use crate::surface::{self, Anchor, Spec};

const SETUP: &str = "focus-setup";
const HELD: &str = "focus-held";
const WIDTH: i32 = 440;
const PRESETS: [u32; 6] = [15, 25, 45, 60, 120, 240];
const DEFAULT: u32 = 25;
const MAX: u32 = 12 * 60;

fn now_ms() -> i64 {
    glib::real_time() / 1000
}

/// What's left: `24:59`, `1:04:59` past the hour.
pub fn left_words(ms: i64) -> String {
    let s = (ms.max(0) + 999) / 1000;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

/// A preset's words.
pub fn preset_words(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m}"),
    }
}

/// One step of the time, kept between 0 and 12 hours.
fn stepped(minutes: u32, by: i32) -> u32 {
    (minutes as i32 + by).clamp(0, MAX as i32) as u32
}

fn clock_at(ms_from_now: i64) -> String {
    glib::DateTime::now_local()
        .and_then(|d| d.add_seconds(ms_from_now as f64 / 1000.0))
        .and_then(|d| d.format("%l:%M %P"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

pub struct FocusUi {
    shell: Rc<Shell>,
    me: Weak<FocusUi>,
    /// The last time set (the setup's default next time).
    last: Cell<u32>,
    covers: RefCell<Vec<(gtk::ApplicationWindow, gtk::Label)>>,
    held: RefCell<Option<(Rc<Frame>, gtk::Label, gtk::Label)>>,
}

impl FocusUi {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let me = Rc::new_cyclic(|me| Self {
            shell: shell.clone(),
            me: me.clone(),
            last: Cell::new(DEFAULT),
            covers: RefCell::default(),
            held: RefCell::default(),
        });
        let w = Rc::downgrade(&me);
        glib::timeout_add_local(Duration::from_secs(1), move || {
            let Some(m) = w.upgrade() else { return glib::ControlFlow::Break };
            m.tick();
            glib::ControlFlow::Continue
        });
        me
    }

    /// What's on now, and what's left of it.
    fn now(&self) -> Option<(Focus, i64)> {
        let f = self.shell.store.focus()?;
        let left = f.until - now_ms();
        Some((f, left))
    }

    fn name(&self, id: &str) -> String {
        self.shell.store.wadspace(id).map_or_else(|| id.to_string(), |w| w.name)
    }

    fn tick(&self) {
        let Some((f, left)) = self.now() else {
            self.covers.borrow_mut().drain(..).for_each(|(w, _)| w.destroy());
            if self.held.borrow_mut().take().is_some() {
                hide_sheet(HELD);
            }
            return;
        };
        let words = left_words(left);
        if self.covers.borrow().is_empty() {
            self.cover(&f);
        }
        for (_, l) in self.covers.borrow().iter() {
            l.set_label(&words);
        }
        if let Some((_, l, until)) = &*self.held.borrow() {
            l.set_label(&words);
            until.set_label(&held_body(&clock_at(left)));
        }
    }

    /// Every display but the primary.
    fn cover(&self, f: &Focus) {        let primary = self.shell.primary.connector();
        let mut covers = self.covers.borrow_mut();
        for m in crate::monitors::monitors() {
            if m.connector() == primary {
                continue;
            }
            let win = surface::make(
                &self.shell.app,
                &m,
                &Spec {
                    namespace: "wados-focus-cover",
                    layer: Layer::Overlay,
                    anchor: Anchor::Fill,
                    exclusive: -1,
                    keyboard: KeyboardMode::None,
                    height: 0,
                },
            );
            surface::stay(&win);
            let col = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(10)
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .build();
            col.append(&Icon::new("timer", 40));
            let left = kit::label("", &["ov-focus-left"]);
            col.append(&left);
            col.append(&kit::label(&format!("Focusing in {} on display 1", self.name(&f.ws)), &["ov-focus-note"]));
            let bg = gtk::Box::builder().css_classes(["ov-focus-cover"]).build();
            col.set_hexpand(true);
            bg.append(&col);
            win.set_child(Some(&bg));
            win.present();
            covers.push((win, left));
        }
    }

    fn close(&self, name: &'static str) {
        hide_sheet(name);
        self.held.borrow_mut().take();
    }

    /// The setup dialog, for `id`.
    pub fn setup(&self, id: &str) {
        let name = self.name(id);
        let w = self.me.clone();
        let f = Rc::new(frame(&format!("Focus in {name}"), "timer", Tone::Accent, WIDTH, move || {
            if let Some(m) = w.upgrade() {
                m.close(SETUP);
            }
        }));
        f.card.set_widget_name("focus-setup");
        let lead = kit::label(
            "It opens full screen, and stays there until the time’s up: no going back to the spaces, into a window or to another wadspace. Restarting the machine is the only way out early.",
            &[],
        );
        lead.set_wrap(true);
        f.body.append(&lead);

        let minutes = Rc::new(Cell::new(self.last.get()));
        let until = kit::label("", &["ov-focus-until"]);
        let shown = kit::label("", &["ov-focus-left"]);
        let cancel = kit::button("Cancel", Variant::Ghost, None, false);
        let start = kit::button("Start focus", Variant::Primary, None, false);
        let paint: Rc<dyn Fn()> = {
            let (minutes, until, shown, start) = (minutes.clone(), until.clone(), shown.clone(), start.clone());
            Rc::new(move || {
                let m = minutes.get();
                shown.set_label(&format!("{}:{:02}", m / 60, m % 60));
                start.set_sensitive(m > 0);
                until.set_label(&if m > 0 {
                    format!("Until {}", clock_at(i64::from(m) * 60_000))
                } else {
                    "Set at least a minute".into()
                });
            })
        };
        let row = gtk::Box::builder().spacing(8).halign(gtk::Align::Center).build();
        let step = |icon: &'static str, tip: &str, by: i32| {
            let b =
                gtk::Button::builder().child(&Icon::new(icon, 15)).css_classes(["ov-step-b"]).tooltip_text(tip).build();
            let (minutes, paint) = (minutes.clone(), paint.clone());
            b.connect_clicked(move |_| {
                minutes.set(stepped(minutes.get(), by));
                paint();
            });
            b
        };
        row.append(&step("minus", "An hour less", -60));
        row.append(&step("minus", "5 minutes less", -5));
        row.append(&shown);
        row.append(&step("plus", "5 minutes more", 5));
        row.append(&step("plus", "An hour more", 60));
        f.body.append(&row);
        let presets = gtk::Box::builder().spacing(6).halign(gtk::Align::Center).build();
        for p in PRESETS {
            let b = kit::button(&preset_words(p), Variant::Quiet, None, true);
            let (minutes, paint) = (minutes.clone(), paint.clone());
            b.connect_clicked(move |_| {
                minutes.set(p);
                paint();
            });
            presets.append(&b);
        }
        f.body.append(&presets);
        f.body.append(&until);
        paint();

        let w = self.me.clone();
        cancel.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.close(SETUP);
            }
        });
        let go: Rc<dyn Fn()> = {
            let (w, id, minutes) = (self.me.clone(), id.to_string(), minutes.clone());
            Rc::new(move || {
                if let Some(m) = w.upgrade() {
                    m.start(&id, minutes.get());
                }
            })
        };
        let g = go.clone();
        start.connect_clicked(move |_| g());
        f.foot.append(&cancel);
        f.foot.append(&start);
        let st = {
            let (minutes, id) = (minutes.clone(), id.to_string());
            Rc::new(move || format!("focus-setup {id} {} min", minutes.get()))
        };
        let press = {
            let g = go.clone();
            Rc::new(move |what: &str| {
                if what == "ok" {
                    g();
                }
                what == "ok"
            })
        };
        show_sheet(Sheet {
            name: SETUP,
            card: f.card.clone().upcast(),
            closed: Rc::new(|| {}),
            enter: go,
            press,
            state: st,
        });
    }

    /// Start focus: what wadd says no to is said.
    pub fn start(&self, id: &str, minutes: u32) {
        if minutes == 0 {
            return;
        }
        self.last.set(minutes);
        hide_sheet(SETUP);
        let id = id.to_string();
        let s = self.shell.clone();
        self.shell.link.call(
            move |w| async move { w.focus_start(&id, minutes).await },
            move |r| {
                if let Err(e) = r {
                    s.toast(Level::Warn, &e.message, None);
                }
            },
        );
    }

    /// The countdown dialog (the bar's Focus button).
    pub fn held(&self) {
        let Some((f, left)) = self.now() else { return };
        let name = self.name(&f.ws);
        let w = self.me.clone();
        let fr = Rc::new(frame(&format!("Focusing in {name}"), "timer", Tone::Accent, WIDTH, move || {
            if let Some(m) = w.upgrade() {
                m.close(HELD);
            }
        }));
        fr.card.set_widget_name("focus-held");
        let big = kit::label(&left_words(left), &["ov-focus-left"]);
        big.set_halign(gtk::Align::Center);
        fr.body.append(&big);
        let body = kit::label(&held_body(&clock_at(left)), &[]);
        body.set_wrap(true);
        fr.body.append(&body);
        let restart = kit::button("Restart now", Variant::Danger, None, false);
        let keep = kit::button("Keep focusing", Variant::Primary, None, false);
        let s = self.shell.clone();
        restart.connect_clicked(move |_| s.power(Power::Restart));
        let w = self.me.clone();
        let close: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(m) = w.upgrade() {
                m.close(HELD);
            }
        });
        let c = close.clone();
        keep.connect_clicked(move |_| c());
        fr.foot.append(&restart);
        fr.foot.append(&keep);
        *self.held.borrow_mut() = Some((fr.clone(), big, body));
        show_sheet(Sheet {
            name: HELD,
            card: fr.card.clone().upcast(),
            closed: Rc::new(|| {}),
            enter: close,
            press: Rc::new(|_| false),
            state: Rc::new(|| "focus-held".to_string()),
        });
    }
}

/// "Turn on server mode?": what's on the screen stops, and the session ends.
pub fn server_confirm(shell: &Rc<Shell>) {
    let s = shell.clone();
    super::confirm(super::Confirm {
        title: "Turn on server mode?".into(),
        icon: "server",
        body: "The screen goes off and this machine only streams. Wadspaces open on this screen or in windows stop; streams keep going. To come back, press Ctrl+Alt+Delete and unlock.".into(),
        ok: "Turn On".into(),
        run: Rc::new(move || {
            let s2 = s.clone();
            s.link.call(
                |w| async move { w.server_set(true).await },
                move |r| {
                    if let Err(e) = r {
                        s2.toast(Level::Warn, &e.message, None);
                    }
                },
            );
        }),
    });
}

fn held_body(until: &str) -> String {
    format!(
        "left, until {until}. It stays on the screen until then. Restarting the machine is the only way out, and anything unsaved in it is lost."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whats_left_reads_as_a_clock_and_rounds_up() {
        assert_eq!(left_words(25 * 60_000), "25:00");
        assert_eq!(left_words(1), "00:01");
        assert_eq!(left_words(0), "00:00");
        assert_eq!(left_words(-5), "00:00");
        assert_eq!(left_words(65 * 60_000 - 1000), "1:04:59");
    }

    #[test]
    fn presets_and_steps_read_and_stay_in_range() {
        assert_eq!(preset_words(25), "25 min");
        assert_eq!(preset_words(60), "1 h");
        assert_eq!(preset_words(240), "4 h");
        assert_eq!(stepped(3, -5), 0);
        assert_eq!(stepped(12 * 60 - 1, 60), 12 * 60);
    }
}
