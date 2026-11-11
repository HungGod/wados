//! The machine apps' form controls, in the spike's look (`src/ui/` and
//! Settings' `parts/`): a [`Toggle`], a [`Slider`], a drop-down [`Choice`],
//! a [`Segmented`] control, a [`TextField`] (and a password one with
//! reveal), a [`Dialog`] over a window (and the Displays trial's timed one,
//! [`display_trial`]), a stacked [`Meter`] (Storage) and a [`Level`] meter
//! (Sound), an [`Empty`] state and a shortcut's key caps ([`keys`]).
//!
//! Each takes plain data and callbacks; `set` changes what one shows
//! without calling them back. [`crate::kit::install`] adds their CSS.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::{gdk, glib, prelude::*};

use crate::Icon;
use crate::kit::{self, Tone, Variant};

/// Their stylesheet (added by `kit::install`).
pub const CSS: &str = r#"
/* ---- a toggle (ui/Toggle) */
switch.form-toggle {
  padding: 2px;
  border-radius: 999px;
  border: 1px solid var(--line-strong);
  background-color: var(--surface-3);
  background-image: none;
  box-shadow: none;
  outline: none;
  color: transparent;
}
switch.form-toggle > image { opacity: 0; }
switch.form-toggle > slider {
  min-width: 18px;
  min-height: 18px;
  margin: 0;
  border-radius: 50%;
  border: none;
  background-color: var(--fg);
  background-image: none;
  box-shadow: none;
}
switch.form-toggle:checked { background-color: var(--accent); border-color: var(--accent); }
switch.form-toggle:checked > slider { background-color: var(--accent-fg); }
switch.form-toggle:disabled { opacity: 0.4; }
switch.form-toggle:focus-visible { box-shadow: 0 0 0 2px var(--accent); }

/* ---- a slider (ui/Slider) */
scale.form-slider { padding: 8px 0; margin: 0; min-height: 6px; }
scale.form-slider > trough {
  min-height: 6px;
  border-radius: 99px;
  border: none;
  outline: none;
  box-shadow: none;
  background-image: none;
  background-color: var(--surface-3);
}
scale.form-slider > trough > highlight {
  min-height: 6px;
  min-width: 0;
  margin: 0;
  border-radius: 99px;
  border: none;
  box-shadow: none;
  background-image: none;
  background-color: var(--accent);
}
scale.form-slider > trough > slider {
  min-width: 12px;
  min-height: 12px;
  margin: -6px;
  border-radius: 50%;
  border: 3px solid var(--bg);
  background-image: none;
  background-color: var(--fg);
  box-shadow: 0 0 0 1px var(--line-strong);
}
scale.form-slider:disabled { opacity: 0.4; }

/* ---- a drop-down (parts/Choice) and its menu (ui/MenuList) */
button.form-choice {
  min-height: 32px;
  padding: 0 10px 0 12px;
  border-radius: 10px;
  border: 1px solid var(--line-strong);
  background-color: var(--surface-3);
  background-image: none;
  box-shadow: none;
  outline: none;
  text-shadow: none;
  color: var(--fg);
  font-size: 13.5px;
  font-weight: 400;
}
button.form-choice:hover { border-color: var(--accent); }
button.form-choice wad-icon { color: var(--muted); }
button.form-choice:disabled { opacity: 0.45; }
popover.form-menu { background: none; box-shadow: none; padding: 0; }
popover.form-menu > contents {
  min-width: 208px;
  padding: 6px;
  border-radius: 14px;
  border: 1px solid var(--line-strong);
  background-color: var(--surface-2);
  box-shadow: 0 18px 48px -12px var(--shadow);
  color: var(--fg);
}
button.form-menu-item {
  min-height: 34px;
  padding: 0 10px;
  border-radius: 9px;
  border: none;
  background: none;
  background-image: none;
  box-shadow: none;
  outline: none;
  text-shadow: none;
  color: var(--fg);
  font-weight: 400;
  font-size: 14px;
}
button.form-menu-item:hover, button.form-menu-item:focus-visible { background-color: var(--surface-3); }
button.form-menu-item .form-menu-ic { min-width: 16px; color: var(--muted); }
button.form-menu-item.on .form-menu-ic { color: var(--muted); }
.form-menu-hint { font-size: 11.5px; color: var(--faint); }

/* ---- a segmented control (ui/Segmented) */
.form-seg { padding: 3px; border-radius: 12px; border: 1px solid var(--line); background-color: var(--surface); }
button.form-seg-item {
  min-height: 30px;
  min-width: 0;
  padding: 0 12px;
  border-radius: 9px;
  border: none;
  background: none;
  background-image: none;
  box-shadow: none;
  outline: none;
  text-shadow: none;
  color: var(--muted);
  font-weight: 400;
  font-size: 13px;
}
.form-seg.sm button.form-seg-item { min-height: 26px; padding: 0 10px; font-size: 12px; }
button.form-seg-item:hover { color: var(--fg); }
button.form-seg-item.on { background-color: var(--surface-3); color: var(--fg); box-shadow: inset 0 0 0 1px var(--line-strong); }

/* ---- a text field (ui/TextField) */
.form-field-label { font-size: 12px; font-weight: 500; color: var(--muted); }
.form-input {
  min-height: 40px;
  padding: 0 14px;
  border-radius: 12px;
  border: 1px solid var(--line-strong);
  background-color: var(--bg-2);
}
.form-input.has-reveal { padding-right: 5px; }
.form-input:focus-within { border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-soft); }
.form-field.has-error .form-input { border-color: var(--danger); }
.form-input > text { background: none; color: var(--fg); caret-color: var(--fg); font-size: 14px; }
.form-input > text.mono { font-family: monospace; font-size: 13px; }
.form-input > text > placeholder { color: var(--faint); }
.form-input > text > selection { background-color: var(--accent-soft); color: var(--fg); }
button.form-reveal {
  min-width: 30px;
  min-height: 30px;
  padding: 0;
  border-radius: 8px;
  border: none;
  background: none;
  background-image: none;
  box-shadow: none;
  color: var(--muted);
}
button.form-reveal:hover { background-color: var(--surface-3); color: var(--fg); }
.form-error { font-size: 12px; color: var(--danger); }

/* ---- a dialog over its window (ui/Dialog) */
.form-scrim { background-color: var(--dim); }
.form-dialog {
  border-radius: 24px;
  border: 1px solid var(--line-strong);
  background-color: var(--surface);
  box-shadow: 0 30px 80px -20px var(--shadow);
}
.form-dialog-head { padding: 20px 20px 0; }
.form-dialog-title { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 18px; letter-spacing: -0.01em; }
.form-badge { min-width: 36px; min-height: 36px; border-radius: 12px; }
.form-badge.accent { background-color: var(--accent-soft); color: var(--accent); }
.form-badge.danger { background-color: var(--danger-soft); color: var(--danger); }
.form-badge.warn { background-color: var(--warn-soft); color: var(--warn); }
.form-badge.magenta { background-color: var(--accent-2-soft); color: var(--accent-2); }
.form-badge.plain { background-color: var(--surface-3); color: var(--muted); }
button.form-x {
  min-width: 30px;
  min-height: 30px;
  padding: 0;
  border-radius: 8px;
  border: none;
  background: none;
  background-image: none;
  box-shadow: none;
  color: var(--muted);
}
button.form-x:hover { background-color: var(--surface-2); color: var(--fg); }
.form-dialog-body { padding: 16px 20px 20px; color: var(--muted); }
.form-dialog-foot { padding: 0 20px 20px; }

