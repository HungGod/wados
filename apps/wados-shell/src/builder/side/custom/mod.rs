//! The sidebar's Customize tab (the spike's `SideCustom.svelte`): its
//! title; its **tag** (the desktop's group it's in: General, loose on the
//! desktop, or one of yours, each a chip in its colour; Work and Gaming
//! suggested, dashed, until they're made, a click making one with it in; New
//! tag for a name of your own); its projects (each with its name and what
//! it is, a repository's name, mounted or copied in, and an x to take it
//! out; "A folder, file or repository" opens [`files`]' menu); its background ([`photos`]:
//! Empty, your photos from ~/Pictures, WadSpaces' pictures, or Upload your
//! own photo); and its **icon** ([`icon`]), always a picture. There's no
//! password to open a wadspace, and no description.
//!
//! Every change is an edit sent to wadd; `show` draws what comes back. It's
//! called often, so what's typed, the scroll and the icon's draft stay, and
//! only what changed is made again.
//!
//! `wadsctl custom …` says what's shown, where things are on the screen,
//! and clicks them (below, [`Panel::ctl`]).

pub mod files;
pub mod icon;
pub mod photos;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use gtk::{gdk, glib, graphene, prelude::*};
use wad_model::chips::project_glyph;
use wad_model::edit::Op;
use wad_model::tags::{GENERAL_ID, general, tag_of};
use wad_model::types::ProjectKind;
use wad_model::{Project, Tag, Wadspace};
use wad_proto::Level;

use super::kit;
use crate::builder::Builder;
use wad_gtk::Wrap;

const CSS: &str = include_str!("custom.css");

/// Tags you might want, until there are: a click makes one, with it in.
const SUGGESTED: [&str; 2] = ["Work", "Gaming"];

/// What the projects button does.
const FILES_TIP: &str = "A folder of this machine or another of yours (mounted), or a GitHub repository (cloned)";

thread_local! {
    /// The panel `wadsctl custom` talks to (the open window's).
    static LIVE: RefCell<Weak<Panel>> = const { RefCell::new(Weak::new()) };
    static WIRED: Cell<bool> = const { Cell::new(false) };
    static STYLED: Cell<bool> = const { Cell::new(false) };
}

/// A chip on the Tag row.
#[derive(Debug, Clone, PartialEq)]
enum Chip {
    Tag(Tag),
    Suggest(&'static str),
}

impl Chip {
    fn key(&self) -> String {
        match self {
            Chip::Tag(t) => format!("tag:{}", t.id),
            Chip::Suggest(n) => format!("suggest:{n}"),
        }
    }
}

pub struct Panel {
    builder: Weak<Builder>,
    me: Weak<Panel>,
    root: gtk::Box,
    ws: RefCell<Option<Wadspace>>,
    editable: Cell<bool>,

    title: gtk::Entry,
    /// The last rename sent (its wadspace, the name), so Enter then leaving sends one.
    renamed: RefCell<Option<(String, String)>>,

    tags: Wrap,
    chips: RefCell<Vec<(Chip, gtk::Button)>>,
    new_tag: gtk::Button,
    ask: gtk::Popover,
    ask_entry: gtk::Entry,
    ask_make: gtk::Button,

    projects: gtk::Box,
    project_rows: RefCell<Vec<(Project, gtk::Box)>>,
    add_files: gtk::Button,

    background: Rc<photos::Picker>,
    icon: Rc<icon::Editor>,

    /// Where the pointer last moved over the panel (its own coordinates), for `calibrate`.
    pointer: Rc<Cell<Option<(f64, f64)>>>,
    /// The window's top left on the screen, as `calibrate` found it.
    origin: Cell<Option<(f64, f64)>>,
}

impl Panel {
    pub fn new(b: &Rc<Builder>) -> Rc<Self> {
        // Once (a panel's made with each Builder window).
        if !STYLED.replace(true)
            && let Some(d) = gdk::Display::default()
        {
            wad_gtk::theme::add_css(&d, CSS);
        }
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["cz"]).build();
        let photos = photos::Photos::new();

        // Title.
        root.append(&heading("Title", true));
        let title = gtk::Entry::builder().placeholder_text("Name this wadspace").css_classes(["cz-title"]).build();
        root.append(&title);

