//! The machine apps' pieces (Settings, Files, and the shell's dialogs), in
//! the spike's look: its buttons (`ui/Button.svelte`), a page of titled
//! groups of rows (Settings' `parts/Page`, `Group`, `Row`), chips, a job's
//! stages as they go (`parts/Stages.svelte`), and a sign-in waiting in the
//! browser (`shell/firstboot/InBrowser.svelte`). [`install`] adds their CSS
//! once, over the theme's.

use std::cell::Cell;

use gtk::{gdk, prelude::*};
use wad_model::{Job, JobStatus, PartState};

use crate::Icon;

/// Their stylesheet.
pub const CSS: &str = r#"
/* ---- buttons (ui/Button: md, sm) */
button.kit-btn {
  min-height: 38px;
  min-width: 0;
  padding: 0 16px;
  border-radius: 12px;
  border: 1px solid transparent;
  box-shadow: none;
  outline: none;
  text-shadow: none;
  background: none;
  background-image: none;
  color: var(--fg);
  font-weight: 500;
  font-size: 14px;
}
button.kit-btn.sm { min-height: 30px; padding: 0 11px; border-radius: 9px; font-size: 13px; }
button.kit-btn.icon-only { padding: 0; min-width: 30px; }
button.kit-btn.primary {
  background-color: var(--accent);
  color: var(--accent-fg);
  font-weight: 600;
  box-shadow: 0 8px 24px -10px var(--accent);
}
button.kit-btn.primary:hover { box-shadow: 0 10px 28px -8px var(--accent); }
button.kit-btn.secondary { background-color: var(--surface-2); border-color: var(--line-strong); }
button.kit-btn.secondary:hover { background-color: var(--surface-3); }
button.kit-btn.ghost { color: var(--muted); }
button.kit-btn.ghost:hover { background-color: var(--surface-2); color: var(--fg); }
button.kit-btn.quiet { border-color: var(--line); color: var(--muted); }
button.kit-btn.quiet:hover { border-color: var(--line-strong); color: var(--fg); }
button.kit-btn.danger { background-color: var(--danger-soft); color: var(--danger); border-color: rgb(255 93 122 / 0.3); }
button.kit-btn.danger:hover { background-color: rgb(255 93 122 / 0.24); }
button.kit-btn:disabled { opacity: 0.4; box-shadow: none; }
button.kit-btn:disabled label { color: inherit; }

/* ---- a machine app's window and title bar (as the Builder's) */
window.kit-window { border-radius: 14px; color: var(--fg); }
headerbar.kit-head { min-height: 40px; padding: 0 6px 0 12px; background: var(--bg-2); border-bottom: 1px solid var(--line); box-shadow: none; color: var(--fg); }
headerbar.kit-head .kit-head-title { font-weight: 650; font-size: 13.5px; }
headerbar.kit-head windowcontrols button { min-width: 28px; min-height: 28px; padding: 0; border-radius: 8px; background: none; box-shadow: none; color: var(--muted); }
headerbar.kit-head windowcontrols button:hover { background: var(--surface-3); color: var(--fg); }

/* ---- a page: its bar, then a centred column of groups */
.kit-page-bar { min-height: 52px; padding: 0 14px; border-bottom: 1px solid var(--line); }
.kit-page-title { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 15px; }
.kit-column { padding: 24px 28px 56px; }
button.kit-back { min-width: 34px; min-height: 34px; padding: 0; border-radius: 10px; border: none; background: none; background-image: none; box-shadow: none; color: var(--muted); }
button.kit-back:hover { background-color: var(--surface-3); color: var(--fg); }

/* ---- a group: its title over a boxed list (GNOME's) */
.kit-group-head { padding: 0 4px; }
.kit-group-title { font-size: 14px; font-weight: 600; }
.kit-group-desc { font-size: 12.5px; color: var(--muted); margin-top: 3px; }
.kit-list { border-radius: 14px; border: 1px solid var(--line); background-color: var(--surface-2); }
.kit-list > .kit-row + .kit-row, .kit-list > .kit-row + .kit-inset, .kit-list > .kit-inset + .kit-row { border-top: 1px solid var(--line); }

