//! The sidebar's Apps tab (the spike's `SideApps.svelte`): every app, by
//! kind (Internet, Development, Office, Graphics, Multimedia, Games), ✓ on
//! the ones in it. A click adds one (`apps.add`) or takes it out (its chip,
//! `space.remove`: Undo's in the toast). Drag one onto a stack or a
//! wadspace to add it there, or onto open desktop for a wadspace of just it.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use wad_model::catalog::{app, app_name, apps};
use wad_model::edit::{Op, Payload};
use wad_model::{AppId, Section, Wadspace};

use super::ctl::{self, Row};
use super::kit;
use crate::builder::{Builder, Tab};
use crate::dnd::{self, Held};

/// The kinds, in the spike's order (any other after).
pub const ORDER: [&str; 6] = ["Internet", "Development", "Office", "Graphics", "Multimedia", "Games"];

/// Every app by kind, in order.
pub fn kinds() -> Vec<(String, Vec<AppId>)> {
    let mut names: Vec<String> = ORDER.iter().map(|s| s.to_string()).collect();
    for a in apps() {
        if !names.contains(&a.category) {
            names.push(a.category.clone());
        }
    }
    names
        .into_iter()
        .map(|k| {
            let ids = apps().iter().filter(|a| a.category == k).map(|a| a.id).collect::<Vec<_>>();
            (k, ids)
        })
        .filter(|(_, ids)| !ids.is_empty())
        .collect()
}

/// A click on `a`: in, or out (its chip, so the toast has Undo).
pub fn toggle(x: &Wadspace, a: AppId) -> Op {
    if x.has_app(a) {
        Op::Remove { ws: x.id.clone(), chip: format!("app:{a}") }
    } else {
        Op::AddApp { ws: x.id.clone(), app: a }
    }
}

/// What dragging `a` carries.
pub fn piece(a: AppId) -> Held {
    Held::Part { payload: Payload::Piece { section: Section::Apps, item: a.as_str().to_string(), from: None } }
}

struct AppRow {
    button: gtk::Button,
    check: wad_gtk::Icon,
}

pub struct Panel {
    builder: Weak<Builder>,
    root: gtk::Box,
    rows: Vec<(AppId, AppRow)>,
    /// The wadspace shown: its id, and whether it can be changed here.
    now: RefCell<Option<(String, bool)>>,
}

impl Panel {
    pub fn new(b: &Rc<Builder>) -> Rc<Self> {
        kit::install_css();
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["side-apps"]).build();
        let me = Rc::new_cyclic(|me: &Weak<Panel>| {
            let mut rows = vec![];
            for (i, (kind, ids)) in kinds().into_iter().enumerate() {
                let h = kit::heading(&kind);
                if i == 0 {
                    h.add_css_class("first");
                }
                root.append(&h);
                for a in ids {
                    let row = app_row(me, a);
                    root.append(&row.button);
                    rows.push((a, row));
                }
            }
            Panel { builder: Rc::downgrade(b), root, rows, now: RefCell::default() }
        });
        let rows: Weak<dyn ctl::Rows> = Rc::downgrade(&me) as Weak<dyn ctl::Rows>;
        ctl::register(b, Tab::Apps, rows);
        me
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// The selected wadspace, as it is now.
    pub fn show(&self, x: &Wadspace, editable: bool) {
        *self.now.borrow_mut() = Some((x.id.clone(), editable));
        for (a, row) in &self.rows {
            let has = x.has_app(*a);
            if has {
                row.button.add_css_class("in");
            } else {
                row.button.remove_css_class("in");
            }
            row.check.set_visible(has);
            row.button.set_sensitive(editable);
            let tip = if has {
                format!("{}: in it (click to take it out)", app_name(*a))
            } else {
                app(*a).map(|c| c.summary.clone()).unwrap_or_default()
            };
            row.button.set_tooltip_text(Some(&tip));
        }
    }

    fn click(&self, a: AppId) {
        let Some(b) = self.builder.upgrade() else { return };
        let Some((id, true)) = self.now.borrow().clone() else { return };
        let Some(x) = b.wadspace(&id) else { return };
        b.shell.op(toggle(&x, a), |_| {});
    }
}

fn app_row(me: &Weak<Panel>, a: AppId) -> AppRow {
    let button = gtk::Button::builder().css_classes(["side-app"]).build();
    button.set_widget_name(&format!("app:{a}"));
    let h = gtk::Box::builder().spacing(12).build();
    h.append(&wad_gtk::app_icon::app(a, 24));
    h.append(
        &gtk::Label::builder()
            .label(app_name(a))
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["side-app-name"])
            .build(),
    );
    let check = wad_gtk::Icon::new("check", 15);
    check.add_css_class("side-check");
    check.set_visible(false);
    h.append(&check);
    let grip = wad_gtk::Icon::new("grip-vertical", 14);
    grip.add_css_class("side-grip");
    h.append(&grip);
    button.set_child(Some(&h));
    let w = me.clone();
    button.connect_clicked(move |_| {
        if let Some(p) = w.upgrade() {
            p.click(a);
        }
    });
    let look = button.downgrade();
    dnd::source(&button, move |_, _| Some((piece(a), look.upgrade()?.upcast())));
    AppRow { button, check }
}

impl ctl::Rows for Panel {
    fn rows(&self) -> Vec<Row> {
        self.rows
            .iter()
            .map(|(a, r)| Row::new(format!("app:{a}"), &r.button, app_name(*a)).lit(r.button.has_css_class("in")))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_app_by_kind_in_the_spikes_order() {
        let ks = kinds();
        let names: Vec<&str> = ks.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["Internet", "Development", "Office", "Graphics", "Multimedia", "Games"]);
        let all: usize = ks.iter().map(|(_, ids)| ids.len()).sum();
        assert_eq!(all, AppId::ALL.len());
        assert_eq!(ks[0].1, [AppId::Wadbrowser]);
        assert!(ks[1].1.contains(&AppId::Terminal));
    }

    #[test]
    fn a_click_adds_it_or_takes_its_chip_out() {
        let mut x = wad_model::ops::blank("art", "Art", "m-surface", 0, 1);
        assert_eq!(toggle(&x, AppId::Krita), Op::AddApp { ws: "art".into(), app: AppId::Krita });
        wad_model::ops::add_app(&mut x, AppId::Krita);
        assert_eq!(toggle(&x, AppId::Krita), Op::Remove { ws: "art".into(), chip: "app:krita".into() });
        assert_eq!(
            piece(AppId::Krita),
            Held::Part { payload: Payload::Piece { section: Section::Apps, item: "krita".into(), from: None } }
        );
    }
}
