//! Settings ▸ Browser: WadBrowser's settings on this machine, which the
//! browser has no page for. Its address bar, its ad and tracker blocker,
//! what it blocks (the machine's blocker groups, each on or off here, made
//! and changed here or in the Builder: a site is blocked in a group), and the
//! machine's web apps (made from pages: Remove). It searches with
//! DuckDuckGo: that isn't a setting.
//!
//! They're the user's `~/.config/wadbrowser/wadbrowser.conf` (wad-model's
//! `browser` format: the groups on by id), which the browser watches: a
//! change counts in its open windows at once. The groups themselves are the
//! Library's (wadd's), which the browser reads from the file wadd keeps. A
//! wadspace's browser has its own settings, from its design (the Builder's
//! add-ons).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use wad_gtk::form::{TextField, Toggle};
use wad_gtk::kit::{self, Group, Row, Tone, Variant};
use wad_model::BrowserApp;
use wad_model::browser::{self, BlockGroup, BrowserPrefs, ConfText};
use wad_proto::Event;

use crate::panel::{Ctx, Panel};
use crate::parts;
use wad_wadd_client::Incoming;

const CSS: &str = r#"
.br-h4 { margin: 4px 2px 0; font-size: 11.5px; font-weight: 600; letter-spacing: 0.06em; color: var(--faint); }
button.br-site {
  min-height: 28px;
  padding: 0 12px;
  border-radius: 99px;
  border: 1px solid var(--line-strong);
  background: transparent;
  background-image: none;
  box-shadow: none;
  color: var(--fg);
  font-size: 12.5px;
  font-weight: 500;
}
button.br-site:hover { background-color: rgb(255 255 255 / 0.06); }
button.br-site.on {
  border-color: color-mix(in srgb, var(--danger) 70%, transparent);
  background-color: color-mix(in srgb, var(--danger) 14%, transparent);
}
.br-add { margin-top: 4px; }
.br-edit {
  margin-top: 6px;
  padding: 14px;
  border-radius: 12px;
  border: 1px solid var(--line-strong);
}
"#;

/// The settings as the browser reads them here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conf {
    pub prefs: BrowserPrefs,
    /// Sites an older Settings blocked one by one (`block`): made a group
    /// of their own the next time this page is shown, then gone from the file.
    pub old_sites: Vec<String>,
}

impl Conf {
    pub fn parse(text: &str) -> Conf {
        let mut c = Conf::default();
        let t = ConfText::parse(text);
        t.over(&mut c.prefs);
        c.old_sites = t.blocked.unwrap_or_default();
        c
    }

    pub fn text(&self) -> String {
        browser::conf(
            "WadBrowser's settings on this machine: Settings ▸ Browser's (the\nbrowser reads them again when they change).",
            &self.prefs,
            &self.old_sites,
        )
    }
}

/// The user's wadbrowser.conf: `$XDG_CONFIG_HOME/wadbrowser/wadbrowser.conf`.
pub fn conf_path() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    config.join("wadbrowser").join("wadbrowser.conf")
}

fn load() -> Conf {
    std::fs::read_to_string(conf_path()).map(|t| Conf::parse(&t)).unwrap_or_default()
}

/// Written whole (a temporary file, then renamed: the browser never reads half of it).
fn store(c: &Conf) -> std::io::Result<()> {
    let path = conf_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("conf.tmp");
    std::fs::write(&tmp, c.text())?;
    std::fs::rename(&tmp, &path)
}

pub fn make(_: &Ctx) -> Rc<dyn Panel> {
    Rc::new_cyclic(|me| BrowserPanel {
        me: me.clone(),
        ui: RefCell::default(),
        apps: RefCell::default(),
        groups: RefCell::default(),
    })
}

struct Ui {
    address: Toggle,
    blocker: Toggle,
    groups: gtk::Box,
    groups_note: gtk::Label,
    /// The group being made or changed (its form), under the list.
    edit: GroupForm,
    apps: gtk::Box,
    apps_note: gtk::Label,
    note: gtk::Label,
}

/// New group, or a group changed: its name and sites.
struct GroupForm {
    root: gtk::Box,
    title: gtk::Label,
    name: TextField,
    sites: TextField,
    remove: gtk::Button,
    /// The group's id ("": a new one).
    id: RefCell<String>,
}

