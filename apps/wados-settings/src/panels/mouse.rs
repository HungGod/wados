//! Settings ▸ Mouse & Touchpad (the spike's `panels/MousePanel.svelte`):
//! the primary button, each one's pointer speed, the scroll direction and
//! tap to click, and a test area that answers clicks, double-clicks and
//! scrolling as they arrive (after labwc's applied what's set here).
//!
//! What's set goes into the settings and the person's labwc rc.xml
//! (`<libinput>`: wad-desk's labwc module), and labwc reads it again.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::{glib, prelude::*};
use wad_desk::settings::{Hand, Mouse, Settings};
use wad_gtk::Icon;
use wad_gtk::form::{Segmented, Slider, Toggle, opt};
use wad_gtk::kit::{self, Group, Row, Tone};

use crate::panel::{Ctx, Panel};
use crate::parts;

const CSS: &str = r#"
.ms-tabs { margin-bottom: -8px; }
.ms-test { min-height: 210px; border-radius: 14px; border: 1px dashed var(--line-strong); background-color: var(--bg-2); }
.ms-target { min-width: 64px; min-height: 64px; border-radius: 50%; background-color: var(--surface-3); color: var(--muted); border: 2px solid var(--line-strong); }
.ms-target.lit { background-color: var(--accent); color: var(--accent-fg); border-color: var(--accent); }
.ms-said { font-weight: 600; }
.ms-legend { font-size: 12px; color: var(--faint); }
"#;

/// The test area's legend.
pub fn legend(m: &Mouse) -> String {
    format!(
        "{} button is primary · {} scrolling",
        if m.primary == Hand::Left { "Left" } else { "Right" },
        if m.natural { "natural" } else { "traditional" }
    )
}

/// What a button (as it arrives, labwc having swapped them for a left hand) is.
pub fn click_words(button: u32) -> &'static str {
    if button == 1 { "Primary click" } else { "Secondary click" }
}

/// What a scroll (as it arrives, natural scrolling applied) does.
pub fn scroll_words(dy: f64) -> &'static str {
    if dy > 0.0 { "The view scrolls down" } else { "The view scrolls up" }
}

pub fn make(_: &Ctx) -> Rc<dyn Panel> {
    Rc::new_cyclic(|me| MousePanel { me: me.clone(), ui: RefCell::default() })
}

struct Ui {
    radios: Vec<(bool, Row, gtk::Box)>,
    natural: Toggle,
    legend: gtk::Label,
    said: gtk::Label,
    target: gtk::Box,
    target_icon: Icon,
    note: gtk::Label,
}

pub struct MousePanel {
    me: Weak<MousePanel>,
    ui: RefCell<Option<Rc<Ui>>>,
}

impl MousePanel {
    fn ui(&self) -> Option<Rc<Ui>> {
        self.ui.borrow().clone()
    }

    fn set(&self, f: impl FnOnce(&mut Mouse) + 'static) {
        let w = self.me.clone();
        parts::input_changed(
            move |s| f(&mut s.mouse),
            move |r| {
                let Some(me) = w.upgrade() else { return };
                let Some(ui) = me.ui() else { return };
                match r {
                    Ok(()) => ui.note.set_visible(false),
                    Err(e) => {
                        ui.note.set_label(&format!("Saved; labwc hasn't taken it yet: {e}"));
                        ui.note.set_visible(true);
                    }
                }
                me.show();
            },
        );
    }

    fn show(&self) {
        let Some(ui) = self.ui() else { return };
        let m = Settings::load().mouse;
        for (natural, row, radio) in &ui.radios {
            let on = *natural == m.natural;
            parts::set_radio(radio, on);
            parts::set_active(&row.root, on);
        }
        ui.natural.set(m.natural);
        ui.legend.set_label(&legend(&m));
    }
}

