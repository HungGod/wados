//! What the sidebar's Apps, Add-ons and Secrets tabs share: their look
//! (`side.css`), icons, sending edits, a key's dropdown (the spike's
//! `ui/MenuList.svelte`) and the small dialog that asks for a name (its
//! `surface.ask`), and doing what a dropdown's choice says ([`act`]).
//!
//! The menu and the dialog open are kept here, so `wadsctl side` can list
//! and pick from them. Opened by wadsctl (no click to grab with), they
//! stay until they're used or closed.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::{gdk, glib, prelude::*};
use wad_model::edit::Op;

use super::addon_list::ItemIcon;
use super::keys::{Act, Choice, Pick, kind_name};
use crate::builder::Builder;
use crate::shell::Shell;

const CSS: &str = include_str!("side.css");

thread_local! {
    static CSS_IN: Cell<bool> = const { Cell::new(false) };
    static BY_CTL: Cell<bool> = const { Cell::new(false) };
    static MENU: RefCell<Option<OpenMenu>> = const { RefCell::new(None) };
    static ASK: RefCell<Option<OpenAsk>> = const { RefCell::new(None) };
}

/// The tabs' stylesheet, once.
pub fn install_css() {
    if CSS_IN.with(|c| c.replace(true)) {
        return;
    }
    if let Some(d) = gdk::Display::default() {
        wad_gtk::theme::add_css(&d, CSS);
    }
}

/// `f`, as wadsctl's: menus and dialogs it opens don't grab (there's no click to grab with).
pub fn by_ctl<R>(f: impl FnOnce() -> R) -> R {
    let was = BY_CTL.with(|c| c.replace(true));
    let r = f();
    BY_CTL.with(|c| c.set(was));
    r
}

fn from_ctl() -> bool {
    BY_CTL.with(Cell::get)
}

// ------------------------------------------------------------ edits

/// Edits sent one after another (each once the one before is done; a refusal, toasted, stops the rest).
pub fn send_all(shell: &Rc<Shell>, ops: Vec<Op>) {
    let mut ops = ops.into_iter();
    let Some(first) = ops.next() else { return };
    let rest: Vec<Op> = ops.collect();
    let s = shell.clone();
    shell.op(first, move |_| send_all(&s, rest));
}

/// What a dropdown's choice says: its edits, or a new key (named first), then what signs in with it.
pub fn act(b: &Rc<Builder>, anchor: &gtk::Widget, a: &Act) {
    match a {
        Act::Ops(ops) => send_all(&b.shell, ops.clone()),
        Act::NewKey { kind, then } => {
            let (shell, kind, then) = (b.shell.clone(), *kind, then.clone());
            ask(
                anchor,
                &format!("A new {} key", kind_name(kind)),
                &[Field { key: "name", label: "Name", placeholder: "you@example.com", text: String::new() }],
                "Add key",
                move |v| {
                    let name = v[0].trim().to_string();
                    if name.is_empty() {
                        return Err("Give it a name".into());
                    }
                    let (s, then) = (shell.clone(), then.clone());
                    shell.op(Op::NewKey { name, kind }, move |r| {
                        if let Some(id) = r.created {
                            send_all(&s, then.ops(Some(&id)));
                        }
                    });
                    Ok(())
                },
            );
        }
    }
}

// ------------------------------------------------------------ icons

/// A picture of WadSpaces' own ('/icons/claude.png'), `size` px square.
pub fn image(path: &str, size: i32) -> gtk::Widget {
    let t = wad_gtk::picture::resolve(path).and_then(|p| wad_gtk::picture::texture(&p, (size * 2) as u32));
    let img = match t {
        Some(t) => gtk::Image::from_paintable(Some(&t)),
        None => gtk::Image::new(),
    };
    img.set_pixel_size(size);
    img.add_css_class("side-img");
    img.upcast()
}

/// An add-on's icon.
pub fn item_icon(icon: &ItemIcon, size: i32) -> gtk::Widget {
    match icon {
        ItemIcon::Site { id, name, url } => wad_gtk::app_icon::site(id, Some(name), url.as_deref(), size),
        ItemIcon::Img(p) => image(p, size),
        ItemIcon::Glyph(g) => wad_gtk::Icon::new(g, size - 3).upcast(),
    }
}

/// A heading over a part of a tab ("INTERNET", "SUGGESTED").
pub fn heading(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text.to_uppercase()).xalign(0.0).css_classes(["side-h5"]).build()
}

// ------------------------------------------------------------ menus

/// An item of the menu open, as wadsctl sees it.
#[derive(Clone)]
pub struct Shown {
    pub id: String,
    pub on: bool,
    pub checked: bool,
    pub row: gtk::Widget,
}

struct OpenMenu {
    popover: gtk::Popover,
    /// What it's for, and its items with what each does.
    of: String,
    items: Vec<(Shown, Rc<dyn Fn()>)>,
}

