//! wados-settings: WadOS's Settings (the spike's `apps/settings`), after
//! GNOME Settings: a sidebar of panels (with a search) beside the one open.
//! Each panel is a module of `panels/`, in the registry there; one not
//! built yet says so. A panel's info (its name, icon, search words, other
//! names) is `nav.rs`'s; what it's given is `panel.rs`'s `Ctx`.
//!
//!     wados-settings [--page <panel or another name for it>] [--dump]
//!
//! One window: a second start (the tray's Tailscale tile, the desktop's
//! Tailscale settings) brings it up at the page it names (`--page updates`
//! is System's Updates). `--dump` prints what the page says once it has
//! wadd's state, and quits (tests).

mod link;
mod nav;
mod panel;
mod panels;
mod parts;

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::{gio, glib, prelude::*};

use panel::{Ctx, Panel};

const APP_ID: &str = "io.wados.Settings";

const CSS: &str = r#"
window.set-window { background-color: var(--surface); }
.set-side { background-color: var(--bg-2); border-right: 1px solid var(--line); min-width: 250px; }
.set-side-head { min-height: 52px; padding: 0 18px; }
.set-side-title { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 15px; }
.set-search { margin: 2px 12px 10px; min-height: 32px; padding: 0 6px 0 10px; border-radius: 10px; border: 1px solid var(--line); background-color: var(--surface); color: var(--faint); }
.set-search:focus-within { border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-soft); }
.set-search > text { background: none; color: var(--fg); caret-color: var(--fg); font-size: 13.5px; }
.set-search > text > placeholder { color: var(--faint); }
.set-search > text > selection { background-color: var(--accent-soft); color: var(--fg); }
button.set-clear { min-width: 20px; min-height: 20px; padding: 0; border-radius: 50%; border: none; box-shadow: none; background-image: none; background-color: var(--surface-3); color: var(--muted); }
.set-nav { padding: 0 10px 14px; }
button.set-item { min-height: 40px; padding: 0 12px; border-radius: 10px; border: none; box-shadow: none; outline: none; background: none; background-image: none; text-shadow: none; color: var(--fg); font-weight: 400; font-size: 14px; }
button.set-item:hover { background-color: var(--surface-2); }
button.set-item wad-icon { color: var(--muted); }
button.set-item.on { background-color: var(--surface-3); box-shadow: inset 0 0 0 1px var(--line); }
button.set-item.on wad-icon { color: var(--accent); }
.set-sep { min-height: 1px; margin: 6px 12px; background-color: var(--line); }
.set-none { margin: 12px; font-size: 13px; color: var(--faint); }
.set-code { padding: 10px 12px; border-radius: 10px; background-color: var(--bg-2); border: 1px solid var(--line); font-size: 12px; color: var(--fg); }
"#;

/// What it was started with.
#[derive(Debug, Default, PartialEq)]
struct Args {
    page: Option<String>,
    dump: bool,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = args.iter().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--page" => a.page = Some(it.next().ok_or("--page needs a page (accounts, updates, …)")?.clone()),
            "--dump" => a.dump = true,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(a)
}

