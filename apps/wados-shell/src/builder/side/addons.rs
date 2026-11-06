//! The sidebar's Add-ons tab (the spike's `SideAddons.svelte`; its lists
//! are [`addon_list`]'s, `addons.ts`): only the add-ons of the apps in it.
//! **Suggested** first (for its apps, not in it yet: what goes with what it
//! has first; a click adds one), then by app: web apps for WadBrowser
//! (Focus, the usual sites, your own, **Add a site**), VS Code's
//! extensions, the command line (its tools, then its own **commands**,
//! RUN or COPY typed exactly, each named RUN_1…, a step of its build),
//! Steam's games and Dolphin's (listed, off till phase 10).
//!
//! Lit: in it. A click adds one (its app comes along) or takes it out
//! (Undo's in the toast); drag one onto a stack or a wadspace, or open
//! desktop.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use wad_gtk::keyed::Keyed;
use wad_model::catalog::{app_name, url_host};
use wad_model::commands::parse_command;
use wad_model::edit::Op;
use wad_model::suggest::Suggestion;
use wad_model::{AppId, BuildCommand, CommandKind, Wadspace};

use super::addon_list::{self, Group, GroupId, Item, focus, focus_toggle, toggle};
use super::ctl::{self, Row};
use super::kit::{self, Field};
use crate::builder::{Builder, Tab};
use crate::dnd::{self, Held};
use wad_gtk::Wrap;

const GROUPS: [GroupId; 5] = [GroupId::Web, GroupId::Extensions, GroupId::Tools, GroupId::Steam, GroupId::Dolphin];

/// One app's part: its heading, its card, and its pills.
struct Part {
    head: gtk::Label,
    card: gtk::Box,
    pills: Wrap,
    /// WadBrowser's Focus, at the end of its "for" line.
    focus: Option<gtk::Button>,
}

/// The command line's own commands, typed exactly.
struct Commands {
    root: gtk::Box,
    lines: RefCell<Keyed<String, BuildCommand, gtk::Box>>,
    form: gtk::Box,
    kind: gtk::Button,
    entry: gtk::Entry,
    add: gtk::Button,
    err: gtk::Label,
    run: Cell<CommandKind>,
}

pub struct Panel {
    builder: Weak<Builder>,
    me: Weak<Panel>,
    root: gtk::Box,
    sug_head: gtk::Label,
    sug_card: gtk::Box,
    sug: Wrap,
    parts: Vec<(GroupId, Part)>,
    site_add: gtk::Button,
    link_steam: gtk::Button,
    disc_upload: gtk::Button,
    no_discs: gtk::Label,
    cmds: Commands,
    none: gtk::Label,
    /// The wadspace shown, and whether it can be changed here.
    now: RefCell<Option<Wadspace>>,
    editable: Cell<bool>,
    /// Its groups' items, by group and key (what a click or a drag reads).
    items: RefCell<HashMap<(GroupId, String), Item>>,
}

