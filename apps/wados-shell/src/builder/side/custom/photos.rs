//! Pictures to pick from, for a background or an icon (the spike's
//! `PhotoPicker.svelte`): Empty (a background's), your photos (the pictures
//! in ~/Pictures, newest first), WadSpaces' own art, named; and **Upload
//! your own photo**: a picture from this computer, copied into ~/Pictures
//! shrunk to fit, under a name nothing there has, then picked.
//!
//! A photo is kept as `~/Pictures/<name>` (wad-model's and wadd's way: wadd
//! finds it in the home of whoever runs it).

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::{gdk_pixbuf, gio, glib, prelude::*};
use wad_model::Wallpaper;
use wad_model::wallpapers::PICTURES;

/// An upload's longest side, at most (a 4K screen's).
pub const MAX_SIDE: i32 = 3840;

/// ~/Pictures.
pub fn dir() -> PathBuf {
    glib::home_dir().join("Pictures")
}

/// One of your photos.
#[derive(Debug, Clone, PartialEq)]
pub struct Photo {
    /// Its file's name ("night-lake.jpg").
    pub file: String,
    /// What its tile says (its name without the extension).
    pub name: String,
}

impl Photo {
    pub fn wallpaper(&self) -> Wallpaper {
        Wallpaper::image(format!("~/Pictures/{}", self.file))
    }
}

/// A picture's key, for tiles and `wadsctl custom`: `empty`, an art's id
/// (`retro`), `photo:<file>` for one in ~/Pictures, or what it is.
pub fn key_of(w: &Wallpaper) -> String {
    match w {
        Wallpaper::Empty => "empty".into(),
        Wallpaper::Color { value } => format!("color:{value}"),
        Wallpaper::Image { value } => match wad_model::wallpapers::picture_of(w) {
            Some(p) => p.id.to_string(),
            None => match value.strip_prefix("~/Pictures/") {
                Some(f) => format!("photo:{f}"),
                None => value.clone(),
            },
        },
    }
}

/// The pictures in `dir`: image files only (not hidden ones), newest first.
pub fn scan(dir: &Path) -> Vec<Photo> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut found: Vec<(std::time::SystemTime, Photo)> = rd
        .flatten()
        .filter_map(|e| {
            let file = e.file_name().into_string().ok()?;
            if file.starts_with('.') {
                return None;
            }
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            let (mime, _) = gio::content_type_guess(Some(&file), None::<&[u8]>);
            let mime = gio::content_type_get_mime_type(&mime).unwrap_or_default();
            if !mime.starts_with("image/") {
                return None;
            }
            let name = match file.rsplit_once('.') {
                Some((s, _)) if !s.is_empty() => s.to_string(),
                _ => file.clone(),
            };
            Some((meta.modified().unwrap_or(std::time::UNIX_EPOCH), Photo { file, name }))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.file.cmp(&b.1.file)));
    found.into_iter().map(|(_, p)| p).collect()
}

/// Your photos, kept current (~/Pictures watched), for every picker.
pub struct Photos {
    list: RefCell<Vec<Photo>>,
    watchers: RefCell<Vec<Box<dyn Fn()>>>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    queued: Cell<bool>,
    me: Weak<Photos>,
}