/* ---- a row: an icon, a title over a subtitle, a value or controls */
.kit-row { min-height: 56px; padding: 9px 14px; }
.kit-ic { min-width: 30px; min-height: 30px; border-radius: 9px; background-color: var(--surface-3); color: var(--muted); }
.kit-ic.accent { background-color: var(--accent-soft); color: var(--accent); }
.kit-ic.warn { background-color: var(--warn-soft); color: var(--warn); }
.kit-ic.danger { background-color: var(--danger-soft); color: var(--danger); }
.kit-ic.magenta { background-color: var(--accent-2-soft); color: var(--accent-2); }
.kit-row-title { font-size: 14px; }
.kit-row-sub { font-size: 12.5px; color: var(--muted); }
.kit-row-value { font-size: 13.5px; color: var(--muted); }
.kit-row-value.mono { font-family: monospace; font-size: 12.5px; }
.kit-inset { padding: 0 14px 14px; }

/* ---- a chip */
.kit-chip { min-height: 24px; padding: 0 10px; border-radius: 99px; background-color: var(--surface-3); color: var(--muted); font-size: 12px; font-weight: 500; }
.kit-chip.ok { background-color: var(--accent-soft); color: var(--accent); }
.kit-chip.busy { color: var(--fg); }
.kit-chip.warn { background-color: var(--warn-soft); color: var(--warn); }

/* ---- a job's stages */
.kit-stages { padding: 12px 14px; border-radius: 12px; background-color: var(--bg-2); border: 1px solid var(--line); }
.kit-st { font-size: 13px; color: var(--faint); }
.kit-st.working { color: var(--fg); }
.kit-st.done { color: var(--muted); }
.kit-st.done wad-icon { color: var(--accent); }
.kit-st.error { color: var(--danger); }
.kit-err { font-size: 13px; color: var(--danger); }
progressbar.kit-progress > trough { min-height: 4px; border-radius: 2px; background-color: var(--surface-3); border: none; }
progressbar.kit-progress > trough > progress { min-height: 4px; min-width: 0; margin: 0; border-radius: 2px; background-image: none; background-color: var(--accent); border: none; box-shadow: none; }

/* ---- a sign-in waiting in the browser */
.kit-in-browser {
  padding: 10px 12px;
  border-radius: 12px;
  border: 1px solid color-mix(in srgb, var(--c-app) 35%, transparent);
  background-color: color-mix(in srgb, var(--c-app) 8%, transparent);
  font-size: 13px;
}
.kit-in-browser > wad-icon { color: var(--c-app); }
.kit-code { font-family: monospace; font-size: 13px; font-weight: 700; letter-spacing: 0.08em; color: var(--fg); padding: 2px 8px; border-radius: 6px; background-color: var(--surface-3); }

/* ---- empty or away: a big glyph, a title, what to do */
.kit-big { min-width: 72px; min-height: 72px; border-radius: 22px; background-image: linear-gradient(160deg, #123247, #0c1b2b); color: var(--c-app); }
.kit-hero-title { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 17px; }
.kit-hero-body { color: var(--muted); }
"#;

thread_local! {
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
}

/// Adds the kit's CSS to `display`, once.
pub fn install(display: &gdk::Display) {
    if INSTALLED.with(|i| i.replace(true)) {
        return;
    }
    crate::theme::add_css(display, CSS);
    crate::theme::add_css(display, crate::form::CSS);
}

/// A button's look (`ui/Button.svelte`'s variants).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Primary,
    Secondary,
    Ghost,
    Danger,
    Quiet,
}

impl Variant {
    pub fn class(self) -> &'static str {
        match self {
            Variant::Primary => "primary",
            Variant::Secondary => "secondary",
            Variant::Ghost => "ghost",
            Variant::Danger => "danger",
            Variant::Quiet => "quiet",
        }
    }
}

/// A button: `label` (none: the icon alone), `icon` before it; `small` is the spike's `sm`.
pub fn button(label: &str, variant: Variant, icon: Option<&'static str>, small: bool) -> gtk::Button {
    let b = gtk::Button::builder().css_classes(["kit-btn", variant.class()]).build();
    if small {
        b.add_css_class("sm");
    }
    let inner = gtk::Box::builder().spacing(if small { 6 } else { 8 }).halign(gtk::Align::Center).build();
    if let Some(i) = icon {
        let ic = Icon::new(i, if small { 14 } else { 16 });
        ic.set_valign(gtk::Align::Center);
        inner.append(&ic);
    }
    if label.is_empty() {
        b.add_css_class("icon-only");
    } else {
        inner.append(&gtk::Label::new(Some(label)));
    }
    b.set_child(Some(&inner));
    b
}