/// `choices` in a dropdown under `anchor` (or at `at`, its pixels), for
/// `of` (what opened it, for wadsctl); `run` does a choice.
pub fn menu(
    anchor: &gtk::Widget,
    at: Option<(f64, f64)>,
    of: &str,
    choices: Vec<Choice>,
    run: impl Fn(&Pick) + 'static,
) {
    close_menu();
    let popover = gtk::Popover::builder()
        .has_arrow(false)
        .autohide(!from_ctl())
        .position(gtk::PositionType::Bottom)
        .halign(gtk::Align::Start)
        .css_classes(["side-menu"])
        .build();
    popover.set_parent(anchor);
    let rect = match at {
        Some((x, y)) => gdk::Rectangle::new(x.round() as i32, y.round() as i32, 1, 1),
        None => gdk::Rectangle::new(0, 0, anchor.width(), anchor.height()),
    };
    popover.set_pointing_to(Some(&rect));
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["side-menu-list"]).build();
    if at.is_none() {
        list.set_size_request(anchor.width().max(200), -1);
    }
    let run = Rc::new(run);
    let mut items = vec![];
    for c in choices {
        match c {
            Choice::Heading(h) => list.append(
                &gtk::Label::builder()
                    .label(h.to_uppercase())
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .max_width_chars(32)
                    .css_classes(["side-menu-heading"])
                    .build(),
            ),
            Choice::Sep => list.append(&gtk::Box::builder().css_classes(["side-menu-sep"]).build()),
            Choice::Item(p) => {
                let b = menu_row(&p);
                let on = p.off.is_none();
                b.set_sensitive(on);
                let (id, checked) = (p.id.clone(), p.checked);
                let go: Rc<dyn Fn()> = {
                    let (run, pop) = (run.clone(), popover.downgrade());
                    Rc::new(move || {
                        if let Some(pop) = pop.upgrade() {
                            pop.popdown();
                        }
                        // After the menu's gone (a choice may open a dialog: wadsctl's, if this was).
                        let (run, p, ctl) = (run.clone(), p.clone(), from_ctl());
                        glib::idle_add_local_once(move || {
                            if ctl {
                                by_ctl(|| run(&p));
                            } else {
                                run(&p);
                            }
                        });
                    })
                };
                let g = go.clone();
                b.connect_clicked(move |_| g());
                list.append(&b);
                items.push((Shown { id, on, checked, row: b.upcast() }, go));
            }
        }
    }
    popover.set_child(Some(&list));
    let me = popover.downgrade();
    popover.connect_closed(move |p| {
        MENU.with(|m| {
            let mut m = m.borrow_mut();
            if m.as_ref().is_some_and(|o| Some(&o.popover) == me.upgrade().as_ref()) {
                *m = None;
            }
        });
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    popover.popup();
    MENU.with(|m| *m.borrow_mut() = Some(OpenMenu { popover, of: of.to_string(), items }));
}

fn menu_row(p: &Pick) -> gtk::Button {
    let b = gtk::Button::builder().css_classes(["side-menu-item"]).build();
    if p.checked {
        b.add_css_class("checked");
    }
    b.set_widget_name(&p.id);
    let h = gtk::Box::builder().spacing(10).build();
    let ic = gtk::Box::builder().css_classes(["side-menu-ic"]).valign(gtk::Align::Center).build();
    ic.set_size_request(16, 16);
    ic.append(&wad_gtk::Icon::new(if p.checked { "check" } else { p.icon }, 15));
    h.append(&ic);
    let text =
        gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).valign(gtk::Align::Center).build();
    text.append(
        &gtk::Label::builder()
            .label(&p.label)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(30)
            .css_classes(["side-menu-label"])
            .build(),
    );
    if let Some(why) = &p.off {
        text.append(
            &gtk::Label::builder()
                .label(why)
                .xalign(0.0)
                .wrap(true)
                .max_width_chars(30)
                .css_classes(["side-menu-hint"])
                .build(),
        );
        b.set_tooltip_text(Some(why));
    }
    h.append(&text);
    b.set_child(Some(&h));
    b
}

/// The menu open: what it's for, and its items.
pub fn open_menu() -> Option<(String, Vec<Shown>)> {
    MENU.with(|m| m.borrow().as_ref().map(|o| (o.of.clone(), o.items.iter().map(|(s, _)| s.clone()).collect())))
}

/// Picks `id` from the menu open.
pub fn choose(id: &str) -> Result<(), String> {
    let go = MENU.with(|m| {
        let m = m.borrow();
        let Some(o) = m.as_ref() else { return Err("no menu is open".to_string()) };
        match o.items.iter().find(|(s, _)| s.id == id) {
            Some((s, _)) if !s.on => Err(format!("{id} is off")),
            Some((_, go)) => Ok(go.clone()),
            None => Err(format!("no {id} in it")),
        }
    })?;
    go();
    Ok(())
}

pub fn close_menu() {
    if let Some(o) = MENU.with(|m| m.borrow_mut().take()) {
        o.popover.popdown();
    }
}

// ------------------------------------------------------------ asking

/// A field of a dialog.
#[derive(Clone)]
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub placeholder: &'static str,
    /// What's in it to begin with.
    pub text: String,
}

