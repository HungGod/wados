//! Settings ▸ Storage (the spike's `panels/StoragePanel.svelte`):
//!
//! - **This Disk**: one bar, base images, wadspaces, everything else and
//!   free, over its legend; how much is used, and free;
//! - **Leftovers**: Clean Up (a Cleanup job; what it freed is wadd's notice);
//! - **Wadspaces**: each of this machine's, biggest first, its image and
//!   what it keeps, and whether it's built;
//! - **Base Images**: each with its reference, when it was refreshed and its size.
//!
//! wadd's `storage` event says the disk; the bases come from `GET
//! /v1/bases` (again when a refresh or a cleanup ends); the wadspaces'
//! names from the library's events.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gtk::{glib, prelude::*};
use wad_gtk::form::{self, Hue};
use wad_gtk::kit::{self, Group, Row, Tone, Variant};
use wad_model::{Built, JobKind, JobStatus};
use wad_proto::{Bases, Event, JobInfo, Storage};
use wad_wadd_client::Incoming;

use crate::link::Link;
use crate::panel::{Ctx, Panel};

/// "12.3 GB" (the spike's `gb`, in GB of 10⁹ bytes).
pub fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// The spike's `sizeFromMb`: "820 MB", "2.4 GB" (of 1024 MB).
pub fn size(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb >= 1024.0 { format!("{:.1} GB", mb / 1024.0) } else { format!("{} MB", mb.round() as u64) }
}

/// What a wadspace's row says of its image.
pub fn built_words(b: Built) -> &'static str {
    match b {
        Built::Never => "Not built yet",
        Built::Update => "Rebuilds when it next opens",
        Built::Built => "Built",
    }
}

/// The group's description: "40.0 GB of 512.0 GB used · 472.0 GB free".
pub fn disk_line(s: &Storage) -> String {
    format!("{} of {} used · {} free", gb(s.used()), gb(s.total), gb(s.free))
}

/// The meter's parts.
pub fn parts(s: &Storage) -> Vec<form::Part> {
    vec![
        form::part("Base images", s.bases as f64, Hue::App),
        form::part("Wadspaces", s.wadspaces as f64, Hue::Accent),
        form::part("System and other", s.other as f64, Hue::Magenta),
    ]
}

#[derive(Debug, Default)]
pub struct State {
    pub connected: bool,
    pub storage: Option<Storage>,
    pub bases: Option<Bases>,
    pub machine: String,
    /// This machine's wadspaces' names, by id.
    pub names: HashMap<String, String>,
    pub cleanup: Option<JobInfo>,
}

impl State {
    /// One event: (the page shows what it changed, the bases should be asked again).
    pub fn apply(&mut self, e: &Event) -> (bool, bool) {
        match e {
            Event::Machine(m) => self.machine = m.id.clone(),
            Event::Storage(s) => self.storage = Some((**s).clone()),
            Event::Library(lib) => {
                self.names = lib.wadspaces.iter().map(|w| (w.id.clone(), w.name.clone())).collect();
            }
            Event::Wadspace(w) => {
                self.names.insert(w.id.clone(), w.name.clone());
            }
            Event::Job { job } if job.job.kind == JobKind::Cleanup => self.cleanup = Some((**job).clone()),
            Event::Job { job } if job.job.kind == JobKind::Pull => {
                return (false, !matches!(job.job.status, JobStatus::Running | JobStatus::Queued));
            }
            _ => return (false, false),
        }
        (true, false)
    }

    pub fn cleaning(&self) -> bool {
        self.cleanup.as_ref().is_some_and(|j| matches!(j.job.status, JobStatus::Running | JobStatus::Queued))
    }
}

const CSS: &str = r#"
.sto-usage { padding: 16px; }
"#;

pub struct StoragePanel {
    page: kit::Page,
    note: gtk::Label,
    meter: form::Meter,
    link: Link,
    me: Weak<StoragePanel>,
    pub state: RefCell<State>,
    asking: Cell<bool>,
    queued: Cell<bool>,
}

