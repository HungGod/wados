//! wados-agents: the Agents app (the spike's `apps/agents`): what wants you,
//! what's at work, Send to a wadspace, and a job in full, with its sandbox's
//! checklist. It only shows and asks: wadd runs everything
//! (apps/wadd/src/agents.rs), and every approval goes through it.
//!
//!     wados-agents [--open <agent or sandbox id>]

mod link;

use std::cell::RefCell;
use std::rc::Rc;

use gtk::{gio, glib, prelude::*};
use link::Link;
use wad_gtk::form::TextField;
use wad_gtk::kit::{self, Variant};
use wad_model::agents::{
    Agent, AgentNote, AgentStatus, Ask, IMPL_WORDS, READY_AT, STEPS, Sandbox, SandboxState, StepState, Verdict, wants,
};
use wad_proto::{Agents, Event};
use wad_wadd_client::Incoming;

const APP_ID: &str = "io.wados.Agents";

const CSS: &str = r#"
window.ag-window { background-color: var(--surface); }
.ag-rail { min-width: 270px; padding: 12px; background-color: var(--bg-2); border-right: 1px solid var(--line); }
.ag-h { margin: 12px 8px 6px; font-size: 11px; font-weight: 700; letter-spacing: 0.07em; color: var(--faint); }
button.ag-row { padding: 8px 10px; border-radius: 10px; background: none; background-image: none; border: none; box-shadow: none; text-shadow: none; }
button.ag-row:hover { background-color: var(--surface-2); }
button.ag-row.on { background-color: var(--surface-3); }
.ag-name { font-size: 13.5px; font-weight: 500; color: var(--fg); }
.ag-sub { font-size: 12px; color: var(--faint); }
.ag-page { padding: 24px 32px; }
.ag-title { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 22px; }
.ag-body { font-size: 14px; color: var(--muted); }
.ag-card { padding: 14px 16px; border-radius: 12px; border: 1px solid var(--line); background-color: var(--surface-2); }
.ag-step { padding: 3px 9px; border-radius: 99px; font-size: 12px; color: var(--faint); background-color: var(--surface-3); }
.ag-step.active, .ag-step.waiting { color: var(--fg); box-shadow: inset 0 0 0 1px var(--accent); }
.ag-step.done { color: var(--accent); }
.ag-step.failed { color: var(--danger); }
.ag-log { font-family: monospace; font-size: 12.5px; color: var(--muted); }
.ag-you { color: var(--fg); }
.ag-mcp { color: #ff7ac6; }
"#;

/// A status, as the app says it.
pub fn status_word(s: AgentStatus) -> &'static str {
    match s {
        AgentStatus::Working => "Working",
        AgentStatus::NeedsYou => "Needs you",
        AgentStatus::Done => "Done",
        AgentStatus::Stopped => "Stopped",
    }
}

/// A line of its work.
pub fn note_line(n: &AgentNote) -> (String, &'static str) {
    match n {
        AgentNote::Say { text } | AgentNote::Final { text } => (text.clone(), "ag-log"),
        AgentNote::You { text } => (format!("> {text}"), "ag-you"),
        AgentNote::Read { path } => (format!("Read {path}"), "ag-log"),
        AgentNote::Edit { path } => (format!("Edit {path}"), "ag-log"),
        AgentNote::Run { command } => (format!("$ {command}"), "ag-log"),
        AgentNote::Mcp { tool, text } => (format!("wadspaces · {tool} {text}"), "ag-mcp"),
    }
}

/// What the checklist's scale says.
pub fn done_word(done: u8) -> &'static str {
    IMPL_WORDS[usize::from(done.min(4))]
}

