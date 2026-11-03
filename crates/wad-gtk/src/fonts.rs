//! The site's fonts: Inter (text) and Space Grotesk (headings and the
//! brand), as WadSpaces bundles them (assets/fonts; the image installs
//! them system-wide). Added to Pango when the system hasn't got them.

use gtk::{pango, prelude::*};

/// The folders the fonts may be in: next to the program's assets, then the
/// repo's (a laptop's build).
fn dirs() -> Vec<std::path::PathBuf> {
    let mut d = vec![std::path::PathBuf::from("/usr/share/wados/assets/fonts")];
    d.push(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts"));
    d
}

/// Makes sure Inter and Space Grotesk can be drawn.
pub fn install() {
    let map = pangocairo_map();
    let have: Vec<String> = map.list_families().iter().map(|f| f.name().to_string()).collect();
    for (family, file) in [("Inter", "Inter-Variable.ttf"), ("Space Grotesk", "SpaceGrotesk-Variable.ttf")] {
        if have.iter().any(|h| h == family || h == &format!("{family} Variable")) {
            continue;
        }
        let Some(path) = dirs().into_iter().map(|d| d.join(file)).find(|p| p.exists()) else {
            tracing::warn!("no {family} font here: text falls back to the system's");
            continue;
        };
        match map.add_font_file(&path) {
            Ok(()) => tracing::debug!("{family}: from {}", path.display()),
            Err(e) => tracing::warn!("{family}: {e}"),
        }
    }
}

fn pangocairo_map() -> pango::FontMap {
    gtk::Widget::default_direction();
    // GTK's own font map: what every label draws with.
    let label = gtk::Label::new(None);
    label.pango_context().font_map().expect("a font map")
}