impl Panel {
    pub fn new(b: &Rc<Builder>) -> Rc<Self> {
        kit::install_css();
        let me = Rc::new_cyclic(|me: &Weak<Panel>| {
            let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["side-addons"]).build();

            // Suggested.
            let sug_head = kit::heading("Suggested");
            sug_head.add_css_class("first");
            let sug_card = card(&["side-suggested"]);
            sug_card.set_widget_name("addons:suggested");
            sug_card.append(&for_line("For the apps in it, not in it yet", None));
            let sug = Wrap::new(6, 6);
            sug_card.append(&sug);
            root.append(&sug_head);
            root.append(&sug_card);

            // A part per app with add-ons.
            let site_add = pill_button("plus", "Add a site", "A site of your own: its name and address");
            site_add.add_css_class("side-site-add");
            site_add.set_halign(gtk::Align::Start);
            let w = me.clone();
            site_add.connect_clicked(move |b| {
                if let Some(p) = w.upgrade() {
                    p.ask_site(b.upcast_ref());
                }
            });
            let link_steam = pill_button("gamepad-2", "Link Steam", "Sign in to Steam, in Settings ▸ Accounts");
            let w = me.clone();
            link_steam.connect_clicked(move |_| {
                if let Some(b) = w.upgrade().and_then(|p| p.builder.upgrade()) {
                    crate::peers::open_app(&b.shell, &["wados-settings", "--page", "accounts"]);
                }
            });
            let disc_upload = pill_button("upload", "Upload iso, rvz file", "An .iso or .rvz from this computer");
            disc_upload.set_halign(gtk::Align::Start);
            let w = me.clone();
            disc_upload.connect_clicked(move |b| {
                if let Some(p) = w.upgrade() {
                    p.choose_disc(b);
                }
            });
            let no_discs = note("Its games are discs: upload one and it shows here.");
            let cmds = commands(me);
            let mut parts = vec![];
            for g in GROUPS {
                let (title, app) = match g {
                    GroupId::Web => ("Web apps", AppId::Wadbrowser),
                    GroupId::Extensions => ("VS Code extensions", AppId::Vscode),
                    GroupId::Tools => ("Command line", AppId::Terminal),
                    GroupId::Steam => ("Steam games", AppId::Steam),
                    GroupId::Dolphin => ("Dolphin games", AppId::Dolphin),
                };
                let head = kit::heading(title);
                let card = card(&[]);
                card.set_widget_name(&format!("addons:{}", g.id()));
                let focus = (g == GroupId::Web).then(|| {
                    let f = pill_button("eye", "Focus", "Focus: its web apps alone, no address bar");
                    f.add_css_class("side-focus");
                    let w = me.clone();
                    f.connect_clicked(move |_| {
                        if let Some(p) = w.upgrade() {
                            p.focus_click();
                        }
                    });
                    f
                });
                card.append(&for_line(&format!("for {}", app_name(app)), Some((app, focus.as_ref()))));
                if g == GroupId::Steam {
                    let line = gtk::Box::builder().spacing(10).css_classes(["side-link-steam"]).build();
                    let l = note("Its games show here once your Steam account is linked.");
                    l.set_hexpand(true);
                    line.append(&l);
                    link_steam.set_valign(gtk::Align::Center);
                    line.append(&link_steam);
                    card.append(&line);
                }
                let pills = Wrap::new(6, 6);
                card.append(&pills);
                match g {
                    GroupId::Web => card.append(&site_add),
                    GroupId::Tools => card.append(&cmds.root),
                    GroupId::Dolphin => {
                        card.append(&no_discs);
                        card.append(&disc_upload);
                    }
                    _ => {}
                }
                root.append(&head);
                root.append(&card);
                parts.push((g, Part { head, card, pills, focus }));
            }
            let none = note(
                "No add-ons for its apps. Add VS Code, WadBrowser, the command line, Steam or Dolphin in Apps for theirs.",
            );
            root.append(&none);
            Panel {
                builder: Rc::downgrade(b),
                me: me.clone(),
                root,
                sug_head,
                sug_card,
                sug,
                parts,
                site_add,
                link_steam,
                disc_upload,
                no_discs,
                cmds,
                none,
                now: RefCell::default(),
                editable: Cell::new(false),
                items: RefCell::default(),
            }
        });
        let rows: Weak<dyn ctl::Rows> = Rc::downgrade(&me) as Weak<dyn ctl::Rows>;
        ctl::register(b, Tab::Addons, rows);
        me
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// The selected wadspace, as it is now.
    pub fn show(&self, x: &Wadspace, editable: bool) {
        let Some(b) = self.builder.upgrade() else { return };
        let other = self.now.borrow().as_ref().is_none_or(|y| y.id != x.id);
        *self.now.borrow_mut() = Some(x.clone());
        self.editable.set(editable);
        if other {
            // Another wadspace: what was being typed was for the last one.
            self.cmds.entry.set_text("");
            self.cmds.err.set_visible(false);
        }
        let me = self.me.clone();

        // Suggested.
        let sug = addon_list::suggested(x, editable);
        self.sug_head.set_visible(!sug.is_empty());
        self.sug_card.set_visible(!sug.is_empty());
        let keyed: Vec<(String, Suggestion)> = sug.into_iter().map(|s| (s.item.clone(), s)).collect();
        self.sug.set(&keyed, |s| suggest_pill(&me, s), paint_suggestion);

        // A part per app in it.
        let groups: Vec<Group> = addon_list::groups(&b.shell.store.library(), x);
        let mut items = HashMap::new();
        for (id, part) in &self.parts {
            let g = groups.iter().find(|g| g.id == *id);
            part.head.set_visible(g.is_some());
            part.card.set_visible(g.is_some());
            let Some(g) = g else { continue };
            for i in &g.items {
                items.insert((*id, i.key.clone()), i.clone());
            }
            let keyed: Vec<(String, Item)> = g.items.iter().map(|i| (i.key.clone(), i.clone())).collect();
            let gid = *id;
            part.pills.set(&keyed, |i| item_pill(&me, gid, i), |_, _| {});
            for (k, w) in part.pills.keyed() {
                if let Some(i) = items.get(&(gid, k)) {
                    paint_item(&w, i, editable);
                }
            }
            if let Some(f) = &part.focus {
                let on = focus(x).is_some();
                lit(f, on);
                f.set_sensitive(editable);
                f.set_tooltip_text(Some(if on {
                    "Focus: its web apps alone, no address bar (on: click to take it off)"
                } else {
                    "Focus: its web apps alone, no address bar"
                }));
            }
            if *id == GroupId::Dolphin {
                self.no_discs.set_visible(g.items.is_empty());
            }
        }
        *self.items.borrow_mut() = items;
        self.none.set_visible(groups.is_empty());
        self.site_add.set_visible(editable);

        // Its commands.
        let cs: Vec<(String, BuildCommand)> = x.commands.iter().map(|c| (c.id.clone(), c.clone())).collect();
        self.cmds.lines.borrow_mut().set(&cs, |c| command_line(&me, c), paint_command);
        for (_, l) in self.cmds.lines.borrow().widgets() {
            if let Some(x) = l.last_child() {
                x.set_visible(editable);
            }
        }
        self.cmds.form.set_visible(editable);
        let typed = !self.cmds.entry.text().trim().is_empty();
        self.cmds.add.set_sensitive(typed);
    }

