//! Settings ▸ Network (the spike's `panels/NetworkPanel.svelte`): the
//! wire, a way to Wi-Fi, Tailscale as the VPN (it's set up in Accounts)
//! and its exit node (one of my online machines this one's internet
//! traffic goes out through, or none), and the proxy (off).
//!
//! The network is wadd's `network` events (its exit node and what can be
//! one too); Tailscale's state and the devices' names are its `tailnet` events.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use wad_gtk::form::{Choice, Opt, opt};
use wad_gtk::kit::{self, Group, Row, Tone};
use wad_proto::{Event, Network, Tailnet, TailnetState};
use wad_wadd_client::Incoming;

use super::accounts::state_word;
use super::radio::{self, Core};
use crate::link::Link;
use crate::panel::{Ctx, Panel};

pub const VPN_DESC: &str = "WadOS reaches your other machines and your phone over Tailscale.";

/// The wire's row: its subtitle, and its address.
pub fn wired(n: &Network) -> (String, Option<String>) {
    let e = &n.ethernet;
    if !e.plugged {
        return ("Cable unplugged".into(), None);
    }
    let sub = match e.speed_mbps {
        Some(s) => format!("Connected · {s} Mb/s"),
        None => "Connected".into(),
    };
    (sub, e.ip.clone())
}

/// Wi-Fi's line.
pub fn wifi_line(n: &Network) -> String {
    if n.airplane {
        "Airplane mode is on".into()
    } else if !n.wifi_on {
        "Off".into()
    } else if let Some(s) = &n.connected {
        format!("Connected to {s}")
    } else {
        "Not connected".into()
    }
}

/// The exit node in use: its name, and whether it's online (it may not be a choice now).
pub fn exit_of(n: &Network, t: &Tailnet) -> Option<(String, bool)> {
    let id = n.exit_node.as_ref()?;
    if let Some(c) = n.exit_nodes.iter().find(|c| &c.id == id) {
        return Some((c.name.clone(), true));
    }
    let d = t.devices.iter().find(|d| &d.id == id);
    Some((d.map(|d| d.name.clone()).unwrap_or_else(|| id.clone()), d.is_some_and(|d| d.online)))
}

/// The exit node's line (the spike's `exitLine`).
pub fn exit_line(n: &Network, t: &Tailnet) -> String {
    if t.state != TailnetState::Running {
        return "Join Tailscale to send your traffic through another of your machines".into();
    }
    match exit_of(n, t) {
        Some((name, true)) => format!("Your internet traffic goes out through {name}"),
        Some((name, false)) => format!("{name} is offline: nothing gets out until it’s back, or you pick another"),
        None if !n.exit_nodes.is_empty() => "Send your internet traffic out through one of your online machines".into(),
        None => "None of your machines is online right now".into(),
    }
}

/// Tailscale's row's subtitle.
pub fn tailscale_line(n: &Network, t: &Tailnet) -> String {
    if t.state != TailnetState::Running {
        return state_word(t.state).into();
    }
    let mut s = format!("{} · {}", state_word(t.state), t.ip);
    if let Some((name, _)) = exit_of(n, t) {
        s.push_str(&format!(" · out through {name}"));
    }
    s
}

/// The exit node's choices: None, the online ones, and the one in use if it's offline.
pub fn exit_options(n: &Network, t: &Tailnet) -> Vec<Opt<String>> {
    let mut o = vec![opt(String::new(), "None").hint("Straight out")];
    for c in &n.exit_nodes {
        o.push(opt(c.id.clone(), &c.name).hint(&kit::os_name(&c.os)));
    }
    if let (Some(id), Some((name, false))) = (&n.exit_node, exit_of(n, t)) {
        o.push(opt(id.clone(), &name).hint("Offline"));
    }
    o
}

#[derive(Debug, Default)]
pub struct State {
    pub connected: bool,
    pub network: Option<Network>,
    pub tailnet: Option<Tailnet>,
    pub error: Option<String>,
}

