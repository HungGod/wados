//! The power key's menu (labwc's XF86PowerOff runs `wadsctl power`): the
//! start menu's power items (`shell/bar/Launcher.svelte`'s footer: Lock,
//! Server mode, Restart, Power off, worded as there), on their own over
//! everything (OVERLAY), so it shows over a full-screen wadspace too, where
//! the bar can't. ←/→ and Enter; Esc, a click outside it or the power key
//! again close it. Like the start menu's, Restart and Power off don't ask
//! again.

use std::rc::Rc;

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{KeyboardMode, Layer};
use wad_gtk::Icon;

use super::{Power, items};
use crate::shell::Shell;
use crate::surface::{self, Anchor, Spec};

const CSS: &str = r#"
window.wados-power { background-color: rgb(5 3 12 / 0.5); }
.pm-card {
  padding: 16px;
  border-radius: 22px;
  border: 1px solid var(--line-strong);
  background-color: rgb(14 9 25 / 0.97);
  box-shadow: 0 30px 80px -20px rgb(0 0 0 / 0.85);
}
.pm-who { padding: 0 6px 12px; margin-bottom: 10px; border-bottom: 1px solid var(--line); }
.pm-avatar { min-width: 30px; min-height: 30px; border-radius: 99px; background-color: var(--accent-2-soft); color: var(--accent-2); font-weight: 700; font-size: 13px; }
.pm-name { font-size: 14px; font-weight: 600; }
button.pm-b {
  background: none; border: none; box-shadow: none; outline: none; text-shadow: none;
  color: var(--fg); min-width: 104px; min-height: 0; padding: 12px 6px 10px; border-radius: 16px;
}
button.pm-b:hover { background-color: rgb(255 255 255 / 0.06); }
button.pm-b:focus-visible { background-color: rgb(255 255 255 / 0.1); box-shadow: inset 0 0 0 2px rgb(255 255 255 / 0.85); }
button.pm-b:disabled { color: var(--faint); }
.pm-ic { min-width: 46px; min-height: 46px; border-radius: 14px; background-color: var(--surface-3); }
button.pm-b.pm-danger .pm-ic { background-color: var(--danger-soft); color: var(--danger); }
button.pm-b:disabled .pm-ic { background-color: var(--surface-2); color: var(--faint); }
.pm-label { font-size: 12.5px; font-weight: 600; }
.pm-foot { padding-top: 10px; margin-top: 10px; border-top: 1px solid var(--line); font-size: 11.5px; color: var(--faint); }
"#;

pub struct Menu {
    shell: Rc<Shell>,
    win: gtk::ApplicationWindow,
    buttons: Vec<(Power, gtk::Button)>,
}