    fn act(&self, op: Option<Op>) {
        let (Some(b), Some(op)) = (self.builder.upgrade(), op) else { return };
        if !self.editable.get() {
            return;
        }
        b.shell.op(op, |_| {});
    }

    fn item_click(&self, g: GroupId, key: &str) {
        let x = self.now.borrow().clone();
        let i = self.items.borrow().get(&(g, key.to_string())).cloned();
        if let (Some(x), Some(i)) = (x, i) {
            self.act(toggle(&x, &i));
        }
    }

    /// A disc from this computer. It stays on this machine unless the person
    /// also wants it in their GitHub packages.
    fn choose_disc(&self, from: &gtk::Button) {
        let Some(b) = self.builder.upgrade() else { return };
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Discs (.iso, .rvz)"));
        filter.add_suffix("iso");
        filter.add_suffix("rvz");
        let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title("Upload iso, rvz file")
            .modal(true)
            .filters(&filters)
            .default_filter(&filter)
            .build();
        let window = from.root().and_downcast::<gtk::Window>();
        let shell = b.shell.clone();
        let w2 = window.clone();
        dialog.open(window.as_ref(), gtk::gio::Cancellable::NONE, move |r| {
            let Some(path) = r.ok().and_then(|f| f.path()) else { return };
            let ask = gtk::AlertDialog::builder()
                .message("Add this disc?")
                .detail("It stays on this machine.")
                .buttons(["Cancel", "Keep it here", "Also keep it in your GitHub packages"])
                .cancel_button(0)
                .default_button(1)
                .build();
            let shell = shell.clone();
            ask.choose(w2.as_ref(), gtk::gio::Cancellable::NONE, move |pick| {
                let push = match pick {
                    Ok(1) => false,
                    Ok(2) => true,
                    _ => return,
                };
                let (s, path) = (shell.clone(), path.display().to_string());
                shell.link.call(
                    move |w| async move { w.disc_add(&path, push).await },
                    move |r| {
                        if let Err(e) = r {
                            s.toast(wad_proto::Level::Warn, &e.message, None);
                        }
                    },
                );
            });
        });
    }

