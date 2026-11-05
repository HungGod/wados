//! The shell's dialogs (`shell/Dialogs.svelte`, in `ui/Dialog.svelte`'s
//! look): a card in the middle of the primary display over a dimmed screen,
//! its tinted badge, title and ×, a body, its buttons at the end. Over
//! everything (a full-screen wadspace too), holding the keys while it's up:
//! Esc closes it, Enter is its main button.
//!
//! What it shows, first that is:
//!
//! - a yes-or-no the shell asked ([`confirm`]: Focus in another machine's
//!   wadspace, "Copy it here");
//! - a sheet of another module's ([`show_sheet`]: the stream sheet, the
//!   share sheet), its card drawn and kept by it, in this one's [`frame`];
//! - a device asking to be trusted (the tailnet's asks, newest first), on
//!   its own as the HUD's did: "{device} wants to manage this machine",
//!   with Later (put off for this session: wadd's), Deny and Trust it. Its
//!   × (and Esc) only closes it here: the tray and Settings still ask.
//!
//! `wadsctl dialog` says what's up; `dialog later|deny|trust|ok|cancel|close`
//! presses its buttons (a sheet's, by the names it gives them).

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{KeyboardMode, Layer};
use wad_gtk::Icon;
use wad_gtk::kit::{self, Variant};
use wad_proto::{DeviceKind, Level, Tailnet, TailnetAsk, TailnetState};

use crate::shell::Shell;
use crate::store::Change;
use crate::surface::{self, Anchor, Spec};

/// Its card's width.
const WIDTH: i32 = 440;

/// A yes-or-no (the spike's `confirm`, in its warning tint): `ok` runs
/// `run`; Cancel (or ×, Esc) doesn't.
#[derive(Clone)]
pub struct Confirm {
    pub title: String,
    pub icon: &'static str,
    pub body: String,
    pub ok: String,
    pub run: Rc<dyn Fn()>,
}

/// A sheet of another module's over everything (the stream sheet, the
/// share sheet): its card, which it draws and keeps up to date itself.
#[derive(Clone)]
pub struct Sheet {
    pub name: &'static str,
    pub card: gtk::Widget,
    /// Its ×, Esc or the dim: it's gone (told after it's taken down).
    pub closed: Rc<dyn Fn()>,
    /// Enter: its main button.
    pub enter: Rc<dyn Fn()>,
    /// Its button `what` (`wadsctl dialog <what>`), pressed if it has it.
    pub press: Rc<dyn Fn(&str) -> bool>,
    /// What it shows, for `wadsctl dialog`.
    pub state: Rc<dyn Fn() -> String>,
}

/// A dialog card's tint (`ui/Dialog.svelte`'s tone): its badge's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Accent,
    Magenta,
    Warn,
}

impl Tone {
    fn class(self) -> &'static str {
        match self {
            Tone::Accent => "accent",
            Tone::Magenta => "magenta",
            Tone::Warn => "warn",
        }
    }
}

/// A dialog card's pieces: its title (to change), badge, body and footer.
pub struct Frame {
    pub card: gtk::Box,
    pub badge: gtk::Box,
    pub title: gtk::Label,
    pub body: gtk::Box,
    pub foot: gtk::Box,
}

impl Frame {
    /// Its tint now.
    pub fn set_tone(&self, tone: Tone) {
        for t in [Tone::Accent, Tone::Magenta, Tone::Warn] {
            self.badge.remove_css_class(t.class());
        }
        self.badge.add_css_class(tone.class());
    }
}

