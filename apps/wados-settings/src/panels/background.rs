//! Settings ▸ Background (the spike's `panels/BackgroundPanel.svelte` and
//! `parts/WallpaperGrid.svelte`): the desktop's background, previewed as
//! the desktop with its panel, then WadSpaces' pictures to pick from. The
//! pick is the settings' `[background] picture`; the shell's wallpapers
//! follow it at once. There's no light or dark style: WadSpaces is dark only.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use wad_desk::settings::Settings;
use wad_gtk::Icon;
use wad_gtk::kit::{self, Group};
use wad_model::wallpapers::PICTURES;

use crate::panel::{Ctx, Panel};
use crate::parts;

const CSS: &str = r#"
.bg-shot { border-radius: 14px; box-shadow: inset 0 0 0 1px var(--line), 0 16px 40px -18px rgb(0 0 0 / 0.6); }
.bg-icon { min-width: 26px; min-height: 20px; border-radius: 5px; background-color: rgb(255 255 255 / 0.16); box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.18); }
.bg-panel { min-height: 16px; padding: 0 8px; background-color: rgb(10 6 20 / 0.82); border-top: 1px solid var(--line); }
.bg-dot { min-width: 8px; min-height: 8px; border-radius: 50%; background-color: var(--accent); }
.bg-bar { min-width: 60px; min-height: 4px; border-radius: 2px; background-color: rgb(255 255 255 / 0.2); }
.bg-name { font-size: 16px; font-weight: 600; }
.bg-what { font-size: 12.5px; color: var(--muted); }
.bg-dark { margin: 4px 4px 0; font-size: 13px; color: var(--muted); }
.bg-h4 { margin: 8px 4px 0; font-size: 11.5px; font-weight: 600; letter-spacing: 0.06em; color: var(--faint); }
button.bg-thumb { padding: 0; border: none; background: none; box-shadow: none; }
.bg-wp { border-radius: 10px; box-shadow: inset 0 0 0 1px var(--line); }
button.bg-thumb:hover .bg-wp { box-shadow: 0 0 0 2px var(--line-strong); }
button.bg-thumb.on .bg-wp { box-shadow: 0 0 0 2px var(--accent), 0 0 18px -4px var(--accent); }
.bg-tick { min-width: 22px; min-height: 22px; margin: 6px; border-radius: 50%; background-color: var(--accent); color: var(--accent-fg); }
.bg-tname { font-size: 12px; color: var(--muted); padding: 0 2px; }
button.bg-thumb.on .bg-tname { color: var(--fg); }
"#;

pub fn make(_: &Ctx) -> Rc<dyn Panel> {
    Rc::new_cyclic(|me| Background { me: me.clone(), ui: RefCell::default() })
}

/// A picture's name ("Duo"), its id's if it's none of WadSpaces'.
pub fn name_of(id: &str) -> String {
    PICTURES.iter().find(|p| p.id == id).map(|p| p.name.to_string()).unwrap_or_else(|| "Duo".into())
}

struct Ui {
    shot: gtk::Picture,
    name: gtk::Label,
    thumbs: Vec<(&'static str, gtk::Button)>,
}

pub struct Background {
    me: Weak<Background>,
    ui: RefCell<Option<Ui>>,
}

fn picture(id: &str, w: i32, h: i32, classes: &[&str]) -> gtk::Picture {
    let p = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .overflow(gtk::Overflow::Hidden)
        .build();
    for c in classes {
        p.add_css_class(c);
    }
    p.set_size_request(w, h);
    if let Some(f) = wad_gtk::assets::path(&format!("wallpapers/art/{id}.svg")) {
        p.set_filename(Some(&f));
    }
    p
}

impl Background {
    fn pick(&self, id: &'static str) {
        if let Err(e) = Settings::update(|s| s.background.picture = id.to_string()) {
            tracing::warn!("settings: {e}");
        }
        self.show(id);
    }