    fn focus_click(&self) {
        let x = self.now.borrow().clone();
        if let Some(x) = x {
            self.act(Some(focus_toggle(&x)));
        }
    }

    fn suggest_click(&self, item: &str) {
        let x = self.now.borrow().clone();
        if let Some(x) = x {
            self.act(addon_list::add_addon(&x, item));
        }
    }

    fn drag_of(&self, g: GroupId, key: &str) -> Option<Held> {
        let i = self.items.borrow().get(&(g, key.to_string())).cloned()?;
        if i.off.is_some() {
            return None;
        }
        i.drag
    }

    /// Add a site: its name and address, in a small dialog.
    fn ask_site(&self, anchor: &gtk::Widget) {
        let (Some(b), Some(x)) = (self.builder.upgrade(), self.now.borrow().clone()) else { return };
        if !self.editable.get() {
            return;
        }
        let shell = b.shell.clone();
        kit::ask(
            anchor,
            "Add a site",
            &[
                Field { key: "name", label: "Name", placeholder: "Notion", text: String::new() },
                Field { key: "url", label: "Address", placeholder: "notion.so", text: String::new() },
            ],
            "Add",
            move |v| {
                let (name, url) = (v[0].trim(), v[1].trim());
                if url.is_empty() {
                    return Err("Type its address".into());
                }
                if url_host(url).is_none() {
                    return Err("That isn’t a web address".into());
                }
                shell.op(Op::AddWebApp { ws: x.id.clone(), name: name.into(), url: url.into() }, |_| {});
                Ok(())
            },
        );
    }

    /// A command of its own, as typed (Enter, or +).
    fn add_command(&self) {
        let (Some(b), Some(x)) = (self.builder.upgrade(), self.now.borrow().clone()) else { return };
        if !self.editable.get() {
            return;
        }
        let typed = self.cmds.entry.text().to_string();
        let kind = self.cmds.run.get();
        if let Err(e) = parse_command(kind, &typed) {
            self.cmds.err.set_label(&e.to_string());
            self.cmds.err.set_visible(true);
            return;
        }
        let entry = self.cmds.entry.downgrade();
        b.shell.op(Op::AddCommand { ws: x.id.clone(), kind, text: typed.clone() }, move |_| {
            // Cleared once it's in (unless something else was typed since).
            if let Some(e) = entry.upgrade()
                && e.text() == typed
            {
                e.set_text("");
            }
        });
    }

    fn remove_command(&self, id: &str) {
        let x = self.now.borrow().clone();
        if let Some(x) = x {
            self.act(Some(Op::RemoveCommand { ws: x.id, id: id.to_string() }));
        }
    }

    fn switch_kind(&self) {
        let k = match self.cmds.run.get() {
            CommandKind::Run => CommandKind::Copy,
            CommandKind::Copy => CommandKind::Run,
        };
        self.cmds.run.set(k);
        self.cmds.kind.set_label(k.word());
        self.cmds.entry.set_placeholder_text(Some(placeholder(k)));
    }
}