pub struct BrowserPanel {
    me: Weak<BrowserPanel>,
    ui: RefCell<Option<Ui>>,
    /// The machine's web apps, as wadd last said.
    apps: RefCell<Vec<BrowserApp>>,
    /// The machine's blocker groups, as wadd last said.
    groups: RefCell<Vec<BlockGroup>>,
}

impl BrowserPanel {
    /// `f` done to the settings, then kept (and shown).
    fn change(&self, f: impl FnOnce(&mut Conf)) {
        let mut c = load();
        f(&mut c);
        c.prefs = c.prefs.tidied();
        let saved = store(&c);
        if let Some(ui) = &*self.ui.borrow() {
            match &saved {
                Ok(()) => ui.note.set_visible(false),
                Err(e) => {
                    ui.note.set_label(&format!("The settings weren’t saved: {e}"));
                    ui.note.set_visible(true);
                }
            }
        }
        self.paint(&c);
    }

    /// Sites an older Settings blocked one by one, made a group of their
    /// own (on here), so none stops being blocked.
    fn upgrade(&self) {
        let c = load();
        if c.old_sites.is_empty() {
            return;
        }
        let Some(ctx) = CTX.with_borrow(|c| c.clone()) else { return };
        let group = BlockGroup { id: String::new(), name: "This machine’s sites".into(), sites: c.old_sites.clone() };
        let w = self.me.clone();
        ctx.link.call(
            move |wadd| async move { wadd.op(wad_model::Op::SaveBlockGroup { group }).await },
            move |r| {
                let (Some(me), Ok(out)) = (w.upgrade(), r) else { return };
                let Some(id) = out.created else { return };
                me.change(move |c| {
                    c.old_sites.clear();
                    if !c.prefs.groups.contains(&id) {
                        c.prefs.groups.push(id);
                    }
                });
            },
        );
    }

    /// The page as `c` says.
    fn paint(&self, c: &Conf) {
        let Some(ui) = &*self.ui.borrow() else { return };
        ui.address.set(c.prefs.address_bar);
        ui.blocker.set(c.prefs.ad_blocker);
        self.paint_groups(c);
    }

    /// The machine's groups, each on here or not (`c`), with Edit.
    fn paint_groups(&self, c: &Conf) {
        let Some(ui) = &*self.ui.borrow() else { return };
        while let Some(w) = ui.groups.first_child() {
            ui.groups.remove(&w);
        }
        let groups = self.groups.borrow().clone();
        for g in &groups {
            let row =
                Row::new(Some("layers"), Tone::Plain, &g.name, &group_summary(g)).named(&format!("group-{}", g.id));
            let edit = kit::button("Edit", Variant::Secondary, None, true);
            edit.set_widget_name(&format!("edit-group-{}", g.id));
            let (w, gc) = (self.me.clone(), g.clone());
            edit.connect_clicked(move |_| {
                if let Some(me) = w.upgrade() {
                    me.open_form(Some(&gc));
                }
            });
            row.ctl.append(&edit);
            let (w, id) = (self.me.clone(), g.id.clone());
            let on = Toggle::new(c.prefs.groups.contains(&g.id), move |on| {
                if let Some(me) = w.upgrade() {
                    let id = id.clone();
                    me.change(move |c| {
                        if c.prefs.groups.contains(&id) != on {
                            c.prefs.toggle_group(&id);
                        }
                    });
                }
            })
            .named(&format!("group-on-{}", g.id));
            row.ctl.append(&on.root);
            ui.groups.append(&row.root);
        }
        ui.groups_note.set_visible(groups.is_empty());
        ui.groups.set_visible(!groups.is_empty());
    }

    /// The group form, for `g` (None: a new group).
    fn open_form(&self, g: Option<&BlockGroup>) {
        let Some(ui) = &*self.ui.borrow() else { return };
        let f = &ui.edit;
        *f.id.borrow_mut() = g.map(|g| g.id.clone()).unwrap_or_default();
        f.title.set_label(if g.is_some() { "EDIT GROUP" } else { "NEW GROUP" });
        f.name.text.set_text(g.map(|g| g.name.as_str()).unwrap_or(""));
        f.sites.text.set_text(&g.map(|g| g.sites.join(" ")).unwrap_or_default());
        f.name.set_error(None);
        f.sites.set_error(None);
        f.remove.set_visible(g.is_some());
        f.root.set_visible(true);
        f.name.text.grab_focus();
    }

