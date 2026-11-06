//! Settings ▸ Displays (the spike's `panels/DisplaysPanel.svelte` and
//! `Arrangement.svelte`), on labwc's output management (wad-desk's outputs
//! thread, a Wayland connection of Settings' own):
//!
//! - **Display Arrangement** (two displays or more): the displays as
//!   they're laid out, in points, each with the background; drag one to
//!   where it sits on the desk and it snaps against the others (wad-desk's
//!   `drop_monitor`, the spike's `arrange.ts`). The Primary Display holds
//!   the panel (the shell follows it: it's kept in `displays.toml`); a row
//!   for each display says where it is by the primary.
//! - **Display**, for the display picked in the arrangement (the only one,
//!   else): its orientation, resolution, refresh rate and scale.
//! - **Brightness & Color**: the backlight (logind's SetBrightness, as the
//!   brightness keys), night light and its colour temperature (the
//!   settings: the shell warms the screens).
//!
//! Every change to the displays is tried for 15 s (`form::display_trial`):
//! Keep Changes saves the layout for this set of displays (the shell puts
//! it back each time they're plugged in), anything else puts them back as
//! they were.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::{gio, glib, prelude::*};
use wad_desk::layout::{self, Head, HeadConf, Layout, Layouts, Mon, Transform};
use wad_desk::outputs::{self, Outputs, Said};
use wad_desk::settings::Settings;
use wad_gtk::Icon;
use wad_gtk::form::{self, Choice, Segmented, Slider, Toggle, opt};
use wad_gtk::kit::{self, Group, Row, Tone};

use crate::panel::{Ctx, Panel};
use crate::parts;

const CSS: &str = r#"
.dsp-arr { padding: 12px; }
.dsp-canvas { border-radius: 14px; border: 1px solid var(--line); background-color: var(--bg-2); }
.dsp-mon { border: 2px solid var(--line-strong); border-radius: 8px; background-color: var(--surface-3); box-shadow: 0 10px 30px -14px rgb(0 0 0 / 0.6); }
.dsp-mon.on { border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-soft); }
.dsp-shade { background-color: rgb(0 0 0 / 0.45); }
.dsp-num { min-width: 24px; min-height: 24px; border-radius: 50%; background-color: var(--fg); color: var(--bg); font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 700; font-size: 13px; }
.dsp-mon.on .dsp-num { background-color: var(--accent); }
.dsp-name { font-size: 12px; font-weight: 600; color: var(--fg); text-shadow: 0 1px 6px rgb(0 0 0 / 0.8); }
.dsp-panel { min-height: 7px; background-color: rgb(10 6 20 / 0.85); border-top: 1px solid var(--line-strong); }
.dsp-hint { font-size: 12px; color: var(--faint); }
"#;

const HEIGHT: i32 = 230;
const PAD: f64 = 34.0;
const TRIAL: u32 = 15;

/// The scales offered (the spike's), and the one in use if it's another.
const SCALES: [f64; 4] = [1.0, 1.25, 1.5, 2.0];

pub fn make(_: &Ctx) -> Rc<dyn Panel> {
    Displays::new()
}

/// The arrangement's displays, in points (wad-desk's `Mon`), the primary marked.
pub fn mons(heads: &[Head], primary: &str) -> Vec<Mon> {
    heads
        .iter()
        .filter(|h| h.enabled)
        .map(|h| {
            let (w, hh) = h.logical();
            Mon {
                id: h.name.clone(),
                w: w as f64,
                h: hh as f64,
                x: h.x as f64,
                y: h.y as f64,
                primary: h.name == primary,
            }
        })
        .collect()
}

/// "2736 × 1824 pixels at 200%".
pub fn pixels_at(h: &Head) -> String {
    match h.mode {
        Some(m) => format!("{} × {} pixels at {}%", m.width, m.height, (h.scale * 100.0).round() as i64),
        None => "Off".into(),
    }
}

/// The orientation's options: Landscape and Portrait (and what it is, if it's turned another way).
pub fn orientations(t: Transform) -> Vec<form::Opt<Transform>> {
    let mut o = vec![opt(Transform::Normal, "Landscape"), opt(Transform::R90, "Portrait")];
    if !matches!(t, Transform::Normal | Transform::R90) {
        o.push(opt(t, if t.sideways() { "Portrait (flipped)" } else { "Landscape (flipped)" }));
    }
    o
}

/// What a scale's called ("125%").
pub fn scale_label(s: f64) -> String {
    format!("{}%", (s * 100.0).round() as i64)
}

