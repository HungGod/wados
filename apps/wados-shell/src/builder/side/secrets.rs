//! The sidebar's Secrets tab (the spike's `SideSecrets.svelte`; its choices
//! are [`keys`]'s, `keys.ts`): who each app signs in as. WadBrowser's web
//! apps in a group per key, each with its key's dropdown (another key, none,
//! a new one: the group follows), and **Drag apps here for a key of their
//! own**. A click picks a web app (again lets it go; its menu's a
//! right-click), so several can be picked; drag any picked one and they all
//! go, onto another group's key or a key of their own, together (how many
//! rides under the pointer). A drop anywhere else does nothing. Every other
//! app that signs in (VS Code's settings, Claude Code, a vault, a Steam
//! account) is a line with its key's dropdown.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use wad_model::catalog::app_name;
use wad_model::edit::Payload;
use wad_model::folders::{Folder, folders_of};
use wad_model::variants::{AppVariant, VariantKind, find_variant};
use wad_model::{AppId, Section, Wadspace};

use super::ctl::{self, Row};
use super::keys::{self, Choice, Pick, folder_id};
use super::kit;
use crate::builder::{Builder, Tab};
use crate::dnd::{self, Held, Verdict};
use wad_gtk::Wrap;

/// The apps with a line each (WadBrowser has its groups).
const LINED: [AppId; 3] = [AppId::Vscode, AppId::Obsidian, AppId::Steam];

/// A WadBrowser group: its web apps, and its key's dropdown.
struct GroupCard {
    card: gtk::Box,
    pills: Wrap,
    select: gtk::Button,
    select_label: gtk::Label,
}

/// WadBrowser's part: its groups, what's picked, and a key of their own.
struct Browser {
    root: gtk::Box,
    list: gtk::Box,
    groups: RefCell<Vec<(String, GroupCard)>>,
    picked_bar: gtk::Box,
    picked_label: gtk::Label,
    unpick: gtk::Button,
    own: gtk::Box,
}

/// One thing that signs in on its own: its name, and its key's dropdown.
struct LineCard {
    card: gtk::Box,
    label: gtk::Label,
    select: gtk::Button,
    select_label: gtk::Label,
}

/// Another app's part: a line per thing of its that signs in.
struct Lines {
    root: gtk::Box,
    list: gtk::Box,
    lines: RefCell<Vec<(String, LineCard)>>,
}

pub struct Panel {
    builder: Weak<Builder>,
    me: Weak<Panel>,
    root: gtk::Box,
    browser: Browser,
    others: Vec<(AppId, Lines)>,
    empty: gtk::Label,
    no_key: gtk::Label,
    /// The wadspace shown, and whether it can be changed here.
    now: RefCell<Option<Wadspace>>,
    editable: Cell<bool>,
    /// The web apps picked (their variants' ids), in the order they were.
    picked: RefCell<Vec<String>>,
}

