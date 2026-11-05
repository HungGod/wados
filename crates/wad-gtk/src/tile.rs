//! A wadspace's icon (`shell/common/WadspaceIcon.svelte`): a tile of glass,
//! so a wadspace stands out from apps and files, with its picture set in it,
//! cut out as its look's icon says. A new one (no picture yet) is the glass
//! alone. Badges: running (a dot), streaming (LIVE and how many watch), not
//! built here (a hammer), didn't open (an alert), and a ring while it's busy.
//!
//! The same tile is the desktop's icon, a tag folder's face, the launcher's
//! small cards and the Builder's stack headers, at different sizes.

use gtk::prelude::*;
use wad_model::Wadspace;

use crate::Icon;
use crate::picture::Picture;

/// A wadspace's stream, as its badge says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Streaming {
    /// Its Stream job is still going.
    Starting,
    /// Live: how many watch.
    Live(u32),
}

impl Streaming {
    /// The tile's badge (`WadspaceIcon.svelte`'s): `LIVE 2`, or `…` while it starts.
    pub fn badge(self) -> String {
        match self {
            Streaming::Starting => "…".into(),
            Streaming::Live(n) => format!("LIVE {n}"),
        }
    }
}

/// How a wadspace is on this machine, as its badges say.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TileState {
    pub running: bool,
    /// Streaming: live (how many watch) or starting.
    pub streaming: Option<Streaming>,
    /// Not built here, or out of date.
    pub unbuilt: bool,
    /// The last open didn't work.
    pub failed: bool,
    /// Busy (building, starting): how far, 0–1.
    pub busy: Option<f64>,
}

/// A file icon's size, which a tile of size 1 matches.
pub const SIDE: i32 = 56;

pub struct WadspaceTile {
    pub root: gtk::Box,
    face: gtk::Overlay,
    glass: gtk::Box,
    picture: Picture,
    dot: gtk::Box,
    live: gtk::Label,
    unbuilt: gtk::Box,
    failed: gtk::Box,
    busy: gtk::ProgressBar,
    name: gtk::Label,
    side: i32,
}

impl WadspaceTile {
    /// A tile `scale` times a file icon's size, with its name under it or not.
    pub fn new(scale: f32, show_name: bool) -> Self {
        let side = (SIDE as f32 * scale).round() as i32;
        let root =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).css_classes(["ws-tile"]).build();
        let face = gtk::Overlay::builder().halign(gtk::Align::Center).build();
        face.set_size_request(side, side);
        let glass = gtk::Box::builder().css_classes(["ws-glass"]).build();
        glass.set_size_request(side, side);
        face.set_child(Some(&glass));
        // Its picture, set in the glass (9% in from each side).
        let inset = (side as f32 * 0.09).round() as i32;
        let picture = Picture::new(&wad_model::Wallpaper::Empty, side as f32 * 0.2);
        picture.set_margin_top(inset);
        picture.set_margin_bottom(inset);
        picture.set_margin_start(inset);
        picture.set_margin_end(inset);
        picture.set_can_target(false);
        face.add_overlay(&picture);
        let busy = gtk::ProgressBar::builder().css_classes(["ws-busy"]).valign(gtk::Align::End).visible(false).build();
        busy.set_margin_start(inset * 2);
        busy.set_margin_end(inset * 2);
        busy.set_margin_bottom(inset * 2);
        face.add_overlay(&busy);
        let dot = gtk::Box::builder()
            .css_classes(["ws-dot"])
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .visible(false)
            .build();
        face.add_overlay(&dot);
        let live = gtk::Label::builder()
            .css_classes(["ws-live"])
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .visible(false)
            .build();
        face.add_overlay(&live);
        let badge = |icon: &'static str, class: &str| {
            let b = gtk::Box::builder()
                .css_classes(["ws-badge", class])
                .halign(gtk::Align::End)
                .valign(gtk::Align::End)
                .visible(false)
                .build();
            b.append(&Icon::new(icon, (side as f32 * 0.17).max(9.0) as i32));
            face.add_overlay(&b);
            b
        };
        let unbuilt = badge("hammer", "ws-unbuilt");
        let failed = badge("circle-alert", "ws-failed");
        root.append(&face);
        let name = gtk::Label::builder()
            .css_classes(["ws-name"])
            .justify(gtk::Justification::Center)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .lines(2)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(12)
            .visible(show_name)
            .build();
        root.append(&name);
        Self { root, face, glass, picture, dot, live, unbuilt, failed, busy, name, side }
    }

    /// Shows `ws` as it is.
    pub fn set(&self, ws: &Wadspace, state: &TileState) {
        let picture = wad_model::look::icon_picture_of(ws);
        let pictured = wad_model::wallpapers::has_picture(picture);
        self.picture.set_wallpaper(picture);
        self.picture.set_crop(Some(&wad_model::look::icon_of(ws)));
        self.picture.set_visible(pictured);
        if pictured {
            self.glass.remove_css_class("ws-bare");
        } else {
            self.glass.add_css_class("ws-bare");
        }
        self.name.set_label(&ws.name);
        self.dot.set_visible(state.running || state.streaming.is_some());
        if state.streaming.is_some() {
            self.dot.add_css_class("ws-streaming");
        } else {
            self.dot.remove_css_class("ws-streaming");
        }
        match state.streaming {
            Some(st) => {
                self.live.set_label(&st.badge());
                self.live.set_visible(true);
            }
            None => self.live.set_visible(false),
        }
        self.unbuilt.set_visible(state.unbuilt && !state.failed);
        self.failed.set_visible(state.failed);
        match state.busy {
            Some(p) => {
                self.busy.set_fraction(p.clamp(0.0, 1.0));
                self.busy.set_visible(true);
            }
            None => self.busy.set_visible(false),
        }
    }

    /// Lit up (a drop target under a drag, the one the keys are on).
    pub fn set_glow(&self, on: bool) {
        if on {
            self.face.add_css_class("ws-glow");
        } else {
            self.face.remove_css_class("ws-glow");
        }
    }

    pub fn side(&self) -> i32 {
        self.side
    }
}

