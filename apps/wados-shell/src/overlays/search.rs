//! Alt+Space, from anywhere (`shell/search/VariantSearch.svelte`): every
//! app's variants in this machine's wadspaces (Gmail in WadBrowser, Mario
//! Kart in Dolphin, Claude Code in VS Code), found by name
//! (`wad_model::variants::search`). ↑/↓ and Enter; Esc (or Alt+Space again)
//! closes.
//!
//! Enter opens the variant's wadspace (or brings it up if it's running).
//! Opening it straight into that app, on its wallpaper, needs wadd to start
//! a session with a window first: that comes later.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{KeyboardMode, Layer};
use wad_gtk::Icon;
use wad_model::catalog::app_name;
use wad_model::variants::{self, VariantKind};
use wad_model::{AppId, SessionDisplay, Wadspace};

use crate::shell::Shell;
use crate::surface::{self, Anchor, Spec};

/// The most it lists.
const LIMIT: usize = 60;

/// One result, as its row shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub ws: String,
    pub ws_name: String,
    pub variant: String,
    pub name: String,
    /// "App", or "Web app · WadBrowser".
    pub kind: String,
    pub app: AppId,
    /// A web app's site: its id, name and address (for its icon).
    pub web: Option<(String, Option<String>, Option<String>)>,
    pub wallpaper: Option<wad_model::Wallpaper>,
}

/// What `q` finds among `list`'s variants on `machine`, in search's order.
pub fn rows(list: &[Wadspace], machine: &str, q: &str) -> Vec<Row> {
    variants::search(list, machine, q, LIMIT)
        .into_iter()
        .map(|f| {
            let v = &f.variant;
            Row {
                ws: f.ws.id.clone(),
                ws_name: f.ws.name.clone(),
                variant: v.id.clone(),
                name: v.name.clone(),
                kind: if v.kind == VariantKind::App {
                    "App".into()
                } else {
                    format!("{} · {}", v.kind.label(), app_name(v.app))
                },
                app: v.app,
                web: (v.kind == VariantKind::Web)
                    .then(|| v.ext.as_ref().map(|e| (e.id.clone(), Some(v.name.clone()), e.url.clone())))
                    .flatten(),
                wallpaper: variants::wallpaper_for(f.ws, v).cloned(),
            }
        })
        .collect()
}

type OnOpen = Box<dyn Fn(&Row)>;

pub struct Search {
    shell: Rc<Shell>,
    win: gtk::ApplicationWindow,
    text: gtk::Text,
    list: gtk::Box,
    scroll: gtk::ScrolledWindow,
    rows: RefCell<Vec<Row>>,
    buttons: RefCell<Vec<gtk::Button>>,
    at: Cell<usize>,
    on_open: RefCell<Option<OnOpen>>,
}

