//! Files' viewers (the spike's `apps/imageview` and `apps/textview`): a
//! picture shown whole on black, or a text file read-only in a monospace
//! page; anything else says what it is. What a double-click opens when no
//! app has opened that kind of file before (`wados-files --view <path>`).

use std::path::Path;

use gtk::{gio, glib, prelude::*};
use wad_gtk::kit;

pub const CSS: &str = r#"
window.viewer-window { background-color: var(--bg-2); }
.iv { background-color: #050309; padding: 12px; }
.tv text { background-color: var(--bg-2); color: var(--fg); }
.tv { font-family: monospace; font-size: 13px; padding: 20px 24px; }
.tv-none { color: var(--muted); }
.tv-none-name { color: var(--fg); font-weight: 600; font-size: 14px; }
.tv-faint { color: var(--faint); }
"#;

/// How it shows a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shows {
    Picture,
    Text(String),
    /// Not a picture or text: its type and size.
    Other {
        mime: String,
        size: u64,
    },
}

/// The most of a text file it reads.
const MAX_TEXT: u64 = 4 << 20;

fn mime_of(path: &Path) -> String {
    let f = gio::File::for_path(path);
    let ct = f
        .query_info("standard::content-type", gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
        .ok()
        .and_then(|i| i.content_type())
        .map(|c| c.to_string())
        .unwrap_or_default();
    gio::content_type_get_mime_type(&ct).map(|m| m.to_string()).unwrap_or(ct)
}

pub fn what(path: &Path) -> Shows {
    let mime = mime_of(path);
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if mime.starts_with("image/") {
        return Shows::Picture;
    }
    let texty = mime.starts_with("text/")
        || [
            "application/json",
            "application/x-shellscript",
            "application/toml",
            "application/xml",
            "application/x-yaml",
        ]
        .contains(&mime.as_str());
    if texty
        && size <= MAX_TEXT
        && let Ok(t) = std::fs::read_to_string(path)
    {
        return Shows::Text(t);
    }
    Shows::Other { mime, size }
}

/// A viewer's window for `path`.
pub fn open(app: &gtk::Application, path: &Path) -> gtk::ApplicationWindow {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "File".into());
    let shows = what(path);
    let picture = shows == Shows::Picture;
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(&name)
        .default_width(if picture { 900 } else { 760 })
        .default_height(if picture { 640 } else { 620 })
        .css_classes(["viewer-window"])
        .build();
    kit::title_bar(&window, if picture { "image" } else { "file-text" }, &name);
    let body: gtk::Widget = match shows {
        Shows::Picture => {
            let p = gtk::Picture::for_filename(path);
            p.set_content_fit(gtk::ContentFit::Contain);
            p.set_can_shrink(true);
            p.add_css_class("iv");
            p.set_widget_name("viewer-picture");
            p.upcast()
        }
        Shows::Text(t) => {
            let tv = gtk::TextView::builder()
                .editable(false)
                .cursor_visible(false)
                .monospace(true)
                .wrap_mode(gtk::WrapMode::WordChar)
                .css_classes(["tv"])
                .build();
            tv.set_widget_name("viewer-text");
            tv.buffer().set_text(&t);
            let s = gtk::ScrolledWindow::builder().child(&tv).hscrollbar_policy(gtk::PolicyType::Never).build();
            s.upcast()
        }
        Shows::Other { mime, size } => {
            let b = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(6)
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .css_classes(["tv-none"])
                .build();
            let l = |t: &str, c: &str| {
                let l = kit::label(t, &[c]);
                l.set_xalign(0.5);
                l.set_justify(gtk::Justification::Center);
                l
            };
            b.append(&l(&name, "tv-none-name"));
            let kind = if mime.is_empty() { "A file".to_string() } else { mime };
            b.append(&l(&format!("{kind} · {}", crate::drive::bytes(size)), "tv-none"));
            b.append(&l("Files shows pictures and text. Open it in a wadspace to work on it.", "tv-faint"));
            b.upcast()
        }
    };
    window.set_child(Some(&body));
    let keys = gtk::EventControllerKey::new();
    let w = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            if let Some(w) = w.upgrade() {
                w.close();
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    window
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_text_and_the_rest() {
        let d = std::env::temp_dir().join(format!("wados-viewer-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("notes.txt"), "remember the milk").unwrap();
        std::fs::write(d.join("blob.bin"), [0u8, 159, 146, 150, 0, 1]).unwrap();
        assert_eq!(what(&d.join("notes.txt")), Shows::Text("remember the milk".into()));
        assert!(matches!(what(&d.join("blob.bin")), Shows::Other { size: 6, .. }));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