    fn show(&self, id: &str) {
        let Some(ui) = &*self.ui.borrow() else { return };
        if let Some(f) = wad_gtk::assets::path(&format!("wallpapers/art/{id}.svg")) {
            ui.shot.set_filename(Some(&f));
        }
        ui.name.set_label(&name_of(id));
        for (p, b) in &ui.thumbs {
            let on = *p == id;
            if on {
                b.add_css_class("on");
            } else {
                b.remove_css_class("on");
            }
            b.update_state(&[gtk::accessible::State::Pressed(if on {
                gtk::AccessibleTristate::True
            } else {
                gtk::AccessibleTristate::False
            })]);
            if let Some(o) =
                b.child().and_downcast::<gtk::Box>().and_then(|c| c.first_child()).and_downcast::<gtk::Overlay>()
            {
                // The tick: the overlay's second child.
                if let Some(t) = o.last_child().filter(|t| t.has_css_class("bg-tick")) {
                    t.set_visible(on);
                }
            }
        }
    }
}

impl Panel for Background {
    fn build(&self, _: &Ctx) -> gtk::Widget {
        parts::install();
        if let Some(d) = gtk::gdk::Display::default() {
            wad_gtk::theme::add_css(&d, CSS);
        }
        let current = Settings::load().background.picture;
        let page = kit::Page::new("Background", false);

        let g = Group::new("", "");
        g.list.set_visible(false);
        let row = gtk::Box::builder().spacing(20).build();
        let shot = gtk::Overlay::builder().overflow(gtk::Overflow::Hidden).css_classes(["bg-shot"]).build();
        let pic = picture(&current, 340, 212, &[]);
        shot.set_child(Some(&pic));
        let icons = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .margin_top(14)
            .margin_start(14)
            .build();
        for _ in 0..3 {
            icons.append(&gtk::Box::builder().css_classes(["bg-icon"]).build());
        }
        shot.add_overlay(&icons);
        let panel = gtk::Box::builder().spacing(8).valign(gtk::Align::End).css_classes(["bg-panel"]).build();
        panel.append(&gtk::Box::builder().css_classes(["bg-dot"]).valign(gtk::Align::Center).build());
        panel.append(&gtk::Box::builder().css_classes(["bg-bar"]).valign(gtk::Align::Center).build());
        shot.add_overlay(&panel);
        row.append(&shot);
        let about =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).valign(gtk::Align::Center).build();
        let name = kit::label(&name_of(&current), &["bg-name"]);
        name.set_widget_name("background-name");
        about.append(&name);
        about.append(&kit::label("The desktop’s background (the lock screen has none)", &["bg-what"]));
        row.append(&about);
        g.root.append(&row);
        let dark = gtk::Box::builder().spacing(8).css_classes(["bg-dark"]).build();
        dark.append(&Icon::new("moon", 14));
        dark.append(&gtk::Label::new(Some("WadSpaces is dark only.")));
        g.root.append(&dark);
        page.add(&g.root);

        let choose = Group::new("Choose a Background", "");
        choose.list.set_visible(false);
        choose.root.append(&kit::label("PICTURES", &["bg-h4"]));
        let grid = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .homogeneous(true)
            .column_spacing(10)
            .row_spacing(10)
            .min_children_per_line(4)
            .max_children_per_line(4)
            .build();
        grid.set_widget_name("background");
        let mut thumbs = vec![];
        for p in PICTURES.iter() {
            let b = gtk::Button::builder().css_classes(["bg-thumb"]).tooltip_text(p.name).build();
            b.set_widget_name(&format!("wallpaper-{}", p.id));
            let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
            let o = gtk::Overlay::new();
            o.set_child(Some(&picture(p.id, 132, 82, &["bg-wp"])));
            let tick = gtk::Box::builder()
                .css_classes(["bg-tick"])
                .halign(gtk::Align::End)
                .valign(gtk::Align::Start)
                .visible(p.id == current)
                .build();
            let t = Icon::new("check", 13);
            t.set_hexpand(true);
            t.set_halign(gtk::Align::Center);
            t.set_valign(gtk::Align::Center);
            tick.append(&t);
            o.add_overlay(&tick);
            col.append(&o);
            col.append(&gtk::Label::builder().label(p.name).xalign(0.0).css_classes(["bg-tname"]).build());
            b.set_child(Some(&col));
            if p.id == current {
                b.add_css_class("on");
            }
            let (w, id) = (self.me.clone(), p.id);
            b.connect_clicked(move |_| {
                if let Some(me) = w.upgrade() {
                    me.pick(id);
                }
            });
            grid.insert(&b, -1);
            thumbs.push((p.id, b));
        }
        choose.root.append(&grid);
        page.add(&choose.root);
        *self.ui.borrow_mut() = Some(Ui { shot: pic, name, thumbs });
        page.root.upcast()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_picture_s_name() {
        assert_eq!(super::name_of("duo"), "Duo");
        assert_eq!(super::name_of("aurora"), "Northern Lights");
        assert_eq!(super::name_of("gone"), "Duo");
    }
}