impl ctl::Rows for Panel {
    fn rows(&self) -> Vec<Row> {
        let mut out = vec![];
        if self.sug_card.is_visible() {
            for (k, w) in self.sug.keyed() {
                out.push(Row::new(format!("suggest:{k}"), &w, label_of(&w)));
            }
        }
        for (_, p) in &self.parts {
            if !p.card.is_visible() {
                continue;
            }
            if let Some(f) = &p.focus {
                out.push(Row::new("focus", f, "Focus").lit(f.has_css_class("in")));
            }
            for (k, w) in p.pills.keyed() {
                out.push(Row::new(format!("addon:{k}"), &w, label_of(&w)).lit(w.has_css_class("in")));
            }
        }
        let card_of = |g: GroupId| self.parts.iter().find(|(id, _)| *id == g).is_some_and(|(_, p)| p.card.is_visible());
        if card_of(GroupId::Web) && self.site_add.is_visible() {
            out.push(Row::new("site-add", &self.site_add, "Add a site"));
        }
        if card_of(GroupId::Tools) {
            for (id, l) in self.cmds.lines.borrow().widgets() {
                out.push(Row::new(format!("command:{id}"), l, line_text(l)));
                if let Some(x) = l.last_child().filter(|x| x.is_visible()) {
                    out.push(Row::new(format!("command-remove:{id}"), &x, "×"));
                }
            }
            if self.cmds.form.is_visible() {
                out.push(Row::new("command-kind", &self.cmds.kind, self.cmds.run.get().word()));
                out.push(Row::new("command-input", &self.cmds.entry, self.cmds.entry.text()));
                out.push(Row::new("command-add", &self.cmds.add, "Add the command"));
            }
            if self.cmds.err.is_visible() {
                out.push(Row::new("command-error", &self.cmds.err, self.cmds.err.label()));
            }
        }
        if card_of(GroupId::Steam) {
            out.push(Row::new("link-steam", &self.link_steam, "Link Steam"));
        }
        if card_of(GroupId::Dolphin) {
            out.push(Row::new("disc-upload", &self.disc_upload, "Upload iso, rvz file"));
        }
        out
    }
}

// ------------------------------------------------------------ its widgets

fn card(classes: &[&str]) -> gtk::Box {
    let c = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["side-card"]).build();
    for k in classes {
        c.add_css_class(k);
    }
    c
}

fn note(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["side-none"]).build()
}

/// A card's first line: what it's for (an app's icon and "for VS Code"), and maybe a button at its end.
fn for_line(text: &str, app: Option<(AppId, Option<&gtk::Button>)>) -> gtk::Box {
    let row = gtk::Box::builder().spacing(6).css_classes(["side-for"]).build();
    if let Some((a, _)) = app {
        row.append(&wad_gtk::app_icon::app(a, 14));
    }
    row.append(&gtk::Label::builder().label(text).xalign(0.0).hexpand(true).build());
    if let Some((_, Some(b))) = app {
        b.set_valign(gtk::Align::Center);
        row.append(b);
    }
    row
}

/// A pill-shaped button: an icon and a label, what it does in its tooltip.
fn pill_button(icon: &'static str, label: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::builder().css_classes(["side-pill"]).tooltip_text(tip).build();
    let h = gtk::Box::builder().spacing(6).build();
    h.append(&wad_gtk::Icon::new(icon, 13));
    h.append(&gtk::Label::new(Some(label)));
    b.set_child(Some(&h));
    b
}

fn lit(b: &impl IsA<gtk::Widget>, on: bool) {
    if on {
        b.add_css_class("in");
    } else {
        b.remove_css_class("in");
    }
}

/// A pill's name's label.
fn name_label(w: &gtk::Widget) -> Option<gtk::Label> {
    let mut stack = vec![w.clone()];
    while let Some(n) = stack.pop() {
        if let Some(l) = n.downcast_ref::<gtk::Label>()
            && l.has_css_class("side-name")
        {
            return Some(l.clone());
        }
        let mut c = n.last_child();
        while let Some(k) = c {
            c = k.prev_sibling();
            stack.push(k);
        }
    }
    None
}