/* ---- a stacked meter (Storage's bar) and a level meter (Sound's) */
.form-legend { font-size: 12.5px; color: var(--muted); }
.form-legend-value { color: var(--fg); font-weight: 500; }
.form-sw { min-width: 10px; min-height: 10px; border-radius: 3px; }
.form-sw.app { background-color: var(--c-app); }
.form-sw.accent { background-color: var(--accent); }
.form-sw.magenta { background-color: var(--accent-2); }
.form-sw.warn { background-color: var(--warn); }
.form-sw.danger { background-color: var(--danger); }
.form-sw.plain { background-color: var(--muted); }
.form-sw.free { background-color: var(--surface-3); box-shadow: inset 0 0 0 1px var(--line-strong); }
.form-lvl { min-height: 12px; border-radius: 2px; background-color: var(--surface-3); }
.form-lvl.lit { background-color: var(--accent); }
.form-lvl.lit.hot { background-color: var(--warn); }

/* ---- an empty state (parts/Empty) */
.form-empty { padding: 48px 16px; }
.form-empty-ic { min-width: 84px; min-height: 84px; border-radius: 50%; background-color: var(--surface-2); color: var(--faint); margin-bottom: 8px; }
.form-empty-title { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 18px; letter-spacing: -0.01em; }
.form-empty-text { color: var(--muted); margin-bottom: 8px; }

/* ---- key caps (parts/Keys) */
.form-key {
  min-width: 10px;
  min-height: 23px;
  padding: 0 7px;
  border-radius: 7px;
  border: 1px solid var(--line-strong);
  border-bottom-width: 2px;
  background-color: var(--surface-3);
  color: var(--fg);
  font-weight: 600;
  font-size: 12px;
}
"#;

// ------------------------------------------------------------ options

/// One of a [`Choice`]'s or a [`Segmented`]'s options: its value, what it
/// says, a hint under it (a Choice's menu) and an icon (a Segmented's).
#[derive(Debug, Clone, PartialEq)]
pub struct Opt<T> {
    pub value: T,
    pub label: String,
    pub hint: Option<String>,
    pub icon: Option<&'static str>,
}

/// An option: `opt(60, "60.00 Hz")`.
pub fn opt<T>(value: T, label: &str) -> Opt<T> {
    Opt { value, label: label.into(), hint: None, icon: None }
}

impl<T> Opt<T> {
    pub fn hint(mut self, hint: &str) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// A (label, value) pair is an option.
impl<T, S: Into<String>> From<(S, T)> for Opt<T> {
    fn from((label, value): (S, T)) -> Self {
        Opt { value, label: label.into(), hint: None, icon: None }
    }
}

/// What `value` says among `options` ("—" when it's none of them).
pub fn label_for<T: PartialEq>(options: &[Opt<T>], value: &T) -> String {
    options.iter().find(|o| o.value == *value).map(|o| o.label.clone()).unwrap_or_else(|| "—".into())
}

fn options<T>(options: impl IntoIterator<Item = impl Into<Opt<T>>>) -> Vec<Opt<T>> {
    options.into_iter().map(Into::into).collect()
}

// ------------------------------------------------------------ toggle

/// A switch (`ui/Toggle.svelte`): `changed` when it's flipped.
#[derive(Clone)]
pub struct Toggle {
    pub root: gtk::Switch,
    quiet: Rc<Cell<bool>>,
}

impl Toggle {
    pub fn new(on: bool, changed: impl Fn(bool) + 'static) -> Self {
        let root = gtk::Switch::builder().active(on).valign(gtk::Align::Center).css_classes(["form-toggle"]).build();
        let quiet = Rc::new(Cell::new(false));
        let q = quiet.clone();
        root.connect_active_notify(move |s| {
            if !q.get() {
                changed(s.is_active());
            }
        });
        Self { root, quiet }
    }

    pub fn is_on(&self) -> bool {
        self.root.is_active()
    }

    /// Shows `on` without calling back (wadd said so).
    pub fn set(&self, on: bool) {
        self.quiet.set(true);
        self.root.set_active(on);
        self.quiet.set(false);
    }

    pub fn named(self, name: &str) -> Self {
        self.root.set_widget_name(name);
        self
    }
}

// ------------------------------------------------------------ slider

/// `v` on the slider's steps from `min`, inside `min..=max`.
pub fn snap(v: f64, min: f64, max: f64, step: f64) -> f64 {
    let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
    let v = v.clamp(lo, hi);
    if step <= 0.0 {
        return v;
    }
    let s = lo + ((v - lo) / step).round() * step;
    // Ten digits: steps of 0.01 add up to 0.30000000000000004 otherwise.
    ((s.clamp(lo, hi)) * 1e10).round() / 1e10
}

/// How far along `min..max` `v` is, 0 to 1.
pub fn fraction(v: f64, min: f64, max: f64) -> f64 {
    if max == min {
        return 0.0;
    }
    ((v - min) / (max - min)).clamp(0.0, 1.0)
}

/// A slider (`ui/Slider.svelte`): `live` as it moves, `committed` once
/// it's let go (or, by keys or the wheel, once it's settled a moment).
#[derive(Clone)]
pub struct Slider {
    pub root: gtk::Scale,
    quiet: Rc<Cell<bool>>,
    last: Rc<Cell<f64>>,
}

impl Slider {
    pub fn new(
        min: f64,
        max: f64,
        step: f64,
        value: f64,
        live: impl Fn(f64) + 'static,
        committed: impl Fn(f64) + 'static,
    ) -> Self {
        let adj = gtk::Adjustment::new(snap(value, min, max, step), min, max, step, step * 10.0, 0.0);
        let root = gtk::Scale::builder()
            .orientation(gtk::Orientation::Horizontal)
            .adjustment(&adj)
            .draw_value(false)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .css_classes(["form-slider"])
            .build();
        root.set_size_request(160, -1);
        let quiet = Rc::new(Cell::new(false));
        let last = Rc::new(Cell::new(adj.value()));
        let held = Rc::new(Cell::new(false));
        let settle: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        let committed: Rc<dyn Fn(f64)> = Rc::new(committed);

        root.connect_change_value(move |r, _, v| {
            r.set_value(snap(v, min, max, step));
            glib::Propagation::Stop
        });
        let commit = {
            let last = last.clone();
            let committed = committed.clone();
            move |v: f64| {
                if v != last.get() {
                    last.set(v);
                    committed(v);
                }
            }
        };
        let commit: Rc<dyn Fn(f64)> = Rc::new(commit);
        {
            let (q, held, settle, commit) = (quiet.clone(), held.clone(), settle.clone(), commit.clone());
            root.connect_value_changed(move |r| {
                if q.get() {
                    return;
                }
                let v = r.value();
                live(v);
                if held.get() {
                    return;
                }
                // Keys and the wheel: committed once it's still for a moment.
                if let Some(id) = settle.borrow_mut().take() {
                    id.remove();
                }
                let (s, c) = (settle.clone(), commit.clone());
                let id = glib::timeout_add_local_once(Duration::from_millis(350), move || {
                    s.borrow_mut().take();
                    c(v);
                });
                *settle.borrow_mut() = Some(id);
            });
        }
        // A press and its release: the drag between is live only.
        let legacy = gtk::EventControllerLegacy::new();
        legacy.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let r = root.downgrade();
            legacy.connect_event(move |_, e| {
                match e.event_type() {
                    gdk::EventType::ButtonPress | gdk::EventType::TouchBegin => held.set(true),
                    gdk::EventType::ButtonRelease | gdk::EventType::TouchEnd | gdk::EventType::TouchCancel
                        if held.replace(false) =>
                    {
                        if let Some(r) = r.upgrade() {
                            let commit = commit.clone();
                            // After the scale's own release has set its value.
                            glib::idle_add_local_once(move || commit(r.value()));
                        }
                    }
                    _ => {}
                }
                glib::Propagation::Proceed
            });
        }
        root.add_controller(legacy);
        Self { root, quiet, last }
    }