/// A machine app's title bar (the Builder's look): its glyph and name, and
/// the window's buttons; set on `window`, which gets its rounded corners.
pub fn title_bar(window: &gtk::ApplicationWindow, icon: &'static str, title: &str) -> gtk::HeaderBar {
    window.add_css_class("kit-window");
    let head = gtk::HeaderBar::builder().css_classes(["kit-head"]).build();
    head.set_decoration_layout(Some(":minimize,maximize,close"));
    let t = gtk::Box::builder().spacing(8).build();
    let ic = Icon::new(icon, 16);
    ic.set_valign(gtk::Align::Center);
    t.append(&ic);
    t.append(&gtk::Label::builder().label(title).css_classes(["kit-head-title"]).build());
    head.pack_start(&t);
    head.set_title_widget(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    window.set_titlebar(Some(&head));
    head
}

/// A label with these classes: it wraps to the room it's given (asking
/// for no more than about 60 characters' width, so a long one doesn't widen
/// what it's in).
pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .max_width_chars(60)
        .build();
    for c in classes {
        l.add_css_class(c);
    }
    l
}

/// A row's icon tile tone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Accent,
    Warn,
    Danger,
    Magenta,
}

impl Tone {
    pub fn class(self) -> &'static str {
        match self {
            Tone::Plain => "plain",
            Tone::Accent => "accent",
            Tone::Warn => "warn",
            Tone::Danger => "danger",
            Tone::Magenta => "magenta",
        }
    }
}

/// An icon in its rounded tile, `side` square.
pub fn icon_tile(icon: &'static str, tone: Tone, side: i32, size: i32) -> gtk::Box {
    let b = gtk::Box::builder().css_classes(["kit-ic", tone.class()]).valign(gtk::Align::Center).hexpand(false).build();
    b.set_size_request(side, side);
    let i = Icon::new(icon, size);
    i.set_hexpand(true);
    i.set_halign(gtk::Align::Center);
    i.set_valign(gtk::Align::Center);
    b.append(&i);
    b
}

/// A panel's page (Settings' `parts/Page.svelte`): its bar (`back`, a
/// back button at its start, when it's a page within a panel; the title;
/// `end` for the panel's own switch), over a scrolling, centred column of
/// groups (`column`, 644 wide, as Accounts').
pub struct Page {
    pub root: gtk::Box,
    pub column: gtk::Box,
    pub end: gtk::Box,
    pub back: Option<gtk::Button>,
    pub title: gtk::Label,
}

impl Page {
    pub fn new(title: &str, back: bool) -> Self {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        let bar = gtk::CenterBox::builder().css_classes(["kit-page-bar"]).build();
        let back = back.then(|| {
            let b = gtk::Button::builder().child(&Icon::new("chevron-left", 18)).css_classes(["kit-back"]).build();
            b.set_valign(gtk::Align::Center);
            b.set_tooltip_text(Some("Back"));
            bar.set_start_widget(Some(&b));
            b
        });
        let t = gtk::Label::builder().label(title).css_classes(["kit-page-title"]).build();
        bar.set_center_widget(Some(&t));
        let end = gtk::Box::builder().spacing(8).valign(gtk::Align::Center).halign(gtk::Align::End).build();
        bar.set_end_widget(Some(&end));
        root.append(&bar);
        let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(28)
            .css_classes(["kit-column"])
            .halign(gtk::Align::Center)
            .build();
        column.set_size_request(644, -1);
        let outer = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        outer.append(&column);
        scroll.set_child(Some(&outer));
        root.append(&scroll);
        Self { root, column, end, back, title: t }
    }

    pub fn add(&self, w: &impl IsA<gtk::Widget>) {
        self.column.append(w);
    }
}

/// A titled group (`parts/Group.svelte`): its head (a title over a
/// description, and `suffix` at its end), then its rows in a boxed list.
pub struct Group {
    pub root: gtk::Box,
    pub list: gtk::Box,
    pub suffix: gtk::Box,
}

impl Group {
    pub fn new(title: &str, description: &str) -> Self {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
        let head = gtk::Box::builder().spacing(12).css_classes(["kit-group-head"]).build();
        let hd = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        if !title.is_empty() {
            hd.append(&label(title, &["kit-group-title"]));
        }
        if !description.is_empty() {
            hd.append(&label(description, &["kit-group-desc"]));
        }
        head.append(&hd);
        let suffix = gtk::Box::builder().spacing(8).valign(gtk::Align::End).build();
        head.append(&suffix);
        root.append(&head);
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["kit-list"]).build();
        list.set_overflow(gtk::Overflow::Hidden);
        root.append(&list);
        Self { root, list, suffix }
    }

    pub fn add(&self, w: &impl IsA<gtk::Widget>) {
        self.list.append(w);
    }
}