/// `heads` as they'd be with `id` changed by `f`. If its size in points
/// changed, the displays to its right (or below it) move with its right
/// (or bottom) edge, so the arrangement stays as it was.
pub fn changed(heads: &[Head], id: &str, f: impl FnOnce(&mut Head)) -> Vec<HeadConf> {
    let mut next: Vec<Head> = heads.to_vec();
    let Some(h) = next.iter_mut().find(|h| h.name == id) else { return heads.iter().map(Head::conf).collect() };
    let (x, y) = (h.x, h.y);
    let (w0, h0) = h.logical();
    f(h);
    let (w1, h1) = h.logical();
    let (dw, dh) = (w1 - w0, h1 - h0);
    if (dw, dh) != (0, 0) {
        for o in next.iter_mut().filter(|o| o.name != id && o.enabled) {
            if o.x >= x + w0 {
                o.x += dw;
            }
            if o.y >= y + h0 {
                o.y += dh;
            }
        }
    }
    next.iter().map(Head::conf).collect()
}

/// `heads` with their places from the arrangement `placed`.
pub fn placed(heads: &[Head], placed: &[Mon]) -> Vec<HeadConf> {
    heads
        .iter()
        .map(|h| {
            let mut c = h.conf();
            if let Some(m) = placed.iter().find(|m| m.id == h.name) {
                (c.x, c.y) = (m.x as i32, m.y as i32);
            }
            c
        })
        .collect()
}

/// A change on trial: the heads as they were, the primary as it was (if it
/// changed), and the dialog.
type Trial = (Vec<HeadConf>, Option<String>, form::Open);

/// What to do with an apply's answer.
type Then = Box<dyn FnOnce(Result<(), String>)>;

struct Ui {
    page: kit::Page,
    arr_group: Group,
    canvas: gtk::Fixed,
    area: gtk::DrawingArea,
    primary: Choice<String>,
    rows: gtk::Box,
    display: Group,
    orientation: Choice<Transform>,
    resolution: Choice<(i32, i32)>,
    resolution_row: Row,
    refresh: Choice<i32>,
    scale: RefCell<Option<Segmented<u64>>>,
    scale_row: Row,
    night: Toggle,
    night_row: Row,
    temp_row: Row,
    note: gtk::Label,
}

pub struct Displays {
    me: Weak<Displays>,
    heads: RefCell<Vec<Head>>,
    /// Said once (Settings' --dump waits for it).
    heard: Cell<bool>,
    outputs: RefCell<Option<Outputs>>,
    /// Why there's no output management.
    failed: RefCell<Option<String>>,
    /// The display the Display group is for.
    selected: RefCell<String>,
    /// A change on trial: what to put back (the heads' and the primary).
    trial: RefCell<Option<Trial>>,
    /// Applies waiting for an answer: what to do then.
    pending: RefCell<Vec<(u64, Then)>>,
    ui: RefCell<Option<Rc<Ui>>>,
    window: RefCell<Option<gtk::ApplicationWindow>>,
    /// The arrangement's drag: which, from where (its rectangle's top left).
    drag: RefCell<Option<(String, f64, f64)>>,
    /// The arrangement's points-to-pixels and where it starts.
    k: Cell<(f64, f64, f64, f64, f64)>,
}