    pub fn value(&self) -> f64 {
        self.root.value()
    }

    /// Shows `v` without calling back.
    pub fn set(&self, v: f64) {
        self.quiet.set(true);
        self.root.set_value(v);
        self.last.set(self.root.value());
        self.quiet.set(false);
    }

    pub fn named(self, name: &str) -> Self {
        self.root.set_widget_name(name);
        self
    }
}

// ------------------------------------------------------------ choice

/// A drop-down (`parts/Choice.svelte`, GNOME's combo row button): the
/// current option's label and a chevron; its menu, under it, checks the
/// current one. `changed` when another's picked.
pub struct Choice<T: Clone + PartialEq + 'static> {
    pub root: gtk::Button,
    inner: Rc<ChoiceInner<T>>,
}

impl<T: Clone + PartialEq + 'static> Clone for Choice<T> {
    fn clone(&self) -> Self {
        Self { root: self.root.clone(), inner: self.inner.clone() }
    }
}

struct ChoiceInner<T> {
    options: RefCell<Vec<Opt<T>>>,
    value: RefCell<T>,
    label: gtk::Label,
    popover: gtk::Popover,
    list: gtk::Box,
    changed: Box<dyn Fn(&T)>,
}

impl<T: Clone + PartialEq + 'static> Choice<T> {
    pub fn new(options: impl IntoIterator<Item = impl Into<Opt<T>>>, value: T, changed: impl Fn(&T) + 'static) -> Self {
        let options = self::options(options);
        let root = gtk::Button::builder().valign(gtk::Align::Center).css_classes(["form-choice"]).build();
        let inner_box = gtk::Box::builder().spacing(8).build();
        let label = gtk::Label::builder()
            .label(label_for(&options, &value))
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(30)
            .build();
        inner_box.append(&label);
        let chev = Icon::new("chevron-down", 14);
        chev.set_valign(gtk::Align::Center);
        inner_box.append(&chev);
        root.set_child(Some(&inner_box));

        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(1).build();
        let popover = gtk::Popover::builder()
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .halign(gtk::Align::Start)
            .css_classes(["form-menu"])
            .child(&list)
            .build();
        popover.set_offset(0, 4);
        popover.set_parent(&root);
        {
            let p = popover.clone();
            root.connect_destroy(move |_| p.unparent());
        }
        let inner = Rc::new(ChoiceInner {
            options: RefCell::new(options),
            value: RefCell::new(value),
            label,
            popover,
            list,
            changed: Box::new(changed),
        });
        // The button keeps its state (its menu's items hold it weakly: no cycle).
        let i = inner.clone();
        root.connect_clicked(move |_| ChoiceInner::open(&i));
        Self { root, inner }
    }

    pub fn value(&self) -> T {
        self.inner.value.borrow().clone()
    }

    /// Shows `v` without calling back.
    pub fn set(&self, v: T) {
        self.inner.label.set_label(&label_for(&self.inner.options.borrow(), &v));
        *self.inner.value.borrow_mut() = v;
    }

    /// Other options (the displays plugged in now), keeping its value.
    pub fn set_options(&self, options: impl IntoIterator<Item = impl Into<Opt<T>>>) {
        *self.inner.options.borrow_mut() = self::options(options);
        let v = self.value();
        self.set(v);
    }

    /// Opens its menu (as a click does).
    pub fn popup(&self) {
        ChoiceInner::open(&self.inner);
    }

    pub fn named(self, name: &str) -> Self {
        self.root.set_widget_name(name);
        self
    }
}

impl<T: Clone + PartialEq + 'static> ChoiceInner<T> {
    fn open(me: &Rc<Self>) {
        while let Some(c) = me.list.first_child() {
            me.list.remove(&c);
        }
        let current = me.value.borrow().clone();
        let mut focus = None;
        for (i, o) in me.options.borrow().iter().enumerate() {
            let b = gtk::Button::builder().css_classes(["form-menu-item"]).build();
            let row = gtk::Box::builder().spacing(10).build();
            let on = o.value == current;
            let ic = Icon::new(if on { "check" } else { "minus" }, 15);
            ic.add_css_class("form-menu-ic");
            ic.set_valign(gtk::Align::Center);
            ic.set_opacity(if on { 1.0 } else { 0.0 });
            row.append(&ic);
            let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).build();
            text.append(&gtk::Label::builder().label(&o.label).xalign(0.0).build());
            if let Some(h) = &o.hint {
                text.append(&gtk::Label::builder().label(h).xalign(0.0).css_classes(["form-menu-hint"]).build());
            }
            row.append(&text);
            b.set_child(Some(&row));
            if on {
                b.add_css_class("on");
                focus = Some(b.clone());
            }
            let w = Rc::downgrade(me);
            b.connect_clicked(move |_| {
                let Some(me) = w.upgrade() else { return };
                me.popover.popdown();
                let Some(v) = me.options.borrow().get(i).map(|o| o.value.clone()) else { return };
                if v == *me.value.borrow() {
                    return;
                }
                me.label.set_label(&label_for(&me.options.borrow(), &v));
                *me.value.borrow_mut() = v.clone();
                (me.changed)(&v);
            });
            me.list.append(&b);
        }
        me.popover.popup();
        if let Some(b) = focus {
            b.grab_focus();
        }
    }
}

// ------------------------------------------------------------ segmented

/// A segmented control (`ui/Segmented.svelte`): one of a few, side by
/// side; `changed` when another's picked.
pub struct Segmented<T: Clone + PartialEq + 'static> {
    pub root: gtk::Box,
    inner: Rc<SegInner<T>>,
}

impl<T: Clone + PartialEq + 'static> Clone for Segmented<T> {
    fn clone(&self) -> Self {
        Self { root: self.root.clone(), inner: self.inner.clone() }
    }
}

