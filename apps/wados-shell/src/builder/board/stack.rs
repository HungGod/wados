//! One wadspace on the board, as a stack (`Stack.svelte`, wad-board's
//! card): its tile, name and state on top (× puts it away), then its four
//! sections, each a card in its colour:
//!
//! - **Customization:** its desktop as it'll be, then its look and files.
//! - **Secrets:** a box per key with what signs in with it (`stack::key_boxes`).
//! - **Add-ons:** a box per app with add-ons; the command line's has its commands.
//! - **Apps:** its apps by name.
//!
//! Its parts ([`Part`]) are what the board drags (its header: all of it; a
//! section by its name; a key's box by its key; one thing), what it drops
//! on (a key's box), and what `wadsctl stack` aims at, by id.
//!
//! Another machine's shows, and doesn't change here: a note on top says
//! where it lives, with Copy here (`Builder.svelte`'s `.away`).

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use wad_model::catalog::app_name;
use wad_model::chips::project_glyph;
use wad_model::edit::{Op, Payload};
use wad_model::stack::{addon_boxes, key_boxes, stack_apps};
use wad_model::variants::{AppVariant, VariantKind, find_variant};
use wad_model::wallpapers::wallpaper_name;
use wad_model::{AppId, BuildCommand, Section, Wadspace};

use super::Board;
use super::pieces::{self, Status};
use super::preview;
use crate::dnd::Held;
use wad_gtk::Wrap;

/// Its parts' paddings (board.css): the stack's and a section's, and the stack's border.
const STACK_PAD: i32 = 8;
const MOD_PAD: i32 = 8;
const BORDER: i32 = 1;
/// A box's narrowest, side by side with others.
const BOX_MIN: i32 = 250;
const BOX_GAP: i32 = 8;

/// The note on another machine's stack (`Builder.svelte`'s `.away`).
const CSS: &str = r#"
.stack-away { padding: 8px 8px 8px 10px; border-radius: 10px; background-color: color-mix(in srgb, var(--c-app) 10%, transparent); border: 1px solid color-mix(in srgb, var(--c-app) 30%, transparent); font-size: 12.5px; color: var(--c-app); }
.stack-away label { color: var(--fg); }
button.stack-away-copy { min-height: 26px; padding: 0 10px; border-radius: 8px; border: none; box-shadow: none; background: var(--c-app); color: var(--accent-fg); font-size: 12.5px; font-weight: 600; }
button.stack-away-copy:hover { background: color-mix(in srgb, var(--c-app) 85%, white); }
"#;

