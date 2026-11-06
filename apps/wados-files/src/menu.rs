//! Files' right-click menus (the spike's `ui/MenuList.svelte`), as data so
//! `--dump --menu` can list them: rows with an icon, a label, a hint (or
//! why it's off) under it and a shortcut, headings, separators, and a row
//! with a submenu (Open with ▸), which opens in its place.

use std::rc::Rc;

use gtk::{gdk, glib, prelude::*};
use wad_gtk::Icon;

pub type Run = Rc<dyn Fn()>;

pub enum Row {
    Heading(String),
    Sep,
    Item(Item),
}

pub struct Item {
    pub id: String,
    pub label: String,
    pub icon: &'static str,
    pub hint: Option<String>,
    pub shortcut: Option<&'static str>,
    /// Off, and why.
    pub off: Option<String>,
    pub danger: bool,
    pub checked: bool,
    pub run: Option<Run>,
    /// Its own rows (Open with ▸), shown in its place when it's picked.
    pub sub: Option<Vec<Row>>,
}

pub fn item(id: &str, label: impl Into<String>, icon: &'static str, run: impl Fn() + 'static) -> Item {
    Item {
        id: id.into(),
        label: label.into(),
        icon,
        hint: None,
        shortcut: None,
        off: None,
        danger: false,
        checked: false,
        run: Some(Rc::new(run)),
        sub: None,
    }
}

impl Item {
    pub fn hint(mut self, h: impl Into<String>) -> Self {
        self.hint = Some(h.into());
        self
    }
    pub fn key(mut self, k: &'static str) -> Self {
        self.shortcut = Some(k);
        self
    }
    pub fn off(mut self, why: Option<impl Into<String>>) -> Self {
        self.off = why.map(Into::into);
        self
    }
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }
    pub fn sub(mut self, rows: Vec<Row>) -> Self {
        self.sub = Some(rows);
        self.run = None;
        self
    }
    pub fn row(self) -> Row {
        Row::Item(self)
    }
}

/// The rows in words (`--dump --menu`): `label` (`label (off: why)`, `✓ label`,
/// `label ▸ [sub; rows]`), `—` for a separator, `# HEADING`.
pub fn words(rows: &[Row]) -> Vec<String> {
    rows.iter()
        .map(|r| match r {
            Row::Heading(h) => format!("# {h}"),
            Row::Sep => "—".into(),
            Row::Item(i) => {
                let mut s = if i.checked { format!("✓ {}", i.label) } else { i.label.clone() };
                if let Some(why) = &i.off {
                    s.push_str(&format!(" (off: {why})"));
                }
                if let Some(sub) = &i.sub {
                    s.push_str(&format!(" ▸ [{}]", words(sub).join("; ")));
                }
                s
            }
        })
        .collect()
}

/// `rows` in a popover on `on` at (x, y) of it.
pub fn show(on: &impl IsA<gtk::Widget>, x: f64, y: f64, rows: Vec<Row>) {
    let pop = gtk::Popover::builder()
        .has_arrow(false)
        .position(gtk::PositionType::Bottom)
        .halign(gtk::Align::Start)
        .css_classes(["files-menu"])
        .build();
    pop.set_parent(on);
    pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    fill(&pop, rows);
    pop.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    pop.popup();
}

fn fill(pop: &gtk::Popover, rows: Vec<Row>) {
    let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["files-menu-list"]).build();
    for r in rows {
        match r {
            Row::Heading(h) => col.append(
                &gtk::Label::builder()
                    .label(h.to_uppercase())
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .max_width_chars(32)
                    .css_classes(["files-menu-heading"])
                    .build(),
            ),
            Row::Sep => col.append(&gtk::Box::builder().css_classes(["files-menu-sep"]).build()),
            Row::Item(it) => {
                let b = button(&it);
                b.set_sensitive(it.off.is_none());
                let p = pop.downgrade();
                let (run, sub) = (it.run.clone(), std::cell::RefCell::new(it.sub));
                b.connect_clicked(move |_| {
                    let Some(p) = p.upgrade() else { return };
                    if let Some(rows) = sub.borrow_mut().take() {
                        // Its submenu, in its place.
                        fill(&p, rows);
                        return;
                    }
                    p.popdown();
                    if let Some(r) = run.clone() {
                        glib::idle_add_local_once(move || r());
                    }
                });
                col.append(&b);
            }
        }
    }
    pop.set_child(Some(&col));
}

fn button(it: &Item) -> gtk::Button {
    let b = gtk::Button::builder().css_classes(["files-menu-item"]).build();
    if it.danger {
        b.add_css_class("danger");
    }
    b.set_widget_name(&it.id);
    let h = gtk::Box::builder().spacing(10).build();
    let ic = gtk::Box::builder().valign(gtk::Align::Center).build();
    ic.set_size_request(16, 16);
    let name = if it.checked { "check" } else { it.icon };
    if !name.is_empty() {
        ic.append(&Icon::new(name, 15));
    }
    h.append(&ic);
    let t =
        gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).valign(gtk::Align::Center).build();
    t.append(&gtk::Label::builder().label(&it.label).xalign(0.0).css_classes(["files-menu-label"]).build());
    if let Some(h2) = it.off.as_ref().filter(|w| !w.is_empty()).or(it.hint.as_ref()) {
        t.append(
            &gtk::Label::builder()
                .label(h2)
                .xalign(0.0)
                .wrap(true)
                .max_width_chars(30)
                .css_classes(["files-menu-hint"])
                .build(),
        );
    }
    h.append(&t);
    if it.sub.is_some() {
        h.append(&Icon::new("chevron-right", 14));
    } else if let Some(k) = it.shortcut {
        h.append(&gtk::Label::builder().label(k).css_classes(["files-menu-kbd"]).valign(gtk::Align::Center).build());
    }
    b.set_child(Some(&h));
    b
}

pub const CSS: &str = r#"
popover.files-menu > contents { padding: 6px; border-radius: 12px; border: 1px solid var(--line-strong); background-color: var(--surface); box-shadow: 0 18px 40px -12px rgb(0 0 0 / 0.6); }
.files-menu-list { min-width: 230px; }
.files-menu-heading { margin: 6px 10px 4px; font-size: 10.5px; font-weight: 700; letter-spacing: 0.07em; color: var(--faint); }
.files-menu-sep { min-height: 1px; margin: 5px 6px; background-color: var(--line); }
button.files-menu-item { padding: 6px 10px; border-radius: 8px; border: none; background: none; box-shadow: none; color: var(--fg); }
button.files-menu-item:hover { background-color: var(--surface-2); }
button.files-menu-item:disabled { opacity: 0.5; }
button.files-menu-item.danger { color: var(--danger); }
.files-menu-label { font-size: 13px; }
.files-menu-hint { font-size: 11.5px; color: var(--muted); }
.files-menu-kbd { font-size: 11px; color: var(--faint); margin-left: 12px; }
"#;
