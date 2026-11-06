//! Settings ▸ Wi-Fi (the spike's `panels/WifiPanel.svelte`): the radio (in
//! the header) and airplane mode, the networks in range (a secure one not
//! joined before asks for its password in place), and the details of the
//! one it's on or a saved one, where it can be forgotten. Saved networks
//! out of range are under them, to forget.
//!
//! What it shows is wadd's (`network` events; `GET /v1/network` once, to
//! learn why when there's none); what it changes goes to wadd, whose
//! answer (and event) it then shows.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::{glib, prelude::*};
use wad_gtk::form::{Empty, TextField, Toggle};
use wad_gtk::kit::{self, Group, Row, Tone, Variant};
use wad_proto::{Event, Network, WifiBand, WifiNetwork};
use wad_wadd_client::Incoming;

use super::radio::{self, Core};
use crate::link::Link;
use crate::panel::{Ctx, Panel};

// ------------------------------------------------------------ what it says

pub const AIRPLANE_SUB: &str = "Takes this machine off the air: no Wi-Fi while it’s on";
pub const HINT: &str = "Click a network to connect. The gear shows a saved network’s details, where you can forget it.";
pub const TOO_SHORT: &str = "Wi-Fi passwords are at least 8 characters.";
pub const WRONG: &str = "That password didn’t work. Check it and try again.";
pub const SAVED_DESC: &str = "Out of range now: it joins them by itself when they’re back.";

/// The spike's bars for a signal (0-100).
pub fn signal_icon(s: u8) -> &'static str {
    if s > 66 {
        "wifi"
    } else if s > 33 {
        "wifi-high"
    } else {
        "wifi-low"
    }
}

pub fn signal_word(s: u8) -> &'static str {
    if s > 80 {
        "Excellent"
    } else if s > 60 {
        "Good"
    } else if s > 40 {
        "OK"
    } else {
        "Weak"
    }
}

pub fn band_word(b: WifiBand) -> &'static str {
    match b {
        WifiBand::Ghz2 => "2.4 GHz",
        WifiBand::Ghz5 => "5 GHz",
        WifiBand::Ghz6 => "6 GHz",
    }
}

/// A network's line under its name.
pub fn status(n: &WifiNetwork) -> &'static str {
    if n.active {
        "Connected"
    } else if n.saved {
        "Saved"
    } else if n.secure {
        "Secure"
    } else {
        "Open"
    }
}

/// The ones in range (and the one it's on), in wadd's order.
pub fn visible(n: &Network) -> Vec<&WifiNetwork> {
    n.networks.iter().filter(|w| w.signal > 0 || w.active).collect()
}

/// Saved, out of range.
pub fn away(n: &Network) -> Vec<&WifiNetwork> {
    n.networks.iter().filter(|w| w.signal == 0 && !w.active && w.saved).collect()
}

/// What a password says before it's sent: too short, or fine (none).
pub fn check_password(p: &str) -> Option<&'static str> {
    (p.chars().count() < 8).then_some(TOO_SHORT)
}

/// What a join that failed says (401: the password).
pub fn join_failed(status: u16, message: &str) -> String {
    if status == 401 { WRONG.into() } else { message.into() }
}

/// A network's details: signal, security, band; and its address and DNS when it's the one it's on.
pub fn detail_rows(w: &WifiNetwork, n: &Network) -> Vec<(&'static str, String, bool)> {
    let mut d = vec![];
    if w.signal > 0 {
        d.push(("Signal", format!("{} ({}%)", signal_word(w.signal), w.signal), false));
    }
    let security =
        w.security.clone().unwrap_or_else(|| if w.secure { "Secured".into() } else { "None (open network)".into() });
    d.push(("Security", security, false));
    if let Some(b) = w.band {
        d.push(("Band", band_word(b).into(), false));
    }
    if w.active {
        if let Some(ip) = &n.ip {
            d.push(("IPv4 address", ip.clone(), true));
        }
        if !n.dns.is_empty() {
            d.push(("DNS", n.dns.join(", "), true));
        }
    }
    d
}

// ------------------------------------------------------------ the page

#[derive(Debug, Default)]
pub struct State {
    pub connected: bool,
    pub network: Option<Network>,
    /// Why there's none (wadd said, e.g. it isn't made on this machine).
    pub error: Option<String>,
}

/// What's open on the page.
#[derive(Default)]
struct Open {
    /// The network asking for its password, its field, and what's wrong.
    joining: Option<(String, TextField)>,
    details: Option<String>,
}