fn label_of(w: &gtk::Widget) -> String {
    name_label(w).map(|l| l.label().to_string()).unwrap_or_default()
}

fn suggest_pill(me: &Weak<Panel>, s: &Suggestion) -> gtk::Widget {
    let b = gtk::Button::builder().css_classes(["side-pill"]).build();
    b.set_widget_name(&format!("suggest:{}", s.item));
    let h = gtk::Box::builder().spacing(6).build();
    h.append(&wad_gtk::Icon::new("plus", 12));
    h.append(&wad_gtk::app_icon::app(s.app, 15));
    h.append(&gtk::Label::builder().label(&s.name).css_classes(["side-name"]).build());
    if let Some(why) = &s.why {
        h.append(&gtk::Label::builder().label(why).css_classes(["side-why"]).build());
    }
    b.set_child(Some(&h));
    paint_suggestion(b.upcast_ref(), s);
    let (w, item) = (me.clone(), s.item.clone());
    b.connect_clicked(move |_| {
        if let Some(p) = w.upgrade() {
            p.suggest_click(&item);
        }
    });
    b.upcast()
}

fn paint_suggestion(w: &gtk::Widget, s: &Suggestion) {
    let why = s.why.as_ref().map(|y| format!(": {}", y.to_lowercase())).unwrap_or_default();
    w.set_tooltip_text(Some(&format!("Add {} to {}{why}", s.name, app_name(s.app))));
}

fn item_pill(me: &Weak<Panel>, g: GroupId, i: &Item) -> gtk::Widget {
    let b = gtk::Button::builder().css_classes(["side-pill"]).build();
    b.set_widget_name(&format!("addon:{}", i.key));
    let h = gtk::Box::builder().spacing(6).build();
    h.append(&kit::item_icon(&i.icon, 16));
    h.append(
        &gtk::Label::builder()
            .label(&i.name)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["side-name"])
            .build(),
    );
    b.set_child(Some(&h));
    let (w, key) = (me.clone(), i.key.clone());
    b.connect_clicked(move |_| {
        if let Some(p) = w.upgrade() {
            p.item_click(g, &key);
        }
    });
    let (w, key, look) = (me.clone(), i.key.clone(), b.downgrade());
    dnd::source(&b, move |_, _| Some((w.upgrade()?.drag_of(g, &key)?, look.upgrade()?.upcast())));
    b.upcast()
}

fn paint_item(w: &gtk::Widget, i: &Item, editable: bool) {
    lit(w, i.lit.is_some());
    if let Some(l) = name_label(w)
        && l.label() != i.name
    {
        l.set_label(&i.name);
    }
    w.set_sensitive(editable && i.off.is_none() && (i.lit.is_some() || i.add.is_some()));
    let tip = match &i.off {
        Some(why) => format!("{}: {why}", i.name),
        None if i.lit.is_some() => format!("{}: in it (click to take it out)", i.name),
        None => format!("Add {}", i.name),
    };
    w.set_tooltip_text(Some(&tip));
}

fn placeholder(k: CommandKind) -> &'static str {
    match k {
        CommandKind::Run => "cargo install ripgrep",
        CommandKind::Copy => "~/dotfiles/.zshrc /home/you/.zshrc",
    }
}