impl Panel {
    pub fn new(b: &Rc<Builder>) -> Rc<Self> {
        kit::install_css();
        let me = Rc::new_cyclic(|me: &Weak<Panel>| {
            let root =
                gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["side-secrets"]).build();
            let browser = browser(me);
            root.append(&browser.root);
            let mut others = vec![];
            for app in LINED {
                let r = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
                r.append(&app_head(app));
                let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
                r.append(&list);
                root.append(&r);
                others.push((app, Lines { root: r, list, lines: RefCell::default() }));
            }
            let empty = note("Nothing in it signs in yet: WadBrowser, VS Code, Obsidian and Steam do.");
            let no_key = note("No key: that’s signed out when the wadspace stops.");
            root.append(&empty);
            root.append(&no_key);
            Panel {
                builder: Rc::downgrade(b),
                me: me.clone(),
                root,
                browser,
                others,
                empty,
                no_key,
                now: RefCell::default(),
                editable: Cell::new(false),
                picked: RefCell::default(),
            }
        });
        let rows: Weak<dyn ctl::Rows> = Rc::downgrade(&me) as Weak<dyn ctl::Rows>;
        ctl::register(b, Tab::Secrets, rows);
        me
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// The selected wadspace, as it is now.
    pub fn show(&self, x: &Wadspace, editable: bool) {
        let other = self.now.borrow().as_ref().is_none_or(|y| y.id != x.id);
        if other {
            self.picked.borrow_mut().clear();
            kit::close_menu();
        }
        *self.now.borrow_mut() = Some(x.clone());
        self.editable.set(editable);
        let apps = keys::secret_apps(x);

        // WadBrowser's groups.
        let has_browser = apps.contains(&AppId::Wadbrowser);
        self.browser.root.set_visible(has_browser);
        let folders = if has_browser { folders_of(x, AppId::Wadbrowser) } else { vec![] };
        let webs: Vec<String> = folders
            .iter()
            .flat_map(|f| &f.items)
            .filter(|v| v.kind == VariantKind::Web)
            .map(|v| v.id.clone())
            .collect();
        self.picked.borrow_mut().retain(|id| webs.contains(id));
        self.sync_groups(&folders, editable);
        self.paint_picks();
        self.browser.own.set_visible(editable);

        // Every other app's lines.
        for (app, part) in &self.others {
            let on = apps.contains(app);
            part.root.set_visible(on);
            if on {
                self.sync_lines(x, *app, part, editable);
            }
        }

        // In its apps' order.
        let mut prev: Option<gtk::Widget> = None;
        for app in &apps {
            let r = if *app == AppId::Wadbrowser {
                Some(self.browser.root.clone())
            } else {
                self.others.iter().find(|(a, _)| a == app).map(|(_, p)| p.root.clone())
            };
            if let Some(r) = r {
                self.root.reorder_child_after(&r, prev.as_ref());
                if let Some(h) = r.first_child() {
                    if prev.is_none() {
                        h.add_css_class("first");
                    } else {
                        h.remove_css_class("first");
                    }
                }
                prev = Some(r.upcast());
            }
        }
        self.empty.set_visible(apps.is_empty());
        self.no_key.set_visible(!apps.is_empty());
    }

    fn sync_groups(&self, folders: &[Folder], editable: bool) {
        let mut old: Vec<(String, GroupCard)> = self.browser.groups.borrow_mut().drain(..).collect();
        let mut now = vec![];
        for f in folders {
            let fid = folder_id(f);
            let g = match old.iter().position(|(k, _)| *k == fid) {
                Some(i) => old.remove(i).1,
                None => group_card(&self.me, &fid),
            };
            let items: Vec<(String, AppVariant)> = f.items.iter().map(|v| (v.id.clone(), v.clone())).collect();
            let me = self.me.clone();
            g.pills.set(
                &items,
                |v| pill(&me, v),
                |w, v| {
                    if let Some(l) = w.first_child().and_then(|h| h.last_child()).and_downcast::<gtk::Label>()
                        && l.label() != v.name
                    {
                        l.set_label(&v.name);
                    }
                },
            );
            match &f.key {
                Some(k) => {
                    g.select_label.set_label(&k.name);
                    g.select.remove_css_class("none");
                }
                None => {
                    g.select_label.set_label("No key");
                    g.select.add_css_class("none");
                }
            }
            g.select.set_sensitive(editable);
            g.select.set_tooltip_text(Some(&match &f.key {
                Some(k) => format!("They sign in with {}: another key, none, or a new one", k.name),
                None => "They sign in with no key: pick one, or a new one".to_string(),
            }));
            now.push((fid, g));
        }
        for (_, g) in old {
            self.browser.list.remove(&g.card);
        }
        let mut prev: Option<gtk::Widget> = None;
        for (_, g) in &now {
            if g.card.parent().is_none() {
                self.browser.list.insert_child_after(&g.card, prev.as_ref());
            } else {
                self.browser.list.reorder_child_after(&g.card, prev.as_ref());
            }
            prev = Some(g.card.clone().upcast());
        }
        *self.browser.groups.borrow_mut() = now;
    }