pub struct NetworkPage {
    core: Rc<Core>,
    me: Weak<NetworkPage>,
    ctx: RefCell<Option<Ctx>>,
    pub state: RefCell<State>,
}

impl NetworkPage {
    pub fn new(link: Link) -> Rc<Self> {
        let core = Core::new("Network", link);
        Rc::new_cyclic(|me: &Weak<NetworkPage>| {
            let w = me.clone();
            core.on_draw(move || {
                if let Some(m) = w.upgrade() {
                    m.draw();
                }
            });
            Self { core, me: me.clone(), ctx: RefCell::default(), state: RefCell::default() }
        })
    }

    fn load(&self) {
        let w = self.me.clone();
        self.core.call(
            "load",
            |wadd| async move { wadd.network().await },
            move |r| {
                let Some(m) = w.upgrade() else { return true };
                let mut st = m.state.borrow_mut();
                match r {
                    Ok(n) => st.network = Some(n.clone()),
                    Err(e) if st.network.is_none() => st.error = Some(e.message.clone()),
                    Err(_) => {}
                }
                true
            },
        );
    }

    fn open(&self, page: &str) {
        if let Some(c) = &*self.ctx.borrow() {
            c.open(page);
        }
    }

    fn set_exit(&self, node: String) {
        let w = self.me.clone();
        self.core.call(
            "exit",
            move |wadd| async move { wadd.set_exit_node(Some(node.as_str()).filter(|n| !n.is_empty())).await },
            move |r| {
                if let (Some(m), Ok(n)) = (w.upgrade(), r) {
                    m.state.borrow_mut().network = Some(n.clone());
                }
                false
            },
        );
    }

    fn draw(&self) {
        let core = &self.core;
        core.clear();
        let col = &core.page.column;
        let st = self.state.borrow();
        if !st.connected {
            col.append(&kit::hero(
                "circle-alert",
                "wadd isn't answering",
                "Settings shows this machine's network once wadd, which runs it, answers.",
            ));
            return;
        }
        let Some(n) = st.network.clone() else {
            if let Some(e) = &st.error {
                col.append(&kit::hero("network", "The network isn't available here", e));
            }
            return;
        };
        let t = st.tailnet.clone().unwrap_or_default();
        drop(st);

        let g = Group::new("Wired", "");
        let (sub, ip) = wired(&n);
        let row = if n.ethernet.plugged {
            let r = Row::new(Some("cable"), Tone::Accent, "Ethernet", &sub);
            match ip {
                Some(ip) => r.value(&ip, true),
                None => r,
            }
        } else {
            Row::new(Some("cable"), Tone::Plain, "Ethernet", &sub)
        };
        g.add(&row.named("ethernet").root);
        col.append(&g.root);

        let g = Group::new("Wireless", "");
        let icon = if n.connected.is_some() && !n.airplane { "wifi" } else { "wifi-off" };
        let row = Row::new(Some(icon), Tone::Plain, "Wi-Fi", &wifi_line(&n)).named("wifi");
        let w = self.me.clone();
        radio::nav_row(&row, move || {
            if let Some(m) = w.upgrade() {
                m.open("wifi");
            }
        });
        g.add(&row.root);
        col.append(&g.root);

        let g = Group::new("VPN", VPN_DESC);
        let running = t.state == TailnetState::Running;
        let row = Row::new(
            Some("network"),
            if running { Tone::Accent } else { Tone::Plain },
            "Tailscale",
            &tailscale_line(&n, &t),
        )
        .named("vpn-tailscale");
        let w = self.me.clone();
        radio::nav_row(&row, move || {
            if let Some(m) = w.upgrade() {
                m.open("accounts");
            }
        });
        g.add(&row.root);
        let exit = exit_of(&n, &t);
        let tone = match exit {
            Some((_, true)) => Tone::Accent,
            Some((_, false)) => Tone::Warn,
            None => Tone::Plain,
        };
        let row = Row::new(Some("route"), tone, "Exit Node", &exit_line(&n, &t)).named("exit-node");
        let w = self.me.clone();
        let choice = Choice::new(exit_options(&n, &t), n.exit_node.clone().unwrap_or_default(), move |v: &String| {
            if let Some(m) = w.upgrade() {
                m.set_exit(v.clone());
            }
        })
        .named("exit-node-choice");
        choice.root.set_sensitive(running && (!n.exit_nodes.is_empty() || exit.is_some()) && !core.busy("exit"));
        row.ctl.append(&choice.root);
        g.add(&row.root);
        col.append(&g.root);

        let g = Group::new("Proxy", "");
        g.add(
            &Row::new(Some("globe"), Tone::Plain, "Network Proxy", "Connections go straight out")
                .value("Off", false)
                .root,
        );
        col.append(&g.root);
    }
}