        // Tag.
        root.append(&heading("Tag", false));
        let tags = Wrap::new(6, 6);
        tags.add_css_class("cz-tags");
        let new_tag = chip_button("New tag", None, true);
        new_tag.set_tooltip_text(Some("A tag of your own"));
        tags.append(&new_tag);
        let ask = gtk::Popover::builder().css_classes(["cz-ask"]).position(gtk::PositionType::Bottom).build();
        let ask_col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
        ask_col.append(&gtk::Label::builder().label("A new tag").xalign(0.0).css_classes(["cz-ask-title"]).build());
        let ask_entry = gtk::Entry::builder().placeholder_text("Its name").css_classes(["cz-title"]).build();
        ask_col.append(&ask_entry);
        let ask_row = gtk::Box::builder().spacing(6).halign(gtk::Align::End).build();
        let ask_cancel = gtk::Button::builder().label("Cancel").css_classes(["cz-ask-b"]).build();
        let ask_make = gtk::Button::builder().label("Make it").css_classes(["cz-ask-b", "go"]).build();
        ask_row.append(&ask_cancel);
        ask_row.append(&ask_make);
        ask_col.append(&ask_row);
        ask.set_child(Some(&ask_col));
        ask.set_parent(&tags);
        root.append(&tags);

        // Projects.
        root.append(&heading("Projects", false));
        let projects = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
        let add_files = gtk::Button::builder().css_classes(["cz-add"]).sensitive(false).tooltip_text(FILES_TIP).build();
        let row = gtk::Box::builder().spacing(6).halign(gtk::Align::Center).build();
        row.append(&wad_gtk::Icon::new("plus", 14));
        row.append(&gtk::Label::new(Some("A folder, file or repository")));
        add_files.set_child(Some(&row));
        // A disabled button's tooltip shows from what's around it.
        let add_wrap = gtk::Box::builder().tooltip_text(FILES_TIP).build();
        add_files.set_hexpand(true);
        add_wrap.append(&add_files);
        projects.append(&add_wrap);
        root.append(&projects);

