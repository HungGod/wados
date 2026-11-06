//! Settings ▸ Sound (the spike's `panels/SoundPanel.svelte`), on PipeWire
//! through its own tools (wad-desk's sound module):
//!
//! - **Output**: the output device (wpctl's sinks; the one picked becomes
//!   the default), Test (a chime, left then right then both, played with
//!   pw-play while a meter for each speaker follows it), the volume and mute.
//! - **Input**: the input device (the sources), its volume, and its level
//!   live (pw-record from it while the page is on screen).
//!
//! It reads `wpctl status` when it's opened, after each change, and every
//! couple of seconds while it's on screen (another app may change it).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use gtk::{glib, prelude::*};
use wad_desk::sound::{self, Channel, Device, Status};
use wad_gtk::form::{Choice, Level, Slider, opt};
use wad_gtk::kit::{self, Group, Row, Tone, Variant};

use crate::panel::{Ctx, Panel};
use crate::parts;

const CSS: &str = r#"
.snd-test { padding: 14px; background-color: var(--bg-2); }
.snd-spk { font-size: 12.5px; color: var(--faint); }
.snd-spk.on { color: var(--fg); }
.snd-muted { font-size: 12.5px; color: var(--warn); }
"#;

pub fn make(_: &Ctx) -> Rc<dyn Panel> {
    Rc::new_cyclic(|me| Sound {
        me: me.clone(),
        status: RefCell::new(Err("not read yet".into())),
        ui: RefCell::default(),
        listening: RefCell::default(),
        testing: Cell::new(false),
    })
}

/// "45%", or "Muted".
pub fn pct_of(d: &Device) -> String {
    if d.muted { "Muted".into() } else { format!("{}%", (d.volume * 100.0).round() as i64) }
}

/// The mute button's glyph (the spike's).
pub fn mute_icon(d: &Device) -> &'static str {
    if d.muted {
        "volume-x"
    } else if d.volume < 0.4 {
        "volume-1"
    } else {
        "volume-2"
    }
}

struct Ui {
    out_row: Row,
    output: Choice<u32>,
    test: gtk::Button,
    test_box: gtk::Box,
    left: (gtk::Box, Level),
    right: (gtk::Box, Level),
    muted_note: gtk::Label,
    mute: gtk::Button,
    volume: Slider,
    pct: gtk::Label,
    in_row: Row,
    input: Choice<u32>,
    in_volume: Slider,
    in_pct: gtk::Label,
    level: Level,
    note: gtk::Label,
    root: gtk::Box,
}

pub struct Sound {
    me: Weak<Sound>,
    status: RefCell<Result<Status, String>>,
    ui: RefCell<Option<Rc<Ui>>>,
    listening: RefCell<Option<sound::Listening>>,
    testing: Cell<bool>,
}

impl Sound {
    fn ui(&self) -> Option<Rc<Ui>> {
        self.ui.borrow().clone()
    }

    /// Reads it again (off the main loop), then shows it.
    fn read(&self) {
        let w = self.me.clone();
        parts::spawn(sound::status, move |r| {
            if let Some(me) = w.upgrade() {
                *me.status.borrow_mut() = r;
                me.show();
            }
        });
    }

    /// Runs a change (off the main loop), then reads it all again.
    fn change(&self, f: impl FnOnce() -> Result<(), String> + Send + 'static) {
        let w = self.me.clone();
        parts::spawn(f, move |r| {
            let Some(me) = w.upgrade() else { return };
            if let Err(e) = r {
                me.say(&e);
            }
            me.read();
        });
    }

    fn say(&self, e: &str) {
        if let Some(ui) = self.ui() {
            ui.note.set_label(e);
            ui.note.set_visible(!e.is_empty());
        }
    }