thread_local! {
    static CSS_ADDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Another machine's stack's note: where it lives, and what to do.
pub fn away_text(machine: &str) -> String {
    format!("It lives on {machine}: copy it here to change it.")
}

/// One of its parts: what it is (for wadsctl), and what it does under a drag.
pub struct Part {
    /// 'top', 'secrets', 'key:work-web', 'secrets/ext:wadbrowser:gmail', 'apps/vscode', 'cmd/<id>'; a × is its thing's id and '#x'.
    pub id: String,
    pub widget: gtk::Widget,
    /// Dragging it carries this, looking like `look`.
    pub drag: Option<(Held, gtk::Widget)>,
    /// A key's box: a thing of its own dropped on it signs in with that key.
    pub key: Option<(AppId, String)>,
    /// A button (a ×, its face): no drag starts on it but its own.
    pub button: Option<gtk::Button>,
}

/// How it's shown: selected, its state, its tile's badges, whether it can be changed here, its width.
pub struct How {
    pub selected: bool,
    pub status: Status,
    pub tile: wad_gtk::TileState,
    pub editable: bool,
    pub col: i32,
}

/// What its sections were last made of.
#[derive(Clone, PartialEq)]
struct Made {
    x: Wadspace,
    editable: bool,
    col: i32,
}

pub struct Stack {
    pub id: String,
    /// What the board lines up: `root` in a column the board's half wide.
    pub column: super::cells::Column,
    pub root: gtk::Box,
    head: gtk::Box,
    tile: wad_gtk::WadspaceTile,
    name: gtk::Label,
    state: gtk::Label,
    close: gtk::Button,
    body: gtk::Box,
    made: RefCell<Option<Made>>,
    parts: RefCell<Vec<Part>>,
}

impl Stack {
    pub fn new(board: &Rc<Board>, id: &str) -> Rc<Self> {
        if !CSS_ADDED.with(|c| c.replace(true))
            && let Some(d) = gtk::gdk::Display::default()
        {
            wad_gtk::theme::add_css(&d, CSS);
        }
        let root =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["stack"]).build();
        root.set_valign(gtk::Align::Start);
        root.set_widget_name(&format!("stack:{id}"));
        let head = gtk::Box::builder().spacing(8).css_classes(["stack-top"]).tooltip_text("Drag it: all of it").build();
        let tile = wad_gtk::WadspaceTile::new(0.62, false);
        tile.root.set_valign(gtk::Align::Center);
        head.append(&tile.root);
        let name = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["stack-name", "display"])
            .build();
        head.append(&name);
        let state = gtk::Label::builder().css_classes(["stack-state"]).valign(gtk::Align::Center).build();
        head.append(&state);
        let close = gtk::Button::builder()
            .css_classes(["stack-close"])
            .tooltip_text("Put it away (it stays on the desktop)")
            .valign(gtk::Align::Center)
            .child(&wad_gtk::Icon::new("x", 13))
            .build();
        head.append(&close);
        root.append(&head);
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
        root.append(&body);

        let w = Rc::downgrade(board);
        let ws = id.to_string();
        close.connect_clicked(move |_| {
            if let Some(b) = w.upgrade().and_then(|b| b.builder()) {
                b.close_stack(&ws);
            }
        });
        // A click anywhere on it selects it (before what's clicked does its own).
        let press = gtk::GestureClick::builder().propagation_phase(gtk::PropagationPhase::Capture).button(0).build();
        let w = Rc::downgrade(board);
        let ws = id.to_string();
        press.connect_pressed(move |_, _, _, _| {
            if let Some(b) = w.upgrade().and_then(|b| b.builder()) {
                b.select(&ws);
            }
        });
        root.add_controller(press);

        let column = super::cells::Column::new(&root);
        Rc::new(Self {
            id: id.to_string(),
            column,
            root,
            head,
            tile,
            name,
            state,
            close,
            body,
            made: RefCell::default(),
            parts: RefCell::default(),
        })
    }

    /// Shown as `x` is now: its header always; its sections when it (or what can be done to it) changed.
    pub fn show(&self, board: &Rc<Board>, x: &Wadspace, how: &How) {
        let How { selected, status, tile, editable, col } = how;
        let (selected, editable, col) = (*selected, *editable, *col);
        self.column.set_width(col);
        if selected {
            self.root.add_css_class("selected");
        } else {
            self.root.remove_css_class("selected");
        }
        if editable {
            self.root.remove_css_class("away");
        } else {
            self.root.add_css_class("away");
        }
        self.tile.set(x, tile);
        self.name.set_label(&x.name);
        self.name.set_tooltip_text(Some(&x.name));
        self.state.set_label(&status.label);
        for t in ["away", "busy", "run", "ok", "todo"] {
            self.state.remove_css_class(t);
        }
        self.state.add_css_class(status.tone.class());
        self.state.set_widget_name(&status.label);
        self.state.set_tooltip_text((!editable).then(|| away_text(&status.label)).as_deref());

        let made = Made { x: x.clone(), editable, col };
        if self.made.borrow().as_ref() == Some(&made) {
            return;
        }
        while let Some(c) = self.body.first_child() {
            self.body.remove(&c);
        }
        let mut parts = vec![
            Part {
                id: "top".into(),
                widget: self.head.clone().upcast(),
                drag: Some((Held::Space { ws: x.id.clone(), from: None }, self.head.clone().upcast())),
                key: None,
                button: None,
            },
            Part {
                id: "close".into(),
                widget: self.close.clone().upcast(),
                drag: None,
                key: None,
                button: Some(self.close.clone()),
            },
        ];
        if !editable {
            self.body.append(&self.away(board, x, &status.label, &mut parts));
        }
        let mut f = Fill {
            board: Rc::downgrade(board),
            x,
            editable,
            inner: col - 2 * (STACK_PAD + MOD_PAD + BORDER),
            parts: &mut parts,
        };
        self.body.append(&f.custom());
        self.body.append(&f.secrets());
        self.body.append(&f.addons());
        self.body.append(&f.apps());
        *self.parts.borrow_mut() = parts;
        *self.made.borrow_mut() = Some(made);
    }

    /// Another machine's note: its glyph, where it lives, Copy here (part `copy-here`).
    fn away(&self, board: &Rc<Board>, x: &Wadspace, machine: &str, parts: &mut Vec<Part>) -> gtk::Box {
        let note = gtk::Box::builder().spacing(8).css_classes(["stack-away"]).build();
        let icon = board.builder().map(|b| b.machine_icon(x)).unwrap_or("monitor");
        let ic = wad_gtk::Icon::new(icon, 15);
        ic.set_valign(gtk::Align::Center);
        note.append(&ic);
        let text = gtk::Label::builder()
            .label(away_text(machine))
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .valign(gtk::Align::Center)
            .build();
        note.append(&text);
        let copy = gtk::Button::builder()
            .label("Copy here")
            .css_classes(["stack-away-copy"])
            .valign(gtk::Align::Center)
            .build();
        let (w, ws) = (Rc::downgrade(board), x.id.clone());
        copy.connect_clicked(move |_| {
            if let Some(b) = w.upgrade().and_then(|b| b.builder()) {
                b.copy_here(&ws);
            }
        });
        note.append(&copy);
        parts.push(Part {
            id: "copy-here".into(),
            widget: copy.clone().upcast(),
            drag: None,
            key: None,
            button: Some(copy),
        });
        note
    }

    /// Its part `id`.
    pub fn part(&self, id: &str) -> Option<gtk::Widget> {
        if id.is_empty() || id == "stack" {
            return Some(self.root.clone().upcast());
        }
        self.parts.borrow().iter().find(|p| p.id == id).map(|p| p.widget.clone())
    }

    /// Its part `id`'s button (a ×, a thing's face, the header's ×).
    pub fn button(&self, id: &str) -> Option<gtk::Button> {
        self.parts.borrow().iter().find(|p| p.id == id).and_then(|p| p.button.clone())
    }

    /// What dragging from `w` (a widget in it) carries: the nearest part that drags, unless a button of its own is nearer.
    pub fn drag_at(&self, w: &gtk::Widget) -> Option<(Held, gtk::Widget)> {
        let parts = self.parts.borrow();
        let mut c = Some(w.clone());
        while let Some(at) = c {
            if at == *self.root.upcast_ref::<gtk::Widget>() {
                return None;
            }
            if let Some(p) = parts.iter().find(|p| p.widget == at) {
                if let Some(d) = &p.drag {
                    return Some(d.clone());
                }
                if p.button.is_some() && p.id.ends_with("#x") || p.id == "close" {
                    return None;
                }
            }
            c = at.parent();
        }
        None
    }

    /// The key's box `w` is in, if it's in one.
    pub fn key_at(&self, w: &gtk::Widget) -> Option<(AppId, String, gtk::Widget)> {
        let parts = self.parts.borrow();
        let mut c = Some(w.clone());
        while let Some(at) = c {
            if at == *self.root.upcast_ref::<gtk::Widget>() {
                return None;
            }
            if let Some((app, key)) = parts.iter().find(|p| p.widget == at).and_then(|p| p.key.clone()) {
                return Some((app, key, at));
            }
            c = at.parent();
        }
        None
    }

    /// Its key's box for `key`.
    pub fn key_box(&self, app: AppId, key: &str) -> Option<gtk::Widget> {
        self.parts
            .borrow()
            .iter()
            .find(|p| p.key.as_ref().is_some_and(|(a, k)| *a == app && k == key))
            .map(|p| p.widget.clone())
    }

    /// What its header says of its state.
    pub fn status_label(&self) -> String {
        self.state.label().to_string()
    }

    /// The ids of its parts (wadsctl).
    pub fn part_ids(&self) -> Vec<String> {
        self.parts.borrow().iter().map(|p| p.id.clone()).collect()
    }
}

