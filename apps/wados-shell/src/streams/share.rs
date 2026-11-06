//! The share sheet (`shell/stream/ShareSheet.svelte`): a live stream's QR
//! code (its one-tap link), its mode and transport, what to do with it, its
//! link, and a public one's user and password (masked, with reveal and
//! copy), what to know (Cloudflare's TURN not answering), who's watching
//! since when, Open viewer (WadBrowser), Stop streaming and Done.
//!
//! The share (the stream's key and password) is asked of wadd when the
//! sheet opens and let go when it closes; it's never logged, nor in what
//! `wadsctl dialog` says.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::{glib, prelude::*};
use wad_gtk::Icon;
use wad_gtk::kit::{self, Variant};
use wad_proto::StreamShare;

use super::model;
use super::sheet::{clear, listed, note};
use crate::overlays::{self, Frame, Sheet, Tone};
use crate::shell::Shell;

const NAME: &str = "share";
const WIDTH: i32 = 580;
/// The QR code's side, in points (the spike's 176, its padding in).
const QR: i32 = 164;

struct St {
    key: String,
    share: Option<StreamShare>,
    /// Why it couldn't be had.
    error: Option<String>,
    reveal: bool,
    /// The field copied a moment ago (its check shows).
    copied: Option<&'static str>,
    /// Which ask this is (an answer to an older one is dropped).
    ask: u64,
}

/// What stays while it's up: the QR's tile (drawn once its share comes), and the rest, drawn again.
struct Parts {
    frame: Frame,
    qr: gtk::Box,
    info: gtk::Box,
    after: gtk::Box,
}

pub struct ShareSheet {
    shell: Rc<Shell>,
    me: Weak<ShareSheet>,
    st: RefCell<Option<St>>,
    parts: RefCell<Option<Rc<Parts>>>,
    buttons: RefCell<Vec<(&'static str, gtk::Button)>>,
    asks: std::cell::Cell<u64>,
    tick: RefCell<Option<glib::SourceId>>,
}

impl ShareSheet {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        Rc::new_cyclic(|me| Self {
            shell: shell.clone(),
            me: me.clone(),
            st: RefCell::default(),
            parts: RefCell::default(),
            buttons: RefCell::default(),
            asks: std::cell::Cell::new(0),
            tick: RefCell::default(),
        })
    }