/// A boxed list's row (`parts/Row.svelte`): an icon, a title over a
/// subtitle, then a value and its controls (`ctl`) at its end.
pub struct Row {
    pub root: gtk::Box,
    pub ctl: gtk::Box,
}

impl Row {
    pub fn new(icon: Option<&'static str>, tone: Tone, title: &str, subtitle: &str) -> Self {
        let root = gtk::Box::builder().spacing(12).css_classes(["kit-row"]).build();
        if let Some(i) = icon {
            root.append(&icon_tile(i, tone, 30, 17));
        }
        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(1)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        text.append(&label(title, &["kit-row-title"]));
        if !subtitle.is_empty() {
            text.append(&label(subtitle, &["kit-row-sub"]));
        }
        root.append(&text);
        let ctl = gtk::Box::builder().spacing(8).valign(gtk::Align::Center).halign(gtk::Align::End).build();
        root.append(&ctl);
        Self { root, ctl }
    }

    /// Its value at its end (`mono`: as code).
    pub fn value(self, v: &str, mono: bool) -> Self {
        let l = gtk::Label::builder().label(v).xalign(1.0).selectable(true).css_classes(["kit-row-value"]).build();
        if mono {
            l.add_css_class("mono");
        }
        self.ctl.append(&l);
        self
    }

    /// Its name for tests (`wadsctl`-style dumps) and CSS.
    pub fn named(self, name: &str) -> Self {
        self.root.set_widget_name(name);
        self
    }
}

/// A chip (Settings' `.chip`): `ok` lit, `busy` with a spinner.
pub fn chip(text: &str, icon: Option<&'static str>, class: Option<&str>) -> gtk::Box {
    let b = gtk::Box::builder().spacing(6).css_classes(["kit-chip"]).valign(gtk::Align::Center).build();
    if let Some(c) = class {
        b.add_css_class(c);
        if c == "busy" {
            let s = gtk::Spinner::builder().spinning(true).build();
            s.set_size_request(12, 12);
            b.append(&s);
        }
    }
    if let Some(i) = icon {
        let ic = Icon::new(i, 12);
        ic.set_valign(gtk::Align::Center);
        b.append(&ic);
    }
    b.append(&gtk::Label::new(Some(text)));
    b
}

/// A job's stages as they go (`parts/Stages.svelte`): each stage's state,
/// `extra` under them (a sign-in in the browser), the progress while it
/// runs, why it stopped when it failed.
pub fn stages(job: &Job, extra: Option<&gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(7).css_classes(["kit-stages"]).build();
    let running = job.status == JobStatus::Running;
    for s in &job.stages {
        let row = gtk::Box::builder().spacing(10).css_classes(["kit-st", state_class(s.state)]).build();
        if s.state == PartState::Working && running {
            let sp = gtk::Spinner::builder().spinning(true).build();
            sp.set_size_request(14, 14);
            row.append(&sp);
        } else {
            row.append(&Icon::new(state_icon(s.state), 14));
        }
        row.append(&label(&s.label, &[]));
        b.append(&row);
    }
    if let Some(e) = extra {
        b.append(e);
    }
    match job.status {
        JobStatus::Running => {
            let p = gtk::ProgressBar::builder()
                .fraction(job.progress.clamp(0.0, 1.0))
                .css_classes(["kit-progress"])
                .build();
            b.append(&p);
        }
        JobStatus::Error => {
            b.append(&label(job.error.as_deref().unwrap_or("It didn’t work"), &["kit-err"]));
        }
        _ => {}
    }
    b
}

fn state_class(s: PartState) -> &'static str {
    match s {
        PartState::Waiting => "waiting",
        PartState::Working => "working",
        PartState::Done => "done",
        PartState::Error => "error",
        PartState::Skipped => "skipped",
    }
}

/// A stage's icon (Stages.svelte's ICON).
pub fn state_icon(s: PartState) -> &'static str {
    match s {
        PartState::Waiting | PartState::Working => "circle",
        PartState::Done => "check",
        PartState::Error => "circle-alert",
        PartState::Skipped => "minus",
    }
}

/// What a sign-in waiting in the browser says (InBrowser.svelte), `name`
/// the provider's ("GitHub").
pub fn in_browser_text(name: &str) -> String {
    format!("Finish signing in to {name} in the browser window.")
}