pub fn make(ctx: &Ctx) -> Rc<dyn Panel> {
    let p = NetworkPage::new(ctx.link.clone());
    *p.ctx.borrow_mut() = Some(ctx.clone());
    Rc::new(NetworkPanel(p))
}

struct NetworkPanel(Rc<NetworkPage>);

impl Panel for NetworkPanel {
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
                let mut st = p.state.borrow_mut();
                match e {
                    Event::Network(n) => {
                        st.network = Some((**n).clone());
                        st.error = None;
                        true
                    }
                    Event::Tailnet(t) => {
                        st.tailnet = Some(t.clone());
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
        radio::dump(&self.0.core.page.root.clone().upcast())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_proto::{DeviceKind, Ethernet, ExitNodeChoice, TailnetDevice};

    fn tailnet() -> Tailnet {
        Tailnet {
            state: TailnetState::Running,
            ip: "100.64.0.1".into(),
            devices: vec![TailnetDevice {
                id: "nNas".into(),
                name: "NAS".into(),
                os: "linux".into(),
                kind: DeviceKind::Server,
                online: false,
                trusted: true,
                mine: true,
            }],
            ..Default::default()
        }
    }

    fn network() -> Network {
        Network {
            wifi_on: true,
            connected: Some("Home".into()),
            ethernet: Ethernet { plugged: true, speed_mbps: Some(1000), ip: Some("192.168.0.24".into()) },
            exit_nodes: vec![ExitNodeChoice {
                id: "nDesk".into(),
                name: "Desk".into(),
                os: "linux".into(),
                kind: DeviceKind::Desktop,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn the_wire_and_wifi_in_the_spikes_words() {
        let mut n = network();
        assert_eq!(wired(&n), ("Connected · 1000 Mb/s".into(), Some("192.168.0.24".into())));
        assert_eq!(wifi_line(&n), "Connected to Home");
        n.ethernet = Ethernet::default();
        assert_eq!(wired(&n).0, "Cable unplugged");
        n.airplane = true;
        assert_eq!(wifi_line(&n), "Airplane mode is on");
    }

    #[test]
    fn the_exit_node_says_where_traffic_goes() {
        let t = tailnet();
        let mut n = network();
        assert_eq!(exit_line(&n, &t), "Send your internet traffic out through one of your online machines");
        assert_eq!(exit_options(&n, &t).len(), 2);
        n.exit_node = Some("nDesk".into());
        assert_eq!(exit_line(&n, &t), "Your internet traffic goes out through Desk");
        assert_eq!(tailscale_line(&n, &t), "Connected · 100.64.0.1 · out through Desk");
        // Offline: still the choice it is, marked so.
        n.exit_node = Some("nNas".into());
        assert_eq!(exit_line(&n, &t), "NAS is offline: nothing gets out until it’s back, or you pick another");
        let o = exit_options(&n, &t);
        assert_eq!((o.len(), o[2].label.as_str()), (3, "NAS"));
        n.exit_node = None;
        n.exit_nodes.clear();
        assert_eq!(exit_line(&n, &t), "None of your machines is online right now");
        let off = Tailnet { state: TailnetState::NeedsLogin, ..t };
        assert_eq!(exit_line(&n, &off), "Join Tailscale to send your traffic through another of your machines");
        assert_eq!(tailscale_line(&n, &off), "Not signed in");
    }
}
