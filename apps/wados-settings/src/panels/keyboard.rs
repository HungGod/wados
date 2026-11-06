//! Settings ▸ Keyboard (the spike's `panels/KeyboardPanel.svelte`): the
//! layout, how held keys repeat (and a box to try it in: the repeat there
//! is labwc's own, as set here), and the shortcuts (the machine's rc.xml's).
//!
//! The layout goes into labwc's `environment` (XKB_DEFAULT_LAYOUT and
//! _VARIANT), the repeat into the person's rc.xml (`<keyboard>`), and labwc
//! reads them again (wad-desk's labwc module).

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use wad_desk::labwc::{LAYOUTS, layout_label};
use wad_desk::settings::Settings;
use wad_gtk::form::{self, Choice, Slider, opt};
use wad_gtk::kit::{self, Group, Row, Tone};

use crate::panel::{Ctx, Panel};
use crate::parts;

/// The shortcuts WadSpaces adds (the machine's rc.xml's keybinds).
pub const WADSPACES_KEYS: [(&str, &str); 5] = [
    ("Switch between running wadspaces", "Super+Tab"),
    ("Search apps and their variants", "Alt+Space"),
    ("Lock the screen", "Ctrl+Alt+L"),
    ("Put a full-screen wadspace into a window", "Ctrl+Alt+↓"),
    ("Come back from server mode", "Ctrl+Alt+Delete"),
];

/// The desktop's own (desktop.rs's).
pub const DESKTOP_KEYS: [(&str, &str); 8] = [
    ("Select everything on the desktop", "Ctrl+A"),
    ("Copy", "Ctrl+C"),
    ("Cut", "Ctrl+X"),
    ("Paste", "Ctrl+V"),
    ("Rename", "F2"),
    ("Move to the trash", "Delete"),
    ("Open", "Enter"),
    ("Close a menu or a popup", "Esc"),
];

pub fn delay_words(ms: u32) -> String {
    format!("{ms} ms before it starts")
}

pub fn rate_words(n: u32) -> String {
    format!("{n} characters a second")
}

pub fn make(_: &Ctx) -> Rc<dyn Panel> {
    Rc::new_cyclic(|me| KeyboardPanel { me: me.clone(), ui: RefCell::default() })
}

struct Ui {
    layout_row: Row,
    note: gtk::Label,
}

pub struct KeyboardPanel {
    me: Weak<KeyboardPanel>,
    ui: RefCell<Option<Rc<Ui>>>,
}

impl KeyboardPanel {
    fn set(&self, f: impl FnOnce(&mut wad_desk::settings::Keyboard) + 'static) {
        let w = self.me.clone();
        parts::input_changed(
            move |s| f(&mut s.keyboard),
            move |r| {
                let Some(me) = w.upgrade() else { return };
                let Some(ui) = me.ui.borrow().clone() else { return };
                match r {
                    Ok(()) => ui.note.set_visible(false),
                    Err(e) => {
                        ui.note.set_label(&format!("Saved; labwc hasn't taken it yet: {e}"));
                        ui.note.set_visible(true);
                    }
                }
                let k = Settings::load().keyboard;
                if let Some(t) = parts::title_of(&ui.layout_row) {
                    t.set_label(&layout_label(&k.layout));
                }
            },
        );
    }
}

impl Panel for KeyboardPanel {
    fn build(&self, _: &Ctx) -> gtk::Widget {
        parts::install();
        let k = Settings::load().keyboard;
        let page = kit::Page::new("Keyboard", false);
        let note = parts::note("", "set-err");
        page.add(&note);

        let src = Group::new("Input Sources", "The keyboard layout you type with.");
        let layout_row = Row::new(Some("keyboard"), Tone::Accent, &layout_label(&k.layout), "Keyboard layout");
        let mut opts: Vec<form::Opt<String>> = LAYOUTS.iter().map(|(l, x)| opt(x.to_string(), l)).collect();
        if !LAYOUTS.iter().any(|(_, x)| *x == k.layout) {
            opts.push(opt(k.layout.clone(), &k.layout));
        }
        let w = self.me.clone();
        layout_row.ctl.append(
            &Choice::new(opts, k.layout.clone(), move |l: &String| {
                if let Some(me) = w.upgrade() {
                    let l = l.clone();
                    me.set(move |k| k.layout = l);
                }
            })
            .named("layout")
            .root,
        );
        src.add(&layout_row.root);
        page.add(&src.root);

        let rep = Group::new("Repeat Keys", "A key you hold down types again and again.");
        let delay_row = Row::new(None, Tone::Plain, "Delay", &delay_words(k.repeat_delay));
        parts::wide(&delay_row);
        delay_row.ctl.append(&parts::end("Short", false));
        let (w, sub) = (self.me.clone(), parts::subtitle_of(&delay_row));
        delay_row.ctl.append(
            &Slider::new(
                150.0,
                1000.0,
                10.0,
                k.repeat_delay as f64,
                move |v| {
                    if let Some(s) = &sub {
                        s.set_label(&delay_words(v as u32));
                    }
                },
                move |v| {
                    if let Some(me) = w.upgrade() {
                        me.set(move |k| k.repeat_delay = v as u32);
                    }
                },
            )
            .named("repeat-delay")
            .root,
        );
        delay_row.ctl.append(&parts::end("Long", false));
        rep.add(&delay_row.root);
        let rate_row = Row::new(None, Tone::Plain, "Speed", &rate_words(k.repeat_rate));
        parts::wide(&rate_row);
        rate_row.ctl.append(&parts::end("Slow", false));
        let (w, sub) = (self.me.clone(), parts::subtitle_of(&rate_row));
        rate_row.ctl.append(
            &Slider::new(
                5.0,
                60.0,
                1.0,
                k.repeat_rate as f64,
                move |v| {
                    if let Some(s) = &sub {
                        s.set_label(&rate_words(v as u32));
                    }
                },
                move |v| {
                    if let Some(me) = w.upgrade() {
                        me.set(move |k| k.repeat_rate = v as u32);
                    }
                },
            )
            .named("repeat-rate")
            .root,
        );
        rate_row.ctl.append(&parts::end("Fast", false));
        rep.add(&rate_row.root);
        let (try_row, try_ctl) = parts::stacked(None, Tone::Plain, "Try It", "Hold a key down in the box");
        let field = form::TextField::new("", "Hold a key…").named("try-repeat");
        field.root.set_hexpand(true);
        field.text.update_property(&[gtk::accessible::Property::Label("Try the repeat")]);
        try_ctl.append(&field.root);
        rep.add(&try_row);
        page.add(&rep.root);

        let sc = Group::new(
            "Keyboard Shortcuts",
            "Super+Tab goes between wadspaces only, never the browser, the terminal or the machine’s other apps. The rest use Ctrl+Alt.",
        );
        for (what, combo) in WADSPACES_KEYS {
            let r = Row::new(None, Tone::Plain, what, "");
            r.ctl.append(&form::keys(combo));
            sc.add(&r.root);
        }
        page.add(&sc.root);
        let desk = Group::new("On the Desktop", "");
        for (what, combo) in DESKTOP_KEYS {
            let r = Row::new(None, Tone::Plain, what, "");
            r.ctl.append(&form::keys(combo));
            desk.add(&r.root);
        }
        page.add(&desk.root);

        *self.ui.borrow_mut() = Some(Rc::new(Ui { layout_row, note }));
        page.root.upcast()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn its_words() {
        assert_eq!(delay_words(500), "500 ms before it starts");
        assert_eq!(rate_words(30), "30 characters a second");
        assert_eq!(layout_label("gb"), "English (UK)");
    }
}