impl Displays {
    fn new() -> Rc<Self> {
        let me = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            heads: RefCell::default(),
            heard: Cell::new(false),
            outputs: RefCell::default(),
            failed: RefCell::default(),
            selected: RefCell::default(),
            trial: RefCell::default(),
            pending: RefCell::default(),
            ui: RefCell::default(),
            window: RefCell::default(),
            drag: RefCell::default(),
            k: Cell::new((1.0, 0.0, 0.0, 0.0, 0.0)),
        });
        me.connect();
        me
    }

    fn connect(&self) {
        let (tx, rx) = async_channel::unbounded::<Said>();
        match outputs::start(move |s| {
            let _ = tx.send_blocking(s);
        }) {
            Ok(o) => *self.outputs.borrow_mut() = Some(o),
            Err(e) => {
                *self.failed.borrow_mut() = Some(e);
                self.heard.set(true);
            }
        }
        let w = self.me.clone();
        glib::spawn_future_local(async move {
            while let Ok(s) = rx.recv().await {
                let Some(me) = w.upgrade() else { return };
                me.said(s);
            }
        });
    }

    fn said(&self, s: Said) {
        match s {
            Said::Heads(h) => {
                *self.heads.borrow_mut() = h;
                self.heard.set(true);
                let sel = self.selected.borrow().clone();
                let heads = self.heads.borrow();
                if !heads.iter().any(|h| h.name == sel && h.enabled) {
                    let p = self.primary_now();
                    *self.selected.borrow_mut() = heads
                        .iter()
                        .find(|h| h.name == p)
                        .or(heads.iter().find(|h| h.enabled))
                        .map(|h| h.name.clone())
                        .unwrap_or_default();
                }
                drop(heads);
                self.refresh();
            }
            Said::Applied(id, r) => {
                let i = self.pending.borrow().iter().position(|(x, _)| *x == id);
                if let Some(i) = i {
                    let (_, f) = self.pending.borrow_mut().remove(i);
                    f(r);
                }
            }
            Said::NightFailed(_) => {}
            Said::Ended(e) => {
                *self.failed.borrow_mut() = Some(e);
                self.refresh();
            }
        }
    }

    /// The primary's connector, as the shell picks it (wad-desk's `primary_of`).
    fn primary_now(&self) -> String {
        let env = std::env::var("WADOS_PRIMARY").ok();
        layout::primary_of(&self.heads.borrow(), &Layouts::load(), env.as_deref()).unwrap_or_default()
    }

    fn apply(&self, confs: Vec<HeadConf>, then: impl FnOnce(Result<(), String>) + 'static) {
        let Some(o) = &*self.outputs.borrow() else {
            then(Err(self.failed.borrow().clone().unwrap_or_else(|| "no output management".into())));
            return;
        };
        let id = o.apply(confs, false);
        self.pending.borrow_mut().push((id, Box::new(then)));
    }

    /// A change to the displays (and maybe the primary), on trial.
    fn try_change(&self, confs: Vec<HeadConf>, primary: Option<String>) {
        if self.trial.borrow().is_some() {
            return;
        }
        let heads = self.heads.borrow().clone();
        let before: Vec<HeadConf> = heads.iter().map(Head::conf).collect();
        let was_primary = self.primary_now();
        let same = layout::already(&heads, &confs);
        if same && primary.as_deref().is_none_or(|p| p == was_primary) {
            return;
        }
        tracing::info!(
            "displays: trying {}{}",
            confs.iter().map(|c| format!("{}@{},{}x{}", c.name, c.x, c.y, c.scale)).collect::<Vec<_>>().join(" "),
            primary.as_ref().map(|p| format!(" primary={p}")).unwrap_or_default()
        );
        if let Some(p) = &primary {
            set_kept_primary(&heads, p);
            self.refresh();
        }
        let w = self.me.clone();
        let go = move |r: Result<(), String>| {
            let Some(me) = w.upgrade() else { return };
            if let Err(e) = r {
                me.say(&format!("Couldn't change the displays: {e}"));
                if primary.is_some() {
                    set_kept_primary(&me.heads.borrow(), &was_primary);
                }
                me.refresh();
                return;
            }
            me.say("");
            let Some(win) = me.window.borrow().clone() else { return };
            let w2 = me.me.clone();
            let open = form::display_trial(&win, TRIAL, move |keep| {
                if let Some(me) = w2.upgrade() {
                    me.trial_done(keep);
                }
            });
            *me.trial.borrow_mut() = Some((before.clone(), primary.as_ref().map(|_| was_primary.clone()), open));
        };
        if same { go(Ok(())) } else { self.apply(confs, go) }
    }

    fn trial_done(&self, keep: bool) {
        let Some((before, old_primary, _)) = self.trial.borrow_mut().take() else { return };
        if keep {
            // Kept for this set of displays (the shell puts it back each time they're plugged in).
            let heads = self.heads.borrow().clone();
            let primary = self.primary_now();
            let mut all = Layouts::load();
            let key = heads.iter().find(|h| h.name == primary).map(Head::key);
            all.keep(Layout::of(&heads, key));
            if let Err(e) = all.save() {
                self.say(&format!("Couldn't keep them: {e}"));
            }
            return;
        }
        if let Some(p) = old_primary {
            set_kept_primary(&self.heads.borrow(), &p);
        }
        let w = self.me.clone();
        self.apply(before, move |r| {
            if let (Some(me), Err(e)) = (w.upgrade(), r) {
                me.say(&format!("Couldn't put the displays back: {e}"));
            }
        });
        self.refresh();
    }

    fn say(&self, what: &str) {
        if let Some(ui) = &*self.ui.borrow() {
            ui.note.set_label(what);
            ui.note.set_visible(!what.is_empty());
        }
    }

    fn ui(&self) -> Option<Rc<Ui>> {
        self.ui.borrow().clone()
    }

    /// Everything shown again from the heads.
    fn refresh(&self) {
        let Some(ui) = self.ui() else { return };
        let heads = self.heads.borrow().clone();
        let on: Vec<&Head> = heads.iter().filter(|h| h.enabled).collect();
        let primary = self.primary_now();
        let multi = heads.len() > 1;
        ui.arr_group.root.set_visible(multi);
        if multi {
            ui.primary.set_options(
                heads
                    .iter()
                    .enumerate()
                    .filter(|(_, h)| h.enabled)
                    .map(|(i, h)| opt(h.name.clone(), &format!("{}  {}", i + 1, h.label())))
                    .collect::<Vec<_>>(),
            );
            ui.primary.set(primary.clone());
            self.fill_rows(&ui, &heads, &primary);
            self.lay_out_canvas(&ui);
        }
        let sel = self.selected.borrow().clone();
        let Some(h) = heads.iter().find(|h| h.name == sel).or(on.first().copied()).cloned() else {
            ui.display.root.set_visible(false);
            if let Some(e) = &*self.failed.borrow() {
                self.say(&format!("The displays can't be set from here: {e}"));
            }
            return;
        };
        ui.display.root.set_visible(true);
        ui.orientation.set_options(orientations(h.transform));
        ui.orientation.set(h.transform);
        let sizes = h.sizes();
        ui.resolution
            .set_options(sizes.iter().map(|(w, hh)| opt((*w, *hh), &layout::size_label(*w, *hh))).collect::<Vec<_>>());
        let now = h.mode.map(|m| (m.width, m.height)).unwrap_or_default();
        ui.resolution.set(now);
        parts::set_subtitle(&ui.resolution_row, &format!("Now {} × {}", now.0, now.1));
        let rates = h.refreshes(now);
        let cur = h.mode.map(|m| m.refresh).unwrap_or(0);
        let mut r: Vec<form::Opt<i32>> = rates.iter().map(|x| opt(*x, &layout::refresh_label(*x))).collect();
        if !rates.contains(&cur) {
            r.push(opt(cur, &layout::refresh_label(cur)));
        }
        ui.refresh.set_options(r);
        ui.refresh.set(cur);
        self.scale_control(&ui, &h);
        let (lw, lh) = h.logical();
        parts::set_subtitle(&ui.scale_row, &format!("Makes everything bigger: {lw} × {lh} to work in"));
    }

    fn scale_control(&self, ui: &Rc<Ui>, h: &Head) {
        // Hundredths, so the options compare exactly.
        let key = |s: f64| (s * 100.0).round() as u64;
        let mut opts: Vec<form::Opt<u64>> = SCALES.iter().map(|s| opt(key(*s), &scale_label(*s))).collect();
        if !SCALES.iter().any(|s| key(*s) == key(h.scale)) {
            opts.push(opt(key(h.scale), &scale_label(h.scale)));
        }
        if let Some(old) = ui.scale.borrow_mut().take() {
            ui.scale_row.ctl.remove(&old.root);
        }
        let (w, name) = (self.me.clone(), h.name.clone());
        let seg = Segmented::new(opts, key(h.scale), true, move |v| {
            if let Some(me) = w.upgrade() {
                let s = *v as f64 / 100.0;
                let confs = changed(&me.heads.borrow(), &name, |h| h.scale = s);
                me.try_change(confs, None);
            }
        })
        .named("scale");
        ui.scale_row.ctl.append(&seg.root);
        *ui.scale.borrow_mut() = Some(seg);
    }

    fn fill_rows(&self, ui: &Rc<Ui>, heads: &[Head], primary: &str) {
        while let Some(c) = ui.rows.first_child() {
            ui.rows.remove(&c);
        }
        let ms = mons(heads, primary);
        let sel = self.selected.borrow().clone();
        for (i, h) in heads.iter().enumerate() {
            let on = sel == h.name;
            let row = Row::new(
                Some(if h.built_in() { "laptop" } else { "monitor" }),
                if on { Tone::Accent } else { Tone::Plain },
                &format!("{}  {}", i + 1, h.label()),
                &pixels_at(h),
            )
            .value(&if h.enabled { layout::where_is(&ms, &h.name) } else { "Off".into() }, false)
            .named(&format!("monitor-{}", h.name));
            parts::set_active(&row.root, on);
            let (w, id) = (self.me.clone(), h.name.clone());
            parts::clickable(&row, move || {
                if let Some(me) = w.upgrade() {
                    me.select(&id);
                }
            });
            ui.rows.append(&row.root);
        }
    }

    fn select(&self, id: &str) {
        if *self.selected.borrow() == id {
            return;
        }
        *self.selected.borrow_mut() = id.to_string();
        self.refresh();
    }

    // ------------------------------------------------------------ the arrangement

    /// The displays' tiles, placed for the canvas's width.
    fn lay_out_canvas(&self, ui: &Rc<Ui>) {
        while let Some(c) = ui.canvas.first_child() {
            ui.canvas.remove(&c);
        }
        let heads = self.heads.borrow().clone();
        let primary = self.primary_now();
        let ms = mons(&heads, &primary);
        if ms.is_empty() {
            return;
        }
        let width = ui.area.width().max(400) as f64;
        let (min_x, min_y) =
            (ms.iter().map(|m| m.x).fold(f64::MAX, f64::min), ms.iter().map(|m| m.y).fold(f64::MAX, f64::min));
        let max_x = ms.iter().map(|m| m.x + m.w).fold(f64::MIN, f64::max);
        let max_y = ms.iter().map(|m| m.y + m.h).fold(f64::MIN, f64::max);
        let k = ((width - PAD * 2.0) / (max_x - min_x)).min((HEIGHT as f64 - PAD * 2.0) / (max_y - min_y)) * 0.82;
        let ox = (width - (max_x - min_x) * k) / 2.0;
        let oy = (HEIGHT as f64 - (max_y - min_y) * k) / 2.0;
        self.k.set((k, ox, oy, min_x, min_y));
        let picture = Settings::load().background.picture;
        let sel = self.selected.borrow().clone();
        for (i, m) in ms.iter().enumerate() {
            let n = heads.iter().position(|h| h.name == m.id).map(|j| j + 1).unwrap_or(i + 1);
            let label = heads.iter().find(|h| h.name == m.id).map(Head::label).unwrap_or_default();
            let tile =
                tile(n, &label, m.primary, m.id == sel, &picture, (m.w * k).round() as i32, (m.h * k).round() as i32);
            tile.set_widget_name(&format!("arr-{}", m.id));
            ui.canvas.put(&tile, ox + (m.x - min_x) * k, oy + (m.y - min_y) * k);
        }
    }

    fn tile_at(&self, x: f64, y: f64) -> Option<(String, f64, f64)> {
        let heads = self.heads.borrow().clone();
        let ms = mons(&heads, &self.primary_now());
        let (k, ox, oy, min_x, min_y) = self.k.get();
        ms.iter().rev().find_map(|m| {
            let (l, t) = (ox + (m.x - min_x) * k, oy + (m.y - min_y) * k);
            (x >= l && x <= l + m.w * k && y >= t && y <= t + m.h * k).then(|| (m.id.clone(), l, t))
        })
    }

    fn drag_begin(&self, x: f64, y: f64) {
        let Some((id, l, t)) = self.tile_at(x, y) else { return };
        self.select(&id);
        *self.drag.borrow_mut() = Some((id, l, t));
    }

    fn drag_update(&self, dx: f64, dy: f64) {
        let (Some(ui), Some((id, l, t))) = (self.ui(), self.drag.borrow().clone()) else { return };
        let mut c = ui.canvas.first_child();
        while let Some(w) = c {
            if w.widget_name() == format!("arr-{id}") {
                ui.canvas.move_(&w, l + dx, t + dy);
                w.set_opacity(0.92);
            }
            c = w.next_sibling();
        }
    }

    fn drag_end(&self, dx: f64, dy: f64) {
        let Some((id, _, _)) = self.drag.borrow_mut().take() else { return };
        let Some(ui) = self.ui() else { return };
        if dx.hypot(dy) <= 3.0 {
            self.lay_out_canvas(&ui);
            return;
        }
        let heads = self.heads.borrow().clone();
        let ms = mons(&heads, &self.primary_now());
        let (k, ..) = self.k.get();
        let Some(m) = ms.iter().find(|m| m.id == id) else { return };
        let dropped = layout::drop_monitor(&ms, &id, m.x + dx / k, m.y + dy / k);
        self.lay_out_canvas(&ui);
        self.try_change(placed(&heads, &dropped), None);
    }
}

