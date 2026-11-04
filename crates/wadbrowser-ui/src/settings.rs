//! GTK settings dialog (edits Preferences and persists to TOML).

use gtk::glib::{self, clone};
use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, Button, CheckButton, DropDown, Entry, Label, Orientation, SpinButton, Window};
use std::cell::RefCell;
use std::rc::Rc;
use wadbrowser_core::search::{BUILTIN_ENGINES, engine_from_id, id_for_engine};
use wadbrowser_core::{AppPaths, Preferences, SearchEngine};

pub fn open_settings_dialog(
    parent: &impl IsA<gtk::Window>,
    paths: AppPaths,
    prefs: Rc<RefCell<Preferences>>,
    on_saved: impl Fn(Preferences) + 'static,
) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("WadBrowser Settings")
        .default_width(520)
        .default_height(560)
        .build();

    let root = GtkBox::new(Orientation::Vertical, 12);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);

    let current = prefs.borrow().clone();

    let homepage = Entry::builder().text(&current.homepage).hexpand(true).build();
    root.append(&labeled("Homepage", &homepage));

    let names: Vec<&str> = BUILTIN_ENGINES.iter().map(|e| e.name).collect();
    let engine_drop = DropDown::from_strings(&names);
    let selected_id = id_for_engine(&current.search_engine);
    if let Some(idx) = BUILTIN_ENGINES.iter().position(|e| e.id == selected_id) {
        engine_drop.set_selected(idx as u32);
    }
    root.append(&labeled_widget("Search engine", &engine_drop));

    let https_first = CheckButton::with_label("HTTPS-First for typed URLs");
    https_first.set_active(current.https_first);
    let block = CheckButton::with_label("Block ads & trackers");
    block.set_active(current.block_ads_trackers);
    let cosmetic = CheckButton::with_label("Cosmetic filtering");
    cosmetic.set_active(current.block_cosmetic_filters);
    let dnt = CheckButton::with_label("Send Do Not Track / navigator.doNotTrack");
    dnt.set_active(current.send_dnt);
    let gpc = CheckButton::with_label("Enable Global Privacy Control signal");
    gpc.set_active(current.send_gpc);
    let restore = CheckButton::with_label("Restore previous session");
    restore.set_active(current.restore_session);
    let bookmark_bar = CheckButton::with_label("Show bookmark bar");
    bookmark_bar.set_active(current.show_bookmark_bar);
    let js = CheckButton::with_label("Enable JavaScript");
    js.set_active(current.enable_javascript);
    let smooth = CheckButton::with_label("Smooth scrolling");
    smooth.set_active(current.enable_smooth_scrolling);
    let clear_on_quit = CheckButton::with_label("Clear history on quit");
    clear_on_quit.set_active(current.clear_history_on_quit);

    for w in [&https_first, &block, &cosmetic, &dnt, &gpc, &restore, &bookmark_bar, &js, &smooth, &clear_on_quit] {
        root.append(w);
    }

    let zoom = SpinButton::with_range(0.5, 3.0, 0.1);
    zoom.set_value(current.default_zoom);
    root.append(&labeled_widget("Default zoom", &zoom));

    let suggest_limit = SpinButton::with_range(1.0, 30.0, 1.0);
    suggest_limit.set_value(current.omnibox_suggestion_limit as f64);
    root.append(&labeled_widget("Omnibox suggestions", &suggest_limit));

    let stats = Label::new(Some("Privacy lists: EasyList + EasyPrivacy (cached under ~/.cache/wadbrowser/filters)"));
    stats.add_css_class("dim-label");
    stats.set_wrap(true);
    root.append(&stats);

    let buttons = GtkBox::new(Orientation::Horizontal, 8);
    buttons.set_halign(Align::End);
    let cancel = Button::with_label("Cancel");
    let save = Button::with_label("Save");
    save.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&save);
    root.append(&buttons);

    let scroll = gtk::ScrolledWindow::builder().child(&root).vexpand(true).build();
    dialog.set_child(Some(&scroll));

    cancel.connect_clicked(clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));

    save.connect_clicked(clone!(
        #[weak]
        dialog,
        #[strong]
        prefs,
        #[strong]
        homepage,
        #[strong]
        engine_drop,
        #[strong]
        https_first,
        #[strong]
        block,
        #[strong]
        cosmetic,
        #[strong]
        dnt,
        #[strong]
        gpc,
        #[strong]
        restore,
        #[strong]
        bookmark_bar,
        #[strong]
        js,
        #[strong]
        smooth,
        #[strong]
        clear_on_quit,
        #[strong]
        zoom,
        #[strong]
        suggest_limit,
        #[strong]
        paths,
        move |_| {
            let mut next = prefs.borrow().clone();
            next.homepage = homepage.text().to_string();
            next.https_first = https_first.is_active();
            next.block_ads_trackers = block.is_active();
            next.block_cosmetic_filters = cosmetic.is_active();
            next.send_dnt = dnt.is_active();
            next.send_gpc = gpc.is_active();
            next.restore_session = restore.is_active();
            next.show_bookmark_bar = bookmark_bar.is_active();
            next.enable_javascript = js.is_active();
            next.enable_smooth_scrolling = smooth.is_active();
            next.clear_history_on_quit = clear_on_quit.is_active();
            next.default_zoom = zoom.value();
            next.omnibox_suggestion_limit = suggest_limit.value() as usize;

            let idx = engine_drop.selected() as usize;
            if let Some(info) = BUILTIN_ENGINES.get(idx) {
                if let Some(engine) = engine_from_id(info.id) {
                    next.search_engine = engine;
                } else {
                    next.search_engine =
                        SearchEngine::Custom { name: info.name.into(), url_template: info.url_template.into() };
                }
            }

            if let Err(err) = next.validate() {
                tracing::warn!("settings invalid: {err}");
                return;
            }
            if let Err(err) = next.save(&paths) {
                tracing::warn!("settings save failed: {err}");
                return;
            }
            *prefs.borrow_mut() = next.clone();
            on_saved(next);
            dialog.close();
        }
    ));

    dialog.present();
}

fn labeled(text: &str, widget: &impl IsA<gtk::Widget>) -> GtkBox {
    labeled_widget(text, widget)
}

fn labeled_widget(text: &str, widget: &impl IsA<gtk::Widget>) -> GtkBox {
    let row = GtkBox::new(Orientation::Vertical, 4);
    let label = Label::new(Some(text));
    label.set_halign(Align::Start);
    row.append(&label);
    row.append(widget);
    row
}