struct Ui {
    window: gtk::ApplicationWindow,
    link: RefCell<Option<Link>>,
    rail: gtk::Box,
    page: gtk::Box,
    state: RefCell<Agents>,
    wadspaces: RefCell<Vec<(String, String)>>,
    /// The job open, or none: the overview.
    open: RefCell<Option<String>>,
    /// What's being typed survives a redraw.
    to: RefCell<String>,
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
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
            .title("Agents")
            .default_width(1180)
            .default_height(720)
            .css_classes(["ag-window"])
            .build();
        kit::title_bar(&window, "bot", "Agents");
        let root = gtk::Box::builder().build();
        let rail =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).css_classes(["ag-rail"]).build();
        let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).hexpand(true).build();
        let page =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).css_classes(["ag-page"]).build();
        scroll.set_child(Some(&page));
        root.append(&rail);
        root.append(&scroll);
        window.set_child(Some(&root));
        let ui = Rc::new(Self {
            window,
            link: RefCell::default(),
            rail,
            page,
            state: RefCell::default(),
            wadspaces: RefCell::default(),
            open: RefCell::default(),
            to: RefCell::default(),
        });
        let w = Rc::downgrade(&ui);
        let link = Link::start(move |i| {
            let Some(u) = w.upgrade() else { return };
            match i {
                Incoming::Event(Event::Agents(a)) => {
                    *u.state.borrow_mut() = *a;
                    u.draw();
                }
                Incoming::Event(Event::Library(l)) => {
                    let list = l.wadspaces.iter().filter(|x| !x.is_trashed()).map(|x| (x.id.clone(), x.name.clone()));
                    *u.wadspaces.borrow_mut() = list.collect();
                    u.draw();
                }
                _ => {}
            }
        });
        *ui.link.borrow_mut() = Some(link);
        ui.load();
        ui
    }

    fn load(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        self.call(
            |c| async move { c.agents().await },
            move |a| {
                if let Some(u) = w.upgrade() {
                    *u.state.borrow_mut() = a;
                    u.draw();
                }
            },
        );
    }

    fn call<T: Send + 'static, F>(&self, f: impl FnOnce(wad_wadd_client::Wadd) -> F, done: impl FnOnce(T) + 'static)
    where
        F: std::future::Future<Output = Result<T, wad_wadd_client::Failure>> + Send + 'static,
    {
        let Some(l) = self.link.borrow().clone() else { return };
        let window = self.window.clone();
        l.call(f, move |r| match r {
            Ok(v) => done(v),
            Err(e) => {
                gtk::AlertDialog::builder().message(e.message).build().show(Some(&window));
            }
        });
    }

    fn name_of(&self, ws: &str) -> String {
        self.wadspaces.borrow().iter().find(|(i, _)| i == ws).map_or_else(|| ws.to_string(), |(_, n)| n.clone())
    }

    fn sandbox_of(&self, a: &Agent) -> Option<Sandbox> {
        self.state.borrow().sandboxes.iter().find(|s| Some(&s.id) == a.sandbox.as_ref()).cloned()
    }

    /// What an id is opened at: a sandbox's is its agent's page.
    fn open_at(self: &Rc<Self>, id: &str) {
        let of_box = self.state.borrow().sandboxes.iter().find(|s| s.id == id).map(|s| s.agent.clone());
        *self.open.borrow_mut() = Some(of_box.unwrap_or_else(|| id.to_string()));
        self.draw();
    }

    fn draw(self: &Rc<Self>) {
        self.draw_rail();
        clear(&self.page);
        let open = self.open.borrow().clone();
        let a = open.and_then(|id| self.state.borrow().agents.iter().find(|a| a.id == id).cloned());
        match a {
            Some(a) => self.job(&a),
            None => self.overview(),
        }
    }

    fn row(self: &Rc<Self>, title: &str, sub: &str, on: bool, id: Option<String>) -> gtk::Button {
        let b = gtk::Button::builder().css_classes(["ag-row"]).build();
        if on {
            b.add_css_class("on");
        }
        let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(1).build();
        let t = kit::label(title, &["ag-name"]);
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        col.append(&t);
        if !sub.is_empty() {
            let s = kit::label(sub, &["ag-sub"]);
            s.set_ellipsize(gtk::pango::EllipsizeMode::End);
            col.append(&s);
        }
        b.set_child(Some(&col));
        let w = Rc::downgrade(self);
        b.connect_clicked(move |_| {
            if let Some(u) = w.upgrade() {
                *u.open.borrow_mut() = id.clone();
                u.draw();
            }
        });
        b
    }

    fn draw_rail(self: &Rc<Self>) {
        clear(&self.rail);
        let open = self.open.borrow().clone();
        self.rail.append(&self.row("Overview", "", open.is_none(), None));
        let st = self.state.borrow().clone();
        let need: Vec<&Agent> = st.agents.iter().filter(|a| a.needs_you()).collect();
        self.rail.append(&kit::label(&format!("Needs you · {}", need.len()), &["ag-h"]));
        for a in need {
            let s = st.sandboxes.iter().find(|s| Some(&s.id) == a.sandbox.as_ref());
            let sub = wants(a, s).unwrap_or_default();
            self.rail.append(&self.row(&self.name_of(&a.ws), &sub, open.as_ref() == Some(&a.id), Some(a.id.clone())));
        }
        self.rail.append(&kit::label("Jobs", &["ag-h"]));
        for a in st.agents.iter().filter(|a| !a.needs_you()) {
            let sub = format!("{} · {}", status_word(a.status), self.name_of(&a.ws));
            self.rail.append(&self.row(&a.task, &sub, open.as_ref() == Some(&a.id), Some(a.id.clone())));
        }
    }

    fn overview(self: &Rc<Self>) {
        self.page.append(&kit::label("Agents", &["ag-title"]));
        let lead = kit::label(
            "Claude Code at work in your wadspaces: what it wants from you, where it's up to, and what you'd like it to do next.",
            &["ag-body"],
        );
        lead.set_wrap(true);
        self.page.append(&lead);
        let st = self.state.borrow().clone();
        self.page.append(&kit::label("Waiting for you", &["ag-h"]));
        let need: Vec<&Agent> = st.agents.iter().filter(|a| a.needs_you()).collect();
        if need.is_empty() {
            self.page.append(&kit::label("Nothing: no agent is waiting on you.", &["ag-body"]));
        }
        for a in need {
            self.page.append(&self.card(a));
        }
        self.page.append(&kit::label("At work", &["ag-h"]));
        let work: Vec<&Agent> = st.agents.iter().filter(|a| a.status == AgentStatus::Working).collect();
        if work.is_empty() {
            self.page.append(&kit::label("No agent is working right now.", &["ag-body"]));
        }
        for a in work {
            self.page.append(&self.card(a));
        }
        self.composer();
    }

    fn card(self: &Rc<Self>, a: &Agent) -> gtk::Box {
        let c = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["ag-card"]).build();
        c.append(&kit::label(&a.task, &["ag-name"]));
        c.append(&kit::label(&format!("{} · {}", self.name_of(&a.ws), a.now), &["ag-sub"]));
        c.append(&steps(a));
        let review = kit::button("Review", Variant::Secondary, None, true);
        review.set_halign(gtk::Align::Start);
        let (w, id) = (Rc::downgrade(self), a.id.clone());
        review.connect_clicked(move |_| {
            if let Some(u) = w.upgrade() {
                u.open_at(&id);
            }
        });
        c.append(&review);
        c
    }

    fn composer(self: &Rc<Self>) {
        self.page.append(&kit::label("Send to a wadspace", &["ag-h"]));
        let lead = kit::label(
            "Ask the Claude Code in one of this machine's wadspaces for anything, in your own words.",
            &["ag-body"],
        );
        self.page.append(&lead);
        let list = self.wadspaces.borrow().clone();
        let names: Vec<&str> = list.iter().map(|(_, n)| n.as_str()).collect();
        let to = gtk::DropDown::from_strings(&names);
        if let Some(at) = list.iter().position(|(i, _)| *i == *self.to.borrow()) {
            to.set_selected(at as u32);
        }
        to.set_halign(gtk::Align::Start);
        let w = Rc::downgrade(self);
        let l2 = list.clone();
        to.connect_selected_notify(move |d| {
            if let (Some(u), Some((id, _))) = (w.upgrade(), l2.get(d.selected() as usize)) {
                *u.to.borrow_mut() = id.clone();
            }
        });
        self.page.append(&to);
        let text = TextField::new("", "What should Claude Code do?").named("composer");
        self.page.append(&text.root);
        let send = kit::button("Send", Variant::Primary, None, false);
        send.set_halign(gtk::Align::Start);
        let w = Rc::downgrade(self);
        let t = text.text.clone();
        let picked = to.clone();
        send.connect_clicked(move |_| {
            let Some(u) = w.upgrade() else { return };
            let Some((ws, _)) = list.get(picked.selected() as usize).cloned() else { return };
            let said = t.text().to_string();
            let keep = Rc::downgrade(&u);
            u.call(
                move |c| async move { c.agent_send(&ws, &said).await },
                move |a| {
                    if let Some(u) = keep.upgrade() {
                        u.open_at(&a.id);
                    }
                },
            );
        });
        self.page.append(&send);
    }

    fn act(self: &Rc<Self>, id: &str, what: &'static str) {
        let id = id.to_string();
        self.call(move |c| async move { c.agent_act(&id, what).await }, |_: ()| {});
    }

    fn button(self: &Rc<Self>, row: &gtk::Box, label: &str, v: Variant, id: &str, what: &'static str) {
        let b = kit::button(label, v, None, false);
        let (w, id) = (Rc::downgrade(self), id.to_string());
        b.connect_clicked(move |_| {
            if let Some(u) = w.upgrade() {
                u.act(&id, what);
            }
        });
        row.append(&b);
    }

    fn job(self: &Rc<Self>, a: &Agent) {
        self.page.append(&kit::label(&a.task, &["ag-title"]));
        self.page.append(&kit::label(
            &format!("Claude Code in {} · {}", self.name_of(&a.ws), status_word(a.status)),
            &["ag-sub"],
        ));
        self.page.append(&steps(a));
        self.page.append(&kit::label(&a.now, &["ag-body"]));
        match &a.ask {
            Some(Ask::Sandbox { title, why }) => {
                let c = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(8)
                    .css_classes(["ag-card"])
                    .build();
                c.append(&kit::label("Wants a sandbox to try it with you", &["ag-name"]));
                for line in [
                    title.as_str(),
                    why.as_str(),
                    "A throwaway wadspace running this build. It can't see your files, wadspaces or sign-ins.",
                ] {
                    let l = kit::label(line, &["ag-body"]);
                    l.set_wrap(true);
                    c.append(&l);
                }
                let row = gtk::Box::builder().spacing(8).build();
                self.button(&row, "Allow", Variant::Primary, &a.id, "allow");
                self.button(&row, "Not now", Variant::Ghost, &a.id, "deny");
                c.append(&row);
                self.page.append(&c);
            }
            Some(Ask::Ship) => {
                let c = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(8)
                    .css_classes(["ag-card"])
                    .build();
                c.append(&kit::label(
                    "It's waiting for you: open a pull request as it is, or try it in a sandbox after all.",
                    &["ag-body"],
                ));
                let row = gtk::Box::builder().spacing(8).build();
                self.button(&row, "Open a pull request", Variant::Primary, &a.id, "ship");
                self.button(&row, "Sandbox after all", Variant::Ghost, &a.id, "allow");
                c.append(&row);
                self.page.append(&c);
            }
            None => {}
        }
        if let Some(s) = self.sandbox_of(a).filter(|s| s.state != SandboxState::Closed) {
            self.checklist(&s, a);
        }
        if a.status == AgentStatus::NeedsYou {
            let t = TextField::new("", &format!("Tell Claude Code something about “{}”", a.task));
            self.page.append(&t.root);
            let (w, id, text) = (Rc::downgrade(self), a.id.clone(), t.text.clone());
            t.on_enter(move |_| {
                let Some(u) = w.upgrade() else { return };
                let (id, said) = (id.clone(), text.text().to_string());
                u.call(move |c| async move { c.agent_say(&id, &said).await }, |_: ()| {});
            });
        }
        self.page.append(&kit::label("Its work", &["ag-h"]));
        if a.notes.is_empty() {
            self.page.append(&kit::label("Nothing yet.", &["ag-body"]));
        }
        for n in &a.notes {
            let (text, class) = note_line(n);
            let l = kit::label(&text, &["ag-log", class]);
            l.set_wrap(true);
            l.set_selectable(true);
            self.page.append(&l);
        }
        if a.live() {
            let row = gtk::Box::builder().spacing(8).build();
            self.button(&row, "Stop", Variant::Danger, &a.id, "stop");
            self.page.append(&row);
        }
    }

    fn checklist(self: &Rc<Self>, s: &Sandbox, a: &Agent) {
        let c = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["ag-card"]).build();
        c.append(&kit::label("Checklist from Claude Code", &["ag-name"]));
        c.append(&kit::label(
            &format!("{} · Build {} · {}% done", s.checked_words(), s.build, s.percent()),
            &["ag-sub"],
        ));
        if s.state != SandboxState::Ready {
            c.append(&kit::label("Its sandbox is getting ready", &["ag-body"]));
        }
        for i in &s.items {
            c.append(&kit::label(&i.label, &["ag-name"]));
            c.append(&kit::label(&format!("{} · {}", done_word(i.done), i.how), &["ag-sub"]));
            if i.fresh {
                c.append(&kit::label(&format!("New in build {}", s.build), &["ag-sub"]));
            }
            if i.done < READY_AT {
                c.append(&kit::label("Not ready to try yet: Claude Code will say when it is.", &["ag-sub"]));
                continue;
            }
            let row = gtk::Box::builder().spacing(6).build();
            for (word, v) in [("Works", Verdict::Works), ("Issue", Verdict::Issue), ("Skip", Verdict::Skip)] {
                let on = i.verdict == Some(v);
                let b = kit::button(word, if on { Variant::Primary } else { Variant::Quiet }, None, true);
                let (w, sid, item) = (Rc::downgrade(self), s.id.clone(), i.id.clone());
                let note = i.note.clone();
                b.connect_clicked(move |_| {
                    let Some(u) = w.upgrade() else { return };
                    let (sid, item, note) = (sid.clone(), item.clone(), note.clone());
                    u.call(move |c| async move { c.sandbox_mark(&sid, &item, Some(v), &note).await }, |_: ()| {});
                });
                row.append(&b);
            }
            c.append(&row);
            if i.verdict == Some(Verdict::Issue) {
                let t = TextField::new("", "What happened?");
                t.set_value(&i.note);
                let (w, sid, item) = (Rc::downgrade(self), s.id.clone(), i.id.clone());
                t.on_change(move |text| {
                    let Some(u) = w.upgrade() else { return };
                    let (sid, item, text) = (sid.clone(), item.clone(), text.to_string());
                    u.call(
                        move |c| async move { c.sandbox_mark(&sid, &item, Some(Verdict::Issue), &text).await },
                        |_: ()| {},
                    );
                });
                c.append(&t.root);
            }
        }
        let row = gtk::Box::builder().spacing(8).build();
        let marked = s.items.iter().filter(|i| i.verdict.is_some()).count();
        let send = kit::button(&format!("Send to Claude Code ({marked})"), Variant::Primary, None, false);
        send.set_sensitive(marked > 0 && s.state == SandboxState::Ready);
        let (w, sid) = (Rc::downgrade(self), s.id.clone());
        send.connect_clicked(move |_| {
            if let Some(u) = w.upgrade() {
                let sid = sid.clone();
                u.call(move |c| async move { c.sandbox_act(&sid, "send").await }, |_: ()| {});
            }
        });
        row.append(&send);
        let label = if s.looks_good() { "Looks good: open a pull request" } else { "Ship it as it is" };
        self.button(&row, label, Variant::Secondary, &a.id, "ship");
        let close = kit::button("Close sandbox", Variant::Ghost, None, false);
        let (w, sid) = (Rc::downgrade(self), s.id.clone());
        close.connect_clicked(move |_| {
            if let Some(u) = w.upgrade() {
                let sid = sid.clone();
                u.call(move |c| async move { c.sandbox_act(&sid, "close").await }, |_: ()| {});
            }
        });
        row.append(&close);
        c.append(&row);
        self.page.append(&c);
    }
}

