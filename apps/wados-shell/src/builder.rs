//! Phase 0's Builder: an ordinary window with pieces to drag onto the
//! desktop, to prove a drag goes from a window to the desktop's layer
//! surface. The real Builder (stacks and a sidebar, `apps/builder/*`) grows
//! from here in phase 3.

use gtk::{gdk, glib, prelude::*};

pub fn open(app: &gtk::Application) -> gtk::ApplicationWindow {
    let w = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Builder")
        .default_width(520)
        .default_height(420)
        .build();
    let col = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    col.append(
        &gtk::Label::builder().label("Drag a piece onto the desktop").css_classes(["display"]).xalign(0.0).build(),
    );
    for piece in ["app:vscode", "web:github", "key:Work"] {
        let l = gtk::Label::builder().label(piece).css_classes(["piece"]).xalign(0.0).build();
        let drag = gtk::DragSource::new();
        drag.set_actions(gdk::DragAction::COPY);
        let value = piece.to_string();
        drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&value.to_value())));
        let name = piece.to_string();
        drag.connect_drag_begin(move |_, _| tracing::info!("drag {name} begins"));
        l.add_controller(drag);
        col.append(&l);
    }
    w.set_child(Some(&col));
    let _ = glib::Type::STRING;
    w.present();
    w
}