impl Photos {
    pub fn new() -> Rc<Self> {
        let me = Rc::new_cyclic(|me| Self {
            list: RefCell::new(scan(&dir())),
            watchers: RefCell::default(),
            monitor: RefCell::default(),
            queued: Cell::new(false),
            me: me.clone(),
        });
        // GLib watches a folder that isn't there yet for when it is.
        match gio::File::for_path(dir()).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
            Ok(m) => {
                let w = me.me.clone();
                m.connect_changed(move |_, _, _, _| {
                    if let Some(p) = w.upgrade() {
                        p.queue_rescan();
                    }
                });
                *me.monitor.borrow_mut() = Some(m);
            }
            Err(e) => tracing::warn!("can't watch ~/Pictures: {e}"),
        }
        me
    }

    pub fn list(&self) -> Vec<Photo> {
        self.list.borrow().clone()
    }

    pub fn watch(&self, f: impl Fn() + 'static) {
        self.watchers.borrow_mut().push(Box::new(f));
    }

    fn queue_rescan(&self) {
        if self.queued.replace(true) {
            return;
        }
        let w = self.me.clone();
        glib::timeout_add_local_once(Duration::from_millis(250), move || {
            if let Some(p) = w.upgrade() {
                p.queued.set(false);
                p.rescan();
            }
        });
    }

    /// ~/Pictures read again (watchers told if it changed).
    pub fn rescan(&self) {
        let now = scan(&dir());
        if *self.list.borrow() == now {
            return;
        }
        *self.list.borrow_mut() = now;
        for f in self.watchers.borrow().iter() {
            f();
        }
    }
}

/// `src` copied into `pictures`, shrunk to fit `max` on its long side (its
/// camera's turn applied): a JPEG, or a PNG if it has see-through parts,
/// under a name nothing there has. Its file's name.
pub fn shrink_into(src: &Path, pictures: &Path, max: i32) -> Result<String, String> {
    let shown = src.file_name().and_then(|n| n.to_str()).unwrap_or("It").to_string();
    let pb = gdk_pixbuf::Pixbuf::from_file(src).map_err(|e| format!("{shown} isn't a picture this can read ({e})"))?;
    let pb = pb.apply_embedded_orientation().unwrap_or(pb);
    let (w, h) = (pb.width(), pb.height());
    let k = (f64::from(max) / f64::from(w.max(h))).min(1.0);
    let pb = if k < 1.0 {
        let (nw, nh) = (((f64::from(w) * k).round() as i32).max(1), ((f64::from(h) * k).round() as i32).max(1));
        pb.scale_simple(nw, nh, gdk_pixbuf::InterpType::Bilinear).ok_or("Couldn't shrink it")?
    } else {
        pb
    };
    std::fs::create_dir_all(pictures).map_err(|e| format!("Couldn't make ~/Pictures: {e}"))?;
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.trim().trim_start_matches('.').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "photo".into());
    // Opaque (a camera's, or a drawing with nothing see-through): a JPEG, as the spike's.
    let see_through = pb.has_alpha() && !opaque(&pb);
    let pb = if pb.has_alpha() && !see_through { without_alpha(&pb).ok_or("Couldn't convert it")? } else { pb };
    let (ext, kind) = if see_through { ("png", "png") } else { ("jpg", "jpeg") };
    let name = free_name(pictures, &stem, ext);
    // Written aside (hidden), then put in place whole: what watches
    // ~/Pictures never sees half a picture.
    let part = pictures.join(format!(".{name}.part"));
    let opts: &[(&str, &str)] = if kind == "jpeg" { &[("quality", "88")] } else { &[] };
    let saved = pb.savev(&part, kind, opts).map_err(|e| format!("Couldn't save it: {e}"));
    let saved = saved.and_then(|()| {
        std::fs::rename(&part, pictures.join(&name)).map_err(|e| format!("Couldn't put it in ~/Pictures: {e}"))
    });
    if saved.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    saved.map(|()| name)
}

/// Every pixel of it solid.
fn opaque(pb: &gdk_pixbuf::Pixbuf) -> bool {
    let (w, h, stride, n) =
        (pb.width() as usize, pb.height() as usize, pb.rowstride() as usize, pb.n_channels() as usize);
    if n != 4 {
        return true;
    }
    let bytes = pb.read_pixel_bytes();
    (0..h).all(|y| bytes[y * stride..y * stride + w * 4].as_chunks::<4>().0.iter().all(|px| px[3] == 255))
}

/// The same picture with no alpha channel (for a JPEG).
fn without_alpha(pb: &gdk_pixbuf::Pixbuf) -> Option<gdk_pixbuf::Pixbuf> {
    let (w, h) = (pb.width(), pb.height());
    let out = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, w, h)?;
    out.fill(0x000000ff);
    pb.composite(&out, 0, 0, w, h, 0.0, 0.0, 1.0, 1.0, gdk_pixbuf::InterpType::Nearest, 255);
    Some(out)
}