/// The CSS for tiles, site cards and initials (add it with the theme's).
pub const CSS: &str = "
.ws-glass {
  border-radius: 26%;
  background-image: linear-gradient(160deg, rgb(255 255 255 / 0.22), rgb(255 255 255 / 0.07) 55%, rgb(255 255 255 / 0.12));
  box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.38), inset 0 1.5px 0 rgb(255 255 255 / 0.55), 0 8px 20px -4px rgb(0 0 0 / 0.55);
}
.ws-glass.ws-bare { background-image: linear-gradient(160deg, rgb(255 255 255 / 0.14), rgb(255 255 255 / 0.04)); }
.ws-glow > .ws-glass { box-shadow: 0 0 0 2px var(--accent), 0 0 18px 2px var(--glow-haze); }
.ws-dot {
  min-width: 10px; min-height: 10px; margin: -3px -3px 0 0;
  border-radius: 999px; background: var(--accent); box-shadow: 0 0 0 2px var(--bg);
}
.ws-dot.ws-streaming { background: var(--accent-2); }
.ws-live {
  margin: -6px -10px 0 0; padding: 0 5px; border-radius: 999px;
  background: var(--accent-2); color: var(--bg);
  font-family: 'Space Grotesk', sans-serif; font-weight: 700; font-size: 9px;
}
.ws-badge {
  margin: 0 -4px -4px 0; padding: 2px; border-radius: 999px;
  background: var(--surface-3); box-shadow: 0 0 0 1.5px var(--bg);
}
.ws-unbuilt { color: var(--unbuilt); }
.ws-failed { color: var(--danger); }
.ws-busy progress { background: var(--accent); min-height: 3px; border-radius: 3px; }
.ws-busy trough { background: rgb(0 0 0 / 0.4); min-height: 3px; border-radius: 3px; }
.ws-name { font-size: 12.5px; color: var(--fg); text-shadow: 0 1px 3px rgb(0 0 0 / 0.7); }
.site-card { background: #fbf9ff; border-radius: 22%; box-shadow: inset 0 0 0 1px rgb(0 0 0 / 0.08); }
.web-initial { background: #2a1d47; border-radius: 22%; }
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stream_badge_says_live_and_how_many_or_that_it_starts() {
        assert_eq!(Streaming::Live(2).badge(), "LIVE 2");
        assert_eq!(Streaming::Live(0).badge(), "LIVE 0");
        assert_eq!(Streaming::Starting.badge(), "…");
    }
}