    /// Up for the stream of `key` (this machine's wadspace, or a peer's key): its share is asked for.
    pub fn open(&self, key: &str) {
        // In place of one of its own that's up (show_sheet doesn't tell it it's gone).
        self.gone();
        let ask = self.asks.get() + 1;
        self.asks.set(ask);
        let w = self.me.clone();
        let frame = overlays::frame("", "radio", Tone::Accent, WIDTH, move || {
            if let Some(m) = w.upgrade() {
                m.close();
            }
        });
        frame.card.set_widget_name("share-sheet");
        let row = gtk::Box::builder().spacing(18).build();
        let qr = gtk::Box::builder()
            .css_classes(["st-qr"])
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .hexpand(false)
            .vexpand(false)
            .build();
        qr.set_size_request(QR + 12, QR + 12);
        row.append(&qr);
        let info = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).hexpand(true).build();
        row.append(&info);
        frame.body.append(&row);
        let after = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        frame.body.append(&after);
        *self.parts.borrow_mut() = Some(Rc::new(Parts { frame, qr, info, after }));
        *self.st.borrow_mut() =
            Some(St { key: key.into(), share: None, error: None, reveal: false, copied: None, ask });
        self.draw();
        let card = self.parts.borrow().as_ref().map(|p| p.frame.card.clone()).expect("just made");
        let (w1, w2, w3, w4) = (self.me.clone(), self.me.clone(), self.me.clone(), self.me.clone());
        overlays::show_sheet(Sheet {
            name: NAME,
            card: card.upcast(),
            closed: Rc::new(move || {
                if let Some(m) = w1.upgrade() {
                    m.gone();
                }
            }),
            enter: Rc::new(move || {
                if let Some(m) = w2.upgrade() {
                    m.close();
                }
            }),
            press: Rc::new(move |what| w3.upgrade().is_some_and(|m| m.press(what))),
            state: Rc::new(move || w4.upgrade().map(|m| m.state()).unwrap_or_default()),
        });
        self.fetch(key, ask);
        // "since …" moves on while it's up.
        let w = self.me.clone();
        let id = glib::timeout_add_seconds_local(20, move || match w.upgrade() {
            Some(m) if m.st.borrow().is_some() => {
                m.draw();
                glib::ControlFlow::Continue
            }
            _ => glib::ControlFlow::Break,
        });
        *self.tick.borrow_mut() = Some(id);
    }

    fn fetch(&self, key: &str, ask: u64) {
        // A peer's: asked of it again, in the mode it streams in (it answers its share again).
        let peer = crate::store::split_key(key).map(|(p, w)| (p.to_string(), w.to_string()));
        let mode = self.shell.store.stream(key).map(|s| s.mode).unwrap_or(wad_model::StreamMode::Private);
        let id = key.to_string();
        let w = self.me.clone();
        self.shell.link.call(
            move |wadd| async move {
                match peer {
                    Some((p, ws)) => {
                        wadd.start_peer_stream(&p, &ws, &wad_proto::StreamRequest { mode, takeover: false, players: 0 })
                            .await
                    }
                    None => wadd.stream_share(&id).await,
                }
            },
            move |r| {
                let Some(m) = w.upgrade() else { return };
                {
                    let mut st = m.st.borrow_mut();
                    let Some(st) = st.as_mut().filter(|s| s.ask == ask) else { return };
                    match r {
                        Ok(s) => st.share = Some(s),
                        Err(e) => st.error = Some(e.message),
                    }
                }
                m.draw_qr();
                m.draw();
            },
        );
    }

    /// The streams changed: drawn again (its stream gone, it closes).
    pub fn refresh(&self) {
        let Some(key) = self.st.borrow().as_ref().map(|s| s.key.clone()) else { return };
        if self.shell.store.stream(&key).is_none() {
            return self.close();
        }
        self.draw();
    }

    /// Taken down: the share's let go.
    fn gone(&self) {
        self.st.borrow_mut().take();
        self.parts.borrow_mut().take();
        self.buttons.borrow_mut().clear();
        if let Some(id) = self.tick.borrow_mut().take() {
            id.remove();
        }
    }

    fn close(&self) {
        overlays::hide_sheet(NAME);
        self.gone();
    }

    fn press(&self, what: &str) -> bool {
        let b = self.buttons.borrow().iter().find(|(k, _)| *k == what).map(|(_, b)| b.clone());
        match b {
            Some(b) if b.is_sensitive() => {
                b.emit_clicked();
                true
            }
            _ => false,
        }
    }

    fn draw_qr(&self) {
        let (Some(p), Some(text)) = (
            self.parts.borrow().clone(),
            self.st.borrow().as_ref().and_then(|s| s.share.as_ref().map(|x| model::qr_text(x).to_string())),
        ) else {
            return;
        };
        clear(&p.qr);
        let scale = p.qr.scale_factor().max(self.shell.primary.scale_factor());
        match super::qr::picture(&text, QR, scale) {
            Some(pic) => {
                pic.set_hexpand(true);
                pic.set_vexpand(true);
                p.qr.append(&pic);
            }
            None => p.qr.append(&kit::label("The link's too long for a QR code", &["st-qr-wait"])),
        }
    }

    fn draw(&self) {
        let Some(p) = self.parts.borrow().clone() else { return };
        let store = &self.shell.store;
        let (key, share, error, reveal, copied) = {
            let st = self.st.borrow();
            let Some(st) = st.as_ref() else { return };
            (st.key.clone(), st.share.clone(), st.error.clone(), st.reveal, st.copied)
        };
        let Some(s) = store.stream(&key) else { return };
        let name = super::name_of(store, &key);
        let peer = super::peer_name(store, &s);
        let f = &p.frame;
        f.title.set_label(&model::share_title(&name));
        f.set_tone(if s.mode.is_public() { Tone::Magenta } else { Tone::Accent });
        if share.is_none() {
            clear(&p.qr);
            let wait: gtk::Widget = match &error {
                Some(_) => Icon::new("circle-alert", 24).upcast(),
                None => gtk::Spinner::builder().spinning(true).build().upcast(),
            };
            wait.set_size_request(24, 24);
            wait.set_hexpand(true);
            wait.set_halign(gtk::Align::Center);
            wait.set_valign(gtk::Align::Center);
            wait.add_css_class("st-qr-wait");
            p.qr.append(&wait);
        }

        clear(&p.info);
        let pill = gtk::Box::builder().spacing(6).css_classes(["st-pill"]).halign(gtk::Align::Start).build();
        if s.mode.is_public() {
            pill.add_css_class("pub");
        }
        pill.append(&Icon::new(super::model::mode_icon(s.mode), 13));
        pill.append(&gtk::Label::new(Some(&model::mode_pill(s.mode, s.transport))));
        p.info.append(&pill);
        p.info.append(&kit::label(model::share_text(s.mode), &["st-p"]));
        let mut buttons = vec![];
        match (&share, &error) {
            (Some(sh), _) => {
                for field in model::share_fields(sh) {
                    p.info.append(&self.field(&field, reveal, copied, &mut buttons));
                }
            }
            (None, Some(e)) => p.info.append(&note("circle-alert", &format!("Its link didn’t come: {e}"), "blocked")),
            (None, None) => p.info.append(&kit::label("Asking for its link…", &["st-p"])),
        }

        clear(&p.after);
        for w in model::warnings(&s) {
            p.after.append(&note("triangle-alert", &w, "warn"));
        }
        let status = gtk::Box::builder().spacing(8).css_classes(["st-status"]).build();
        let dot = gtk::Box::builder().css_classes(["st-live-dot"]).valign(gtk::Align::Center).build();
        if !s.mode.is_public() {
            dot.add_css_class("priv");
        }
        status.append(&dot);
        let line = kit::label(&model::status_line(&s, peer.as_deref(), super::now_ms()), &[]);
        line.set_widget_name("share-status");
        status.append(&line);
        p.after.append(&status);

        clear(&f.foot);
        let open = kit::button("Open viewer", Variant::Ghost, Some("external-link"), false);
        open.set_sensitive(share.is_some());
        open.set_tooltip_text(Some("Its link, signed in, in WadBrowser"));
        let (w, title) = (self.me.clone(), format!("{name} · stream"));
        open.connect_clicked(move |_| {
            let Some(m) = w.upgrade() else { return };
            let link = m.st.borrow().as_ref().and_then(|s| s.share.as_ref().map(|x| x.open_link.clone()));
            if let (Some(link), Some(key)) = (link, m.st.borrow().as_ref().map(|s| s.key.clone())) {
                super::open_viewer(&m.shell, &key, &title, &link);
            }
        });
        let stop = kit::button("Stop streaming", Variant::Danger, Some("circle-stop"), false);
        let (w, k) = (self.me.clone(), key.clone());
        stop.connect_clicked(move |_| {
            super::stop(&k);
            if let Some(m) = w.upgrade() {
                m.close();
            }
        });
        let done = kit::button("Done", Variant::Primary, None, false);
        let w = self.me.clone();
        done.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.close();
            }
        });
        f.foot.append(&open);
        f.foot.append(&stop);
        f.foot.append(&done);
        buttons.extend([("open", open), ("stop", stop), ("done", done)]);
        *self.buttons.borrow_mut() = buttons;
    }

    /// A field: its name, what it is (a secret masked until it's shown), and its buttons.
    fn field(
        &self,
        field: &model::Field,
        reveal: bool,
        copied: Option<&'static str>,
        buttons: &mut Vec<(&'static str, gtk::Button)>,
    ) -> gtk::Box {
        let row = gtk::Box::builder().spacing(6).css_classes(["st-field"]).build();
        let l = gtk::Label::builder().label(field.label).xalign(0.0).css_classes(["st-field-l"]).build();
        l.set_size_request(62, -1);
        row.append(&l);
        let shown = if field.secret && !reveal { model::MASK.to_string() } else { field.shown.clone() };
        let code = gtk::Label::builder()
            .label(&shown)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(30)
            .selectable(!field.secret || reveal)
            .css_classes(["st-code"])
            .build();
        code.set_widget_name(&format!("field-{}", field.id));
        row.append(&code);
        if field.secret {
            let b = icon_button(if reveal { "eye-off" } else { "eye" }, if reveal { "Hide it" } else { "Show it" });
            let w = self.me.clone();
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    if let Some(s) = m.st.borrow_mut().as_mut() {
                        s.reveal = !s.reveal;
                    }
                    m.draw();
                }
            });
            row.append(&b);
            buttons.push(("reveal", b));
        }
        let id = field.id;
        let b = icon_button(
            if copied == Some(id) { "check" } else { "copy" },
            &format!("Copy its {}", field.label.to_lowercase()),
        );
        let (w, text) = (self.me.clone(), field.copied.clone());
        b.connect_clicked(move |b| {
            b.clipboard().set_text(&text);
            let Some(m) = w.upgrade() else { return };
            if let Some(s) = m.st.borrow_mut().as_mut() {
                s.copied = Some(id);
            }
            m.draw();
            let w = m.me.clone();
            glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                if let Some(m) = w.upgrade() {
                    let again = match m.st.borrow_mut().as_mut() {
                        Some(s) if s.copied == Some(id) => {
                            s.copied = None;
                            true
                        }
                        _ => false,
                    };
                    if again {
                        m.draw();
                    }
                }
            });
        });
        row.append(&b);
        buttons.push((copy_name(id), b));
        row
    }

    /// What it shows, for `wadsctl dialog`: never the share's link, key or password.
    fn state(&self) -> String {
        let st = self.st.borrow();
        let Some(st) = st.as_ref() else { return "none".into() };
        let s = self.shell.store.stream(&st.key);
        let peer = s.as_ref().and_then(|s| super::peer_name(&self.shell.store, s));
        let fields: Vec<&str> =
            st.share.as_ref().map(|sh| model::share_fields(sh).iter().map(|f| f.id).collect()).unwrap_or_default();
        let qr = st.share.as_ref().and_then(|sh| super::qr::matrix(model::qr_text(sh))).map(|m| m.width).unwrap_or(0);
        format!(
            "{} loaded={} error={:?} fields={} reveal={} qr={qr} warnings={} status={:?} buttons={}",
            st.key,
            st.share.is_some(),
            st.error.clone().unwrap_or_default(),
            fields.join(","),
            st.reveal,
            s.as_ref().map(|s| model::warnings(s).len()).unwrap_or(0),
            s.as_ref().map(|s| model::status_line(s, peer.as_deref(), super::now_ms())).unwrap_or_default(),
            listed(&self.buttons.borrow()),
        )
    }
}

fn copy_name(id: &str) -> &'static str {
    match id {
        "link" => "copy-link",
        "user" => "copy-user",
        "player-1" => "copy-player-1",
        "player-2" => "copy-player-2",
        "player-3" => "copy-player-3",
        "player-4" => "copy-player-4",
        _ => "copy-password",
    }
}

fn icon_button(icon: &'static str, tip: &str) -> gtk::Button {
    gtk::Button::builder()
        .child(&Icon::new(icon, 14))
        .css_classes(["st-fb"])
        .tooltip_text(tip)
        .valign(gtk::Align::Center)
        .build()
}