    fn sync_lines(&self, x: &Wadspace, app: AppId, part: &Lines, editable: bool) {
        let mut old: Vec<(String, LineCard)> = part.lines.borrow_mut().drain(..).collect();
        let mut now = vec![];
        for v in keys::lines(x, app) {
            let l = match old.iter().position(|(k, _)| *k == v.id) {
                Some(i) => old.remove(i).1,
                None => line_card(&self.me, &v.id),
            };
            l.label.set_label(&keys::line_label(&v));
            let (text, has) = keys::line_key(x, &v);
            l.select_label.set_label(&text);
            if has {
                l.select.remove_css_class("none");
            } else {
                l.select.add_css_class("none");
            }
            l.select.set_sensitive(editable);
            l.select.set_tooltip_text(Some(&format!("What {} signs in with", keys::line_label(&v))));
            now.push((v.id.clone(), l));
        }
        for (_, l) in old {
            part.list.remove(&l.card);
        }
        let mut prev: Option<gtk::Widget> = None;
        for (_, l) in &now {
            if l.card.parent().is_none() {
                part.list.insert_child_after(&l.card, prev.as_ref());
            } else {
                part.list.reorder_child_after(&l.card, prev.as_ref());
            }
            prev = Some(l.card.clone().upcast());
        }
        *part.lines.borrow_mut() = now;
    }

    /// The pills lit as they're picked, and the bar that says how many.
    fn paint_picks(&self) {
        let picked = self.picked.borrow();
        let x = self.now.borrow();
        for (_, g) in self.browser.groups.borrow().iter() {
            for (id, w) in g.pills.keyed() {
                let web = id.starts_with("ext:")
                    && x.as_ref().and_then(|x| find_variant(x, &id)).is_some_and(|v| v.kind == VariantKind::Web);
                if picked.contains(&id) {
                    w.add_css_class("picked");
                } else {
                    w.remove_css_class("picked");
                }
                let name = x.as_ref().and_then(|x| find_variant(x, &id)).map(|v| v.name).unwrap_or_default();
                w.set_tooltip_text(Some(&if web && self.editable.get() {
                    format!("{name}: click to pick it (with others, to drag them together); right-click for its menu")
                } else {
                    name
                }));
            }
        }
        let n = picked.len();
        self.browser.picked_bar.set_visible(n > 0 && self.editable.get());
        self.browser.picked_label.set_label(&format!("{n} picked: drag one to move them together"));
    }

    fn pick(&self, id: &str) {
        {
            let mut p = self.picked.borrow_mut();
            match p.iter().position(|x| x == id) {
                Some(i) => {
                    p.remove(i);
                }
                None => p.push(id.to_string()),
            }
        }
        self.paint_picks();
    }

    fn unpick(&self) {
        self.picked.borrow_mut().clear();
        self.paint_picks();
    }

    /// A click on a pill: a web app's picks it; anything else's opens its menu.
    fn pill_click(&self, id: &str, anchor: &gtk::Widget) {
        if !self.editable.get() {
            return;
        }
        let v = self.now.borrow().as_ref().and_then(|x| find_variant(x, id));
        match v {
            Some(v) if v.kind == VariantKind::Web => self.pick(id),
            Some(_) => self.pill_menu(id, anchor, None),
            None => {}
        }
    }

    /// A pill's own menu: start straight into it, or take it out.
    fn pill_menu(&self, id: &str, anchor: &gtk::Widget, at: Option<(f64, f64)>) {
        if !self.editable.get() {
            return;
        }
        let x = self.now.borrow().clone();
        let Some(x) = x else { return };
        let Some(v) = find_variant(&x, id) else { return };
        let choices = keys::thing_menu(&x, &v);
        self.open(anchor, at, &format!("pill:{id}"), choices);
    }

    /// A dropdown's choices under `anchor`; a choice is done as it says.
    fn open(&self, anchor: &gtk::Widget, at: Option<(f64, f64)>, of: &str, choices: Vec<Choice>) {
        let (wb, wa, wp) = (self.builder.clone(), anchor.downgrade(), self.me.clone());
        let own = of == "own-key";
        kit::menu(anchor, at, of, choices, move |p: &Pick| {
            let (Some(b), Some(a)) = (wb.upgrade(), wa.upgrade()) else { return };
            if own && let Some(panel) = wp.upgrade() {
                panel.unpick();
            }
            kit::act(&b, &a, &p.act);
        });
    }