impl Search {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let win = surface::make(
            &shell.app,
            &shell.primary,
            &Spec {
                namespace: "wados-search",
                layer: Layer::Overlay,
                anchor: Anchor::Fill,
                exclusive: -1,
                keyboard: KeyboardMode::Exclusive,
                height: 0,
            },
        );
        let (_, h) = shell.size();
        let outer = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        let bx = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["ov-search"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Start)
            .margin_top((h as f64 * 0.14) as i32)
            .build();
        bx.set_size_request(620, -1);
        let field = gtk::Box::builder().spacing(12).css_classes(["ov-field"]).build();
        field.append(&Icon::new("search", 18));
        let text = gtk::Text::builder()
            .placeholder_text("Search apps and their variants")
            .hexpand(true)
            .css_classes(["ov-q"])
            .build();
        field.append(&text);
        field.append(&gtk::Label::builder().label("Alt Space").css_classes(["ov-kbd"]).build());
        bx.append(&field);
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["ov-results"]).build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height((h as f64 * 0.64) as i32 - 110)
            .child(&list)
            .build();
        bx.append(&scroll);
        let foot = gtk::Box::builder().spacing(16).css_classes(["ov-foot"]).build();
        for t in ["↑↓ to move", "Enter opens its wadspace", "Esc closes"] {
            foot.append(&gtk::Label::new(Some(t)));
        }
        bx.append(&foot);
        outer.append(&bx);
        win.set_child(Some(&outer));
        let me = Rc::new(Self {
            shell: shell.clone(),
            win,
            text,
            list,
            scroll,
            rows: RefCell::default(),
            buttons: RefCell::default(),
            at: Cell::new(0),
            on_open: RefCell::default(),
        });
        let w = Rc::downgrade(&me);
        me.text.connect_changed(move |_| {
            if let Some(m) = w.upgrade() {
                m.at.set(0);
                m.fill();
            }
        });
        // A click outside the box closes it.
        let click = gtk::GestureClick::new();
        let (w, b) = (Rc::downgrade(&me), bx.downgrade());
        click.connect_pressed(move |g, _, x, y| {
            let (Some(m), Some(b)) = (w.upgrade(), b.upgrade()) else { return };
            let Some(win) = g.widget() else { return };
            let inside = b
                .compute_bounds(&win)
                .is_some_and(|r| r.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)));
            if !inside {
                m.close();
            }
        });
        me.win.add_controller(click);
        me.keys();
        me
    }

    pub fn on_open(&self, f: impl Fn(&Row) + 'static) {
        *self.on_open.borrow_mut() = Some(Box::new(f));
    }

    pub fn is_open(&self) -> bool {
        self.win.is_visible()
    }

    /// Alt+Space: open it (or close it, open).
    pub fn toggle(self: &Rc<Self>) {
        if self.is_open() {
            self.close();
        } else {
            self.open();
        }
    }

    pub fn open(self: &Rc<Self>) {
        self.text.set_text("");
        self.at.set(0);
        self.fill();
        self.win.present();
        self.text.grab_focus();
    }

    pub fn close(&self) {
        self.win.set_visible(false);
    }

    /// For tests: type `q` into it.
    pub fn set_query(self: &Rc<Self>, q: &str) {
        if !self.is_open() {
            self.open();
        }
        self.text.set_text(q);
    }

    pub fn state(&self) -> String {
        if !self.is_open() {
            return "ok closed".into();
        }
        let rows = self.rows.borrow();
        let first: Vec<String> = rows.iter().take(5).map(|r| format!("{}/{}", r.ws, r.variant)).collect();
        format!(
            "ok open q={:?} results={} at={} first=[{}]",
            self.text.text().as_str(),
            rows.len(),
            self.at.get(),
            first.join(",")
        )
    }

    fn fill(self: &Rc<Self>) {
        let rows = {
            let lib = self.shell.store.library();
            rows(&lib.wadspaces, &self.shell.store.machine_id(), &self.text.text())
        };
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let mut buttons = vec![];
        for (i, r) in rows.iter().enumerate() {
            let b = self.row(r, i);
            self.list.append(&b);
            buttons.push(b);
        }
        if rows.is_empty() {
            let q = self.text.text();
            let msg = if self.shell.store.live().is_empty() {
                "No wadspaces on this machine yet.".to_string()
            } else {
                format!("Nothing matches \u{201c}{q}\u{201d}.")
            };
            self.list.append(&gtk::Label::builder().label(msg).xalign(0.0).css_classes(["ov-none"]).build());
        }
        *self.rows.borrow_mut() = rows;
        *self.buttons.borrow_mut() = buttons;
        self.mark();
    }

    fn row(self: &Rc<Self>, r: &Row, i: usize) -> gtk::Button {
        let line = gtk::Box::builder().spacing(12).build();
        let icon = match &r.web {
            Some((id, name, url)) => wad_gtk::app_icon::site(id, name.as_deref(), url.as_deref(), 30),
            None => wad_gtk::app_icon::app(r.app, 30),
        };
        icon.set_valign(gtk::Align::Center);
        // Its card expands inside; the row's icon doesn't.
        icon.set_hexpand(false);
        line.append(&icon);
        let t = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        t.append(
            &gtk::Label::builder()
                .label(&r.name)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["ov-r-name"])
                .build(),
        );
        t.append(&gtk::Label::builder().label(&r.kind).xalign(0.0).css_classes(["ov-r-kind"]).build());
        line.append(&t);
        let ws = gtk::Box::builder().spacing(7).css_classes(["ov-r-ws"]).valign(gtk::Align::Center).build();
        if let Some(wp) = r.wallpaper.as_ref().filter(|w| wad_model::wallpapers::has_picture(w)) {
            let p = wad_gtk::Picture::new(wp, 5.0);
            p.set_size_request(18, 18);
            p.set_valign(gtk::Align::Center);
            ws.append(&p);
        }
        ws.append(&gtk::Label::new(Some(&r.ws_name)));
        if self.shell.store.is_running(&r.ws) {
            let dot =
                gtk::Box::builder().css_classes(["ov-dot"]).valign(gtk::Align::Center).tooltip_text("Running").build();
            ws.append(&dot);
        }
        line.append(&ws);
        let b = gtk::Button::builder().child(&line).css_classes(["ov-r"]).build();
        let w = Rc::downgrade(self);
        b.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.at.set(i);
                m.enter();
            }
        });
        let motion = gtk::EventControllerMotion::new();
        let w = Rc::downgrade(self);
        motion.connect_enter(move |_, _, _| {
            if let Some(m) = w.upgrade() {
                m.at.set(i);
                m.mark();
            }
        });
        b.add_controller(motion);
        b
    }

    fn mark(&self) {
        let at = self.at.get();
        for (i, b) in self.buttons.borrow().iter().enumerate() {
            if i == at {
                b.add_css_class("ov-on");
            } else {
                b.remove_css_class("ov-on");
            }
        }
    }

    fn step(&self, by: i32) {
        let n = self.rows.borrow().len();
        if n == 0 {
            return;
        }
        let at = (self.at.get() as i32 + by).clamp(0, n as i32 - 1) as usize;
        self.at.set(at);
        self.mark();
        // Keep it in view.
        if let Some(b) = self.buttons.borrow().get(at)
            && let Some(r) = b.compute_bounds(&self.list)
        {
            let adj = self.scroll.vadjustment();
            let (top, bottom) = (r.y() as f64, (r.y() + r.height()) as f64);
            if top < adj.value() {
                adj.set_value(top);
            } else if bottom > adj.value() + adj.page_size() {
                adj.set_value(bottom - adj.page_size());
            }
        }
    }

    /// Enter: the one the keys are on.
    pub fn enter(&self) {
        let Some(r) = self.rows.borrow().get(self.at.get()).cloned() else { return };
        self.close();
        if let Some(f) = self.on_open.borrow().as_ref() {
            f(&r);
        }
        if self.shell.store.is_running(&r.ws) {
            self.shell.show(&r.ws);
        } else {
            self.shell.open(&r.ws, SessionDisplay::Screen);
        }
    }

    fn keys(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(m) = w.upgrade() else { return glib::Propagation::Proceed };
            match key {
                gdk::Key::Down => m.step(1),
                gdk::Key::Up => m.step(-1),
                gdk::Key::Page_Down => m.step(8),
                gdk::Key::Page_Up => m.step(-8),
                gdk::Key::Return | gdk::Key::KP_Enter => m.enter(),
                gdk::Key::Escape => m.close(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.win.add_controller(keys);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_model::{AppIn, Ext};

    fn ws(id: &str, name: &str, order: i64, apps: &[AppId], exts: Vec<Ext>) -> Wadspace {
        let mut w = wad_model::starter::starter_wadspace("m-here", 0);
        w.id = id.into();
        w.name = name.into();
        w.order = order;
        w.starter = false;
        w.apps = apps.iter().map(|a| AppIn::new(*a)).collect();
        w.exts = exts;
        w.commands.clear();
        w
    }

    fn web(id: &str, name: &str) -> Ext {
        let mut e = Ext::new(AppId::Wadbrowser, id).named(name);
        e.url = Some(format!("https://{id}.example"));
        e
    }

    fn library() -> Vec<Wadspace> {
        let mut away = ws("away", "Away", 0, &[AppId::Wadbrowser], vec![web("gmail", "Gmail")]);
        away.machine = "m-elsewhere".into();
        vec![
            ws(
                "work",
                "Work",
                2,
                &[AppId::Wadbrowser, AppId::Vscode],
                vec![web("gmail", "Gmail"), web("github", "GitHub")],
            ),
            ws("mail", "Mail", 1, &[AppId::Wadbrowser], vec![web("gmail-work", "Work Gmail")]),
            away,
        ]
    }

    #[test]
    fn names_starting_with_it_come_first_then_words_then_the_rest() {
        let lib = library();
        let r = rows(&lib, "m-here", "gm");
        let names: Vec<(&str, &str)> = r.iter().map(|r| (r.ws.as_str(), r.name.as_str())).collect();
        // "Gmail" starts with it; "Work Gmail" has a word that does. Another machine's never shows.
        assert_eq!(names, [("work", "Gmail"), ("mail", "Work Gmail")]);
        assert_eq!(r[0].kind, "Web app · WadBrowser");
        assert!(r[0].web.as_ref().is_some_and(|(id, _, _)| id == "gmail"));
    }

    #[test]
    fn an_app_is_found_by_its_name_and_its_wadspace() {
        let lib = library();
        let r = rows(&lib, "m-here", "code");
        assert!(r.iter().any(|r| r.ws == "work" && r.kind == "App" && r.app == AppId::Vscode));
        // By the wadspace's name: everything in it, after anything named so
        // (those that contain it as well as each other, in the wadspaces' order).
        let r = rows(&lib, "m-here", "mail");
        let names: Vec<(&str, &str)> = r.iter().map(|r| (r.ws.as_str(), r.name.as_str())).collect();
        assert_eq!(names[..2], [("mail", "Work Gmail"), ("work", "Gmail")]);
        let app = r.iter().position(|r| r.ws == "mail" && r.app == AppId::Wadbrowser && r.kind == "App");
        assert!(app.is_some_and(|i| i >= 2));
    }

    #[test]
    fn nothing_typed_lists_everything_in_order() {
        let lib = library();
        let r = rows(&lib, "m-here", "");
        // The wadspaces in their order (Mail, then Work), each's apps and their variants.
        assert_eq!(r.first().map(|r| r.ws.as_str()), Some("mail"));
        assert!(r.iter().all(|r| r.ws != "away"));
        assert!(r.len() <= LIMIT);
    }
}
