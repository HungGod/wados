//! The downloads button (Chrome's, in the header bar): it shows once
//! something's been downloaded this run, with a ring round it while
//! something's downloading (how far, all of them together). Its popover
//! lists the recent ones, newest first: each its name, how far it is
//! ("3.1 MB of 10 MB · 4 s left") with a bar, and Cancel while it's going;
//! once it's done, a click opens it and the folder button shows it in its
//! folder (Files, with it picked). Under them, the Downloads folder and
//! every download (`wadbrowser://downloads`).
//!
//! One list for the browser (downloads are each network session's, not a
//! window's: wadbrowser_webkit::downloads); every window's button shows it.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::{gio, glib, prelude::*};
use wadbrowser_webkit::DownloadEvent;

use crate::menus::Run;

/// How many the popover keeps (the page has every one).
const KEEP: usize = 20;
/// A popover that opened by itself (a download started) closes after this, unless the pointer's on it.
const AUTO_CLOSE: Duration = Duration::from_secs(5);

/// Where a download is.
#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// Bytes so far, of `total` (when the server said).
    Running {
        received: u64,
        total: Option<u64>,
    },
    Done {
        size: u64,
    },
    Failed(String),
    Cancelled,
}

/// One download, as the popover shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: u64,
    pub name: String,
    pub path: PathBuf,
    /// The site it came from ("example.com").
    pub host: String,
    /// When it started (ms, monotonic): for the time left.
    pub started_ms: i64,
    pub state: State,
}

/// The ring round the button.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ring {
    /// Nothing downloading.
    Off,
    /// Downloading, how far unknown (no server said how big).
    Unknown,
    /// Downloading, this far (0–1) all together.
    At(f64),
}

/// `ev` into `items` (newest first, at most [`KEEP`]): true if it changed them.
pub fn apply(items: &mut Vec<Item>, ev: &DownloadEvent, now_ms: i64) -> bool {
    match ev {
        DownloadEvent::Started { id, name, path, url } => {
            items.retain(|i| i.id != *id);
            items.insert(
                0,
                Item {
                    id: *id,
                    name: name.clone(),
                    path: path.clone(),
                    host: wadbrowser_core::site::bare_host(&wadbrowser_core::site::host_of(url)),
                    started_ms: now_ms,
                    state: State::Running { received: 0, total: None },
                },
            );
            items.truncate(KEEP);
            true
        }
        DownloadEvent::Progress { id, received, total } => {
            let Some(i) = items.iter_mut().find(|i| i.id == *id) else { return false };
            if !matches!(i.state, State::Running { .. }) {
                return false;
            }
            i.state = State::Running { received: *received, total: *total };
            true
        }
        DownloadEvent::Finished { id, name, path } => {
            let Some(i) = items.iter_mut().find(|i| i.id == *id) else { return false };
            let got = match i.state {
                State::Running { received, .. } => received,
                _ => 0,
            };
            let size = if got > 0 { got } else { std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) };
            i.name = name.clone();
            i.path = path.clone();
            i.state = State::Done { size };
            true
        }
        DownloadEvent::Failed { id, cancelled, error, .. } => {
            let Some(i) = items.iter_mut().find(|i| i.id == *id) else { return false };
            i.state = if *cancelled { State::Cancelled } else { State::Failed(error.clone()) };
            true
        }
    }
}

/// The ring for `items`: how far the ones downloading are, together.
pub fn ring(items: &[Item]) -> Ring {
    let running: Vec<(u64, Option<u64>)> = items
        .iter()
        .filter_map(|i| match i.state {
            State::Running { received, total } => Some((received, total)),
            _ => None,
        })
        .collect();
    if running.is_empty() {
        return Ring::Off;
    }
    if running.iter().any(|(_, t)| t.is_none()) {
        return Ring::Unknown;
    }
    let (got, of) = running.iter().fold((0u64, 0u64), |(g, o), (r, t)| (g + r, o + t.unwrap_or(0)));
    if of == 0 { Ring::Unknown } else { Ring::At((got as f64 / of as f64).clamp(0.0, 1.0)) }
}