/// A dialog's card (`ui/Dialog.svelte`): its tinted badge, title and ×
/// (`close`), a body, its buttons at the end; `width` wide.
pub fn frame(title: &str, icon: &'static str, tone: Tone, width: i32, close: impl Fn() + 'static) -> Frame {
    let card = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["ov-dialog"]).build();
    card.set_size_request(width, -1);
    let head = gtk::Box::builder().spacing(12).css_classes(["ov-dialog-head"]).build();
    let badge = gtk::Box::builder()
        .css_classes(["ov-dialog-badge", tone.class()])
        .valign(gtk::Align::Center)
        .hexpand(false)
        .build();
    badge.set_size_request(36, 36);
    let badge_icon = Icon::new(icon, 18);
    badge_icon.set_hexpand(true);
    badge_icon.set_halign(gtk::Align::Center);
    badge_icon.set_valign(gtk::Align::Center);
    badge.append(&badge_icon);
    head.append(&badge);
    let t = kit::label(title, &["ov-dialog-title"]);
    t.set_hexpand(true);
    t.set_valign(gtk::Align::Center);
    head.append(&t);
    let x = gtk::Button::builder()
        .child(&Icon::new("x", 16))
        .css_classes(["ov-dialog-x"])
        .valign(gtk::Align::Center)
        .tooltip_text("Close")
        .build();
    x.connect_clicked(move |_| close());
    head.append(&x);
    card.append(&head);
    let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["ov-dialog-body"]).build();
    card.append(&body);
    let foot = gtk::Box::builder().spacing(8).halign(gtk::Align::End).css_classes(["ov-dialog-foot"]).build();
    card.append(&foot);
    Frame { card, badge, title: t, body, foot }
}

/// What an ask's dialog says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskText {
    pub title: String,
    pub body: String,
    pub icon: &'static str,
}

/// A device's glyph.
pub fn device_icon(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Phone | DeviceKind::Other => "smartphone",
        DeviceKind::Tablet => "tablet",
        DeviceKind::Laptop => "laptop",
        DeviceKind::Desktop => "monitor",
        DeviceKind::Server => "server",
    }
}

/// The trust ask's words (Dialogs.svelte's).
pub fn ask_text(a: &TailnetAsk) -> AskText {
    AskText {
        title: format!("{} wants to manage this machine", a.device),
        body: format!(
            "It’s on your tailnet ({}, {}). Trusted, it can open, stream, build and edit this machine’s wadspaces, and restart it to update.",
            kit::os_name(&a.os),
            a.ip
        ),
        icon: device_icon(a.kind),
    }
}

/// The ask to show: the newest not closed here, while the tailnet runs.
pub fn ask_to_show<'a>(t: Option<&'a Tailnet>, closed: &[String]) -> Option<&'a TailnetAsk> {
    let t = t.filter(|t| t.state == TailnetState::Running)?;
    t.asks.iter().rev().find(|a| !closed.contains(&a.id))
}

#[derive(Clone, PartialEq)]
enum Up {
    Confirm,
    Sheet(&'static str),
    Ask(String),
}

#[derive(Default)]
struct St {
    confirm: Option<Confirm>,
    sheet: Option<Sheet>,
    /// Asks closed with × this run (still asking in the tray and Settings).
    closed: Vec<String>,
    up: Option<Up>,
    /// An answer on its way to wadd.
    busy: bool,
    /// Its buttons, by what they do (for wadsctl).
    buttons: Vec<(&'static str, gtk::Button)>,
}

pub struct Dialogs {
    shell: Rc<Shell>,
    me: Weak<Dialogs>,
    win: gtk::ApplicationWindow,
    holder: gtk::Box,
    st: RefCell<St>,
}

thread_local! {
    static DIALOGS: RefCell<Weak<Dialogs>> = RefCell::default();
}

/// Asks a yes-or-no over everything (an ask waits behind it).
pub fn confirm(c: Confirm) {
    if let Some(d) = DIALOGS.with(|d| d.borrow().upgrade()) {
        d.st.borrow_mut().confirm = Some(c);
        d.draw();
    }
}

/// Shows a sheet over everything, in place of the one up (which is told
/// it's gone, unless it's the same sheet showing itself again); a yes-or-no
/// shows over it, an ask waits behind it. The surface stays up between the
/// two (taking it down and up again in one turn can leave it down).
pub fn show_sheet(s: Sheet) {
    if let Some(d) = DIALOGS.with(|d| d.borrow().upgrade()) {
        let name = s.name;
        let old = d.st.borrow_mut().sheet.replace(s);
        if let Some(old) = old {
            // Its card's taken down, so the new one's drawn.
            d.st.borrow_mut().up = None;
            if old.name != name {
                (old.closed)();
            }
        }
        d.draw();
    }
}

/// Takes the sheet `name` down, if it's the one up (without telling it).
pub fn hide_sheet(name: &str) {
    if let Some(d) = DIALOGS.with(|d| d.borrow().upgrade()) {
        let up = d.st.borrow().sheet.as_ref().is_some_and(|s| s.name == name);
        if up {
            d.st.borrow_mut().sheet = None;
            d.draw();
        }
    }
}

impl Dialogs {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let win = surface::make(
            &shell.app,
            &shell.primary,
            &Spec {
                namespace: "wados-dialog",
                layer: Layer::Overlay,
                anchor: Anchor::Fill,
                exclusive: -1,
                keyboard: KeyboardMode::Exclusive,
                height: 0,
            },
        );
        let scrim = gtk::Box::builder().css_classes(["ov-scrim"]).hexpand(true).vexpand(true).build();
        let holder = gtk::Box::builder().halign(gtk::Align::Center).valign(gtk::Align::Center).hexpand(true).build();
        scrim.append(&holder);
        win.set_child(Some(&scrim));
        let me =
            Rc::new_cyclic(|me| Self { shell: shell.clone(), me: me.clone(), win, holder, st: RefCell::default() });
        DIALOGS.with(|d| *d.borrow_mut() = Rc::downgrade(&me));

