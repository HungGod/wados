//! The stream sheet (`shell/stream/StreamSheet.svelte`): streaming one of
//! this machine's wadspaces. Its picture and where it is now, Private |
//! Public link, what that means, Secure (only when a VM runtime's here),
//! why it can't start (the button's off, its tooltip saying so), and when
//! it's on this screen, the confirm: "{name} is on this screen. Streaming
//! it takes it off the screen." Then its Stream job's stages as they go;
//! live, the share sheet takes over. Hide leaves it going (its toast says
//! when it's live, or why it didn't start, with Retry).

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use wad_gtk::Icon;
use wad_gtk::kit::{self, Variant};
use wad_model::{JobStatus, SessionDisplay, StreamMode};
use wad_proto::{StreamInfo, StreamRequest};

use super::model;
use crate::overlays::{self, Frame, Sheet, Tone};
use crate::shell::Shell;

const NAME: &str = "stream";
const WIDTH: i32 = 520;

#[derive(Debug, Clone, Default)]
struct St {
    ws: String,
    public: bool,
    secure: bool,
    /// It's on the screen and Stream was pressed: the confirm shows.
    confirming: bool,
    /// The Stream job it started (or found starting).
    job: Option<String>,
    /// Why the start was refused (before a job).
    error: Option<String>,
    /// Its start on its way to wadd.
    busy: bool,
    /// Play together: how many players, or 0.
    players: u8,
}

pub struct StreamSheet {
    shell: Rc<Shell>,
    me: Weak<StreamSheet>,
    st: RefCell<Option<St>>,
    frame: RefCell<Option<Rc<Frame>>>,
    buttons: RefCell<Vec<(&'static str, gtk::Button)>>,
}

impl StreamSheet {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        Rc::new_cyclic(|me| Self {
            shell: shell.clone(),
            me: me.clone(),
            st: RefCell::default(),
            frame: RefCell::default(),
            buttons: RefCell::default(),
        })
    }

    /// Up for this machine's `ws`, at `mode`.
    pub fn open(&self, ws: &str, mode: StreamMode) {
        self.show(St { ws: ws.into(), public: mode.is_public(), secure: mode.is_secure(), ..St::default() });
    }

    /// Up for `ws`'s stream starting here: its job as it goes.
    pub fn open_starting(&self, ws: &str, s: &StreamInfo) {
        self.show(St {
            ws: ws.into(),
            public: s.mode.is_public(),
            secure: s.mode.is_secure(),
            job: s.job.clone().or_else(|| Some(String::new())),
            ..St::default()
        });
    }

    /// Up, and started at once (Retry): `req` as it was asked.
    pub fn start_now(&self, ws: &str, req: StreamRequest) {
        self.open(ws, req.mode);
        self.start(req);
    }

    fn show(&self, st: St) {
        // In place of one of its own that's up (show_sheet doesn't tell it it's gone).
        self.gone();
        let w = self.me.clone();
        let close = move || {
            if let Some(m) = w.upgrade() {
                m.close();
            }
        };
        let frame = Rc::new(overlays::frame("", "radio", Tone::Accent, WIDTH, close));
        frame.card.set_widget_name("stream-sheet");
        *self.st.borrow_mut() = Some(st);
        *self.frame.borrow_mut() = Some(frame.clone());
        self.draw();
        let (w1, w2, w3, w4) = (self.me.clone(), self.me.clone(), self.me.clone(), self.me.clone());
        overlays::show_sheet(Sheet {
            name: NAME,
            card: frame.card.clone().upcast(),
            closed: Rc::new(move || {
                if let Some(m) = w1.upgrade() {
                    m.gone();
                }
            }),
            enter: Rc::new(move || {
                if let Some(m) = w2.upgrade() {
                    m.enter();
                }
            }),
            press: Rc::new(move |what| w3.upgrade().is_some_and(|m| m.press(what))),
            state: Rc::new(move || w4.upgrade().map(|m| m.state()).unwrap_or_default()),
        });
    }

