//! Dragging things between the shell's surfaces: the Builder's stacks and
//! sidebar, the desktop and the bar (the spike's `dnd/controller`). It's
//! one process, so what's held is known here ([`held`]) while it's held:
//! every target says what letting go over it would do (a hint under the
//! pointer, in the drag's icon) and does it when it's let go. A target that
//! says nothing refuses it, and letting go there does nothing.
//!
//! - [`source`] makes a widget a drag source of what's under the pointer.
//! - [`target`] makes a widget a drop target: `offer` (what a drop would do,
//!   as its hint; none: nothing), `land` (do it), `hover` (lit or not).
//! - [`on_held`] is told when something's picked up or let go (the Builder's
//!   slot shows while one's held).
//! - [`begin`]/[`end`] are for a drag source of its own (the desktop's
//!   icons), so its drags are held here too.
//!
//! The drag carries [`MIME`] (what's held, as JSON) so targets in this
//! process recognise it; another app sees nothing it knows.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::{gdk, glib, prelude::*};
use serde::{Deserialize, Serialize};
use wad_model::Section;
use wad_model::edit::Payload;

pub const MIME: &str = "application/x-wados-drag";

/// Where a wadspace was picked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpaceFrom {
    /// Its desktop icon (moved: dropped on open desktop, it moves there).
    Desktop,
    /// Out of a tag's open folder (dropped on open desktop: untagged).
    Folder,
    /// Its tile on the bar.
    Bar,
}

/// What's held.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "held", rename_all = "camelCase")]
pub enum Held {
    /// A whole wadspace: a Builder stack (`from` none), or its icon somewhere.
    Space { ws: String, from: Option<SpaceFrom> },
    /// A section of a stack, one thing, or a key's group (the model's payload).
    Part { payload: Payload },
    /// Several of one wadspace's things picked together (the sidebar's Secrets: web apps), as pieces.
    Pieces { section: Section, items: Vec<String>, from: String },
}

impl Held {
    /// The wadspace it came out of, if it came out of one.
    pub fn source_ws(&self) -> Option<&str> {
        match self {
            Held::Space { ws, .. } => Some(ws),
            Held::Part { payload } => match payload {
                Payload::Section { ws, .. } | Payload::Folder { ws, .. } => Some(ws),
                Payload::Piece { from, .. } => from.as_deref(),
            },
            Held::Pieces { from, .. } => Some(from),
        }
    }
}

/// What letting go over a target would do, said under the pointer; or
/// why it won't take it (`ok` false: refused, with its reason shown).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub ok: bool,
    pub hint: String,
}

impl Verdict {
    /// It takes it: `hint` says what happens.
    pub fn yes(hint: impl Into<String>) -> Option<Self> {
        Some(Self { ok: true, hint: hint.into() })
    }

    /// It doesn't, and says why.
    pub fn no(hint: impl Into<String>) -> Option<Self> {
        Some(Self { ok: false, hint: hint.into() })
    }
}

type Listener = Rc<dyn Fn(Option<&Held>)>;

#[derive(Default)]
struct State {
    held: Option<Held>,
    hint: Option<gtk::Label>,
    listeners: Vec<Listener>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

/// What's held now (none: nothing).
pub fn held() -> Option<Held> {
    STATE.with(|s| s.borrow().held.clone())
}

/// `f` each time something's picked up (Some) or let go (None).
pub fn on_held(f: impl Fn(Option<&Held>) + 'static) {
    STATE.with(|s| s.borrow_mut().listeners.push(Rc::new(f)));
}

/// A drag of a source of its own has begun: `what` is held.
pub fn begin(what: Held) {
    let listeners = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.held = Some(what.clone());
        s.listeners.clone()
    });
    for f in listeners {
        f(Some(&what));
    }
}

/// It's been let go (dropped or not).
pub fn end() {
    let listeners = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.held = None;
        s.hint = None;
        s.listeners.clone()
    });
    for f in listeners {
        f(None);
    }
}

fn set_hint(v: Option<&Verdict>) {
    STATE.with(|s| {
        if let Some(l) = &s.borrow().hint {
            match v {
                Some(v) => {
                    l.set_label(&v.hint);
                    l.set_visible(true);
                    if v.ok {
                        l.remove_css_class("no");
                    } else {
                        l.add_css_class("no");
                    }
                }
                None => l.set_visible(false),
            }
        }
    });
}

/// The bytes a drag of `what` carries.
pub fn content(what: &Held) -> gdk::ContentProvider {
    let json = serde_json::to_vec(what).unwrap_or_default();
    gdk::ContentProvider::for_bytes(MIME, &glib::Bytes::from_owned(json))
}

/// The formats a target takes: ours.
pub fn formats() -> gdk::ContentFormats {
    gdk::ContentFormats::new(&[MIME])
}

/// What's been picked up, before its drag begins: it, its look, and where on that the pointer is.
type Picked = (Held, gtk::Widget, f64, f64);

