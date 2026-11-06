//! Settings ▸ Bluetooth (the spike's `panels/BluetoothPanel.svelte`): the
//! radio (in the header), the devices paired ("My Devices", with their
//! battery) and the ones nearby ("Nearby Devices"); connecting,
//! disconnecting, forgetting, and pairing.
//!
//! While its page is open it looks for devices (and this machine can be
//! seen by them), asking wadd again every minute (wadd stops by itself
//! two minutes after the last ask). Pairing doesn't wait: what it asks
//! comes as the `btPrompt` event, shown as a dialog over the window (a
//! code to confirm, a code to type on the device, a PIN to enter, may it
//! connect?), and the answer goes back to wadd; a pairing that fails is a
//! notice, said under the title.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::{glib, prelude::*};
use wad_gtk::form::{self, Empty, TextField, Toggle};
use wad_gtk::kit::{self, Group, Row, Tone, Variant};
use wad_proto::{Bluetooth, BtAnswer, BtAsk, BtDevice, BtKind, BtPrompt, Event};
use wad_wadd_client::Incoming;

use super::radio::{self, Core};
use crate::link::Link;
use crate::panel::{Ctx, Panel};

/// Looking for devices is asked for again this often while the page is open.
const SCAN_AGAIN: Duration = Duration::from_secs(60);

/// A device's glyph (the spike's ICON).
pub fn kind_icon(k: BtKind) -> &'static str {
    match k {
        BtKind::Headphones => "headphones",
        BtKind::Mouse => "mouse",
        BtKind::Keyboard => "keyboard",
        BtKind::Phone => "smartphone",
        BtKind::Speaker => "speaker",
        BtKind::Controller => "gamepad-2",
        BtKind::Other => "bluetooth",
    }
}

pub fn battery_icon(b: u8) -> &'static str {
    if b < 25 { "battery-low" } else { "battery" }
}

pub fn lead(name: &str) -> String {
    format!("Visible as “{name}” to devices nearby while this page is open.")
}

/// What a pairing prompt says: its title, its text, and its code (if it shows one).
pub fn prompt_words(p: &BtPrompt) -> (String, String, Option<String>) {
    let n = &p.name;
    match &p.kind {
        BtAsk::Confirm { code } => (
            format!("Pair with {n}?"),
            format!("Check that {n} shows this code, then choose Pair."),
            Some(code.clone()),
        ),
        BtAsk::Show { code } => {
            (format!("Pair with {n}"), format!("Type this code on {n}, then press Enter."), Some(code.clone()))
        }
        BtAsk::Enter => {
            (format!("Pair with {n}"), format!("Enter the PIN {n} asks for. Older devices often use 0000."), None)
        }
        BtAsk::Authorize => (format!("Let {n} connect?"), format!("{n} wants to connect to this machine."), None),
    }
}

#[derive(Debug, Default)]
pub struct State {
    pub connected: bool,
    pub bluetooth: Option<Bluetooth>,
    pub error: Option<String>,
    pub prompt: Option<BtPrompt>,
}

pub struct BluetoothPage {
    core: Rc<Core>,
    me: Weak<BluetoothPage>,
    toggle: Toggle,
    ctx: RefCell<Option<Ctx>>,
    pub state: RefCell<State>,
    /// Being paired (by address), until it shows paired or it fails.
    pairing: RefCell<HashSet<String>>,
    /// The prompt's dialog that's open (by the prompt's id).
    dialog: RefCell<Option<(String, form::Open)>>,
    /// The page is shown: it looks for devices.
    shown: std::cell::Cell<bool>,
}