    /// The wadspace it's up for.
    pub fn ws(&self) -> Option<String> {
        self.st.borrow().as_ref().map(|s| s.ws.clone())
    }

    /// Something changed (its job, its stream, its run): drawn again; live, the share sheet takes over.
    pub fn refresh(&self) {
        let Some(st) = self.st.borrow().clone() else { return };
        let store = &self.shell.store;
        let stream = store.stream(&st.ws);
        // Its job, once the stream names it.
        if st.job.as_deref() == Some("")
            && let Some(j) = stream.as_ref().and_then(|s| s.job.clone())
        {
            self.with(|s| s.job = Some(j));
        }
        let done = st.job.is_some()
            && stream.as_ref().is_some_and(|s| s.live)
            && st.job.as_deref().and_then(|j| store.job(j)).is_none_or(|j| j.job.status == JobStatus::Done);
        if done {
            // The share sheet takes its place (which lets this one go).
            super::share(&st.ws);
            if self.st.borrow().as_ref().is_some_and(|s| s.ws == st.ws) {
                self.close();
            }
            return;
        }
        self.draw();
    }

    fn with(&self, f: impl FnOnce(&mut St)) {
        if let Some(s) = self.st.borrow_mut().as_mut() {
            f(s);
        }
    }

    /// Taken down (by its × or Esc, or the dim): it's let go.
    fn gone(&self) {
        self.st.borrow_mut().take();
        self.frame.borrow_mut().take();
        self.buttons.borrow_mut().clear();
    }

    fn close(&self) {
        overlays::hide_sheet(NAME);
        self.gone();
    }

    fn mode(st: &St) -> StreamMode {
        model::mode_of(st.public, st.secure)
    }

    /// On this screen (or in a window here): streaming it takes it off.
    fn on_screen(&self, ws: &str) -> bool {
        matches!(self.shell.store.display(ws), Some(SessionDisplay::Screen | SessionDisplay::Window))
    }

    fn enter(&self) {
        let primary = ["go", "retry", "hide"];
        let b = self.buttons.borrow().iter().find(|(k, _)| primary.contains(k)).map(|(_, b)| b.clone());
        if let Some(b) = b
            && b.is_sensitive()
        {
            b.emit_clicked();
        }
    }

    fn press(&self, what: &str) -> bool {
        match what {
            "private" | "public" => {
                let public = what == "public";
                self.with(|s| s.public = public);
                self.draw();
                return true;
            }
            "secure" => {
                let mode = self.st.borrow().as_ref().map(|s| model::mode_of(s.public, true));
                let Some(mode) = mode else { return false };
                if model::why_not(self.shell.store.stream_modes().as_ref(), mode).is_some()
                    || !model::secure_shown(self.shell.store.stream_modes().as_ref())
                {
                    return false;
                }
                self.with(|s| s.secure = !s.secure);
                self.draw();
                return true;
            }
            _ => {}
        }
        let b = self.buttons.borrow().iter().find(|(k, _)| *k == what).map(|(_, b)| b.clone());
        match b {
            Some(b) if b.is_sensitive() => {
                b.emit_clicked();
                true
            }
            _ => false,
        }
    }

    /// Its main button: the confirm first, if it's on the screen; then it starts.
    pub fn go(&self) {
        let Some(st) = self.st.borrow().clone() else { return };
        let mode = Self::mode(&st);
        if model::why_not(self.shell.store.stream_modes().as_ref(), mode).is_some() || st.busy {
            return;
        }
        let on_screen = self.on_screen(&st.ws);
        if on_screen && !st.confirming {
            self.with(|s| s.confirming = true);
            return self.draw();
        }
        self.start(StreamRequest { mode, takeover: on_screen || st.confirming, players: st.players });
    }