/// One window (a second start hands it its arguments), but `--dump` runs on its own.
fn flags(argv: &[String]) -> gio::ApplicationFlags {
    let mut f = gio::ApplicationFlags::HANDLES_COMMAND_LINE;
    if argv.iter().any(|a| a == "--dump") {
        f |= gio::ApplicationFlags::NON_UNIQUE;
    }
    f
}

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();
    let argv: Vec<String> = std::env::args().collect();
    match argv.get(1).map(String::as_str) {
        Some("--version") => {
            println!("wados-settings {}", env!("CARGO_PKG_VERSION"));
            return glib::ExitCode::SUCCESS;
        }
        // Lock Screen & Privacy's password change without the window (the
        // VM's check): the current password, then the new one, a line each on stdin.
        Some("--change-password") => return panels::lock::change_from_stdin(),
        // Power's mode switch without the window (the VM's check).
        Some("--power-mode") => return panels::power::mode_from_cli(argv.get(2).map(String::as_str)),
        Some("--help" | "-h") => {
            let ids: Vec<_> = nav::panels().map(|p| p.id).collect();
            println!(
                "usage: wados-settings [--page <page>] [--dump]\npages: {} (and other names: updates, tailscale, …)",
                ids.join(", ")
            );
            return glib::ExitCode::SUCCESS;
        }
        _ => {}
    }
    if let Err(e) = parse(&argv) {
        eprintln!("wados-settings: {e}");
        return glib::ExitCode::FAILURE;
    }
    let app = gtk::Application::builder().application_id(APP_ID).flags(flags(&argv)).build();
    let ui: Rc<OnceCell<Rc<Ui>>> = Rc::default();
    let u = ui.clone();
    app.connect_command_line(move |app, cmd| {
        let args: Vec<String> = cmd.arguments().iter().map(|a| a.to_string_lossy().into_owned()).collect();
        let a = match parse(&args) {
            Ok(a) => a,
            Err(e) => {
                cmd.printerr_literal(&format!("wados-settings: {e}\n"));
                return glib::ExitCode::FAILURE;
            }
        };
        let fresh = u.get().is_none();
        let ui = u.get_or_init(|| Ui::new(app));
        match a.page.as_deref() {
            Some(p) => {
                if nav::resolve(p, None).is_none() {
                    cmd.printerr_literal(&format!(
                        "wados-settings: there's no page {p:?}: opening {}\n",
                        first_label()
                    ));
                }
                ui.go(p);
            }
            None if fresh => ui.go(nav::FIRST),
            None => {}
        }
        if a.dump {
            ui.dump_when_ready(app);
        }
        ui.window.present();
        glib::ExitCode::SUCCESS
    });
    app.run_with_args(&argv)
}

fn first_label() -> &'static str {
    nav::info(nav::FIRST).map_or("", |p| p.label)
}