pub fn make(ctx: &Ctx) -> Rc<dyn Panel> {
    if let Some(d) = gtk::gdk::Display::default() {
        wad_gtk::theme::add_css(&d, CSS);
    }
    let page = kit::Page::new("Storage", false);
    let note = kit::label("", &["kit-group-desc"]);
    note.set_visible(false);
    let meter = form::Meter::new(0.0, vec![], Some("Free"), |n| gb(n as u64));
    meter.root.set_widget_name("storage-meter");
    Rc::new(Wrap(Rc::new_cyclic(|me| StoragePanel {
        page,
        note,
        meter,
        link: ctx.link.clone(),
        me: me.clone(),
        state: RefCell::default(),
        asking: Cell::new(false),
        queued: Cell::new(false),
    })))
}

impl StoragePanel {
    fn queue_draw(&self) {
        if self.queued.replace(true) {
            return;
        }
        let w = self.me.clone();
        glib::idle_add_local_once(move || {
            if let Some(m) = w.upgrade() {
                m.queued.set(false);
                m.draw();
            }
        });
    }

    fn fetch_bases(&self) {
        let w = self.me.clone();
        self.link.call(
            |wadd| async move { wadd.bases().await },
            move |r| {
                let Some(m) = w.upgrade() else { return };
                if let Ok(b) = r {
                    m.state.borrow_mut().bases = Some(b);
                }
                m.queue_draw();
            },
        );
    }

    /// The disk, asked (before its first event, or when the event's late).
    fn fetch_storage(&self) {
        let w = self.me.clone();
        self.link.call(
            |wadd| async move { wadd.storage().await },
            move |r| {
                let Some(m) = w.upgrade() else { return };
                match r {
                    Ok(s) => {
                        let mut st = m.state.borrow_mut();
                        if st.storage.is_none() {
                            st.storage = Some(s);
                        }
                    }
                    Err(e) => {
                        m.note.set_label(&e.message);
                        m.note.set_visible(true);
                    }
                }
                m.queue_draw();
            },
        );
    }

    fn clean_up(&self) {
        self.asking.set(true);
        self.note.set_visible(false);
        self.queue_draw();
        let w = self.me.clone();
        self.link.call(
            |wadd| async move { wadd.storage_cleanup().await },
            move |r| {
                let Some(m) = w.upgrade() else { return };
                m.asking.set(false);
                match r {
                    Ok(j) => m.state.borrow_mut().cleanup = Some(j),
                    Err(e) => {
                        m.note.set_label(&e.message);
                        m.note.set_visible(true);
                    }
                }
                m.queue_draw();
            },
        );
    }

    fn draw(&self) {
        let column = &self.page.column;
        while let Some(c) = column.first_child() {
            column.remove(&c);
        }
        let st = self.state.borrow();
        if !st.connected {
            column.append(&kit::hero(
                "circle-alert",
                "wadd isn't answering",
                "Settings shows this machine's disk once wadd, which keeps its images, answers.",
            ));
            return;
        }
        column.append(&self.note);
        let Some(s) = &st.storage else {
            return;
        };
        let disk = Group::new("This Disk", &disk_line(s));
        disk.root.set_widget_name("group-disk");
        self.meter.set(s.total as f64, parts(s));
        if let Some(p) = self.meter.root.parent() {
            p.downcast::<gtk::Box>().expect("the usage box").remove(&self.meter.root);
        }
        let usage = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["sto-usage"]).build();
        usage.append(&self.meter.root);
        disk.add(&usage);
        column.append(&disk.root);