/// A sign-in waiting in the browser: finish it there (GitHub's: with
/// `code`, which its window shows too); Show the browser (`show`) brings
/// its window back.
pub fn in_browser(name: &str, code: Option<&str>, show: impl Fn() + 'static) -> gtk::Box {
    let b = gtk::Box::builder().spacing(10).css_classes(["kit-in-browser"]).build();
    let ic = Icon::new("globe", 16);
    ic.set_valign(gtk::Align::Center);
    b.append(&ic);
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    text.append(&label(&in_browser_text(name), &[]));
    if let Some(c) = code.filter(|c| !c.is_empty()) {
        let row = gtk::Box::builder().spacing(6).build();
        row.append(&label("Its code:", &["kit-row-sub"]));
        let l = gtk::Label::builder().label(c).css_classes(["kit-code"]).build();
        l.set_widget_name("signin-code");
        row.append(&l);
        text.append(&row);
    }
    b.append(&text);
    let btn = button("Show the browser", Variant::Secondary, Some("app-window"), true);
    btn.set_valign(gtk::Align::Center);
    btn.set_widget_name("show-browser");
    btn.connect_clicked(move |_| show());
    b.append(&btn);
    b
}

/// A big glyph over a title and what to do (Files' `.join`, an empty page).
pub fn hero(icon: &'static str, title: &str, body: &str) -> gtk::Box {
    let b = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .vexpand(true)
        .build();
    b.set_size_request(-1, -1);
    let big = gtk::Box::builder().css_classes(["kit-big"]).halign(gtk::Align::Center).hexpand(false).build();
    let i = Icon::new(icon, 34);
    i.set_hexpand(true);
    i.set_halign(gtk::Align::Center);
    i.set_valign(gtk::Align::Center);
    big.append(&i);
    b.append(&big);
    let t = label(title, &["kit-hero-title"]);
    t.set_xalign(0.5);
    t.set_justify(gtk::Justification::Center);
    b.append(&t);
    if !body.is_empty() {
        let p = label(body, &["kit-hero-body"]);
        p.set_xalign(0.5);
        p.set_justify(gtk::Justification::Center);
        p.set_max_width_chars(46);
        b.append(&p);
    }
    b
}

/// What Tailscale says a device runs, as people write it ('android' → 'Android').
pub fn os_name(os: &str) -> String {
    match os.to_ascii_lowercase().as_str() {
        "android" => "Android".into(),
        "ios" => "iOS".into(),
        "ipados" => "iPadOS".into(),
        "macos" | "darwin" => "macOS".into(),
        "linux" => "Linux".into(),
        "windows" => "Windows".into(),
        "freebsd" => "FreeBSD".into(),
        "" => "Unknown".into(),
        _ => os.to_string(),
    }
}

/// How long ago `at` was at `now` (both ms), as the spike's `ago`.
pub fn ago(now_ms: i64, at_ms: i64) -> String {
    if at_ms <= 0 {
        return "never".into();
    }
    let s = ((now_ms - at_ms).max(0) as f64) / 1000.0;
    if s < 45.0 {
        return "just now".into();
    }
    if s < 3600.0 {
        return format!("{} min ago", (s / 60.0).round() as i64);
    }
    if s < 86400.0 {
        return format!("{} h ago", (s / 3600.0).round() as i64);
    }
    let d = (s / 86400.0).round() as i64;
    format!("{d} day{} ago", if d == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ago_reads_as_the_spikes() {
        let now = 1_000_000_000;
        assert_eq!(ago(now, 0), "never");
        assert_eq!(ago(now, now - 10_000), "just now");
        assert_eq!(ago(now, now - 5 * 60_000), "5 min ago");
        assert_eq!(ago(now, now - 3 * 3_600_000), "3 h ago");
        assert_eq!(ago(now, now - 51 * 3_600_000), "2 days ago");
        assert_eq!(ago(now, now - 24 * 3_600_000), "1 day ago");
    }

    #[test]
    fn oses_and_the_in_browser_words() {
        assert_eq!(os_name("android"), "Android");
        assert_eq!(os_name("iOS"), "iOS");
        assert_eq!(os_name("plan9"), "plan9");
        assert_eq!(in_browser_text("GitHub"), "Finish signing in to GitHub in the browser window.");
        assert_eq!(state_icon(PartState::Done), "check");
    }
}
