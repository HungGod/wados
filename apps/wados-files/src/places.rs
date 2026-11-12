//! Files' own places (the spike's `FolderView.svelte` and `Item.svelte`, in
//! its `local` and `trash` modes, and `WadspaceListing.svelte`):
//!
//! - **A folder here**, as a grid or a list: click, Ctrl and Shift pick,
//!   double-click (or Enter) opens: a folder in Files, a file with the app
//!   that last opened its kind (Open with's memory) or else Files' viewer.
//!   Its menu (`fileMenu`): Open, Open with ▸, Cut, Copy, Paste, Rename,
//!   Move to Trash (with Undo); the background's: New folder, Paste, Select
//!   all, View as list or grid, Show hidden files. Delete, F2, Ctrl+A/C/X/V,
//!   Ctrl+H, Ctrl+Shift+N.
//! - **The Trash**: its files (freedesktop.org's) and this machine's
//!   wadspaces in it, newest first; Restore (where each came from), Delete
//!   permanently (asked first: a wadspace goes for good, `DELETE
//!   /v1/wadspaces/{id}`), Empty Trash and Restore all.
//! - **One of this machine's wadspaces**, read-only: what's in it.

use std::path::{Path, PathBuf};

use gtk::{gdk, gio, glib, prelude::*};
use wad_gtk::form::Dialog;
use wad_gtk::kit::{self, Tone, Variant};
use wad_gtk::{Icon, TileState, WadspaceTile};
use wad_model::open_with::{self, Opening, Row as OwRow};
use wad_model::{AppId, ProjectKind, SessionDisplay, catalog};

use crate::local::{self, Item};
use crate::menu::{self, Row, item as mi};
use crate::state::Loc;
use crate::ui::{Files, View, item, name_label, now_ms};
use crate::viewer;

/// The Trash's keys: a file's and a wadspace's.
const FILE: &str = "file:";
const WS: &str = "ws:";

impl Files {
    // ------------------------------------------------------------ what's here

    /// The open folder's things, filtered.
    pub(crate) fn items(&self) -> Vec<Item> {
        let q = self.q();
        match &*self.here.borrow() {
            Some(Ok(v)) => {
                v.iter().filter(|i| q.is_empty() || i.display.to_lowercase().contains(&q)).cloned().collect()
            }
            _ => vec![],
        }
    }

    fn dir(&self) -> Option<PathBuf> {
        match &*self.loc.borrow() {
            Loc::Local(p) => Some(p.clone()),
            _ => None,
        }
    }

    fn picked_items(&self) -> Vec<Item> {
        let sel = self.selected.borrow().clone();
        self.items().into_iter().filter(|i| sel.contains(&i.name)).collect()
    }