    fn group_menu(&self, fid: &str, anchor: &gtk::Widget) {
        let (Some(b), Some(x)) = (self.builder.upgrade(), self.now.borrow().clone()) else { return };
        if !self.editable.get() {
            return;
        }
        let Some(f) = folders_of(&x, AppId::Wadbrowser).into_iter().find(|f| folder_id(f) == fid) else { return };
        let choices = keys::group_key_items(&b.shell.store.library(), &x, AppId::Wadbrowser, &f);
        self.open(anchor, None, &format!("key-select:{fid}"), choices);
    }

    fn line_menu(&self, vid: &str, anchor: &gtk::Widget) {
        let (Some(b), Some(x)) = (self.builder.upgrade(), self.now.borrow().clone()) else { return };
        if !self.editable.get() {
            return;
        }
        let Some(v) = find_variant(&x, vid) else { return };
        let choices = keys::thing_key_items(&b.shell.store.library(), &x, &v);
        self.open(anchor, None, &format!("key-select:{vid}"), choices);
    }

    /// What dragging a pill carries: the picked ones, if it's one of several picked; else it alone.
    fn drag_of(&self, id: &str) -> Option<Held> {
        let x = self.now.borrow().clone()?;
        let picked = self.picked.borrow();
        if picked.len() > 1 && picked.iter().any(|p| p == id) {
            let items = picked.iter().map(|p| p.trim_start_matches("ext:").to_string()).collect();
            return Some(Held::Pieces { section: Section::Addons, items, from: x.id.clone() });
        }
        let v = find_variant(&x, id)?;
        let payload = match (&v.kind, &v.ext) {
            (VariantKind::App, _) => {
                Payload::Piece { section: Section::Apps, item: v.app.as_str().to_string(), from: Some(x.id.clone()) }
            }
            (_, Some(e)) => Payload::Piece {
                section: Section::Addons,
                item: format!("{}:{}", e.app, e.id),
                from: Some(x.id.clone()),
            },
            _ => return None,
        };
        Some(Held::Part { payload })
    }

    /// The wadspace as it is now (a drop lands after the drag's done with).
    fn fresh(&self) -> Option<Wadspace> {
        let b = self.builder.upgrade()?;
        let id = self.now.borrow().as_ref()?.id.clone();
        b.wadspace(&id)
    }

    fn group_offer(&self, fid: &str, held: &Held) -> Option<Verdict> {
        if !self.editable.get() {
            return None;
        }
        let x = self.now.borrow().clone()?;
        keys::group_drop(&x, fid, held).and_then(|(_, hint)| Verdict::yes(hint))
    }

    fn group_land(&self, fid: &str, held: &Held) {
        let (Some(b), Some(x)) = (self.builder.upgrade(), self.fresh()) else { return };
        if let Some((ops, _)) = keys::group_drop(&x, fid, held) {
            self.unpick();
            kit::send_all(&b.shell, ops);
        }
    }

    fn own_offer(&self, held: &Held) -> Option<Verdict> {
        if !self.editable.get() {
            return None;
        }
        let x = self.now.borrow().clone()?;
        keys::own_drop(&x, held).and_then(|(_, hint)| Verdict::yes(hint))
    }

    /// Web apps let go on "a key of their own": which key they share (a saved one no group has, or a new one).
    fn own_land(&self, held: &Held, at: (f64, f64)) {
        let (Some(b), Some(x)) = (self.builder.upgrade(), self.fresh()) else { return };
        let Some((vs, _)) = keys::own_drop(&x, held) else { return };
        let choices = keys::own_key_items(&b.shell.store.library(), &x, &vs);
        let own = self.browser.own.clone().upcast::<gtk::Widget>();
        self.open(&own, Some(at), "own-key", choices);
    }
}

