//! The desktop's icons, one per spot (the spike's `.icon` buttons): a
//! wadspace's glass tile (wad_gtk::WadspaceTile, as `WadspaceIcon.svelte`), a
//! tag's folder (`TagFolder.svelte`: a frosted square in the tag's colour
//! holding its first four wadspaces' pictures), a file (`FileIcon.svelte`: a
//! folder, a picture's thumbnail, or a page with its glyph and extension),
//! and the desktop's own tiles, Machines and the Trash. Each has its name
//! under it.

use std::cell::RefCell;
use std::collections::HashSet;

use gtk::{gdk, prelude::*};
use wad_gtk::{Icon, Picture, TileState, WadspaceTile};
use wad_model::{Tag, Wadspace, Wallpaper};

use super::files::{self, Entry, Kind};
use super::grid::{ICON_H, ICON_W};

/// A wadspace's tile and a folder's face: 1.3 times a file's 56 px.
pub const SPACE_SCALE: f32 = 1.3;
/// In an open folder.
pub const INSIDE_SCALE: f32 = 1.15;
const FILE_SIDE: i32 = 46;
const TILE_SIDE: i32 = 60;
/// A name's width: an icon's box less its padding.
const NAME_W: i32 = ICON_W as i32 - 8;

/// What an icon shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Space {
        ws: Box<Wadspace>,
        state: TileState,
    },
    Folder {
        tag: Tag,
        spaces: Vec<Wadspace>,
    },
    File(Entry),
    Machines,
    /// WadSpaces Live's: opens the installer.
    Install,
    Trash {
        full: bool,
    },
}

impl Item {
    pub fn name(&self) -> String {
        match self {
            Item::Space { ws, .. } => ws.name.clone(),
            Item::Folder { tag, .. } => tag.name.clone(),
            Item::File(e) => e.display.clone(),
            Item::Machines => "Machines".into(),
            Item::Install => "Install".into(),
            Item::Trash { .. } => "Trash".into(),
        }
    }

    /// Same kind of icon (so its widgets can be kept, only updated).
    fn same_kind(&self, o: &Item) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(o)
    }
}

/// One icon in its spot.
pub struct Cell {
    pub root: gtk::Box,
    face: gtk::Box,
    name: gtk::Label,
    tile: RefCell<Option<WadspaceTile>>,
    item: RefCell<Item>,
}

impl Cell {
    pub fn new(key: &str, item: &Item) -> Self {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .css_classes(["desk-icon"])
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .build();
        root.set_widget_name(&format!("desk:{key}"));
        root.set_size_request(ICON_W as i32, ICON_H as i32);
        let face = gtk::Box::builder().halign(gtk::Align::Center).build();
        let name = gtk::Label::builder()
            .css_classes(["desk-name"])
            .justify(gtk::Justification::Center)
            .halign(gtk::Align::Center)
            .build();
        root.append(&face);
        root.append(&name);
        let me = Self { root, face, name, tile: RefCell::new(None), item: RefCell::new(item.clone()) };
        me.build(item);
        me
    }

    /// Shows `item` (only what changed is redone).
    pub fn set(&self, item: &Item) {
        if *self.item.borrow() == *item {
            return;
        }
        let same = self.item.borrow().same_kind(item);
        self.item.replace(item.clone());
        if let (Item::Space { ws, state }, true) = (item, same) {
            let tile = self.tile.borrow();
            if let Some(t) = tile.as_ref() {
                t.set(ws, state);
                self.name.set_label(&ws.name);
                return;
            }
        }
        self.build(item);
    }

    pub fn item(&self) -> Item {
        self.item.borrow().clone()
    }

    fn build(&self, item: &Item) {
        while let Some(c) = self.face.first_child() {
            self.face.remove(&c);
        }
        *self.tile.borrow_mut() = None;
        for c in ["file", "tile"] {
            self.root.remove_css_class(c);
        }
        let (lines, wrap) = match item {
            Item::Space { .. } | Item::Folder { .. } => (2, true),
            _ => (1, false),
        };
        self.name.set_wrap(wrap);
        self.name.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        self.name.set_lines(lines);
        self.name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        fit_name(&self.name);
        self.name.set_label(&item.name());
        self.name.set_attributes(Some(&no_hyphens()));
        match item {
            Item::Space { ws, state } => {
                let t = WadspaceTile::new(SPACE_SCALE, false);
                t.set(ws, state);
                self.face.append(&t.root);
                *self.tile.borrow_mut() = Some(t);
            }
            Item::Folder { tag, spaces } => self.face.append(&folder_face(tag, spaces, SPACE_SCALE)),
            Item::File(e) => {
                self.root.add_css_class("file");
                self.face.append(&file_face(e));
            }
            Item::Machines => {
                self.root.add_css_class("tile");
                self.face.append(&sys_tile("machines", "server", 0.56, false));
            }
            Item::Install => {
                self.root.add_css_class("tile");
                self.face.append(&sys_tile("machines", "hard-drive", 0.56, false));
            }
            Item::Trash { full } => {
                self.root.add_css_class("tile");
                self.face.append(&sys_tile("trash", "trash", 0.6, *full));
            }
        }
    }

