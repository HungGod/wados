//! The tray's sound and display flyouts (`Flyout.svelte`'s `volume` and
//! `brightness`):
//!
//! - **Sound**: a switch (sound on, or muted), the volume's slider and its
//!   percent, the outputs (the default one ticked; a click makes another
//!   the default), and Sound settings.
//! - **Display**: the brightness's slider (logind's SetBrightness, in the
//!   machine's session), night light's switch (the settings', which the
//!   shell's desk.rs follows), and Display settings.
//!
//! Each is made when it opens; while it's open, what changes (a key, a
//! drag in Settings) is shown in it rather than making it again, so a
//! slider being dragged stays under the pointer.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::{gio, prelude::*};
use wad_gtk::Icon;
use wad_gtk::form::{Slider, Toggle};

use super::system::Status;
use super::ui::{self, label, set_class};
use crate::session::media::Vol;
use crate::shell::Shell;

const CSS: &str = r#"
.wb-slider { padding: 4px 6px 8px; }
.wb-slider > wad-icon { color: var(--muted); }
.wb-pct { min-width: 30px; font-size: 12.5px; color: var(--muted); }
.wb-h4 { margin: 6px 8px 2px; font-size: 11.5px; font-weight: 600; letter-spacing: 0.06em; color: var(--faint); }
"#;

struct SoundUi {
    toggle: Toggle,
    slider: Slider,
    pct: gtk::Label,
    outputs: gtk::Box,
}

struct DisplayUi {
    slider: Option<Slider>,
    night: Toggle,
}

pub struct Levels {
    shell: Rc<Shell>,
    me: Weak<Levels>,
    sound: RefCell<Option<SoundUi>>,
    display: RefCell<Option<DisplayUi>>,
}

