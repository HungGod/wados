//! wados-installer: WadSpaces Live's "Install WadSpaces" (the spike's
//! `apps/installer`): Welcome, the computer's name, the account, the disk,
//! its encryption, the install, Done. It only asks: wadd installs
//! (`POST /v1/install`, apps/wadd/src/install.rs), and has the last word on
//! every rule here. Closed while it installs, the job carries on, and
//! opening it again finds it.
//!
//! Its passwords stay in its fields until they're sent, and nothing logs them.

mod link;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::{glib, prelude::*};
use link::Link;
use wad_gtk::form::TextField;
use wad_gtk::kit::{self, Variant};
use wad_model::JobStatus;
use wad_model::install_names::{computer_name_problem, password_problem, shown_name, strength, username_problem};
use wad_proto::{InstallDisk, InstallRequest, JobLog};

const APP_ID: &str = "io.wados.Installer";
static MARK: &[u8] = include_bytes!("../../../assets/brand/wadspaces-icon-dark-transparent.svg");

const STEPS: [&str; 7] = ["Welcome", "Computer", "Account", "Disk", "Encrypt", "Install", "Done"];
const LOG_LINES: usize = 12;

const CSS: &str = r#"
window.inst-window { background-color: var(--surface); }
.inst-steps { padding: 14px 24px; border-bottom: 1px solid var(--line); }
.inst-step { font-size: 12.5px; color: var(--faint); }
.inst-step.on { color: var(--fg); font-weight: 600; }
.inst-step.past { color: var(--muted); }
.inst-page { padding: 28px 40px; }
.inst-title { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 22px; }
.inst-body { font-size: 14px; color: var(--muted); }
.inst-foot { padding: 14px 24px; border-top: 1px solid var(--line); }
button.inst-disk { padding: 10px 12px; border-radius: 10px; border: 1px solid var(--line); background: none; background-image: none; box-shadow: none; text-shadow: none; }
button.inst-disk.on { border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-soft); }
.inst-disk-name { font-size: 14px; font-weight: 500; color: var(--fg); }
.inst-disk-sub { font-size: 12.5px; color: var(--faint); }
.inst-warn { font-size: 13px; color: var(--warn, #ffc14d); }
.inst-log { font-family: monospace; font-size: 11.5px; color: var(--faint); }
"#;

/// A disk's size as the row says it.
fn size_word(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1000.0 { format!("{:.1} TB", gb / 1000.0) } else { format!("{gb:.0} GB") }
}

/// Which disk to offer first: the first that isn't removable.
fn first_choice(disks: &[InstallDisk]) -> Option<usize> {
    disks.iter().position(|d| !d.removable).or((!disks.is_empty()).then_some(0))
}

/// Why the two can't be the password, if they can't; `same` says it in that field's words.
fn pair_problem(one: &str, two: &str, same: &str) -> Option<String> {
    password_problem(one).or_else(|| (one != two).then(|| same.to_string()))
}

struct Fields {
    name: TextField,
    user: TextField,
    pass: TextField,
    pass2: TextField,
    disk_pass: TextField,
    disk_pass2: TextField,
}

struct Ui {
    window: gtk::ApplicationWindow,
    link: Link,
    steps: gtk::Box,
    stack: gtk::Stack,
    back: gtk::Button,
    next: gtk::Button,
    at: Cell<usize>,
    f: Fields,
    disks: RefCell<Vec<InstallDisk>>,
    chosen: Cell<Option<usize>>,
    disk_list: gtk::Box,
    encrypt: wad_gtk::form::Toggle,
    meter: gtk::Label,
    disk_meter: gtk::Label,
    /// The install's page.
    head: gtk::Label,
    sub: gtk::Label,
    progress: gtk::Box,
    job: RefCell<Option<String>>,
    error: RefCell<Option<String>>,
}

fn page(title: &str, body: &str) -> gtk::Box {
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).css_classes(["inst-page"]).build();
    b.append(&kit::label(title, &["inst-title"]));
    if !body.is_empty() {
        let l = kit::label(body, &["inst-body"]);
        l.set_wrap(true);
        b.append(&l);
    }
    b
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
            .title("Install WadSpaces")
            .default_width(760)
            .default_height(620)
            .css_classes(["inst-window"])
            .build();
        kit::title_bar(&window, "hard-drive", "Install WadSpaces");
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        let steps = gtk::Box::builder().spacing(18).css_classes(["inst-steps"]).build();
        let stack = gtk::Stack::builder().vexpand(true).transition_type(gtk::StackTransitionType::None).build();
        let foot = gtk::Box::builder().spacing(10).css_classes(["inst-foot"]).build();
        let back = kit::button("Back", Variant::Ghost, None, false);
        let next = kit::button("Next", Variant::Primary, None, false);
        let gap = gtk::Box::builder().hexpand(true).build();
        foot.append(&back);
        foot.append(&gap);
        foot.append(&next);
        root.append(&steps);
        root.append(&stack);
        root.append(&foot);
        window.set_child(Some(&root));

        let f = Fields {
            name: TextField::new("Name", "studio").named("name"),
            user: TextField::new("Username", "sam").named("username"),
            pass: TextField::password("Password", "At least 8 characters").named("password"),
            pass2: TextField::password("Confirm the password", "").named("password2"),
            disk_pass: TextField::password("Disk password", "At least 8 characters").named("disk-password"),
            disk_pass2: TextField::password("Confirm the disk password", "").named("disk-password2"),
        };
        f.name.set_value("studio");
        let ui = Rc::new_cyclic(|me: &std::rc::Weak<Ui>| {
            let w = me.clone();
            let encrypt = wad_gtk::form::Toggle::new(true, move |_| {
                if let Some(u) = w.upgrade() {
                    u.refresh();
                }
            });
            Ui {
                window,
                link: Link::start(),
                steps,
                stack,
                back,
                next,
                at: Cell::new(0),
                f,
                disks: RefCell::default(),
                chosen: Cell::new(None),
                disk_list: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build(),
                encrypt,
                meter: kit::label("", &["inst-disk-sub"]),
                disk_meter: kit::label("", &["inst-disk-sub"]),
                head: kit::label("Installing WadSpaces", &["inst-title"]),
                sub: kit::label("", &["inst-body"]),
                progress: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build(),
                job: RefCell::default(),
                error: RefCell::default(),
            }
        });
        ui.pages();
        for field in [&ui.f.name, &ui.f.user, &ui.f.pass, &ui.f.pass2, &ui.f.disk_pass, &ui.f.disk_pass2] {
            let u = Rc::downgrade(&ui);
            field.on_change(move |_| {
                if let Some(u) = u.upgrade() {
                    u.refresh();
                }
            });
        }
        let u = Rc::downgrade(&ui);
        ui.next.connect_clicked(move |_| {
            if let Some(u) = u.upgrade() {
                u.forward();
            }
        });
        let u = Rc::downgrade(&ui);
        ui.back.connect_clicked(move |_| {
            if let Some(u) = u.upgrade() {
                u.backward();
            }
        });
        // Enter in a field goes on, but never past Encrypt: erasing a disk takes a click.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let u = Rc::downgrade(&ui);
        keys.connect_key_pressed(move |_, key, _, _| {
            let enter = matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter);
            let Some(u) = u.upgrade().filter(|_| enter) else { return glib::Propagation::Proceed };
            let in_field = gtk::prelude::GtkWindowExt::focus(&u.window).is_some_and(|w| w.is::<gtk::Text>());
            if in_field && u.at.get() < 4 && u.next.is_sensitive() {
                u.forward();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        ui.window.add_controller(keys);
        ui.go(0);
        ui.find_install();
        ui
    }

    fn pages(self: &Rc<Self>) {
        let welcome = page("Welcome to WadSpaces", "Follow the steps to install it on this computer.");
        let mark = wad_gtk::icon::Svg::new(MARK, 72, 72);
        mark.set_halign(gtk::Align::Start);
        welcome.prepend(&mark);
        let computer = page("This computer's name", "");
        computer.append(&self.f.name.root);
        let account = page("Your account", "");
        account.append(&self.f.user.root);
        account.append(&self.f.pass.root);
        account.append(&self.meter);
        account.append(&self.f.pass2.root);
        let disk = page(
            "Where WadSpaces goes",
            "Choose the disk to install on. Everything on it is erased, and WadSpaces becomes the only system on it.",
        );
        disk.append(&self.disk_list);
        let encrypt = page("Encrypt the disk", "");
        let row = gtk::Box::builder().spacing(12).build();
        row.append(&kit::label("Encrypt it with a password", &["inst-disk-name"]));
        let gap = gtk::Box::builder().hexpand(true).build();
        row.append(&gap);
        row.append(&self.encrypt.root);
        encrypt.append(&row);
        encrypt.append(&self.f.disk_pass.root);
        encrypt.append(&self.disk_meter);
        encrypt.append(&self.f.disk_pass2.root);
        let note = kit::label(
            "You type it each time this computer starts. Without it, nothing on the disk can be read.",
            &["inst-body"],
        );
        note.set_wrap(true);
        encrypt.append(&note);
        let install =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).css_classes(["inst-page"]).build();
        install.append(&self.head);
        self.sub.set_wrap(true);
        install.append(&self.sub);
        let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        scroll.set_child(Some(&self.progress));
        install.append(&scroll);
        let done = page("Done", "Everything is ready");
        let pages: [&gtk::Box; 7] = [&welcome, &computer, &account, &disk, &encrypt, &install, &done];
        for (i, p) in pages.into_iter().enumerate() {
            self.stack.add_named(p, Some(STEPS[i]));
        }
    }

    fn go(self: &Rc<Self>, at: usize) {
        self.at.set(at);
        self.stack.set_visible_child_name(STEPS[at]);
        while let Some(c) = self.steps.first_child() {
            self.steps.remove(&c);
        }
        for (i, s) in STEPS.iter().enumerate() {
            let class = if i == at {
                "on"
            } else if i < at {
                "past"
            } else {
                "next"
            };
            self.steps.append(&kit::label(s, &["inst-step", class]));
        }
        if at == 3 {
            self.load_disks();
        }
        self.refresh();
        // Where the keyboard starts: never on Cancel, and never on the button that erases.
        match at {
            1 => self.f.name.focus(),
            2 => self.f.user.focus(),
            4 => self.f.disk_pass.focus(),
            0 | 3 => {
                self.next.grab_focus();
            }
            _ => {}
        }
    }

    /// The fields' words, and whether Next may be pressed.
    fn refresh(&self) {
        let f = &self.f;
        let name = computer_name_problem(&f.name.value());
        f.name.set_error(name.as_deref());
        let user = (!f.user.value().is_empty()).then(|| username_problem(&f.user.value())).flatten();
        f.user.set_error(user.as_deref());
        let (one, two) = (f.pass.value(), f.pass2.value());
        self.meter.set_label(strength(&one).1);
        let differ = two.chars().count() >= one.chars().count() && !two.is_empty() && one != two;
        f.pass2.set_error(differ.then_some("Not the same as the password"));
        let (d1, d2) = (f.disk_pass.value(), f.disk_pass2.value());
        let on = self.encrypt.is_on();
        for w in [&f.disk_pass.root, &f.disk_pass2.root] {
            w.set_visible(on);
        }
        self.disk_meter.set_visible(on);
        self.disk_meter.set_label(strength(&d1).1);
        let differ = d2.chars().count() >= d1.chars().count() && !d2.is_empty() && d1 != d2;
        f.disk_pass2.set_error(differ.then_some("Not the same as the disk password"));

        let at = self.at.get();
        let chosen = self.chosen.get().and_then(|i| self.disks.borrow().get(i).cloned());
        let ok = match at {
            1 => name.is_none(),
            2 => username_problem(&f.user.value()).is_none() && pair_problem(&one, &two, "").is_none(),
            3 => chosen.is_some(),
            4 => !on || pair_problem(&d1, &d2, "").is_none(),
            _ => true,
        };
        self.next.set_sensitive(ok);
        let failed = self.error.borrow().is_some();
        self.next.set_label(match at {
            0 => "Get started",
            4 => "",
            5 if failed => "Retry",
            6 => "Restart",
            _ => "Next",
        });
        let erasing = at == 4;
        for (v, on) in [(Variant::Danger, erasing), (Variant::Primary, !erasing)] {
            if on {
                self.next.add_css_class(v.class());
            } else {
                self.next.remove_css_class(v.class());
            }
        }
        // On Welcome and Disk, Enter is the default button's.
        self.window.set_default_widget((at < 4).then_some(&self.next));
        if erasing {
            let model = chosen.map_or_else(String::new, |d| d.model);
            self.next.set_label(&format!("Erase {model} and install"));
        }
        self.next.set_visible(at != 5 || failed);
        self.back.set_visible(at < 6 && (at != 5 || failed));
        self.back.set_label(match at {
            0 => "Cancel",
            5 => "Change the name or disk",
            _ => "Back",
        });
    }

    fn backward(self: &Rc<Self>) {
        match self.at.get() {
            0 => self.window.close(),
            5 => {
                *self.error.borrow_mut() = None;
                self.go(1);
            }
            n => self.go(n - 1),
        }
    }

    fn forward(self: &Rc<Self>) {
        match self.at.get() {
            4 | 5 => self.start(),
            6 => {
                let _ = std::process::Command::new("systemctl").arg("reboot").spawn();
            }
            n => self.go(n + 1),
        }
    }

    fn load_disks(self: &Rc<Self>) {
        let u = Rc::downgrade(self);
        self.link.call(
            |w| async move { w.install().await },
            move |r| {
                let Some(u) = u.upgrade() else { return };
                match r {
                    Ok(i) => {
                        if u.chosen.get().is_none() {
                            u.chosen.set(first_choice(&i.disks));
                        }
                        *u.disks.borrow_mut() = i.disks;
                        u.draw_disks();
                    }
                    Err(e) => u.none_here(&e.message),
                }
                u.refresh();
            },
        );
    }

    fn none_here(&self, why: &str) {
        while let Some(c) = self.disk_list.first_child() {
            self.disk_list.remove(&c);
        }
        self.disk_list.append(&kit::label(why, &["inst-warn"]));
    }

    fn draw_disks(self: &Rc<Self>) {
        while let Some(c) = self.disk_list.first_child() {
            self.disk_list.remove(&c);
        }
        let disks = self.disks.borrow();
        if disks.is_empty() {
            self.disk_list.append(&kit::label("There's no disk here to install on", &["inst-disk-name"]));
            let l = kit::label(
                "Is it in RAID or Intel RST mode? Set the firmware's storage to AHCI, then start from the stick again.",
                &["inst-body"],
            );
            l.set_wrap(true);
            self.disk_list.append(&l);
            return;
        }
        for (i, d) in disks.iter().enumerate() {
            let on = self.chosen.get() == Some(i);
            let b = gtk::Button::builder().css_classes(["inst-disk"]).build();
            if on {
                b.add_css_class("on");
            }
            let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
            let usb = if d.removable { "  ·  USB" } else { "" };
            col.append(&kit::label(&format!("{} · {}{usb}", d.model, size_word(d.size)), &["inst-disk-name"]));
            let holds = d.holds.as_deref().unwrap_or("empty");
            col.append(&kit::label(&format!("{} · {holds}", d.name), &["inst-disk-sub"]));
            b.set_child(Some(&col));
            let u = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(u) = u.upgrade() {
                    u.chosen.set(Some(i));
                    u.draw_disks();
                    u.refresh();
                }
            });
            self.disk_list.append(&b);
        }
        if let Some(d) = self.chosen.get().and_then(|i| disks.get(i)).filter(|d| d.removable) {
            let l = kit::label(
                &format!(
                    "{} is a USB disk: this computer would only start WadSpaces with it plugged in. Pick an internal disk unless you mean it.",
                    d.model
                ),
                &["inst-warn"],
            );
            l.set_wrap(true);
            self.disk_list.append(&l);
        }
    }

    fn request(&self) -> Option<InstallRequest> {
        let f = &self.f;
        let d = self.chosen.get().and_then(|i| self.disks.borrow().get(i).cloned())?;
        Some(InstallRequest {
            name: f.name.value(),
            username: f.user.value(),
            password: f.pass.value(),
            by_id: d.by_id,
            size: d.size,
            serial: d.serial,
            disk_password: self.encrypt.is_on().then(|| f.disk_pass.value()),
        })
    }

    fn start(self: &Rc<Self>) {
        let Some(req) = self.request() else { return };
        let name = shown_name(&req.name);
        let model =
            self.chosen.get().and_then(|i| self.disks.borrow().get(i).map(|d| (d.model.clone(), d.name.clone())));
        if let Some((model, disk)) = model {
            self.sub.set_label(&format!(
                "{name} goes on {model} ({disk}). Leave the stick in and this computer on until it's done: a few minutes."
            ));
        }
        *self.error.borrow_mut() = None;
        self.head.set_label("Installing WadSpaces");
        self.go(5);
        let u = Rc::downgrade(self);
        self.link.call(
            move |w| async move { w.install_start(&req).await },
            move |r| {
                let Some(u) = u.upgrade() else { return };
                match r {
                    Ok(j) => u.follow(j.job.id),
                    Err(e) => u.failed(e.message),
                }
            },
        );
    }

    fn failed(self: &Rc<Self>, why: String) {
        self.head.set_label("The install stopped");
        while let Some(c) = self.progress.first_child() {
            self.progress.remove(&c);
        }
        let l = kit::label(&format!("{why}. Retry runs the install again."), &["inst-warn"]);
        l.set_wrap(true);
        self.progress.append(&l);
        *self.error.borrow_mut() = Some(why);
        self.refresh();
    }

    /// An install already under way (the window was closed, or this is a second one).
    fn find_install(self: &Rc<Self>) {
        let u = Rc::downgrade(self);
        self.link.call(
            |w| async move { w.install().await },
            move |r| {
                let (Some(u), Ok(i)) = (u.upgrade(), r) else { return };
                if let Some(id) = i.job {
                    u.go(5);
                    u.follow(id);
                }
            },
        );
    }

    fn follow(self: &Rc<Self>, id: String) {
        *self.job.borrow_mut() = Some(id);
        let u = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_secs(1), move || {
            let Some(u) = u.upgrade() else { return glib::ControlFlow::Break };
            let id = u.job.borrow().clone();
            let Some(id) = id else { return glib::ControlFlow::Break };
            let keep = Rc::downgrade(&u);
            u.link.call(
                move |w| async move { w.job(&id, 0).await },
                move |r| {
                    if let (Some(u), Ok(j)) = (keep.upgrade(), r) {
                        u.show_job(&j);
                    }
                },
            );
            glib::ControlFlow::Continue
        });
    }

    fn show_job(self: &Rc<Self>, log: &JobLog) {
        let job = &log.job.job;
        if self.job.borrow().as_deref() != Some(job.id.as_str()) {
            return;
        }
        match job.status {
            JobStatus::Done => {
                *self.job.borrow_mut() = None;
                self.go(6);
                return;
            }
            JobStatus::Error | JobStatus::Cancelled => {
                *self.job.borrow_mut() = None;
                self.failed(job.error.clone().unwrap_or_else(|| "It didn’t finish".into()));
                return;
            }
            _ => {}
        }
        while let Some(c) = self.progress.first_child() {
            self.progress.remove(&c);
        }
        self.progress.append(&kit::stages(job, None));
        let tail = log.lines.len().saturating_sub(LOG_LINES);
        let l = kit::label(&log.lines[tail..].join("\n"), &["inst-log"]);
        l.set_wrap(true);
        l.set_selectable(false);
        self.progress.append(&l);
    }
}

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("wados-installer {}", env!("CARGO_PKG_VERSION"));
        return glib::ExitCode::SUCCESS;
    }
    // `--page <n>`: opens at that step (0 to 6, for screenshots).
    let args: Vec<String> = std::env::args().collect();
    let page = args.iter().position(|a| a == "--page").and_then(|i| args.get(i + 1)?.parse::<usize>().ok());
    let app = gtk::Application::builder().application_id(APP_ID).build();
    let ui: Rc<RefCell<Option<Rc<Ui>>>> = Rc::default();
    app.connect_activate(move |app| {
        let mut slot = ui.borrow_mut();
        let u = slot.get_or_insert_with(|| Ui::new(app));
        if let Some(n) = page.filter(|n| *n < STEPS.len()) {
            u.go(n);
        }
        u.window.present();
    });
    app.run_with_args::<&str>(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(removable: bool) -> InstallDisk {
        InstallDisk {
            by_id: "x".into(),
            name: "x".into(),
            model: "x".into(),
            serial: "x".into(),
            size: 1,
            removable,
            holds: None,
        }
    }

    #[test]
    fn a_disk_reads_in_gb_then_tb() {
        assert_eq!(size_word(512_000_000_000), "512 GB");
        assert_eq!(size_word(2_000_000_000_000), "2.0 TB");
    }

    #[test]
    fn an_internal_disk_is_offered_first() {
        assert_eq!(first_choice(&[disk(true), disk(false)]), Some(1));
        assert_eq!(first_choice(&[disk(true)]), Some(0));
        assert_eq!(first_choice(&[]), None);
    }

    #[test]
    fn a_password_must_be_long_enough_and_typed_twice() {
        assert_eq!(pair_problem("short", "short", "x").as_deref(), Some("At least 8 characters"));
        assert_eq!(pair_problem("long enough", "other", "differ").as_deref(), Some("differ"));
        assert_eq!(pair_problem("long enough", "long enough", "differ"), None);
    }
}