        // Background.
        root.append(&heading("Background", false));
        let me = Rc::new_cyclic(|me: &Weak<Panel>| {
            let (w, f) = (me.clone(), me.clone());
            let background = photos::Picker::new(
                &photos,
                true,
                move |wp| {
                    if let Some(p) = w.upgrade() {
                        p.edit(|ws| Op::SetWallpaper { ws, wallpaper: wp });
                    }
                },
                move |why| {
                    if let Some(p) = f.upgrade() {
                        p.say(why);
                    }
                },
            );
            Self {
                builder: Rc::downgrade(b),
                me: me.clone(),
                root,
                ws: RefCell::default(),
                editable: Cell::new(false),
                title,
                renamed: RefCell::default(),
                tags,
                chips: RefCell::default(),
                new_tag,
                ask,
                ask_entry,
                ask_make,
                projects,
                project_rows: RefCell::default(),
                add_files,
                background,
                icon: icon::Editor::new(b, &photos),
                pointer: Rc::default(),
                origin: Cell::new(None),
            }
        });
        me.root.append(&me.background.widget());
        me.root.append(&heading("Icon", false));
        me.root.append(&me.icon.widget());
        me.wire(&ask_cancel);
        me.ctl();
        me
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn wire(&self, ask_cancel: &gtk::Button) {
        // The title: Enter, or leaving it, renames.
        let w = self.me.clone();
        self.title.connect_activate(move |e| {
            if let Some(p) = w.upgrade() {
                p.commit_title();
                // As the spike's blur: the keys leave it.
                if let Some(r) = e.root() {
                    r.set_focus(None::<&gtk::Widget>);
                }
            }
        });
        let f = gtk::EventControllerFocus::new();
        let w = self.me.clone();
        f.connect_leave(move |_| {
            if let Some(p) = w.upgrade() {
                p.commit_title();
            }
        });
        self.title.add_controller(f);

        // New tag: a name of your own, in a small dialog by it.
        let w = self.me.clone();
        self.new_tag.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.ask_tag();
            }
        });
        let w = self.me.clone();
        self.ask_entry.connect_activate(move |_| {
            if let Some(p) = w.upgrade() {
                p.make_tag();
            }
        });
        let w = self.me.clone();
        self.ask_make.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.make_tag();
            }
        });
        let a = self.ask.downgrade();
        ask_cancel.connect_clicked(move |_| a.upgrade().iter().for_each(|a| a.popdown()));

        // A folder, file or repository: its menu.
        let w = self.me.clone();
        self.add_files.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.files_menu(false);
            }
        });

        // Where the pointer last moved over it (kept when it leaves: a
        // tooltip coming up under it is a leave).
        let m = gtk::EventControllerMotion::new();
        let p = self.pointer.clone();
        m.connect_motion(move |_, x, y| p.set(Some((x, y))));
        self.root.add_controller(m);
    }

    // ------------------------------------------------------------ shown

    /// The selected wadspace, as it is now.
    pub fn show(&self, x: &Wadspace, editable: bool) {
        let Some(b) = self.builder.upgrade() else { return };
        let was = self.ws.borrow().clone();
        let other = was.as_ref().is_none_or(|y| y.id != x.id);
        // Another one: what was typed into the last one's title is its name.
        if other && let Some(y) = &was {
            self.commit_title_of(y);
            self.ask.popdown();
        }
        *self.ws.borrow_mut() = Some(x.clone());
        self.editable.set(editable);

        // Its title: what's typed stays while it's typed.
        self.title.set_editable(editable);
        let typing = self.title.state_flags().contains(gtk::StateFlags::FOCUS_WITHIN);
        if (other || !typing) && self.title.text() != x.name {
            self.title.set_text(&x.name);
        }
        if !typing {
            // Sent and settled (or refused): the same name can be sent again.
            self.renamed.replace(None);
        }
        if b.take_title_focus() {
            self.focus_title();
        }

        self.show_tags(&b, x, editable);
        self.show_projects(x, editable);
        self.background.set(Some(&x.look.wallpaper), editable);
        self.icon.show(x, editable);
    }

    /// Ready to type over (a new one).
    fn focus_title(&self) {
        let t = self.title.clone();
        let go = move |t: &gtk::Entry| {
            t.grab_focus();
            t.select_region(0, -1);
        };
        go(&t);
        // The window may be on its way up: again once it's drawn.
        glib::idle_add_local_once(move || {
            if !t.state_flags().contains(gtk::StateFlags::FOCUS_WITHIN) {
                go(&t);
            }
        });
    }

    fn show_tags(&self, b: &Builder, x: &Wadspace, editable: bool) {
        let tags = b.shell.store.library().tags.clone();
        let mut want: Vec<Chip> = std::iter::once(general()).chain(tags.iter().cloned()).map(Chip::Tag).collect();
        want.extend(
            SUGGESTED.into_iter().filter(|n| !tags.iter().any(|t| t.name.eq_ignore_ascii_case(n))).map(Chip::Suggest),
        );
        let same = self.chips.borrow().iter().map(|(c, _)| c).eq(want.iter());
        if !same {
            for (_, button) in self.chips.borrow_mut().drain(..) {
                button.unparent();
            }
            let mut chips = vec![];
            for c in want {
                let button = self.chip(&c);
                self.tags.insert_before(&button, &self.new_tag);
                chips.push((c, button));
            }
            *self.chips.borrow_mut() = chips;
        }
        let current = tag_of(&tags, x);
        for (c, button) in self.chips.borrow().iter() {
            let on = matches!(c, Chip::Tag(t) if t.id == current);
            if on {
                button.add_css_class("on");
            } else {
                button.remove_css_class("on");
            }
            button.set_sensitive(editable);
            if let Chip::Suggest(n) = c {
                button.set_tooltip_text(Some(&format!("A new tag, {n}, with {} in it", x.name)));
            }
        }
        self.new_tag.set_sensitive(editable);
    }

    fn chip(&self, c: &Chip) -> gtk::Button {
        let button = match c {
            Chip::Tag(t) => {
                let b = chip_button(&t.name, Some(&t.color), false);
                b.set_tooltip_text(Some(&if t.id == GENERAL_ID {
                    "General: loose on the desktop".to_string()
                } else {
                    format!("{}: in its folder on the desktop", t.name)
                }));
                b
            }
            Chip::Suggest(n) => chip_button(n, None, true),
        };
        let (w, c) = (self.me.clone(), c.clone());
        button.connect_clicked(move |_| {
            let Some(p) = w.upgrade() else { return };
            match &c {
                Chip::Tag(t) => {
                    let tag = (t.id != GENERAL_ID).then(|| t.id.clone());
                    p.edit(|ws| Op::SetTag { ws, tag });
                }
                Chip::Suggest(n) => p.tag_as(n),
            }
        });
        button
    }

    fn show_projects(&self, x: &Wadspace, editable: bool) {
        let want = &x.look.projects;
        let same = {
            let rows = self.project_rows.borrow();
            rows.len() == want.len() && rows.iter().zip(want).all(|((p, _), q)| p == q)
        };
        if !same {
            for (_, row) in self.project_rows.borrow_mut().drain(..) {
                self.projects.remove(&row);
            }
            let mut rows = vec![];
            for p in want {
                let row = self.project_row(p);
                self.projects.insert_child_after(&row, rows.last().map(|(_, r): &(Project, gtk::Box)| r));
                rows.push((p.clone(), row));
            }
            *self.project_rows.borrow_mut() = rows;
        }
        for (_, row) in self.project_rows.borrow().iter() {
            if let Some(x) = row.last_child() {
                x.set_visible(editable);
            }
        }
        self.add_files.set_sensitive(editable);
    }

    /// The projects button's menu ([`files`]): this machine's folders now,
    /// the repositories and the other machines once wadd says (asked each
    /// time it opens). `by_ctl`: wadsctl's (it doesn't grab).
    fn files_menu(&self, by_ctl: bool) {
        let Some(b) = self.builder.upgrade() else { return };
        let Some(x) = self.ws.borrow().clone() else { return };
        if !self.editable.get() {
            return;
        }
        let folders = files::folders(&glib::home_dir(), files::MOST_FOLDERS);
        let (bw, anchor) = (Rc::downgrade(&b), self.add_files.clone());
        let known = b.shell.store.peers();
        b.shell.link.call(
            |w| async move { Ok((w.github_repos().await.map_err(|e| e.message), w.peers().await.ok())) },
            move |r| {
                let Some(b) = bw.upgrade() else { return };
                let (repos, peers) = r.unwrap_or_else(|e| (Err(e.message), None));
                let here = b.shell.store.machine_id();
                let choices = files::choices(&x, &here, &folders, &repos, &peers.unwrap_or(known));
                let open = || {
                    let bw = Rc::downgrade(&b);
                    let a: gtk::Widget = anchor.clone().upcast();
                    let at = a.clone();
                    kit::menu(&a, None, "add-files", choices, move |pick| {
                        if let Some(b) = bw.upgrade() {
                            kit::act(&b, &at, &pick.act);
                        }
                    });
                };
                if by_ctl { kit::by_ctl(open) } else { open() }
            },
        );
    }

    fn project_row(&self, p: &Project) -> gtk::Box {
        let row = gtk::Box::builder().spacing(8).css_classes(["cz-project"]).build();
        row.set_widget_name(&format!("project:{}", p.id));
        row.append(&wad_gtk::Icon::new(project_glyph(p), 15));
        row.append(&gtk::Label::builder().label(&p.as_name).css_classes(["cz-pn"]).build());
        let kind = match &p.kind {
            ProjectKind::Github { repo, .. } => repo.clone(),
            ProjectKind::Mount { .. } => "mounted".into(),
            ProjectKind::Copy { .. } => "copied in".into(),
        };
        row.append(
            &gtk::Label::builder()
                .label(&kind)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["cz-pk"])
                .build(),
        );
        let x = gtk::Button::builder()
            .child(&wad_gtk::Icon::new("x", 13))
            .css_classes(["cz-project-x"])
            .tooltip_text(format!("Take {} out", p.as_name))
            .build();
        let (w, id) = (self.me.clone(), p.id.clone());
        x.connect_clicked(move |_| {
            if let Some(panel) = w.upgrade() {
                let project = id.clone();
                panel.edit(|ws| Op::RemoveProject { ws, project });
            }
        });
        row.append(&x);
        row
    }

    // ------------------------------------------------------------ edits

    /// An edit of the one shown, if it can change.
    fn edit(&self, op: impl FnOnce(String) -> Op) {
        let Some(id) = self.ws.borrow().as_ref().map(|x| x.id.clone()) else { return };
        if !self.editable.get() {
            return;
        }
        if let Some(b) = self.builder.upgrade() {
            b.shell.op(op(id), |_| {});
        }
    }

    fn say(&self, text: &str) {
        if let Some(b) = self.builder.upgrade() {
            b.say(Level::Warn, text);
        }
    }

    fn commit_title(&self) {
        let x = self.ws.borrow().clone();
        if let Some(x) = x {
            self.commit_title_of(&x);
        }
    }

    /// What's typed is `x`'s name (nothing typed: its name back).
    fn commit_title_of(&self, x: &Wadspace) {
        let text = self.title.text().trim().to_string();
        if text.is_empty() || !self.editable.get() {
            self.title.set_text(&x.name);
            return;
        }
        let ask = (x.id.clone(), text.clone());
        if text == x.name || self.renamed.borrow().as_ref() == Some(&ask) {
            return;
        }
        *self.renamed.borrow_mut() = Some(ask);
        if let Some(b) = self.builder.upgrade() {
            b.shell.op(Op::Rename { ws: x.id.clone(), name: text }, |_| {});
        }
    }

    /// Into the tag of that name: one there already, or a new one (with it in).
    fn tag_as(&self, name: &str) {
        let n = name.trim();
        if n.is_empty() {
            return;
        }
        let Some(b) = self.builder.upgrade() else { return };
        let have = {
            let lib = b.shell.store.library();
            std::iter::once(general()).chain(lib.tags.iter().cloned()).find(|t| t.name.eq_ignore_ascii_case(n))
        };
        match have {
            Some(t) => {
                let tag = (t.id != GENERAL_ID).then_some(t.id);
                self.edit(|ws| Op::SetTag { ws, tag });
            }
            None => {
                let name = n.to_string();
                self.edit(|ws| Op::CreateTag { name, ws: Some(ws) });
            }
        }
    }

    fn ask_tag(&self) {
        if !self.editable.get() {
            return;
        }
        if let Some(p) = self.new_tag.compute_point(&self.tags, &graphene::Point::new(0.0, 0.0)) {
            let (w, h) = (self.new_tag.width(), self.new_tag.height());
            self.ask.set_pointing_to(Some(&gdk::Rectangle::new(p.x() as i32, p.y() as i32, w, h)));
        }
        self.ask_entry.set_text("");
        self.ask.popup();
        self.ask_entry.grab_focus();
    }

    fn make_tag(&self) {
        let name = self.ask_entry.text().to_string();
        self.ask.popdown();
        self.tag_as(&name);
    }

    // ------------------------------------------------------------ wadsctl

    /// `wadsctl custom` comes to the panel of the window open now. It's
    /// wired once, after whatever's running (a panel's made while the
    /// window is, which may be inside a `wadsctl builder …`).
    fn ctl(&self) {
        let Some(b) = self.builder.upgrade() else { return };
        LIVE.with(|l| *l.borrow_mut() = self.me.clone());
        if WIRED.replace(true) {
            return;
        }
        let shell = b.shell.clone();
        glib::idle_add_local_once(move || {
            shell.on_ctl("custom", |args| match LIVE.with(|l| l.borrow().upgrade()) {
                Some(p) => p.answer(args),
                None => "error the Builder isn't open".into(),
            });
        });
    }

    /// `wadsctl custom …`:
    ///
    /// - (nothing): what's shown: the title (and whether it's being typed),
    ///   the tag chips (the one lit), the backgrounds (the one lit), the
    ///   projects, and the icon's source, square and zoom;
    /// - `where <id>`: a thing's middle on the screen (scrolled into view):
    ///   `title`, `tag:<id>`, `suggest:<Name>`, `new-tag`, `ask-entry`,
    ///   `ask-make`, `project:<id>`, `project-remove:<id>`, `add-files`,
    ///   `upload`, `bg:<key>` (`empty`, `retro`, `photo:<file>`),
    ///   `icon-source:background|own`, `icon-upload`, `icon-bg:<key>`,
    ///   `crop`, `frame` (the icon's square), `zoom` (its knob), `reset`;
    /// - `pick <key>`, `icon-pick <key>`, `tag <id>` (`general`, a tag's id,
    ///   `suggest:Work`): clicked; `new-tag [name…]`: the dialog (and, with a
    ///   name, typed in and made); `icon-source background|own`;
    ///   `zoom <1-3>`; `reset`; `remove-project <id>`; `add-files` (the
    ///   projects button's menu, then `wadsctl side open` / `side choose <id>`);
    /// - `origin`: where the window's top left is taken to be;
    ///   `calibrate <x> <y>`: the pointer's at (x, y) over the panel: where
    ///   the window is, for `where` (otherwise it's where labwc's region
    ///   puts it, wrong once it's been moved);
    /// - `upload <path> [icon]`: a picture into ~/Pictures for the
    ///   background (or the icon's own), as the file dialog's answer.
    fn answer(&self, args: &[&str]) -> String {
        let Some(b) = self.builder.upgrade() else { return "error gone".into() };
        if !b.is_open() || self.ws.borrow().is_none() {
            return "error the Customize tab isn't showing (wadsctl builder tab customize)".into();
        }
        let ok = |r: Result<(), String>| match r {
            Ok(()) => "ok".to_string(),
            Err(e) => format!("error {e}"),
        };
        match args {
            [] => self.describe(),
            ["origin"] => {
                let Some(window) = self.root.root().and_downcast::<gtk::Window>() else { return "error no window".into() };
                let (x, y) = self.origin.get().unwrap_or_else(|| window_origin(&window, b.shell.size()));
                let how = if self.origin.get().is_some() { "calibrated" } else { "region" };
                format!("ok {x:.0} {y:.0} {how} window={}x{}", window.width(), window.height())
            }
            ["where", id @ ..] if !id.is_empty() => match self.spot(&id.join(" ")) {
                Some((w, x, y)) => match self.on_screen(&w, x, y) {
                    Some((sx, sy)) => format!("ok {sx:.0} {sy:.0}"),
                    None => format!("error {} isn't showing", id.join(" ")),
                },
                None => format!("error no {}", id.join(" ")),
            },
            ["pick", key @ ..] if !key.is_empty() => ok(self.background.pick(&key.join(" "))),
            ["icon-pick", key @ ..] if !key.is_empty() => ok(self.icon.picker().pick(&key.join(" "))),
            ["tag", id] => {
                let key = if id.starts_with("suggest:") { id.to_string() } else { format!("tag:{id}") };
                let button = self.chips.borrow().iter().find(|(c, _)| c.key() == key).map(|(_, b)| b.clone());
                ok(match button {
                    Some(b) if b.is_sensitive() => {
                        b.emit_clicked();
                        Ok(())
                    }
                    Some(_) => Err(format!("{id} can't be picked here")),
                    None => Err(format!("no tag {id}")),
                })
            }
            ["new-tag"] => {
                self.new_tag.emit_clicked();
                ok(if self.ask.is_visible() { Ok(()) } else { Err("it can't change here".into()) })
            }
            ["new-tag", name @ ..] => {
                self.new_tag.emit_clicked();
                if !self.ask.is_visible() {
                    return "error it can't change here".into();
                }
                self.ask_entry.set_text(&name.join(" "));
                self.ask_entry.emit_activate();
                "ok".into()
            }
            ["icon-source", s @ ("background" | "own")] => ok(self.icon.source(*s == "own")),
            ["zoom", z] => match z.parse::<f64>() {
                Ok(z) if self.editable.get() => {
                    self.icon.set_zoom(z);
                    "ok".into()
                }
                Ok(_) => "error it can't change here".into(),
                Err(_) => "error zoom <1-3>".into(),
            },
            ["reset"] => {
                self.icon.do_reset();
                "ok".into()
            }
            ["add-files"] if self.editable.get() => {
                self.files_menu(true);
                "ok".into()
            }
            ["remove-project", id] => {
                let row = self.project_rows.borrow().iter().find(|(p, _)| p.id == *id).map(|(_, r)| r.clone());
                match row.and_then(|r| r.last_child()).and_downcast::<gtk::Button>() {
                    Some(x) if x.is_visible() => {
                        x.emit_clicked();
                        "ok".into()
                    }
                    Some(_) => "error it can't change here".into(),
                    None => format!("error no project {id}"),
                }
            }
            ["calibrate", x, y] => {
                let (Ok(x), Ok(y)) = (x.parse::<f64>(), y.parse::<f64>()) else { return "error calibrate <x> <y>".into() };
                match self.calibrate(x, y) {
                    Some((ox, oy)) => format!("ok {ox:.0} {oy:.0}"),
                    None => "error the pointer hasn't moved over the panel (move it there first)".into(),
                }
            }
            ["upload", rest @ ..] if !rest.is_empty() => {
                let (words, icon) = match rest {
                    [p @ .., "icon"] if !p.is_empty() => (p, true),
                    p => (p, false),
                };
                let path = PathBuf::from(words.join(" "));
                if !path.is_file() {
                    return format!("error no file {}", path.display());
                }
                if !self.editable.get() {
                    return "error it can't change here".into();
                }
                if icon {
                    self.icon.picker().upload(path);
                } else {
                    self.background.upload(path);
                }
                "ok".into()
            }
            _ => "error custom [where <id>|pick <key>|icon-pick <key>|tag <id>|new-tag [name]|icon-source background|own|zoom <z>|reset|add-files|remove-project <id>|upload <path> [icon]]".into(),
        }
    }

    fn describe(&self) -> String {
        let x = self.ws.borrow().clone();
        let Some(x) = x else { return "error nothing shown".into() };
        let chips = self.chips.borrow();
        let tag = chips
            .iter()
            .find(|(_, b)| b.has_css_class("on"))
            .map(|(c, _)| c.key().trim_start_matches("tag:").to_string())
            .unwrap_or_else(|| "-".into());
        let tags: Vec<String> =
            chips.iter().filter_map(|(c, _)| if let Chip::Tag(t) = c { Some(t.id.clone()) } else { None }).collect();
        let suggested: Vec<&str> =
            chips.iter().filter_map(|(c, _)| if let Chip::Suggest(n) = c { Some(*n) } else { None }).collect();
        let (start, end) = self.title.selection_bounds().unwrap_or((0, 0));
        format!(
            "ok ws={} title={:?} typing={} selected={} editable={} tag={} tags={} suggested={} asking={} projects={} background={:?} backgrounds={:?} {}",
            x.id,
            self.title.text().as_str(),
            self.title.state_flags().contains(gtk::StateFlags::FOCUS_WITHIN),
            end - start,
            self.editable.get(),
            tag,
            tags.join(","),
            suggested.join(","),
            self.ask.is_visible(),
            self.project_rows.borrow().iter().map(|(p, _)| p.id.clone()).collect::<Vec<_>>().join(","),
            self.background.lit().unwrap_or_else(|| "-".into()),
            self.background.keys().join(","),
            self.icon.describe(),
        )
    }

    /// A thing by id, and the point on it to aim at.
    fn spot(&self, id: &str) -> Option<(gtk::Widget, f64, f64)> {
        let mid = |w: &gtk::Widget| Some((w.clone(), f64::from(w.width()) / 2.0, f64::from(w.height()) / 2.0));
        match id {
            "title" => mid(self.title.upcast_ref()),
            "new-tag" => mid(self.new_tag.upcast_ref()),
            "ask-entry" => mid(self.ask_entry.upcast_ref()),
            "ask-make" => mid(self.ask_make.upcast_ref()),
            "add-files" => mid(self.add_files.upcast_ref()),
            "upload" => mid(self.background.upload_button().upcast_ref()),
            _ => {
                if let Some(t) = id
                    .strip_prefix("tag:")
                    .map(|t| format!("tag:{t}"))
                    .or_else(|| id.starts_with("suggest:").then(|| id.to_string()))
                {
                    let chips = self.chips.borrow();
                    let b = chips.iter().find(|(c, _)| c.key() == t).map(|(_, b)| b.clone())?;
                    return mid(b.upcast_ref());
                }
                if let Some(key) = id.strip_prefix("bg:") {
                    return mid(self.background.find(key)?.upcast_ref());
                }
                if let Some(pid) = id.strip_prefix("project-remove:") {
                    let rows = self.project_rows.borrow();
                    let r = rows.iter().find(|(p, _)| p.id == pid).map(|(_, r)| r.clone())?;
                    return mid(&r.last_child()?);
                }
                if let Some(pid) = id.strip_prefix("project:") {
                    let rows = self.project_rows.borrow();
                    let r = rows.iter().find(|(p, _)| p.id == pid).map(|(_, r)| r.clone())?;
                    return mid(r.upcast_ref());
                }
                self.icon.spot(id)
            }
        }
    }

    /// The pointer is at (`x`, `y`) on the screen, over the panel: where
    /// the window is, from where in it the pointer is (Wayland never says).
    fn calibrate(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let (lx, ly) = self.pointer.get()?;
        let window = self.root.root()?.downcast::<gtk::Window>().ok()?;
        let p = self.root.compute_point(&window, &graphene::Point::new(lx as f32, ly as f32))?;
        let o = (x - f64::from(p.x()), y - f64::from(p.y()));
        self.origin.set(Some(o));
        Some(o)
    }

    /// A point of `w` on the screen, in logical pixels: the panel scrolled
    /// so it shows first. (A popover's own surface sits under where it points.)
    fn on_screen(&self, w: &gtk::Widget, x: f64, y: f64) -> Option<(f64, f64)> {
        let window = self.root.root()?.downcast::<gtk::Window>().ok()?;
        let pt = graphene::Point::new(x as f32, y as f32);
        let size = self.builder.upgrade()?.shell.size();
        let (ox, oy) = self.origin.get().unwrap_or_else(|| window_origin(&window, size));
        if w.is_ancestor(&self.ask) || *w == *self.ask.upcast_ref::<gtk::Widget>() {
            if !self.ask.is_visible() {
                return None;
            }
            // In the dialog: a surface of its own, where the compositor put it
            // against the window's (each past its own shadow).
            let p = w.compute_point(&self.ask, &pt)?;
            let popup = self.ask.surface()?.downcast::<gdk::Popup>().ok()?;
            let (ax, ay) = self.ask.surface_transform();
            let (wx, wy) = window.surface_transform();
            let x = ox - wx + f64::from(popup.position_x()) + ax + f64::from(p.x());
            let y = oy - wy + f64::from(popup.position_y()) + ay + f64::from(p.y());
            return Some((x, y));
        }
        if !w.is_drawable() {
            return None;
        }
        // Where it's laid out, less any scroll not laid out yet (the panel's
        // box sits at minus the scroll in its viewport once it is).
        let scroll = self.root.ancestor(gtk::ScrolledWindow::static_type()).and_downcast::<gtk::ScrolledWindow>()?;
        let viewport = self.root.parent()?;
        let adj = scroll.vadjustment();
        let pending = || -> Option<f64> { Some(adj.value() + f64::from(self.root.compute_bounds(&viewport)?.y())) };
        let p = w.compute_point(&window, &pt)?;
        let view = scroll.compute_bounds(&window)?;
        let (top, bottom) = (f64::from(view.y()), f64::from(view.y() + view.height()));
        let mut y = f64::from(p.y()) - pending()?;
        // Scrolled so it shows, if it doesn't.
        if y < top + 24.0 || y > bottom - 24.0 {
            let to = (adj.value() + y - (top + bottom) / 2.0)
                .clamp(adj.lower(), (adj.upper() - adj.page_size()).max(adj.lower()));
            adj.set_value(to);
            y = f64::from(p.y()) - pending()?;
        }
        Some((ox + f64::from(p.x()), oy + y))
    }
}

