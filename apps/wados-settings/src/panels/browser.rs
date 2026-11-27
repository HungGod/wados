//! Settings ▸ Browser: WadBrowser's settings on this machine, which the
//! browser has no page for. Its address bar, its ad and tracker blocker,
//! what it searches with, the sites it blocks (social media a tap each, or
//! one typed), and the machine's web apps (made from pages: Remove).
//!
//! They're the user's `~/.config/wadbrowser/wadbrowser.conf` (wad-model's
//! `browser` format), which the browser watches: a change counts in its
//! open windows at once. A wadspace's browser has its own, from its design
//! (the Builder's add-ons).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use wad_gtk::form::{Choice, TextField, Toggle, opt};
use wad_gtk::kit::{self, Group, Row, Tone, Variant};
use wad_model::BrowserApp;
use wad_model::browser::{self, BrowserPrefs, ConfText, SUGGESTED_BLOCKS, Search};
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
"#;

/// The settings as the browser reads them here: the address bar, and the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conf {
    pub address_bar: bool,
    pub prefs: BrowserPrefs,
}

impl Default for Conf {
    fn default() -> Self {
        Conf { address_bar: true, prefs: BrowserPrefs::default() }
    }
}

impl Conf {
    pub fn parse(text: &str) -> Conf {
        let mut c = Conf::default();
        ConfText::parse(text).over(&mut c.prefs, &mut c.address_bar);
        c
    }