    fn show(&self) {
        let Some(ui) = self.ui() else { return };
        let st = match &*self.status.borrow() {
            Ok(s) => s.clone(),
            Err(e) => {
                self.say(&format!("The sound server (PipeWire) isn't answering: {e}"));
                return;
            }
        };
        self.say("");
        match st.output() {
            Some(o) => {
                ui.output.set_options(st.outputs.iter().map(|d| opt(d.id, &d.name)).collect::<Vec<_>>());
                ui.output.set(o.id);
                parts::set_subtitle(&ui.out_row, &o.name);
                if let Some(t) = ui.out_row.root.first_child()
                    && let Some(i) = t.first_child().and_downcast::<wad_gtk::Icon>()
                {
                    i.set(sound::output_icon(&o.name));
                }
                if let Some(i) = ui.mute.child().and_then(|c| c.first_child()).and_downcast::<wad_gtk::Icon>() {
                    i.set(mute_icon(o));
                }
                ui.mute.set_tooltip_text(Some(if o.muted { "Unmute" } else { "Mute" }));
                ui.volume.set(if o.muted { 0.0 } else { o.volume.min(1.0) });
                ui.pct.set_label(&pct_of(o));
                ui.muted_note.set_visible(o.muted);
            }
            None => {
                parts::set_subtitle(&ui.out_row, "No outputs");
                ui.output.set_options(Vec::<wad_gtk::form::Opt<u32>>::new());
            }
        }
        match st.input() {
            Some(i) => {
                ui.input.set_options(st.inputs.iter().map(|d| opt(d.id, &d.name)).collect::<Vec<_>>());
                ui.input.set(i.id);
                parts::set_subtitle(&ui.in_row, &i.name);
                ui.in_volume.set(i.volume.min(1.0));
                ui.in_pct.set_label(&format!("{}%", (i.volume * 100.0).round() as i64));
            }
            None => parts::set_subtitle(&ui.in_row, "No inputs"),
        }
    }

    /// The speaker test: the chime played, the meters following it.
    fn test(&self) {
        if self.testing.replace(true) {
            return;
        }
        let Some(ui) = self.ui() else { return };
        let file = glib::user_runtime_dir().join("wados-sound-test.wav");
        let child = match sound::play_test(&file) {
            Ok(c) => Some(c),
            Err(e) => {
                self.say(&e);
                None
            }
        };
        let loud = self
            .status
            .borrow()
            .as_ref()
            .ok()
            .and_then(|s| s.output().map(|o| if o.muted { 0.0 } else { o.volume }))
            .unwrap_or(0.0);
        ui.test.set_sensitive(false);
        ui.test_box.set_visible(true);
        let start = Instant::now();
        let (w, child) = (self.me.clone(), RefCell::new(child));
        let (l, r) = (Cell::new(0.0f64), Cell::new(0.0f64));
        glib::timeout_add_local(Duration::from_millis(50), move || {
            let Some(me) = w.upgrade() else { return glib::ControlFlow::Break };
            let Some(ui) = me.ui() else { return glib::ControlFlow::Break };
            let t = start.elapsed().as_secs_f64();
            let ch = sound::test_channel(t);
            let wobble = || 0.55 + glib::random_double() * 0.45;
            let left = matches!(ch, Some(Channel::Left | Channel::Both));
            let right = matches!(ch, Some(Channel::Right | Channel::Both));
            l.set(if left { loud * wobble() } else { l.get() * 0.7 });
            r.set(if right { loud * wobble() } else { r.get() * 0.7 });
            ui.left.1.set(l.get());
            ui.right.1.set(r.get());
            for (b, on) in [(&ui.left.0, left), (&ui.right.0, right)] {
                if on {
                    b.add_css_class("on");
                } else {
                    b.remove_css_class("on");
                }
            }
            if ch.is_some() {
                return glib::ControlFlow::Continue;
            }
            if let Some(mut c) = child.borrow_mut().take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            me.testing.set(false);
            ui.test.set_sensitive(true);
            ui.test_box.set_visible(false);
            glib::ControlFlow::Break
        });
    }