impl Panel for MousePanel {
    fn build(&self, _: &Ctx) -> gtk::Widget {
        parts::install();
        if let Some(d) = gtk::gdk::Display::default() {
            wad_gtk::theme::add_css(&d, CSS);
        }
        let m = Settings::load().mouse;
        let page = kit::Page::new("Mouse & Touchpad", false);
        let note = parts::note("", "set-err");
        page.add(&note);

        let mouse = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(28).build();
        let touchpad = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(28).visible(false).build();
        let (mb, tb) = (mouse.clone(), touchpad.clone());
        let tabs = Segmented::new(
            [opt("mouse", "Mouse").icon("mouse"), opt("touchpad", "Touchpad").icon("mouse-pointer-2")],
            "mouse",
            false,
            move |v: &&str| {
                mb.set_visible(*v == "mouse");
                tb.set_visible(*v == "touchpad");
            },
        )
        .named("mouse-tabs");
        tabs.root.set_halign(gtk::Align::Center);
        tabs.root.add_css_class("ms-tabs");
        page.add(&tabs.root);

        let general = Group::new("General", "");
        let prow = Row::new(None, Tone::Plain, "Primary Button", "The order of the buttons on mice and touchpads");
        let w = self.me.clone();
        prow.ctl.append(
            &Segmented::new([opt(Hand::Left, "Left"), opt(Hand::Right, "Right")], m.primary, true, move |h: &Hand| {
                if let Some(me) = w.upgrade() {
                    let h = *h;
                    me.set(move |m| m.primary = h);
                }
            })
            .named("primary-button")
            .root,
        );
        general.add(&prow.root);
        page.add(&general.root);

        // The mouse's.
        let g = Group::new("Mouse", "");
        let srow = Row::new(Some("mouse"), Tone::Plain, "Pointer Speed", "");
        parts::wide(&srow);
        srow.ctl.append(&parts::end("Slow", false));
        let w = self.me.clone();
        srow.ctl.append(
            &Slider::new(
                0.0,
                1.0,
                0.01,
                m.speed,
                |_| {},
                move |v| {
                    if let Some(me) = w.upgrade() {
                        me.set(move |m| m.speed = v);
                    }
                },
            )
            .named("mouse-speed")
            .root,
        );
        srow.ctl.append(&parts::end("Fast", false));
        g.add(&srow.root);
        mouse.append(&g.root);
        let dir = Group::new("Scroll Direction", "");
        let mut radios = vec![];
        for (natural, title, sub) in [
            (false, "Traditional", "Scrolling moves the view"),
            (true, "Natural", "Scrolling moves the content, as on a touchscreen"),
        ] {
            let r = Row::new(None, Tone::Plain, title, sub).named(&format!("scroll-{title}"));
            let radio = parts::radio(m.natural == natural);
            r.ctl.append(&radio);
            parts::set_active(&r.root, m.natural == natural);
            let w = self.me.clone();
            parts::clickable(&r, move || {
                if let Some(me) = w.upgrade() {
                    me.set(move |m| m.natural = natural);
                }
            });
            dir.add(&r.root);
            radios.push((natural, r, radio));
        }
        mouse.append(&dir.root);
        page.add(&mouse);

        // The touchpad's.
        let g = Group::new("Touchpad", "");
        let srow = Row::new(Some("mouse-pointer-2"), Tone::Plain, "Pointer Speed", "");
        parts::wide(&srow);
        srow.ctl.append(&parts::end("Slow", false));
        let w = self.me.clone();
        srow.ctl.append(
            &Slider::new(
                0.0,
                1.0,
                0.01,
                m.touchpad_speed,
                |_| {},
                move |v| {
                    if let Some(me) = w.upgrade() {
                        me.set(move |m| m.touchpad_speed = v);
                    }
                },
            )
            .named("touchpad-speed")
            .root,
        );
        srow.ctl.append(&parts::end("Fast", false));
        g.add(&srow.root);
        let tap = Row::new(Some("hand"), Tone::Plain, "Tap to Click", "Tap the touchpad with one finger to click");
        let w = self.me.clone();
        tap.ctl.append(
            &Toggle::new(m.tap_to_click, move |on| {
                if let Some(me) = w.upgrade() {
                    me.set(move |m| m.tap_to_click = on);
                }
            })
            .named("tap-to-click")
            .root,
        );
        g.add(&tap.root);
        let nat = Row::new(None, Tone::Plain, "Natural Scrolling", "Scrolling moves the content, not the view");
        let w = self.me.clone();
        let natural = Toggle::new(m.natural, move |on| {
            if let Some(me) = w.upgrade() {
                me.set(move |m| m.natural = on);
            }
        })
        .named("natural-scrolling");
        nat.ctl.append(&natural.root);
        g.add(&nat.root);
        touchpad.append(&g.root);
        page.add(&touchpad);

        // The test area.
        let test =
            Group::new("Test Your Settings", "Try the buttons and the scrolling here; it follows what’s set above.");
        test.list.set_visible(false);
        let area = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .valign(gtk::Align::Fill)
            .css_classes(["ms-test"])
            .build();
        area.set_widget_name("mouse-test");
        area.set_cursor_from_name(Some("crosshair"));
        let inner = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .vexpand(true)
            .valign(gtk::Align::Center)
            .build();
        let target = gtk::Box::builder().css_classes(["ms-target"]).halign(gtk::Align::Center).build();
        let target_icon = Icon::new("mouse-pointer-2", 22);
        target_icon.set_hexpand(true);
        target.append(&target_icon);
        inner.append(&target);
        let said = gtk::Label::builder().label("Click, double-click or scroll here").css_classes(["ms-said"]).build();
        inner.append(&said);
        let legend_l = gtk::Label::builder().label(legend(&m)).css_classes(["ms-legend"]).build();
        inner.append(&legend_l);
        area.append(&inner);
        test.root.append(&area);
        page.add(&test.root);

        let click = gtk::GestureClick::builder().button(0).build();
        let w = self.me.clone();
        let lit_until = Rc::new(Cell::new(0i64));
        click.connect_pressed(move |g, n, _, _| {
            let Some(me) = w.upgrade() else { return };
            let Some(ui) = me.ui() else { return };
            if n == 2 && g.current_button() == 1 {
                ui.said.set_label("Double-click");
                ui.target.add_css_class("lit");
                ui.target_icon.set("check");
                lit_until.set(glib::monotonic_time() + 700_000);
                let (w, until) = (me.me.clone(), lit_until.clone());
                glib::timeout_add_local_once(Duration::from_millis(720), move || {
                    if let Some(me) = w.upgrade()
                        && let Some(ui) = me.ui()
                        && glib::monotonic_time() >= until.get()
                    {
                        ui.target.remove_css_class("lit");
                        ui.target_icon.set("mouse-pointer-2");
                    }
                });
            } else {
                ui.said.set_label(click_words(g.current_button()));
            }
        });
        area.add_controller(click);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let w = self.me.clone();
        scroll.connect_scroll(move |_, _, dy| {
            if let Some(me) = w.upgrade()
                && let Some(ui) = me.ui()
                && dy != 0.0
            {
                ui.said.set_label(scroll_words(dy));
            }
            glib::Propagation::Stop
        });
        area.add_controller(scroll);

        *self.ui.borrow_mut() =
            Some(Rc::new(Ui { radios, natural, legend: legend_l, said, target, target_icon, note }));
        page.root.upcast()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_area_s_words() {
        assert_eq!(legend(&Mouse::default()), "Left button is primary · traditional scrolling");
        let m = Mouse { primary: Hand::Right, natural: true, ..Mouse::default() };
        assert_eq!(legend(&m), "Right button is primary · natural scrolling");
        assert_eq!(click_words(1), "Primary click");
        assert_eq!(click_words(3), "Secondary click");
        assert_eq!(scroll_words(1.0), "The view scrolls down");
        assert_eq!(scroll_words(-1.0), "The view scrolls up");
    }
}