/// Keeps `connector` as the primary for `heads`' set (the shell follows it at once).
fn set_kept_primary(heads: &[Head], connector: &str) {
    let mut all = Layouts::load();
    all.set_primary(heads, connector);
    if let Err(e) = all.save() {
        tracing::warn!("displays.toml: {e}");
    }
}

/// A display in the arrangement: the background, its number and name, the panel on the primary.
fn tile(n: usize, name: &str, primary: bool, on: bool, picture: &str, w: i32, h: i32) -> gtk::Overlay {
    let o = gtk::Overlay::builder().css_classes(["dsp-mon"]).overflow(gtk::Overflow::Hidden).build();
    if on {
        o.add_css_class("on");
    }
    o.set_size_request(w.max(24), h.max(18));
    let pic = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).can_shrink(true).build();
    if let Some(p) = wad_gtk::assets::path(&format!("wallpapers/art/{picture}.svg")) {
        pic.set_filename(Some(&p));
    }
    o.set_child(Some(&pic));
    o.add_overlay(&gtk::Box::builder().css_classes(["dsp-shade"]).build());
    let num = gtk::Label::builder()
        .label(n.to_string())
        .css_classes(["dsp-num"])
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .margin_top(8)
        .margin_start(8)
        .build();
    o.add_overlay(&num);
    let l = gtk::Label::builder()
        .label(name)
        .css_classes(["dsp-name"])
        .valign(gtk::Align::End)
        .margin_bottom(14)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    o.add_overlay(&l);
    if primary {
        o.add_overlay(&gtk::Box::builder().css_classes(["dsp-panel"]).valign(gtk::Align::End).build());
    }
    o.update_property(&[gtk::accessible::Property::Label(&format!(
        "{n}: {name}{}",
        if primary { " (primary)" } else { "" }
    ))]);
    o.set_can_target(false);
    o
}

