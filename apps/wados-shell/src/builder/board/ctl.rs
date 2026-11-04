//! `wadsctl stack …`: what the board shows, where its parts are on the
//! screen (so a script can aim the pointer), and a thing's menu, opened and
//! picked from.
//!
//! ```text
//! stack                          the stacks: id "name" state [selected] and where each is; the slot
//! stack show <ws>                what its stack shows, part by part (ids as `at` takes them)
//! stack parts <ws>               its parts' ids
//! stack at <ws> [<part> [x]]     ok cx cy x y w h: the part's middle and box on the screen (logical px)
//! stack at slot|board            the slot's (while it shows), the board's
//! stack click <ws> <part> [x]    clicks it: a thing (its menu), its ×, a box's ×, close
//! stack menu <ws> <part>         a thing's menu, opened (as its click does); what's on it
//! stack menu                     the menu open now
//! stack pick <id>                picks from it (a submenu opens; Rename asks)
//! stack at menu <id>             where its item is on the screen
//! stack ask <text>               answers what it asks (Rename), and closes it
//! stack unmenu                   closes it
//! stack pointer                  where the board last saw the pointer: its px, and the screen's
//! stack origin [<x> <y>]         the window's top left on the screen, as worked out (or told)
//! ```
//!
//! A part's id: `top` (its header: drag it, all of it), `close`, a section
//! (`customization`, `secrets`, `addons`, `apps`; `<section>:head` its
//! name), a key's box `key:<key>` (`key:<key>:head`), an app's add-ons'
//! box `box:<app>`, a thing `secrets/<variant>` or `addons/<variant>`
//! (`secrets/ext:wadbrowser:gmail`), an app `apps/<app>`, a command
//! `cmd/<id>`. `x` after one: its ×.

use std::cell::Cell;
use std::rc::Rc;

use gtk::{glib, prelude::*};
use wad_model::catalog::app_name;
use wad_model::stack::{addon_boxes, key_boxes, stack_apps};
use wad_model::variants::find_variant;
use wad_model::wallpapers::wallpaper_name;

use super::{Board, pieces};
use crate::shell::Shell;

thread_local! {
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
}

/// `wadsctl stack …`, once (each window's board answers while it's shown).
/// After this turn of the main loop: a board's made while a wadsctl
/// command (`builder edit`) is being answered, with the commands in use.
pub fn register(shell: &Rc<Shell>) {
    if REGISTERED.replace(true) {
        return;
    }
    let shell = shell.clone();
    glib::idle_add_local_once(move || {
        shell.on_ctl("stack", |args| match Board::current() {
            Some(b) => b.ctl(args),
            None => "error the Builder isn't open".into(),
        });
    });
}

const USAGE: &str = "error stack [show <ws>|parts <ws>|at <ws> [<part> [x]]|at slot|at board|at menu <id>|click <ws> <part> [x]|menu [<ws> <part>]|pick <id>|ask <text>|unmenu|pointer|origin [<x> <y>]]";

