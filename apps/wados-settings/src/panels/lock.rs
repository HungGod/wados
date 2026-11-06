//! Settings ▸ Lock Screen & Privacy (the spike's `panels/LockPanel.svelte`):
//!
//! - **Screen Lock**: the automatic lock's time, locking on suspend, and
//!   notifications on the lock screen (the settings: the shell follows
//!   them at once); Lock Now (`wadsctl lock`).
//! - **Password**: the current one, the new one twice. The current one's
//!   checked with PAM, then `passwd` changes it (wad-desk's pam module:
//!   only root writes /etc/shadow). The spike's words for what's wrong.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use wad_desk::pam::{self, Refused};
use wad_desk::settings::{self, Settings};
use wad_gtk::form::{self, Choice, TextField, Toggle, opt};
use wad_gtk::kit::{self, Group, Row, Tone, Variant};

use gtk::prelude::*;

use crate::panel::{Ctx, Panel};
use crate::parts;

const CSS: &str = r#"
.lk-pw { padding: 16px; }
"#;

pub const MIN_LEN: usize = 8;

/// What's wrong with what's typed, before PAM's asked (the spike's words, in its order).
pub fn problem(current: &str, new: &str, confirm: &str) -> Option<&'static str> {
    if current.is_empty() {
        Some("Your current password, please")
    } else if new.chars().count() < MIN_LEN {
        Some("The new password needs at least 8 characters")
    } else if new != confirm {
        Some("The new passwords don’t match")
    } else if new == current {
        Some("That’s the password you have now")
    } else {
        None
    }
}

/// What a refusal says.
pub fn refused_words(r: &Refused) -> String {
    match r {
        Refused::Wrong => "That isn’t your current password".into(),
        Refused::Weak(why) => format!("Choose another: {why}"),
        Refused::Failed(why) => format!("The password wasn’t changed: {why}"),
    }
}

pub const CHANGED: &str = "Password changed. Use it next time you unlock.";

/// The PAM service the current password's checked with (the lock screen's).
fn service() -> String {
    std::env::var("WADOS_PAM_SERVICE").unwrap_or_else(|_| "wados-lock".into())
}

/// `wados-settings --change-password`: the current password and the new
/// one, a line each on stdin; the panel's checks and change; its words out.
pub fn change_from_stdin() -> gtk::glib::ExitCode {
    let mut lines = std::io::stdin().lines().map_while(Result::ok);
    let (current, new) = (lines.next().unwrap_or_default(), lines.next().unwrap_or_default());
    let said = match problem(&current, &new, &new) {
        Some(p) => Err(p.to_string()),
        None => {
            let login = gtk::glib::user_name().to_string_lossy().into_owned();
            pam::change(&service(), &login, &current, &new).map_err(|e| refused_words(&e))
        }
    };
    match said {
        Ok(()) => {
            println!("{CHANGED}");
            gtk::glib::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            gtk::glib::ExitCode::FAILURE
        }
    }
}

pub fn make(_: &Ctx) -> Rc<dyn Panel> {
    Rc::new_cyclic(|me| LockPanel { me: me.clone(), ui: RefCell::default() })
}

struct Ui {
    current: TextField,
    new: TextField,
    confirm: TextField,
    said: gtk::Label,
    change: gtk::Button,
    lock_note: gtk::Label,
}

pub struct LockPanel {
    me: Weak<LockPanel>,
    ui: RefCell<Option<Rc<Ui>>>,
}

fn save(f: impl FnOnce(&mut Settings)) {
    if let Err(e) = Settings::update(f) {
        tracing::warn!("settings: {e}");
    }
}

impl LockPanel {
    fn ui(&self) -> Option<Rc<Ui>> {
        self.ui.borrow().clone()
    }

    /// The foot's line: an error, that it worked, or the note.
    fn say(&self, what: Option<(&str, &str)>) {
        let Some(ui) = self.ui() else { return };
        for c in ["set-err", "set-ok", "set-note"] {
            ui.said.remove_css_class(c);
        }
        let (text, class) = what.unwrap_or(("At least 8 characters.", "set-note"));
        ui.said.set_label(text);
        ui.said.add_css_class(class);
    }

    fn typed(&self) {
        let Some(ui) = self.ui() else { return };
        let all = !ui.current.value().is_empty() && !ui.new.value().is_empty() && !ui.confirm.value().is_empty();
        ui.change.set_sensitive(all);
        self.say(None);
    }

    fn change(&self) {
        let Some(ui) = self.ui() else { return };
        let (current, new, confirm) = (ui.current.value(), ui.new.value(), ui.confirm.value());
        if let Some(p) = problem(&current, &new, &confirm) {
            self.say(Some((p, "set-err")));
            return;
        }
        ui.change.set_sensitive(false);
        self.say(Some(("Changing it…", "set-note")));
        let login = gtk::glib::user_name().to_string_lossy().into_owned();
        let w = self.me.clone();
        parts::spawn(
            move || pam::change(&service(), &login, &current, &new),
            move |r| {
                let Some(me) = w.upgrade() else { return };
                let Some(ui) = me.ui() else { return };
                match r {
                    Ok(()) => {
                        for f in [&ui.current, &ui.new, &ui.confirm] {
                            f.set_value("");
                        }
                        me.say(Some((CHANGED, "set-ok")));
                    }
                    Err(e) => {
                        ui.change.set_sensitive(true);
                        me.say(Some((&refused_words(&e), "set-err")));
                    }
                }
            },
        );
    }
}

