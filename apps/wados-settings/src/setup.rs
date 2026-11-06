//! Set up WadSpaces (`wados-settings --setup`, the spike's `apps/setup`):
//! opened by the shell at the first sign-in on a new machine. Welcome,
//! Wi-Fi, Accounts, WadOS, Done. wadd keeps where it's got to, so closed
//! early it opens again where it was. Wi-Fi and Accounts are Settings' own panels.
//! Steam is on the Accounts page, Settings' own.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::{glib, prelude::*};
use wad_gtk::kit::{self, Variant};
use wad_proto::SetupStep;
use wad_wadd_client::Incoming;

use crate::link::Link;
use crate::panel::{Ctx, Panel};
use crate::panels;

const CSS: &str = r#"
.setup-steps { padding: 14px 24px; border-bottom: 1px solid var(--line); }
.setup-step { font-size: 12.5px; color: var(--faint); }
.setup-step.on { color: var(--fg); font-weight: 600; }
.setup-page { padding: 28px 40px; }
.setup-title { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 22px; }
.setup-body { font-size: 14px; color: var(--muted); }
.setup-foot { padding: 14px 24px; border-top: 1px solid var(--line); }
"#;

static MARK: &[u8] = include_bytes!("../../../assets/brand/wadspaces-icon-dark-transparent.svg");

const STEPS: [(SetupStep, &str); 5] = [
    (SetupStep::Welcome, "Welcome"),
    (SetupStep::Wifi, "Wi-Fi"),
    (SetupStep::Accounts, "Accounts"),
    (SetupStep::Design, "WadOS"),
    (SetupStep::Done, "Done"),
];

fn index(step: SetupStep) -> usize {
    STEPS.iter().position(|(s, _)| *s == step).unwrap_or(0)
}

/// Words for the Wi-Fi step, and whether it's online.
fn online(n: &wad_proto::Network) -> Option<String> {
    if n.ethernet.plugged {
        Some("Connected to the wired network".into())
    } else {
        n.connected.as_ref().map(|s| format!("Connected to {s}"))
    }
}

struct Setup {
    window: gtk::ApplicationWindow,
    ctx: Ctx,
    steps: gtk::Box,
    stack: gtk::Stack,
    back: gtk::Button,
    skip: gtk::Button,
    next: gtk::Button,
    at: std::cell::Cell<SetupStep>,
    wifi_said: gtk::Label,
    is_online: std::cell::Cell<bool>,
    panels: RefCell<Vec<Rc<dyn Panel>>>,
    starter: wad_gtk::form::Toggle,
}

fn title_page(title: &str, body: &str) -> gtk::Box {
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).css_classes(["setup-page"]).build();
    b.append(&kit::label(title, &["setup-title"]));
    if !body.is_empty() {
        let l = kit::label(body, &["setup-body"]);
        l.set_wrap(true);
        b.append(&l);
    }
    b
}

/// Opens it (one window: a second `--setup` only brings it up).
pub fn present(app: &gtk::Application, slot: &RefCell<Option<gtk::ApplicationWindow>>) {
    if let Some(w) = slot.borrow().as_ref() {
        w.present();
        return;
    }
    if let Some(d) = gtk::gdk::Display::default() {
        wad_gtk::theme::install(&d);
        wad_gtk::kit::install(&d);
        wad_gtk::theme::add_css(&d, CSS);
    }
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Set up WadSpaces")
        .default_width(860)
        .default_height(680)
        .css_classes(["set-window"])
        .build();
    kit::title_bar(&window, "settings", "Set up WadSpaces");
    let me: Rc<RefCell<Option<Rc<Setup>>>> = Rc::default();
    let m = me.clone();
    let link = Link::start(move |i: Incoming| {
        let Some(s) = m.borrow().clone() else { return };
        for p in s.panels.borrow().iter() {
            p.on_event(&s.ctx, &i);
        }
        s.look_online();
    });
    let ctx = Ctx::new(link, window.clone(), |_| {});
    let s = Setup::new(window.clone(), ctx);
    *me.borrow_mut() = Some(s.clone());
    // Kept alive by the window.
    let keep = me.clone();
    window.connect_close_request(move |_| {
        keep.borrow_mut().take();
        glib::Propagation::Proceed
    });
    *slot.borrow_mut() = Some(window.clone());
    s.resume();
    window.present();
}

impl Setup {
    fn new(window: gtk::ApplicationWindow, ctx: Ctx) -> Rc<Self> {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        let steps = gtk::Box::builder().spacing(18).css_classes(["setup-steps"]).build();
        let stack = gtk::Stack::builder().vexpand(true).transition_type(gtk::StackTransitionType::None).build();
        let foot = gtk::Box::builder().spacing(10).css_classes(["setup-foot"]).build();
        let back = kit::button("Back", Variant::Ghost, None, false);
        let skip = kit::button("Skip", Variant::Ghost, None, false);
        let next = kit::button("Continue", Variant::Primary, None, false);
        foot.append(&back);
        foot.append(&gtk::Box::builder().hexpand(true).build());
        foot.append(&skip);
        foot.append(&next);
        root.append(&steps);
        root.append(&stack);
        root.append(&foot);
        window.set_child(Some(&root));

        let welcome = title_page("WadSpaces is almost ready", "A few more steps.");
        let mark = wad_gtk::icon::Svg::new(MARK, 72, 72);
        mark.set_halign(gtk::Align::Start);
        welcome.prepend(&mark);
        stack.add_named(&welcome, Some("Welcome"));

        let wifi_panel = panels::wifi::make(&ctx);
        let accounts_panel = panels::accounts::make(&ctx);
        let wifi = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
        let wifi_said = kit::label("", &["setup-body"]);
        wifi_said.set_margin_start(40);
        let w = wifi_panel.build(&ctx);
        w.set_vexpand(true);
        wifi.append(&w);
        wifi.append(&wifi_said);
        stack.add_named(&wifi, Some("Wi-Fi"));
        let a = accounts_panel.build(&ctx);
        stack.add_named(&a, Some("Accounts"));

        let design = title_page(
            "Create your own operating system",
            "Tired of waiting for updates? The wadspace below is your own branch of the official WadSpaces repo, set up with VS Code and Claude Code. Start there and shape the OS to suit you.",
        );
        let row = gtk::Box::builder().spacing(12).build();
        let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).hexpand(true).build();
        col.append(&kit::label("WadOS", &["setup-title"]));
        let about = kit::label(
            "VS Code, WadBrowser and Claude Code, with the WadSpaces repo: for working on WadSpaces itself.",
            &["setup-body"],
        );
        about.set_wrap(true);
        col.append(&about);
        row.append(&col);
        let starter = wad_gtk::form::Toggle::new(true, |_| {});
        row.append(&starter.root);
        design.append(&row);
        stack.add_named(&design, Some("WadOS"));

