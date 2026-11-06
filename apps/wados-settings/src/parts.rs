//! What the session panels (Displays, Sound, Power, Background, Lock,
//! Mouse & Touchpad, Keyboard, System) share beyond `kit`'s rows: the
//! spike's Row variants (`wide`, `stacked`, a whole row that's a button,
//! `active`, `nav`), a radio, the slider ends' words, and work done off the
//! main loop.

use std::cell::Cell;

use gtk::{gio, glib, prelude::*};
use wad_gtk::Icon;
use wad_gtk::kit::{self, Row};

const CSS: &str = r#"
.set-click:hover { background-color: color-mix(in srgb, var(--surface-3) 70%, transparent); }
.set-active { background-color: color-mix(in srgb, var(--accent) 7%, transparent); }
.set-radio { min-width: 16px; min-height: 16px; border-radius: 50%; border: 2px solid var(--line-strong); }
.set-radio.on { border-color: var(--accent); background-image: radial-gradient(circle, var(--accent) 0, var(--accent) 5px, transparent 5.5px); }
.set-end { font-size: 12px; color: var(--faint); }
.set-end.warm { color: var(--warn); }
.set-pct { min-width: 38px; font-size: 12.5px; color: var(--muted); }
.set-nav { color: var(--faint); }
.set-stacked-ctl { padding: 0 14px 12px 14px; }
.set-stacked-ctl.indent { padding-left: 56px; }
.set-note { font-size: 12.5px; color: var(--faint); }
.set-err { font-size: 13px; color: var(--danger); }
.set-ok { font-size: 13px; color: var(--accent); }
.set-hint { font-size: 12px; color: var(--faint); }
.set-inset { padding: 12px; }
.set-hero-title { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 20px; margin-top: 8px; }
.set-hero-sub { color: var(--muted); }
.set-big-time { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 48px; }
"#;

thread_local! {
    static CSS_ADDED: Cell<bool> = const { Cell::new(false) };
}

/// Their stylesheet (once).
pub fn install() {
    if !CSS_ADDED.with(|c| c.replace(true))
        && let Some(d) = gtk::gdk::Display::default()
    {
        wad_gtk::theme::add_css(&d, CSS);
    }
}

/// `work` off the main loop, then `done` with what it gave, on it.
pub fn spawn<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    let h = gio::spawn_blocking(work);
    glib::spawn_future_local(async move {
        if let Ok(r) = h.await {
            done(r);
        }
    });
}

/// The row's text, its second child (after its icon, if it has one).
fn text_of(row: &Row) -> Option<gtk::Widget> {
    row.ctl.prev_sibling()
}

/// A `wide` row: its control takes about half of it (a slider between its end words).
pub fn wide(row: &Row) {
    if let Some(t) = text_of(row) {
        t.set_hexpand(true);
    }
    row.ctl.set_hexpand(true);
    row.ctl.set_halign(gtk::Align::Fill);
    row.ctl.set_size_request(290, -1);
    row.ctl.set_spacing(10);
}

/// A whole row that's a button.
pub fn clickable(row: &Row, f: impl Fn() + 'static) {
    row.root.add_css_class("set-click");
    row.root.set_focusable(true);
    row.root.set_cursor_from_name(Some("pointer"));
    let click = gtk::GestureClick::new();
    click.connect_released(move |g, _, _, _| {
        g.set_state(gtk::EventSequenceState::Claimed);
        f();
    });
    row.root.add_controller(click);
}

/// Lit as the one chosen.
pub fn set_active(row: &gtk::Box, on: bool) {
    if on {
        row.add_css_class("set-active");
    } else {
        row.remove_css_class("set-active");
    }
}

/// A radio (a mode, a scroll direction), on or off.
pub fn radio(on: bool) -> gtk::Box {
    let r = gtk::Box::builder().css_classes(["set-radio"]).valign(gtk::Align::Center).build();
    set_radio(&r, on);
    r
}

pub fn set_radio(r: &gtk::Box, on: bool) {
    if on {
        r.add_css_class("on");
    } else {
        r.remove_css_class("on");
    }
}

/// A chevron (another page) or an arrow out (another app) at a row's end.
pub fn nav(row: &Row, external: bool) {
    let i = Icon::new(if external { "external-link" } else { "chevron-right" }, 16);
    i.add_css_class("set-nav");
    i.set_valign(gtk::Align::Center);
    row.root.append(&i);
}

/// A slider's end word ("Slow", "Fast"; `warm`: "Warmer").
pub fn end(text: &str, warm: bool) -> gtk::Label {
    let l = gtk::Label::builder().label(text).css_classes(["set-end"]).valign(gtk::Align::Center).build();
    if warm {
        l.add_css_class("warm");
    }
    l
}

/// A percent after a slider ("80%").
pub fn pct(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(1.0).css_classes(["set-pct"]).valign(gtk::Align::Center).build()
}

/// A `stacked` row: its line, then its control under it (indented past its icon).
pub fn stacked(icon: Option<&'static str>, tone: kit::Tone, title: &str, subtitle: &str) -> (gtk::Box, gtk::Box) {
    let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["kit-row-stack"]).build();
    let row = Row::new(icon, tone, title, subtitle);
    root.append(&row.root);
    let ctl = gtk::Box::builder().spacing(8).css_classes(["set-stacked-ctl"]).build();
    if icon.is_some() {
        ctl.add_css_class("indent");
    }
    root.append(&ctl);
    (root, ctl)
}

/// A line of small words (a note under a group, what went wrong).
pub fn note(text: &str, class: &str) -> gtk::Label {
    let l = kit::label(text, &[class]);
    l.set_visible(!text.is_empty());
    l
}

