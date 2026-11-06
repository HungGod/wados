//! Files' Machines (the spike's `MachinesView.svelte`, `PeerView.svelte`):
//!
//! - **Machines:** my other machines on the tailnet as device tiles (Online,
//!   or when it was last seen; what it runs; how many wadspaces), any device
//!   asking to be trusted on top. Off the tailnet: a way to join it.
//! - **A machine:** its Home (over Taildrive) and its wadspaces, each with
//!   Copy here; offline, it says so.
//! - **A folder in its home:** what's in it, over Taildrive; a folder opens
//!   in Files, a file with its default app.

use gtk::{gdk, glib, prelude::*};
use wad_gtk::Icon;
use wad_gtk::kit::{self, Variant};
use wad_model::PeerKind;
use wad_proto::{TailnetAsk, TailnetState};

use crate::drive::{self, Entry};
use crate::state::{self, Loc, State};
use crate::ui::{Files, item, now_ms};

/// A menu's row: its icon, name, hint, and what it does.
type MenuRow<'a> = (&'static str, &'a str, Option<String>, Box<dyn Fn()>);

impl Files {
    // ------------------------------------------------------------ Machines

    pub(crate) fn machines_view(&self) -> String {
        let st = self.state.borrow();
        let t = st.tailnet.clone().unwrap_or_default();
        if t.state != TailnetState::Running {
            let body = match t.state {
                TailnetState::NeedsLogin | TailnetState::Off => {
                    "Sign in to Tailscale and your other WadOS machines show up here, with their wadspaces and folders."
                }
                TailnetState::Stopped => {
                    "Tailscale is stopped on this machine. Turn it on to reach your other machines."
                }
                _ => "Tailscale is still connecting…",
            };
            let hero = kit::hero("network", "Join your tailnet to see your other machines", body);
            let b = kit::button("Tailscale settings", Variant::Primary, Some("network"), false);
            b.set_halign(gtk::Align::Center);
            b.connect_clicked(|_| open_accounts());
            hero.append(&b);
            self.body.append(&hero);
            return String::new();
        }
        for a in asks(&st).iter().rev() {
            self.body.append(&self.ask_row(a));
        }
        let q = self.q();
        let machines: Vec<_> = st
            .machines()
            .into_iter()
            .filter(|p| q.is_empty() || p.peer.name.to_lowercase().contains(&q) || p.dns_name.contains(&q))
            .cloned()
            .collect();
        if machines.is_empty() {
            let (title, body) = if q.is_empty() {
                (
                    "No other machines yet".to_string(),
                    "Your other WadOS machines on this tailnet, signed in to Tailscale as you, show up here."
                        .to_string(),
                )
            } else {
                ("No machines match".into(), format!("Nothing called “{}”.", self.filter.text()))
            };
            self.body.append(&kit::hero("server", &title, &body));
            return String::new();
        }
        let grid = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .min_children_per_line(1)
            .max_children_per_line(4)
            .column_spacing(14)
            .row_spacing(14)
            .homogeneous(true)
            .valign(gtk::Align::Start)
            .build();
        let now = now_ms();
        let selected = self.sel_first();
        for p in &machines {
            let n = st.spaces.get(&p.peer.id).filter(|s| s.at > 0).map(|s| s.list.len());
            grid.append(&self.device(p, now, n, selected.as_deref() == Some(p.peer.id.as_str())));
        }
        self.body.append(&grid);
        format!("{} machine{}", machines.len(), if machines.len() == 1 { "" } else { "s" })
    }