impl ctl::Rows for Panel {
    fn rows(&self) -> Vec<Row> {
        let mut out = vec![];
        if self.browser.root.is_visible() {
            let picked = self.picked.borrow();
            for (fid, g) in self.browser.groups.borrow().iter() {
                out.push(Row::new(format!("group:{fid}"), &g.card, fid.clone()).lit(g.card.has_css_class("hot")));
                for (id, w) in g.pills.keyed() {
                    let menu: Rc<dyn Fn()> = {
                        let (me, id, w) = (self.me.clone(), id.clone(), w.downgrade());
                        Rc::new(move || {
                            if let (Some(p), Some(w)) = (me.upgrade(), w.upgrade()) {
                                p.pill_menu(&id, &w, None);
                            }
                        })
                    };
                    let name = pill_name(&w);
                    out.push(Row::new(format!("pill:{id}"), &w, name).picked(picked.contains(&id)).menu(menu));
                }
                out.push(
                    Row::new(format!("key-select:{fid}"), &g.select, g.select_label.label())
                        .lit(!g.select.has_css_class("none")),
                );
            }
            if self.browser.picked_bar.is_visible() {
                out.push(Row::new("unpick", &self.browser.unpick, self.browser.picked_label.label()));
            }
            if self.browser.own.is_visible() {
                out.push(Row::new("own-key", &self.browser.own, "Drag apps here for a key of their own"));
            }
        }
        for (_, part) in &self.others {
            if !part.root.is_visible() {
                continue;
            }
            for (vid, l) in part.lines.borrow().iter() {
                let label = format!("{}: {}", l.label.label(), l.select_label.label());
                out.push(Row::new(format!("key-select:{vid}"), &l.select, label).lit(!l.select.has_css_class("none")));
            }
        }
        out
    }
}

// ------------------------------------------------------------ its widgets

fn note(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["side-note"]).build()
}

fn app_head(app: AppId) -> gtk::Box {
    let h = gtk::Box::builder().spacing(8).css_classes(["side-app-h"]).build();
    h.append(&wad_gtk::app_icon::app(app, 16));
    h.append(&gtk::Label::builder().label(app_name(app)).xalign(0.0).build());
    h
}

fn browser(me: &Weak<Panel>) -> Browser {
    let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    root.append(&app_head(AppId::Wadbrowser));
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    root.append(&list);

    let picked_bar = gtk::Box::builder().spacing(8).css_classes(["side-picked"]).visible(false).build();
    let picked_label = gtk::Label::builder().xalign(0.0).hexpand(true).wrap(true).build();
    picked_bar.append(&picked_label);
    let unpick = gtk::Button::builder().label("Clear").css_classes(["side-clear"]).valign(gtk::Align::Center).build();
    let w = me.clone();
    unpick.connect_clicked(move |_| {
        if let Some(p) = w.upgrade() {
            p.unpick();
        }
    });
    picked_bar.append(&unpick);
    root.append(&picked_bar);

    let own = gtk::Box::builder().css_classes(["side-own"]).build();
    own.set_widget_name("own-key");
    let inner = gtk::Box::builder().spacing(6).halign(gtk::Align::Center).hexpand(true).build();
    inner.append(&wad_gtk::Icon::new("plus", 14));
    inner.append(&gtk::Label::new(Some("Drag apps here for a key of their own")));
    own.append(&inner);
    let (wo, wl, wh) = (me.clone(), me.clone(), own.downgrade());
    dnd::target(
        &own,
        move |h, _, _| wo.upgrade()?.own_offer(h),
        move |h, x, y| {
            if let Some(p) = wl.upgrade() {
                p.own_land(h, (x, y));
            }
        },
        move |on| {
            if let Some(o) = wh.upgrade() {
                if on {
                    o.add_css_class("hot");
                } else {
                    o.remove_css_class("hot");
                }
            }
        },
    );
    root.append(&own);
    Browser { root, list, groups: RefCell::default(), picked_bar, picked_label, unpick, own }
}

/// A key's dropdown: the key, and a chevron.
fn select_button() -> (gtk::Button, gtk::Label) {
    let b = gtk::Button::builder().css_classes(["side-select"]).build();
    let h = gtk::Box::builder().spacing(8).build();
    h.append(&wad_gtk::Icon::new("key-round", 15));
    let l = gtk::Label::builder().xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build();
    h.append(&l);
    h.append(&wad_gtk::Icon::new("chevron-down", 15));
    b.set_child(Some(&h));
    (b, l)
}

