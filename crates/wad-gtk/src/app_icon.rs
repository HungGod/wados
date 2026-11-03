//! An app's icon (`ui/AppIcon.svelte`): a catalog app's own; the command
//! line's terminal glyph; a web app's site icon on a white card (so a dark
//! logo reads on the dark desktop), or, with none, its initial on a violet
//! card in one of six tints.

use gtk::{gdk, prelude::*};
use wad_model::AppId;

use crate::Icon;

const TINTS: [&str; 6] = ["#c6ff1f", "#7fd3ff", "#ff3d81", "#ffb36b", "#b58cff", "#5cf2c4"];

fn image(path: &std::path::Path, size: i32, scale: i32) -> gtk::Image {
    let t: Option<gdk::Texture> = crate::picture::texture(path, (size * scale) as u32);
    let img = match t {
        Some(t) => gtk::Image::from_paintable(Some(&t)),
        None => gtk::Image::new(),
    };
    img.set_pixel_size(size);
    img
}

/// A catalog app's icon, `size` px square.
pub fn app(id: AppId, size: i32) -> gtk::Widget {
    if id == AppId::Terminal {
        let i = Icon::new("square-terminal", (size as f32 * 0.92) as i32);
        i.set_size_request(size, size);
        return i.upcast();
    }
    let path = crate::picture::resolve(wad_model::catalog::app_icon(id));
    match path {
        Some(p) => image(&p, size, 2).upcast(),
        None => Icon::new("app-window", size).upcast(),
    }
}

/// A web app's icon: its site's (one of WadSpaces' own), or its initial.
pub fn site(id: &str, name: Option<&str>, url: Option<&str>, size: i32) -> gtk::Widget {
    let local =
        wad_model::catalog::site_icon(id, url).filter(|s| s.starts_with('/')).and_then(|s| crate::picture::resolve(&s));
    if let Some(p) = local {
        let card = gtk::Box::builder()
            .css_classes(["site-card"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        card.set_size_request(size, size);
        let img = image(&p, (size as f32 * 0.66) as i32, 2);
        img.set_hexpand(true);
        img.set_vexpand(true);
        card.append(&img);
        return card.upcast();
    }
    let mut h: u32 = 0;
    for c in id.chars() {
        h = h.wrapping_mul(31).wrapping_add(c as u32);
    }
    let tint = TINTS[h as usize % TINTS.len()];
    let shown = name.unwrap_or(id);
    let shown = shown.strip_prefix("Google ").unwrap_or(shown);
    let letter = shown.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".into());
    let l = gtk::Label::builder().label(letter).css_classes(["web-initial", "display"]).build();
    l.set_size_request(size, size);
    // Its tint, on its own (a provider per letter would pile up): inline markup.
    l.set_markup(&format!(
        "<span foreground=\"{tint}\" font_size=\"{}pt\">{}</span>",
        (size as f32 * 0.42).round() as i32,
        glib_escape(&l.label())
    ));
    l.upcast()
}

fn glib_escape(s: &str) -> String {
    gtk::glib::markup_escape_text(s).to_string()
}