/// The backlight (the first in /sys/class/backlight): its name, now, its brightest.
fn backlight() -> Option<(String, u32, u32)> {
    let dir = std::path::Path::new("/sys/class/backlight");
    let mut names: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.file_name()).collect();
    names.sort();
    let name = names.into_iter().next()?.to_string_lossy().into_owned();
    let read = |f: &str| std::fs::read_to_string(dir.join(&name).join(f)).ok()?.trim().parse::<u32>().ok();
    let (now, max) = (read("brightness")?, read("max_brightness")?);
    (max > 0).then_some((name, now, max))
}

/// logind's SetBrightness on this session (no root), as the brightness keys.
fn set_brightness(frac: f64, done: impl FnOnce(Result<(), String>) + 'static) {
    let Some((name, _, max)) = backlight() else { return done(Err("there's no backlight".into())) };
    let to = ((frac.clamp(0.15, 1.0) * max as f64).round() as u32).clamp(1, max);
    let Ok(bus) = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE) else {
        return done(Err("no system bus".into()));
    };
    let call = bus.call_future(
        Some("org.freedesktop.login1"),
        "/org/freedesktop/login1/session/auto",
        "org.freedesktop.login1.Session",
        "SetBrightness",
        Some(&("backlight", name.as_str(), to).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        5000,
    );
    glib::spawn_future_local(async move {
        done(call.await.map(|_| ()).map_err(|e| e.message().to_string()));
    });
}