        // A press on the dim (not the card) closes it.
        let click = gtk::GestureClick::new();
        let w = me.me.clone();
        click.connect_pressed(move |g, _, x, y| {
            let Some(m) = w.upgrade() else { return };
            let on_card = m
                .holder
                .first_child()
                .and_then(|c| c.compute_bounds(&m.win))
                .is_some_and(|r| r.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)));
            if !on_card {
                g.set_state(gtk::EventSequenceState::Claimed);
                m.close();
            }
        });
        scrim.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        let w = me.me.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(m) = w.upgrade() else { return glib::Propagation::Proceed };
            match key {
                gdk::Key::Escape => {
                    m.close();
                    glib::Propagation::Stop
                }
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    let up = m.st.borrow().up.clone();
                    match up {
                        Some(Up::Sheet(_)) => {
                            let enter = m.st.borrow().sheet.as_ref().map(|s| s.enter.clone());
                            if let Some(e) = enter {
                                e();
                            }
                        }
                        Some(Up::Confirm) => {
                            m.press("ok");
                        }
                        _ => {
                            m.press("trust");
                        }
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        me.win.add_controller(keys);

        let w = me.me.clone();
        shell.store.watch(move |c| {
            if let Some(m) = w.upgrade()
                && matches!(c, Change::Tailnet | Change::Connected(_))
            {
                let w = m.me.clone();
                glib::idle_add_local_once(move || {
                    if let Some(m) = w.upgrade() {
                        m.st.borrow_mut().busy = false;
                        m.draw();
                    }
                });
            }
        });
        me
    }

    /// What it shows now (`wadsctl dialog`).
    pub fn state(&self) -> String {
        let st = self.st.borrow();
        let up = match &st.up {
            None => "none".to_string(),
            Some(Up::Confirm) => {
                format!("confirm {:?}", st.confirm.as_ref().map(|c| c.title.clone()).unwrap_or_default())
            }
            // A sheet lists its own buttons (`(off)` after one that can't be pressed).
            Some(Up::Sheet(name)) => {
                let state = st.sheet.as_ref().map(|s| s.state.clone());
                return format!("ok sheet {name} {}", state.map(|f| f()).unwrap_or_default());
            }
            Some(Up::Ask(id)) => {
                let t = self.shell.store.tailnet();
                let title = t
                    .as_ref()
                    .and_then(|t| t.asks.iter().find(|a| &a.id == id))
                    .map(|a| ask_text(a).title)
                    .unwrap_or_default();
                format!("ask {id} {title:?}")
            }
        };
        let buttons: Vec<&str> = st.buttons.iter().map(|(k, _)| *k).collect();
        format!("ok {up} buttons={}", buttons.join(","))
    }

    /// Its button `what` pressed (wadsctl's, Enter's).
    pub fn press(&self, what: &str) -> bool {
        let sheet = {
            let st = self.st.borrow();
            matches!(st.up, Some(Up::Sheet(_))).then(|| st.sheet.as_ref().map(|s| s.press.clone())).flatten()
        };
        if let Some(press) = sheet {
            if what == "close" {
                self.close();
                return true;
            }
            return press(what);
        }
        let b = self.st.borrow().buttons.iter().find(|(k, _)| *k == what).map(|(_, b)| b.clone());
        match b {
            Some(b) if b.is_sensitive() => {
                b.emit_clicked();
                true
            }
            _ if what == "close" => {
                self.close();
                true
            }
            _ => false,
        }
    }

    /// × (or Esc, or the dim): a yes-or-no is a no; a sheet is told it's
    /// gone; an ask is closed here.
    pub fn close(&self) {
        let mut gone = None;
        {
            let mut st = self.st.borrow_mut();
            match st.up.clone() {
                Some(Up::Confirm) => st.confirm = None,
                Some(Up::Sheet(_)) => gone = st.sheet.take(),
                Some(Up::Ask(id)) => st.closed.push(id),
                None => return,
            }
        }
        self.draw();
        if let Some(s) = gone {
            (s.closed)();
        }
    }

    fn draw(&self) {
        let store = &self.shell.store;
        let tailnet = store.tailnet();
        let (up, confirm, sheet, ask) = {
            let st = self.st.borrow();
            let ask = if store.connected() { ask_to_show(tailnet.as_ref(), &st.closed).cloned() } else { None };
            let up = if st.confirm.is_some() {
                Some(Up::Confirm)
            } else if let Some(s) = &st.sheet {
                Some(Up::Sheet(s.name))
            } else {
                ask.as_ref().map(|a| Up::Ask(a.id.clone()))
            };
            (up, st.confirm.clone(), st.sheet.clone(), ask)
        };
        let same = self.st.borrow().up == up && self.win.is_visible();
        let busy = self.st.borrow().busy;
        if !same {
            while let Some(c) = self.holder.first_child() {
                self.holder.remove(&c);
            }
            let mut buttons = vec![];
            match (&up, confirm, ask) {
                (Some(Up::Confirm), Some(c), _) => self.holder.append(&self.confirm_card(&c, &mut buttons)),
                (Some(Up::Sheet(_)), _, _) => {
                    if let Some(s) = &sheet {
                        if let Some(p) = s.card.parent().and_downcast::<gtk::Box>() {
                            p.remove(&s.card);
                        }
                        self.holder.append(&s.card);
                    }
                }
                (Some(Up::Ask(_)), _, Some(a)) => self.holder.append(&self.ask_card(&a, &mut buttons)),
                _ => {}
            }
            let mut st = self.st.borrow_mut();
            st.up = up.clone();
            st.buttons = buttons;
        }
        for (_, b) in self.st.borrow().buttons.iter() {
            b.set_sensitive(!busy);
        }
        if up.is_some() {
            if !self.win.is_visible() {
                self.win.present();
            }
        } else if self.win.is_visible() {
            self.win.set_visible(false);
            self.shell.keys_back();
        }
    }

    /// A card: its badge (in the warning tint, as the spike's asks and confirms), title, ×, body and footer.
    fn frame(&self, title: &str, icon: &'static str, body: &str) -> (gtk::Box, gtk::Box) {
        let w = self.me.clone();
        let f = frame(title, icon, Tone::Warn, WIDTH, move || {
            if let Some(m) = w.upgrade() {
                m.close();
            }
        });
        let b = kit::label(body, &[]);
        b.set_max_width_chars(48);
        f.body.append(&b);
        (f.card, f.foot)
    }

    fn confirm_card(&self, c: &Confirm, buttons: &mut Vec<(&'static str, gtk::Button)>) -> gtk::Box {
        let (card, foot) = self.frame(&c.title, c.icon, &c.body);
        let cancel = kit::button("Cancel", Variant::Ghost, None, false);
        let w = self.me.clone();
        cancel.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.close();
            }
        });
        let ok = kit::button(&c.ok, Variant::Primary, None, false);
        let (w, run) = (self.me.clone(), c.run.clone());
        ok.connect_clicked(move |_| {
            // Taken before closing: closing drops it.
            if let Some(m) = w.upgrade() {
                m.st.borrow_mut().confirm = None;
                m.draw();
            }
            run();
        });
        foot.append(&cancel);
        foot.append(&ok);
        buttons.extend([("cancel", cancel), ("ok", ok)]);
        card
    }

    fn ask_card(&self, a: &TailnetAsk, buttons: &mut Vec<(&'static str, gtk::Button)>) -> gtk::Box {
        let text = ask_text(a);
        let (card, foot) = self.frame(&text.title, text.icon, &text.body);
        let later = kit::button("Later", Variant::Ghost, None, false);
        let deny = kit::button("Deny", Variant::Danger, None, false);
        let trust = kit::button("Trust it", Variant::Primary, Some("shield-check"), false);
        for (what, b) in [("later", &later), ("deny", &deny), ("trust", &trust)] {
            let (w, id) = (self.me.clone(), a.id.clone());
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.answer(what, &id);
                }
            });
            foot.append(b);
        }
        buttons.extend([("later", later), ("deny", deny), ("trust", trust)]);
        card
    }

    /// An ask answered: to wadd (the tailnet's event takes it away).
    fn answer(&self, what: &'static str, id: &str) {
        self.st.borrow_mut().busy = true;
        self.draw();
        let (w, id) = (self.me.clone(), id.to_string());
        self.shell.link.call(
            move |wadd| async move {
                match what {
                    "trust" => wadd.trust_device(&id).await,
                    "deny" => wadd.deny_ask(&id).await,
                    _ => wadd.later_ask(&id).await,
                }
            },
            move |r| {
                let Some(m) = w.upgrade() else { return };
                m.st.borrow_mut().busy = false;
                if let Err(e) = r {
                    m.shell.toast(Level::Error, &e.message, None);
                }
                m.draw();
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(id: &str, device: &str, kind: DeviceKind, at: i64) -> TailnetAsk {
        TailnetAsk { id: id.into(), device: device.into(), os: "android".into(), kind, ip: "100.88.20.7".into(), at }
    }

    #[test]
    fn the_ask_says_who_and_what_trust_lets_it_do() {
        let t = ask_text(&ask("nTab1", "Galaxy Tab S9", DeviceKind::Tablet, 5));
        assert_eq!(t.title, "Galaxy Tab S9 wants to manage this machine");
        assert_eq!(
            t.body,
            "It’s on your tailnet (Android, 100.88.20.7). Trusted, it can open, stream, build and edit this machine’s wadspaces, and restart it to update."
        );
        assert_eq!(t.icon, "tablet");
        assert_eq!(device_icon(DeviceKind::Phone), "smartphone");
    }

    #[test]
    fn the_newest_ask_comes_first_and_closed_ones_wait() {
        let mut t = Tailnet {
            state: TailnetState::Running,
            asks: vec![ask("a", "Pixel", DeviceKind::Phone, 1), ask("b", "Tab", DeviceKind::Tablet, 2)],
            ..Tailnet::default()
        };
        assert_eq!(ask_to_show(Some(&t), &[]).map(|a| a.id.as_str()), Some("b"));
        assert_eq!(ask_to_show(Some(&t), &["b".into()]).map(|a| a.id.as_str()), Some("a"));
        assert_eq!(ask_to_show(Some(&t), &["a".into(), "b".into()]), None);
        // Off the tailnet (or not said yet), nobody asks.
        t.state = TailnetState::NeedsLogin;
        assert_eq!(ask_to_show(Some(&t), &[]), None);
        assert_eq!(ask_to_show(None, &[]), None);
    }
}
