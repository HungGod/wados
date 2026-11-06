//! Every display that isn't the primary gets its own bar along its top
//! (`shell/display/ExtraDisplay.svelte`): its number and name, what's
//! running on it (its wadspaces and the machine's windows there), and the
//! time. The launcher, the tray and the desktop's icons stay on the primary.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{KeyboardMode, Layer};

use super::shared::{self, Topic};
use super::tasks::{self, in_front};
use super::ui::{self, Item};
use super::wm;
use crate::machine::BAR;
use crate::shell::Shell;
use crate::surface::{self, Anchor, Spec};
use crate::wm::{Cmd, Top, Which};

pub struct Displays {
    bars: Vec<Rc<DisplayBar>>,
}

struct DisplayBar {
    shell: Rc<Shell>,
    _win: gtk::ApplicationWindow,
    connector: String,
    apps: gtk::Box,
    time: gtk::Label,
    sig: RefCell<String>,
}

impl Displays {
    pub fn new(shell: &Rc<Shell>) -> Self {
        super::style::install();
        let bars = shell.others.iter().enumerate().map(|(i, m)| DisplayBar::new(shell, m, i + 2)).collect();
        Self { bars }
    }

    /// Its bars gone (the displays changed: new ones are made).
    pub fn close(&self) {
        for b in &self.bars {
            b._win.destroy();
        }
    }

    /// What each shows, for `wadsctl displays`.
    pub fn describe(&self) -> String {
        self.bars.iter().map(|b| b.describe()).collect::<Vec<_>>().join(" ; ")
    }
}

/// A display's name: its description, model or connector.
pub fn name_of(m: &gdk::Monitor) -> String {
    let pick = |s: Option<glib::GString>| s.map(|s| s.to_string()).filter(|s| !s.trim().is_empty());
    let known = |s: &String| !matches!(s.to_ascii_lowercase().as_str(), "unknown" | "headless" | "virtual");
    pick(m.model())
        .filter(known)
        .or_else(|| pick(m.description()).filter(known))
        .or_else(|| pick(m.connector()))
        .unwrap_or_else(|| "Display".into())
}

impl DisplayBar {
    fn new(shell: &Rc<Shell>, monitor: &gdk::Monitor, n: usize) -> Rc<Self> {
        let win = surface::make(
            &shell.app,
            monitor,
            &Spec {
                namespace: "wados-display-bar",
                layer: Layer::Top,
                anchor: Anchor::Top,
                exclusive: BAR,
                // As the bar: no keys of its own (its popovers grab them).
                keyboard: KeyboardMode::None,
                height: BAR,
            },
        );
        surface::stay(&win);
        let row = gtk::Box::builder().spacing(10).margin_start(10).margin_end(10).valign(gtk::Align::Center).build();
        let who = gtk::Box::builder()
            .spacing(9)
            .css_classes(["wb-glass", "wb-who"])
            .tooltip_text(format!("Display {n}"))
            .build();
        who.append(
            &gtk::Label::builder().label(n.to_string()).css_classes(["wb-n"]).valign(gtk::Align::Center).build(),
        );
        who.append(
            &gtk::Label::builder().label(name_of(monitor)).css_classes(["wb-h3"]).valign(gtk::Align::Center).build(),
        );
        row.append(&who);
        let apps = gtk::Box::builder().spacing(2).css_classes(["wb-glass", "wb-apps"]).hexpand(true).build();
        row.append(&apps);
        let time = gtk::Label::builder().css_classes(["wb-time"]).build();
        let tbox = gtk::Box::builder().css_classes(["wb-glass"]).build();
        tbox.append(&time);
        row.append(&tbox);
        win.set_child(Some(&row));
        let me = Rc::new(Self {
            shell: shell.clone(),
            _win: win.clone(),
            connector: monitor.connector().map(|c| c.to_string()).unwrap_or_default(),
            apps,
            time,
            sig: RefCell::default(),
        });
        let shared = shared::get(shell);
        let w = Rc::downgrade(&me);
        shared.on(Topic::Windows, move || {
            if let Some(m) = w.upgrade() {
                m.fill();
            }
        });
        let w = Rc::downgrade(&me);
        shell.store.watch(move |_| {
            if let Some(m) = w.upgrade() {
                m.fill();
            }
        });
        let w = Rc::downgrade(&me);
        let tick = move || {
            let Some(m) = w.upgrade() else { return glib::ControlFlow::Break };
            // Settings ▸ System ▸ Date & Time's format.
            let f = if m.shell.desk.settings().clock.h24 { "%H:%M" } else { "%-I:%M %P" };
            if let Ok(t) = glib::DateTime::now_local().and_then(|n| n.format(f)) {
                m.time.set_label(&t);
            }
            glib::ControlFlow::Continue
        };
        tick();
        glib::timeout_add_seconds_local(5, tick);
        me.fill();
        win.present();
        me
    }

    /// Its windows: a wadspace's, or the machine's.
    fn here(&self) -> Vec<Top> {
        self.shell.placer.windows().into_iter().filter(|t| t.outputs.contains(&self.connector)).collect()
    }

    fn fill(&self) {
        let here = self.here();
        let sig = format!(
            "{:?}",
            here.iter()
                .map(|t| (
                    t.id,
                    t.title.clone(),
                    in_front(t),
                    t.minimized,
                    self.shell.store.tile_state(t.wadspace().unwrap_or(""))
                ))
                .collect::<Vec<_>>()
        );
        if *self.sig.borrow() == sig {
            return;
        }
        *self.sig.borrow_mut() = sig;
        while let Some(c) = self.apps.first_child() {
            self.apps.remove(&c);
        }
        let (spaces, windows): (Vec<Top>, Vec<Top>) = here.into_iter().partition(|t| t.wadspace().is_some());
        for t in &spaces {
            let Some(ws) = t.wadspace().and_then(|id| self.shell.store.wadspace(id)) else { continue };
            let tile = wad_gtk::WadspaceTile::new(0.6, false);
            tile.set(&ws, &self.shell.store.tile_state(&ws.id));
            let item = Item::new(&ws.name, &tile.root);
            item.set_active(in_front(t));
            let (s, id) = (self.shell.clone(), ws.id.clone());
            item.button.connect_clicked(move |_| s.show(&id));
            self.apps.append(&item.button);
        }
        if !spaces.is_empty() && !windows.is_empty() {
            self.apps.append(&gtk::Box::builder().css_classes(["wb-rule"]).valign(gtk::Align::Center).build());
        }
        for t in &windows {
            let item = Item::new(&tasks::title(t), &ui::glyph(tasks::glyph(&t.app_id), false));
            item.set_active(in_front(t));
            item.set_dim(t.minimized);
            let (s, t) = (self.shell.clone(), t.clone());
            item.button.connect_clicked(move |_| {
                let which = Which(format!("#{}", t.id));
                wm(&s, if in_front(&t) { Cmd::Minimize(which) } else { Cmd::Activate(which) });
            });
            self.apps.append(&item.button);
        }
        if self.apps.first_child().is_none() {
            self.apps.append(&gtk::Label::builder().label("Nothing on this display").css_classes(["wb-none"]).build());
        }
    }

    fn describe(&self) -> String {
        let here: Vec<String> = self
            .here()
            .iter()
            .map(|t| t.wadspace().map(str::to_string).unwrap_or_else(|| format!("#{}", t.id)))
            .collect();
        format!("{} [{}] {}", self.connector, here.join(","), self.time.label())
    }
}