    /// The input level while the page is on screen.
    fn listen(&self, on: bool) {
        if !on {
            self.listening.borrow_mut().take();
            if let Some(ui) = self.ui() {
                ui.level.set(0.0);
            }
            return;
        }
        if self.listening.borrow().is_some() {
            return;
        }
        let (tx, rx) = async_channel::unbounded::<f64>();
        match sound::listen(move |v| {
            let _ = tx.send_blocking(v);
        }) {
            Ok(l) => *self.listening.borrow_mut() = Some(l),
            Err(e) => {
                tracing::info!("input level: {e}");
                return;
            }
        }
        let w = self.me.clone();
        glib::spawn_future_local(async move {
            let mut shown = 0.0f64;
            while let Ok(v) = rx.recv().await {
                let Some(me) = w.upgrade() else { return };
                let Some(ui) = me.ui() else { return };
                // Up at once, down gently (a meter's ballistics).
                shown = if v > shown { v } else { shown + (v - shown) * 0.3 };
                ui.level.set(shown);
            }
        });
    }
}

fn speaker(name: &str) -> (gtk::Box, Level) {
    let b = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .hexpand(true)
        .css_classes(["snd-spk"])
        .build();
    b.append(&gtk::Label::builder().label(name).xalign(0.0).build());
    let l = Level::new(28);
    b.append(&l.root);
    (b, l)
}

impl Panel for Sound {
    fn build(&self, _: &Ctx) -> gtk::Widget {
        parts::install();
        if let Some(d) = gtk::gdk::Display::default() {
            wad_gtk::theme::add_css(&d, CSS);
        }
        let page = kit::Page::new("Sound", false);
        let note = parts::note("", "set-err");
        page.add(&note);

        let out = Group::new("Output", "");
        let out_row = Row::new(Some("speaker"), Tone::Accent, "Output Device", "");
        let w = self.me.clone();
        let output = Choice::new(Vec::<wad_gtk::form::Opt<u32>>::new(), 0, move |id: &u32| {
            if let Some(me) = w.upgrade() {
                let id = *id;
                me.change(move || sound::set_default(id));
            }
        })
        .named("output");
        out_row.ctl.append(&output.root);
        let test = kit::button("Test", Variant::Secondary, Some("play"), true);
        test.set_widget_name("sound-test");
        let w = self.me.clone();
        test.connect_clicked(move |_| {
            if let Some(me) = w.upgrade() {
                me.test();
            }
        });
        out_row.ctl.append(&test);
        out.add(&out_row.root);
        let test_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["snd-test"])
            .visible(false)
            .build();
        test_box.set_widget_name("sound-test-meters");
        let spk = gtk::Box::builder().spacing(16).homogeneous(true).build();
        let (left, right) = (speaker("Front Left"), speaker("Front Right"));
        spk.append(&left.0);
        spk.append(&right.0);
        test_box.append(&spk);
        let muted_note = kit::label("The output is muted, so the test is silent.", &["snd-muted"]);
        muted_note.set_visible(false);
        test_box.append(&muted_note);
        out.add(&test_box);
        let vrow = Row::new(None, Tone::Plain, "Volume", "");
        parts::wide(&vrow);
        let mute = kit::button("", Variant::Ghost, Some("volume-2"), true);
        mute.set_widget_name("mute");
        mute.set_tooltip_text(Some("Mute"));
        let w = self.me.clone();
        mute.connect_clicked(move |_| {
            let Some(me) = w.upgrade() else { return };
            let muted = me.status.borrow().as_ref().ok().and_then(|s| s.output().map(|o| o.muted)).unwrap_or(false);
            me.change(move || sound::set_mute(sound::SINK, !muted));
        });
        vrow.ctl.append(&mute);
        let pct = parts::pct("");
        pct.set_size_request(46, -1);
        let (w, p) = (self.me.clone(), pct.clone());
        let volume = Slider::new(
            0.0,
            1.0,
            0.01,
            0.0,
            move |v| p.set_label(&format!("{}%", (v * 100.0).round() as i64)),
            move |v| {
                if let Some(me) = w.upgrade() {
                    me.change(move || sound::set_volume(sound::SINK, v));
                }
            },
        )
        .named("output-volume");
        vrow.ctl.append(&volume.root);
        vrow.ctl.append(&pct);
        out.add(&vrow.root);
        page.add(&out.root);