/// Making its sections.
struct Fill<'a> {
    board: Weak<Board>,
    x: &'a Wadspace,
    editable: bool,
    /// A section's inside width.
    inner: i32,
    parts: &'a mut Vec<Part>,
}

impl Fill<'_> {
    fn part(&mut self, id: impl Into<String>, widget: &impl IsA<gtk::Widget>) -> &mut Part {
        self.parts.push(Part { id: id.into(), widget: widget.clone().upcast(), drag: None, key: None, button: None });
        self.parts.last_mut().expect("just pushed")
    }

    /// A section: a card in its colour, its name (drag it: the whole section) and a count.
    fn section(&mut self, section: Section, id: &str, icon: &'static str, count: Option<usize>) -> gtk::Box {
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(7)
            .css_classes(["stack-mod", id])
            .build();
        let h =
            gtk::Box::builder().spacing(6).css_classes(["stack-h4"]).tooltip_text("Drag it: the whole section").build();
        h.append(&wad_gtk::Icon::new(icon, 13));
        h.append(&gtk::Label::builder().label(section.label().to_uppercase()).css_classes(["stack-h4-label"]).build());
        if let Some(n) = count {
            h.append(&gtk::Label::builder().label(n.to_string()).css_classes(["stack-n"]).build());
        }
        card.append(&h);
        self.part(id, &card);
        let drag = Held::Part { payload: Payload::Section { ws: self.x.id.clone(), section } };
        self.part(format!("{id}:head"), &h).drag = Some((drag, h.clone().upcast()));
        card
    }

    fn none(text: &str) -> gtk::Label {
        gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["stack-none"]).build()
    }

    /// Boxes side by side when there's room for them.
    fn boxes(&self) -> gtk::Grid {
        gtk::Grid::builder().column_homogeneous(true).column_spacing(BOX_GAP).row_spacing(BOX_GAP).build()
    }

    /// The `i`th box in its place.
    fn put(&self, g: &gtk::Grid, i: usize, w: &impl IsA<gtk::Widget>) {
        let cols = ((self.inner + BOX_GAP) / (BOX_MIN + BOX_GAP)).max(1) as usize;
        g.attach(w, (i % cols) as i32, (i / cols) as i32, 1, 1);
    }

    fn custom(&mut self) -> gtk::Box {
        let card = self.section(Section::Look, "customization", "palette", None);
        card.append(&preview::desk(self.x, self.inner));
        let chips = Wrap::new(4, 4);
        chips.append(&chip("palette", &wallpaper_name(&self.x.look.wallpaper)));
        for p in &self.x.look.projects {
            chips.append(&chip(project_glyph(p), &p.as_name));
        }
        card.append(&chips);
        card
    }

    fn secrets(&mut self) -> gtk::Box {
        let keys = key_boxes(self.x);
        let card = self.section(Section::Secrets, "secrets", "key-round", Some(keys.len()));
        let grid = self.boxes();
        for (i, b) in keys.iter().enumerate() {
            let Some(key) = b.folder.key.clone() else { continue };
            let bx = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["stack-box"]).build();
            let head = gtk::Box::builder()
                .spacing(7)
                .css_classes(["stack-bh"])
                .tooltip_text("Drag it: the key, with what signs in with it")
                .build();
            let one = b.folder.items.len() == 1 && b.folder.items[0].kind != VariantKind::App;
            head.append(&if one {
                preview::thing_icon(&b.folder.items[0], 15)
            } else {
                wad_gtk::app_icon::app(b.app, 15)
            });
            head.append(
                &gtk::Label::builder()
                    .label(&key.name)
                    .tooltip_text(&key.name)
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["stack-kn"])
                    .build(),
            );
            head.append(
                &gtk::Label::builder().label(b.folder.items.len().to_string()).css_classes(["stack-n"]).build(),
            );
            let id = format!("key:{}", key.id);
            if self.editable {
                let x = self.boxx("Take the key off (they sign in with nothing)");
                let (w, ws, app, k) = (self.board.clone(), self.x.id.clone(), b.app, key.id.clone());
                x.connect_clicked(move |_| {
                    if let Some(b) = w.upgrade() {
                        b.op(Op::DropKey { ws: ws.clone(), app, key: k.clone() });
                    }
                });
                head.append(&x);
                self.part(format!("{id}#x"), &x).button = Some(x.clone());
            }
            bx.append(&head);
            let pills = Wrap::new(0, 0);
            pills.add_css_class("stack-pills");
            for v in &b.folder.items {
                pills.append(&self.pill("secrets", v));
            }
            bx.append(&pills);
            self.part(&id, &bx).key = Some((b.app, key.id.clone()));
            let drag = Held::Part {
                payload: Payload::Folder { ws: self.x.id.clone(), app: b.app, key: Some(key.id.clone()) },
            };
            self.part(format!("{id}:head"), &head).drag = Some((drag, bx.clone().upcast()));
            self.put(&grid, i, &bx);
        }
        if keys.is_empty() {
            grid.attach(&Self::none("No secrets: give its apps keys in the sidebar’s Secrets"), 0, 0, 1, 1);
        }
        card.append(&grid);
        card
    }

    fn addons(&mut self) -> gtk::Box {
        let boxes = addon_boxes(self.x);
        let n: usize = boxes.iter().map(|b| b.items.len()).sum::<usize>() + self.x.commands.len();
        let card = self.section(Section::Addons, "addons", "puzzle", Some(n));
        let grid = self.boxes();
        for (i, b) in boxes.iter().enumerate() {
            let bx = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["stack-box"]).build();
            let head = gtk::Box::builder().spacing(7).css_classes(["stack-bh", "still"]).build();
            head.append(&wad_gtk::app_icon::app(b.app, 15));
            head.append(
                &gtk::Label::builder()
                    .label(app_name(b.app))
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["stack-kn"])
                    .build(),
            );
            head.append(
                &gtk::Label::builder()
                    .label((b.items.len() + b.commands.len()).to_string())
                    .css_classes(["stack-n"])
                    .build(),
            );
            let id = format!("box:{}", b.app);
            if self.editable && !b.items.is_empty() {
                let x = self.boxx("Take its add-ons out");
                let (w, ws, app) = (self.board.clone(), self.x.id.clone(), b.app);
                x.connect_clicked(move |_| {
                    if let Some(b) = w.upgrade() {
                        b.op(Op::RemoveAddons { ws: ws.clone(), app });
                    }
                });
                head.append(&x);
                self.part(format!("{id}#x"), &x).button = Some(x.clone());
            }
            bx.append(&head);
            let pills = Wrap::new(0, 0);
            pills.add_css_class("stack-pills");
            for v in &b.items {
                pills.append(&self.pill("addons", v));
            }
            for c in &b.commands {
                pills.append(&self.command(c));
            }
            bx.append(&pills);
            self.part(&id, &bx);
            self.put(&grid, i, &bx);
        }
        if boxes.is_empty() {
            grid.attach(&Self::none("No add-ons: click one in the sidebar’s Add-ons"), 0, 0, 1, 1);
        }
        card.append(&grid);
        card
    }

    fn apps(&mut self) -> gtk::Box {
        let apps = stack_apps(self.x);
        let card = self.section(Section::Apps, "apps", "layout-grid", Some(apps.len()));
        let tiles = Wrap::new(1, 1);
        for app in &apps {
            if let Some(v) = find_variant(self.x, &format!("app:{app}")) {
                tiles.append(&self.tile(&v));
            }
        }
        if apps.is_empty() {
            card.append(&Self::none("No apps: click one in the sidebar’s Apps"));
        } else {
            card.append(&tiles);
        }
        card
    }

    /// A box's ×.
    fn boxx(&self, tip: &str) -> gtk::Button {
        gtk::Button::builder()
            .css_classes(["stack-bx"])
            .tooltip_text(tip)
            .valign(gtk::Align::Center)
            .child(&wad_gtk::Icon::new("x", 11))
            .build()
    }

    /// One thing as a pill (`Pill.svelte`): its icon and name. Click it for its menu; hover it for its ×; drag it (just it).
    fn pill(&mut self, section: &str, v: &AppVariant) -> gtk::Widget {
        let wrap = gtk::Overlay::builder().css_classes(["stack-pill-wrap"]).build();
        let face = gtk::Button::builder().css_classes(["stack-pill"]).tooltip_text(&v.name).build();
        face.set_margin_top(6);
        face.set_margin_end(6);
        let row = gtk::Box::builder().spacing(6).build();
        let icon = preview::thing_icon(v, 16);
        icon.set_valign(gtk::Align::Center);
        row.append(&icon);
        row.append(
            &gtk::Label::builder()
                .label(&v.name)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["stack-pill-name"])
                .build(),
        );
        if pieces::launches(self.x, v) {
            row.append(&star());
        }
        face.set_child(Some(&row));
        wrap.set_child(Some(&face));
        let id = format!("{section}/{}", v.id);
        self.menu_on(&face, &id, v);
        if self.editable {
            let x = remove_x("Take it out");
            x.set_halign(gtk::Align::End);
            x.set_valign(gtk::Align::Start);
            let (w, ws, chip) = (self.board.clone(), self.x.id.clone(), v.id.clone());
            x.connect_clicked(move |_| {
                if let Some(b) = w.upgrade() {
                    b.op(Op::Remove { ws: ws.clone(), chip: chip.clone() });
                }
            });
            wrap.add_overlay(&x);
            self.part(format!("{id}#x"), &x).button = Some(x.clone());
        }
        let drag = pieces::piece(&self.x.id, v).map(|payload| (Held::Part { payload }, wrap.clone().upcast()));
        let p = self.part(&id, &wrap);
        p.drag = drag;
        p.button = Some(face);
        wrap.upcast()
    }

    /// One of its own commands, by its name (RUN_1); the command in its tooltip; × takes it out.
    fn command(&mut self, c: &BuildCommand) -> gtk::Widget {
        let b = gtk::Box::builder()
            .spacing(4)
            .css_classes(["stack-cmd", "mono"])
            .tooltip_text(format!("{} {}", c.kind.word(), c.text))
            .valign(gtk::Align::Center)
            .build();
        b.set_margin_top(6);
        b.set_margin_end(6);
        b.append(&gtk::Label::builder().label(&c.name).build());
        let id = format!("cmd/{}", c.id);
        if self.editable {
            let x = gtk::Button::builder()
                .css_classes(["stack-cx"])
                .tooltip_text(format!("Take {} out", c.name))
                .child(&wad_gtk::Icon::new("x", 10))
                .build();
            let (w, ws, cid) = (self.board.clone(), self.x.id.clone(), c.id.clone());
            x.connect_clicked(move |_| {
                if let Some(b) = w.upgrade() {
                    b.op(Op::RemoveCommand { ws: ws.clone(), id: cid.clone() });
                }
            });
            b.append(&x);
            self.part(format!("{id}#x"), &x).button = Some(x.clone());
        }
        self.part(&id, &b);
        b.upcast()
    }

    /// One of its apps, by name (click it for its menu; the one it starts straight into has a star).
    fn tile(&mut self, v: &AppVariant) -> gtk::Widget {
        let wrap = gtk::Overlay::builder().css_classes(["stack-tile-wrap"]).build();
        let face = gtk::Button::builder().css_classes(["stack-tile"]).build();
        face.set_margin_top(5);
        face.set_margin_end(5);
        let launches = pieces::launches(self.x, v);
        face.set_tooltip_text(Some(&if launches {
            format!("{}: it starts straight into this", v.name)
        } else {
            v.name.clone()
        }));
        let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
        let icon = wad_gtk::app_icon::app(v.app, 28);
        icon.set_halign(gtk::Align::Center);
        col.append(&icon);
        col.append(
            &gtk::Label::builder()
                .label(&v.name)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .max_width_chars(7)
                .css_classes(["stack-tn"])
                .build(),
        );
        face.set_child(Some(&col));
        wrap.set_child(Some(&face));
        if launches {
            let s = star();
            s.set_halign(gtk::Align::Start);
            s.set_valign(gtk::Align::Start);
            s.set_margin_start(6);
            s.set_margin_top(9);
            wrap.add_overlay(&s);
        }
        let id = format!("apps/{}", v.app);
        self.menu_on(&face, &id, v);
        if self.editable {
            let x = remove_x(&format!("Take {} out", v.name));
            x.set_halign(gtk::Align::End);
            x.set_valign(gtk::Align::Start);
            let (w, ws, chip) = (self.board.clone(), self.x.id.clone(), v.id.clone());
            x.connect_clicked(move |_| {
                if let Some(b) = w.upgrade() {
                    b.op(Op::Remove { ws: ws.clone(), chip: chip.clone() });
                }
            });
            wrap.add_overlay(&x);
            self.part(format!("{id}#x"), &x).button = Some(x.clone());
        }
        let drag = pieces::piece(&self.x.id, v).map(|payload| (Held::Part { payload }, wrap.clone().upcast()));
        let p = self.part(&id, &wrap);
        p.drag = drag;
        p.button = Some(face);
        wrap.upcast()
    }

    /// Clicking `face` opens `v`'s menu, under its part `part` (another machine's has none: it doesn't change).
    fn menu_on(&self, face: &gtk::Button, part: &str, v: &AppVariant) {
        if !self.editable {
            return;
        }
        let (w, ws, part, vid) = (self.board.clone(), self.x.id.clone(), part.to_string(), v.id.clone());
        face.connect_clicked(move |_| {
            if let Some(b) = w.upgrade() {
                b.open_menu(&ws, &part, &vid, true);
            }
        });
    }
}