fn group_card(me: &Weak<Panel>, fid: &str) -> GroupCard {
    let card =
        gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["side-group"]).build();
    card.set_widget_name(&format!("group:{fid}"));
    let pills = Wrap::new(6, 6);
    card.append(&pills);
    let (select, select_label) = select_button();
    select.set_widget_name(&format!("key-select:{fid}"));
    let (w, f) = (me.clone(), fid.to_string());
    select.connect_clicked(move |b| {
        if let Some(p) = w.upgrade() {
            p.group_menu(&f, b.upcast_ref());
        }
    });
    card.append(&select);
    let (wo, wl, fo, fl, wc) = (me.clone(), me.clone(), fid.to_string(), fid.to_string(), card.downgrade());
    dnd::target(
        &card,
        move |h, _, _| wo.upgrade()?.group_offer(&fo, h),
        move |h, _, _| {
            if let Some(p) = wl.upgrade() {
                p.group_land(&fl, h);
            }
        },
        move |on| {
            if let Some(c) = wc.upgrade() {
                if on {
                    c.add_css_class("hot");
                } else {
                    c.remove_css_class("hot");
                }
            }
        },
    );
    GroupCard { card, pills, select, select_label }
}

fn line_card(me: &Weak<Panel>, vid: &str) -> LineCard {
    let card =
        gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["side-line"]).build();
    let label = gtk::Label::builder().xalign(0.0).css_classes(["side-label"]).build();
    card.append(&label);
    let (select, select_label) = select_button();
    select.set_widget_name(&format!("key-select:{vid}"));
    let (w, v) = (me.clone(), vid.to_string());
    select.connect_clicked(move |b| {
        if let Some(p) = w.upgrade() {
            p.line_menu(&v, b.upcast_ref());
        }
    });
    card.append(&select);
    LineCard { card, label, select, select_label }
}

/// A thing's icon: the app's, a web app's site's, or a glyph for what it is.
fn thing_icon(v: &AppVariant, size: i32) -> gtk::Widget {
    match (&v.kind, &v.ext) {
        (VariantKind::App, _) => wad_gtk::app_icon::app(v.app, size),
        (VariantKind::Web, Some(e)) => wad_gtk::app_icon::site(&e.id, Some(&v.name), e.url.as_deref(), size),
        (VariantKind::Extension, Some(e)) if e.id == "anthropic.claude-code" => kit::image("/icons/claude.png", size),
        (kind, _) => {
            let glyph = match kind {
                VariantKind::Mode => "eye",
                VariantKind::Extension => "puzzle",
                VariantKind::Tool => "square-terminal",
                VariantKind::Game if v.app == AppId::Dolphin => "disc-3",
                VariantKind::Game | VariantKind::Named => "gamepad-2",
                _ => "key-round",
            };
            wad_gtk::Icon::new(glyph, size - 3).upcast()
        }
    }
}

/// A thing as a pill: its icon and name. A click picks a web app (or
/// opens anything else's menu); a right-click opens its menu; a drag
/// carries it (or the picked ones).
fn pill(me: &Weak<Panel>, v: &AppVariant) -> gtk::Widget {
    let b = gtk::Button::builder().css_classes(["side-thing"]).build();
    b.set_widget_name(&format!("pill:{}", v.id));
    let h = gtk::Box::builder().spacing(6).build();
    h.append(&thing_icon(v, 16));
    h.append(
        &gtk::Label::builder()
            .label(&v.name)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["side-name"])
            .build(),
    );
    b.set_child(Some(&h));
    let (w, id) = (me.clone(), v.id.clone());
    b.connect_clicked(move |b| {
        if let Some(p) = w.upgrade() {
            p.pill_click(&id, b.upcast_ref());
        }
    });
    let right = gtk::GestureClick::builder().button(3).build();
    let (w, id, at) = (me.clone(), v.id.clone(), b.downgrade());
    right.connect_pressed(move |g, _, x, y| {
        g.set_state(gtk::EventSequenceState::Claimed);
        if let (Some(p), Some(a)) = (w.upgrade(), at.upgrade()) {
            p.pill_menu(&id, a.upcast_ref(), Some((x, y)));
        }
    });
    b.add_controller(right);
    let (w, id, look) = (me.clone(), v.id.clone(), b.downgrade());
    dnd::source(&b, move |_, _| Some((w.upgrade()?.drag_of(&id)?, look.upgrade()?.upcast())));
    b.upcast()
}

fn pill_name(w: &gtk::Widget) -> String {
    w.first_child()
        .and_then(|h| h.last_child())
        .and_downcast::<gtk::Label>()
        .map(|l| l.label().to_string())
        .unwrap_or_default()
}
