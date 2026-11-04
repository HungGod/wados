//! The launcher's bar along its top (Home.svelte's `header.bar`): the
//! machine at its start (click it for the list: this one and my others
//! online; a chevron when there are others), a divider, then the tabs
//! (Running and Recent, a small divider, the tags), each with how many it
//! has. The tab shown is white; a line under the one the keys are on.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;

use super::model::Tab;

/// One tab as the bar shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct TabView {
    pub tab: Tab,
    pub label: String,
    pub count: usize,
}

struct Row {
    tab: Tab,
    root: gtk::Overlay,
    label: gtk::Label,
    count: gtk::Label,
    line: gtk::Box,
}

pub struct Bar {
    pub root: gtk::Box,
    pub machine: gtk::Overlay,
    machine_icon: wad_gtk::Icon,
    machine_more: wad_gtk::Icon,
    machine_label: gtk::Label,
    machine_line: gtk::Box,
    tabs: gtk::Box,
    rows: RefCell<Vec<Row>>,
}

/// The line under what the keys are on.
fn line() -> gtk::Box {
    gtk::Box::builder()
        .css_classes(["home-line"])
        .valign(gtk::Align::End)
        .halign(gtk::Align::Fill)
        .visible(false)
        .can_target(false)
        .build()
}

fn divider(small: bool) -> gtk::Box {
    let d = gtk::Box::builder().css_classes(["home-divider"]).valign(gtk::Align::Center).build();
    if small {
        d.add_css_class("small");
    }
    d
}

impl Bar {
    pub fn new() -> Self {
        let root = gtk::Box::builder().spacing(10).css_classes(["home-bar"]).valign(gtk::Align::Center).build();
        let machine = gtk::Overlay::builder().css_classes(["home-machine"]).build();
        let inner = gtk::Box::builder().spacing(10).css_classes(["home-machine-in"]).build();
        let machine_icon = wad_gtk::Icon::new("house", 22);
        inner.append(&machine_icon);
        let machine_label = gtk::Label::new(None);
        inner.append(&machine_label);
        let machine_more = wad_gtk::Icon::new("chevron-down", 18);
        machine_more.set_valign(gtk::Align::Center);
        machine_more.set_visible(false);
        inner.append(&machine_more);
        machine.set_child(Some(&inner));
        let machine_line = line();
        machine.add_overlay(&machine_line);
        machine.set_tooltip_text(Some("This machine’s wadspaces"));
        root.append(&machine);
        root.append(&divider(false));
        let tabs = gtk::Box::builder().spacing(6).valign(gtk::Align::Center).build();
        root.append(&tabs);
        Self { root, machine, machine_icon, machine_more, machine_label, machine_line, tabs, rows: RefCell::default() }
    }

    /// The machine shown: its name and glyph; `others`, there are others to pick.
    pub fn set_machine(&self, name: &str, icon: &'static str, others: bool) {
        if self.machine_label.label() != name {
            self.machine_label.set_label(name);
        }
        self.machine_icon.set(icon);
        self.machine_more.set_visible(others);
        self.machine.set_tooltip_text(Some(if others {
            "Another machine’s wadspaces"
        } else {
            "This machine’s wadspaces"
        }));
    }

    /// Shows `tabs`, `view` the one shown, the keys on `keyed` (0 the
    /// machine, then each tab by its place + 1); a click on a tab calls
    /// `clicked` with its place.
    pub fn set(&self, tabs: &[TabView], view: &Tab, keyed: Option<usize>, clicked: &Rc<dyn Fn(usize)>) {
        self.machine_line.set_visible(keyed == Some(0));
        let same = {
            let rows = self.rows.borrow();
            rows.len() == tabs.len() && rows.iter().zip(tabs).all(|(r, t)| r.tab == t.tab)
        };
        if !same {
            while let Some(c) = self.tabs.first_child() {
                self.tabs.remove(&c);
            }
            let mut rows = Vec::with_capacity(tabs.len());
            for (i, t) in tabs.iter().enumerate() {
                let row = tab_row(t);
                let click = gtk::GestureClick::new();
                let f = clicked.clone();
                click.connect_released(move |g, n, _, _| {
                    if n == 1 {
                        g.set_state(gtk::EventSequenceState::Claimed);
                        f(i);
                    }
                });
                row.root.add_controller(click);
                self.tabs.append(&row.root);
                if t.tab == Tab::Recent {
                    self.tabs.append(&divider(true));
                }
                rows.push(row);
            }
            *self.rows.borrow_mut() = rows;
        }
        for (i, (r, t)) in self.rows.borrow().iter().zip(tabs).enumerate() {
            if r.label.label() != t.label {
                r.label.set_label(&t.label);
            }
            let count = t.count.to_string();
            if r.count.label() != count {
                r.count.set_label(&count);
            }
            if &t.tab == view {
                r.root.add_css_class("on");
            } else {
                r.root.remove_css_class("on");
            }
            r.line.set_visible(keyed == Some(i + 1));
        }
    }

    /// The tabs' widgets, in order (for wadsctl's bounds).
    pub fn tab_widgets(&self) -> Vec<gtk::Overlay> {
        self.rows.borrow().iter().map(|r| r.root.clone()).collect()
    }
}

fn tab_row(t: &TabView) -> Row {
    let root = gtk::Overlay::builder().css_classes(["home-tab"]).build();
    let inner = gtk::Box::builder().spacing(7).css_classes(["home-tab-in"]).build();
    let label = gtk::Label::builder().label(&t.label).valign(gtk::Align::BaselineFill).build();
    let count = gtk::Label::builder()
        .label(t.count.to_string())
        .css_classes(["home-count"])
        .valign(gtk::Align::BaselineFill)
        .build();
    inner.append(&label);
    inner.append(&count);
    root.set_child(Some(&inner));
    let line = line();
    root.add_overlay(&line);
    Row { tab: t.tab.clone(), root, label, count, line }
}