impl Board {
    fn ctl(self: &Rc<Self>, args: &[&str]) -> String {
        let Some(b) = self.builder() else { return "error gone".into() };
        match args {
            [] => {
                let mut out = vec![];
                let slot = match (self.slot_at.borrow().as_ref(), self.hot.borrow().as_ref()) {
                    (None, _) => "-".to_string(),
                    (Some(a), hot) => format!(
                        "{}:after={}:\"{}\"",
                        if hot == Some(&super::Zone::Slot) { "hot" } else { "shown" },
                        a.after.as_deref().unwrap_or("-"),
                        a.hint
                    ),
                };
                out.push(format!(
                    "ok stacks={} selected={} slot={slot} col={} viewport={}",
                    self.stacks.borrow().len(),
                    b.selected().unwrap_or_default(),
                    self.col.get(),
                    self.scroll.hadjustment().page_size()
                ));
                for s in self.stacks.borrow().iter() {
                    let Some(x) = b.wadspace(&s.id) else { continue };
                    let state = s.status_label();
                    let sel = if b.selected().as_deref() == Some(s.id.as_str()) { " selected" } else { "" };
                    let at = self.screen(&s.root).map(|r| fmt_rect(&r)).unwrap_or_else(|| "unshown".into());
                    out.push(format!("{} \"{}\" {state}{sel} {at}", s.id, x.name));
                }
                out.join(" | ")
            }
            ["show", ws] => self.describe(ws),
            ["parts", ws] => match self.stack(ws) {
                Some(s) => format!("ok {}", s.part_ids().join(" ")),
                None => format!("error no stack {ws}"),
            },
            ["at", "slot"] => match self.slot_at.borrow().is_some() {
                true => self
                    .screen(&self.slot)
                    .map(|r| format!("ok {}", fmt_rect(&r)))
                    .unwrap_or_else(|| "error it isn't laid out".into()),
                false => "error no slot: nothing it takes is held".into(),
            },
            ["at", "board"] => self
                .screen(&self.root)
                .map(|r| format!("ok {}", fmt_rect(&r)))
                .unwrap_or_else(|| "error it isn't shown".into()),
            ["at", "menu", id] => match self.open().and_then(|m| m.item_at(id)) {
                Some(((x, y, w, h), (px, py))) => {
                    // The popup's surface is placed on the window's, whose own origin is its widget's less its shadow.
                    let (ox, oy) = self.origin_of();
                    let (tx, ty) = self.root.native().map(|n| n.surface_transform()).unwrap_or((0.0, 0.0));
                    format!("ok {}", fmt_rect(&(ox - tx + px + x, oy - ty + py + y, w, h)))
                }
                None => format!("error no item {id} shown"),
            },
            ["at", ws] => self.at(ws, "stack", false),
            ["at", ws, part] => self.at(ws, part, false),
            ["at", ws, part, "x"] => self.at(ws, part, true),
            ["click", ws, part] => self.click(ws, part, false),
            ["click", ws, part, "x"] => self.click(ws, part, true),
            ["menu"] => match self.open() {
                Some(m) => format!("ok {}", m.describe()),
                None => "ok none".into(),
            },
            ["menu", ws, part] => {
                let Some(variant) = variant_of(part) else { return format!("error {part} has no menu") };
                match self.open_menu(ws, part, &variant, false) {
                    Some(m) => format!("ok {}", m.describe()),
                    None => format!("error no menu for {part} in {ws} (another machine's, or it isn't there)"),
                }
            }
            ["pick", id] => match self.open() {
                Some(m) => match m.pick(id) {
                    Ok(()) => match self.open() {
                        Some(m) => format!("ok {}", m.describe()),
                        None => "ok".into(),
                    },
                    Err(e) => format!("error {e}"),
                },
                None => "error no menu is open".into(),
            },
            ["ask", text @ ..] => match self.open() {
                Some(m) if m.answer(Some(&text.join(" "))) => "ok".into(),
                _ => "error nothing's asking".into(),
            },
            ["unmenu"] => {
                self.close_menu();
                "ok".into()
            }
            ["pointer"] => match self.pointer.get() {
                Some((x, y)) => {
                    let (ox, oy) = self.origin_of();
                    let (wx, wy) = self.in_window(x, y).unwrap_or((x, y));
                    format!("ok board {x:.0} {y:.0} window {wx:.0} {wy:.0} screen {:.0} {:.0}", ox + wx, oy + wy)
                }
                None => "error it hasn't seen the pointer".into(),
            },
            ["origin"] => {
                let (x, y) = self.origin_of();
                format!("ok {x:.0} {y:.0}{}", if self.origin.get().is_some() { " told" } else { "" })
            }
            ["origin", x, y] => match (x.parse::<f64>(), y.parse::<f64>()) {
                (Ok(x), Ok(y)) => {
                    self.origin.set(Some((x, y)));
                    "ok".into()
                }
                _ => "error origin <x> <y>".into(),
            },
            _ => USAGE.into(),
        }
    }

    fn at(&self, ws: &str, part: &str, x: bool) -> String {
        let Some(s) = self.stack(ws) else { return format!("error no stack {ws}") };
        let id = if x { format!("{part}#x") } else { part.to_string() };
        let Some(w) = s.part(&id) else { return format!("error no part {id} in {ws}") };
        match self.screen(&w) {
            Some(r) => format!("ok {}", fmt_rect(&r)),
            None => "error it isn't laid out".into(),
        }
    }

    fn click(&self, ws: &str, part: &str, x: bool) -> String {
        let Some(s) = self.stack(ws) else { return format!("error no stack {ws}") };
        let id = if x { format!("{part}#x") } else { part.to_string() };
        let Some(button) = s.button(&id) else { return format!("error {id} in {ws} isn't a button") };
        if !button.is_sensitive() {
            return format!("error {id} is off");
        }
        button.emit_clicked();
        "ok".into()
    }