        let left =
            Group::new("Leftovers", "Old build layers, stopped containers and caches that nothing uses any more.");
        left.root.set_widget_name("group-leftovers");
        let row = Row::new(
            Some("trash"),
            Tone::Plain,
            "Clean Up Leftovers",
            "Your wadspaces, their sign-ins and your files stay as they are",
        )
        .named("cleanup");
        let busy = st.cleaning() || self.asking.get();
        let b = kit::button(if busy { "Cleaning Up…" } else { "Clean Up" }, Variant::Secondary, Some("sparkles"), true);
        b.set_widget_name("cleanup-button");
        b.set_sensitive(!busy);
        let w = self.me.clone();
        b.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.clean_up();
            }
        });
        row.ctl.append(&b);
        left.add(&row.root);
        if let Some(j) =
            st.cleanup.as_ref().filter(|j| j.job.status == JobStatus::Running || j.job.status == JobStatus::Error)
        {
            let inset = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["kit-inset"]).build();
            inset.append(&kit::stages(&j.job, None));
            left.add(&inset);
        }
        column.append(&left.root);

        let ws = Group::new("Wadspaces", "Each one’s set image on this machine.");
        ws.root.set_widget_name("group-wadspaces");
        if s.per_wadspace.is_empty() {
            ws.add(&Row::new(None, Tone::Plain, "No wadspaces on this machine yet", "").root);
        }
        for w in &s.per_wadspace {
            let name = st.names.get(&w.id).cloned().unwrap_or_else(|| w.id.clone());
            let tone = if w.built == Built::Built { Tone::Accent } else { Tone::Plain };
            ws.add(
                &Row::new(Some("layout-grid"), tone, &name, built_words(w.built))
                    .value(&size(w.bytes), false)
                    .named(&format!("ws-{}", w.id))
                    .root,
            );
        }
        column.append(&ws.root);

        if let Some(b) = &st.bases {
            let g = Group::new(
                "Base Images",
                "The Fedora 44 images every wadspace is built on; refreshed every 15 minutes.",
            );
            g.root.set_widget_name("group-bases");
            let now = glib::real_time() / 1000;
            for (i, base) in b.bases.iter().enumerate() {
                let present = !base.missing;
                let sub = if present {
                    format!("{} · refreshed {}", base.image, kit::ago(now, base.refreshed_at.unwrap_or(0)))
                } else {
                    format!("{} · not downloaded", base.image)
                };
                let value = match (present, base.size) {
                    (true, Some(n)) => size(n),
                    _ => "—".into(),
                };
                g.add(
                    &Row::new(Some("package"), if present { Tone::Accent } else { Tone::Plain }, &base.name, &sub)
                        .value(&value, false)
                        .named(&format!("base-{i}"))
                        .root,
                );
            }
            column.append(&g.root);
        }
    }
}

struct Wrap(Rc<StoragePanel>);

impl Panel for Wrap {
    fn build(&self, _: &Ctx) -> gtk::Widget {
        let p = &self.0;
        if p.state.borrow().storage.is_none() && p.state.borrow().connected {
            p.fetch_storage();
        }
        p.draw();
        p.page.root.clone().upcast()
    }

    fn on_event(&self, _: &Ctx, i: &Incoming) {
        let p = &self.0;
        let (changed, bases, first) = match i {
            Incoming::Event(e) => {
                let mut st = p.state.borrow_mut();
                let was = st.connected;
                st.connected = true;
                let ended = matches!(e, Event::Job { job } if job.job.kind == JobKind::Cleanup
                    && !matches!(job.job.status, JobStatus::Running | JobStatus::Queued));
                let (c, b) = st.apply(e);
                (c || !was, b || !was || ended, !was)
            }
            Incoming::Disconnected => (std::mem::replace(&mut p.state.borrow_mut().connected, false), false, false),
            Incoming::Other(..) => (false, false, false),
        };
        if first {
            p.fetch_storage();
        }
        if bases {
            p.fetch_bases();
        }
        if changed {
            p.queue_draw();
        }
    }

    fn ready(&self) -> bool {
        let st = self.0.state.borrow();
        st.connected && st.storage.is_some() && st.bases.is_some()
    }

    fn dump(&self, _: &gtk::Widget) -> String {
        self.0.draw();
        crate::panel::dump_labels(self.0.page.root.upcast_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_disk_in_words() {
        let s = Storage {
            total: 512_000_000_000,
            free: 400_000_000_000,
            bases: 2_720_000_000,
            wadspaces: 8_000_000_000,
            other: 101_280_000_000,
            per_wadspace: vec![],
        };
        assert_eq!(disk_line(&s), "112.0 GB of 512.0 GB used · 400.0 GB free");
        let p = parts(&s);
        assert_eq!(
            p.iter().map(|p| p.label.as_str()).collect::<Vec<_>>(),
            ["Base images", "Wadspaces", "System and other"]
        );
        assert_eq!(size(820 * 1_048_576), "820 MB");
        assert_eq!(size(2_516_582_400), "2.3 GB");
        assert_eq!(built_words(Built::Update), "Rebuilds when it next opens");
        assert_eq!(built_words(Built::Never), "Not built yet");
    }

    #[test]
    fn the_state_takes_its_events() {
        let mut st = State::default();
        let s = Storage { total: 1, ..Storage::default() };
        assert_eq!(st.apply(&Event::Storage(Box::new(s))), (true, false));
        assert!(!st.cleaning());
        assert_eq!(st.apply(&Event::WadspaceGone { id: "x".into() }), (false, false));
    }
}