/// `<stem>.<ext>` in `dir`, or `<stem> (2).<ext>`… if that's taken (the desktop's way).
pub fn free_name(dir: &Path, stem: &str, ext: &str) -> String {
    let taken = |n: &str| dir.join(n).symlink_metadata().is_ok() || dir.join(format!(".{n}.part")).exists();
    std::iter::once(format!("{stem}.{ext}"))
        .chain((2..).map(|n| format!("{stem} ({n}).{ext}")))
        .find(|n| !taken(n))
        .unwrap_or_else(|| format!("{stem}.{ext}"))
}

// ------------------------------------------------------------ the picker

struct Tile {
    key: String,
    label: String,
    wallpaper: Wallpaper,
    button: gtk::Button,
}

/// A grid of pictures to pick from, with Upload your own photo over it.
pub struct Picker {
    root: gtk::Box,
    upload: gtk::Button,
    upload_label: gtk::Label,
    grid: gtk::Grid,
    tiles: RefCell<Vec<Tile>>,
    /// Offer Empty (no background) first.
    empty: bool,
    selected: RefCell<Option<Wallpaper>>,
    editable: Cell<bool>,
    busy: Cell<bool>,
    photos: Rc<Photos>,
    on_pick: Box<dyn Fn(Wallpaper)>,
    on_fail: Box<dyn Fn(&str)>,
    me: Weak<Picker>,
}