/// The window: the sidebar, the open panel; every panel, made at the start.
struct Ui {
    window: gtk::ApplicationWindow,
    ctx: Ctx,
    panels: Vec<(&'static str, Rc<dyn Panel>)>,
    /// Each panel's page, once it's been opened.
    pages: RefCell<HashMap<&'static str, gtk::Widget>>,
    stack: gtk::Stack,
    side: Side,
    at: Cell<&'static str>,
}

impl Ui {
    fn new(app: &gtk::Application) -> Rc<Self> {
        if let Some(d) = gtk::gdk::Display::default() {
            wad_gtk::theme::install(&d);
            wad_gtk::kit::install(&d);
            wad_gtk::theme::add_css(&d, CSS);
        }
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Settings")
            .default_width(980)
            .default_height(720)
            .css_classes(["set-window"])
            .build();
        wad_gtk::kit::title_bar(&window, "settings", "Settings");

        // wadd's events go to every panel; they start coming once the main loop runs.
        let me: Rc<OnceCell<Weak<Ui>>> = Rc::default();
        let m = me.clone();
        let link = link::Link::start(move |i| {
            let Some(ui) = m.get().and_then(Weak::upgrade) else { return };
            for (_, p) in &ui.panels {
                p.on_event(&ui.ctx, &i);
            }
        });
        let m = me.clone();
        let ctx = Ctx::new(link, window.clone(), move |page| {
            if let Some(ui) = m.get().and_then(Weak::upgrade) {
                ui.go(page);
            }
        });
        let panels = panels::REGISTRY.iter().map(|(id, make)| (*id, make(&ctx))).collect();

        let stack = gtk::Stack::builder().hexpand(true).transition_type(gtk::StackTransitionType::None).build();
        let m = me.clone();
        let side = Side::new(move |id| {
            if let Some(ui) = m.get().and_then(Weak::upgrade) {
                ui.go(id);
            }
        });
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.append(&side.root);
        body.append(&stack);
        window.set_child(Some(&body));

        let ui = Rc::new(Self { window, ctx, panels, pages: RefCell::default(), stack, side, at: Cell::new("") });
        let _ = me.set(Rc::downgrade(&ui));
        // Typing finds a panel: the search has the keys from the start, and Ctrl+F brings them back.
        GtkWindowExt::set_focus(&ui.window, Some(&ui.side.search));
        let find = gtk::ShortcutController::new();
        find.set_scope(gtk::ShortcutScope::Global);
        let t = ui.side.search.clone();
        find.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("<Control>f"),
            Some(gtk::CallbackAction::new(move |_, _| {
                t.grab_focus();
                glib::Propagation::Stop
            })),
        ));
        ui.window.add_controller(find);
        ui
    }

    fn panel(&self, id: &str) -> Option<&Rc<dyn Panel>> {
        self.panels.iter().find(|(i, _)| *i == id).map(|(_, p)| p)
    }

    /// Opens a panel by its id or another name (Wi-Fi when it's neither).
    fn go(&self, page: &str) {
        let (id, sub) = nav::resolve(page, None).unwrap_or((nav::FIRST, None));
        let Some(panel) = self.panel(id).cloned() else { return };
        let built = self.pages.borrow().get(id).cloned();
        let w = match built {
            Some(w) => w,
            None => {
                let w = panel.build(&self.ctx);
                self.stack.add_named(&w, Some(id));
                self.pages.borrow_mut().insert(id, w.clone());
                w
            }
        };
        self.stack.set_visible_child(&w);
        self.at.set(id);
        self.side.select(id);
        panel.show(sub);
    }

    /// `--dump`: what the open page says once its panel's ready (5 s at most), then quit.
    fn dump_when_ready(self: &Rc<Self>, app: &gtk::Application) {
        let (me, app) = (Rc::downgrade(self), app.clone());
        let tries = Cell::new(0);
        glib::timeout_add_local(Duration::from_millis(200), move || {
            let Some(me) = me.upgrade() else { return glib::ControlFlow::Break };
            tries.set(tries.get() + 1);
            let id = me.at.get();
            let Some(panel) = me.panel(id).cloned() else { return glib::ControlFlow::Break };
            if !panel.ready() && tries.get() < 25 {
                return glib::ControlFlow::Continue;
            }
            let page = me.pages.borrow().get(id).cloned();
            if let Some(page) = page {
                println!("{}", panel.dump(&page));
            }
            app.quit();
            glib::ControlFlow::Break
        });
    }
}

/// The sidebar: its title, the search, the panels in their groups.
struct Side {
    root: gtk::Box,
    nav: gtk::Box,
    search: gtk::Text,
    items: Vec<(&'static str, gtk::Button)>,
}

impl Side {
    fn new(go: impl Fn(&str) + 'static) -> Self {
        let go: Rc<dyn Fn(&str)> = Rc::new(go);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .hexpand(false)
            .css_classes(["set-side"])
            .build();
        root.set_size_request(250, -1);
        let head = gtk::Box::builder().css_classes(["set-side-head"]).build();
        head.append(&gtk::Label::builder().label("Settings").css_classes(["set-side-title"]).build());
        root.append(&head);

        let bar = gtk::Box::builder().spacing(8).css_classes(["set-search"]).build();
        let si = wad_gtk::Icon::new("search", 15);
        si.set_valign(gtk::Align::Center);
        bar.append(&si);
        let search =
            gtk::Text::builder().placeholder_text("Search settings").hexpand(true).valign(gtk::Align::Center).build();
        search.set_widget_name("settings-search");
        search.update_property(&[gtk::accessible::Property::Label("Search settings")]);
        bar.append(&search);
        let clear = gtk::Button::builder()
            .child(&wad_gtk::Icon::new("x", 13))
            .valign(gtk::Align::Center)
            .visible(false)
            .css_classes(["set-clear"])
            .build();
        clear.set_tooltip_text(Some("Clear the search"));
        bar.append(&clear);
        root.append(&bar);

        let nav =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).css_classes(["set-nav"]).build();
        let scroll =
            gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&nav).build();
        root.append(&scroll);

