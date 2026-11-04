//! What one of an app's things looks like (`ThingIcon.svelte`): the app's
//! icon, a web app's site's, Claude Code's own, or a glyph (an extension, a
//! tool, Focus, a game). And a stack's desktop as it'll be when it opens
//! (`DeskPreview.svelte`, compact): its picture, its icons as they're laid
//! out (an app's add-ons a badge on it; "N more" past what fits), and its
//! panel (WadSpaces' mark, its apps, the time).

use gtk::prelude::*;
use wad_gtk::app_icon;
use wad_model::Wadspace;
use wad_model::stack::stack_apps;
use wad_model::variants::{AppVariant, VariantKind};

use super::cells::Cells;
use super::pieces::{self, DESK_CELL, DESK_ICON, DESK_TASK};

/// One of an app's things, `size` px square (it doesn't grow: a site's card centres its picture by expanding it).
pub fn thing_icon(v: &AppVariant, size: i32) -> gtk::Widget {
    let w = thing(v, size);
    w.set_hexpand(false);
    w.set_vexpand(false);
    w
}

fn thing(v: &AppVariant, size: i32) -> gtk::Widget {
    match v.kind {
        VariantKind::App => app_icon::app(v.app, size),
        VariantKind::Web => {
            let e = v.ext.as_ref();
            app_icon::site(
                e.map(|e| e.id.as_str()).unwrap_or_default(),
                Some(v.name.as_str()),
                e.and_then(|e| e.url.as_deref()),
                size,
            )
        }
        _ if v.ext.as_ref().is_some_and(|e| e.id == "anthropic.claude-code") => {
            picture("/icons/claude.png", size).unwrap_or_else(|| glyph("puzzle", size))
        }
        VariantKind::Extension => glyph("puzzle", size),
        VariantKind::Tool => glyph("square-terminal", size),
        VariantKind::Mode => glyph("eye", size),
        VariantKind::Game => glyph("gamepad-2", size),
        VariantKind::Named => glyph("layers", size),
        VariantKind::SignIn => glyph("key-round", size),
    }
}

/// A glyph on a rounded tile of its own.
pub fn glyph(name: &'static str, size: i32) -> gtk::Widget {
    let b = gtk::Box::builder()
        .css_classes(["thing-glyph"])
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .hexpand(false)
        .vexpand(false)
        .build();
    b.set_size_request(size, size);
    let i = wad_gtk::Icon::new(name, (size as f32 * 0.62).round() as i32);
    i.set_hexpand(true);
    i.set_halign(gtk::Align::Center);
    i.set_valign(gtk::Align::Center);
    b.append(&i);
    b.upcast()
}

/// One of WadSpaces' pictures (`/icons/…`, `/brand/…`), `size` px square.
fn picture(src: &str, size: i32) -> Option<gtk::Widget> {
    let p = wad_gtk::picture::resolve(src)?;
    let t = wad_gtk::picture::texture(&p, (size * 2) as u32)?;
    let img = gtk::Image::from_paintable(Some(&t));
    img.set_pixel_size(size);
    img.add_css_class("thing-pic");
    Some(img.upcast())
}

/// Its desktop, `width` px wide (half as tall).
pub fn desk(x: &Wadspace, width: i32) -> gtk::Widget {
    let height = (width as f64 / pieces::DESK_ASPECT).round() as i32;
    let o = gtk::Overlay::builder().css_classes(["desk-preview"]).overflow(gtk::Overflow::Hidden).build();
    o.set_size_request(width, height);
    o.set_widget_name(&format!("desk:{}", x.id));
    let wp = wad_gtk::Picture::new(&x.look.wallpaper, 10.0);
    wp.set_size_request(width, height);
    wp.set_can_target(false);
    o.set_child(Some(&wp));

    // Its icons, down each column then across, as many as fit.
    let rows = pieces::desk_rows(width).max(1);
    let grid = Cells::new(DESK_CELL, rows as i32);
    grid.set_halign(gtk::Align::Start);
    grid.set_valign(gtk::Align::Start);
    grid.set_can_target(false);
    grid.set_margin_start(6);
    grid.set_margin_top(6);
    let icons = pieces::desk_icons(x);
    let (shown, more) = pieces::fit(icons.len(), width);
    for d in icons.iter().take(shown) {
        let face = match (&d.thing, d.glyph) {
            (Some(t), _) => thing_icon(t, DESK_ICON),
            (None, Some(g)) => {
                let i = wad_gtk::Icon::new(g, (DESK_ICON as f32 * 0.7) as i32);
                i.add_css_class("desk-glyph");
                i.upcast()
            }
            (None, None) => glyph("app-window", DESK_ICON),
        };
        grid.append(&cell(face, &d.name, d.badge.as_deref()));
    }
    if more > 0 {
        let plus = gtk::Box::builder()
            .css_classes(["desk-more"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .hexpand(false)
            .build();
        plus.set_size_request(DESK_ICON, DESK_ICON);
        let i = wad_gtk::Icon::new("plus", 15);
        i.set_hexpand(true);
        i.set_halign(gtk::Align::Center);
        i.set_valign(gtk::Align::Center);
        plus.append(&i);
        grid.append(&cell(plus.upcast(), &format!("{more} more"), None));
    }
    o.add_overlay(&grid);

    // Its panel.
    let task =
        gtk::Box::builder().spacing(6).css_classes(["desk-task"]).valign(gtk::Align::End).can_target(false).build();
    task.set_size_request(-1, DESK_TASK);
    if let Some(mark) = picture("/brand/wadspaces-icon-dark-transparent.svg", 14) {
        mark.set_valign(gtk::Align::Center);
        task.append(&mark);
    }
    task.append(&gtk::Box::builder().css_classes(["desk-tsep"]).valign(gtk::Align::Center).build());
    for app in stack_apps(x) {
        let i = app_icon::app(app, 13);
        i.set_valign(gtk::Align::Center);
        task.append(&i);
    }
    task.append(&gtk::Box::builder().hexpand(true).build());
    task.append(&gtk::Label::builder().label("9:41").css_classes(["desk-clock"]).build());
    o.add_overlay(&task);
    o.upcast()
}

/// An icon's cell: its face (with its badge) over its name.
fn cell(face: gtk::Widget, name: &str, badge: Option<&str>) -> gtk::Widget {
    let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).css_classes(["desk-icon"]).build();
    col.set_size_request(DESK_CELL.0, DESK_CELL.1);
    let o = gtk::Overlay::builder().halign(gtk::Align::Center).build();
    let holder = gtk::Box::builder().halign(gtk::Align::Center).valign(gtk::Align::Center).hexpand(false).build();
    holder.set_size_request(30, 28);
    face.set_halign(gtk::Align::Center);
    face.set_valign(gtk::Align::Center);
    face.set_hexpand(true);
    holder.append(&face);
    o.set_child(Some(&holder));
    if let Some(b) = badge {
        let l = gtk::Label::builder().label(b).css_classes(["desk-badge"]).build();
        if b == "focus" {
            l.add_css_class("word");
            l.set_halign(gtk::Align::Center);
            l.set_valign(gtk::Align::End);
        } else {
            l.set_halign(gtk::Align::End);
            l.set_valign(gtk::Align::Start);
        }
        o.add_overlay(&l);
    }
    col.append(&o);
    let l =
        gtk::Label::builder().label(name).css_classes(["desk-label"]).ellipsize(gtk::pango::EllipsizeMode::End).build();
    col.append(&l);
    col.upcast()
}