impl Panel for Displays {
    fn build(&self, ctx: &Ctx) -> gtk::Widget {
        parts::install();
        if let Some(d) = gtk::gdk::Display::default() {
            wad_gtk::theme::add_css(&d, CSS);
        }
        *self.window.borrow_mut() = Some(ctx.window.clone());
        let page = kit::Page::new("Displays", false);
        let note = parts::note("", "set-err");
        page.add(&note);

        // The arrangement.
        let arr_group = Group::new("Display Arrangement", "Drag the displays to match how they sit on your desk.");
        let hint = gtk::Box::builder().spacing(6).css_classes(["dsp-hint"]).build();
        hint.append(&Icon::new("move", 14));
        hint.append(&gtk::Label::new(Some("Drag to move")));
        arr_group.suffix.append(&hint);
        let holder = gtk::Box::builder().css_classes(["dsp-arr"]).build();
        let stack =
            gtk::Overlay::builder().css_classes(["dsp-canvas"]).hexpand(true).overflow(gtk::Overflow::Hidden).build();
        let area = gtk::DrawingArea::builder().hexpand(true).height_request(HEIGHT).build();
        area.set_draw_func(|_, cr, w, h| {
            // The spike's dots, 18 px apart.
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.06);
            let mut y = 1.0;
            while y < h as f64 {
                let mut x = 1.0;
                while x < w as f64 {
                    cr.rectangle(x, y, 1.0, 1.0);
                    x += 18.0;
                }
                y += 18.0;
            }
            let _ = cr.fill();
        });
        stack.set_child(Some(&area));
        let canvas = gtk::Fixed::new();
        canvas.set_widget_name("arrangement");
        stack.add_overlay(&canvas);
        holder.append(&stack);
        arr_group.add(&holder);
        let drag = gtk::GestureDrag::new();
        let w = self.me.clone();
        drag.connect_drag_begin(move |_, x, y| {
            if let Some(me) = w.upgrade() {
                me.drag_begin(x, y);
            }
        });
        let w = self.me.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            if let Some(me) = w.upgrade() {
                me.drag_update(dx, dy);
            }
        });
        let w = self.me.clone();
        drag.connect_drag_end(move |_, dx, dy| {
            if let Some(me) = w.upgrade() {
                me.drag_end(dx, dy);
            }
        });
        stack.add_controller(drag);
        let w = self.me.clone();
        area.connect_resize(move |_, _, _| {
            let w = w.clone();
            glib::idle_add_local_once(move || {
                if let Some(me) = w.upgrade()
                    && let Some(ui) = me.ui()
                {
                    me.lay_out_canvas(&ui);
                }
            });
        });

        let prow = Row::new(None, Tone::Plain, "Primary Display", "It holds the panel, and new windows open on it");
        let w = self.me.clone();
        let primary = Choice::new(Vec::<form::Opt<String>>::new(), String::new(), move |p: &String| {
            if let Some(me) = w.upgrade() {
                let confs: Vec<HeadConf> = me.heads.borrow().iter().map(Head::conf).collect();
                me.try_change(confs, Some(p.clone()));
            }
        })
        .named("primary");
        prow.ctl.append(&primary.root);
        arr_group.add(&prow.root);
        let rows = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        arr_group.add(&rows);
        page.add(&arr_group.root);

        // The display picked.
        let display = Group::new("Display", "");
        let orow = Row::new(None, Tone::Plain, "Orientation", "");
        let w = self.me.clone();
        let orientation = Choice::new(orientations(Transform::Normal), Transform::Normal, move |t: &Transform| {
            if let Some(me) = w.upgrade() {
                let (t, sel) = (*t, me.selected.borrow().clone());
                let confs = changed(&me.heads.borrow(), &sel, |h| h.transform = t);
                me.try_change(confs, None);
            }
        })
        .named("orientation");
        orow.ctl.append(&orientation.root);
        display.add(&orow.root);
        let resolution_row = Row::new(None, Tone::Plain, "Resolution", "Now");
        let w = self.me.clone();
        let resolution = Choice::new(Vec::<form::Opt<(i32, i32)>>::new(), (0, 0), move |sz: &(i32, i32)| {
            if let Some(me) = w.upgrade() {
                let (sz, sel) = (*sz, me.selected.borrow().clone());
                let confs = changed(&me.heads.borrow(), &sel, |h| h.mode = h.mode_at(sz, None).or(h.mode));
                me.try_change(confs, None);
            }
        })
        .named("size");
        resolution_row.ctl.append(&resolution.root);
        display.add(&resolution_row.root);
        let rrow = Row::new(None, Tone::Plain, "Refresh Rate", "");
        let w = self.me.clone();
        let refresh = Choice::new(Vec::<form::Opt<i32>>::new(), 0, move |r: &i32| {
            if let Some(me) = w.upgrade() {
                let (r, sel) = (*r, me.selected.borrow().clone());
                let confs = changed(&me.heads.borrow(), &sel, |h| {
                    if let Some(m) = h.mode {
                        h.mode = h.mode_at((m.width, m.height), Some(r)).or(h.mode);
                    }
                });
                me.try_change(confs, None);
            }
        })
        .named("refresh");
        rrow.ctl.append(&refresh.root);
        display.add(&rrow.root);
        let scale_row = Row::new(None, Tone::Plain, "Scale", "Makes everything bigger");
        display.add(&scale_row.root);
        page.add(&display.root);

        // Brightness and colour.
        let s = Settings::load();
        let color = Group::new("Brightness & Color", "");
        let brow = Row::new(Some("sun"), Tone::Plain, "Brightness", "");
        parts::wide(&brow);
        let dim = Icon::new("sun-dim", 15);
        dim.set_valign(gtk::Align::Center);
        brow.ctl.append(&dim);
        let b = backlight();
        let bright_pct = parts::pct(
            &b.as_ref()
                .map(|(_, n, m)| format!("{}%", (*n as f64 * 100.0 / *m as f64).round() as i64))
                .unwrap_or_default(),
        );
        let (p2, w) = (bright_pct.clone(), self.me.clone());
        let bright = Slider::new(
            0.15,
            1.0,
            0.01,
            b.as_ref().map(|(_, n, m)| *n as f64 / *m as f64).unwrap_or(1.0),
            move |v| {
                p2.set_label(&format!("{}%", (v * 100.0).round() as i64));
                let w = w.clone();
                set_brightness(v, move |r| {
                    if let (Some(me), Err(e)) = (w.upgrade(), r) {
                        me.say(&format!("Couldn't change the brightness: {e}"));
                    }
                });
            },
            |_| {},
        )
        .named("brightness");
        brow.ctl.append(&bright.root);
        brow.ctl.append(&bright_pct);
        brow.root.set_visible(b.is_some());
        color.add(&brow.root);
        let night_row = Row::new(
            Some("moon-star"),
            if s.display.night_light { Tone::Warn } else { Tone::Plain },
            "Night Light",
            "Makes the screen warmer, which can help you sleep",
        );
        let w = self.me.clone();
        let night = Toggle::new(s.display.night_light, move |on| {
            if let Err(e) = Settings::update(|s| s.display.night_light = on) {
                tracing::warn!("settings: {e}");
            }
            if let Some(me) = w.upgrade() {
                me.night_shown(on);
            }
        })
        .named("night-light");
        night_row.ctl.append(&night.root);
        color.add(&night_row.root);
        let temp_row = Row::new(
            None,
            Tone::Plain,
            "Color Temperature",
            if s.display.night_light { "" } else { "Used when night light is on" },
        );
        parts::wide(&temp_row);
        temp_row.ctl.append(&parts::end("Cooler", false));
        let temp = Slider::new(
            0.0,
            1.0,
            0.01,
            s.display.night_temp,
            |_| {},
            |v| {
                if let Err(e) = Settings::update(|s| s.display.night_temp = v) {
                    tracing::warn!("settings: {e}");
                }
            },
        )
        .named("night-temp");
        temp_row.ctl.append(&temp.root);
        temp_row.ctl.append(&parts::end("Warmer", true));
        color.add(&temp_row.root);
        page.add(&color.root);

        let ui = Rc::new(Ui {
            page,
            arr_group,
            canvas,
            area,
            primary,
            rows,
            display,
            orientation,
            resolution,
            resolution_row,
            refresh,
            scale: RefCell::default(),
            scale_row,
            night,
            night_row,
            temp_row,
            note,
        });
        let root = ui.page.root.clone();
        *self.ui.borrow_mut() = Some(ui);
        self.refresh();
        self.night_check();
        root.upcast()
    }

    fn ready(&self) -> bool {
        self.heard.get()
    }
}