    fn start(&self, req: StreamRequest) {
        let Some(ws) = self.ws() else { return };
        self.with(|s| {
            s.busy = true;
            s.error = None;
        });
        self.draw();
        let Some(streams) = super::get() else { return };
        let w = self.me.clone();
        streams.start(&ws.clone(), req, move |r| {
            let Some(m) = w.upgrade() else { return };
            if m.ws().as_deref() != Some(ws.as_str()) {
                return;
            }
            match r {
                Ok(job) => m.with(|s| {
                    s.busy = false;
                    s.job = Some(job.job.id.clone());
                }),
                // wadd says it's on the screen (this shell hadn't seen it yet): the confirm.
                Err(e) if e.status == 409 && !req.takeover && e.message.contains("screen") => m.with(|s| {
                    s.busy = false;
                    s.confirming = true;
                }),
                Err(e) => m.with(|s| {
                    s.busy = false;
                    s.error = Some(e.message.clone());
                }),
            }
            m.refresh();
        });
    }

    fn retry(&self) {
        let st = self.st.borrow().clone();
        let Some(st) = st else { return };
        self.with(|s| {
            s.job = None;
            s.error = None;
        });
        self.start(StreamRequest {
            mode: Self::mode(&st),
            takeover: st.confirming || self.on_screen(&st.ws),
            players: st.players,
        });
    }

    // ------------------------------------------------------------ drawing