impl BluetoothPage {
    pub fn new(link: Link) -> Rc<Self> {
        let core = Core::new("Bluetooth", link);
        let me = Rc::new_cyclic(|me: &Weak<BluetoothPage>| {
            let w = me.clone();
            let toggle = Toggle::new(false, move |on| {
                if let Some(m) = w.upgrade() {
                    m.power(on);
                }
            })
            .named("bluetooth-toggle");
            core.page.end.append(&toggle.root);
            let w = me.clone();
            core.on_draw(move || {
                if let Some(m) = w.upgrade() {
                    m.draw();
                }
            });
            Self {
                core,
                me: me.clone(),
                toggle,
                ctx: RefCell::default(),
                state: RefCell::default(),
                pairing: RefCell::default(),
                dialog: RefCell::default(),
                shown: std::cell::Cell::new(false),
            }
        });
        // Looking for devices while the page is shown.
        let root = &me.core.page.root;
        let w = Rc::downgrade(&me);
        root.connect_map(move |_| {
            let Some(m) = w.upgrade() else { return };
            m.shown.set(true);
            m.scan(true);
            let w = Rc::downgrade(&m);
            glib::timeout_add_local(SCAN_AGAIN, move || {
                let Some(m) = w.upgrade() else { return glib::ControlFlow::Break };
                if !m.shown.get() {
                    return glib::ControlFlow::Break;
                }
                m.scan(true);
                glib::ControlFlow::Continue
            });
        });
        let w = Rc::downgrade(&me);
        root.connect_unmap(move |_| {
            if let Some(m) = w.upgrade() {
                m.shown.set(false);
                m.scan(false);
            }
        });
        me
    }

    fn take(&self, b: &Bluetooth) {
        let mut st = self.state.borrow_mut();
        // Paired now: no longer being paired.
        self.pairing.borrow_mut().retain(|a| !b.devices.iter().any(|d| &d.address == a && d.paired));
        st.bluetooth = Some(b.clone());
        st.error = None;
    }

