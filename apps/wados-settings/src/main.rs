//! wados-settings: WadOS's Settings (the spike's `apps/settings`), after
//! GNOME Settings: a sidebar of panels beside the one open. Phase 6 has
//! Accounts (accounts.rs); the others show in the sidebar, dimmed, and come
//! in phase 8.
//!
//!     wados-settings [--page accounts]
//!
//! One window: a second start (the tray's Tailscale tile, the desktop's
//! Tailscale settings) brings it up at the page it names. `--dump` prints
//! what the page says once it has wadd's state, and quits (tests).

mod accounts;
mod link;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::{glib, prelude::*};
use wad_wadd_client::Incoming;

const APP_ID: &str = "io.wados.Settings";

/// The sidebar's panels (the spike's `nav.ts`), in its groups; only Accounts is here yet.
const PANELS: &[&[(&str, &str, &str)]] = &[
    &[("wifi", "Wi-Fi", "wifi"), ("network", "Network", "network"), ("bluetooth", "Bluetooth", "bluetooth")],
    &[("displays", "Displays", "monitor"), ("sound", "Sound", "volume-2"), ("power", "Power", "battery-charging")],
    &[("background", "Background", "image"), ("lock", "Lock Screen & Privacy", "lock")],
    &[("accounts", "Accounts", "users"), ("storage", "Storage", "hard-drive")],
    &[("mouse", "Mouse & Touchpad", "mouse"), ("keyboard", "Keyboard", "keyboard")],
    &[("system", "System", "info")],
];

const CSS: &str = r#"
window.set-window { background-color: var(--surface); }
.set-side { background-color: var(--bg-2); border-right: 1px solid var(--line); min-width: 250px; }
.set-side-head { min-height: 52px; padding: 0 18px; }
.set-side-title { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 15px; }
.set-search { margin: 2px 12px 10px; min-height: 34px; padding: 0 10px; border-radius: 10px; border: 1px solid var(--line); background-color: var(--surface); color: var(--faint); font-size: 13.5px; }
.set-nav { padding: 0 10px 14px; }
.set-item { min-height: 40px; padding: 0 12px; border-radius: 10px; font-size: 14px; }
.set-item wad-icon { color: var(--muted); }
.set-item.on { background-color: var(--surface-3); box-shadow: inset 0 0 0 1px var(--line); }
.set-item.on wad-icon { color: var(--accent); }
.set-item.off { opacity: 0.42; }
.set-sep { min-height: 1px; margin: 6px 12px; background-color: var(--line); }
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
            "--page" => a.page = Some(it.next().ok_or("--page needs a page (accounts)")?.clone()),
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
        Some("--help" | "-h") => {
            println!("usage: wados-settings [--page accounts] [--dump]");
            return glib::ExitCode::SUCCESS;
        }
        _ => {}
    }
    if let Err(e) = parse(&argv) {
        eprintln!("wados-settings: {e}");
        return glib::ExitCode::FAILURE;
    }
    let app = gtk::Application::builder().application_id(APP_ID).flags(flags(&argv)).build();
    let ui: Rc<std::cell::OnceCell<Rc<Ui>>> = Rc::default();
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
        let ui = u.get_or_init(|| Ui::new(app));
        if let Some(p) = a.page.as_deref().filter(|p| *p != "accounts") {
            cmd.printerr_literal(&format!("wados-settings: there's no {p} page yet (phase 8): Accounts is\n"));
        }
        if a.dump {
            ui.dump_when_ready(app);
        }
        ui.window.present();
        glib::ExitCode::SUCCESS
    });
    app.run_with_args(&argv)
}

struct Ui {
    window: gtk::ApplicationWindow,
    accounts: Rc<accounts::Accounts>,
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
        // Events come before the window shows: the panel's made first.
        let cell: Rc<std::cell::OnceCell<Rc<accounts::Accounts>>> = Rc::default();
        let c = cell.clone();
        let link = link::Link::start(move |i| {
            let Some(a) = c.get() else { return };
            let changed = match i {
                Incoming::Event(e) => {
                    let mut st = a.state.borrow_mut();
                    let was = st.connected;
                    st.connected = true;
                    st.apply(e) || !was
                }
                Incoming::Disconnected => {
                    let mut st = a.state.borrow_mut();
                    std::mem::replace(&mut st.connected, false)
                }
                Incoming::Other(..) => false,
            };
            if changed {
                a.queue_draw();
            }
        });
        let accounts = accounts::Accounts::new(link);
        let _ = cell.set(accounts.clone());

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.append(&side());
        body.append(&accounts.root);
        window.set_child(Some(&body));
        accounts.draw();
        Rc::new(Self { window, accounts })
    }

    /// `--dump`: what the page says once wadd's had a moment, then quit.
    fn dump_when_ready(self: &Rc<Self>, app: &gtk::Application) {
        let (me, app) = (self.clone(), app.clone());
        let tries = Cell::new(0);
        glib::timeout_add_local(Duration::from_millis(200), move || {
            tries.set(tries.get() + 1);
            if !me.accounts.state.borrow().connected && tries.get() < 25 {
                return glib::ControlFlow::Continue;
            }
            me.accounts.draw();
            println!("{}", me.accounts.dump());
            app.quit();
            glib::ControlFlow::Break
        });
    }
}

/// The sidebar: its title, a search (with the other panels, phase 8), the panels.
fn side() -> gtk::Box {
    let side =
        gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(false).css_classes(["set-side"]).build();
    side.set_size_request(250, -1);
    let head = gtk::Box::builder().css_classes(["set-side-head"]).build();
    head.append(&gtk::Label::builder().label("Settings").css_classes(["set-side-title"]).build());
    side.append(&head);
    let search = gtk::Box::builder().spacing(8).css_classes(["set-search"]).build();
    search.set_tooltip_text(Some("Searching settings comes with the other panels, in phase 8"));
    let si = wad_gtk::Icon::new("search", 15);
    si.set_valign(gtk::Align::Center);
    search.append(&si);
    search.append(&gtk::Label::builder().label("Search settings").valign(gtk::Align::Center).build());
    side.append(&search);
    let nav = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).css_classes(["set-nav"]).build();
    for (i, group) in PANELS.iter().enumerate() {
        if i > 0 {
            nav.append(&gtk::Box::builder().css_classes(["set-sep"]).build());
        }
        for (id, label, icon) in group.iter() {
            let item = gtk::Box::builder().spacing(12).css_classes(["set-item"]).build();
            let ic = wad_gtk::Icon::new(icon, 17);
            ic.set_valign(gtk::Align::Center);
            item.append(&ic);
            item.append(&gtk::Label::builder().label(*label).valign(gtk::Align::Center).build());
            if *id == "accounts" {
                item.add_css_class("on");
            } else {
                item.add_css_class("off");
                item.set_tooltip_text(Some(&format!("{label} comes in phase 8")));
            }
            nav.append(&item);
        }
    }
    side.append(&nav);
    side
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
        assert!(parse(&args(&["--page"])).is_err());
        assert!(parse(&args(&["--nope"])).is_err());
        assert!(PANELS.iter().flat_map(|g| g.iter()).any(|(id, _, _)| *id == "accounts"));
    }
}