/// "3.1 MB", "820 KB", "12 bytes".
pub fn bytes(n: u64) -> String {
    const K: f64 = 1000.0;
    let f = n as f64;
    if f < K {
        format!("{n} byte{}", if n == 1 { "" } else { "s" })
    } else if f < K * K {
        format!("{:.0} KB", f / K)
    } else if f < K * K * K {
        format!("{:.1} MB", f / (K * K))
    } else {
        format!("{:.2} GB", f / (K * K * K))
    }
}

/// "4 s", "3 min", "1 h 5 min".
fn duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{} s", secs.max(1)),
        60..=3599 => format!("{} min", secs.div_ceil(60)),
        _ => format!("{} h {} min", secs / 3600, (secs % 3600) / 60),
    }
}

/// What a download's line under its name says.
pub fn status_line(i: &Item, now_ms: i64) -> String {
    match &i.state {
        State::Running { received, total } => {
            let secs = ((now_ms - i.started_ms).max(1) as f64) / 1000.0;
            let rate = *received as f64 / secs;
            match total {
                Some(t) if *received > 0 && rate > 0.0 => {
                    let left = ((t.saturating_sub(*received)) as f64 / rate).ceil() as u64;
                    format!("{} of {} · {} left", bytes(*received), bytes(*t), duration(left))
                }
                Some(t) => format!("{} of {}", bytes(*received), bytes(*t)),
                None if *received > 0 => format!("{} · {}/s", bytes(*received), bytes(rate as u64)),
                None => "Starting…".into(),
            }
        }
        State::Done { size } if i.host.is_empty() => bytes(*size),
        State::Done { size } => format!("{} · {}", bytes(*size), i.host),
        State::Failed(e) if e.is_empty() => "Failed".into(),
        State::Failed(e) => format!("Failed — {e}"),
        State::Cancelled => "Cancelled".into(),
    }
}

/// How far the bar under its name is (none: no bar).
fn bar(i: &Item) -> Option<Option<f64>> {
    match i.state {
        State::Running { received, total: Some(t) } if t > 0 => Some(Some((received as f64 / t as f64).min(1.0))),
        State::Running { .. } => Some(None),
        _ => None,
    }
}

// ------------------------------------------------------------ the browser's one list

thread_local! {
    static ITEMS: RefCell<Vec<Item>> = const { RefCell::new(Vec::new()) };
    static BUTTONS: RefCell<Vec<Weak<DownloadsButton>>> = const { RefCell::new(Vec::new()) };
}

fn now_ms() -> i64 {
    glib::monotonic_time() / 1000
}

/// A download's news: into the list, and every window's button.
pub fn tell(ev: &DownloadEvent) {
    if !ITEMS.with_borrow_mut(|items| apply(items, ev, now_ms())) {
        return;
    }
    let buttons: Vec<Rc<DownloadsButton>> = BUTTONS.with_borrow_mut(|b| {
        b.retain(|w| w.strong_count() > 0);
        b.iter().filter_map(Weak::upgrade).collect()
    });
    for b in buttons {
        b.sync();
    }
}

// ------------------------------------------------------------ opening

/// `wados-files`, when it's here (on WadOS; not in a wadspace).
fn files_app() -> bool {
    glib::find_program_in_path("wados-files").is_some()
}

fn spawn(args: &[&std::ffi::OsStr]) -> bool {
    match gio::Subprocess::newv(args, gio::SubprocessFlags::NONE) {
        Ok(p) => {
            // Reaped when it's done.
            p.wait_async(gio::Cancellable::NONE, |_| {});
            true
        }
        Err(e) => {
            tracing::warn!(%e, "couldn't start {:?}", args.first());
            false
        }
    }
}

fn launch_uri(uri: &str) {
    if let Err(e) = gio::AppInfo::launch_default_for_uri(uri, None::<&gio::AppLaunchContext>) {
        tracing::info!(%uri, %e, "nothing opened it");
    }
}

/// The file, with the app for its kind.
pub fn open_file(parent: Option<&gtk::Window>, path: &Path) {
    let l = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    let uri = gio::File::for_path(path).uri().to_string();
    l.launch(parent, gio::Cancellable::NONE, move |r| {
        if r.is_err() {
            launch_uri(&uri);
        }
    });
}