    fn close_form(&self) {
        if let Some(ui) = &*self.ui.borrow() {
            ui.edit.root.set_visible(false);
        }
    }

    /// The form's group saved (a new one is on here at once).
    fn save_group(&self) {
        let Some(ctx) = CTX.with_borrow(|c| c.clone()) else { return };
        let (id, name, sites) = {
            let Some(ui) = &*self.ui.borrow() else { return };
            let f = &ui.edit;
            (f.id.borrow().clone(), f.name.text.text().to_string(), f.sites.text.text().to_string())
        };
        let group = match group_from(&id, &name, &sites) {
            Ok(g) => g,
            Err((on_name, why)) => {
                if let Some(ui) = &*self.ui.borrow() {
                    let (bad, ok) =
                        if on_name { (&ui.edit.name, &ui.edit.sites) } else { (&ui.edit.sites, &ui.edit.name) };
                    bad.set_error(Some(&why));
                    ok.set_error(None);
                }
                return;
            }
        };
        let w = self.me.clone();
        let new = id.is_empty();
        ctx.link.call(
            move |wadd| async move { wadd.op(wad_model::Op::SaveBlockGroup { group }).await },
            move |r| {
                let Some(me) = w.upgrade() else { return };
                match r {
                    Ok(out) => {
                        me.close_form();
                        if new && let Some(id) = out.created {
                            me.change(move |c| {
                                if !c.prefs.groups.contains(&id) {
                                    c.prefs.groups.push(id);
                                }
                            });
                        }
                    }
                    Err(e) => me.say(&format!("The group wasn’t saved: {}", e.message)),
                }
            },
        );
    }

    fn remove_group(&self) {
        let Some(ctx) = CTX.with_borrow(|c| c.clone()) else { return };
        let id = match &*self.ui.borrow() {
            Some(ui) => ui.edit.id.borrow().clone(),
            None => return,
        };
        if id.is_empty() {
            return;
        }
        let w = self.me.clone();
        let gone = id.clone();
        ctx.link.call(
            move |wadd| async move { wadd.op(wad_model::Op::RemoveBlockGroup { id }).await },
            move |r| {
                let Some(me) = w.upgrade() else { return };
                match r {
                    Ok(_) => {
                        me.close_form();
                        me.change(move |c| c.prefs.groups.retain(|g| g != &gone));
                    }
                    Err(e) => me.say(&format!("The group wasn’t removed: {}", e.message)),
                }
            },
        );
    }

    fn say(&self, text: &str) {
        if let Some(ui) = &*self.ui.borrow() {
            ui.note.set_label(text);
            ui.note.set_visible(true);
        }
    }

    /// The machine's web apps, as wadd last said.
    fn paint_apps(&self) {
        let Some(ui) = &*self.ui.borrow() else { return };
        while let Some(c) = ui.apps.first_child() {
            ui.apps.remove(&c);
        }
        let apps = self.apps.borrow().clone();
        for a in &apps {
            let row = Row::new(Some("app-window"), Tone::Plain, &a.name, &a.url).named(&format!("webapp-{}", a.id));
            let b = kit::button("Remove", Variant::Secondary, None, true);
            b.set_widget_name(&format!("remove-webapp-{}", a.id));
            let (w, id) = (self.me.clone(), a.id.clone());
            b.connect_clicked(move |b| {
                b.set_sensitive(false);
                if let Some(me) = w.upgrade() {
                    me.remove_app(&id);
                }
            });
            row.ctl.append(&b);
            ui.apps.append(&row.root);
        }
        ui.apps_note.set_visible(apps.is_empty());
        ui.apps.set_visible(!apps.is_empty());
    }

    fn remove_app(&self, id: &str) {
        let Some(ctx) = CTX.with_borrow(|c| c.clone()) else { return };
        let (id, w) = (id.to_string(), self.me.clone());
        ctx.link.call(
            move |wadd| async move { wadd.op(wad_model::Op::RemoveWebApp { id }).await },
            move |r| {
                if let (Some(me), Err(e)) = (w.upgrade(), r)
                    && let Some(ui) = &*me.ui.borrow()
                {
                    ui.note.set_label(&format!("The web app wasn’t removed: {}", e.message));
                    ui.note.set_visible(true);
                }
            },
        );
    }
}