struct SegInner<T> {
    options: Vec<Opt<T>>,
    buttons: Vec<glib::WeakRef<gtk::Button>>,
    value: RefCell<T>,
    changed: Box<dyn Fn(&T)>,
}

impl<T: Clone + PartialEq + 'static> Segmented<T> {
    /// `small`: the spike's `size="sm"`.
    pub fn new(
        options: impl IntoIterator<Item = impl Into<Opt<T>>>,
        value: T,
        small: bool,
        changed: impl Fn(&T) + 'static,
    ) -> Self {
        let options = self::options(options);
        let root = gtk::Box::builder().spacing(2).valign(gtk::Align::Center).halign(gtk::Align::Start).build();
        root.add_css_class("form-seg");
        if small {
            root.add_css_class("sm");
        }
        let mut buttons = vec![];
        for o in &options {
            let b = gtk::Button::builder().css_classes(["form-seg-item"]).build();
            let row = gtk::Box::builder().spacing(6).halign(gtk::Align::Center).build();
            if let Some(i) = o.icon {
                let ic = Icon::new(i, 14);
                ic.set_valign(gtk::Align::Center);
                row.append(&ic);
            }
            row.append(&gtk::Label::new(Some(&o.label)));
            b.set_child(Some(&row));
            if o.value == value {
                b.add_css_class("on");
            }
            root.append(&b);
            buttons.push(b);
        }
        let inner = Rc::new(SegInner {
            options,
            buttons: buttons.iter().map(|b| b.downgrade()).collect(),
            value: RefCell::new(value),
            changed: Box::new(changed),
        });
        // Each button keeps the state (which holds them weakly: no cycle).
        for (i, b) in buttons.iter().enumerate() {
            let me = inner.clone();
            b.connect_clicked(move |_| {
                let v = me.options[i].value.clone();
                if v == *me.value.borrow() {
                    return;
                }
                me.show(&v);
                (me.changed)(&v);
            });
        }
        Self { root, inner }
    }

    pub fn value(&self) -> T {
        self.inner.value.borrow().clone()
    }

    /// Shows `v` without calling back.
    pub fn set(&self, v: T) {
        self.inner.show(&v);
    }

    pub fn named(self, name: &str) -> Self {
        self.root.set_widget_name(name);
        self
    }
}

impl<T: Clone + PartialEq> SegInner<T> {
    fn show(&self, v: &T) {
        for (o, b) in self.options.iter().zip(&self.buttons) {
            let Some(b) = b.upgrade() else { continue };
            if o.value == *v {
                b.add_css_class("on");
            } else {
                b.remove_css_class("on");
            }
        }
        *self.value.borrow_mut() = v.clone();
    }
}

// ------------------------------------------------------------ text field

/// A text field (`ui/TextField.svelte`): its label over it, the field, a
/// line saying what's wrong under it. [`TextField::password`] hides what's
/// typed, with an eye to show it.
#[derive(Clone)]
pub struct TextField {
    pub root: gtk::Box,
    pub text: gtk::Text,
    input: gtk::Box,
    error: gtk::Label,
}

impl TextField {
    /// `label` (none: "") over a field saying `placeholder` while it's empty.
    pub fn new(label: &str, placeholder: &str) -> Self {
        let root =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).css_classes(["form-field"]).build();
        if !label.is_empty() {
            root.append(&gtk::Label::builder().label(label).xalign(0.0).css_classes(["form-field-label"]).build());
        }
        let input = gtk::Box::builder().spacing(4).css_classes(["form-input"]).build();
        let text = gtk::Text::builder().placeholder_text(placeholder).hexpand(true).valign(gtk::Align::Center).build();
        input.append(&text);
        {
            // A click anywhere in the box puts the caret in it.
            let t = text.clone();
            let click = gtk::GestureClick::new();
            click.connect_pressed(move |_, _, _, _| {
                if !t.has_focus() {
                    // Not `grab_focus`: it selects everything typed (a GTK
                    // text field selects all on focus), so typing would
                    // overwrite what's already there.
                    t.grab_focus_without_selecting();
                }
            });
            input.add_controller(click);
        }
        root.append(&input);
        let error = gtk::Label::builder().xalign(0.0).wrap(true).visible(false).css_classes(["form-error"]).build();
        root.append(&error);
        Self { root, text, input, error }
    }

    /// A password: hidden, with an eye that shows it (and hides it again).
    pub fn password(label: &str, placeholder: &str) -> Self {
        let f = Self::new(label, placeholder);
        f.text.set_visibility(false);
        f.text.set_input_purpose(gtk::InputPurpose::Password);
        f.input.add_css_class("has-reveal");
        let eye = Icon::new(reveal_icon(false), 16);
        let b = gtk::Button::builder().child(&eye).valign(gtk::Align::Center).css_classes(["form-reveal"]).build();
        b.set_tooltip_text(Some(reveal_tip(false)));
        b.set_focus_on_click(false);
        let t = f.text.clone();
        b.connect_clicked(move |b| {
            let vis = !shown(&t);
            t.set_visibility(vis);
            eye.set(reveal_icon(vis));
            b.set_tooltip_text(Some(reveal_tip(vis)));
        });
        f.input.append(&b);
        f
    }

    /// Typed as code (a hostname, a key).
    pub fn mono(self) -> Self {
        self.text.add_css_class("mono");
        self
    }

    pub fn named(self, name: &str) -> Self {
        self.root.set_widget_name(name);
        self
    }

    pub fn value(&self) -> String {
        self.text.text().to_string()
    }

    pub fn set_value(&self, v: &str) {
        self.text.set_text(v);
    }

    /// What's wrong, under it (none: nothing).
    pub fn set_error(&self, e: Option<&str>) {
        match e.filter(|e| !e.is_empty()) {
            Some(e) => {
                self.error.set_label(e);
                self.error.set_visible(true);
                self.root.add_css_class("has-error");
            }
            None => {
                self.error.set_visible(false);
                self.root.remove_css_class("has-error");
            }
        }
    }

    /// Called with what it says as it's typed.
    pub fn on_change(&self, f: impl Fn(&str) + 'static) {
        self.text.connect_changed(move |t| f(&t.text()));
    }

    /// Called on Enter.
    pub fn on_enter(&self, f: impl Fn(&str) + 'static) {
        self.text.connect_activate(move |t| f(&t.text()));
    }

    /// Puts the caret in it, without selecting what's typed. A GTK text
    /// field selects all of its text when focused, so re-grabbing the focus
    /// (the Wi-Fi panel does this whenever its page is drawn again, once per
    /// keystroke) would select the whole field and the next character typed
    /// would overwrite it.
    pub fn focus(&self) {
        self.text.grab_focus_without_selecting();
    }

    /// A password field showing what's typed.
    pub fn is_revealed(&self) -> bool {
        shown(&self.text)
    }
}

fn shown(t: &gtk::Text) -> bool {
    t.property::<bool>("visibility")
}