        let items = nav::panels()
            .map(|p| {
                let b = gtk::Button::builder().css_classes(["set-item"]).build();
                b.set_widget_name(&format!("nav-{}", p.id));
                b.set_tooltip_text(Some(p.label));
                let row = gtk::Box::builder().spacing(12).build();
                let ic = wad_gtk::Icon::new(p.icon, 17);
                ic.set_valign(gtk::Align::Center);
                row.append(&ic);
                row.append(
                    &gtk::Label::builder()
                        .label(p.label)
                        .valign(gtk::Align::Center)
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .build(),
                );
                b.set_child(Some(&row));
                let (go, id) = (go.clone(), p.id);
                b.connect_clicked(move |_| go(id));
                (p.id, b)
            })
            .collect();
        let side = Self { root, nav, search, items };
        side.fill("");

        let me = (side.nav.clone(), side.items.clone(), clear.clone());
        side.search.connect_changed(move |t| {
            let q = t.text();
            me.2.set_visible(!q.is_empty());
            fill(&me.0, &me.1, &q);
        });
        side.search.connect_activate(move |t| {
            if let Some(p) = nav::first_match(&t.text()) {
                go(p.id);
            }
        });
        let keys = gtk::EventControllerKey::new();
        let t = side.search.clone();
        keys.connect_key_pressed(move |_, k, _, _| {
            if k == gtk::gdk::Key::Escape && !t.text().is_empty() {
                t.set_text("");
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        side.search.add_controller(keys);
        let t = side.search.clone();
        clear.connect_clicked(move |_| {
            t.set_text("");
            t.grab_focus();
        });
        side
    }

    fn fill(&self, query: &str) {
        fill(&self.nav, &self.items, query);
    }

    /// Lights the open panel's item.
    fn select(&self, id: &str) {
        for (i, b) in &self.items {
            if *i == id {
                b.add_css_class("on");
            } else {
                b.remove_css_class("on");
            }
        }
    }
}

/// The sidebar's list for a search: the groups that match, or that nothing does.
fn fill(nav: &gtk::Box, items: &[(&'static str, gtk::Button)], query: &str) {
    while let Some(c) = nav.first_child() {
        nav.remove(&c);
    }
    let groups = nav::filter(query);
    if groups.is_empty() {
        let l = wad_gtk::kit::label(&nav::nothing_matches(query), &["set-none"]);
        l.set_widget_name("nav-none");
        nav.append(&l);
        return;
    }
    for (i, g) in groups.iter().enumerate() {
        if i > 0 {
            nav.append(&gtk::Box::builder().css_classes(["set-sep"]).build());
        }
        for p in g {
            if let Some((_, b)) = items.iter().find(|(id, _)| *id == p.id) {
                nav.append(b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        std::iter::once("wados-settings").chain(a.iter().copied()).map(String::from).collect()
    }

    #[test]
    fn it_opens_on_a_page() {
        assert_eq!(parse(&args(&[])).unwrap(), Args::default());
        assert_eq!(parse(&args(&["--page", "accounts"])).unwrap().page.as_deref(), Some("accounts"));
        let a = parse(&args(&["--page", "updates", "--dump"])).unwrap();
        assert!(a.dump);
        assert_eq!(nav::resolve(a.page.as_deref().unwrap(), None), Some(("system", Some("updates"))));
        assert!(parse(&args(&["--page"])).is_err());
        assert!(parse(&args(&["--nope"])).is_err());
        assert_eq!(first_label(), "Wi-Fi");
        assert!(flags(&args(&["--dump"])).contains(gio::ApplicationFlags::NON_UNIQUE));
        assert!(!flags(&args(&[])).contains(gio::ApplicationFlags::NON_UNIQUE));
    }

    #[test]
    fn a_panel_not_built_yet_says_so() {
        assert_eq!(panel::being_built("Wi-Fi"), "Wi-Fi is being built");
    }
}