    fn device(&self, p: &wad_proto::PeerInfo, now: i64, spaces: Option<usize>, selected: bool) -> gtk::Box {
        let d = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).css_classes(["device"]).build();
        d.set_size_request(230, -1);
        d.set_widget_name(&format!("peer-{}", p.peer.id));
        if !p.peer.online {
            d.add_css_class("offline");
        }
        if selected {
            d.add_css_class("selected");
        }
        let glyph = gtk::Overlay::builder().halign(gtk::Align::Start).build();
        let g = gtk::Box::builder().css_classes(["device-glyph"]).hexpand(false).build();
        let ic = Icon::new(kind_icon(p.peer.kind), 40);
        ic.set_hexpand(true);
        ic.set_halign(gtk::Align::Center);
        ic.set_valign(gtk::Align::Center);
        g.append(&ic);
        glyph.set_child(Some(&g));
        let dot =
            gtk::Box::builder().css_classes(["device-dot"]).halign(gtk::Align::End).valign(gtk::Align::End).build();
        dot.set_margin_bottom(8);
        if p.peer.online {
            dot.add_css_class("on");
        }
        glyph.add_overlay(&dot);
        d.append(&glyph);
        d.append(&kit::label(&p.peer.name, &["device-name"]));
        d.append(&kit::label(&p.dns_name, &["device-host"]));
        let s = kit::label(&state::status(p, now), &["device-status"]);
        if p.peer.online {
            s.add_css_class("on");
        }
        d.append(&s);
        let facts = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(3).margin_top(8).build();
        facts.append(&fact("hard-drive", &p.peer.os));
        if let Some(gpu) = &p.peer.gpu {
            facts.append(&fact("cpu", gpu));
        }
        if let Some(n) = spaces {
            facts.append(&fact("monitor", &state::count(n)));
        }
        d.append(&facts);
        // A click picks it, a double-click (or Enter) opens it, a right-click is its menu.
        let click = gtk::GestureClick::new();
        let (w, id) = (self.me.clone(), p.peer.id.clone());
        click.connect_released(move |g, n, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            let Some(m) = w.upgrade() else { return };
            if n >= 2 {
                m.go(Loc::Peer(id.clone()));
            } else {
                m.pick_only(&id);
                m.queue_draw();
            }
        });
        d.add_controller(click);
        let menu = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let (w, id, addr, name) = (self.me.clone(), p.peer.id.clone(), p.dns_name.clone(), p.peer.name.clone());
        let dw = d.downgrade();
        menu.connect_pressed(move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            let (Some(m), Some(d)) = (w.upgrade(), dw.upgrade()) else { return };
            m.pick_only(&id);
            m.machine_menu(&d, &id, &name, &addr, x, y);
        });
        d.add_controller(menu);
        d
    }

    /// A machine's menu (MachinesView's): Open, Copy address, Tailscale settings.
    fn machine_menu(&self, on: &gtk::Box, id: &str, name: &str, addr: &str, x: f64, y: f64) {
        let pop = gtk::Popover::builder().has_arrow(false).css_classes(["files-menu"]).build();
        let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
        col.append(&kit::label(&name.to_uppercase(), &["files-h5"]));
        let items: Vec<MenuRow> = vec![
            ("folder-open", "Open", None, {
                let (w, id) = (self.me.clone(), id.to_string());
                Box::new(move || {
                    if let Some(m) = w.upgrade() {
                        m.go(Loc::Peer(id.clone()));
                    }
                })
            }),
            ("copy", "Copy address", Some(addr.to_string()), {
                let addr = addr.to_string();
                Box::new(move || {
                    if let Some(d) = gdk::Display::default() {
                        d.clipboard().set_text(&addr);
                    }
                })
            }),
            ("network", "Tailscale settings", None, Box::new(open_accounts)),
        ];
        for (icon, label, hint, run) in items {
            let r = gtk::Box::builder().spacing(10).build();
            r.append(&Icon::new(icon, 16));
            let t = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
            t.append(&kit::label(label, &[]));
            if let Some(h) = hint {
                t.append(&kit::label(&h, &["item-sub"]));
            }
            r.append(&t);
            let b = gtk::Button::builder().child(&r).css_classes(["kit-btn", "ghost", "sm"]).build();
            let p = pop.downgrade();
            b.connect_clicked(move |_| {
                if let Some(p) = p.upgrade() {
                    p.popdown();
                }
                run();
            });
            col.append(&b);
        }
        pop.set_child(Some(&col));
        pop.set_parent(on);
        pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        pop.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        pop.popup();
    }

    fn ask_row(&self, a: &TailnetAsk) -> gtk::Box {
        let r = gtk::Box::builder().spacing(12).css_classes(["files-ask"]).build();
        r.set_widget_name(&format!("ask-{}", a.id));
        let badge =
            gtk::Box::builder().css_classes(["files-ask-badge"]).valign(gtk::Align::Center).hexpand(false).build();
        let ic = Icon::new(device_icon(a.kind), 18);
        ic.set_hexpand(true);
        ic.set_halign(gtk::Align::Center);
        ic.set_valign(gtk::Align::Center);
        badge.append(&ic);
        r.append(&badge);
        let what = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
        let t = gtk::Label::builder().xalign(0.0).wrap(true).build();
        t.set_markup(&format!("<b>{} wants to be trusted</b>", glib::markup_escape_text(&a.device)));
        what.append(&t);
        what.append(&kit::label(
            &format!(
                "{} · {} · asked {}. Trusted, it can open, stream and build your wadspaces.",
                kit::os_name(&a.os),
                a.ip,
                kit::ago(now_ms(), a.at)
            ),
            &["files-ask-sub"],
        ));
        r.append(&what);
        let deny = kit::button("Deny", Variant::Ghost, None, true);
        let trust = kit::button("Trust", Variant::Primary, Some("shield-check"), true);
        for (b, yes) in [(&deny, false), (&trust, true)] {
            b.set_valign(gtk::Align::Center);
            let (w, id) = (self.me.clone(), a.id.clone());
            b.connect_clicked(move |b| {
                b.set_sensitive(false);
                let Some(m) = w.upgrade() else { return };
                let (w, id) = (m.me.clone(), id.clone());
                m.link.call(
                    move |wadd| async move { if yes { wadd.trust_device(&id).await } else { wadd.deny_ask(&id).await } },
                    move |r| {
                        if let (Err(e), Some(m)) = (r, w.upgrade()) {
                            m.say(&e.message);
                            m.queue_draw();
                        }
                    },
                );
            });
            r.append(b);
        }
        r
    }

    // ------------------------------------------------------------ a machine

    pub(crate) fn peer_view(&self, peer: &str) -> String {
        let st = self.state.borrow();
        let Some(p) = st.peer(peer).cloned() else {
            self.body.append(&gone("That machine isn't on your tailnet any more."));
            return String::new();
        };
        if !st.running() {
            self.body.append(&gone("Join your tailnet to see your other machines."));
            return String::new();
        }
        if !p.peer.online {
            let away = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(8)
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .vexpand(true)
                .css_classes(["files-away"])
                .build();
            let ic = Icon::new("cloud-off", 30);
            ic.set_halign(gtk::Align::Center);
            away.append(&ic);
            let t = kit::label(&format!("{} is offline", p.peer.name), &["files-away-title"]);
            t.set_xalign(0.5);
            away.append(&t);
            let s = kit::label(&format!("Last seen {}", kit::ago(now_ms(), p.peer.last_seen)), &[]);
            s.set_xalign(0.5);
            away.append(&s);
            self.body.append(&away);
            return String::new();
        }
        let q = self.q();
        let selected = self.sel_first();
        let mut items = 0;
        // Its Home, over Taildrive.
        if q.is_empty() || "home".contains(&q) {
            self.body.append(&kit::label("HOME", &["files-section"]));
            let home = item("folder", "Home", "Over Taildrive", selected.as_deref() == Some("home"));
            home.set_halign(gtk::Align::Start);
            home.set_widget_name("home");
            let (w, id) = (self.me.clone(), peer.to_string());
            let click = gtk::GestureClick::new();
            click.connect_released(move |g, n, _, _| {
                g.set_state(gtk::EventSequenceState::Claimed);
                let Some(m) = w.upgrade() else { return };
                if n >= 2 {
                    m.go(Loc::Folder { peer: id.clone(), path: vec![] });
                } else {
                    m.pick_only("home");
                    m.queue_draw();
                }
            });
            home.add_controller(click);
            self.body.append(&home);
            items += 1;
        }
        // Its wadspaces: Copy here.
        self.body.append(&kit::label("WADSPACES", &["files-section"]));
        let spaces = st.spaces.get(peer).cloned().unwrap_or_default();
        if spaces.at == 0 {
            let why = match &spaces.error {
                Some(e) => format!("{} didn’t answer: {e}", p.peer.name),
                None => format!("Asking {} for its wadspaces…", p.peer.name),
            };
            self.body.append(&kit::label(&why, &["files-away"]));
        } else {
            let list: Vec<_> =
                spaces.list.iter().filter(|x| q.is_empty() || x.name.to_lowercase().contains(&q)).collect();
            if list.is_empty() {
                self.body.append(&kit::label(&format!("No wadspaces on {}.", p.peer.name), &["files-away"]));
            }
            let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
            for x in list {
                col.append(&self.space_row(peer, x, st.copying(peer, &x.id)));
                items += 1;
            }
            self.body.append(&col);
        }
        format!("{items} item{}", if items == 1 { "" } else { "s" })
    }

    fn space_row(&self, peer: &str, x: &wad_model::Wadspace, copying: bool) -> gtk::Box {
        let r = gtk::Box::builder().spacing(12).css_classes(["space-row"]).build();
        r.set_widget_name(&format!("ws-{}", x.id));
        let tile = wad_gtk::WadspaceTile::new(0.8, false);
        tile.set(x, &wad_gtk::TileState::default());
        tile.root.set_valign(gtk::Align::Center);
        r.append(&tile.root);
        let t = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        t.append(&kit::label(&x.name, &["space-name"]));
        let apps: Vec<&str> = x.apps.iter().map(|a| wad_model::catalog::app_name(a.app)).collect();
        if !apps.is_empty() {
            t.append(&kit::label(&apps.join(", "), &["item-sub"]));
        }
        r.append(&t);
        let b = kit::button(if copying { "Copying…" } else { "Copy here" }, Variant::Secondary, Some("download"), true);
        b.set_tooltip_text(Some("Copy it to this machine"));
        b.set_sensitive(!copying);
        b.set_valign(gtk::Align::Center);
        let (w, peer, ws) = (self.me.clone(), peer.to_string(), x.id.clone());
        b.connect_clicked(move |b| {
            b.set_sensitive(false);
            let Some(m) = w.upgrade() else { return };
            let (w, p, s) = (m.me.clone(), peer.clone(), ws.clone());
            let (p2, s2) = (peer.clone(), ws.clone());
            m.link.call(
                move |wadd| async move { wadd.copy_from_peer(&p, &s).await },
                move |r| {
                    let Some(m) = w.upgrade() else { return };
                    match r {
                        Ok(job) => m.state.borrow_mut().started(job, &p2, &s2),
                        Err(e) => m.say(&e.message),
                    }
                    m.queue_draw();
                },
            );
        });
        r.append(&b);
        r
    }

    // ------------------------------------------------------------ a folder

    pub(crate) fn folder_view(&self, peer: &str, path: &[String]) -> String {
        let name = self.state.borrow().peer(peer).map(|p| p.peer.name.clone()).unwrap_or_else(|| "that machine".into());
        let got = self.listing.borrow().got.clone();
        match got {
            None => {
                let b = gtk::Box::builder()
                    .spacing(10)
                    .halign(gtk::Align::Center)
                    .valign(gtk::Align::Center)
                    .vexpand(true)
                    .build();
                let s = gtk::Spinner::builder().spinning(true).build();
                b.append(&s);
                b.append(&kit::label(&format!("Over Taildrive from {name}…"), &["files-away"]));
                self.body.append(&b);
                String::new()
            }
            Some(Err(why)) => {
                let here = path.last().map(String::as_str).unwrap_or("Home");
                self.body.append(&kit::hero("folder-open", &format!("{here} on {name} didn’t open"), &why));
                String::new()
            }
            Some(Ok(list)) => {
                let q = self.q();
                let list: Vec<Entry> =
                    list.into_iter().filter(|e| q.is_empty() || e.display.to_lowercase().contains(&q)).collect();
                if list.is_empty() {
                    let t = if q.is_empty() {
                        "This folder is empty.".to_string()
                    } else {
                        format!("Nothing here matches “{}”.", self.filter.text())
                    };
                    self.body.append(&kit::label(&t, &["files-away"]));
                    return String::new();
                }
                let grid = gtk::FlowBox::builder()
                    .selection_mode(gtk::SelectionMode::None)
                    .min_children_per_line(2)
                    .max_children_per_line(12)
                    .column_spacing(6)
                    .row_spacing(6)
                    .homogeneous(true)
                    .valign(gtk::Align::Start)
                    .build();
                let selected = self.sel_first();
                for e in &list {
                    let sub = if e.folder { String::new() } else { drive::bytes(e.size) };
                    let it = item(e.icon(), &e.display, &sub, selected.as_deref() == Some(e.name.as_str()));
                    it.set_widget_name(&format!("entry-{}", e.name));
                    let click = gtk::GestureClick::new();
                    let (w, en) = (self.me.clone(), e.clone());
                    click.connect_released(move |g, n, _, _| {
                        g.set_state(gtk::EventSequenceState::Claimed);
                        let Some(m) = w.upgrade() else { return };
                        if n >= 2 {
                            m.open_entry(&en);
                        } else {
                            m.pick_only(&en.name);
                            m.queue_draw();
                        }
                    });
                    it.add_controller(click);
                    grid.append(&it);
                }
                self.body.append(&grid);
                format!("{} item{}", list.len(), if list.len() == 1 { "" } else { "s" })
            }
        }
    }
}