/// The folder it's in, with it picked: Files on WadOS, else the file
/// manager (its FileManager1, else whatever opens folders).
pub fn show_in_folder(parent: Option<&gtk::Window>, path: &Path) {
    if files_app() && spawn(&["wados-files".as_ref(), "--select".as_ref(), path.as_os_str()]) {
        return;
    }
    let folder = path.parent().map(|p| gio::File::for_path(p).uri().to_string()).unwrap_or_default();
    let l = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    l.open_containing_folder(parent, gio::Cancellable::NONE, move |r| {
        if r.is_err() && !folder.is_empty() {
            launch_uri(&folder);
        }
    });
}

/// A folder, in Files (or the file manager).
pub fn open_folder(parent: Option<&gtk::Window>, dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    if files_app() && spawn(&["wados-files".as_ref(), dir.as_os_str()]) {
        return;
    }
    let uri = gio::File::for_path(dir).uri().to_string();
    let l = gtk::FileLauncher::new(Some(&gio::File::for_path(dir)));
    l.launch(parent, gio::Cancellable::NONE, move |r| {
        if r.is_err() {
            launch_uri(&uri);
        }
    });
}

// ------------------------------------------------------------ the button

/// What it does that's the window's.
#[derive(Clone)]
pub struct DownloadsActions {
    /// "Show all downloads": wadbrowser://downloads.
    pub show_all: Run,
}

/// One download's row in the popover.
struct Row {
    root: gtk::Box,
    glyph: wad_gtk::Icon,
    name: gtk::Label,
    status: gtk::Label,
    bar: gtk::ProgressBar,
    cancel: gtk::Button,
    folder: gtk::Button,
}

/// The header button and its popover.
pub struct DownloadsButton {
    me: Weak<DownloadsButton>,
    button: gtk::Button,
    ring: Rc<Cell<Ring>>,
    area: gtk::DrawingArea,
    popover: gtk::Popover,
    list: gtk::Box,
    rows: RefCell<Vec<(u64, Row)>>,
    /// The pointer's on the popover (it doesn't close by itself then).
    hovered: Cell<bool>,
    /// The popover's close-by-itself timer.
    closing: RefCell<Option<glib::SourceId>>,
}