pub struct Wifi {
    core: Rc<Core>,
    me: Weak<Wifi>,
    toggle: Toggle,
    pub state: RefCell<State>,
    open: RefCell<Open>,
}

impl Wifi {
    pub fn new(link: Link) -> Rc<Self> {
        let core = Core::new("Wi-Fi", link);
        Rc::new_cyclic(|me: &Weak<Wifi>| {
            let w = me.clone();
            let toggle = Toggle::new(false, move |on| {
                if let Some(m) = w.upgrade() {
                    m.set_wifi(on);
                }
            })
            .named("wifi-toggle");
            core.page.end.append(&toggle.root);
            let w = me.clone();
            core.on_draw(move || {
                if let Some(m) = w.upgrade() {
                    m.draw();
                }
            });
            Self { core, me: me.clone(), toggle, state: RefCell::default(), open: RefCell::default() }
        })
    }

    pub fn root(&self) -> gtk::Widget {
        self.core.page.root.clone().upcast()
    }

    fn set_wifi(&self, on: bool) {
        let w = self.me.clone();
        self.core.call("wifi", move |wadd| async move { wadd.set_wifi(on).await }, move |r| keep(&w, r))
    }

    fn set_airplane(&self, on: bool) {
        let w = self.me.clone();
        self.core.call("airplane", move |wadd| async move { wadd.set_airplane(on).await }, move |r| keep(&w, r))
    }

    /// Asks wadd how the network is (once it answers: why not, when it can't say).
    pub fn load(&self) {
        let w = self.me.clone();
        self.core.call(
            "load",
            |wadd| async move { wadd.network().await },
            move |r| {
                if let (Some(m), Err(e)) = (w.upgrade(), r) {
                    let mut st = m.state.borrow_mut();
                    if st.network.is_none() {
                        st.error = Some(e.message.clone());
                    }
                    return true;
                }
                keep(&w, r)
            },
        );
    }

    /// A network's row clicked (the spike's `pick`).
    fn pick(&self, n: &WifiNetwork) {
        {
            let mut o = self.open.borrow_mut();
            if n.active {
                o.details = if o.details.as_deref() == Some(n.ssid.as_str()) { None } else { Some(n.ssid.clone()) };
                o.joining = None;
                drop(o);
                self.core.queue_draw();
                return;
            }
            if n.secure && !n.saved {
                let again = o.joining.as_ref().is_some_and(|(s, _)| s == &n.ssid);
                o.joining = if again { None } else { Some((n.ssid.clone(), self.password_field(&n.ssid))) };
                o.details = None;
                drop(o);
                self.core.queue_draw();
                self.focus_field();
                return;
            }
            o.details = None;
        }
        self.connect(&n.ssid, None);
    }