    fn keep(&self) -> impl FnOnce(&Result<Bluetooth, wad_wadd_client::Failure>) -> bool + 'static {
        let w = self.me.clone();
        move |r| {
            if let (Some(m), Ok(b)) = (w.upgrade(), r) {
                m.take(b);
            }
            false
        }
    }

    fn load(&self) {
        let w = self.me.clone();
        self.core.call(
            "load",
            |wadd| async move { wadd.bluetooth().await },
            move |r| {
                let Some(m) = w.upgrade() else { return true };
                match r {
                    Ok(b) => m.take(b),
                    Err(e) => {
                        let mut st = m.state.borrow_mut();
                        if st.bluetooth.is_none() {
                            st.error = Some(e.message.clone());
                        }
                    }
                }
                true
            },
        );
    }

    fn power(&self, on: bool) {
        self.core.call("power", move |wadd| async move { wadd.set_bluetooth(on).await }, self.keep());
        if on && self.shown.get() {
            // Looking for devices once it's on.
            let w = self.me.clone();
            glib::timeout_add_local_once(Duration::from_millis(800), move || {
                if let Some(m) = w.upgrade() {
                    m.scan(true);
                }
            });
        }
    }

    fn scan(&self, on: bool) {
        let is_on = self.state.borrow().bluetooth.as_ref().is_some_and(|b| b.on);
        if !is_on {
            return;
        }
        let w = self.me.clone();
        self.core.call(
            "scan",
            move |wadd| async move { wadd.bt_scan(on).await },
            move |r| {
                if let (Some(m), Ok(b)) = (w.upgrade(), r) {
                    m.take(b);
                }
                // Not looking isn't worth a word.
                true
            },
        );
    }

    fn device(&self, d: &BtDevice, action: &'static str) {
        let a = d.address.clone();
        if action == "pair" {
            self.pairing.borrow_mut().insert(a.clone());
            // Nothing heard in two minutes: it's over.
            let (w, x) = (self.me.clone(), a.clone());
            glib::timeout_add_local_once(Duration::from_secs(120), move || {
                if let Some(m) = w.upgrade()
                    && m.pairing.borrow_mut().remove(&x)
                {
                    m.core.queue_draw();
                }
            });
        }
        let w = self.me.clone();
        let x = a.clone();
        self.core.call(
            &format!("{action}:{a}"),
            move |wadd| async move {
                match action {
                    "pair" => wadd.bt_pair(&a).await,
                    "connect" => wadd.bt_connect(&a).await,
                    "disconnect" => wadd.bt_disconnect(&a).await,
                    _ => wadd.bt_forget(&a).await,
                }
            },
            move |r| {
                let Some(m) = w.upgrade() else { return true };
                match r {
                    Ok(b) => m.take(b),
                    Err(_) => {
                        m.pairing.borrow_mut().remove(&x);
                    }
                }
                false
            },
        );
    }

    /// What pairing asks, shown (none: its dialog closed).
    fn prompt(&self, p: Option<BtPrompt>) {
        let open = self.dialog.borrow_mut().take();
        if let Some((id, o)) = open {
            if p.as_ref().is_some_and(|p| p.id == id) {
                *self.dialog.borrow_mut() = Some((id, o));
                return;
            }
            o.close();
        }
        self.state.borrow_mut().prompt = p.clone();
        let Some(p) = p else { return };
        let Some(ctx) = self.ctx.borrow().clone() else { return };
        let (title, text, code) = prompt_words(&p);
        let mut d = form::Dialog::new(&title).icon("bluetooth", Tone::Accent).text(&text).named("bt-prompt");
        if let Some(c) = &code {
            let l = kit::label(c, &["set-code"]);
            l.set_xalign(0.5);
            l.set_selectable(true);
            d = d.child(&l);
        }
        let pin = matches!(p.kind, BtAsk::Enter).then(|| TextField::new("PIN", "0000").mono().named("bt-pin"));
        if let Some(f) = &pin {
            d = d.child(&f.root);
        }
        d = match p.kind {
            BtAsk::Confirm { .. } => {
                d.button("no", "Cancel", Variant::Ghost).button("yes", "Pair", Variant::Primary).cancel("no")
            }
            BtAsk::Show { .. } => d.button("no", "Cancel", Variant::Ghost).cancel("no"),
            BtAsk::Enter => d
                .button("no", "Cancel", Variant::Ghost)
                .button("yes", "Pair", Variant::Primary)
                .hold("yes")
                .cancel("no"),
            BtAsk::Authorize => {
                d.button("no", "Deny", Variant::Ghost).button("yes", "Allow", Variant::Primary).cancel("no")
            }
        };
        let (w, id) = (self.me.clone(), p.id.clone());
        let f = pin.clone();
        let o = ctx.dialog(d, move |o, answer| {
            let Some(m) = w.upgrade() else { return };
            let accept = answer == "yes";
            let pin = f.as_ref().map(|f| f.value().trim().to_string());
            if accept && let (Some(f), Some(p)) = (&f, &pin) {
                if p.is_empty() || p.len() > 16 || !p.chars().all(|c| c.is_ascii_alphanumeric()) {
                    f.set_error(Some("A PIN is up to 16 letters and digits."));
                    return;
                }
                o.close();
            }
            m.dialog.borrow_mut().take();
            m.answer(&id, BtAnswer { accept, pin: pin.filter(|_| accept) });
        });
        if let Some(f) = &pin {
            f.focus();
        }
        *self.dialog.borrow_mut() = Some((p.id.clone(), o));
    }

    fn answer(&self, id: &str, a: BtAnswer) {
        let id = id.to_string();
        let w = self.me.clone();
        self.core.call(
            "answer",
            move |wadd| async move { wadd.bt_answer(&id, &a).await },
            move |r| {
                if let (Some(m), Ok(b)) = (w.upgrade(), r) {
                    m.take(b);
                }
                // Gone already (answered elsewhere, or timed out): nothing to say.
                matches!(r, Err(e) if e.status == 404)
            },
        );
    }

    fn draw(&self) {
        let core = &self.core;
        core.clear();
        let col = &core.page.column;
        let st = self.state.borrow();
        if !st.connected {
            self.toggle.root.set_visible(false);
            col.append(&kit::hero(
                "circle-alert",
                "wadd isn't answering",
                "Settings shows this machine's Bluetooth once wadd, which runs it, answers.",
            ));
            return;
        }
        let Some(b) = st.bluetooth.clone() else {
            self.toggle.root.set_visible(false);
            if let Some(e) = &st.error {
                col.append(&kit::hero("bluetooth-off", "Bluetooth isn't available here", e));
            }
            return;
        };
        drop(st);
        self.toggle.root.set_visible(true);
        self.toggle.set(b.on);
        self.toggle.root.set_sensitive(!core.busy("power"));

        if !b.on {
            let e = Empty::new(
                "bluetooth-off",
                "Bluetooth is off",
                "Turn it on to connect headphones, a mouse, a keyboard or a controller.",
            );
            let t = kit::button("Turn On", Variant::Primary, None, false);
            let w = self.me.clone();
            t.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.power(true);
                }
            });
            e.actions.append(&t);
            col.append(&e.root);
            return;
        }
        col.append(&kit::label(&lead(&b.discoverable_name), &["set-lead"]));

        let paired: Vec<&BtDevice> = b.devices.iter().filter(|d| d.paired).collect();
        if !paired.is_empty() {
            let g = Group::new("My Devices", "");
            g.root.set_widget_name("group-my-devices");
            for d in paired {
                g.add(&self.paired_row(d).root);
            }
            col.append(&g.root);
        }

        let g = Group::new("Nearby Devices", "");
        g.root.set_widget_name("group-nearby");
        if b.scanning {
            let s = gtk::Box::builder().spacing(8).css_classes(["set-searching"]).build();
            let sp = gtk::Spinner::builder().spinning(true).build();
            sp.set_size_request(14, 14);
            s.append(&sp);
            s.append(&gtk::Label::new(Some("Searching for devices…")));
            g.suffix.append(&s);
        }
        let nearby: Vec<&BtDevice> = b.devices.iter().filter(|d| !d.paired).collect();
        if nearby.is_empty() {
            g.add(
                &Row::new(None, Tone::Plain, "Nothing else nearby yet", "Put a device in pairing mode to see it here.")
                    .root,
            );
        }
        for d in nearby {
            let pairing = self.pairing.borrow().contains(&d.address) || core.busy(&format!("pair:{}", d.address));
            let row = Row::new(
                Some(kind_icon(d.kind)),
                Tone::Plain,
                &d.name,
                if pairing { "Pairing…" } else { "Not set up" },
            )
            .named(&format!("bt-{}", d.address));
            let p = kit::button(if pairing { "Pairing…" } else { "Pair" }, Variant::Secondary, None, true);
            p.set_sensitive(!pairing);
            let (w, dev) = (self.me.clone(), d.clone());
            p.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.device(&dev, "pair");
                }
            });
            row.ctl.append(&p);
            g.add(&row.root);
        }
        col.append(&g.root);
    }

    fn paired_row(&self, d: &BtDevice) -> Row {
        let core = &self.core;
        let row = Row::new(
            Some(kind_icon(d.kind)),
            if d.connected { Tone::Accent } else { Tone::Plain },
            &d.name,
            if d.connected { "Connected" } else { "Disconnected" },
        )
        .named(&format!("bt-{}", d.address));
        if let (true, Some(b)) = (d.connected, d.battery) {
            let bx = gtk::Box::builder().spacing(5).css_classes(["set-battery"]).valign(gtk::Align::Center).build();
            bx.set_tooltip_text(Some("Battery"));
            bx.append(&wad_gtk::Icon::new(battery_icon(b), 15));
            bx.append(&gtk::Label::new(Some(&format!("{b}%"))));
            row.ctl.append(&bx);
        }
        let action = if d.connected { "disconnect" } else { "connect" };
        let busy = core.busy(&format!("{action}:{}", d.address));
        let label = match (d.connected, busy) {
            (false, true) => "Connecting…",
            (false, false) => "Connect",
            (true, _) => "Disconnect",
        };
        let b = kit::button(label, if d.connected { Variant::Quiet } else { Variant::Secondary }, None, true);
        b.set_sensitive(!busy);
        let (w, dev) = (self.me.clone(), d.clone());
        b.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.device(&dev, action);
            }
        });
        row.ctl.append(&b);
        let f = kit::button("", Variant::Ghost, Some("trash"), true);
        f.set_tooltip_text(Some(&format!("Forget {}", d.name)));
        f.set_sensitive(!core.busy(&format!("forget:{}", d.address)));
        let (w, dev) = (self.me.clone(), d.clone());
        f.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.device(&dev, "forget");
            }
        });
        row.ctl.append(&f);
        row
    }
}