impl DownloadsButton {
    pub fn new(actions: DownloadsActions) -> Rc<DownloadsButton> {
        let ring = Rc::new(Cell::new(Ring::Off));
        let area = gtk::DrawingArea::builder().can_target(false).build();
        area.set_size_request(26, 26);
        let r = ring.clone();
        area.set_draw_func(move |a, cr, w, h| draw_ring(a, cr, w, h, r.get()));
        let icon = wad_gtk::Icon::new("download", 16);
        icon.set_halign(gtk::Align::Center);
        icon.set_valign(gtk::Align::Center);
        let over = gtk::Overlay::builder().child(&area).build();
        over.add_overlay(&icon);
        let button = gtk::Button::builder()
            .child(&over)
            .css_classes(["wb-hb", "wb-downloads"])
            .tooltip_text("Downloads")
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        button.update_property(&[gtk::accessible::Property::Label("Downloads")]);

        let col =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["wb-pop-col"]).build();
        col.set_size_request(340, -1);
        col.append(&gtk::Label::builder().label("Recent downloads").xalign(0.0).css_classes(["wb-pop-title"]).build());
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(380)
            .child(&list)
            .build();
        col.append(&scroll);
        col.append(&gtk::Box::builder().css_classes(["wb-menu-sep"]).build());
        let link = |icon: &'static str, label: &str| {
            let inner = gtk::Box::builder().spacing(8).build();
            inner.append(&wad_gtk::Icon::new(icon, 15));
            inner.append(&gtk::Label::new(Some(label)));
            gtk::Button::builder().child(&inner).css_classes(["wb-pop-link"]).build()
        };
        let folder_link = link("folder-open", "Open Downloads folder");
        let all_link = link("download", "Show all downloads");
        col.append(&folder_link);
        col.append(&all_link);

        let popover = gtk::Popover::builder()
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .halign(gtk::Align::End)
            .css_classes(["wb-pop", "wb-downloads-pop"])
            .child(&col)
            .build();
        popover.set_offset(0, 6);
        popover.set_parent(&button);

        let me = Rc::new_cyclic(|me| DownloadsButton {
            me: me.clone(),
            button,
            ring,
            area,
            popover,
            list,
            rows: RefCell::default(),
            hovered: Cell::new(false),
            closing: RefCell::default(),
        });
        let w = Rc::downgrade(&me);
        me.button.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                s.popup(false);
            }
        });
        let hover = gtk::EventControllerMotion::new();
        let w = Rc::downgrade(&me);
        hover.connect_enter(move |_, _, _| {
            if let Some(s) = w.upgrade() {
                s.hovered.set(true);
            }
        });
        let w = Rc::downgrade(&me);
        hover.connect_leave(move |_| {
            if let Some(s) = w.upgrade() {
                s.hovered.set(false);
            }
        });
        me.popover.add_controller(hover);
        let w = Rc::downgrade(&me);
        me.popover.connect_closed(move |_| {
            if let Some(s) = w.upgrade()
                && let Some(id) = s.closing.borrow_mut().take()
            {
                id.remove();
            }
        });
        let w = Rc::downgrade(&me);
        folder_link.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                s.popover.popdown();
                open_folder(s.window().as_ref(), &wadbrowser_webkit::downloads::downloads_dir());
            }
        });
        let (pop, show_all) = (me.popover.downgrade(), actions.show_all.clone());
        all_link.connect_clicked(move |_| {
            if let Some(p) = pop.upgrade() {
                p.popdown();
            }
            let show_all = show_all.clone();
            glib::idle_add_local_once(move || show_all());
        });
        BUTTONS.with_borrow_mut(|b| b.push(Rc::downgrade(&me)));
        me.sync();
        me
    }

    pub fn button(&self) -> &gtk::Button {
        &self.button
    }

    fn window(&self) -> Option<gtk::Window> {
        self.button.root().and_downcast::<gtk::Window>()
    }

    /// Its popover, open. `auto`: a download started (it closes by itself
    /// after a while, unless the pointer's on it, and doesn't take clicks
    /// away from the page meanwhile).
    pub fn popup(&self, auto: bool) {
        if !self.button.is_visible() || !self.button.is_mapped() {
            return;
        }
        if auto && self.popover.is_visible() {
            return;
        }
        if let Some(id) = self.closing.borrow_mut().take() {
            id.remove();
        }
        self.popover.set_autohide(!auto);
        self.popover.popup();
        if auto {
            let w = self.me.clone();
            let id = glib::timeout_add_local(AUTO_CLOSE, move || {
                let Some(s) = w.upgrade() else { return glib::ControlFlow::Break };
                if s.hovered.get() {
                    return glib::ControlFlow::Continue;
                }
                s.closing.borrow_mut().take();
                s.popover.popdown();
                glib::ControlFlow::Break
            });
            *self.closing.borrow_mut() = Some(id);
        }
    }

    /// The list as it is now: its rows, the ring, shown once there's one.
    fn sync(&self) {
        let items = ITEMS.with_borrow(|i| i.clone());
        self.button.set_visible(!items.is_empty());
        let r = ring(&items);
        if self.ring.replace(r) != r || r != Ring::Off {
            self.area.queue_draw();
        }
        if r == Ring::Off {
            self.button.remove_css_class("running");
        } else {
            self.button.add_css_class("running");
        }
        // Gone ones out; new ones in at the top (newest first).
        {
            let mut rows = self.rows.borrow_mut();
            rows.retain(|(id, row)| {
                let keep = items.iter().any(|i| i.id == *id);
                if !keep {
                    self.list.remove(&row.root);
                }
                keep
            });
        }
        for i in items.iter().rev() {
            if !self.rows.borrow().iter().any(|(id, _)| *id == i.id) {
                let row = self.row(i.id);
                self.list.prepend(&row.root);
                self.rows.borrow_mut().push((i.id, row));
            }
        }
        let now = now_ms();
        for (id, row) in self.rows.borrow().iter() {
            if let Some(i) = items.iter().find(|i| i.id == *id) {
                fill(row, i, now);
            }
        }
    }

    fn row(&self, id: u64) -> Row {
        let root = gtk::Box::builder().spacing(10).css_classes(["wb-dl-row"]).build();
        let glyph = wad_gtk::Icon::new("file", 20);
        glyph.add_css_class("wb-dl-glyph");
        glyph.set_valign(gtk::Align::Center);
        root.append(&glyph);
        let words = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(3).hexpand(true).build();
        let name = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .max_width_chars(30)
            .css_classes(["wb-dl-name"])
            .build();
        let status = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(36)
            .css_classes(["wb-pop-small"])
            .build();
        let bar = gtk::ProgressBar::builder().css_classes(["wb-dl-bar"]).build();
        words.append(&name);
        words.append(&status);
        words.append(&bar);
        root.append(&words);
        let small = |icon: &'static str, tip: &str| {
            let b = gtk::Button::builder()
                .child(&wad_gtk::Icon::new(icon, 15))
                .css_classes(["wb-hb", "wb-dl-act"])
                .valign(gtk::Align::Center)
                .tooltip_text(tip)
                .build();
            b.update_property(&[gtk::accessible::Property::Label(tip)]);
            b
        };
        let cancel = small("x", "Cancel");
        let folder = small("folder-open", "Show in folder");
        root.append(&cancel);
        root.append(&folder);
        cancel.connect_clicked(move |_| wadbrowser_webkit::downloads::cancel(id));
        let (b, pop) = (self.button.downgrade(), self.popover.downgrade());
        folder.connect_clicked(move |_| {
            let Some(i) = ITEMS.with_borrow(|i| i.iter().find(|x| x.id == id).cloned()) else { return };
            if let Some(p) = pop.upgrade() {
                p.popdown();
            }
            let win = b.upgrade().and_then(|b| b.root()).and_downcast::<gtk::Window>();
            show_in_folder(win.as_ref(), &i.path);
        });
        // A click on a finished one opens it.
        let click = gtk::GestureClick::builder().button(1).build();
        let (b, pop) = (self.button.downgrade(), self.popover.downgrade());
        click.connect_released(move |g, _, x, y| {
            let Some(i) = ITEMS.with_borrow(|i| i.iter().find(|x| x.id == id).cloned()) else { return };
            if !matches!(i.state, State::Done { .. }) {
                return;
            }
            // Not when it was one of its buttons.
            if let Some(w) = g.widget()
                && let Some(hit) = w.pick(x, y, gtk::PickFlags::DEFAULT)
                && hit.ancestor(gtk::Button::static_type()).is_some()
            {
                return;
            }
            if let Some(p) = pop.upgrade() {
                p.popdown();
            }
            let win = b.upgrade().and_then(|b| b.root()).and_downcast::<gtk::Window>();
            open_file(win.as_ref(), &i.path);
        });
        root.add_controller(click);
        Row { root, glyph, name, status, bar, cancel, folder }
    }
}