impl Displays {
    fn night_shown(&self, on: bool) {
        let Some(ui) = self.ui() else { return };
        let tile = ui.night_row.root.first_child();
        if let Some(t) = tile {
            if on {
                t.remove_css_class("plain");
                t.add_css_class("warn");
            } else {
                t.remove_css_class("warn");
                t.add_css_class("plain");
            }
        }
        parts::set_subtitle(&ui.temp_row, if on { "" } else { "Used when night light is on" });
        let w = self.me.clone();
        // The shell's had a moment to try it.
        glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
            if let Some(me) = w.upgrade() {
                me.night_check();
            }
        });
    }

    /// Asks the shell (`wadsctl desk`) whether night light works here.
    fn night_check(&self) {
        let w = self.me.clone();
        parts::spawn(
            || {
                std::process::Command::new("wadsctl")
                    .arg("desk")
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            },
            move |out| {
                let Some(me) = w.upgrade() else { return };
                let Some(ui) = me.ui() else { return };
                let failed = out
                    .as_deref()
                    .and_then(|o| o.split("(failed: ").nth(1))
                    .map(|r| r.split(") ").next().unwrap_or(r).trim_end_matches(')').to_string());
                if ui.night.is_on()
                    && let Some(e) = failed
                {
                    parts::set_subtitle(&ui.night_row, &format!("It isn't working on this screen: {e}"));
                } else {
                    parts::set_subtitle(&ui.night_row, "Makes the screen warmer, which can help you sleep");
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_desk::layout::Mode;

    fn head(name: &str, w: i32, h: i32, scale: f64, x: i32) -> Head {
        let m = Mode { width: w, height: h, refresh: 60000, preferred: true };
        Head { name: name.into(), enabled: true, modes: vec![m], mode: Some(m), x, y: 0, scale, ..Default::default() }
    }

    #[test]
    fn the_arrangement_in_points() {
        let heads = [head("eDP-1", 3000, 2000, 2.0, 0), head("DP-1", 3840, 2160, 1.5, 1500)];
        let ms = mons(&heads, "eDP-1");
        assert_eq!((ms[0].w, ms[0].h, ms[0].primary), (1500.0, 1000.0, true));
        assert_eq!((ms[1].w, ms[1].x, ms[1].primary), (2560.0, 1500.0, false));
        assert_eq!(pixels_at(&heads[0]), "3000 × 2000 pixels at 200%");
        assert_eq!(layout::where_is(&ms, "DP-1"), "Right of 1");
    }

    #[test]
    fn a_display_made_bigger_still_touches_its_neighbour() {
        let heads = [head("eDP-1", 3000, 2000, 2.0, 0), head("DP-1", 3840, 2160, 1.5, 1500)];
        // At 100% the laptop's 3000 points wide: the monitor moves out of its way.
        let c = changed(&heads, "eDP-1", |h| h.scale = 1.0);
        assert_eq!((c[0].x, c[0].scale), (0, 1.0));
        assert_eq!(c[1].x, 3000);
        // Turned to portrait, the monitor stands 2160 / 1.5 = 1440 wide.
        let c = changed(&heads, "DP-1", |h| h.transform = Transform::R90);
        assert_eq!(c[1].transform, Transform::R90);
        assert_eq!(c[1].x, 1500);
        // The monitor on the left made smaller: the laptop to its right comes in.
        let heads = [head("DP-1", 3840, 2160, 1.5, 0), head("eDP-1", 3000, 2000, 2.0, 2560)];
        let c = changed(&heads, "DP-1", |h| h.scale = 2.0);
        assert_eq!((c[0].x, c[1].x), (0, 1920));
    }

    #[test]
    fn its_words() {
        assert_eq!(scale_label(1.25), "125%");
        assert_eq!(orientations(Transform::Normal).len(), 2);
        assert_eq!(orientations(Transform::R270)[2].label, "Portrait (flipped)");
        assert_eq!(orientations(Transform::R180)[2].label, "Landscape (flipped)");
        let heads = [head("A", 100, 100, 1.0, 0), head("B", 100, 100, 1.0, 100)];
        let p = placed(&heads, &[Mon { id: "B".into(), w: 100.0, h: 100.0, x: -100.0, y: 0.0, primary: false }]);
        assert_eq!((p[0].x, p[1].x), (0, -100));
    }
}