    /// A folder here: its things; none if it can't be read.
    pub(crate) fn local_view(&self, p: &Path) -> Option<usize> {
        let err = match &*self.here.borrow() {
            Some(Err(e)) => Some(e.clone()),
            _ => None,
        };
        if let Some(why) = err {
            let gone = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(12)
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .vexpand(true)
                .css_classes(["files-empty"])
                .build();
            gone.append(&Icon::new("folder-open", 30));
            let l = kit::label(if p.exists() { &why } else { "This folder isn’t here any more." }, &[]);
            l.set_xalign(0.5);
            gone.append(&l);
            let b = kit::button("Go home", Variant::Secondary, Some("house"), true);
            b.set_halign(gtk::Align::Center);
            let w = self.me.clone();
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.go(Loc::Local(m.home.clone()));
                }
            });
            gone.append(&b);
            self.body.append(&gone);
            return None;
        }
        let items = self.items();
        let empty = if self.q().is_empty() {
            "This folder is empty. Drop things here, or make a new folder.".to_string()
        } else {
            format!("Nothing here matches “{}”.", self.filter.text())
        };
        let order: Vec<String> = items.iter().map(|i| i.name.clone()).collect();
        let cut = self.cut.borrow().clone().unwrap_or_default();
        let shown: Vec<Shown> = items
            .iter()
            .map(|i| Shown {
                key: i.name.clone(),
                name: i.display.clone(),
                glyph: local::glyph(&i.content_type, i.folder),
                thumb: thumb(i),
                size: size_of(i),
                cols: [
                    size_of(i),
                    if i.modified > 0 { kit::ago(now_ms(), i.modified * 1000) } else { String::new() },
                    local::kind_label(&i.name, &i.content_type, i.folder),
                ],
                cut: cut.contains(&i.path),
                tile: None,
            })
            .collect();
        self.draw_items(&shown, &order, false, &["Name", "Size", "Modified", "Kind"], &empty);
        Some(items.len())
    }

    /// The Trash: its strip, then what's in it.
    pub(crate) fn trash_view(&self) -> usize {
        let files = self.trash.borrow().clone();
        let spaces = self.state.borrow().trashed();
        let q = self.q();
        let fits = |n: &str| q.is_empty() || n.to_lowercase().contains(&q);
        let mut shown: Vec<(i64, Shown)> = vec![];
        for t in files.iter().filter(|t| fits(&t.display)) {
            shown.push((
                t.at,
                Shown {
                    key: format!("{FILE}{}", t.name),
                    name: t.display.clone(),
                    glyph: local::glyph(&t.content_type, t.folder),
                    thumb: None,
                    size: if t.folder { String::new() } else { crate::drive::bytes(t.size) },
                    cols: [
                        t.from.parent().map(local::pretty).unwrap_or_default(),
                        if t.at > 0 { kit::ago(now_ms(), t.at * 1000) } else { String::new() },
                        if t.folder { "—".into() } else { crate::drive::bytes(t.size) },
                    ],
                    cut: false,
                    tile: None,
                },
            ));
        }
        for x in spaces.iter().filter(|x| fits(&x.name)) {
            let at = x.deleted_at.unwrap_or(0) / 1000;
            shown.push((
                at,
                Shown {
                    key: format!("{WS}{}", x.id),
                    name: x.name.clone(),
                    glyph: "monitor",
                    thumb: None,
                    size: "Wadspace".into(),
                    cols: [
                        "Wadspaces".into(),
                        if at > 0 { kit::ago(now_ms(), at * 1000) } else { String::new() },
                        "—".into(),
                    ],
                    cut: false,
                    tile: Some(x.clone()),
                },
            ));
        }
        shown.sort_by_key(|s| std::cmp::Reverse(s.0));
        let shown: Vec<Shown> = shown.into_iter().map(|(_, s)| s).collect();
        let total = files.len() + spaces.len();
        let strip = gtk::Box::builder().spacing(10).css_classes(["files-strip"]).build();
        strip.append(&Icon::new("trash", 15));
        strip.append(&kit::label(
            if total > 0 {
                "Restore what you need; emptying the Trash deletes the rest for good, wadspaces too."
            } else {
                "Things you delete wait here until you empty it."
            },
            &[],
        ));
        strip.last_child().unwrap().set_hexpand(true);
        let restore = kit::button("Restore", Variant::Ghost, Some("undo-2"), true);
        restore.set_sensitive(!self.selected.borrow().is_empty());
        let w = self.me.clone();
        restore.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                let keys = m.selected.borrow().clone();
                m.restore(&keys);
            }
        });
        strip.append(&restore);
        let empty = kit::button("Empty Trash", Variant::Danger, Some("trash"), true);
        empty.set_widget_name("empty-trash");
        empty.set_sensitive(total > 0);
        let w = self.me.clone();
        empty.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.ask_empty_trash();
            }
        });
        strip.append(&empty);
        // The strip sits above the scrolled things: in the body, first.
        self.body.append(&strip);
        let empty_text = if q.is_empty() {
            "The Trash is empty.".to_string()
        } else {
            format!("Nothing in the Trash matches “{}”.", self.filter.text())
        };
        let order: Vec<String> = shown.iter().map(|s| s.key.clone()).collect();
        self.draw_items(&shown, &order, true, &["Name", "From", "Trashed", "Size"], &empty_text);
        shown.len()
    }

    /// The grid or the list of `shown`, or what it says when there's nothing.
    fn draw_items(&self, shown: &[Shown], order: &[String], trash: bool, heads: &[&str; 4], empty: &str) {
        let area = gtk::Box::builder().orientation(gtk::Orientation::Vertical).vexpand(true).build();
        // A click on the background clears the pick; its right-click is its menu.
        let bg = gtk::GestureClick::builder().button(0).build();
        let w = self.me.clone();
        bg.connect_pressed(move |g, _, x, y| {
            let Some(m) = w.upgrade() else { return };
            if g.current_button() == gdk::BUTTON_SECONDARY {
                m.selected.borrow_mut().clear();
                m.queue_draw();
                if let Some(a) = g.widget() {
                    menu::show(&a, x, y, m.background_menu());
                }
            } else if !m.selected.borrow().is_empty() {
                m.selected.borrow_mut().clear();
                m.queue_draw();
            }
        });
        area.add_controller(bg);
        if shown.is_empty() {
            let e = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(10)
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .vexpand(true)
                .css_classes(["files-empty"])
                .build();
            e.append(&Icon::new(if trash { "trash" } else { "folder-open" }, 30));
            let l = kit::label(empty, &[]);
            l.set_xalign(0.5);
            e.append(&l);
            area.append(&e);
            self.body.append(&area);
            return;
        }
        let renaming = self.renaming.borrow().clone();
        match self.view.get() {
            View::Grid => {
                let grid = gtk::FlowBox::builder()
                    .selection_mode(gtk::SelectionMode::None)
                    .min_children_per_line(2)
                    .max_children_per_line(16)
                    .column_spacing(4)
                    .row_spacing(4)
                    .homogeneous(true)
                    .valign(gtk::Align::Start)
                    .build();
                for s in shown {
                    let sel = self.is_sel(&s.key);
                    let it = match &s.tile {
                        Some(x) => {
                            let b = gtk::Box::builder()
                                .orientation(gtk::Orientation::Vertical)
                                .spacing(6)
                                .css_classes(["item"])
                                .build();
                            b.set_size_request(112, 112);
                            if sel {
                                b.add_css_class("selected");
                            }
                            let tile = WadspaceTile::new(0.5, false);
                            tile.set(x, &TileState::default());
                            tile.root.set_halign(gtk::Align::Center);
                            b.append(&tile.root);
                            b.append(&name_label(&s.name, sel));
                            b
                        }
                        None => {
                            let b = item(s.glyph, &s.name, "", sel);
                            if let Some(t) = &s.thumb {
                                let pic = gtk::Picture::for_filename(t);
                                pic.set_content_fit(gtk::ContentFit::Contain);
                                pic.set_size_request(60, 60);
                                if let Some(first) = b.first_child() {
                                    b.remove(&first);
                                }
                                b.prepend(&pic);
                            }
                            b
                        }
                    };
                    if s.cut {
                        it.add_css_class("cut");
                    }
                    if renaming.as_deref() == Some(s.key.as_str()) {
                        self.rename_in(&it, &s.key, &s.name);
                    }
                    it.set_widget_name(&format!("item-{}", s.key));
                    self.wire_item(&it, &s.key, order, trash);
                    grid.append(&it);
                }
                area.append(&grid);
            }
            View::List => {
                let head = gtk::Box::builder().spacing(12).css_classes(["head"]).build();
                for (i, h) in heads.iter().enumerate() {
                    let l = gtk::Label::builder().label(*h).xalign(0.0).build();
                    size_col(&l, i, trash);
                    head.append(&l);
                }
                area.append(&head);
                for s in shown {
                    let sel = self.is_sel(&s.key);
                    let r = gtk::Box::builder().spacing(12).css_classes(["row"]).build();
                    if sel {
                        r.add_css_class("selected");
                    }
                    if s.cut {
                        r.add_css_class("cut");
                    }
                    let main = gtk::Box::builder().spacing(10).build();
                    let ic = Icon::new(s.glyph, 16);
                    ic.add_css_class("item-glyph");
                    main.append(&ic);
                    main.append(
                        &gtk::Label::builder().label(&s.name).ellipsize(gtk::pango::EllipsizeMode::End).build(),
                    );
                    size_col(&main, 0, trash);
                    r.append(&main);
                    for (i, c) in s.cols.iter().enumerate() {
                        let l = gtk::Label::builder()
                            .label(c)
                            .xalign(0.0)
                            .ellipsize(gtk::pango::EllipsizeMode::End)
                            .css_classes(["muted"])
                            .build();
                        size_col(&l, i + 1, trash);
                        r.append(&l);
                    }
                    if renaming.as_deref() == Some(s.key.as_str()) {
                        self.rename_in(&main, &s.key, &s.name);
                    }
                    r.set_widget_name(&format!("item-{}", s.key));
                    self.wire_item(&r, &s.key, order, trash);
                    area.append(&r);
                }
            }
        }
        self.body.append(&area);
    }

    /// An item's clicks: pick, open, its menu.
    fn wire_item(&self, it: &gtk::Box, key: &str, order: &[String], trash: bool) {
        let click = gtk::GestureClick::new();
        let (w, k, order) = (self.me.clone(), key.to_string(), order.to_vec());
        click.connect_pressed(move |g, n, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            let Some(m) = w.upgrade() else { return };
            if m.renaming.borrow().as_deref() == Some(k.as_str()) {
                return;
            }
            if m.presses(&k, n) >= 2 {
                if !trash {
                    m.pick_only(&k);
                    m.open_key(&k);
                }
            } else {
                m.click_pick(&k, g.current_event_state(), &order);
            }
        });
        it.add_controller(click);
        let right = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let (w, k) = (self.me.clone(), key.to_string());
        right.connect_pressed(move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            let Some(m) = w.upgrade() else { return };
            if !m.is_sel(&k) {
                m.pick_only(&k);
            }
            if let Some(on) = g.widget() {
                menu::show(&on, x, y, m.menu_for(&k));
            }
        });
        it.add_controller(right);
    }

    /// Its name as a text to type, in place of its label.
    fn rename_in(&self, it: &gtk::Box, key: &str, name: &str) {
        let mut c = it.first_child();
        while let Some(x) = c {
            c = x.next_sibling();
            if x.downcast_ref::<gtk::Label>().is_some() {
                let e = gtk::Entry::builder().text(name).css_classes(["item-rename"]).build();
                e.set_widget_name("rename");
                e.set_width_chars(12);
                it.insert_child_after(&e, Some(&x));
                it.remove(&x);
                let (w, k) = (self.me.clone(), key.to_string());
                e.connect_activate(move |e| {
                    if let Some(m) = w.upgrade() {
                        m.end_rename(&k, Some(e.text().to_string()));
                    }
                });
                let keys = gtk::EventControllerKey::new();
                let (w, k) = (self.me.clone(), key.to_string());
                keys.connect_key_pressed(move |_, key, _, _| {
                    if key == gdk::Key::Escape {
                        if let Some(m) = w.upgrade() {
                            m.end_rename(&k, None);
                        }
                        return glib::Propagation::Stop;
                    }
                    glib::Propagation::Proceed
                });
                e.add_controller(keys);
                let focus = gtk::EventControllerFocus::new();
                let (w, k, ew) = (self.me.clone(), key.to_string(), e.downgrade());
                focus.connect_leave(move |_| {
                    if let (Some(m), Some(e)) = (w.upgrade(), ew.upgrade())
                        && m.renaming.borrow().as_deref() == Some(k.as_str())
                    {
                        m.end_rename(&k, Some(e.text().to_string()));
                    }
                });
                e.add_controller(focus);
                // The name before its extension picked, as the spike does.
                let stop = match name.rfind('.') {
                    Some(i) if i > 0 && !self.items().iter().any(|x| x.name == key && x.folder) => {
                        name[..i].chars().count() as i32
                    }
                    _ => -1,
                };
                glib::idle_add_local_once(move || {
                    e.grab_focus();
                    e.select_region(0, stop);
                });
                return;
            }
        }
    }

    pub(crate) fn start_rename(&self, key: &str) {
        self.pick_only(key);
        *self.renaming.borrow_mut() = Some(key.to_string());
        self.queue_draw();
    }

    fn end_rename(&self, key: &str, to: Option<String>) {
        if self.renaming.borrow_mut().take().is_none() {
            return;
        }
        if let (Some(to), Some(dir)) = (to, self.dir()) {
            match local::rename(&dir.join(key), &to) {
                Ok(p) => {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    *self.selected.borrow_mut() = vec![name];
                }
                Err(e) => self.say(&format!("Couldn’t rename it: {e}")),
            }
        }
        self.reload();
        self.queue_draw();
    }

    // ------------------------------------------------------------ opening

    /// Opened (double-click, Enter): a folder in Files, a file with the app
    /// that last opened its kind (in a space that has it), else in a viewer.
    pub(crate) fn open_key(&self, key: &str) {
        let Some(i) = self.items().into_iter().find(|i| i.name == key) else { return };
        if i.folder {
            self.go(Loc::Local(i.path));
            return;
        }
        // An archive: what's in it, in a folder beside it.
        if local::is_archive(&i.name) {
            self.extract(&i.name);
            return;
        }
        let o = Opening::new(&i.name, Some(&i.content_type).filter(|c| !c.is_empty()).map(String::as_str), false);
        let live = self.state.borrow().live();
        match open_with::remembered_for(&o, &live, &wad_gtk::opened::load()) {
            Some((app, ws)) => self.open_with(std::slice::from_ref(&i), app, Some(ws)),
            None => viewer::open(&self.app, &i.path).present(),
        }
    }

    /// Open with ▸'s rows for `items`.
    pub(crate) fn open_with_rows(&self, items: &[Item]) -> Vec<Row> {
        let opening: Vec<Opening> = items
            .iter()
            .map(|i| {
                Opening::new(&i.name, Some(&i.content_type).filter(|c| !c.is_empty()).map(String::as_str), i.folder)
            })
            .collect();
        let last = opening.first().and_then(|o| wad_gtk::opened::load().get(&o.mime()));
        let live = self.state.borrow().live();
        open_with::rows(&opening, &live, last)
            .into_iter()
            .map(|r| match r {
                OwRow::Sep => Row::Sep,
                OwRow::Nothing => mi("open-with:none", "No app opens it", "", || {}).off(Some("")).row(),
                OwRow::App { app, ws, label, hint, checked } => {
                    let id = OwRow::App { app, ws: ws.clone(), label: String::new(), hint: None, checked }.id();
                    let (w, items) = (self.me.clone(), items.to_vec());
                    let icon = if ws.is_some() { "monitor-play" } else { "plus" };
                    let mut it = mi(&id, label, icon, move || {
                        if let Some(m) = w.upgrade() {
                            m.open_with(&items, app, ws.clone());
                        }
                    })
                    .checked(checked);
                    if let Some(h) = hint {
                        it = it.hint(h);
                    }
                    it.row()
                }
            })
            .collect()
    }

    /// `items` opened with `app` in `ws` (none: a new space for it), for now.
    pub(crate) fn open_with(&self, items: &[Item], app: AppId, ws: Option<String>) {
        let Some(first) = items.first() else { return };
        let o = Opening::new(
            &first.name,
            Some(&first.content_type).filter(|c| !c.is_empty()).map(String::as_str),
            first.folder,
        );
        wad_gtk::opened::remember(&o.mime(), app);
        let what = match items.len() {
            1 => first.display.clone(),
            n => format!("{} and {} more", first.display, n - 1),
        };
        let space = ws.as_deref().and_then(|w| self.state.borrow().wadspace(w).map(|x| x.name.clone()));
        let into = space.map_or_else(|| "a new space".to_string(), |s| s.to_string());
        self.say(&format!("Opening {what} with {} in {into}…", catalog::app_name(app)));
        let files: Vec<String> = items.iter().map(|i| i.path.to_string_lossy().into_owned()).collect();
        let w = self.me.clone();
        self.link.call(
            move |wadd| async move { wadd.open_files_with(app, ws.as_deref(), &files, None).await },
            move |r| {
                let Some(m) = w.upgrade() else { return };
                match r {
                    Ok(_) => m.note_hide(),
                    Err(e) => m.say(&format!("{what} didn’t open: {}", e.message)),
                }
            },
        );
    }

    /// A good open says nothing more (the note's for what went wrong).
    fn note_hide(&self) {
        if self.undo.borrow().is_empty() {
            self.hide_note();
        }
    }

    // ------------------------------------------------------------ menus

    /// The menu of what's at `key` (the pick, if it's in it).
    pub(crate) fn menu_for(&self, key: &str) -> Vec<Row> {
        match &*self.loc.borrow() {
            Loc::Trash => self.trash_menu(key),
            Loc::Local(_) => self.file_menu(key),
            _ => vec![],
        }
    }

    fn file_menu(&self, key: &str) -> Vec<Row> {
        let Some(i) = self.items().into_iter().find(|i| i.name == key) else { return vec![] };
        let picked = if self.is_sel(key) { self.picked_items() } else { vec![i.clone()] };
        let keys: Vec<String> = picked.iter().map(|p| p.name.clone()).collect();
        let w = self.me.clone();
        let on = move |f: Box<dyn Fn(&Files)>| {
            let w = w.clone();
            move || {
                if let Some(m) = w.upgrade() {
                    f(&m);
                }
            }
        };
        let (k, ks1, ks2, ks3, dir) = (key.to_string(), keys.clone(), keys.clone(), keys.clone(), i.path.clone());
        vec![
            mi("open", "Open", if i.folder { "folder-open" } else { "file" }, on(Box::new(move |m| m.open_key(&k))))
                .key("Enter")
                .row(),
            mi("open-with", "Open with", "app-window", || {}).sub(self.open_with_rows(&picked)).row(),
        ]
        .into_iter()
        .chain((!i.folder && local::is_archive(&i.name)).then(|| {
            let k = key.to_string();
            mi("extract", "Extract here", "file-archive", on(Box::new(move |m| m.extract(&k)))).row()
        }))
        .chain([
            Row::Sep,
            mi("cut", "Cut", "scissors", on(Box::new(move |m| m.copy(&ks1, true)))).key("Ctrl+X").row(),
            mi("copy", "Copy", "copy", on(Box::new(move |m| m.copy(&ks2, false)))).key("Ctrl+C").row(),
            mi("paste", "Paste", "clipboard-paste", on(Box::new(move |m| m.paste(dir.clone()))))
                .key("Ctrl+V")
                .off(if !i.folder {
                    Some("Paste into a folder")
                } else if !self.can_paste() {
                    Some("The clipboard is empty")
                } else {
                    None
                })
                .row(),
            {
                let k = key.to_string();
                mi("rename", "Rename", "pencil", on(Box::new(move |m| m.start_rename(&k))))
                    .key("F2")
                    .off((keys.len() > 1).then_some("One at a time"))
                    .row()
            },
            Row::Sep,
            mi("trash", "Move to Trash", "trash", on(Box::new(move |m| m.trash(&ks3)))).key("Delete").danger().row(),
        ])
        .collect()
    }

    fn trash_menu(&self, key: &str) -> Vec<Row> {
        let keys = if self.is_sel(key) { self.selected.borrow().clone() } else { vec![key.to_string()] };
        let name = self.trash_name(key);
        let from = key
            .strip_prefix(FILE)
            .and_then(|n| self.trash.borrow().iter().find(|t| t.name == n).map(|t| t.from.clone()))
            .and_then(|f| f.parent().map(local::pretty));
        let heading = if keys.len() > 1 { format!("{} items", keys.len()) } else { name };
        let (w1, w2, w3) = (self.me.clone(), self.me.clone(), self.me.clone());
        let (k1, k2) = (keys.clone(), keys.clone());
        let mut restore = mi("restore", "Restore", "undo-2", move || {
            if let Some(m) = w1.upgrade() {
                m.restore(&k1);
            }
        });
        if keys.len() == 1 {
            if let Some(f) = from {
                restore = restore.hint(format!("Back to {f}"));
            } else if key.starts_with(WS) {
                restore = restore.hint("Back on the desktop");
            }
        }
        vec![
            Row::Heading(heading),
            restore.row(),
            mi("delete-forever", "Delete permanently", "trash", move || {
                if let Some(m) = w2.upgrade() {
                    m.ask_delete(&k2);
                }
            })
            .key("Delete")
            .danger()
            .row(),
            Row::Sep,
            mi("empty-trash", "Empty Trash", "trash", move || {
                if let Some(m) = w3.upgrade() {
                    m.ask_empty_trash();
                }
            })
            .danger()
            .row(),
        ]
    }

    pub(crate) fn background_menu(&self) -> Vec<Row> {
        let loc = self.loc.borrow().clone();
        let w = self.me.clone();
        let on = move |f: Box<dyn Fn(&Files)>| {
            let w = w.clone();
            move || {
                if let Some(m) = w.upgrade() {
                    f(&m);
                }
            }
        };
        let has = match &loc {
            Loc::Trash => !self.trash.borrow().is_empty() || !self.state.borrow().trashed().is_empty(),
            _ => !self.items().is_empty(),
        };
        let select_all = mi("select-all", "Select all", "check-check", on(Box::new(|m| m.select_all())))
            .key("Ctrl+A")
            .off((!has).then_some("Nothing here"));
        let view = match self.view.get() {
            View::Grid => mi("view", "View as list", "list", on(Box::new(|m| m.set_view(View::List)))),
            View::List => mi("view", "View as grid", "layout-grid", on(Box::new(|m| m.set_view(View::Grid)))),
        };
        let hidden = if self.hidden.get() {
            mi("hidden", "Hide hidden files", "eye-off", on(Box::new(|m| m.toggle_hidden())))
        } else {
            mi("hidden", "Show hidden files", "eye", on(Box::new(|m| m.toggle_hidden())))
        }
        .key("Ctrl+H");
        match loc {
            Loc::Local(dir) => vec![
                mi("new-folder", "New folder", "folder-plus", on(Box::new(|m| m.new_folder()))).row(),
                mi("paste", "Paste", "clipboard-paste", on(Box::new(move |m| m.paste(dir.clone()))))
                    .key("Ctrl+V")
                    .off((!self.can_paste()).then_some("The clipboard is empty"))
                    .row(),
                Row::Sep,
                select_all.row(),
                view.row(),
                hidden.row(),
            ],
            Loc::Trash => vec![
                mi("restore-all", "Restore all", "undo-2", on(Box::new(|m| m.restore_all())))
                    .off((!has).then_some("It’s empty"))
                    .row(),
                mi("empty-trash", "Empty Trash", "trash", on(Box::new(|m| m.ask_empty_trash())))
                    .danger()
                    .off((!has).then_some("It’s empty"))
                    .row(),
                Row::Sep,
                select_all.row(),
                view.row(),
            ],
            _ => vec![select_all.row(), view.row()],
        }
    }

    /// A menu in words (`--dump --menu <name>`; `background` is the background's).
    pub fn menu_words(&self, name: &str) -> String {
        let rows = if name == "background" {
            self.background_menu()
        } else {
            let key = match &*self.loc.borrow() {
                Loc::Trash => self
                    .trash
                    .borrow()
                    .iter()
                    .find(|t| t.display == name)
                    .map(|t| format!("{FILE}{}", t.name))
                    .or_else(|| {
                        self.state.borrow().trashed().iter().find(|x| x.name == name).map(|x| format!("{WS}{}", x.id))
                    })
                    .unwrap_or_default(),
                _ => name.to_string(),
            };
            self.pick_only(&key);
            self.menu_for(&key)
        };
        menu::words(&rows).join(" | ")
    }

    /// Something done as a click would (`--dump --act "<what> [<arg>]"`, for tests):
    /// `new-folder`, `rename <old>=<new>`, `trash <name>`, `undo`, `restore <name>`,
    /// `delete <name>`, `empty`, `view list|grid`, `select <name>`.
    pub fn act(&self, a: &str) -> Result<(), String> {
        let (verb, arg) = a.split_once(' ').unwrap_or((a, ""));
        let key_of = |name: &str| -> String {
            self.trash
                .borrow()
                .iter()
                .find(|t| t.display == name)
                .map(|t| format!("{FILE}{}", t.name))
                .or_else(|| {
                    self.state.borrow().trashed().iter().find(|x| x.name == name).map(|x| format!("{WS}{}", x.id))
                })
                .unwrap_or_else(|| name.to_string())
        };
        match verb {
            "new-folder" => self.new_folder(),
            "rename" => {
                let (from, to) = arg.split_once('=').ok_or("rename <old>=<new>")?;
                *self.renaming.borrow_mut() = Some(from.to_string());
                self.end_rename(from, Some(to.to_string()));
            }
            "trash" => self.trash(&[arg.to_string()]),
            "undo" => self.undo_trash(),
            "restore" => self.restore(&[key_of(arg)]),
            "delete" => self.delete_forever(&[key_of(arg)]),
            "empty" => self.empty_trash(),
            "view" => self.set_view(if arg == "list" { View::List } else { View::Grid }),
            "select" => self.pick_only(&key_of(arg)),
            "hidden" => self.toggle_hidden(),
            _ => return Err(format!("no such act {verb:?}")),
        }
        self.reload();
        Ok(())
    }

    fn set_view(&self, v: View) {
        self.view.set(v);
        self.queue_draw();
    }

    /// Hidden things (dot-files, backups) shown, or not: the folder read again.
    pub(crate) fn toggle_hidden(&self) {
        self.hidden.set(!self.hidden.get());
        // What's picked and now hidden isn't picked.
        if !self.hidden.get() {
            self.selected.borrow_mut().retain(|k| !k.starts_with('.') && !k.ends_with('~'));
        }
        self.reload();
        self.queue_draw();
    }

    fn select_all(&self) {
        let keys: Vec<String> = match &*self.loc.borrow() {
            Loc::Trash => {
                let mut k: Vec<String> = self.trash.borrow().iter().map(|t| format!("{FILE}{}", t.name)).collect();
                k.extend(self.state.borrow().trashed().iter().map(|x| format!("{WS}{}", x.id)));
                k
            }
            Loc::Local(_) => self.items().into_iter().map(|i| i.name).collect(),
            _ => return,
        };
        *self.selected.borrow_mut() = keys;
        self.queue_draw();
    }

    /// A key in a folder here or the Trash: true if it did something.
    pub(crate) fn view_key(&self, key: gdk::Key, ctrl: bool, shift: bool) -> bool {
        let loc = self.loc.borrow().clone();
        let sel = self.selected.borrow().clone();
        match (&loc, key) {
            (_, gdk::Key::BackSpace) => self.go_up(),
            (_, gdk::Key::Escape) if !sel.is_empty() => {
                self.selected.borrow_mut().clear();
                self.queue_draw();
            }
            (Loc::Local(_) | Loc::Trash, gdk::Key::a | gdk::Key::A) if ctrl => self.select_all(),
            (Loc::Local(_), gdk::Key::Delete) if !sel.is_empty() => self.trash(&sel),
            (Loc::Local(_), gdk::Key::F2) if sel.len() == 1 => self.start_rename(&sel[0]),
            (Loc::Local(_), gdk::Key::Return | gdk::Key::KP_Enter) if sel.len() == 1 => self.open_key(&sel[0]),
            (Loc::Local(_), gdk::Key::c | gdk::Key::C) if ctrl && !sel.is_empty() => self.copy(&sel, false),
            (Loc::Local(_), gdk::Key::x | gdk::Key::X) if ctrl && !sel.is_empty() => self.copy(&sel, true),
            (Loc::Local(d), gdk::Key::v | gdk::Key::V) if ctrl => self.paste(d.clone()),
            (Loc::Local(_), gdk::Key::n | gdk::Key::N) if ctrl && shift => self.new_folder(),
            (Loc::Local(_), gdk::Key::h | gdk::Key::H) if ctrl => self.toggle_hidden(),
            (Loc::Trash, gdk::Key::Delete) if !sel.is_empty() => self.ask_delete(&sel),
            (Loc::Machines | Loc::Peer(_) | Loc::Folder { .. }, gdk::Key::Return | gdk::Key::KP_Enter) => {
                self.open_selected()
            }
            _ => return false,
        }
        true
    }

    fn open_selected(&self) {
        let Some(sel) = self.sel_first() else { return };
        let loc = self.loc.borrow().clone();
        match loc {
            Loc::Machines => self.go(Loc::Peer(sel)),
            Loc::Peer(p) if sel == "home" => self.go(Loc::Folder { peer: p, path: vec![] }),
            Loc::Folder { .. } => {
                if let Some(e) = self.entries().into_iter().find(|e| e.name == sel) {
                    self.open_entry(&e);
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ doing things

    pub(crate) fn new_folder(&self) {
        let Some(dir) = self.dir() else { return };
        match local::new_folder(&dir) {
            Ok(name) => {
                self.reload();
                self.start_rename(&name);
            }
            Err(e) => self.say(&format!("Couldn’t make a folder: {e}")),
        }
    }

    /// Into the Trash, with an Undo.
    pub(crate) fn trash(&self, keys: &[String]) {
        let Some(dir) = self.dir() else { return };
        let paths: Vec<PathBuf> = keys.iter().map(|k| dir.join(k)).collect();
        match local::trash(&paths) {
            Ok(names) => {
                let n = paths.len();
                let what =
                    if n == 1 { "Moved to the Trash".to_string() } else { format!("{n} items moved to the Trash") };
                self.say_undo(&what, names);
                self.selected.borrow_mut().clear();
            }
            Err(e) => self.say(&format!("Couldn’t put it in the Trash: {e}")),
        }
        self.reload();
        self.queue_draw();
    }

    /// The note's Undo: what it moved to the Trash, back.
    pub(crate) fn undo_trash(&self) {
        let names = std::mem::take(&mut *self.undo.borrow_mut());
        let mut back = vec![];
        for n in &names {
            match local::restore(n) {
                Ok(p) => back.push(p),
                Err(e) => self.say(&e),
            }
        }
        if !back.is_empty() {
            self.say(if back.len() == 1 { "It’s back" } else { "They’re back" });
            *self.selected.borrow_mut() =
                back.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
        }
        self.reload();
        self.queue_draw();
    }

    fn trash_name(&self, key: &str) -> String {
        if let Some(n) = key.strip_prefix(FILE) {
            return self.trash.borrow().iter().find(|t| t.name == n).map(|t| t.display.clone()).unwrap_or_default();
        }
        if let Some(id) = key.strip_prefix(WS) {
            return self.state.borrow().wadspace(id).map(|x| x.name.clone()).unwrap_or_default();
        }
        String::new()
    }

    /// Back where they came from (a wadspace: on the desktop again).
    fn restore(&self, keys: &[String]) {
        for k in keys {
            if let Some(n) = k.strip_prefix(FILE) {
                if let Err(e) = local::restore(n) {
                    self.say(&e);
                }
            } else if let Some(id) = k.strip_prefix(WS) {
                let (w, id) = (self.me.clone(), id.to_string());
                self.link.call(
                    move |wadd| async move { wadd.op(wad_model::Op::Restore { ws: id }).await },
                    move |r| {
                        if let (Err(e), Some(m)) = (r, w.upgrade()) {
                            m.say(&e.message);
                        }
                    },
                );
            }
        }
        self.selected.borrow_mut().clear();
        self.reload();
        self.queue_draw();
    }

    fn restore_all(&self) {
        let mut keys: Vec<String> = self.trash.borrow().iter().map(|t| format!("{FILE}{}", t.name)).collect();
        keys.extend(self.state.borrow().trashed().iter().map(|x| format!("{WS}{}", x.id)));
        self.restore(&keys);
    }

    /// Asks first: "Delete “x” for good?"
    fn ask_delete(&self, keys: &[String]) {
        let title = match keys {
            [one] => format!("Delete “{}” for good?", self.trash_name(one)),
            many => format!("Delete {} items for good?", many.len()),
        };
        let (w, keys) = (self.me.clone(), keys.to_vec());
        Dialog::new(&title)
            .icon("trash", Tone::Danger)
            .text("It won’t come back, and a wadspace goes with everything in it.")
            .button("cancel", "Cancel", Variant::Ghost)
            .button("delete", "Delete permanently", Variant::Danger)
            .cancel("cancel")
            .named("delete-forever")
            .open(&self.window, move |_, id| {
                if id == "delete"
                    && let Some(m) = w.upgrade()
                {
                    m.delete_forever(&keys);
                }
            });
    }

    pub(crate) fn delete_forever(&self, keys: &[String]) {
        for k in keys {
            if let Some(n) = k.strip_prefix(FILE) {
                if let Err(e) = local::delete_forever(n) {
                    self.say(&e);
                }
            } else if let Some(id) = k.strip_prefix(WS) {
                self.purge(id);
            }
        }
        self.selected.borrow_mut().clear();
        self.reload();
        self.queue_draw();
    }

    /// A wadspace in the Trash, gone for good (wadd deletes it and everything in it).
    fn purge(&self, id: &str) {
        let (w, id) = (self.me.clone(), id.to_string());
        self.link.call(
            move |wadd| async move { wadd.purge_wadspace(&id).await },
            move |r| {
                if let (Err(e), Some(m)) = (r, w.upgrade()) {
                    m.say(&e.message);
                }
            },
        );
    }

    /// "Empty the Trash?" first.
    pub(crate) fn ask_empty_trash(&self) {
        let w = self.me.clone();
        Dialog::new("Empty the Trash?")
            .icon("trash", Tone::Danger)
            .text("Everything in it goes for good, wadspaces too.")
            .button("cancel", "Cancel", Variant::Ghost)
            .button("empty", "Empty Trash", Variant::Danger)
            .cancel("cancel")
            .named("empty-trash-dialog")
            .open(&self.window, move |_, id| {
                if id == "empty"
                    && let Some(m) = w.upgrade()
                {
                    m.empty_trash();
                }
            });
    }

    pub(crate) fn empty_trash(&self) {
        let ids: Vec<String> = self.state.borrow().trashed().into_iter().map(|x| x.id).collect();
        for id in ids {
            self.purge(&id);
        }
        let w = self.me.clone();
        glib::spawn_future_local(async move {
            let r = gio::spawn_blocking(local::empty_trash).await;
            let Some(m) = w.upgrade() else { return };
            match r {
                Ok(Ok(())) => {}
                Ok(Err(e)) => m.say(&format!("Couldn’t empty the Trash: {e}")),
                Err(_) => m.say("Couldn’t empty the Trash"),
            }
            m.reload();
            m.queue_draw();
        });
    }

    // ------------------------------------------------------------ the clipboard

    /// These on the clipboard (to be moved when pasted, if `cut`), as the desktop puts files there.
    fn copy(&self, keys: &[String], cut: bool) {
        let Some(dir) = self.dir() else { return };
        let paths: Vec<PathBuf> = keys.iter().map(|k| dir.join(k)).collect();
        let list: Vec<gio::File> = paths.iter().map(gio::File::for_path).collect();
        let gnome = format!(
            "{}\n{}",
            if cut { "cut" } else { "copy" },
            list.iter().map(|f| f.uri().to_string()).collect::<Vec<_>>().join("\n")
        );
        let p = gdk::ContentProvider::new_union(&[
            gdk::ContentProvider::for_value(&gdk::FileList::from_array(&list).to_value()),
            gdk::ContentProvider::for_bytes(
                "x-special/gnome-copied-files",
                &glib::Bytes::from_owned(gnome.into_bytes()),
            ),
        ]);
        if let Err(e) = self.window.clipboard().set_content(Some(&p)) {
            tracing::warn!("the clipboard: {e}");
        }
        *self.cut.borrow_mut() = cut.then_some(paths);
        self.queue_draw();
    }

    fn can_paste(&self) -> bool {
        self.window.clipboard().formats().contains_type(gdk::FileList::static_type())
    }

    /// The clipboard's files into `dir` (moved, if they were cut here).
    /// The archive `key` extracted into a folder beside it, which is picked
    /// when it's done (it can take a while: the note says so meanwhile).
    pub(crate) fn extract(&self, key: &str) {
        let Some(dir) = self.dir() else { return };
        let archive = dir.join(key);
        self.say(&format!("Extracting {key}…"));
        let w = self.me.clone();
        glib::spawn_future_local(async move {
            let r = gio::spawn_blocking(move || local::extract(&archive)).await;
            let Some(m) = w.upgrade() else { return };
            match r {
                Ok(Ok(folder)) => {
                    m.say(&format!("Extracted into {folder}"));
                    m.reload();
                    m.pick_only(&folder);
                }
                Ok(Err(e)) => m.say(&e),
                Err(_) => m.say("It didn’t extract"),
            }
            m.reload();
            m.queue_draw();
        });
    }

    fn paste(&self, dir: PathBuf) {
        let (w, clip) = (self.me.clone(), self.window.clipboard());
        glib::spawn_future_local(async move {
            let got = clip.read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT).await;
            let Some(m) = w.upgrade() else { return };
            let paths: Vec<PathBuf> = got
                .ok()
                .and_then(|v| v.get::<gdk::FileList>().ok())
                .map(|l| l.files().iter().filter_map(|f| f.path()).collect())
                .unwrap_or_default();
            if paths.is_empty() {
                m.say("There are no files on the clipboard");
                return;
            }
            let cut = m.cut.borrow().as_ref() == Some(&paths);
            if cut {
                *m.cut.borrow_mut() = None;
            }
            let r =
                gio::spawn_blocking(
                    move || {
                        if cut { local::move_into(&paths, &dir) } else { local::copy_into(&paths, &dir) }
                    },
                )
                .await;
            match r {
                Ok(Ok(names)) => *m.selected.borrow_mut() = names,
                Ok(Err(e)) => m.say(&e),
                Err(_) => m.say("It didn’t paste"),
            }
            m.reload();
            m.queue_draw();
        });
    }

    // ------------------------------------------------------------ a wadspace

    pub(crate) fn space_view(&self, id: &str) {
        let st = self.state.borrow();
        let Some(x) = st.wadspace(id).cloned() else {
            self.body.append(&kit::hero("monitor", "That wadspace is gone.", ""));
            return;
        };
        if x.is_trashed() {
            self.body.append(&kit::hero("monitor", &format!("{} is in the Trash.", x.name), ""));
            return;
        }
        let running = st.running.contains(id);
        drop(st);
        let head = gtk::Box::builder().spacing(22).margin_start(12).margin_top(10).build();
        let tile = WadspaceTile::new(0.72, false);
        tile.set(&x, &TileState::default());
        tile.root.set_valign(gtk::Align::Center);
        head.append(&tile.root);
        let about =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).valign(gtk::Align::Center).build();
        about.append(&kit::label(&x.name, &["ws-h2"]));
        let facts = gtk::Box::builder().spacing(14).build();
        facts.append(&kit::label(
            &format!("{} app{}", x.apps.len(), if x.apps.len() == 1 { "" } else { "s" }),
            &["ws-fact"],
        ));
        if running {
            facts.append(&kit::label("Running", &["ws-fact", "on"]));
        }
        about.append(&facts);
        let open = kit::button(if running { "Show" } else { "Open" }, Variant::Primary, Some("monitor-play"), true);
        open.set_halign(gtk::Align::Start);
        let (w, i) = (self.me.clone(), x.id.clone());
        open.connect_clicked(move |_| {
            let Some(m) = w.upgrade() else { return };
            let (w2, i) = (m.me.clone(), i.clone());
            m.link.call(
                move |wadd| async move { wadd.open(&i, SessionDisplay::Screen).await },
                move |r| {
                    if let (Err(e), Some(m)) = (r, w2.upgrade()) {
                        m.say(&e.message);
                    }
                },
            );
        });
        about.append(&open);
        head.append(&about);
        self.body.append(&head);
        if !x.apps.is_empty() {
            self.body.append(&section("IN IT"));
            let apps = gtk::FlowBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .column_spacing(8)
                .row_spacing(8)
                .max_children_per_line(8)
                .margin_start(12)
                .build();
            for a in &x.apps {
                let c = gtk::Box::builder().spacing(8).css_classes(["ws-app"]).build();
                c.append(&wad_gtk::app_icon::app(a.app, 22));
                c.append(&gtk::Label::new(Some(catalog::app_name(a.app))));
                apps.append(&c);
            }
            self.body.append(&apps);
        }
        self.body.append(&section("PROJECTS"));
        if x.look.projects.is_empty() {
            let l = kit::label("No projects", &["files-away"]);
            l.set_margin_start(12);
            self.body.append(&l);
        }
        let machine = |m: &str| {
            let st = self.state.borrow();
            if st.machine.as_ref().is_some_and(|x| x.id == m) || m.is_empty() {
                "this machine".to_string()
            } else {
                st.peer(m).map(|p| p.peer.name.clone()).unwrap_or_else(|| "another machine".into())
            }
        };
        for p in &x.look.projects {
            let (icon, from) = match &p.kind {
                ProjectKind::Github { repo, setup } => (
                    "git-branch",
                    format!("github.com/{repo}{}", setup.as_ref().map(|s| format!(" · then {s}")).unwrap_or_default()),
                ),
                ProjectKind::Mount { source, .. } => {
                    ("link", format!("Mounted from {}: {}", machine(&source.machine_id), source.paths.join(", ")))
                }
                ProjectKind::Copy { source } => {
                    ("copy", format!("Copied from {}: {}", machine(&source.machine_id), source.paths.join(", ")))
                }
            };
            let r = gtk::Box::builder().spacing(8).margin_start(12).margin_top(4).build();
            r.append(&Icon::new(icon, 13));
            r.append(&kit::label(&p.as_name, &[]));
            r.append(&kit::label(&from, &["files-away"]));
            self.body.append(&r);
        }
        let foot = kit::label("Read-only here. The Builder changes what’s in a wadspace.", &["files-away"]);
        foot.set_margin_start(12);
        foot.set_margin_top(20);
        self.body.append(&foot);
    }
}

/// One thing as the grid and the list show it.
struct Shown {
    key: String,
    name: String,
    glyph: &'static str,
    thumb: Option<PathBuf>,
    #[allow(dead_code)]
    size: String,
    /// The list's other columns.
    cols: [String; 3],
    cut: bool,
    /// A wadspace's tile (one in the Trash).
    tile: Option<wad_model::Wadspace>,
}

fn section(t: &str) -> gtk::Label {
    let l = kit::label(t, &["files-section"]);
    l.set_margin_top(18);
    l.set_margin_start(12);
    l
}

/// A list column's width (the spike's grid templates).
fn size_col(w: &impl IsA<gtk::Widget>, i: usize, trash: bool) {
    let widths = if trash { [-1, 150, 96, 84] } else { [-1, 84, 104, 130] };
    if i == 0 {
        w.set_hexpand(true);
    } else {
        w.set_size_request(widths[i], -1);
    }
}

/// A folder's "3 items", a file's size.
fn size_of(i: &Item) -> String {
    match i.count {
        Some(n) => format!("{n} item{}", if n == 1 { "" } else { "s" }),
        None => crate::drive::bytes(i.size),
    }
}

/// A picture's own thumbnail (a small one shown as it is).
fn thumb(i: &Item) -> Option<PathBuf> {
    (i.content_type.starts_with("image/") && i.size < 8 << 20 && i.content_type != "image/svg+xml")
        .then(|| i.path.clone())
}