thread_local! {
    /// The panel's line to wadd (for Remove).
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

impl Panel for BrowserPanel {
    fn build(&self, ctx: &Ctx) -> gtk::Widget {
        parts::install();
        if let Some(d) = gtk::gdk::Display::default() {
            wad_gtk::theme::add_css(&d, CSS);
        }
        CTX.with_borrow_mut(|c| *c = Some(ctx.clone()));
        let c = load();
        self.upgrade();
        let page = kit::Page::new("Browser", false);

        let main = Group::new(
            "WadBrowser",
            "The browser on this machine. A wadspace’s has its own, in its Builder’s add-ons.",
        );
        let w = self.me.clone();
        let address = Toggle::new(c.prefs.address_bar, move |on| {
            if let Some(me) = w.upgrade() {
                me.change(|c| c.prefs.address_bar = on);
            }
        })
        .named("browser-address-bar");
        let row = Row::new(Some("globe"), Tone::Plain, "Address Bar", "Off: its tabs alone, no address to type");
        row.ctl.append(&address.root);
        main.add(&row.root);
        let w = self.me.clone();
        let blocker = Toggle::new(c.prefs.ad_blocker, move |on| {
            if let Some(me) = w.upgrade() {
                me.change(|c| c.prefs.ad_blocker = on);
            }
        })
        .named("browser-ad-blocker");
        let row = Row::new(Some("shield-check"), Tone::Plain, "Ad and Tracker Blocker", "EasyList and EasyPrivacy");
        row.ctl.append(&blocker.root);
        main.add(&row.root);
        let note = parts::note("", "set-err");
        main.root.append(&note);
        page.add(&main.root);

        let groups = Group::new(
            "Blocker Groups",
            "Sites blocked together: one switch for them all. The machine’s, so they’re in the Builder too.",
        );
        groups.list.set_visible(false);
        let group_list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["kit-list"]).build();
        groups.root.append(&group_list);
        let groups_note = parts::note("None yet: make one below.", "set-note");
        groups.root.append(&groups_note);
        let edit = group_form(&self.me);
        groups.root.append(&edit.root);
        let new = kit::button("New group", Variant::Secondary, Some("plus"), true);
        new.set_widget_name("browser-new-group");
        new.set_halign(gtk::Align::Start);
        let w = self.me.clone();
        new.connect_clicked(move |_| {
            if let Some(me) = w.upgrade() {
                me.open_form(None);
            }
        });
        groups.root.append(&new);
        page.add(&groups.root);

        let web =
            Group::new("Web Apps", "Made from pages in WadBrowser (right-click a page: Create app from this page).");
        web.list.set_visible(false);
        let apps = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["kit-list"]).build();
        web.root.append(&apps);
        let apps_note = parts::note("None yet.", "set-note");
        web.root.append(&apps_note);
        page.add(&web.root);

        *self.ui.borrow_mut() =
            Some(Ui { address, blocker, groups: group_list, groups_note, edit, apps, apps_note, note });
        self.paint(&c);
        self.paint_apps();
        page.root.upcast()
    }

    fn on_event(&self, _: &Ctx, i: &Incoming) {
        if let Incoming::Event(Event::Library(lib)) = i {
            if *self.groups.borrow() != *lib.block_groups {
                *self.groups.borrow_mut() = lib.block_groups.to_vec();
                self.paint_groups(&load());
            }
            if *self.apps.borrow() != lib.web_apps {
                *self.apps.borrow_mut() = lib.web_apps.clone();
                self.paint_apps();
            }
        }
    }

    fn show(&self, _: Option<&str>) {
        // Read again: it may have changed since (by hand, or another Settings).
        self.paint(&load());
    }
}

/// A group's line under its name: how many sites, and the first few.
fn group_summary(g: &BlockGroup) -> String {
    let shown: Vec<&str> = g.sites.iter().take(3).map(String::as_str).collect();
    let more = g.sites.len().saturating_sub(shown.len());
    let sites = if more > 0 { format!("{} and {more} more", shown.join(", ")) } else { shown.join(", ") };
    match g.sites.len() {
        0 => "No sites".into(),
        1 => sites,
        n => format!("{n} sites: {sites}"),
    }
}

