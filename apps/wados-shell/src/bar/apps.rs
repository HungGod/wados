//! The machine's own apps in the desktop's top bar (`MachineApps.svelte`):
//! Settings, Files and WadBrowser, each a launcher that is its own task once
//! open (KDE's icons-only task manager), so their windows never get a second
//! button. Open, a line under it (a dot per window when there's more than
//! one), lit in front. Click: open it; bring its window up; in front
//! already, put it down; with more than one, go round them. Hover for its
//! windows, to pick one or close it. Then the machine's other windows (the
//! terminal, the Builder…), sideways when there's more than fits. Short of
//! room, the apps are their icons alone.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::{glib, prelude::*};
use wad_gtk::{Icon, Svg};

use super::running::display_connectors;
use super::shared::{Shared, Topic};
use super::tasks::{self, Click, MachineApp, in_front};
use super::ui::{self, Entry, Item, entry};
use super::wm;
use crate::shell::Shell;
use crate::wm::{Cmd, Top, Which};

static BROWSER: &[u8] = include_bytes!("../../../../assets/brand/wadbrowser-icon-dark.svg");

struct AppItem {
    app: MachineApp,
    item: Item,
    lines: gtk::Box,
}

pub struct Apps {
    pub root: gtk::Box,
    shell: Rc<Shell>,
    shared: Rc<Shared>,
    apps: Vec<AppItem>,
    others: gtk::Box,
    other_items: RefCell<Vec<(u32, Item)>>,
    sig: RefCell<String>,
    peek: RefCell<Option<(MachineApp, gtk::Popover)>>,
    entering: RefCell<Option<glib::SourceId>>,
    leaving: RefCell<Option<glib::SourceId>>,
    compact: Cell<bool>,
}