/// Makes `widget` a drag source: `what(x, y)` is what's under (x, y) (in
/// `widget`'s coordinates) and the widget it looks like under the pointer
/// (none: no drag from there).
pub fn source(
    widget: &impl IsA<gtk::Widget>,
    what: impl Fn(f64, f64) -> Option<(Held, gtk::Widget)> + 'static,
) -> gtk::DragSource {
    let src = gtk::DragSource::builder().actions(gdk::DragAction::COPY).build();
    let picked: Rc<RefCell<Option<Picked>>> = Rc::default();
    let p = picked.clone();
    let w = widget.as_ref().downgrade();
    src.connect_prepare(move |_, x, y| {
        let (held, look) = what(x, y)?;
        // Where on the look the pointer is, so it stays under it.
        let (hx, hy) = w
            .upgrade()
            .and_then(|w| w.compute_point(&look, &gtk::graphene::Point::new(x as f32, y as f32)))
            .map(|pt| (pt.x() as f64, pt.y() as f64))
            .unwrap_or((12.0, 12.0));
        let c = content(&held);
        *p.borrow_mut() = Some((held, look, hx, hy));
        Some(c)
    });
    let p = picked.clone();
    src.connect_drag_begin(move |_, drag| {
        let Some((held, look, hx, hy)) = p.borrow_mut().take() else { return };
        let icon = gtk::DragIcon::for_drag(drag);
        icon.set_child(Some(&ghost(&look)));
        drag.set_hotspot(hx.round() as i32, hy.round() as i32);
        begin(held);
    });
    src.connect_drag_end(|_, _, _| end());
    src.connect_drag_cancel(|_, _, _| {
        end();
        false
    });
    widget.as_ref().add_controller(src.clone());
    src
}

/// The drag's icon: `look` as it is (live), and the hint under it.
fn ghost(look: &gtk::Widget) -> gtk::Widget {
    let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).css_classes(["dnd-ghost"]).build();
    let pic = gtk::Picture::for_paintable(&gtk::WidgetPaintable::new(Some(look)));
    pic.set_can_shrink(false);
    let (w, h) = (look.width().max(1), look.height().max(1));
    pic.set_size_request(w, h);
    pic.set_opacity(0.92);
    col.append(&pic);
    let hint = gtk::Label::builder().css_classes(["dnd-hint"]).halign(gtk::Align::Start).visible(false).build();
    col.append(&hint);
    STATE.with(|s| s.borrow_mut().hint = Some(hint));
    col.upcast()
}

/// Makes `widget` a drop target for what's held: `offer(held, x, y)` says
/// what letting go at (x, y) would do (none: nothing, and it's refused; a
/// [`Verdict::no`]: refused, saying why); `land(held, x, y)` does it;
/// `hover(on)` lights it while it would take it.
pub fn target(
    widget: &impl IsA<gtk::Widget>,
    offer: impl Fn(&Held, f64, f64) -> Option<Verdict> + 'static,
    land: impl Fn(&Held, f64, f64) + 'static,
    hover: impl Fn(bool) + 'static,
) -> gtk::DropTargetAsync {
    let t = gtk::DropTargetAsync::new(Some(formats()), gdk::DragAction::COPY);
    let offer = Rc::new(offer);
    let land = Rc::new(land);
    let hover = Rc::new(hover);
    let over = {
        let (offer, hover) = (offer.clone(), hover.clone());
        move |x: f64, y: f64| -> gdk::DragAction {
            let v = held().and_then(|h| offer(&h, x, y));
            set_hint(v.as_ref());
            let ok = v.is_some_and(|v| v.ok);
            hover(ok);
            if ok { gdk::DragAction::COPY } else { gdk::DragAction::empty() }
        }
    };
    let o = Rc::new(over);
    let o2 = o.clone();
    t.connect_drag_enter(move |_, _, x, y| o2(x, y));
    let o2 = o.clone();
    t.connect_drag_motion(move |_, _, x, y| o2(x, y));
    let h = hover.clone();
    t.connect_drag_leave(move |_, _| {
        set_hint(None);
        h(false);
    });
    t.connect_drop(move |_, drop, x, y| {
        hover(false);
        let Some(h) = held() else {
            drop.finish(gdk::DragAction::empty());
            return false;
        };
        if !offer(&h, x, y).is_some_and(|v| v.ok) {
            drop.finish(gdk::DragAction::empty());
            return false;
        }
        drop.finish(gdk::DragAction::COPY);
        // After the drag's done with (a land may open windows or menus).
        let land = land.clone();
        glib::idle_add_local_once(move || land(&h, x, y));
        true
    });
    widget.as_ref().add_controller(t.clone());
    t
}

/// The ghost's and the hint's look.
pub const CSS: &str = "
.dnd-ghost { padding: 2px; }
.dnd-hint {
  padding: 4px 10px; border-radius: 999px;
  background: var(--accent); color: var(--bg);
  font-weight: 600; font-size: 12px;
  box-shadow: 0 6px 18px -6px rgb(0 0 0 / 0.6);
}
.dnd-hint.no { background: var(--surface-3); color: var(--muted); }
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_s_held_goes_as_json_and_comes_back() {
        let all = [
            Held::Space { ws: "studio".into(), from: Some(SpaceFrom::Desktop) },
            Held::Part {
                payload: Payload::Piece {
                    section: Section::Addons,
                    item: "wadbrowser:gmail".into(),
                    from: Some("v".into()),
                },
            },
            Held::Pieces { section: Section::Addons, items: vec!["gmail".into(), "drive".into()], from: "v".into() },
        ];
        for h in all {
            let back: Held = serde_json::from_slice(&serde_json::to_vec(&h).unwrap()).unwrap();
            assert_eq!(back, h);
        }
    }

    #[test]
    fn it_knows_the_wadspace_it_came_out_of() {
        let p =
            Held::Part { payload: Payload::Folder { ws: "v".into(), app: wad_model::AppId::Wadbrowser, key: None } };
        assert_eq!(p.source_ws(), Some("v"));
        let p = Held::Part { payload: Payload::Piece { section: Section::Apps, item: "krita".into(), from: None } };
        assert_eq!(p.source_ws(), None);
    }
}