pub fn make(ctx: &Ctx) -> Rc<dyn Panel> {
    let p = BluetoothPage::new(ctx.link.clone());
    *p.ctx.borrow_mut() = Some(ctx.clone());
    Rc::new(BluetoothPanel(p))
}

struct BluetoothPanel(Rc<BluetoothPage>);

impl Panel for BluetoothPanel {
    fn build(&self, _: &Ctx) -> gtk::Widget {
        self.0.draw();
        self.0.core.page.root.clone().upcast()
    }

    fn on_event(&self, _: &Ctx, i: &Incoming) {
        let p = &self.0;
        let changed = match i {
            Incoming::Event(e) => {
                let was = std::mem::replace(&mut p.state.borrow_mut().connected, true);
                if !was {
                    p.load();
                }
                match e {
                    Event::Bluetooth(b) => {
                        p.take(b);
                        // Off (airplane mode, the switch): nothing's being paired.
                        if !b.on {
                            p.pairing.borrow_mut().clear();
                            p.prompt(None);
                        }
                        true
                    }
                    Event::BtPrompt { prompt } => {
                        p.prompt(prompt.clone());
                        true
                    }
                    // A pairing that failed (wadd's notice): said here too.
                    Event::Notice { title, body, icon, .. } if icon.as_deref() == Some("bluetooth") => {
                        p.pairing.borrow_mut().clear();
                        let said = match body {
                            Some(b) => format!("{title}: {b}"),
                            None => title.clone(),
                        };
                        p.core.say(Some(&said));
                        true
                    }
                    _ => !was,
                }
            }
            Incoming::Disconnected => {
                p.prompt(None);
                std::mem::replace(&mut p.state.borrow_mut().connected, false)
            }
            Incoming::Other(..) => false,
        };
        if changed {
            p.core.queue_draw();
        }
    }