impl Menu {
    /// `on`: an item was chosen (the menu has closed).
    pub fn new(shell: &Rc<Shell>, machine: bool, on: impl Fn(Power) + 'static) -> Rc<Self> {
        if let Some(d) = gdk::Display::default() {
            wad_gtk::theme::add_css(&d, CSS);
        }
        let win = surface::make(
            &shell.app,
            &shell.primary,
            &Spec {
                namespace: "wados-power",
                layer: Layer::Overlay,
                anchor: Anchor::Fill,
                exclusive: -1,
                keyboard: KeyboardMode::Exclusive,
                height: 0,
            },
        );
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["pm-card"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        // Who (as the start menu's footer has it).
        let who = gtk::Box::builder().spacing(10).css_classes(["pm-who"]).build();
        let name = glib::real_name().to_string_lossy().to_string();
        let name =
            if name.is_empty() || name == "Unknown" { glib::user_name().to_string_lossy().to_string() } else { name };
        who.append(
            &gtk::Label::builder()
                .label(name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default())
                .css_classes(["pm-avatar"])
                .build(),
        );
        who.append(&gtk::Label::builder().label(&name).css_classes(["pm-name"]).build());
        card.append(&who);

        let row = gtk::Box::builder().spacing(4).homogeneous(true).build();
        let on: Rc<dyn Fn(Power)> = Rc::new(on);
        let mut buttons = vec![];
        for it in items(machine) {
            let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
            let ic = gtk::Box::builder().css_classes(["pm-ic"]).homogeneous(true).halign(gtk::Align::Center).build();
            ic.append(&Icon::new(it.icon, 20));
            col.append(&ic);
            col.append(&gtk::Label::builder().label(it.label).css_classes(["pm-label"]).build());
            let b = gtk::Button::builder().child(&col).css_classes(["pm-b"]).tooltip_text(&it.tip).build();
            if it.power == Power::Off {
                b.add_css_class("pm-danger");
            }
            if it.on {
                row.append(&b);
            } else {
                b.set_sensitive(false);
                // Its tooltip still says why.
                let holder = gtk::Box::builder().tooltip_text(&it.tip).build();
                b.set_hexpand(true);
                holder.append(&b);
                row.append(&holder);
            }
            buttons.push((it.power, b));
        }
        card.append(&row);
        let foot = gtk::Box::builder().spacing(16).css_classes(["pm-foot"]).build();
        for t in ["←→ to move", "Enter chooses", "Esc closes"] {
            foot.append(&gtk::Label::new(Some(t)));
        }
        card.append(&foot);
        win.set_child(Some(&card));
        let me = Rc::new(Self { shell: shell.clone(), win, buttons });
        for (p, b) in me.buttons.iter().filter(|(_, b)| b.is_sensitive()) {
            let (w, on, p) = (Rc::downgrade(&me), on.clone(), *p);
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.close();
                }
                on(p);
            });
        }

        // A click outside it closes it.
        let click = gtk::GestureClick::new();
        let (w, c) = (Rc::downgrade(&me), card.downgrade());
        click.connect_pressed(move |g, _, x, y| {
            let (Some(m), Some(c), Some(win)) = (w.upgrade(), c.upgrade(), g.widget()) else { return };
            let inside = c
                .compute_bounds(&win)
                .is_some_and(|r| r.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)));
            if !inside {
                m.close();
            }
        });
        me.win.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(&me);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape
                && let Some(m) = w.upgrade()
            {
                m.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        me.win.add_controller(keys);
        me
    }

    pub fn is_open(&self) -> bool {
        self.win.is_visible()
    }

    /// Open, the keys on Power off (Lock when it's off here).
    pub fn open(&self) {
        let _ = self.shell.ctl(&["bar", "flyout", "none"]);
        self.win.present();
        let first = [Power::Off, Power::Lock].into_iter().find_map(|p| self.button(p).filter(|b| b.is_sensitive()));
        if let Some(b) = first {
            b.grab_focus();
        }
        self.win.set_focus_visible(true);
    }

    pub fn close(&self) {
        if self.is_open() {
            self.win.set_visible(false);
            self.shell.keys_back();
        }
    }

    /// The power key: open it, or close it.
    pub fn toggle(&self) {
        if self.is_open() { self.close() } else { self.open() }
    }

    fn button(&self, p: Power) -> Option<gtk::Button> {
        self.buttons.iter().find(|(q, _)| *q == p).map(|(_, b)| b.clone())
    }

    /// For `wadsctl power state`.
    pub fn state(&self) -> String {
        let items: Vec<String> = self
            .buttons
            .iter()
            .map(|(p, b)| format!("{}={}", p.word(), if b.is_sensitive() { "on" } else { "off" }))
            .collect();
        if !self.is_open() {
            return format!("ok closed {}", items.join(" "));
        }
        let focus = gtk::prelude::GtkWindowExt::focus(&self.win);
        let at = self
            .buttons
            .iter()
            .find(|(_, b)| focus.as_ref().is_some_and(|f| f == b.upcast_ref::<gtk::Widget>() || f.is_ancestor(b)))
            .map(|(p, _)| p.word())
            .unwrap_or("none");
        format!("ok open keys={at} {}", items.join(" "))
    }
}