        let inp = Group::new("Input", "");
        let in_row = Row::new(Some("mic"), Tone::Plain, "Input Device", "");
        let w = self.me.clone();
        let input = Choice::new(Vec::<wad_gtk::form::Opt<u32>>::new(), 0, move |id: &u32| {
            if let Some(me) = w.upgrade() {
                let id = *id;
                me.change(move || sound::set_default(id));
            }
        })
        .named("input");
        in_row.ctl.append(&input.root);
        inp.add(&in_row.root);
        let ivrow = Row::new(None, Tone::Plain, "Volume", "");
        parts::wide(&ivrow);
        let in_pct = parts::pct("");
        in_pct.set_size_request(46, -1);
        let (w, p) = (self.me.clone(), in_pct.clone());
        let in_volume = Slider::new(
            0.0,
            1.0,
            0.01,
            0.0,
            move |v| p.set_label(&format!("{}%", (v * 100.0).round() as i64)),
            move |v| {
                if let Some(me) = w.upgrade() {
                    me.change(move || sound::set_volume(sound::SOURCE, v));
                }
            },
        )
        .named("input-volume");
        ivrow.ctl.append(&in_volume.root);
        ivrow.ctl.append(&in_pct);
        inp.add(&ivrow.root);
        let lrow = Row::new(None, Tone::Plain, "Input Level", "Speak to try it");
        parts::wide(&lrow);
        let level = Level::new(28);
        level.root.set_widget_name("input-level");
        lrow.ctl.append(&level.root);
        inp.add(&lrow.root);
        page.add(&inp.root);

        let root = page.root.clone();
        let ui = Rc::new(Ui {
            out_row,
            output,
            test,
            test_box,
            left,
            right,
            muted_note,
            mute,
            volume,
            pct,
            in_row,
            input,
            in_volume,
            in_pct,
            level,
            note,
            root: root.clone(),
        });
        *self.ui.borrow_mut() = Some(ui);
        // While it's on screen: the level, and what other apps change.
        let w = self.me.clone();
        root.connect_map(move |_| {
            if let Some(me) = w.upgrade() {
                me.listen(true);
                me.read();
            }
        });
        let w = self.me.clone();
        root.connect_unmap(move |_| {
            if let Some(me) = w.upgrade() {
                me.listen(false);
            }
        });
        let w = self.me.clone();
        glib::timeout_add_seconds_local(2, move || {
            let Some(me) = w.upgrade() else { return glib::ControlFlow::Break };
            if me.ui().is_some_and(|u| u.root.is_mapped()) && !me.testing.get() {
                me.read();
            }
            glib::ControlFlow::Continue
        });
        self.read();
        root.upcast()
    }

    fn ready(&self) -> bool {
        // Read once (or failed).
        !matches!(&*self.status.borrow(), Err(e) if e == "not read yet")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn its_words() {
        let d = |volume, muted| Device { id: 1, name: "Speakers".into(), default: true, volume, muted };
        assert_eq!(pct_of(&d(0.45, false)), "45%");
        assert_eq!(pct_of(&d(0.45, true)), "Muted");
        assert_eq!(mute_icon(&d(0.45, true)), "volume-x");
        assert_eq!(mute_icon(&d(0.2, false)), "volume-1");
        assert_eq!(mute_icon(&d(0.8, false)), "volume-2");
    }
}