fn commands(me: &Weak<Panel>) -> Commands {
    let root =
        gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["side-commands"]).build();
    root.append(
        &gtk::Label::builder().label("Commands: your own, typed exactly").xalign(0.0).css_classes(["side-sub"]).build(),
    );
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
    root.append(&list);
    let form = gtk::Box::builder().spacing(6).css_classes(["side-type"]).build();
    let kind = gtk::Button::builder()
        .label("RUN")
        .css_classes(["side-kind"])
        .valign(gtk::Align::Center)
        .tooltip_text(
            "RUN a command, or COPY a path on this machine into it: click to switch (or type RUN or COPY first)",
        )
        .build();
    let entry = gtk::Entry::builder()
        .placeholder_text(placeholder(CommandKind::Run))
        .hexpand(true)
        .valign(gtk::Align::Center)
        .css_classes(["side-entry"])
        .tooltip_text("A command of its own")
        .build();
    entry.set_input_hints(gtk::InputHints::NO_SPELLCHECK);
    let add = gtk::Button::builder()
        .css_classes(["side-add"])
        .valign(gtk::Align::Center)
        .tooltip_text("Add the command")
        .sensitive(false)
        .build();
    add.set_child(Some(&wad_gtk::Icon::new("plus", 14)));
    form.append(&kind);
    form.append(&entry);
    form.append(&add);
    root.append(&form);
    let err = gtk::Label::builder().xalign(0.0).wrap(true).visible(false).css_classes(["side-err"]).build();
    root.append(&err);

    let w = me.clone();
    kind.connect_clicked(move |_| {
        if let Some(p) = w.upgrade() {
            p.switch_kind();
        }
    });
    let w = me.clone();
    entry.connect_activate(move |_| {
        if let Some(p) = w.upgrade() {
            p.add_command();
        }
    });
    let (a, e) = (add.downgrade(), err.downgrade());
    entry.connect_changed(move |en| {
        if let Some(a) = a.upgrade() {
            a.set_sensitive(!en.text().trim().is_empty());
        }
        if let Some(e) = e.upgrade() {
            e.set_visible(false);
        }
    });
    let w = me.clone();
    add.connect_clicked(move |_| {
        if let Some(p) = w.upgrade() {
            p.add_command();
        }
    });
    Commands {
        root,
        lines: RefCell::new(Keyed::new(list)),
        form,
        kind,
        entry,
        add,
        err,
        run: Cell::new(CommandKind::Run),
    }
}

/// A command: its name (RUN_1), what it does exactly, and × (the last child).
fn command_line(me: &Weak<Panel>, c: &BuildCommand) -> gtk::Box {
    let line = gtk::Box::builder().spacing(8).css_classes(["side-cmd"]).build();
    line.set_widget_name(&format!("command:{}", c.id));
    line.append(&gtk::Label::builder().label(&c.name).valign(gtk::Align::Start).css_classes(["side-cmd-name"]).build());
    line.append(
        &gtk::Label::builder()
            .label(&c.text)
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .lines(2)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .width_chars(1)
            .max_width_chars(1)
            .css_classes(["side-cmd-text"])
            .build(),
    );
    let x = gtk::Button::builder().css_classes(["side-x"]).valign(gtk::Align::Start).build();
    x.set_child(Some(&wad_gtk::Icon::new("x", 11)));
    x.set_tooltip_text(Some(&format!("Take {} out", c.name)));
    let (w, id) = (me.clone(), c.id.clone());
    x.connect_clicked(move |_| {
        if let Some(p) = w.upgrade() {
            p.remove_command(&id);
        }
    });
    line.append(&x);
    paint_command(&line, c);
    line
}

fn paint_command(line: &gtk::Box, c: &BuildCommand) {
    let name = line.first_child().and_downcast::<gtk::Label>();
    let text = name.as_ref().and_then(|n| n.next_sibling()).and_downcast::<gtk::Label>();
    if let Some(n) = name {
        n.set_label(&c.name);
    }
    if let Some(t) = text {
        t.set_label(&c.text);
        t.set_tooltip_text(Some(&format!("{} {}", c.kind.word(), c.text)));
    }
}

fn line_text(line: &gtk::Box) -> String {
    let name = line.first_child().and_downcast::<gtk::Label>().map(|l| l.label().to_string()).unwrap_or_default();
    let text = line
        .first_child()
        .and_then(|n| n.next_sibling())
        .and_downcast::<gtk::Label>()
        .map(|l| l.label().to_string())
        .unwrap_or_default();
    format!("{name} {text}")
}