/// A chip of its look: its wallpaper's name, a file's.
fn chip(icon: &'static str, label: &str) -> gtk::Widget {
    let b = gtk::Box::builder().spacing(4).css_classes(["stack-chip"]).build();
    let i = wad_gtk::Icon::new(icon, 11);
    i.set_valign(gtk::Align::Center);
    b.append(&i);
    b.append(&gtk::Label::builder().label(label).ellipsize(gtk::pango::EllipsizeMode::End).build());
    b.upcast()
}

/// A thing's × (shows on hover).
fn remove_x(tip: &str) -> gtk::Button {
    gtk::Button::builder().css_classes(["stack-x"]).tooltip_text(tip).child(&wad_gtk::Icon::new("x", 10)).build()
}

/// It starts straight into this.
fn star() -> gtk::Widget {
    let s = gtk::Box::builder().css_classes(["stack-star"]).valign(gtk::Align::Center).hexpand(false).build();
    s.set_size_request(15, 15);
    let i = wad_gtk::Icon::filled("play", 8);
    i.set_hexpand(true);
    i.set_halign(gtk::Align::Center);
    i.set_valign(gtk::Align::Center);
    s.append(&i);
    s.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn another_machines_stack_says_where_it_lives() {
        assert_eq!(away_text("Desk"), "It lives on Desk: copy it here to change it.");
    }
}