impl Levels {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        if let Some(d) = gtk::gdk::Display::default() {
            wad_gtk::theme::add_css(&d, CSS);
        }
        let me = Rc::new_cyclic(|me| Self {
            shell: shell.clone(),
            me: me.clone(),
            sound: RefCell::default(),
            display: RefCell::default(),
        });
        let w = Rc::downgrade(&me);
        shell.desk.on_change(move |s| {
            if let Some(m) = w.upgrade()
                && let Some(d) = &*m.display.borrow()
            {
                d.night.set(s.display.night_light);
            }
        });
        me
    }

    /// The sound flyout, into `col`.
    pub fn volume(&self, col: &gtk::Box, st: &Status, close: Rc<dyn Fn()>) {
        let v = st.volume;
        let on = v.is_some_and(|v| !v.muted);
        let s = self.shell.clone();
        let toggle = Toggle::new(on, move |on| s.sound(Vol::Muted(!on))).named("fly-sound-on");
        toggle.root.set_tooltip_text(Some("Sound on"));
        col.append(&head("Sound", Some(toggle.root.upcast_ref())));
        let Some(v) = v else {
            col.append(&ui::label("The sound server (PipeWire) isn't answering.", &["wb-p"]));
            *self.sound.borrow_mut() = None;
            col.append(&self.foot("Sound settings", "sound", close));
            return;
        };
        let row = gtk::Box::builder().spacing(10).css_classes(["wb-slider"]).build();
        let ic = Icon::new("volume-2", 18);
        ic.set_valign(gtk::Align::Center);
        row.append(&ic);
        let pct = label(&pct_of(v.level), &["wb-pct"]);
        pct.set_xalign(1.0);
        let (s, p) = (self.shell.clone(), pct.clone());
        let slider = Slider::new(
            0.0,
            1.0,
            0.01,
            if v.muted { 0.0 } else { v.level },
            move |x| {
                p.set_label(&pct_of(x));
                s.sound(Vol::Set(x));
            },
            |_| {},
        )
        .named("fly-volume");
        row.append(&slider.root);
        row.append(&pct);
        col.append(&row);
        col.append(&label("Output", &["wb-h4"]));
        let outputs = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
        col.append(&outputs);
        col.append(&self.foot("Sound settings", "sound", close));
        *self.sound.borrow_mut() = Some(SoundUi { toggle, slider, pct, outputs });
        self.outputs();
    }

    /// The outputs, read (wpctl status, off the main loop) into the open flyout.
    fn outputs(&self) {
        let w = self.me.clone();
        glib_spawn(wad_desk::sound::status, move |r| {
            let Some(me) = w.upgrade() else { return };
            let ui = me.sound.borrow();
            let Some(ui) = ui.as_ref() else { return };
            while let Some(c) = ui.outputs.first_child() {
                ui.outputs.remove(&c);
            }
            let outs = match r {
                Ok(st) => st.outputs,
                Err(e) => {
                    ui.outputs.append(&ui::label(&e, &["wb-p"]));
                    return;
                }
            };
            if outs.is_empty() {
                ui.outputs.append(&ui::label("No outputs.", &["wb-p"]));
            }
            for o in outs {
                let b = gtk::Button::builder().css_classes(["wb-row"]).build();
                b.set_widget_name(&format!("fly-output-{}", o.id));
                let r = gtk::Box::builder().spacing(12).build();
                r.append(&Icon::new(wad_desk::sound::output_icon(&o.name), 18));
                let t = label(&o.name, &[]);
                t.set_hexpand(true);
                t.set_ellipsize(gtk::pango::EllipsizeMode::End);
                r.append(&t);
                if o.default {
                    r.append(&Icon::new("check", 14));
                }
                set_class(&b, "wb-on-row", o.default);
                b.set_child(Some(&r));
                let (s, w, id) = (me.shell.clone(), me.me.clone(), o.id);
                b.connect_clicked(move |_| {
                    s.sound(Vol::Output(id));
                    let w = w.clone();
                    // Read again once wpctl's had its moment.
                    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                        if let Some(m) = w.upgrade() {
                            m.outputs();
                        }
                    });
                });
                ui.outputs.append(&b);
            }
        });
    }

    /// The display flyout, into `col`.
    pub fn brightness(&self, col: &gtk::Box, st: &Status, close: Rc<dyn Fn()>) {
        col.append(&head("Display", None));
        let slider = match st.brightness {
            Some(b) => {
                let row = gtk::Box::builder().spacing(10).css_classes(["wb-slider"]).build();
                let dim = Icon::new("sun-dim", 18);
                dim.set_valign(gtk::Align::Center);
                row.append(&dim);
                let s = self.shell.clone();
                let slider = Slider::new(
                    0.15,
                    1.0,
                    0.01,
                    b,
                    move |x| {
                        if let Err(e) = s.set_brightness(x) {
                            tracing::info!("brightness: {e}");
                        }
                    },
                    |_| {},
                )
                .named("fly-brightness");
                if !self.shell.is_machine() {
                    slider.root.set_sensitive(false);
                    slider.root.set_tooltip_text(Some(&crate::session::off_here("Brightness")));
                }
                row.append(&slider.root);
                let sun = Icon::new("sun", 18);
                sun.set_valign(gtk::Align::Center);
                row.append(&sun);
                col.append(&row);
                Some(slider)
            }
            None => {
                col.append(&ui::label("This screen has no brightness to set here.", &["wb-p"]));
                None
            }
        };
        let r = gtk::Box::builder().spacing(12).css_classes(["wb-row"]).build();
        r.append(&Icon::new("moon-star", 18));
        let t = label("Night light", &[]);
        t.set_hexpand(true);
        r.append(&t);
        let d = self.shell.desk.clone();
        let night = Toggle::new(d.settings().display.night_light, move |on| d.update(|s| s.display.night_light = on))
            .named("fly-night-light");
        night.root.set_tooltip_text(Some("Night light"));
        r.append(&night.root);
        col.append(&r);
        if let Some(e) = self.shell.desk.night_failed().filter(|_| night.is_on()) {
            col.append(&ui::label(&format!("Night light isn't working here: {e}"), &["wb-p"]));
        }
        col.append(&self.foot("Display settings", "displays", close));
        *self.display.borrow_mut() = Some(DisplayUi { slider, night });
    }

    /// What changed (a key, a click in Settings), shown in what's open.
    pub fn update(&self, st: &Status) {
        if let (Some(ui), Some(v)) = (&*self.sound.borrow(), st.volume) {
            ui.toggle.set(!v.muted);
            ui.pct.set_label(&pct_of(v.level));
            ui.slider.set(if v.muted { 0.0 } else { v.level });
        }
        if let Some(ui) = &*self.display.borrow()
            && let (Some(s), Some(b)) = (&ui.slider, st.brightness)
            && (s.value() - b).abs() > 0.02
        {
            s.set(b);
        }
    }

    fn foot(&self, text: &str, page: &'static str, close: Rc<dyn Fn()>) -> gtk::Box {
        let f = gtk::Box::builder().css_classes(["wb-foot"]).build();
        let r = gtk::Box::builder().spacing(6).build();
        r.append(&Icon::new("settings", 14));
        r.append(&gtk::Label::new(Some(text)));
        let b = gtk::Button::builder().child(&r).css_classes(["wb-quiet"]).build();
        let s = self.shell.clone();
        b.connect_clicked(move |_| {
            close();
            crate::peers::open_app(&s, &["wados-settings", "--page", page]);
        });
        f.append(&b);
        f
    }
}

/// A volume's percent ("45").
fn pct_of(v: f64) -> String {
    format!("{}", (v * 100.0).round() as i64)
}

fn head(title: &str, extra: Option<&gtk::Widget>) -> gtk::Box {
    let h = gtk::Box::builder().css_classes(["wb-fly-head"]).build();
    let t = label(title, &["wb-h3"]);
    t.set_hexpand(true);
    h.append(&t);
    if let Some(e) = extra {
        e.set_valign(gtk::Align::Center);
        h.append(e);
    }
    h
}

/// `work` off the main loop, then `done` with its answer on it.
fn glib_spawn<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    let h = gio::spawn_blocking(work);
    gtk::glib::spawn_future_local(async move {
        if let Ok(r) = h.await {
            done(r);
        }
    });
}