    fn ready(&self) -> bool {
        let st = self.0.state.borrow();
        st.connected && (st.bluetooth.is_some() || st.error.is_some())
    }

    fn dump(&self, _: &gtk::Widget) -> String {
        self.0.draw();
        radio::dump(&self.0.core.page.root.clone().upcast())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_and_prompts_in_words() {
        assert_eq!(kind_icon(BtKind::Controller), "gamepad-2");
        assert_eq!(kind_icon(BtKind::Phone), "smartphone");
        assert_eq!((battery_icon(80), battery_icon(10)), ("battery", "battery-low"));
        assert_eq!(lead("desk"), "Visible as “desk” to devices nearby while this page is open.");
        let p = |kind| BtPrompt { id: "bt-1".into(), device: "AA".into(), name: "JBL Flip 6".into(), kind };
        let (t, x, c) = prompt_words(&p(BtAsk::Confirm { code: "492817".into() }));
        assert_eq!(t, "Pair with JBL Flip 6?");
        assert!(x.contains("shows this code"));
        assert_eq!(c.as_deref(), Some("492817"));
        assert_eq!(
            prompt_words(&p(BtAsk::Show { code: "123456".into() })).1,
            "Type this code on JBL Flip 6, then press Enter."
        );
        assert_eq!(prompt_words(&p(BtAsk::Enter)).2, None);
        assert_eq!(prompt_words(&p(BtAsk::Authorize)).0, "Let JBL Flip 6 connect?");
    }
}