        let done = title_page("All done", "Enjoy WadSpaces!");
        stack.add_named(&done, Some("Done"));

        let s = Rc::new(Self {
            window,
            ctx,
            steps,
            stack,
            back,
            skip,
            next,
            at: std::cell::Cell::new(SetupStep::Welcome),
            wifi_said,
            is_online: std::cell::Cell::new(false),
            panels: RefCell::new(vec![wifi_panel, accounts_panel]),
            starter,
        });
        let m = Rc::downgrade(&s);
        s.next.connect_clicked(move |_| {
            if let Some(m) = m.upgrade() {
                m.forward(true);
            }
        });
        let m = Rc::downgrade(&s);
        s.skip.connect_clicked(move |_| {
            if let Some(m) = m.upgrade() {
                m.forward(false);
            }
        });
        let m = Rc::downgrade(&s);
        s.back.connect_clicked(move |_| {
            if let Some(m) = m.upgrade() {
                let i = index(m.at.get());
                if i > 0 {
                    m.save(STEPS[i - 1].0);
                }
            }
        });
        s
    }

    /// Where wadd says it was.
    fn resume(self: &Rc<Self>) {
        let m = Rc::downgrade(self);
        self.ctx.link.call(
            |w| async move { w.setup().await },
            move |r| {
                let Some(m) = m.upgrade() else { return };
                m.show(r.map_or(SetupStep::Welcome, |s| s.step));
            },
        );
    }

    fn look_online(self: &Rc<Self>) {
        let m = Rc::downgrade(self);
        self.ctx.link.call(
            |w| async move { w.network().await },
            move |r| {
                let Some(m) = m.upgrade() else { return };
                let said = r.ok().and_then(|n| online(&n));
                m.is_online.set(said.is_some());
                m.wifi_said.set_label(said.as_deref().unwrap_or(""));
                m.draw();
            },
        );
    }

    fn save(self: &Rc<Self>, step: SetupStep) {
        let m = Rc::downgrade(self);
        self.ctx.link.call(
            move |w| async move { w.setup_step(step).await },
            move |r| {
                if let (Some(m), Ok(s)) = (m.upgrade(), r) {
                    m.show(s.step);
                }
            },
        );
    }

    fn show(self: &Rc<Self>, step: SetupStep) {
        self.at.set(step);
        self.stack.set_visible_child_name(STEPS[index(step)].1);
        for p in self.panels.borrow().iter() {
            p.show(None);
        }
        self.look_online();
        self.draw();
    }

    fn draw(&self) {
        let step = self.at.get();
        while let Some(c) = self.steps.first_child() {
            self.steps.remove(&c);
        }
        for (s, name) in STEPS {
            self.steps.append(&kit::label(name, &["setup-step", if s == step { "on" } else { "off" }]));
        }
        let (next, skip) = match step {
            SetupStep::Welcome => ("Continue", false),
            SetupStep::Wifi => ("Next", !self.is_online.get()),
            SetupStep::Accounts => ("Next", true),
            SetupStep::Design => ("Install", true),
            SetupStep::Done => ("Close", false),
        };
        self.next.set_label(next);
        self.next.set_sensitive(step != SetupStep::Wifi || self.is_online.get());
        self.skip.set_visible(skip);
        self.back.set_visible(!matches!(step, SetupStep::Welcome | SetupStep::Done));
    }

    /// Next (`yes`), or Skip.
    fn forward(self: &Rc<Self>, yes: bool) {
        match self.at.get() {
            SetupStep::Done => self.window.close(),
            SetupStep::Design => {
                let starter = yes && self.starter.is_on();
                let m = Rc::downgrade(self);
                self.ctx.link.call(
                    move |w| async move { w.setup_design(starter).await },
                    move |r| {
                        if let (Some(m), Ok(s)) = (m.upgrade(), r) {
                            m.show(s.step);
                        }
                    },
                );
            }
            step => self.save(STEPS[index(step) + 1].0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cable_or_a_network_is_online() {
        let mut n = wad_proto::Network::default();
        assert_eq!(online(&n), None);
        n.connected = Some("home".into());
        assert_eq!(online(&n).as_deref(), Some("Connected to home"));
        n.ethernet.plugged = true;
        assert_eq!(online(&n).as_deref(), Some("Connected to the wired network"));
    }

    #[test]
    fn the_steps_run_in_the_spikes_order() {
        assert_eq!(STEPS.map(|(_, n)| n), ["Welcome", "Wi-Fi", "Accounts", "WadOS", "Done"]);
    }
}