    fn password_field(&self, ssid: &str) -> TextField {
        let f = TextField::password("", &format!("Password for {ssid}")).named("wifi-password");
        f.root.set_hexpand(true);
        let (w, s) = (self.me.clone(), ssid.to_string());
        f.on_enter(move |_| {
            if let Some(m) = w.upgrade() {
                m.join(&s);
            }
        });
        let w = self.me.clone();
        f.on_change(move |_| {
            if let Some(m) = w.upgrade()
                && let Some((_, f)) = &m.open.borrow().joining
            {
                f.set_error(None);
                m.core.queue_draw();
            }
        });
        let key = gtk::EventControllerKey::new();
        let w = self.me.clone();
        key.connect_key_pressed(move |_, k, _, _| {
            if k == gtk::gdk::Key::Escape
                && let Some(m) = w.upgrade()
            {
                m.open.borrow_mut().joining = None;
                m.core.queue_draw();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        f.text.add_controller(key);
        f
    }

    fn focus_field(&self) {
        let w = self.me.clone();
        glib::idle_add_local_once(move || {
            if let Some(m) = w.upgrade()
                && let Some((_, f)) = &m.open.borrow().joining
            {
                f.focus();
            }
        });
    }

    /// Connect, with the password typed.
    fn join(&self, ssid: &str) {
        let Some(f) = self.open.borrow().joining.as_ref().filter(|(s, _)| s == ssid).map(|(_, f)| f.clone()) else {
            return;
        };
        let p = f.value();
        if let Some(e) = check_password(&p) {
            f.set_error(Some(e));
            return;
        }
        self.connect(ssid, Some(p));
    }

    fn connect(&self, ssid: &str, password: Option<String>) {
        let (w, s) = (self.me.clone(), ssid.to_string());
        let what = format!("join:{ssid}");
        let had_password = password.is_some();
        self.core.call(
            &what,
            {
                let s = s.clone();
                move |wadd| async move { wadd.wifi_connect(&s, password.as_deref()).await }
            },
            move |r| {
                let Some(m) = w.upgrade() else { return true };
                match r {
                    Ok(n) => {
                        m.open.borrow_mut().joining = None;
                        m.take(n.clone());
                        true
                    }
                    Err(e) if e.status == 401 || had_password => {
                        // Its password: say so by its field (a saved one's changed: ask for it).
                        let mut o = m.open.borrow_mut();
                        if o.joining.as_ref().is_none_or(|(x, _)| x != &s) {
                            o.joining = Some((s.clone(), m.password_field(&s)));
                        }
                        if let Some((_, f)) = &o.joining {
                            f.set_error(Some(&join_failed(e.status, &e.message)));
                        }
                        drop(o);
                        m.focus_field();
                        true
                    }
                    Err(_) => false,
                }
            },
        );
    }

    fn disconnect(&self) {
        let w = self.me.clone();
        self.core.call("disconnect", |wadd| async move { wadd.wifi_disconnect().await }, move |r| keep(&w, r));
    }

    fn forget(&self, ssid: &str) {
        let (w, s) = (self.me.clone(), ssid.to_string());
        self.open.borrow_mut().details = None;
        self.core.call(
            &format!("forget:{ssid}"),
            move |wadd| async move { wadd.wifi_forget(&s).await },
            move |r| keep(&w, r),
        );
    }

    /// What wadd answered, shown.
    fn take(&self, n: Network) {
        let mut st = self.state.borrow_mut();
        st.network = Some(n);
        st.error = None;
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
                "Settings shows this machine's Wi-Fi once wadd, which runs it, answers.",
            ));
            return;
        }
        let Some(n) = st.network.clone() else {
            self.toggle.root.set_visible(false);
            if let Some(e) = &st.error {
                col.append(&kit::hero("wifi-off", "Wi-Fi isn't available here", e));
            }
            return;
        };
        drop(st);
        self.toggle.root.set_visible(true);
        self.toggle.set(n.wifi_on && !n.airplane);
        self.toggle.root.set_sensitive(!n.airplane && !core.busy("wifi"));

        let g = Group::new("", "");
        let row = Row::new(Some("plane"), Tone::Plain, "Airplane Mode", AIRPLANE_SUB).named("airplane");
        let w = self.me.clone();
        let t = Toggle::new(n.airplane, move |on| {
            if let Some(m) = w.upgrade() {
                m.set_airplane(on);
            }
        })
        .named("airplane-toggle");
        t.root.set_sensitive(!core.busy("airplane"));
        row.ctl.append(&t.root);
        g.add(&row.root);
        col.append(&g.root);

        if n.airplane {
            let e = Empty::new("plane", "Airplane mode is on", "Turn it off to use Wi-Fi.");
            let b = kit::button("Turn Off Airplane Mode", Variant::Secondary, None, false);
            let w = self.me.clone();
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.set_airplane(false);
                }
            });
            e.actions.append(&b);
            col.append(&e.root);
            return;
        }
        if !n.wifi_on {
            let e =
                Empty::new("wifi-off", "Wi-Fi is off", "Turn it on to see the networks in range and connect to one.");
            let b = kit::button("Turn On", Variant::Primary, None, false);
            let w = self.me.clone();
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.set_wifi(true);
                }
            });
            e.actions.append(&b);
            col.append(&e.root);
            return;
        }

        let around = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
        let g = Group::new("Visible Networks", "");
        g.root.set_widget_name("group-networks");
        let nets = visible(&n);
        if nets.is_empty() {
            g.add(&Row::new(None, Tone::Plain, "No networks in range", "It keeps looking.").root);
        }
        for w in nets {
            self.network_row(&g, w, &n);
        }
        around.append(&g.root);
        around.append(&kit::label(HINT, &["set-hint"]));
        col.append(&around);

        let saved = away(&n);
        if !saved.is_empty() {
            let g = Group::new("Saved Networks", SAVED_DESC);
            g.root.set_widget_name("group-saved");
            for w in saved {
                let row = Row::new(Some("wifi-off"), Tone::Plain, &w.ssid, "Not in range")
                    .named(&format!("saved-{}", w.ssid));
                let b = kit::button("Forget", Variant::Quiet, None, true);
                b.set_sensitive(!core.busy(&format!("forget:{}", w.ssid)));
                let (m, s) = (self.me.clone(), w.ssid.clone());
                b.connect_clicked(move |_| {
                    if let Some(m) = m.upgrade() {
                        m.forget(&s);
                    }
                });
                row.ctl.append(&b);
                g.add(&row.root);
            }
            col.append(&g.root);
        }
    }

    fn network_row(&self, g: &Group, w: &WifiNetwork, n: &Network) {
        let core = &self.core;
        let joining = core.busy(&format!("join:{}", w.ssid));
        let sub = if joining { "Connecting…" } else { status(w) };
        let tone = if w.active { Tone::Accent } else { Tone::Plain };
        let row = Row::new(Some(signal_icon(w.signal)), tone, &w.ssid, sub).named(&format!("net-{}", w.ssid));
        if w.active {
            row.root.add_css_class("active");
        }
        if w.secure {
            let l = wad_gtk::Icon::new("lock", 14);
            l.add_css_class("set-muted-ic");
            row.ctl.append(&l);
        }
        if w.active {
            row.ctl.append(&wad_gtk::Icon::new("check", 16));
        }
        let open = self.open.borrow();
        let details = open.details.as_deref() == Some(w.ssid.as_str());
        if w.saved || w.active {
            let gear = radio::icon_button("settings", "Details", "set-gear");
            if details {
                gear.add_css_class("open");
            }
            let (m, s) = (self.me.clone(), w.ssid.clone());
            gear.connect_clicked(move |_| {
                if let Some(m) = m.upgrade() {
                    let mut o = m.open.borrow_mut();
                    o.joining = None;
                    o.details = if o.details.as_deref() == Some(s.as_str()) { None } else { Some(s.clone()) };
                    drop(o);
                    m.core.queue_draw();
                }
            });
            row.ctl.append(&gear);
        }
        let (m, net) = (self.me.clone(), w.clone());
        radio::clickable(&row, move || {
            if let Some(m) = m.upgrade() {
                m.pick(&net);
            }
        });
        g.add(&row.root);

        if let Some((_, f)) = open.joining.as_ref().filter(|(s, _)| s == &w.ssid) {
            let inset = radio::inset();
            let form = gtk::Box::builder().spacing(8).build();
            let had_focus = f.text.has_focus();
            radio::take(&f.root);
            form.append(&f.root);
            let cancel = kit::button("Cancel", Variant::Ghost, None, true);
            cancel.set_valign(gtk::Align::Start);
            let m = self.me.clone();
            cancel.connect_clicked(move |_| {
                if let Some(m) = m.upgrade() {
                    m.open.borrow_mut().joining = None;
                    m.core.queue_draw();
                }
            });
            let connect = kit::button(if joining { "Connecting…" } else { "Connect" }, Variant::Primary, None, true);
            connect.set_valign(gtk::Align::Start);
            connect.set_widget_name("wifi-join");
            connect.set_sensitive(!joining && !f.value().is_empty());
            let (m, s) = (self.me.clone(), w.ssid.clone());
            connect.connect_clicked(move |_| {
                if let Some(m) = m.upgrade() {
                    m.join(&s);
                }
            });
            form.append(&cancel);
            form.append(&connect);
            inset.append(&form);
            g.add(&inset);
            if had_focus {
                self.focus_field();
            }
        } else if details {
            let inset = radio::inset();
            inset.append(&radio::details(&detail_rows(w, n)));
            let acts = gtk::Box::builder().spacing(8).halign(gtk::Align::End).build();
            if w.active {
                let b = kit::button("Disconnect", Variant::Ghost, None, true);
                b.set_sensitive(!core.busy("disconnect"));
                let m = self.me.clone();
                b.connect_clicked(move |_| {
                    if let Some(m) = m.upgrade() {
                        m.disconnect();
                    }
                });
                acts.append(&b);
            } else {
                let b = kit::button("Connect", Variant::Secondary, None, true);
                b.set_sensitive(!joining);
                let (m, s) = (self.me.clone(), w.ssid.clone());
                b.connect_clicked(move |_| {
                    if let Some(m) = m.upgrade() {
                        m.open.borrow_mut().details = None;
                        m.connect(&s, None);
                    }
                });
                acts.append(&b);
            }
            if w.saved {
                let b = kit::button("Forget Network", Variant::Danger, Some("trash"), true);
                b.set_widget_name("wifi-forget");
                b.set_sensitive(!core.busy(&format!("forget:{}", w.ssid)));
                let (m, s) = (self.me.clone(), w.ssid.clone());
                b.connect_clicked(move |_| {
                    if let Some(m) = m.upgrade() {
                        m.forget(&s);
                    }
                });
                acts.append(&b);
            }
            inset.append(&acts);
            g.add(&inset);
        }
    }
}