struct OpenAsk {
    popover: gtk::Popover,
    title: String,
    fields: Vec<(&'static str, gtk::Entry)>,
    ok: gtk::Button,
}

/// A small dialog by `anchor`: `title`, `fields`, and `ok_label`'s button
/// (or Enter). `done` gets what's in the fields; an error is said in it
/// (it stays), else it closes.
pub fn ask(
    anchor: &gtk::Widget,
    title: &str,
    fields: &[Field],
    ok_label: &str,
    done: impl Fn(&[String]) -> Result<(), String> + 'static,
) {
    close_ask();
    let popover = gtk::Popover::builder()
        .has_arrow(false)
        .autohide(!from_ctl())
        .position(gtk::PositionType::Bottom)
        .halign(gtk::Align::Start)
        .css_classes(["side-ask"])
        .build();
    popover.set_parent(anchor);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(0, 0, anchor.width(), anchor.height())));
    let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
    col.set_size_request(anchor.width().clamp(240, 320), -1);
    col.append(&gtk::Label::builder().label(title).xalign(0.0).css_classes(["side-ask-title"]).build());
    let mut entries = vec![];
    for f in fields {
        let one = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
        one.append(&gtk::Label::builder().label(f.label).xalign(0.0).css_classes(["side-ask-label"]).build());
        let e = gtk::Entry::builder()
            .placeholder_text(f.placeholder)
            .text(&f.text)
            .hexpand(true)
            .css_classes(["side-ask-entry"])
            .build();
        e.set_widget_name(f.key);
        one.append(&e);
        col.append(&one);
        entries.push((f.key, e));
    }
    let err = gtk::Label::builder().xalign(0.0).wrap(true).visible(false).css_classes(["side-err"]).build();
    col.append(&err);
    let row = gtk::Box::builder().spacing(8).halign(gtk::Align::End).build();
    let cancel = gtk::Button::builder().label("Cancel").css_classes(["side-ask-cancel"]).build();
    let ok = gtk::Button::builder().label(ok_label).css_classes(["side-ask-ok"]).build();
    row.append(&cancel);
    row.append(&ok);
    col.append(&row);
    popover.set_child(Some(&col));

    let done = Rc::new(done);
    let submit: Rc<dyn Fn()> = {
        let (entries, pop, err) = (entries.clone(), popover.downgrade(), err.clone());
        Rc::new(move || {
            let v: Vec<String> = entries.iter().map(|(_, e)| e.text().to_string()).collect();
            match done(&v) {
                Ok(()) => {
                    if let Some(p) = pop.upgrade() {
                        p.popdown();
                    }
                }
                Err(e) => {
                    err.set_label(&e);
                    err.set_visible(true);
                }
            }
        })
    };
    let s = submit.clone();
    ok.connect_clicked(move |_| s());
    for (_, e) in &entries {
        let s = submit.clone();
        e.connect_activate(move |_| s());
        let err = err.clone();
        e.connect_changed(move |_| err.set_visible(false));
    }
    let pop = popover.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(p) = pop.upgrade() {
            p.popdown();
        }
    });
    let me = popover.downgrade();
    popover.connect_closed(move |p| {
        ASK.with(|a| {
            let mut a = a.borrow_mut();
            if a.as_ref().is_some_and(|o| Some(&o.popover) == me.upgrade().as_ref()) {
                *a = None;
            }
        });
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    popover.popup();
    if let Some((_, e)) = entries.first() {
        e.grab_focus();
    }
    ASK.with(|a| *a.borrow_mut() = Some(OpenAsk { popover, title: title.to_string(), fields: entries, ok }));
}

/// The dialog open, as wadsctl sees it: its title, its fields (each its
/// key, what's in it, its entry), and its OK.
pub struct Asking {
    pub title: String,
    pub fields: Vec<(String, String, gtk::Widget)>,
    pub ok: gtk::Widget,
}

pub fn open_ask() -> Option<Asking> {
    ASK.with(|a| {
        a.borrow().as_ref().map(|o| Asking {
            title: o.title.clone(),
            fields: o.fields.iter().map(|(k, e)| (k.to_string(), e.text().to_string(), e.clone().upcast())).collect(),
            ok: o.ok.clone().upcast(),
        })
    })
}

/// Types `text` into the open dialog's `field`.
pub fn fill(field: &str, text: &str) -> Result<(), String> {
    ASK.with(|a| {
        let a = a.borrow();
        let o = a.as_ref().ok_or("no dialog is open")?;
        let (_, e) = o.fields.iter().find(|(k, _)| *k == field).ok_or(format!("no field {field}"))?;
        e.set_text(text);
        Ok(())
    })
}

/// The open dialog's OK.
pub fn ask_ok() -> Result<(), String> {
    let ok = ASK.with(|a| a.borrow().as_ref().map(|o| o.ok.clone())).ok_or("no dialog is open")?;
    ok.emit_clicked();
    Ok(())
}

pub fn close_ask() {
    if let Some(o) = ASK.with(|a| a.borrow_mut().take()) {
        o.popover.popdown();
    }
}