    pub fn text(&self) -> String {
        browser::conf(
            "WadBrowser's settings on this machine: Settings ▸ Browser's (the\nbrowser reads them again when they change).",
            self.address_bar,
            &self.prefs,
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
    Rc::new_cyclic(|me| BrowserPanel { me: me.clone(), ui: RefCell::default(), apps: RefCell::default() })
}

struct Ui {
    address: Toggle,
    blocker: Toggle,
    search: Choice<Search>,
    social: Vec<(&'static str, gtk::Button)>,
    own: gtk::Box,
    own_note: gtk::Label,
    field: TextField,
    apps: gtk::Box,
    apps_note: gtk::Label,
    note: gtk::Label,
}

pub struct BrowserPanel {
    me: Weak<BrowserPanel>,
    ui: RefCell<Option<Ui>>,
    /// The machine's web apps, as wadd last said.
    apps: RefCell<Vec<BrowserApp>>,
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

    fn block_typed(&self) {
        let Some(text) = self.ui.borrow().as_ref().map(|u| u.field.text.text().to_string()) else { return };
        let site = browser::site(&text);
        let have = load();
        let why = match &site {
            None => Some("That isn’t a website: type one like example.com".to_string()),
            Some(s) if have.prefs.blocked.contains(s) => Some(format!("{s} is blocked already")),
            Some(_) => None,
        };
        if let Some(ui) = &*self.ui.borrow() {
            if let Some(why) = &why {
                ui.field.set_error(Some(why));
                return;
            }
            ui.field.set_error(None);
            ui.field.text.set_text("");
        }
        if let Some(s) = site {
            self.change(move |c| {
                c.prefs.block(&s);
            });
        }
    }

    /// The page as `c` says.
    fn paint(&self, c: &Conf) {
        let Some(ui) = &*self.ui.borrow() else { return };
        ui.address.set(c.address_bar);
        ui.blocker.set(c.prefs.ad_blocker);
        ui.search.set(c.prefs.search);
        for (site, b) in &ui.social {
            let on = c.prefs.blocked.iter().any(|x| x == site);
            if on {
                b.add_css_class("on");
            } else {
                b.remove_css_class("on");
            }
            b.set_tooltip_text(Some(&if on {
                format!("{site} is blocked: click to unblock it")
            } else {
                format!("Block {site}")
            }));
        }
        while let Some(c) = ui.own.first_child() {
            ui.own.remove(&c);
        }
        let own: Vec<&String> = c.prefs.blocked.iter().filter(|s| !SUGGESTED_BLOCKS.contains(&s.as_str())).collect();
        for site in &own {
            let row = Row::new(Some("ban"), Tone::Plain, site, "And its subdomains").named(&format!("blocked-{site}"));
            let b = kit::button("Unblock", Variant::Secondary, None, true);
            b.set_widget_name(&format!("unblock-{site}"));
            let (w, s) = (self.me.clone(), site.to_string());
            b.connect_clicked(move |_| {
                if let Some(me) = w.upgrade() {
                    let s = s.clone();
                    me.change(move |c| {
                        c.prefs.unblock(&s);
                    });
                }
            });
            row.ctl.append(&b);
            ui.own.append(&row.root);
        }
        ui.own_note.set_visible(own.is_empty());
        ui.own.set_visible(!own.is_empty());
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
        let page = kit::Page::new("Browser", false);

        let main = Group::new(
            "WadBrowser",
            "The browser on this machine. A wadspace’s has its own, in its Builder’s add-ons.",
        );
        let w = self.me.clone();
        let address = Toggle::new(c.address_bar, move |on| {
            if let Some(me) = w.upgrade() {
                me.change(|c| c.address_bar = on);
            }
        })
        .named("browser-address-bar");
        let row =
            Row::new(Some("globe"), Tone::Plain, "Address Bar", "Off: its tabs alone (Focus), no address to type");
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
        let w = self.me.clone();
        let search = Choice::new(Search::ALL.map(|s| opt(s, s.name())), c.prefs.search, move |s: &Search| {
            let s = *s;
            if let Some(me) = w.upgrade() {
                me.change(|c| c.prefs.search = s);
            }
        })
        .named("browser-search");
        let row =
            Row::new(Some("search"), Tone::Plain, "Search Engine", "What the address bar and new tabs search with");
        row.ctl.append(&search.root);
        main.add(&row.root);
        let note = parts::note("", "set-err");
        main.root.append(&note);
        page.add(&main.root);

        let blocked = Group::new("Blocked Sites", "They don’t open in WadBrowser, nor do their subdomains.");
        blocked.list.set_visible(false);
        blocked.root.append(&kit::label("SOCIAL MEDIA", &["br-h4"]));
        let grid = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .column_spacing(8)
            .row_spacing(8)
            .min_children_per_line(3)
            .max_children_per_line(6)
            .build();
        let mut social = vec![];
        for site in SUGGESTED_BLOCKS {
            let b = gtk::Button::builder().label(site).css_classes(["br-site"]).build();
            b.set_widget_name(&format!("block-{site}"));
            let w = self.me.clone();
            b.connect_clicked(move |_| {
                if let Some(me) = w.upgrade() {
                    me.change(|c| {
                        if !c.prefs.unblock(site) {
                            c.prefs.block(site);
                        }
                    });
                }
            });
            grid.insert(&b, -1);
            social.push((site, b));
        }
        blocked.root.append(&grid);
        blocked.root.append(&kit::label("YOUR OWN", &["br-h4"]));
        let own = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["kit-list"]).build();
        blocked.root.append(&own);
        let own_note = parts::note("None of your own yet: type one below.", "set-note");
        blocked.root.append(&own_note);
        let add = gtk::Box::builder().spacing(8).css_classes(["br-add"]).build();
        let field = TextField::new("", "A website to block: example.com").named("browser-block-site");
        field.root.set_hexpand(true);
        add.append(&field.root);
        let block = kit::button("Block", Variant::Primary, Some("ban"), true);
        block.set_widget_name("browser-block");
        block.set_valign(gtk::Align::Start);
        add.append(&block);
        blocked.root.append(&add);
        let w = self.me.clone();
        block.connect_clicked(move |_| {
            if let Some(me) = w.upgrade() {
                me.block_typed();
            }
        });
        let w = self.me.clone();
        field.text.connect_activate(move |_| {
            if let Some(me) = w.upgrade() {
                me.block_typed();
            }
        });
        page.add(&blocked.root);

        let web =
            Group::new("Web Apps", "Made from pages in WadBrowser (right-click a page: Create app from this page).");
        web.list.set_visible(false);
        let apps = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["kit-list"]).build();
        web.root.append(&apps);
        let apps_note = parts::note("None yet.", "set-note");
        web.root.append(&apps_note);
        page.add(&web.root);

        *self.ui.borrow_mut() =
            Some(Ui { address, blocker, search, social, own, own_note, field, apps, apps_note, note });
        self.paint(&c);
        self.paint_apps();
        page.root.upcast()
    }

    fn on_event(&self, _: &Ctx, i: &Incoming) {
        if let Incoming::Event(Event::Library(lib)) = i {
            if *self.apps.borrow() == lib.web_apps {
                return;
            }
            *self.apps.borrow_mut() = lib.web_apps.clone();
            self.paint_apps();
        }
    }

    fn show(&self, _: Option<&str>) {
        // Read again: it may have changed since (by hand, or another Settings).
        self.paint(&load());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_conf_reads_back() {
        let c = Conf {
            address_bar: false,
            prefs: BrowserPrefs { ad_blocker: false, search: Search::Wikipedia, blocked: vec!["x.com".into()] },
        };
        assert_eq!(Conf::parse(&c.text()), c);
        assert_eq!(Conf::parse(""), Conf::default());
        assert!(Conf::default().address_bar && Conf::default().prefs.ad_blocker);
    }

    #[test]
    fn it_is_the_users_own_file() {
        assert!(conf_path().ends_with("wadbrowser/wadbrowser.conf"));
    }
}