/// A network wadd answered with, kept (an error: the note says it).
fn keep(w: &Weak<Wifi>, r: &Result<Network, wad_wadd_client::Failure>) -> bool {
    if let (Some(m), Ok(n)) = (w.upgrade(), r) {
        m.take(n.clone());
    }
    false
}

// ------------------------------------------------------------ as a panel

pub fn make(ctx: &Ctx) -> Rc<dyn Panel> {
    Rc::new(WifiPanel(Wifi::new(ctx.link.clone())))
}

struct WifiPanel(Rc<Wifi>);

impl Panel for WifiPanel {
    fn build(&self, _: &Ctx) -> gtk::Widget {
        self.0.draw();
        self.0.root()
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
                    Event::Network(n) => {
                        p.take((**n).clone());
                        true
                    }
                    _ => !was,
                }
            }
            Incoming::Disconnected => std::mem::replace(&mut p.state.borrow_mut().connected, false),
            Incoming::Other(..) => false,
        };
        if changed {
            p.core.queue_draw();
        }
    }

    fn ready(&self) -> bool {
        let st = self.0.state.borrow();
        st.connected && (st.network.is_some() || st.error.is_some())
    }

    fn dump(&self, _: &gtk::Widget) -> String {
        self.0.draw();
        radio::dump(&self.0.root())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(ssid: &str, signal: u8, secure: bool, saved: bool, active: bool) -> WifiNetwork {
        WifiNetwork {
            ssid: ssid.into(),
            signal,
            secure,
            saved,
            active,
            band: Some(WifiBand::Ghz5),
            security: secure.then(|| "WPA2 Personal".into()),
        }
    }

    #[test]
    fn a_network_in_the_spikes_words() {
        assert_eq!((signal_icon(82), signal_icon(50), signal_icon(20)), ("wifi", "wifi-high", "wifi-low"));
        assert_eq!(
            (signal_word(90), signal_word(64), signal_word(41), signal_word(23)),
            ("Excellent", "Good", "OK", "Weak")
        );
        assert_eq!(status(&net("Home", 80, true, true, true)), "Connected");
        assert_eq!(status(&net("Office", 60, true, true, false)), "Saved");
        assert_eq!(status(&net("Next door", 40, true, false, false)), "Secure");
        assert_eq!(status(&net("Cafe", 50, false, false, false)), "Open");
        assert_eq!(band_word(WifiBand::Ghz2), "2.4 GHz");
    }

    #[test]
    fn passwords_are_checked_before_they_go_and_a_wrong_one_says_so() {
        assert_eq!(check_password("short"), Some("Wi-Fi passwords are at least 8 characters."));
        assert_eq!(check_password("long enough"), None);
        assert_eq!(join_failed(401, "That isn't the password for Home"), WRONG);
        assert_eq!(join_failed(404, "Home isn't in range"), "Home isn't in range");
    }

    #[test]
    fn in_range_and_saved_away_and_the_details() {
        let n = Network {
            wifi_on: true,
            connected: Some("Home".into()),
            networks: vec![
                net("Home", 82, true, true, true),
                net("Office", 0, true, true, false),
                net("Cafe", 58, false, false, false),
            ],
            ip: Some("192.168.1.42".into()),
            dns: vec!["192.168.1.1".into(), "1.1.1.1".into()],
            ..Default::default()
        };
        let v: Vec<&str> = visible(&n).iter().map(|w| w.ssid.as_str()).collect();
        assert_eq!(v, ["Home", "Cafe"]);
        let a: Vec<&str> = away(&n).iter().map(|w| w.ssid.as_str()).collect();
        assert_eq!(a, ["Office"]);
        let d = detail_rows(&n.networks[0], &n);
        let words: Vec<(&str, &str)> = d.iter().map(|(k, v, _)| (*k, v.as_str())).collect();
        assert_eq!(
            words,
            [
                ("Signal", "Excellent (82%)"),
                ("Security", "WPA2 Personal"),
                ("Band", "5 GHz"),
                ("IPv4 address", "192.168.1.42"),
                ("DNS", "192.168.1.1, 1.1.1.1"),
            ]
        );
        let cafe = detail_rows(&n.networks[2], &n);
        assert_eq!(cafe[1].1, "None (open network)");
        assert_eq!(cafe.len(), 3, "not the one it's on: no address");
    }
}