/// The eye a password field shows: what clicking it does.
pub fn reveal_icon(shown: bool) -> &'static str {
    if shown { "eye-off" } else { "eye" }
}

fn reveal_tip(shown: bool) -> &'static str {
    if shown { "Hide the password" } else { "Show the password" }
}

// ------------------------------------------------------------ dialog

/// A modal dialog over a window (`ui/Dialog.svelte`): a badge and its
/// title (with a close button), its body, its buttons at the foot. It's
/// drawn in the window, over the rest, which is dimmed and can't be used
/// meanwhile. Each button answers its id; Esc, the close button and a click
/// outside answer [`Dialog::cancel`]'s (with none, they do nothing and
/// there's no close button).
///
/// ```ignore
/// Dialog::new("Forget “Home”?")
///     .icon("wifi", Tone::Danger)
///     .text("You'll need its password to join it again.")
///     .button("cancel", "Cancel", Variant::Ghost)
///     .button("forget", "Forget", Variant::Danger)
///     .cancel("cancel")
///     .open(&window, |id| if id == "forget" { /* … */ });
/// ```
pub struct Dialog {
    title: String,
    icon: Option<(&'static str, Tone)>,
    width: i32,
    body: Vec<gtk::Widget>,
    buttons: Vec<(String, String, Variant)>,
    hold: Vec<String>,
    cancel: Option<String>,
    countdown: Option<Countdown>,
    name: Option<String>,
}

struct Countdown {
    seconds: u32,
    answer: String,
    text: Box<dyn Fn(u32) -> String>,
}

impl Dialog {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.into(),
            icon: None,
            width: 440,
            body: vec![],
            buttons: vec![],
            hold: vec![],
            cancel: None,
            countdown: None,
            name: None,
        }
    }

    /// The badge before its title.
    pub fn icon(mut self, icon: &'static str, tone: Tone) -> Self {
        self.icon = Some((icon, tone));
        self
    }

    /// Its width (440 by default).
    pub fn width(mut self, w: i32) -> Self {
        self.width = w;
        self
    }

    /// A paragraph of its body.
    pub fn text(self, t: &str) -> Self {
        let l = kit::label(t, &[]);
        self.child(&l)
    }

    /// A paragraph in Pango markup (`<b>` reads in the text's colour).
    pub fn markup(self, m: &str) -> Self {
        let l = kit::label("", &[]);
        l.set_markup(&brighten(m));
        self.child(&l)
    }

    /// Anything else in its body (a text field, a list).
    pub fn child(mut self, w: &impl IsA<gtk::Widget>) -> Self {
        self.body.push(w.clone().upcast());
        self
    }

    /// A button at its foot, after the others, answering `id`.
    pub fn button(mut self, id: &str, label: &str, variant: Variant) -> Self {
        self.buttons.push((id.into(), label.into(), variant));
        self
    }

    /// `id`'s button answers but leaves it open (a form checking what's typed: close it with [`Open::close`]).
    pub fn hold(mut self, id: &str) -> Self {
        self.hold.push(id.into());
        self
    }

    /// What Esc, its close button and a click outside answer.
    pub fn cancel(mut self, id: &str) -> Self {
        self.cancel = Some(id.into());
        self
    }

    /// Answers `answer` by itself after `seconds`, showing `text(seconds left)` (markup) meanwhile, first in its body.
    pub fn countdown(mut self, seconds: u32, answer: &str, text: impl Fn(u32) -> String + 'static) -> Self {
        self.countdown = Some(Countdown { seconds, answer: answer.into(), text: Box::new(text) });
        self
    }

    /// Its widget name, for tests (its buttons are named `{name}-{id}`).
    pub fn named(mut self, name: &str) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Shows it over `window`; `answer` gets it and the id it's answered
    /// with (once it's closed, unless that's a [`Dialog::hold`] button's).
    /// It stays open as long as it's shown: the [`Open`] it gives back
    /// needn't be kept.
    pub fn open(self, window: &impl IsA<gtk::Window>, answer: impl Fn(&Open, &str) + 'static) -> Open {
        let overlay = host(window.upcast_ref());
        let scrim = gtk::Box::builder().hexpand(true).vexpand(true).css_classes(["form-scrim"]).build();
        let dialog = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .css_classes(["form-dialog"])
            .build();
        dialog.set_size_request(self.width, -1);
        if let Some(n) = &self.name {
            dialog.set_widget_name(n);
        }
        scrim.append(&dialog);

        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .css_classes(["form-dialog-body"])
            .build();
        // Its widgets keep it (it holds them weakly: gone with them, no cycle).
        let open = Open(Rc::new(OpenInner {
            overlay: overlay.downgrade(),
            scrim: scrim.downgrade(),
            body: body.downgrade(),
            buttons: RefCell::default(),
            closed: Cell::new(false),
            answer: Box::new(answer),
        }));

        let head = gtk::Box::builder().spacing(12).css_classes(["form-dialog-head"]).build();
        if let Some((icon, tone)) = self.icon {
            let b = gtk::Box::builder()
                .css_classes(["form-badge", tone.class()])
                .valign(gtk::Align::Center)
                .hexpand(false)
                .build();
            let i = Icon::new(icon, 18);
            i.set_hexpand(true);
            i.set_halign(gtk::Align::Center);
            i.set_valign(gtk::Align::Center);
            b.append(&i);
            head.append(&b);
        }
        let t = kit::label(&self.title, &["form-dialog-title"]);
        t.set_hexpand(true);
        t.set_valign(gtk::Align::Center);
        head.append(&t);
        if let Some(c) = &self.cancel {
            let x = gtk::Button::builder().child(&Icon::new("x", 16)).css_classes(["form-x"]).build();
            x.set_tooltip_text(Some("Close"));
            x.set_valign(gtk::Align::Center);
            let (o, c) = (open.0.clone(), c.clone());
            x.connect_clicked(move |_| answer_with(&o, &c, true));
            head.append(&x);
        }
        if !self.title.is_empty() || self.icon.is_some() {
            dialog.append(&head);
        }

        let tick = self.countdown.map(|cd| {
            let l = kit::label("", &[]);
            l.set_widget_name("dialog-countdown");
            l.set_markup(&brighten(&(cd.text)(cd.seconds)));
            body.append(&l);
            (cd, l)
        });
        for w in &self.body {
            body.append(w);
        }
        dialog.append(&body);

        let mut last = None;
        if !self.buttons.is_empty() {
            let foot = gtk::Box::builder().spacing(8).halign(gtk::Align::End).css_classes(["form-dialog-foot"]).build();
            for (id, label, variant) in &self.buttons {
                let b = kit::button(label, *variant, None, false);
                if let Some(n) = &self.name {
                    b.set_widget_name(&format!("{n}-{id}"));
                }
                let (o, id2, closes) = (open.0.clone(), id.clone(), !self.hold.contains(id));
                b.connect_clicked(move |_| answer_with(&o, &id2, closes));
                foot.append(&b);
                open.0.buttons.borrow_mut().push((id.clone(), b.downgrade()));
                last = Some(b);
            }
            dialog.append(&foot);
        }

        // Esc, and a click outside it.
        if let Some(c) = self.cancel.clone() {
            let keys = gtk::EventControllerKey::new();
            let (o, c2) = (open.0.clone(), c.clone());
            keys.connect_key_pressed(move |_, k, _, _| {
                if k == gdk::Key::Escape {
                    answer_with(&o, &c2, true);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            scrim.add_controller(keys);
            let click = gtk::GestureClick::new();
            let (o, s) = (open.0.clone(), scrim.downgrade());
            click.connect_released(move |_, _, x, y| {
                let Some(s) = s.upgrade() else { return };
                if s.pick(x, y, gtk::PickFlags::DEFAULT).as_ref() == Some(s.upcast_ref()) {
                    answer_with(&o, &c, true);
                }
            });
            scrim.add_controller(click);
        }

        // What's under it can't be reached meanwhile.
        if let Some(base) = overlay.child() {
            base.set_can_focus(false);
        }
        overlay.add_overlay(&scrim);
        let first = body.first_child();
        glib::idle_add_local_once(move || {
            // A field in its body first, else its last button (GNOME's default).
            let field = first.and_then(|f| find_text(&f));
            match (field, last) {
                (Some(f), _) => {
                    f.grab_focus();
                }
                (None, Some(b)) => {
                    b.grab_focus();
                }
                _ => {}
            }
        });

        if let Some((cd, label)) = tick {
            let deadline = glib::monotonic_time() + i64::from(cd.seconds) * 1_000_000;
            let w = Rc::downgrade(&open.0);
            let shown = Cell::new(cd.seconds);
            glib::timeout_add_local(Duration::from_millis(200), move || {
                let Some(o) = w.upgrade() else { return glib::ControlFlow::Break };
                if o.closed.get() {
                    return glib::ControlFlow::Break;
                }
                let left = seconds_left(deadline, glib::monotonic_time());
                if left != shown.replace(left) {
                    label.set_markup(&brighten(&(cd.text)(left)));
                }
                if left == 0 {
                    answer_with(&o, &cd.answer, true);
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
        }
        open
    }
}

/// A dialog that's open: close it, or reach its body and buttons.
#[derive(Clone)]
pub struct Open(Rc<OpenInner>);

struct OpenInner {
    overlay: glib::WeakRef<gtk::Overlay>,
    scrim: glib::WeakRef<gtk::Box>,
    body: glib::WeakRef<gtk::Box>,
    buttons: RefCell<Vec<(String, glib::WeakRef<gtk::Button>)>>,
    closed: Cell<bool>,
    answer: Box<Answer>,
}

/// What a dialog's answer goes to.
type Answer = dyn Fn(&Open, &str);

impl Open {
    /// Closes it without an answer.
    pub fn close(&self) {
        self.0.close();
    }

    pub fn is_open(&self) -> bool {
        !self.0.closed.get()
    }

    /// Its body, under the countdown (when it has one); none once it's gone.
    pub fn body(&self) -> Option<gtk::Box> {
        self.0.body.upgrade()
    }

    /// The button answering `id`.
    pub fn button(&self, id: &str) -> Option<gtk::Button> {
        self.0.buttons.borrow().iter().find(|(i, _)| i == id).and_then(|(_, b)| b.upgrade())
    }

    /// Answers `id` as its button would (and closes it).
    pub fn answer(&self, id: &str) {
        answer_with(&self.0, id, true);
    }
}

impl OpenInner {
    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        let Some(overlay) = self.overlay.upgrade() else { return };
        if let Some(scrim) = self.scrim.upgrade() {
            overlay.remove_overlay(&scrim);
        }
        // The window's back when no other dialog's over it.
        let mut others = false;
        let mut c = overlay.first_child();
        while let Some(w) = c {
            others |= w.has_css_class("form-scrim");
            c = w.next_sibling();
        }
        if let Some(base) = overlay.child().filter(|_| !others) {
            base.set_can_focus(true);
        }
    }
}

fn answer_with(o: &Rc<OpenInner>, id: &str, closes: bool) {
    if o.closed.get() {
        return;
    }
    if closes {
        o.close();
    }
    (o.answer)(&Open(o.clone()), id);
}

fn find_text(w: &gtk::Widget) -> Option<gtk::Widget> {
    if w.is::<gtk::Text>() {
        return Some(w.clone());
    }
    let mut c = w.first_child();
    while let Some(x) = c {
        if let Some(t) = find_text(&x) {
            return Some(t);
        }
        c = x.next_sibling();
    }
    w.next_sibling().and_then(|n| find_text(&n))
}

/// The overlay a window's dialogs go in: its child, wrapped once.
pub fn host(window: &gtk::Window) -> gtk::Overlay {
    if let Some(o) = window.child().and_downcast::<gtk::Overlay>().filter(|o| o.has_css_class("form-host")) {
        return o;
    }
    let o = gtk::Overlay::builder().css_classes(["form-host"]).build();
    let child = window.child();
    window.set_child(Some(&o));
    o.set_child(child.as_ref());
    o
}

/// Whole seconds left before `deadline` at `now` (µs, monotonic), rounded up.
pub fn seconds_left(deadline_us: i64, now_us: i64) -> u32 {
    let left = (deadline_us - now_us).max(0) as u64;
    left.div_ceil(1_000_000) as u32
}

/// `<b>` in a dialog's text reads in the text's colour (`--fg`), as the spike's `strong`.
fn brighten(markup: &str) -> String {
    markup.replace("<b>", "<b><span foreground=\"#f5f2fa\">").replace("</b>", "</span></b>")
}

/// The Displays trial's title.
pub const TRIAL_TITLE: &str = "Keep these display settings?";

/// What the Displays trial says with `n` seconds left (markup).
pub fn trial_text(n: u32) -> String {
    format!("They go back in <b>{n} second{}</b> unless you keep them.", if n == 1 { "" } else { "s" })
}

/// Displays' trial (Settings.svelte's): keep the new settings, or they go
/// back in `seconds`. `done(true)`: kept; `done(false)`: Revert, Esc, or
/// the time ran out.
pub fn display_trial(window: &impl IsA<gtk::Window>, seconds: u32, done: impl Fn(bool) + 'static) -> Open {
    Dialog::new(TRIAL_TITLE)
        .icon("monitor", Tone::Warn)
        .width(420)
        .countdown(seconds, "revert", trial_text)
        .button("revert", "Revert", Variant::Ghost)
        .button("keep", "Keep Changes", Variant::Primary)
        .cancel("revert")
        .named("display-trial")
        .open(window, move |_, id| done(id == "keep"))
}

// ------------------------------------------------------------ meters

/// A meter part's colour (the spike's tokens).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hue {
    /// `--c-app`, the blue (Storage's base images).
    App,
    /// `--accent` (its wadspaces).
    Accent,
    /// `--accent-2` (system and other).
    Magenta,
    Warn,
    Danger,
    /// `--muted`.
    Plain,
}

impl Hue {
    pub fn class(self) -> &'static str {
        match self {
            Hue::App => "app",
            Hue::Accent => "accent",
            Hue::Magenta => "magenta",
            Hue::Warn => "warn",
            Hue::Danger => "danger",
            Hue::Plain => "plain",
        }
    }

    /// As drawn (the tokens in theme.rs): r, g, b, a.
    pub fn rgba(self) -> (f64, f64, f64, f64) {
        let hex = |h: u32| {
            (f64::from((h >> 16) & 0xff) / 255.0, f64::from((h >> 8) & 0xff) / 255.0, f64::from(h & 0xff) / 255.0, 1.0)
        };
        match self {
            Hue::App => hex(0x7fd3ff),
            Hue::Accent => hex(0xc6ff1f),
            Hue::Magenta => hex(0xff3d81),
            Hue::Warn => hex(0xe8c36a),
            Hue::Danger => hex(0xff5d7a),
            Hue::Plain => (245.0 / 255.0, 242.0 / 255.0, 250.0 / 255.0, 0.62),
        }
    }
}

/// One part of a stacked [`Meter`]: what it is, how much, its colour.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub label: String,
    pub amount: f64,
    pub hue: Hue,
}

pub fn part(label: &str, amount: f64, hue: Hue) -> Part {
    Part { label: label.into(), amount, hue }
}

/// Where each part sits along a bar of `total` (start, width; 0 to 1),
/// in order, none past its end.
pub fn spans(total: f64, amounts: &[f64]) -> Vec<(f64, f64)> {
    let mut at = 0.0_f64;
    amounts
        .iter()
        .map(|a| {
            let w = if total > 0.0 { (a.max(0.0) / total).min(1.0 - at) } else { 0.0 };
            let s = (at, w.max(0.0));
            at += s.1;
            s
        })
        .collect()
}

/// What's left of `total` after the parts.
pub fn free(total: f64, parts: &[Part]) -> f64 {
    (total - parts.iter().map(|p| p.amount.max(0.0)).sum::<f64>()).max(0.0)
}

/// A stacked bar (Storage's disk: base images, wadspaces, everything
/// else, free) over its legend: each part's swatch, name and amount
/// (`fmt`ted), and what's free when `free_label` says what to call it.
#[derive(Clone)]
pub struct Meter {
    pub root: gtk::Box,
    bar: gtk::DrawingArea,
    legend: crate::Wrap,
    data: Rc<RefCell<(f64, Vec<Part>)>>,
    free_label: Option<String>,
    fmt: Rc<dyn Fn(f64) -> String>,
}

impl Meter {
    pub fn new(total: f64, parts: Vec<Part>, free_label: Option<&str>, fmt: impl Fn(f64) -> String + 'static) -> Self {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).build();
        let bar = gtk::DrawingArea::builder().content_height(16).hexpand(true).build();
        let data: Rc<RefCell<(f64, Vec<Part>)>> = Rc::new(RefCell::new((total, parts)));
        {
            let d = data.clone();
            bar.set_draw_func(move |_, cr, w, h| {
                let (total, parts) = &*d.borrow();
                draw_bar(cr, f64::from(w), f64::from(h), *total, parts);
            });
        }
        root.append(&bar);
        let legend = crate::Wrap::new(20, 8);
        legend.add_css_class("form-legend");
        root.append(&legend);
        let m = Self { root, bar, legend, data, free_label: free_label.map(Into::into), fmt: Rc::new(fmt) };
        m.fill_legend();
        m
    }

    /// Other amounts.
    pub fn set(&self, total: f64, parts: Vec<Part>) {
        *self.data.borrow_mut() = (total, parts);
        self.bar.queue_draw();
        self.fill_legend();
    }

    fn fill_legend(&self) {
        while let Some(c) = self.legend.first_child() {
            c.unparent();
        }
        let (total, parts) = &*self.data.borrow();
        let item = |label: &str, amount: f64, class: &str| {
            let b = gtk::Box::builder().spacing(7).build();
            let sw = gtk::Box::builder().css_classes(["form-sw", class]).valign(gtk::Align::Center).build();
            b.append(&sw);
            b.append(&gtk::Label::new(Some(label)));
            b.append(&gtk::Label::builder().label((self.fmt)(amount)).css_classes(["form-legend-value"]).build());
            b
        };
        for p in parts {
            self.legend.append(&item(&p.label, p.amount, p.hue.class()));
        }
        if let Some(f) = &self.free_label {
            self.legend.append(&item(f, free(*total, parts), "free"));
        }
    }
}

fn draw_bar(cr: &gtk::cairo::Context, w: f64, h: f64, total: f64, parts: &[Part]) {
    let r = h / 2.0;
    let pill = |cr: &gtk::cairo::Context| {
        cr.new_sub_path();
        cr.arc(w - r, r, r, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
        cr.arc(r, r, r, std::f64::consts::FRAC_PI_2, 3.0 * std::f64::consts::FRAC_PI_2);
        cr.close_path();
    };
    pill(cr);
    cr.clip();
    // --surface-3 under it all: the free part.
    cr.set_source_rgb(0x24 as f64 / 255.0, 0x1a as f64 / 255.0, 0x39 as f64 / 255.0);
    cr.paint().ok();
    let amounts: Vec<f64> = parts.iter().map(|p| p.amount).collect();
    for (i, (p, (s, sw))) in parts.iter().zip(spans(total, &amounts)).enumerate() {
        if sw <= 0.0 {
            continue;
        }
        let (r, g, b, a) = p.hue.rgba();
        cr.set_source_rgba(r, g, b, a);
        cr.rectangle(s * w, 0.0, sw * w, h);
        cr.fill().ok();
        // A gap of the list's colour (--surface-2) between parts.
        if i > 0 {
            cr.set_source_rgb(0x1b as f64 / 255.0, 0x13 as f64 / 255.0, 0x2d as f64 / 255.0);
            cr.rectangle(s * w, 0.0, 2.0, h);
            cr.fill().ok();
        }
    }
}

/// How many of a level meter's `bars` are lit at `level` (0 to 1).
pub fn lit(level: f64, bars: usize) -> usize {
    (level.clamp(0.0, 1.0) * bars as f64).round() as usize
}

/// The level meter's hot end (its last bars, in the warning colour).
pub fn hot(i: usize, bars: usize) -> bool {
    i as f64 >= bars as f64 * 0.82
}

/// A level meter (Sound's `parts/Meter.svelte`): a row of bars lit up to its level.
#[derive(Clone)]
pub struct Level {
    pub root: gtk::Box,
    bars: Vec<gtk::Box>,
}

impl Level {
    pub fn new(bars: usize) -> Self {
        let root = gtk::Box::builder().spacing(3).hexpand(true).homogeneous(true).valign(gtk::Align::Center).build();
        let bars: Vec<gtk::Box> = (0..bars)
            .map(|i| {
                let b = gtk::Box::builder().css_classes(["form-lvl"]).hexpand(true).build();
                if hot(i, bars) {
                    b.add_css_class("hot");
                }
                root.append(&b);
                b
            })
            .collect();
        Self { root, bars }
    }

    pub fn set(&self, level: f64) {
        let n = lit(level, self.bars.len());
        for (i, b) in self.bars.iter().enumerate() {
            if i < n {
                b.add_css_class("lit");
            } else {
                b.remove_css_class("lit");
            }
        }
    }
}

// ------------------------------------------------------------ empty, keys

/// A panel's empty state (`parts/Empty.svelte`: Wi-Fi off, Bluetooth
/// off): a big icon in a circle, a title, a line or two, then `actions`
/// (the way back on).
#[derive(Clone)]
pub struct Empty {
    pub root: gtk::Box,
    pub actions: gtk::Box,
}

impl Empty {
    pub fn new(icon: &'static str, title: &str, text: &str) -> Self {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .halign(gtk::Align::Center)
            .css_classes(["form-empty"])
            .build();
        let ic = gtk::Box::builder().css_classes(["form-empty-ic"]).halign(gtk::Align::Center).build();
        let i = Icon::new(icon, 40);
        i.set_hexpand(true);
        i.set_halign(gtk::Align::Center);
        i.set_valign(gtk::Align::Center);
        ic.append(&i);
        root.append(&ic);
        let t = kit::label(title, &["form-empty-title"]);
        t.set_xalign(0.5);
        t.set_justify(gtk::Justification::Center);
        root.append(&t);
        if !text.is_empty() {
            let p = kit::label(text, &["form-empty-text"]);
            p.set_xalign(0.5);
            p.set_justify(gtk::Justification::Center);
            p.set_max_width_chars(46);
            root.append(&p);
        }
        let actions = gtk::Box::builder().spacing(8).halign(gtk::Align::Center).build();
        root.append(&actions);
        Self { root, actions }
    }
}

/// A key combination's keys: "Ctrl+Alt+L" is Ctrl, Alt, L; a `+` at the end is a key ("Ctrl++").
pub fn split_keys(combo: &str) -> Vec<&str> {
    if combo == "+" {
        return vec!["+"];
    }
    let mut out = vec![];
    let mut start = 0;
    let b = combo.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c == b'+' && i + 1 < b.len() && i >= start {
            if i > start {
                out.push(&combo[start..i]);
                start = i + 1;
            } else {
                // "++" mid-way: this + is the key.
                out.push("+");
                start = i + 2;
            }
        }
    }
    if start < combo.len() {
        out.push(&combo[start..]);
    }
    out
}

/// A shortcut as key caps (`parts/Keys.svelte`).
pub fn keys(combo: &str) -> gtk::Box {
    let b = gtk::Box::builder().spacing(4).valign(gtk::Align::Center).halign(gtk::Align::End).build();
    b.update_property(&[gtk::accessible::Property::Label(combo)]);
    for k in split_keys(combo) {
        b.append(&gtk::Label::builder().label(k).xalign(0.5).css_classes(["form-key"]).build());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_read_their_labels() {
        let o: Vec<Opt<u32>> = options([("60.00 Hz", 60), ("120.00 Hz", 120)]);
        assert_eq!(label_for(&o, &120), "120.00 Hz");
        assert_eq!(label_for(&o, &75), "—");
        let h = opt("", "None").hint("Straight out").icon("ban");
        assert_eq!((h.hint.as_deref(), h.icon), (Some("Straight out"), Some("ban")));
    }

    #[test]
    fn a_slider_keeps_to_its_steps() {
        assert_eq!(snap(0.333, 0.0, 1.0, 0.01), 0.33);
        assert_eq!(snap(0.3, 0.0, 1.0, 0.01), 0.3);
        assert_eq!(snap(7.0, 0.0, 5.0, 1.0), 5.0);
        assert_eq!(snap(-1.0, 0.15, 1.0, 0.05), 0.15);
        assert_eq!(snap(0.62, 0.15, 1.0, 0.05), 0.6);
        assert_eq!(snap(1.26, 1.0, 2.0, 0.25), 1.25);
        assert_eq!(snap(0.4321, 0.0, 1.0, 0.0), 0.4321);
        assert_eq!(fraction(0.75, 0.5, 1.0), 0.5);
        assert_eq!(fraction(2.0, 0.0, 1.0), 1.0);
        assert_eq!(fraction(3.0, 1.0, 1.0), 0.0);
    }

    #[test]
    fn the_trial_counts_down_in_words() {
        assert_eq!(trial_text(15), "They go back in <b>15 seconds</b> unless you keep them.");
        assert_eq!(trial_text(1), "They go back in <b>1 second</b> unless you keep them.");
        assert_eq!(seconds_left(20_000_000, 5_000_000), 15);
        assert_eq!(seconds_left(20_000_000, 5_000_001), 15);
        assert_eq!(seconds_left(20_000_000, 19_000_001), 1);
        assert_eq!(seconds_left(20_000_000, 20_000_000), 0);
        assert_eq!(seconds_left(20_000_000, 30_000_000), 0);
        assert_eq!(brighten("a <b>b</b>"), "a <b><span foreground=\"#f5f2fa\">b</span></b>");
    }

    #[test]
    fn a_stacked_meter_fits_its_bar() {
        assert_eq!(spans(100.0, &[20.0, 30.0, 10.0]), vec![(0.0, 0.2), (0.2, 0.3), (0.5, 0.1)]);
        // More than there's room for: cut at the end.
        assert_eq!(spans(10.0, &[8.0, 5.0, 1.0]), vec![(0.0, 0.8), (0.8, 0.19999999999999996), (1.0, 0.0)]);
        assert_eq!(spans(0.0, &[1.0]), vec![(0.0, 0.0)]);
        assert_eq!(spans(10.0, &[-3.0, 5.0]), vec![(0.0, 0.0), (0.0, 0.5)]);
        let p = [part("Base images", 20.0, Hue::App), part("Wadspaces", 30.0, Hue::Accent)];
        assert_eq!(free(100.0, &p), 50.0);
        assert_eq!(free(40.0, &p), 0.0);
        assert_eq!(Hue::Accent.rgba(), (198.0 / 255.0, 1.0, 31.0 / 255.0, 1.0));
    }

    #[test]
    fn a_level_meter_lights_up_to_its_level() {
        assert_eq!(lit(0.5, 28), 14);
        assert_eq!(lit(2.0, 28), 28);
        assert_eq!(lit(-1.0, 28), 0);
        assert!(!hot(22, 28) && hot(23, 28) && hot(27, 28));
    }

    #[test]
    fn a_shortcut_splits_into_its_keys() {
        assert_eq!(split_keys("Ctrl+Alt+L"), ["Ctrl", "Alt", "L"]);
        assert_eq!(split_keys("Super"), ["Super"]);
        assert_eq!(split_keys("+"), ["+"]);
        assert_eq!(split_keys("Ctrl++"), ["Ctrl", "+"]);
        assert_eq!(split_keys("Ctrl+++Alt"), ["Ctrl", "+", "Alt"]);
        assert_eq!(split_keys("Super+Shift+Left"), ["Super", "Shift", "Left"]);
    }

    #[test]
    fn the_eye_says_what_it_does() {
        assert_eq!(reveal_icon(false), "eye");
        assert_eq!(reveal_icon(true), "eye-off");
    }
}