impl Picker {
    pub fn new(
        photos: &Rc<Photos>,
        empty: bool,
        on_pick: impl Fn(Wallpaper) + 'static,
        on_fail: impl Fn(&str) + 'static,
    ) -> Rc<Self> {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["cz-picker"]).build();
        let upload = gtk::Button::builder().css_classes(["cz-upload"]).halign(gtk::Align::Start).build();
        let row = gtk::Box::builder().spacing(7).build();
        row.append(&wad_gtk::Icon::new("upload", 14));
        let upload_label = gtk::Label::new(Some("Upload your own photo"));
        row.append(&upload_label);
        upload.set_child(Some(&row));
        root.append(&upload);
        let grid = gtk::Grid::builder().column_homogeneous(true).column_spacing(6).row_spacing(6).build();
        root.append(&grid);
        let me = Rc::new_cyclic(|me| Self {
            root,
            upload,
            upload_label,
            grid,
            tiles: RefCell::default(),
            empty,
            selected: RefCell::default(),
            editable: Cell::new(true),
            busy: Cell::new(false),
            photos: photos.clone(),
            on_pick: Box::new(on_pick),
            on_fail: Box::new(on_fail),
            me: me.clone(),
        });
        let w = me.me.clone();
        me.upload.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.choose_file();
            }
        });
        let w = me.me.clone();
        photos.watch(move || {
            if let Some(p) = w.upgrade() {
                p.refill();
            }
        });
        me.refill();
        me
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// What's lit (the one it is now) and whether it can change.
    pub fn set(&self, selected: Option<&Wallpaper>, editable: bool) {
        *self.selected.borrow_mut() = selected.cloned();
        self.editable.set(editable);
        self.restyle();
    }

    /// The tiles: Empty, your photos, WadSpaces' pictures.
    fn items(&self) -> Vec<(String, String, Wallpaper)> {
        let mut out = vec![];
        if self.empty {
            out.push(("empty".to_string(), "Empty".to_string(), Wallpaper::Empty));
        }
        for p in self.photos.list() {
            let wp = p.wallpaper();
            out.push((key_of(&wp), p.name, wp));
        }
        for p in PICTURES {
            out.push((p.id.to_string(), p.name.to_string(), p.wallpaper()));
        }
        out
    }

    /// Tiles made for what's new, and laid out again only when the list changed.
    fn refill(&self) {
        let items = self.items();
        let same = {
            let t = self.tiles.borrow();
            t.len() == items.len() && t.iter().zip(&items).all(|(a, (k, _, _))| a.key == *k)
        };
        if same {
            return;
        }
        let mut old: Vec<Tile> = self.tiles.borrow_mut().drain(..).collect();
        for t in &old {
            self.grid.remove(&t.button);
        }
        let mut tiles = vec![];
        for (i, (key, label, wp)) in items.into_iter().enumerate() {
            let tile = match old.iter().position(|t| t.key == key) {
                Some(at) => old.swap_remove(at),
                None => self.tile(key, label, wp),
            };
            self.grid.attach(&tile.button, (i % 3) as i32, (i / 3) as i32, 1, 1);
            tiles.push(tile);
        }
        *self.tiles.borrow_mut() = tiles;
        self.restyle();
    }

    fn tile(&self, key: String, label: String, wallpaper: Wallpaper) -> Tile {
        let button = gtk::Button::builder().css_classes(["cz-bg"]).build();
        // A photo's name can be longer than its tile.
        if key.starts_with("photo:") {
            button.set_tooltip_text(Some(&label));
        }
        let over = gtk::Overlay::new();
        let picture = wad_gtk::Picture::new(&wallpaper, 9.0);
        over.set_child(Some(&picture));
        // The ring (lit when it's the one) over the picture, then its name.
        let ring = gtk::Box::builder().css_classes(["cz-ring"]).can_target(false).build();
        over.add_overlay(&ring);
        let name = gtk::Label::builder()
            .label(&label)
            .css_classes(["cz-bg-name"])
            .halign(gtk::Align::Fill)
            .valign(gtk::Align::End)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .can_target(false)
            .build();
        over.add_overlay(&name);
        button.set_child(Some(&over));
        if wallpaper == Wallpaper::Empty {
            button.add_css_class("empty");
        }
        let (w, wp) = (self.me.clone(), wallpaper.clone());
        button.connect_clicked(move |_| {
            if let Some(p) = w.upgrade()
                && p.editable.get()
            {
                (p.on_pick)(wp.clone());
            }
        });
        Tile { key, label, wallpaper, button }
    }

    fn restyle(&self) {
        let selected = self.selected.borrow();
        let editable = self.editable.get();
        for t in self.tiles.borrow().iter() {
            let on = selected.as_ref().is_some_and(|s| *s == t.wallpaper);
            if on {
                t.button.add_css_class("on");
            } else {
                t.button.remove_css_class("on");
            }
            t.button.set_sensitive(editable);
        }
        self.upload.set_sensitive(editable && !self.busy.get());
    }

    /// The one lit, by its key.
    pub fn lit(&self) -> Option<String> {
        let selected = self.selected.borrow();
        let s = selected.as_ref()?;
        Some(self.tiles.borrow().iter().find(|t| t.wallpaper == *s).map(|t| t.key.clone()).unwrap_or_else(|| key_of(s)))
    }

    /// Every tile's key, in order.
    pub fn keys(&self) -> Vec<String> {
        self.tiles.borrow().iter().map(|t| t.key.clone()).collect()
    }

    /// A tile, by its key (or its name: "Retro").
    pub fn find(&self, id: &str) -> Option<gtk::Button> {
        let tiles = self.tiles.borrow();
        tiles
            .iter()
            .find(|t| t.key == id)
            .or_else(|| tiles.iter().find(|t| t.label.eq_ignore_ascii_case(id)))
            .map(|t| t.button.clone())
    }

    pub fn upload_button(&self) -> gtk::Button {
        self.upload.clone()
    }

    /// A tile clicked, as the pointer would.
    pub fn pick(&self, id: &str) -> Result<(), String> {
        let b = self.find(id).ok_or_else(|| format!("no picture {id}"))?;
        if !b.is_sensitive() {
            return Err(format!("{id} can't be picked here"));
        }
        b.emit_clicked();
        Ok(())
    }

    // ------------------------------------------------------------ upload

    fn choose_file(&self) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Pictures"));
        filter.add_mime_type("image/*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title("Upload your own photo")
            .accept_label("Upload")
            .modal(true)
            .filters(&filters)
            .default_filter(&filter)
            .build();
        let window = self.root.root().and_downcast::<gtk::Window>();
        let w = self.me.clone();
        dialog.open(window.as_ref(), gio::Cancellable::NONE, move |r| {
            let Some(p) = w.upgrade() else { return };
            match r.map(|f| f.path()) {
                Ok(Some(path)) => p.upload(path),
                Ok(None) => (p.on_fail)("That file isn't on this computer"),
                // Dismissed.
                Err(_) => {}
            }
        });
    }

    /// `path` into ~/Pictures (shrunk to fit), then picked: what the file
    /// dialog's answer does, and `wadsctl custom upload`.
    pub fn upload(&self, path: PathBuf) {
        if self.busy.replace(true) {
            return;
        }
        self.upload_label.set_label("Uploading…");
        self.restyle();
        let w = self.me.clone();
        glib::spawn_future_local(async move {
            let to = dir();
            let r = gio::spawn_blocking(move || shrink_into(&path, &to, MAX_SIDE))
                .await
                .unwrap_or_else(|_| Err("Couldn't upload it".into()));
            let Some(p) = w.upgrade() else { return };
            p.busy.set(false);
            p.upload_label.set_label("Upload your own photo");
            p.restyle();
            match r {
                Ok(file) => {
                    p.photos.rescan();
                    (p.on_pick)(Wallpaper::image(format!("~/Pictures/{file}")));
                }
                Err(e) => (p.on_fail)(&e),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_name_art_photos_and_empty() {
        assert_eq!(key_of(&Wallpaper::Empty), "empty");
        assert_eq!(key_of(&wad_model::wallpapers::art_of("retro")), "retro");
        assert_eq!(key_of(&Wallpaper::image("~/Pictures/night-lake.jpg")), "photo:night-lake.jpg");
        assert_eq!(key_of(&Wallpaper::image("/srv/x.png")), "/srv/x.png");
    }

    #[test]
    fn only_pictures_newest_first_and_an_upload_shrunk_beside_them() {
        let home = std::env::temp_dir().join(format!("wados-photos-{}", std::process::id()));
        let pics = home.join("Pictures");
        std::fs::create_dir_all(&pics).expect("a folder");
        std::fs::write(pics.join("notes.txt"), "not a picture").expect("a file");
        std::fs::write(pics.join(".hidden.png"), "").expect("a file");
        // A picture 400 x 100, see-through: it stays a PNG.
        let pb = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 400, 100).expect("a pixbuf");
        pb.fill(0x7fd3ff80);
        let src = home.join("wide.png");
        pb.savev(&src, "png", &[]).expect("saved");
        let first = shrink_into(&src, &pics, 200).expect("uploaded");
        assert_eq!(first, "wide.png");
        let again = shrink_into(&src, &pics, 200).expect("uploaded");
        assert_eq!(again, "wide (2).png");
        let (_, w, h) = gdk_pixbuf::Pixbuf::file_info(pics.join(&first)).expect("a picture");
        assert_eq!((w, h), (200, 50));
        // Opaque (even with an alpha channel): a JPEG.
        let pb = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 100, 300).expect("a pixbuf");
        pb.fill(0xc6ff1fff);
        let src = home.join("tall.png");
        pb.savev(&src, "png", &[]).expect("saved");
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(shrink_into(&src, &pics, 150).expect("uploaded"), "tall.jpg");
        let (_, w, h) = gdk_pixbuf::Pixbuf::file_info(pics.join("tall.jpg")).expect("a picture");
        assert_eq!((w, h), (50, 150));
        let names: Vec<String> = scan(&pics).into_iter().map(|p| p.file).collect();
        assert_eq!(names.first().map(String::as_str), Some("tall.jpg"));
        assert_eq!(names.len(), 3);
        assert!(!names.iter().any(|n| n.ends_with(".txt") || n.starts_with('.')));
        assert!(shrink_into(&pics.join("notes.txt"), &pics, 150).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }
}
