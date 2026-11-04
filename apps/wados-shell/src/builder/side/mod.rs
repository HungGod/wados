//! The Builder's sidebar: it edits the selected stack, in four tabs (each
//! its icon over its name, in its section's colour), and the tab's panel:
//!
//! | Tab | Panel | The spike's |
//! |---|---|---|
//! | Apps | [`apps`] | `SideApps.svelte` |
//! | Add-ons | [`addons`] | `SideAddons.svelte`, `addons.ts` |
//! | Secrets | [`secrets`] | `SideSecrets.svelte`, `keys.ts` |
//! | Customize | [`custom`] | `SideCustom.svelte`, `IconEditor.svelte`, `PhotoPicker.svelte` |
//!
//! Every panel has the same face: `new(builder)`, `widget()`, and
//! `show(x, editable)`, told the selected wadspace as it is now each time it
//! (or the library) changes while its tab shows, and when it comes into view.

pub mod addons;
pub mod apps;
pub mod custom;
pub mod secrets;
// What Apps, Add-ons and Secrets share: their lists and choices (pure), and their widgets.
pub mod addon_list;
mod ctl;
pub mod keys;
mod kit;

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::prelude::*;

use super::{Builder, Tab};

pub struct Sidebar {
    builder: Weak<Builder>,
    root: gtk::Box,
    tabs: Vec<(Tab, gtk::Button)>,
    pages: gtk::Stack,
    apps: Rc<apps::Panel>,
    addons: Rc<addons::Panel>,
    secrets: Rc<secrets::Panel>,
    custom: Rc<custom::Panel>,
    shown: RefCell<Option<(Tab, String)>>,
}

impl Sidebar {
    pub fn new(b: &Rc<Builder>) -> Rc<Self> {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["builder-side"]).build();
        let head = gtk::Box::builder().css_classes(["builder-side-head"]).build();
        let row = gtk::Box::builder().homogeneous(true).spacing(2).hexpand(true).css_classes(["builder-tabs"]).build();
        let mut tabs = vec![];
        for t in Tab::ALL {
            let (label, icon) = match t {
                Tab::Apps => ("Apps", "layout-grid"),
                Tab::Addons => ("Add-ons", "puzzle"),
                Tab::Secrets => ("Secrets", "key-round"),
                Tab::Customize => ("Customize", "palette"),
            };
            let col = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(3)
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .build();
            let i = wad_gtk::Icon::new(icon, 17);
            i.set_halign(gtk::Align::Center);
            col.append(&i);
            col.append(&gtk::Label::builder().label(label).ellipsize(gtk::pango::EllipsizeMode::End).build());
            let button = gtk::Button::builder().child(&col).css_classes(["builder-tab", t.id()]).build();
            let w = Rc::downgrade(b);
            button.connect_clicked(move |_| {
                if let Some(b) = w.upgrade() {
                    b.set_tab(t);
                }
            });
            row.append(&button);
            tabs.push((t, button));
        }
        head.append(&row);
        root.append(&head);

        let pages = gtk::Stack::builder().vexpand(true).transition_type(gtk::StackTransitionType::None).build();
        let apps = apps::Panel::new(b);
        let addons = addons::Panel::new(b);
        let secrets = secrets::Panel::new(b);
        let custom = custom::Panel::new(b);
        for (t, w) in [
            (Tab::Apps, apps.widget()),
            (Tab::Addons, addons.widget()),
            (Tab::Secrets, secrets.widget()),
            (Tab::Customize, custom.widget()),
        ] {
            let scroll = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .child(&w)
                .build();
            w.add_css_class("builder-panel");
            pages.add_named(&scroll, Some(t.id()));
        }
        root.append(&pages);
        Rc::new(Self {
            builder: Rc::downgrade(b),
            root,
            tabs,
            pages,
            apps,
            addons,
            secrets,
            custom,
            shown: RefCell::default(),
        })
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// The selected wadspace and tab, as they are now.
    pub fn refresh(&self) {
        let Some(b) = self.builder.upgrade() else { return };
        let Some(x) = b.selected_ws() else {
            self.root.set_visible(false);
            *self.shown.borrow_mut() = None;
            return;
        };
        self.root.set_visible(true);
        let tab = b.tab();
        for (t, button) in &self.tabs {
            if *t == tab {
                button.add_css_class("on");
            } else {
                button.remove_css_class("on");
            }
        }
        // Another wadspace or tab: its panel starts at the top.
        let now = (tab, x.id.clone());
        if self.shown.borrow().as_ref() != Some(&now) {
            self.pages.set_visible_child_name(tab.id());
            if let Some(s) = self.pages.visible_child().and_downcast::<gtk::ScrolledWindow>() {
                s.vadjustment().set_value(0.0);
            }
            *self.shown.borrow_mut() = Some(now);
        }
        let editable = b.editable(&x);
        match tab {
            Tab::Apps => self.apps.show(&x, editable),
            Tab::Addons => self.addons.show(&x, editable),
            Tab::Secrets => self.secrets.show(&x, editable),
            Tab::Customize => self.custom.show(&x, editable),
        }
    }
}