/// A row's subtitle label, to change what it says.
pub fn subtitle_of(row: &Row) -> Option<gtk::Label> {
    let t = text_of(row)?;
    let first = t.first_child()?;
    first.next_sibling().and_downcast::<gtk::Label>()
}

/// A row's title label.
pub fn title_of(row: &Row) -> Option<gtk::Label> {
    text_of(row)?.first_child().and_downcast::<gtk::Label>()
}

/// Sets a row's subtitle (made if it had none).
pub fn set_subtitle(row: &Row, text: &str) {
    match subtitle_of(row) {
        Some(l) => {
            l.set_label(text);
            l.set_visible(!text.is_empty());
        }
        None if !text.is_empty() => {
            if let Some(t) = text_of(row).and_downcast::<gtk::Box>() {
                t.append(&kit::label(text, &["kit-row-sub"]));
            }
        }
        None => {}
    }
}

// ------------------------------------------------------------ D-Bus

/// A system service's properties (`GetAll`), as a dictionary.
pub fn get_all(name: &str, path: &str, iface: &str, done: impl FnOnce(Result<glib::VariantDict, String>) + 'static) {
    call(name, path, "org.freedesktop.DBus.Properties", "GetAll", (iface,).to_variant(), false, move |r| {
        done(r.map(|v| glib::VariantDict::new(Some(&v.child_value(0)))))
    });
}

/// Sets a system service's property (interactive: polkit may ask).
pub fn set_prop(
    name: &str,
    path: &str,
    iface: &str,
    prop: &str,
    value: glib::Variant,
    done: impl FnOnce(Result<(), String>) + 'static,
) {
    call(name, path, "org.freedesktop.DBus.Properties", "Set", (iface, prop, value).to_variant(), true, move |r| {
        done(r.map(|_| ()))
    });
}

/// A method of a system service; `interactive`: polkit may ask for a password.
pub fn call(
    name: &str,
    path: &str,
    iface: &str,
    method: &str,
    args: glib::Variant,
    interactive: bool,
    done: impl FnOnce(Result<glib::Variant, String>) + 'static,
) {
    let bus = match gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE) {
        Ok(b) => b,
        Err(e) => return done(Err(format!("the system bus: {}", e.message()))),
    };
    let flags =
        if interactive { gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION } else { gio::DBusCallFlags::NONE };
    let c = bus.call_future(
        Some(name),
        path,
        iface,
        method,
        Some(&args),
        None,
        flags,
        if interactive { i32::MAX } else { 5000 },
    );
    glib::spawn_future_local(async move {
        done(c.await.map_err(|e| dbus_words(&e)));
    });
}

/// A D-Bus error in words: polkit's refusal said plainly.
pub fn dbus_words(e: &glib::Error) -> String {
    let remote = gio::DBusError::remote_error(e).map(|s| s.to_string()).unwrap_or_default();
    let mut m = e.message().to_string();
    if let Some(i) = m.find(": ").filter(|_| m.starts_with("GDBus.Error:")) {
        m = m[i + 2..].to_string();
    }
    if remote.contains("AccessDenied")
        || remote.contains("NotAuthorized")
        || remote.contains("InteractiveAuthorizationRequired")
    {
        return format!("not allowed from here: polkit lets only the person at the machine do it ({m})");
    }
    if remote.contains("ServiceUnknown") || remote.contains("NameHasNoOwner") {
        return "The service isn't running on this machine".into();
    }
    m
}

/// "4 h 12 min", "35 min" (the spike's `duration`).
pub fn duration(minutes: f64) -> String {
    let m = minutes.round().max(1.0) as i64;
    let h = m / 60;
    if h > 0 { format!("{h} h {} min", m % 60) } else { format!("{m} min") }
}

// ------------------------------------------------------------ labwc

/// A mouse or keyboard setting changed: saved, then labwc's rc.xml and
/// environment written from them all and labwc told to read them again
/// (off the main loop); `done` with what went wrong, if anything.
pub fn input_changed(
    f: impl FnOnce(&mut wad_desk::settings::Settings),
    done: impl FnOnce(Result<(), String>) + 'static,
) {
    let s = match wad_desk::settings::Settings::update(f) {
        Ok(s) => s,
        Err(e) => return done(Err(e)),
    };
    spawn(move || wad_desk::labwc::apply(&s.mouse, &s.keyboard), done);
}

#[cfg(test)]
mod tests {
    #[test]
    fn durations() {
        assert_eq!(super::duration(252.0), "4 h 12 min");
        assert_eq!(super::duration(35.4), "35 min");
        assert_eq!(super::duration(0.0), "1 min");
        assert_eq!(super::duration(60.0), "1 h 0 min");
    }

    #[test]
    fn a_polkit_refusal_in_words() {
        let e = gtk::gio::DBusError::new_for_dbus_error(
            "org.freedesktop.DBus.Error.AccessDenied",
            "Not Authorized: org.freedesktop.UPower.PowerProfiles.switch-profile",
        );
        assert_eq!(
            crate::panels::power::refused_words(&super::dbus_words(&e)),
            "Couldn't change the power mode: not allowed from here: polkit lets only the person at the machine do it \
             (Not Authorized: org.freedesktop.UPower.PowerProfiles.switch-profile)"
        );
        let gone = gtk::gio::DBusError::new_for_dbus_error("org.freedesktop.DBus.Error.ServiceUnknown", "no");
        assert_eq!(super::dbus_words(&gone), "The service isn't running on this machine");
    }
}