    /// What `ws`'s stack shows.
    fn describe(&self, ws: &str) -> String {
        let Some(b) = self.builder() else { return "error gone".into() };
        let (Some(x), Some(_)) = (b.wadspace(ws), self.stack(ws)) else { return format!("error no stack {ws}") };
        let mut out = vec![format!(
            "ok {} \"{}\" {}{}{}",
            x.id,
            x.name,
            status_of(self, ws),
            if b.selected().as_deref() == Some(ws) { " selected" } else { "" },
            if b.editable(&x) { "" } else { " away" }
        )];
        let icons = pieces::desk_icons(&x);
        let (shown, more) = pieces::fit(icons.len(), self.col.get() - 34);
        let names: Vec<String> = icons
            .iter()
            .take(shown)
            .map(|i| match &i.badge {
                Some(b) => format!("{} ({b})", i.name),
                None => i.name.clone(),
            })
            .chain((more > 0).then(|| format!("{more} more")))
            .collect();
        let files: Vec<&str> = x.look.projects.iter().map(|p| p.as_name.as_str()).collect();
        out.push(format!(
            "customization: wallpaper \"{}\"; files [{}]; desktop [{}]",
            wallpaper_name(&x.look.wallpaper),
            files.join(", "),
            names.join(", ")
        ));
        let keys = key_boxes(&x);
        out.push(format!("secrets {}", keys.len()));
        for k in &keys {
            let Some(key) = &k.folder.key else { continue };
            let pills: Vec<String> = k.folder.items.iter().map(|v| pill(&x, "secrets", v)).collect();
            out.push(format!("  key:{} \"{}\" {}: {}", key.id, key.name, k.folder.items.len(), pills.join(", ")));
        }
        let boxes = addon_boxes(&x);
        out.push(format!("addons {}", boxes.iter().map(|b| b.items.len()).sum::<usize>() + x.commands.len()));
        for a in &boxes {
            let mut pills: Vec<String> = a.items.iter().map(|v| pill(&x, "addons", v)).collect();
            pills.extend(
                a.commands.iter().map(|c| format!("cmd/{} \"{}\" ({} {})", c.id, c.name, c.kind.word(), c.text)),
            );
            out.push(format!(
                "  box:{} \"{}\" {}: {}",
                a.app,
                app_name(a.app),
                a.items.len() + a.commands.len(),
                pills.join(", ")
            ));
        }
        let apps = stack_apps(&x);
        let tiles: Vec<String> = apps
            .iter()
            .filter_map(|a| find_variant(&x, &format!("app:{a}")))
            .map(|v| format!("apps/{} \"{}\"{}", v.app, v.name, if pieces::launches(&x, &v) { " ★" } else { "" }))
            .collect();
        out.push(format!("apps {}: {}", apps.len(), tiles.join(", ")));
        out.join(" | ")
    }

    /// `w`'s box on the screen: (x, y, w, h).
    fn screen(&self, w: &impl IsA<gtk::Widget>) -> Option<(f64, f64, f64, f64)> {
        let root = w.as_ref().root()?;
        let r = w.as_ref().compute_bounds(&root)?;
        let (ox, oy) = self.origin_of();
        Some((ox + r.x() as f64, oy + r.y() as f64, r.width() as f64, r.height() as f64))
    }

    /// (x, y) on the board, in its window's pixels.
    fn in_window(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let root = self.root.root()?;
        let p = self.root.compute_point(&root, &gtk::graphene::Point::new(x as f32, y as f32))?;
        Some((p.x() as f64, p.y() as f64))
    }

    /// The window's top left on the screen: told, or where labwc puts it
    /// (its `builder` region, of the usable area under the bar: 26% across,
    /// and its gap, half of it beside another region, all of it at an edge).
    fn origin_of(&self) -> (f64, f64) {
        if let Some(o) = self.origin.get() {
            return o;
        }
        let Some(b) = self.builder() else { return (0.0, 0.0) };
        let g = b.shell.primary.geometry();
        let gap = 10.0;
        let x = g.x() as f64 + (g.width() * 26 / 100) as f64 + gap / 2.0;
        let y = g.y() as f64 + crate::machine::BAR as f64 + gap;
        (x, y)
    }
}

/// A thing as `show` says it: its id (as `at` takes it), its name, ★ if it starts straight into it.
fn pill(x: &wad_model::Wadspace, section: &str, v: &wad_model::variants::AppVariant) -> String {
    format!("{section}/{} \"{}\"{}", v.id, v.name, if pieces::launches(x, v) { " ★" } else { "" })
}

/// A part's thing (its menu's): 'secrets/ext:…' and 'addons/ext:…' are that variant; 'apps/vscode' is 'app:vscode'.
fn variant_of(part: &str) -> Option<String> {
    let (section, rest) = part.split_once('/')?;
    match section {
        "secrets" | "addons" => Some(rest.to_string()),
        "apps" => Some(format!("app:{rest}")),
        _ => None,
    }
}

fn status_of(b: &Board, ws: &str) -> String {
    b.stack(ws).map(|s| s.status_label()).unwrap_or_default()
}

fn fmt_rect(r: &(f64, f64, f64, f64)) -> String {
    let (x, y, w, h) = *r;
    format!("{:.0} {:.0} {x:.0} {y:.0} {w:.0} {h:.0}", x + w / 2.0, y + h / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_s_thing() {
        assert_eq!(variant_of("secrets/ext:wadbrowser:gmail").as_deref(), Some("ext:wadbrowser:gmail"));
        assert_eq!(variant_of("apps/vscode").as_deref(), Some("app:vscode"));
        assert_eq!(variant_of("key:work-web"), None);
        assert_eq!(variant_of("cmd/c1"), None);
    }
}
