//! The background, on every monitor (BACKGROUND layer): a picture filling
//! it. Duo (lime and pink, mirrored) is the desktop's to start with.

use gtk::{gdk, prelude::*};
use gtk4_layer_shell::{KeyboardMode, Layer};

use crate::surface::{self, Anchor, Spec};

pub fn surface(app: &gtk::Application, monitor: &gdk::Monitor) -> gtk::ApplicationWindow {
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
    let pic = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).can_shrink(true).build();
    if let Some(p) = crate::assets::path("wallpapers/art/duo.svg") {
        pic.set_filename(Some(&p));
    }
    w.set_child(Some(&pic));
    surface::click_through(&w);
    w
}