/// A row as `i` is now.
fn fill(row: &Row, i: &Item, now: i64) {
    row.name.set_label(&i.name);
    row.root.set_tooltip_text(Some(&i.path.to_string_lossy()));
    row.status.set_label(&status_line(i, now));
    match bar(i) {
        Some(Some(f)) => {
            row.bar.set_visible(true);
            row.bar.set_fraction(f);
        }
        Some(None) => {
            row.bar.set_visible(true);
            row.bar.pulse();
        }
        None => row.bar.set_visible(false),
    }
    let running = matches!(i.state, State::Running { .. });
    let done = matches!(i.state, State::Done { .. });
    row.cancel.set_visible(running);
    row.folder.set_visible(done);
    row.glyph.set(if done { "file" } else { "download" });
    for (c, on) in [("done", done), ("failed", matches!(i.state, State::Failed(_) | State::Cancelled))] {
        if on {
            row.root.add_css_class(c);
        } else {
            row.root.remove_css_class(c);
        }
    }
}

/// The ring: a faint track, and how far in the accent colour.
fn draw_ring(a: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: i32, h: i32, r: Ring) {
    if r == Ring::Off {
        return;
    }
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let radius = (w.min(h) as f64 / 2.0) - 1.5;
    let c = a.color();
    cr.set_line_width(2.0);
    cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, 0.22);
    cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
    let _ = cr.stroke();
    let start = -std::f64::consts::FRAC_PI_2;
    let (from, to) = match r {
        Ring::At(f) => (start, start + std::f64::consts::TAU * f.max(0.02)),
        // How far isn't known: a quarter, turning.
        _ => {
            let turn = (glib::monotonic_time() as f64 / 1_000_000.0) % 1.0 * std::f64::consts::TAU;
            (start + turn, start + turn + std::f64::consts::FRAC_PI_2)
        }
    };
    cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, 0.95);
    cr.arc(cx, cy, radius, from, to);
    let _ = cr.stroke();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(id: u64) -> DownloadEvent {
        DownloadEvent::Started {
            id,
            name: format!("f{id}.zip"),
            path: format!("/home/live/Downloads/f{id}.zip").into(),
            url: "https://www.example.com/f.zip".into(),
        }
    }

    #[test]
    fn a_download_from_start_to_done() {
        let mut items = vec![];
        assert!(apply(&mut items, &started(1), 0));
        assert_eq!(items[0].host, "example.com");
        assert_eq!(status_line(&items[0], 0), "Starting…");
        assert_eq!(ring(&items), Ring::Unknown);
        let p = DownloadEvent::Progress { id: 1, received: 2_000_000, total: Some(10_000_000) };
        assert!(apply(&mut items, &p, 1_000));
        // 2 MB in 1 s: 8 MB to go is 4 s.
        assert_eq!(status_line(&items[0], 1_000), "2.0 MB of 10.0 MB · 4 s left");
        assert_eq!(ring(&items), Ring::At(0.2));
        let path: PathBuf = "/home/live/Downloads/f1.zip".into();
        let done = DownloadEvent::Finished { id: 1, name: "f1.zip".into(), path };
        assert!(apply(&mut items, &done, 2_000));
        assert_eq!(items[0].state, State::Done { size: 2_000_000 });
        assert_eq!(status_line(&items[0], 2_000), "2.0 MB · example.com");
        assert_eq!(ring(&items), Ring::Off);
        // Progress after the end changes nothing.
        assert!(!apply(&mut items, &p, 3_000));
    }

    #[test]
    fn newest_first_and_the_ring_is_all_of_them() {
        let mut items = vec![];
        apply(&mut items, &started(1), 0);
        apply(&mut items, &started(2), 0);
        assert_eq!(items.iter().map(|i| i.id).collect::<Vec<_>>(), [2, 1]);
        apply(&mut items, &DownloadEvent::Progress { id: 1, received: 1, total: Some(4) }, 1);
        apply(&mut items, &DownloadEvent::Progress { id: 2, received: 1, total: Some(4) }, 1);
        assert_eq!(ring(&items), Ring::At(0.25));
        let fail = DownloadEvent::Failed { id: 2, name: "f2.zip".into(), cancelled: true, error: String::new() };
        apply(&mut items, &fail, 2);
        assert_eq!(status_line(&items[0], 2), "Cancelled");
        let fail = DownloadEvent::Failed { id: 1, name: "f1.zip".into(), cancelled: false, error: "Not found".into() };
        apply(&mut items, &fail, 2);
        assert_eq!(status_line(&items[1], 2), "Failed — Not found");
        assert_eq!(ring(&items), Ring::Off);
        for id in 3..40 {
            apply(&mut items, &started(id), 3);
        }
        assert_eq!(items.len(), KEEP);
        assert_eq!(items[0].id, 39);
    }

    #[test]
    fn sizes_and_times_in_words() {
        assert_eq!(bytes(1), "1 byte");
        assert_eq!(bytes(999), "999 bytes");
        assert_eq!(bytes(820_000), "820 KB");
        assert_eq!(bytes(3_140_000), "3.1 MB");
        assert_eq!(bytes(4_700_000_000), "4.70 GB");
        assert_eq!(duration(0), "1 s");
        assert_eq!(duration(61), "2 min");
        assert_eq!(duration(3_900), "1 h 5 min");
    }
}
