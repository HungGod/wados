//! The background, on every monitor (BACKGROUND layer): a picture filling
//! it, the one Settings ▸ Background chose (`[background] picture` in the
//! settings: one of WadSpaces' pictures by its id). Duo (lime and pink,
//! mirrored) is the desktop's to start with.

use gtk::{gdk, prelude::*};
use gtk4_layer_shell::{KeyboardMode, Layer};

use crate::surface::{self, Anchor, Spec};

/// The picture's file for `id` (an unknown one is Duo).
pub fn file_of(id: &str) -> Option<std::path::PathBuf> {
    let id = if wad_model::wallpapers::PICTURES.iter().any(|p| p.id == id) { id } else { "duo" };
    crate::assets::path(&format!("wallpapers/art/{id}.svg"))
}

pub fn surface(app: &gtk::Application, monitor: &gdk::Monitor, picture: &str) -> gtk::ApplicationWindow {
    let w = surface::make(
        app,
        monitor,
        &Spec {
            namespace: "wados-wallpaper",
            layer: Layer::Background,
            anchor: Anchor::Fill,
            exclusive: -1,
            keyboard: KeyboardMode::None,
            height: 0,
        },
    );
    surface::stay(&w);
    let pic = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).can_shrink(true).build();
    let page = gtk::Fixed::new();
    page.put(&pic, 0.0, 0.0);
    w.set_child(Some(&page));
    fit(&w, monitor);
    set(&w, picture);
    surface::click_through(&w);
    w
}

fn page(w: &gtk::ApplicationWindow) -> Option<(gtk::Fixed, gtk::Picture)> {
    let page = w.child().and_downcast::<gtk::Fixed>()?;
    let pic = page.first_child().and_downcast::<gtk::Picture>()?;
    Some((page, pic))
}

/// The picture as big as `monitor`.
pub fn fit(w: &gtk::ApplicationWindow, monitor: &gdk::Monitor) {
    if let Some((_, pic)) = page(w) {
        let g = monitor.geometry();
        pic.set_size_request(g.width(), g.height());
    }
}

/// Shows picture `id` on a wallpaper surface.
pub fn set(w: &gtk::ApplicationWindow, id: &str) {
    if let (Some((_, pic)), Some(p)) = (page(w), file_of(id)) {
        pic.set_filename(Some(&p));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_picture_by_its_id() {
        assert!(super::file_of("silk").unwrap().ends_with("wallpapers/art/silk.svg"));
        assert!(super::file_of("nope").unwrap().ends_with("wallpapers/art/duo.svg"));
    }
}
