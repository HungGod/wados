//! The form controls (wad_gtk::form) on one page, to look at and screenshot:
//!
//!     cargo run -p wad-gtk --example form-gallery [-- --dialog | --trial | --choice | --typed]
//!
//! `--dialog` opens a dialog over it, `--trial` the Displays trial,
//! `--choice` a drop-down's menu, `--typed` fills the fields in (the
//! password shown). What each control's set to prints as it changes.

use std::rc::Rc;

use gtk::{glib, prelude::*};
use wad_gtk::form::{self, Choice, Dialog, Empty, Hue, Level, Meter, Segmented, Slider, TextField, Toggle, opt};
use wad_gtk::kit::{self, Group, Row, Tone, Variant};

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let has = |a: &str| args.iter().any(|x| x == a);
    let (dialog, trial, choice, typed) = (has("--dialog"), has("--trial"), has("--choice"), has("--typed"));
    let app = gtk::Application::builder()
        .application_id("io.wados.FormGallery")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| build(app, dialog, trial, choice, typed));
    app.run_with_args(&args[..1])
}

fn build(app: &gtk::Application, dialog: bool, trial: bool, choice: bool, typed: bool) {
    let d = gtk::gdk::Display::default().expect("a display");
    wad_gtk::theme::install(&d);
    wad_gtk::kit::install(&d);
    wad_gtk::theme::add_css(&d, "window.gallery { background-color: var(--surface); }");
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Form gallery")
        .default_width(1100)
        .default_height(1200)
        .css_classes(["gallery"])
        .build();
    kit::title_bar(&window, "settings", "Form gallery");

    let page = kit::Page::new("Form controls", true);
    page.column.set_size_request(980, -1);
    let cols = gtk::Box::builder().spacing(28).homogeneous(true).build();
    let left = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(28).build();
    let right = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(28).build();
    cols.append(&left);
    cols.append(&right);
    page.add(&cols);

    // Toggles, a slider, a drop-down, a segmented control: as Displays has them.
    let g = Group::new("Brightness & Color", "Toggle, Slider");
    let r = Row::new(Some("sun"), Tone::Plain, "Brightness", "");
    let pct = gtk::Label::builder().label("70%").width_chars(4).xalign(1.0).css_classes(["kit-row-value"]).build();
    let p = pct.clone();
    let s = Slider::new(
        0.15,
        1.0,
        0.01,
        0.7,
        move |v| p.set_label(&format!("{}%", (v * 100.0).round())),
        |v| println!("brightness committed {v}"),
    );
    r.ctl.append(&s.root);
    r.ctl.append(&pct);
    r.ctl.set_hexpand(true);
    g.add(&r.root);
    let r = Row::new(Some("moon-star"), Tone::Warn, "Night Light", "Makes the screen warmer, which can help you sleep");
    r.ctl.append(&Toggle::new(true, |on| println!("night light {on}")).root);
    g.add(&r.root);
    let r = Row::new(None, Tone::Plain, "Automatic Brightness", "Off");
    r.ctl.append(&Toggle::new(false, |on| println!("auto {on}")).root);
    g.add(&r.root);
    let r = Row::new(None, Tone::Plain, "Unavailable", "A toggle that can't be used");
    let t = Toggle::new(true, |_| {});
    t.root.set_sensitive(false);
    r.ctl.append(&t.root);
    g.add(&r.root);
    left.append(&g.root);

    let g = Group::new("Display", "Choice, Segmented");
    let r = Row::new(None, Tone::Plain, "Refresh Rate", "");
    let refresh = Choice::new([("60.00 Hz", 60), ("120.00 Hz", 120)], 120, |v| println!("refresh {v}"));
    r.ctl.append(&refresh.root);
    g.add(&r.root);
    let r = Row::new(None, Tone::Plain, "Exit Node", "Where this machine's traffic leaves the tailnet");
    let exit = Choice::new(
        [
            opt("", "None").hint("Straight out"),
            opt("desk", "Desk").hint("linux"),
            opt("pixel", "Pixel 8").hint("android"),
        ],
        "",
        |v| println!("exit node {v:?}"),
    );
    r.ctl.append(&exit.root);
    g.add(&r.root);
    let r = Row::new(None, Tone::Plain, "Scale", "Makes everything bigger: 1500 × 1000 to work in");
    let scale = Segmented::new([("100%", 100), ("125%", 125), ("150%", 150), ("200%", 200)], 200, true, |v| {
        println!("scale {v}")
    });
    r.ctl.append(&scale.root);
    g.add(&r.root);
    let r = Row::new(None, Tone::Plain, "Power Mode", "");
    let mode = Segmented::new(
        [
            opt("saver", "Saver").icon("leaf"),
            opt("balanced", "Balanced").icon("scale"),
            opt("perf", "Performance").icon("zap"),
        ],
        "balanced",
        false,
        |v| println!("power {v}"),
    );
    r.ctl.append(&mode.root);
    g.add(&r.root);
    left.append(&g.root);

    let g = Group::new("Sound", "Level");
    let r = Row::new(Some("mic"), Tone::Plain, "Input Level", "");
    let lvl = Level::new(28);
    lvl.set(0.9);
    lvl.root.set_size_request(240, -1);
    r.ctl.append(&lvl.root);
    g.add(&r.root);
    left.append(&g.root);

    let g = Group::new("Keyboard Shortcuts", "Keys");
    for (what, combo) in [("Lock the screen", "Super+L"), ("Show the launcher", "Alt+Space"), ("Zoom in", "Ctrl++")] {
        let r = Row::new(None, Tone::Plain, what, "");
        r.ctl.append(&form::keys(combo));
        g.add(&r.root);
    }
    left.append(&g.root);

    // Text fields, as Lock Screen's password change has them.
    let g = Group::new("Change Password", "TextField, password with reveal, an error line");
    let inset =
        gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).css_classes(["kit-row"]).build();
    let host = TextField::new("Hostname", "surface").mono();
    let cur = TextField::password("Current password", "");
    let new = TextField::password("New password", "At least 8 characters");
    new.set_error(Some("The new password needs at least 8 characters"));
    {
        let n = new.clone();
        new.on_change(move |v| n.set_error((v.len() < 8).then_some("The new password needs at least 8 characters")));
    }
    if typed {
        host.set_value("surface-book");
        cur.set_value("hunter22");
        new.set_value("short");
    }
    for f in [&host, &cur, &new] {
        inset.append(&f.root);
    }
    g.add(&inset);
    right.append(&g.root);

    // Storage's disk.
    let g = Group::new("Samsung PM9A1 (nvme0n1)", "212.4 GB of 476.9 GB used · 264.5 GB free");
    let m = Meter::new(
        476.9,
        vec![
            form::part("Base images", 38.2, Hue::App),
            form::part("Wadspaces", 121.7, Hue::Accent),
            form::part("System and other", 52.5, Hue::Magenta),
        ],
        Some("Free"),
        |gb| format!("{gb:.1} GB"),
    );
    m.root.add_css_class("kit-row");
    g.add(&m.root);
    right.append(&g.root);

    let g = Group::new("Empty", "Wi-Fi off");
    let e = Empty::new("wifi-off", "Wi-Fi is off", "Turn it on to see the networks in range and join one.");
    e.actions.append(&kit::button("Turn On Wi-Fi", Variant::Primary, None, false));
    g.add(&e.root);
    right.append(&g.root);

    // Dialogs.
    let g = Group::new("Dialogs", "Dialog, and the Displays trial");
    let r = Row::new(Some("app-window"), Tone::Plain, "A Dialog", "Esc and the close button answer Cancel");
    let b = kit::button("Open", Variant::Secondary, None, true);
    let win = window.clone();
    b.connect_clicked(move |_| {
        forget(&win);
    });
    r.ctl.append(&b);
    g.add(&r.root);
    let r = Row::new(Some("monitor"), Tone::Plain, "Displays Trial", "Reverts in 15 seconds");
    let b = kit::button("Try", Variant::Secondary, None, true);
    let win = window.clone();
    b.connect_clicked(move |_| {
        form::display_trial(&win, 15, |kept| println!("display trial: {}", if kept { "kept" } else { "reverted" }));
    });
    r.ctl.append(&b);
    g.add(&r.root);
    right.append(&g.root);

    window.set_child(Some(&page.root));
    window.present();

    let win = window.clone();
    let refresh = Rc::new(refresh);
    glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
        if dialog {
            forget(&win);
        }
        if trial {
            form::display_trial(&win, 15, |kept| println!("display trial: {kept}"));
        }
        if choice {
            refresh.popup();
        }
    });
}

fn forget(win: &gtk::ApplicationWindow) {
    let pw = TextField::password("Its password", "");
    pw.set_value("correct horse");
    Dialog::new("Forget “Home”?")
        .icon("wifi", Tone::Danger)
        .text("You’ll need its password to join it again. Your other networks stay as they are.")
        .child(&pw.root)
        .button("cancel", "Cancel", Variant::Ghost)
        .button("forget", "Forget", Variant::Danger)
        .cancel("cancel")
        .named("forget")
        .open(win, |_, id| println!("dialog answered {id}"));
}