    /// Play together: nobody, or 2 to 4 players, each with a link of their own.
    fn players_row(&self, st: &St) -> gtk::Box {
        let row = gtk::Box::builder().spacing(6).build();
        let label = kit::label("Play together", &["st-what"]);
        label.set_hexpand(true);
        label.set_tooltip_text(Some("A link per player: their controller is the game’s, for their player"));
        row.append(&label);
        for (n, word) in [(0u8, "Off"), (2, "2"), (3, "3"), (4, "4")] {
            let v = if st.players == n { Variant::Primary } else { Variant::Quiet };
            let b = kit::button(word, v, None, true);
            let w = self.me.clone();
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.with(|s| s.players = n);
                    m.draw();
                }
            });
            row.append(&b);
        }
        row
    }

    fn draw(&self) {
        let (Some(st), Some(f)) = (self.st.borrow().clone(), self.frame.borrow().clone()) else { return };
        let store = &self.shell.store;
        let name = super::name_of(store, &st.ws);
        let started = st.job.is_some();
        let mode = Self::mode(&st);
        f.title.set_label(&model::sheet_title(&name, started));
        f.set_tone(if st.public { Tone::Magenta } else { Tone::Accent });
        clear(&f.body);
        clear(&f.foot);
        let mut buttons = vec![];
        let avail = store.stream_modes();
        let why = model::why_not(avail.as_ref(), mode);

        if !started {
            f.body.append(&self.look(&st, &name));
            f.body.append(&self.segmented(st.public));
            f.body.append(&kit::label(model::what_text(st.public), &["st-what"]));
            if model::secure_shown(avail.as_ref()) {
                f.body.append(&self.secure_row(&st, avail.as_ref()));
            }
            let games = store.wadspace(&st.ws).is_some_and(|w| wad_model::games::plays_together(&w));
            if games && st.public && !st.secure {
                f.body.append(&self.players_row(&st));
            }
            if let Some(w) = &why {
                f.body.append(&note("circle-alert", w, "blocked"));
            }
            if let Some(e) = &st.error {
                f.body.append(&note("circle-alert", e, "blocked"));
            }
            if st.confirming {
                f.body.append(&note("monitor", &model::takeover_text(&name), "move"));
            }
            let cancel = kit::button("Cancel", Variant::Ghost, None, false);
            let w = self.me.clone();
            cancel.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.close();
                }
            });
            let g = model::go_button(mode, avail.as_ref(), st.confirming, st.busy);
            let variant = if g.primary { Variant::Primary } else { Variant::Secondary };
            let go = kit::button(&g.label, variant, Some("radio"), false);
            go.set_sensitive(g.enabled);
            go.set_tooltip_text(g.tip.as_deref());
            let w = self.me.clone();
            go.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.go();
                }
            });
            f.foot.append(&cancel);
            f.foot.append(&go);
            buttons.extend([("cancel", cancel), ("go", go)]);
        } else {
            let job = st.job.as_deref().filter(|j| !j.is_empty()).and_then(|j| store.job(j));
            match &job {
                Some(j) => f.body.append(&kit::stages(&j.job, None)),
                None => {
                    let row = gtk::Box::builder().spacing(10).css_classes(["kit-stages"]).build();
                    let sp = gtk::Spinner::builder().spinning(true).build();
                    sp.set_size_request(14, 14);
                    row.append(&sp);
                    row.append(&kit::label("Starting its stream…", &["kit-st", "working"]));
                    f.body.append(&row);
                }
            }
            if let Some(s) = store.stream(&st.ws) {
                for w in model::warnings(&s) {
                    f.body.append(&note("triangle-alert", &w, "warn"));
                }
            }
            let failed = job.as_ref().is_some_and(|j| j.job.status == JobStatus::Error);
            if failed {
                let close = kit::button("Close", Variant::Ghost, None, false);
                let w = self.me.clone();
                close.connect_clicked(move |_| {
                    if let Some(m) = w.upgrade() {
                        m.close();
                    }
                });
                let retry = kit::button("Retry", Variant::Primary, Some("refresh-cw"), false);
                let w = self.me.clone();
                retry.connect_clicked(move |_| {
                    if let Some(m) = w.upgrade() {
                        m.retry();
                    }
                });
                f.foot.append(&close);
                f.foot.append(&retry);
                buttons.extend([("close-it", close), ("retry", retry)]);
            } else {
                let hide = kit::button("Hide", Variant::Ghost, None, false);
                hide.set_tooltip_text(Some("It goes on: a toast says when it’s live"));
                let w = self.me.clone();
                hide.connect_clicked(move |_| {
                    if let Some(m) = w.upgrade() {
                        m.close();
                    }
                });
                f.foot.append(&hide);
                buttons.push(("hide", hide));
            }
        }
        *self.buttons.borrow_mut() = buttons;
    }

    /// Its picture, its name, and where it is now.
    fn look(&self, st: &St, name: &str) -> gtk::Box {
        let store = &self.shell.store;
        let row = gtk::Box::builder().spacing(14).css_classes(["st-look"]).build();
        if let Some(x) = store.wadspace(&st.ws).filter(|x| wad_model::wallpapers::has_picture(&x.look.wallpaper)) {
            let pic = wad_gtk::Picture::new(&x.look.wallpaper, 12.0);
            pic.set_size_request(150, 94);
            pic.set_hexpand(false);
            pic.set_vexpand(false);
            pic.add_css_class("st-thumb");
            pic.set_can_target(false);
            row.append(&pic);
        }
        let col =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).valign(gtk::Align::Center).build();
        let n = gtk::Box::builder().spacing(6).build();
        n.append(&Icon::new("monitor-play", 14));
        n.append(&kit::label(name, &["st-name"]));
        col.append(&n);
        let (icon, where_) = match store.display(&st.ws) {
            Some(SessionDisplay::Screen) => ("monitor", "On this screen"),
            Some(SessionDisplay::Window) => ("app-window", "In a window here"),
            Some(_) => ("play", "Running here"),
            None => ("power", "It starts for the stream"),
        };
        let chip = gtk::Box::builder().spacing(6).css_classes(["st-chip"]).halign(gtk::Align::Start).build();
        chip.append(&Icon::new(icon, 12));
        chip.append(&gtk::Label::new(Some(where_)));
        col.append(&chip);
        row.append(&col);
        row
    }

    /// Private | Public link.
    fn segmented(&self, public: bool) -> gtk::Box {
        let seg = gtk::Box::builder().spacing(2).css_classes(["st-seg"]).halign(gtk::Align::Start).build();
        for (what, label, icon, on) in
            [("private", "Private", "network", !public), ("public", "Public link", "globe", public)]
        {
            let r = gtk::Box::builder().spacing(6).build();
            r.append(&Icon::new(icon, 14));
            r.append(&gtk::Label::new(Some(label)));
            let b = gtk::Button::builder().child(&r).css_classes(["st-seg-b"]).build();
            b.set_widget_name(&format!("seg-{what}"));
            if on {
                b.add_css_class("on");
            }
            let w = self.me.clone();
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.press(what);
                }
            });
            seg.append(&b);
        }
        seg
    }

    /// Secure: in a VM of its own (off, saying why, when this side's can't start).
    fn secure_row(&self, st: &St, avail: Option<&wad_proto::StreamsAvailability>) -> gtk::Box {
        let row = gtk::Box::builder().spacing(12).css_classes(["st-secure"]).build();
        row.set_tooltip_text(Some(model::SECURE_TIP));
        row.append(&kit::icon_tile("shield-check", kit::Tone::Accent, 36, 18));
        let t = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
        t.append(&kit::label("Secure", &["st-secure-t"]));
        t.append(&kit::label(model::SECURE_TEXT, &["st-secure-s"]));
        row.append(&t);
        let why = model::why_not(avail, model::mode_of(st.public, true));
        let sw = gtk::Switch::builder()
            .active(st.secure && why.is_none())
            .sensitive(why.is_none())
            .valign(gtk::Align::Center)
            .css_classes(["st-toggle"])
            .build();
        sw.set_tooltip_text(Some(why.as_deref().unwrap_or(model::SECURE_TIP)));
        let w = self.me.clone();
        sw.connect_state_set(move |_, on| {
            if let Some(m) = w.upgrade() {
                m.with(|s| s.secure = on);
                let w = m.me.clone();
                // Not inside its own signal: it's drawn again.
                gtk::glib::idle_add_local_once(move || {
                    if let Some(m) = w.upgrade() {
                        m.draw();
                    }
                });
            }
            gtk::glib::Propagation::Proceed
        });
        row.append(&sw);
        row
    }

    fn state(&self) -> String {
        let Some(st) = self.st.borrow().clone() else { return "none".into() };
        let mode = Self::mode(&st);
        let why = model::why_not(self.shell.store.stream_modes().as_ref(), mode);
        format!(
            "{} mode={} confirming={} job={} blocked={:?} secure_shown={} buttons={}",
            st.ws,
            model::mode_id(mode),
            st.confirming,
            st.job.as_deref().filter(|j| !j.is_empty()).unwrap_or(if st.job.is_some() { "starting" } else { "none" }),
            why.unwrap_or_default(),
            model::secure_shown(self.shell.store.stream_modes().as_ref()),
            listed(&self.buttons.borrow()),
        )
    }
}

/// A sheet's buttons for `wadsctl dialog`: each by its name, `(off)` after one that can't be pressed.
pub fn listed(buttons: &[(&'static str, gtk::Button)]) -> String {
    let names: Vec<String> =
        buttons.iter().map(|(k, b)| if b.is_sensitive() { k.to_string() } else { format!("{k}(off)") }).collect();
    names.join(",")
}

/// A box of what's in the way, a warning, or the confirm (`.blocked`, `.warn`, `.move`).
pub fn note(icon: &'static str, text: &str, class: &str) -> gtk::Box {
    let b = gtk::Box::builder().spacing(8).css_classes(["st-box", class]).build();
    let i = Icon::new(icon, 16);
    i.set_valign(gtk::Align::Start);
    b.append(&i);
    let l = kit::label(text, &[]);
    l.set_hexpand(true);
    l.set_max_width_chars(56);
    b.append(&l);
    b
}

pub fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}