/// The Builder's window's top left on the screen. Wayland doesn't tell a
/// window where it is: this is where labwc puts it, its `builder` region
/// (host/etc/xdg/labwc/rc.xml: the right 74% of what's under the bar, to the
/// bottom, inside its 10 px gap), or the whole of that maximized. Either way
/// it ends at that area's bottom right, so it's found from the window's size
/// (the bar may not have been up yet when it was placed: then it's taller).
/// `wadsctl custom calibrate` finds it for a window that's been moved.
fn window_origin(window: &gtk::Window, screen: (i32, i32)) -> (f64, f64) {
    region_origin(screen, (window.width(), window.height()))
}

fn region_origin((sw, sh): (i32, i32), (w, h): (i32, i32)) -> (f64, f64) {
    const GAP: i32 = 10;
    (f64::from(sw - GAP - w), f64::from(sh - GAP - h))
}

/// A section's heading (the spike's `h5`: small capitals, faint).
fn heading(text: &str, first: bool) -> gtk::Label {
    let l = gtk::Label::builder().label(text.to_uppercase()).xalign(0.0).css_classes(["cz-h"]).build();
    if first {
        l.add_css_class("first");
    }
    l
}

/// A tag's chip: its colour's dot and its name, or (dashed) a plus and a name to make.
fn chip_button(name: &str, color: Option<&str>, dashed: bool) -> gtk::Button {
    let b = gtk::Button::builder().css_classes(["cz-chip"]).build();
    let row = gtk::Box::builder().spacing(6).build();
    match color.and_then(|c| gdk::RGBA::parse(c).ok()) {
        Some(c) => {
            let dot = gtk::DrawingArea::builder().content_width(8).content_height(8).valign(gtk::Align::Center).build();
            dot.set_draw_func(move |_, cr, w, h| {
                let r = f64::from(w.min(h)) / 2.0;
                cr.arc(f64::from(w) / 2.0, f64::from(h) / 2.0, r, 0.0, std::f64::consts::TAU);
                cr.set_source_rgba(c.red().into(), c.green().into(), c.blue().into(), c.alpha().into());
                let _ = cr.fill();
            });
            row.append(&dot);
        }
        None if dashed => row.append(&wad_gtk::Icon::new("plus", 12)),
        None => {}
    }
    row.append(&gtk::Label::new(Some(name)));
    b.set_child(Some(&row));
    if dashed {
        b.add_css_class("suggest");
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builder_sits_in_labwcs_region() {
        // 1500 x 1000 (the Surface Book 2 at 200%): 26% into what's left of the 10 px gaps, under the bar.
        // 1500 x 1000 (the Surface Book 2 at 200%): 74% of what's inside the
        // 10 px gaps, under the 78 px bar…
        assert_eq!(region_origin((1500, 1000), (1095, 902)), (395.0, 88.0));
        // …or placed before the bar was up…
        assert_eq!(region_origin((1500, 1000), (1095, 980)), (395.0, 10.0));
        // …or maximized.
        assert_eq!(region_origin((1500, 1000), (1480, 902)), (10.0, 88.0));
    }

    #[test]
    fn chips_are_keyed_by_what_they_are() {
        assert_eq!(Chip::Tag(general()).key(), "tag:general");
        assert_eq!(Chip::Suggest("Work").key(), "suggest:Work");
    }
}