impl Apps {
    pub fn new(shell: &Rc<Shell>, shared: &Rc<Shared>) -> Rc<Self> {
        let root =
            gtk::Box::builder().spacing(2).css_classes(["wb-strip"]).hexpand(true).valign(gtk::Align::Center).build();
        let apps: Vec<AppItem> = MachineApp::ALL
            .into_iter()
            .map(|app| {
                let icon: gtk::Widget = match app {
                    MachineApp::Browser => {
                        let b = gtk::Box::builder().css_classes(["wb-glyph", "wb-own"]).homogeneous(true).build();
                        b.append(&Svg::new(BROWSER, 26, 26));
                        b.upcast()
                    }
                    _ => ui::glyph(app.icon(), false).upcast(),
                };
                let item = Item::new(app.name(), &icon);
                let lines = gtk::Box::builder()
                    .spacing(3)
                    .css_classes(["wb-lines"])
                    .halign(gtk::Align::Center)
                    .valign(gtk::Align::End)
                    .visible(false)
                    .build();
                item.over.add_overlay(&lines);
                root.append(&item.button);
                AppItem { app, item, lines }
            })
            .collect();
        let others = gtk::Box::builder().spacing(2).valign(gtk::Align::Center).build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .css_classes(["wb-scroll"])
            .hexpand(true)
            .child(&others)
            .valign(gtk::Align::Center)
            .build();
        root.append(&scroll);
        let me = Rc::new(Self {
            root,
            shell: shell.clone(),
            shared: shared.clone(),
            apps,
            others,
            other_items: RefCell::default(),
            sig: RefCell::default(),
            peek: RefCell::default(),
            entering: RefCell::default(),
            leaving: RefCell::default(),
            compact: Cell::new(false),
        });
        for a in &me.apps {
            let (w, app) = (Rc::downgrade(&me), a.app);
            a.item.button.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.click_app(app);
                }
            });
            let motion = gtk::EventControllerMotion::new();
            let (w, app) = (Rc::downgrade(&me), a.app);
            motion.connect_enter(move |_, _, _| {
                if let Some(m) = w.upgrade() {
                    m.hover(Some(app));
                }
            });
            let w = Rc::downgrade(&me);
            motion.connect_leave(move |_| {
                if let Some(m) = w.upgrade() {
                    m.hover(None);
                }
            });
            a.item.button.add_controller(motion);
        }
        let w = Rc::downgrade(&me);
        shared.on(Topic::Windows, move || {
            if let Some(m) = w.upgrade() {
                m.fill();
            }
        });
        me.fill();
        me
    }

    fn grouped(&self) -> tasks::Tasks {
        tasks::group(&self.shell.placer.windows(), &self.shared.recent.borrow())
    }

    /// Icons alone when the names would leave the other windows no room.
    pub fn fit(&self) {
        let Some(parent) = self.root.parent() else { return };
        let room = parent.width_request();
        let running = self.root.prev_sibling().map(|p| crate::machine::natural(&p).0 as i32).unwrap_or(0);
        let mine = room - running - 10;
        let compact = mine > 0 && mine < (MachineApp::ALL.len() as i32) * 74 + 160;
        if self.compact.replace(compact) != compact {
            for a in &self.apps {
                a.item.set_compact(compact);
            }
            for (_, i) in self.other_items.borrow().iter() {
                i.set_compact(compact);
            }
        }
    }

    fn fill(self: &Rc<Self>) {
        let t = self.grouped();
        let outputs = display_connectors(&self.shell);
        let sig = format!(
            "{:?}",
            (
                t.apps
                    .iter()
                    .map(|(a, w)| (*a, w.iter().map(|x| (x.id, in_front(x))).collect::<Vec<_>>()))
                    .collect::<Vec<_>>(),
                t.others
                    .iter()
                    .map(|x| (x.id, x.title.clone(), in_front(x), x.minimized, x.outputs.clone()))
                    .collect::<Vec<_>>()
            )
        );
        if *self.sig.borrow() == sig {
            return;
        }
        *self.sig.borrow_mut() = sig;
        for a in &self.apps {
            let mine = t.apps.iter().find(|(x, _)| *x == a.app).map(|(_, w)| w.clone()).unwrap_or_default();
            while let Some(c) = a.lines.first_child() {
                a.lines.remove(&c);
            }
            for _ in mine.iter().take(3) {
                a.lines.append(&gtk::Box::builder().css_classes(["wb-line"]).build());
            }
            if mine.len() == 1
                && let Some(l) = a.lines.first_child()
            {
                l.add_css_class("wb-only");
            }
            a.lines.set_visible(!mine.is_empty());
            let front = mine.iter().any(in_front);
            ui::set_class(&a.lines, "wb-lit", front);
            a.item.set_active(front);
            a.item.button.set_tooltip_text(if mine.is_empty() { Some(a.app.name()) } else { None });
        }
        // The peek follows its app's windows (gone with the last of them).
        let peeking = self.peek.borrow().as_ref().map(|(a, _)| *a);
        if let Some(app) = peeking {
            let left = t.apps.iter().find(|(x, _)| *x == app).is_some_and(|(_, w)| !w.is_empty());
            if left {
                self.show_peek(app);
            } else {
                self.close_peek();
            }
        }

        while let Some(c) = self.others.first_child() {
            self.others.remove(&c);
        }
        let mut items = vec![];
        for w in &t.others {
            let icon = ui::glyph(tasks::glyph(&w.app_id), false);
            let item = Item::new(&tasks::title(w), &icon);
            item.set_active(in_front(w));
            item.set_dim(w.minimized);
            item.set_compact(self.compact.get());
            if w.minimized {
                item.button.set_tooltip_text(Some(&format!("{} (minimized)", tasks::title(w))));
            }
            if let Some(n) =
                w.outputs.first().and_then(|o| outputs.iter().position(|c| c == o)).filter(|i| *i > 0).map(|i| i + 1)
            {
                item.over.add_overlay(&ui::display_number(n));
            }
            let (s, win) = (self.shell.clone(), w.clone());
            item.button.connect_clicked(move |_| {
                let which = Which(format!("#{}", win.id));
                wm(&s, if in_front(&win) { Cmd::Minimize(which) } else { Cmd::Activate(which) });
            });
            let (me, id) = (Rc::downgrade(self), w.id);
            ui::on_right_click(&item.button, move |_, _| {
                if let Some(m) = me.upgrade() {
                    m.window_menu(id);
                }
            });
            self.others.append(&item.button);
            items.push((w.id, item));
        }
        *self.other_items.borrow_mut() = items;
        self.fit();
    }

    fn click_app(&self, app: MachineApp) {
        self.close_peek();
        let mine = self.grouped().apps.into_iter().find(|(a, _)| *a == app).map(|(_, w)| w).unwrap_or_default();
        match tasks::click(&mine) {
            Click::Launch => launch(&self.shell, app),
            Click::Activate(id) => wm(&self.shell, Cmd::Activate(Which(format!("#{id}")))),
            Click::Minimize(id) => wm(&self.shell, Cmd::Minimize(Which(format!("#{id}")))),
        }
    }

    fn window_menu(&self, id: u32) -> bool {
        let items = self.other_items.borrow();
        let Some((_, item)) = items.iter().find(|(x, _)| *x == id) else { return false };
        let Some(t) = self.shell.placer.windows().into_iter().find(|t| t.id == id) else { return false };
        let which = move || Which(format!("#{id}"));
        let (s1, s2) = (self.shell.clone(), self.shell.clone());
        let first = if t.minimized {
            entry("Show it", "app-window", move || wm(&s1, Cmd::Activate(which())))
        } else {
            entry("Minimize", "minus", move || wm(&s1, Cmd::Minimize(which())))
        };
        ui::menu(
            &self.root,
            &item.button,
            vec![first, Entry::Sep, entry("Close", "x", move || wm(&s2, Cmd::Close(which()))).danger()],
            true,
        );
        true
    }

    // ------------------------------------------------------------ hovering: its windows

    fn hover(self: &Rc<Self>, app: Option<MachineApp>) {
        if let Some(id) = self.entering.borrow_mut().take() {
            id.remove();
        }
        match app {
            Some(app) => {
                if let Some(id) = self.leaving.borrow_mut().take() {
                    id.remove();
                }
                let open = self.peek.borrow().is_some();
                let w = Rc::downgrade(self);
                let id = glib::timeout_add_local_once(Duration::from_millis(if open { 0 } else { 280 }), move || {
                    if let Some(m) = w.upgrade() {
                        m.entering.borrow_mut().take();
                        m.show_peek(app);
                    }
                });
                *self.entering.borrow_mut() = Some(id);
            }
            None => self.leave_soon(),
        }
    }

    fn leave_soon(self: &Rc<Self>) {
        if let Some(id) = self.leaving.borrow_mut().take() {
            id.remove();
        }
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(220), move || {
            if let Some(m) = w.upgrade() {
                m.leaving.borrow_mut().take();
                m.close_peek();
            }
        });
        *self.leaving.borrow_mut() = Some(id);
    }

    fn keep(&self) {
        if let Some(id) = self.leaving.borrow_mut().take() {
            id.remove();
        }
    }

    pub fn peek(self: &Rc<Self>, key: &str) -> bool {
        let Some(app) = MachineApp::ALL.into_iter().find(|a| a.key() == key) else { return false };
        self.show_peek(app)
    }

    fn show_peek(self: &Rc<Self>, app: MachineApp) -> bool {
        let mine = self.grouped().apps.into_iter().find(|(a, _)| *a == app).map(|(_, w)| w).unwrap_or_default();
        if mine.is_empty() {
            self.close_peek();
            return false;
        }
        let Some(a) = self.apps.iter().find(|a| a.app == app) else { return false };
        let pop = match self.peek.borrow().as_ref() {
            Some((was, p)) if *was == app => Some(p.clone()),
            _ => None,
        };
        let pop = pop.unwrap_or_else(|| {
            self.close_peek();
            let p = ui::popover(&self.root, &["wb-peek"], true);
            p.set_autohide(false);
            let motion = gtk::EventControllerMotion::new();
            let w = Rc::downgrade(self);
            motion.connect_enter(move |_, _, _| {
                if let Some(m) = w.upgrade() {
                    m.keep();
                }
            });
            let w = Rc::downgrade(self);
            motion.connect_leave(move |_| {
                if let Some(m) = w.upgrade() {
                    m.leave_soon();
                }
            });
            p.add_controller(motion);
            *self.peek.borrow_mut() = Some((app, p.clone()));
            p
        });
        let row = gtk::Box::builder().spacing(8).build();
        for t in &mine {
            row.append(&self.peek_card(t));
        }
        pop.set_child(Some(&row));
        ui::point_at(&pop, &a.item.button);
        pop.popup();
        true
    }

    fn peek_card(self: &Rc<Self>, t: &Top) -> gtk::Widget {
        let card = gtk::Overlay::new();
        let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
        let shot = gtk::Box::builder().css_classes(["wb-shot"]).homogeneous(true).build();
        shot.append(&Icon::new(tasks::glyph(&t.app_id), 22));
        col.append(&shot);
        let title = gtk::Label::builder()
            .label(tasks::title(t))
            .css_classes(["wb-pick-title"])
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(18)
            .margin_end(22)
            .build();
        col.append(&title);
        let pick = gtk::Button::builder().child(&col).css_classes(["wb-pick"]).build();
        ui::set_class(&pick, "wb-front", in_front(t));
        let (w, id) = (Rc::downgrade(self), t.id);
        pick.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.close_peek();
                wm(&m.shell, Cmd::Activate(Which(format!("#{id}"))));
            }
        });
        card.set_child(Some(&pick));
        if t.minimized {
            card.add_overlay(
                &gtk::Label::builder()
                    .label("Minimized")
                    .css_classes(["wb-min-tag"])
                    .halign(gtk::Align::Start)
                    .valign(gtk::Align::Start)
                    .build(),
            );
        }
        let close = gtk::Button::builder()
            .child(&Icon::new("x", 13))
            .css_classes(["wb-close"])
            .tooltip_text("Close it")
            .halign(gtk::Align::End)
            .valign(gtk::Align::End)
            .build();
        let s = self.shell.clone();
        close.connect_clicked(move |_| wm(&s, Cmd::Close(Which(format!("#{id}")))));
        card.add_overlay(&close);
        card.upcast()
    }

    fn close_peek(&self) {
        if let Some((_, p)) = self.peek.borrow_mut().take() {
            p.popdown();
            glib::idle_add_local_once(move || p.unparent());
        }
    }

    // ------------------------------------------------------------ wadsctl

    pub fn menu(&self, what: &str) -> bool {
        let id = what.trim_start_matches('#').parse::<u32>().ok();
        id.is_some_and(|id| self.window_menu(id))
    }

    pub fn click(&self, what: &str) -> bool {
        if let Some(a) = self.apps.iter().find(|a| a.app.key() == what) {
            a.item.button.emit_clicked();
            return true;
        }
        let id = what.trim_start_matches('#').parse::<u32>().ok();
        let items = self.other_items.borrow();
        match items.iter().find(|(x, _)| Some(*x) == id) {
            Some((_, i)) => {
                i.button.emit_clicked();
                true
            }
            None => false,
        }
    }

    pub fn describe(&self) -> String {
        let t = self.grouped();
        let apps: Vec<String> = t
            .apps
            .iter()
            .map(|(a, w)| format!("{}:{}{}", a.key(), w.len(), if w.iter().any(in_front) { "*" } else { "" }))
            .collect();
        let others: Vec<String> = t
            .others
            .iter()
            .map(|w| format!("#{}{}{}", w.id, if in_front(w) { "*" } else { "" }, if w.minimized { "_" } else { "" }))
            .collect();
        format!("apps=[{}] windows=[{}]", apps.join(","), others.join(","))
    }
}

/// Starts a machine app, or says when it comes.
pub fn launch(shell: &Shell, app: MachineApp) {
    let (argv, phase) = app.launch();
    if !ui::spawn(argv) {
        shell.toast(wad_proto::Level::Info, &format!("{} comes in phase {phase}", app.name()), None);
    }
}