    pub fn set_flag(&self, class: &str, on: bool) {
        if on {
            self.root.add_css_class(class);
        } else {
            self.root.remove_css_class(class);
        }
    }

    /// The name under it hidden (while it's being typed over).
    pub fn set_naming(&self, on: bool) {
        self.name.set_opacity(if on { 0.0 } else { 1.0 });
    }
}

// ------------------------------------------------------------ a tag's colour

thread_local! {
    /// The tag colours given a class so far, and the stylesheet that gives them.
    static TAG_COLORS: RefCell<(HashSet<String>, Option<gtk::CssProvider>)> = RefCell::default();
}

/// A tag's colour as hex digits, if it's one ("#7fd3ff"); else General's.
fn tag_hex(color: &str) -> String {
    let hex: String = color.trim_start_matches('#').chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let ok = color.starts_with('#') && matches!(hex.len(), 3 | 4 | 6 | 8) && hex.len() + 1 == color.len();
    if ok { hex.to_lowercase() } else { wad_model::tags::GENERAL_COLOR.trim_start_matches('#').to_string() }
}

/// The class that sets `--c` to a tag's colour (one per colour, made as needed).
pub fn tag_class(color: &str) -> String {
    let hex = tag_hex(color);
    TAG_COLORS.with(|t| {
        let mut t = t.borrow_mut();
        if t.0.insert(hex.clone()) {
            let css: String = t.0.iter().map(|h| format!(".desk-tc-{h} {{ --c: #{h}; }}\n")).collect();
            let p = t.1.get_or_insert_with(|| {
                let p = gtk::CssProvider::new();
                if let Some(d) = gdk::Display::default() {
                    gtk::style_context_add_provider_for_display(&d, &p, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
                }
                p
            });
            p.load_from_string(&css);
        }
    });
    format!("desk-tc-{hex}")
}

/// A tag's folder: a frosted square in its colour with its first four
/// wadspaces' pictures (as their icons cut them out), like a phone's app folder.
pub fn folder_face(tag: &Tag, spaces: &[Wadspace], scale: f32) -> gtk::Widget {
    let side = (wad_gtk::tile::SIDE as f32 * scale).round() as i32;
    let face = gtk::Box::builder().css_classes(["desk-folder-face", &tag_class(&tag.color)]).build();
    face.set_size_request(side, side);
    let mini = (side as f32 * 0.32).round() as i32;
    let gap = (side as f32 * 0.08).round() as i32;
    let grid = gtk::Grid::builder()
        .row_spacing(gap)
        .column_spacing(gap)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    for i in 0..4 {
        let w: gtk::Widget = match spaces.get(i) {
            Some(x) => {
                let picture = wad_model::look::icon_picture_of(x);
                if wad_model::wallpapers::has_picture(picture) {
                    let p = Picture::icon(picture, &wad_model::look::icon_of(x), mini as f32 * 0.22);
                    p.add_css_class("desk-mini");
                    p.upcast()
                } else {
                    gtk::Box::builder().css_classes(["desk-mini", "glass"]).build().upcast()
                }
            }
            None => gtk::Box::builder().css_classes(["desk-slot"]).build().upcast(),
        };
        w.set_size_request(mini, mini);
        w.set_can_target(false);
        grid.attach(&w, (i % 2) as i32, (i / 2) as i32, 1, 1);
    }
    face.append(&grid);
    face.upcast()
}

/// A file's icon (46 px): a folder, a picture's thumbnail, or a page.
fn file_face(e: &Entry) -> gtk::Widget {
    let side = FILE_SIDE;
    let holder = gtk::Box::builder().halign(gtk::Align::Center).valign(gtk::Align::Center).build();
    holder.set_size_request(side, side);
    match e.kind {
        Kind::Folder => {
            let img = match wad_gtk::assets::path("folder.svg")
                .and_then(|p| wad_gtk::picture::texture(&p, (side * 2) as u32))
            {
                Some(t) => gtk::Image::from_paintable(Some(&t)),
                None => gtk::Image::new(),
            };
            img.set_pixel_size(side);
            img.set_hexpand(true);
            img.add_css_class("desk-folder-file");
            holder.append(&img);
        }
        Kind::Image => {
            let value = e.picture().to_string_lossy().into_owned();
            let p = Picture::new(&Wallpaper::Image { value }, 6.0);
            p.add_css_class("desk-thumb");
            p.set_size_request((side as f32 * 0.88).round() as i32, (side as f32 * 0.72).round() as i32);
            p.set_halign(gtk::Align::Center);
            p.set_valign(gtk::Align::Center);
            p.set_hexpand(true);
            holder.append(&p);
        }
        Kind::Doc => {
            let page = gtk::Overlay::builder()
                .css_classes(["desk-doc"])
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .hexpand(true)
                .build();
            page.set_size_request((side as f32 * 0.74).round() as i32, (side as f32 * 0.92).round() as i32);
            let glyph = Icon::new(files::glyph(&e.content_type), (side as f32 * 0.38).round() as i32);
            glyph.set_halign(gtk::Align::Center);
            glyph.set_valign(gtk::Align::Center);
            let ext = files::ext_label(&e.name);
            if !ext.is_empty() {
                glyph.set_valign(gtk::Align::Start);
                glyph.set_margin_top(7);
            }
            page.set_child(Some(&glyph));
            if !ext.is_empty() {
                let l = gtk::Label::builder()
                    .label(&ext)
                    .css_classes(["desk-ext"])
                    .halign(gtk::Align::Center)
                    .valign(gtk::Align::End)
                    .build();
                page.add_overlay(&l);
            }
            holder.append(&page);
        }
    }
    holder.upcast()
}

/// The desktop's own tiles (60 px): Machines' blue, the Trash's violet (a
/// dot on it while there's something in it).
fn sys_tile(class: &str, icon: &'static str, k: f32, full: bool) -> gtk::Widget {
    let o = gtk::Overlay::builder().halign(gtk::Align::Center).build();
    let t = gtk::Box::builder().css_classes(["desk-sys", class]).build();
    t.set_size_request(TILE_SIDE, TILE_SIDE);
    let i = Icon::new(icon, (TILE_SIDE as f32 * k).round() as i32);
    i.set_halign(gtk::Align::Center);
    i.set_valign(gtk::Align::Center);
    i.set_hexpand(true);
    t.append(&i);
    o.set_child(Some(&t));
    if full {
        let dot =
            gtk::Box::builder().css_classes(["desk-full"]).halign(gtk::Align::End).valign(gtk::Align::Start).build();
        dot.set_size_request(10, 10);
        o.add_overlay(&dot);
    }
    o.upcast()
}

/// A wadspace in an open folder (smaller, its name under it).
pub fn inside(ws: &Wadspace, state: &TileState) -> gtk::Box {
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .css_classes(["desk-icon", "inside"])
        .build();
    root.set_widget_name(&format!("in:{}", ws.id));
    root.set_size_request(ICON_W as i32, 104);
    let t = WadspaceTile::new(INSIDE_SCALE, false);
    t.set(ws, state);
    root.append(&t.root);
    root.append(
        &gtk::Label::builder()
            .label(&ws.name)
            .css_classes(["desk-name"])
            .justify(gtk::Justification::Center)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .lines(2)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .attributes(&no_hyphens())
            .build(),
    );
    if let Some(name) = root.last_child().and_downcast::<gtk::Label>() {
        fit_name(&name);
    }
    root
}

/// A name as wide as an icon's box inside (the spike's 92 px): wrapped or
/// cut off there, never wider.
fn fit_name(l: &gtk::Label) {
    l.set_max_width_chars(1);
    l.set_size_request(NAME_W, -1);
}

/// A long name broken where it has to be, without a hyphen ("VanuaAcadem / y", as the spike's).
fn no_hyphens() -> gtk::pango::AttrList {
    let a = gtk::pango::AttrList::new();
    a.insert(gtk::pango::AttrInt::new_insert_hyphens(false));
    a
}

#[cfg(test)]
mod tests {
    #[test]
    fn tag_colours_are_only_colours() {
        assert_eq!(super::tag_hex("#7FD3FF"), "7fd3ff");
        assert_eq!(super::tag_hex("#fff"), "fff");
        // Nothing else gets into the stylesheet.
        assert_eq!(super::tag_hex("red; } * { color: red"), "9a93ad");
        assert_eq!(super::tag_hex("#12"), "9a93ad");
        assert_eq!(super::tag_hex("#7fd3ff;x"), "9a93ad");
    }
}