pub(crate) fn asks(st: &State) -> Vec<TailnetAsk> {
    st.tailnet.as_ref().filter(|t| t.state == TailnetState::Running).map(|t| t.asks.clone()).unwrap_or_default()
}

fn fact(icon: &'static str, text: &str) -> gtk::Box {
    let b = gtk::Box::builder().spacing(7).css_classes(["device-fact"]).build();
    let ic = Icon::new(icon, 13);
    ic.set_valign(gtk::Align::Center);
    b.append(&ic);
    b.append(&kit::label(text, &[]));
    b
}

fn gone(what: &str) -> gtk::Box {
    kit::hero("network", what, "")
}

pub(crate) fn kind_icon(k: PeerKind) -> &'static str {
    match k {
        PeerKind::Laptop => "laptop",
        PeerKind::Server => "server",
        PeerKind::Desktop => "monitor",
    }
}

fn device_icon(k: wad_proto::DeviceKind) -> &'static str {
    use wad_proto::DeviceKind;
    match k {
        DeviceKind::Phone | DeviceKind::Other => "smartphone",
        DeviceKind::Tablet => "tablet",
        DeviceKind::Laptop => "laptop",
        DeviceKind::Desktop => "monitor",
        DeviceKind::Server => "server",
    }
}

/// Settings ▸ Accounts (its window, if it's open).
pub(crate) fn open_accounts() {
    let args = ["wados-settings", "--page", "accounts"].map(std::ffi::OsStr::new);
    if let Err(e) = gtk::gio::Subprocess::newv(&args, gtk::gio::SubprocessFlags::NONE) {
        tracing::warn!("wados-settings: {e}");
    }
}