/// The group a form says, or what's wrong (true: with its name).
fn group_from(id: &str, name: &str, sites: &str) -> Result<BlockGroup, (bool, String)> {
    let name = name.trim();
    if name.is_empty() {
        return Err((true, "Give the group a name".into()));
    }
    let mut out: Vec<String> = vec![];
    for t in sites.split([' ', ',', '\n', '\t']).filter(|t| !t.trim().is_empty()) {
        let Some(site) = browser::site(t) else {
            return Err((false, format!("{} isn’t a website: type them like example.com", t.trim())));
        };
        if !out.contains(&site) {
            out.push(site);
        }
    }
    if out.is_empty() {
        return Err((false, "Type the sites it blocks: example.com news.com".into()));
    }
    Ok(BlockGroup { id: id.into(), name: name.into(), sites: out })
}

/// The group form (hidden till New group or Edit).
fn group_form(me: &Weak<BrowserPanel>) -> GroupForm {
    let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).css_classes(["br-edit"]).build();
    root.set_visible(false);
    let title = kit::label("NEW GROUP", &["br-h4"]);
    root.append(&title);
    let name = TextField::new("Name", "Distractions").named("browser-group-name");
    root.append(&name.root);
    let sites = TextField::new("Sites it blocks", "news.com, example.com").named("browser-group-sites");
    root.append(&sites.root);
    let row = gtk::Box::builder().spacing(8).build();
    let remove = kit::button("Remove group", Variant::Secondary, Some("trash"), true);
    remove.set_widget_name("browser-group-remove");
    remove.set_hexpand(true);
    remove.set_halign(gtk::Align::Start);
    let cancel = kit::button("Cancel", Variant::Secondary, None, true);
    let save = kit::button("Save", Variant::Primary, None, true);
    save.set_widget_name("browser-group-save");
    row.append(&remove);
    row.append(&cancel);
    row.append(&save);
    root.append(&row);
    let w = me.clone();
    save.connect_clicked(move |_| {
        if let Some(me) = w.upgrade() {
            me.save_group();
        }
    });
    for t in [&name.text, &sites.text] {
        let w = me.clone();
        t.connect_activate(move |_| {
            if let Some(me) = w.upgrade() {
                me.save_group();
            }
        });
    }
    let w = me.clone();
    cancel.connect_clicked(move |_| {
        if let Some(me) = w.upgrade() {
            me.close_form();
        }
    });
    let w = me.clone();
    remove.connect_clicked(move |_| {
        if let Some(me) = w.upgrade() {
            me.remove_group();
        }
    });
    GroupForm { root, title, name, sites, remove, id: RefCell::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_conf_reads_back() {
        let c = Conf {
            prefs: BrowserPrefs {
                address_bar: false,
                ad_blocker: false,
                groups: vec!["social-media".into()],
                ..Default::default()
            },
            old_sites: vec![],
        };
        assert_eq!(Conf::parse(&c.text()), c);
        assert_eq!(Conf::parse(""), Conf::default());
        assert!(Conf::default().prefs.address_bar && Conf::default().prefs.ad_blocker);
        // An older file's own sites: read, to be made a group.
        assert_eq!(Conf::parse("block = x.com\n").old_sites, vec!["x.com"]);
    }

    #[test]
    fn a_group_is_read_from_its_form() {
        let g = group_from("", " News ", "bbc.co.uk, https://www.cnn.com/x bbc.co.uk").unwrap();
        assert_eq!((g.name.as_str(), g.sites.clone()), ("News", vec!["bbc.co.uk".to_string(), "cnn.com".into()]));
        assert!(group_from("", "", "x.com").unwrap_err().0, "the name is what's wrong");
        assert!(!group_from("", "N", "x.com nope").unwrap_err().0, "a site is");
        assert_eq!(group_summary(&g), "2 sites: bbc.co.uk, cnn.com");
    }

    #[test]
    fn it_is_the_users_own_file() {
        assert!(conf_path().ends_with("wadbrowser/wadbrowser.conf"));
    }
}