impl Panel for LockPanel {
    fn build(&self, _: &Ctx) -> gtk::Widget {
        parts::install();
        if let Some(d) = gtk::gdk::Display::default() {
            wad_gtk::theme::add_css(&d, CSS);
        }
        let s = Settings::load();
        let page = kit::Page::new("Lock Screen & Privacy", false);

        let lock = Group::new("Screen Lock", "");
        let auto = Row::new(
            Some("timer"),
            Tone::Plain,
            "Automatic Screen Lock",
            "Locks the screen after it’s been idle this long",
        );
        let opts: Vec<form::Opt<u32>> =
            settings::minute_options(&[1, 5, 10, 30]).into_iter().map(|(l, n)| opt(n, &l)).collect();
        auto.ctl.append(
            &Choice::new(opts, s.lock.auto_lock_min, |v: &u32| {
                let v = *v;
                save(move |s| s.lock.auto_lock_min = v);
            })
            .named("autolock")
            .root,
        );
        lock.add(&auto.root);
        let sus = Row::new(Some("moon"), Tone::Plain, "Lock on Suspend", "Ask for the password when the machine wakes");
        sus.ctl.append(
            &Toggle::new(s.lock.on_suspend, |on| save(move |s| s.lock.on_suspend = on)).named("lock-on-suspend").root,
        );
        lock.add(&sus.root);
        let notes =
            Row::new(Some("bell"), Tone::Plain, "Lock Screen Notifications", "Show notifications on the lock screen");
        notes.ctl.append(
            &Toggle::new(s.lock.notifications, |on| save(move |s| s.lock.notifications = on))
                .named("lock-notifications")
                .root,
        );
        lock.add(&notes.root);
        let now = Row::new(Some("lock"), Tone::Plain, "Lock the Screen Now", "");
        now.ctl.append(&form::keys("Ctrl+Alt+L"));
        let b = kit::button("Lock Now", Variant::Secondary, Some("lock"), true);
        b.set_widget_name("lock-now");
        let w = self.me.clone();
        b.connect_clicked(move |_| {
            let w = w.clone();
            parts::spawn(
                || match std::process::Command::new("wadsctl").arg("lock").output() {
                    Ok(o) if o.status.success() => Ok(()),
                    Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
                    Err(e) => Err(format!("wadsctl: {e}")),
                },
                move |r| {
                    if let (Some(me), Err(e)) = (w.upgrade(), r)
                        && let Some(ui) = me.ui()
                    {
                        ui.lock_note.set_label(&format!("Couldn't lock: {e}"));
                        ui.lock_note.set_visible(true);
                    }
                },
            );
        });
        now.ctl.append(&b);
        lock.add(&now.root);
        let lock_note = parts::note("", "set-err");
        lock.root.append(&lock_note);
        page.add(&lock.root);

        let pw = Group::new("Password", "It unlocks the lock screen.");
        let form =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).css_classes(["lk-pw"]).build();
        let current = TextField::password("Current password", "").named("current");
        let new = TextField::password("New password", "").named("new");
        let confirm = TextField::password("Confirm the new password", "").named("confirm");
        for f in [&current, &new, &confirm] {
            form.append(&f.root);
        }
        let foot = gtk::Box::builder().spacing(12).margin_top(4).build();
        let said = kit::label("At least 8 characters.", &["set-note"]);
        said.set_hexpand(true);
        said.set_widget_name("password-said");
        foot.append(&said);
        let change = kit::button("Change Password", Variant::Primary, Some("key-round"), false);
        change.set_widget_name("change-password");
        change.set_sensitive(false);
        foot.append(&change);
        form.append(&foot);
        pw.add(&form);
        page.add(&pw.root);

        *self.ui.borrow_mut() = Some(Rc::new(Ui {
            current: current.clone(),
            new: new.clone(),
            confirm: confirm.clone(),
            said,
            change: change.clone(),
            lock_note,
        }));
        for f in [&current, &new, &confirm] {
            let w = self.me.clone();
            f.on_change(move |_| {
                if let Some(me) = w.upgrade() {
                    me.typed();
                }
            });
        }
        let w = self.me.clone();
        confirm.on_enter(move |_| {
            if let Some(me) = w.upgrade() {
                me.change();
            }
        });
        let w = self.me.clone();
        change.connect_clicked(move |_| {
            if let Some(me) = w.upgrade() {
                me.change();
            }
        });
        page.root.upcast()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spike_s_words_in_its_order() {
        assert_eq!(problem("", "", ""), Some("Your current password, please"));
        assert_eq!(problem("old", "short", "short"), Some("The new password needs at least 8 characters"));
        assert_eq!(problem("old", "long enough", "long enougj"), Some("The new passwords don’t match"));
        assert_eq!(problem("same one!", "same one!", "same one!"), Some("That’s the password you have now"));
        assert_eq!(problem("old", "long enough", "long enough"), None);
        assert_eq!(refused_words(&Refused::Wrong), "That isn’t your current password");
        assert_eq!(
            refused_words(&Refused::Weak("It is too simplistic.".into())),
            "Choose another: It is too simplistic."
        );
    }
}