fn steps(a: &Agent) -> gtk::Box {
    let row = gtk::Box::builder().spacing(6).build();
    for (s, (_, word)) in a.steps.iter().zip(STEPS) {
        let class = match s.state {
            StepState::Todo => "todo",
            StepState::Active => "active",
            StepState::Waiting => "waiting",
            StepState::Done => "done",
            StepState::Failed => "failed",
        };
        row.append(&kit::label(word, &["ag-step", class]));
    }
    row
}

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) == Some("--version") {
        println!("wados-agents {}", env!("CARGO_PKG_VERSION"));
        return glib::ExitCode::SUCCESS;
    }
    let app =
        gtk::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE).build();
    let ui: RefCell<Option<Rc<Ui>>> = RefCell::default();
    app.connect_command_line(move |app, cmd| {
        let args: Vec<String> = cmd.arguments().iter().map(|a| a.to_string_lossy().into_owned()).collect();
        let mut slot = ui.borrow_mut();
        let u = slot.get_or_insert_with(|| Ui::new(app));
        if let Some(i) = args.iter().position(|a| a == "--open")
            && let Some(id) = args.get(i + 1)
        {
            u.open_at(id);
        }
        u.window.present();
        glib::ExitCode::SUCCESS
    });
    app.run_with_args(&argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_says_things_as_the_spike_does() {
        assert_eq!(status_word(AgentStatus::NeedsYou), "Needs you");
        assert_eq!(done_word(0), "Not started");
        assert_eq!(done_word(9), "Done");
        let (t, c) = note_line(&AgentNote::Mcp { tool: "notify".into(), text: "hi".into() });
        assert_eq!((t.as_str(), c), ("wadspaces · notify hi", "ag-mcp"));
        assert_eq!(note_line(&AgentNote::You { text: "go".into() }).0, "> go");
    }
}
