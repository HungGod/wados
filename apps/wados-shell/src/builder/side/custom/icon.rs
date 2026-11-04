//! The Customize tab's Icon (the spike's `IconEditor.svelte`): the picture
//! it's cut out of (its background, or a picture of its own: yours,
//! WadSpaces', or one you upload), and the square of it it shows, on the
//! picture as it fills a screen. Drag the square (or click where it should
//! go; the arrow keys nudge it), zoom in with the slider; Reset for the
//! middle of it.
//!
//! What it is while it's being changed is a draft: drawn at once, sent as
//! one `space.icon` patch when the drag ends (or the slider's let go), and
//! kept till wadd's change comes back, so the square never jumps back.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::{gdk, glib, graphene, gsk, prelude::*, subclass::prelude::*};
use wad_model::edit::Op;
use wad_model::look::{ICON_ASPECT, ICON_ZOOM_MAX, ICON_ZOOM_MIN, IconPatch, clamp_icon, icon_of, icon_picture_of};
use wad_model::wallpapers::has_picture;
use wad_model::{IconLook, Wadspace, Wallpaper};
use wad_proto::Level;

use super::photos::{Photos, Picker};
use crate::builder::Builder;

/// The slider's steps (the spike's).
const ZOOM_STEP: f64 = 0.05;
/// How long a slider moved by the keys (or the wheel) waits before it's sent.
const SETTLE: Duration = Duration::from_millis(300);
/// How long a draft waits for wadd's change before it gives up.
const STALE: Duration = Duration::from_millis(2500);

fn near(a: &IconLook, b: &IconLook) -> bool {
    (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6 && (a.zoom - b.zoom).abs() < 1e-6
}

/// The square in the middle, as big as it goes (what Reset puts back).
pub fn at_middle(i: &IconLook) -> bool {
    near(i, &clamp_icon(IconLook::default()))
}

// ------------------------------------------------------------ the crop view

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Crop {
        pub picture: RefCell<Option<wad_gtk::Picture>>,
        pub look: RefCell<IconLook>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Crop {
        const NAME: &'static str = "WadIconCrop";
        type Type = super::Crop;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("cz-crop");
        }
    }

    impl ObjectImpl for Crop {
        fn constructed(&self) {
            self.parent_constructed();
            let p = wad_gtk::Picture::new(&Wallpaper::Empty, 10.0);
            p.set_parent(&*self.obj());
            *self.picture.borrow_mut() = Some(p);
        }

        fn dispose(&self) {
            if let Some(p) = self.picture.borrow_mut().take() {
                p.unparent();
            }
        }
    }

    impl WidgetImpl for Crop {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        /// As wide as it's given, 16:10.
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Horizontal => (120, 320, -1, -1),
                _ => {
                    let w = if for_size < 0 { 320 } else { for_size };
                    let h = (f64::from(w) / ICON_ASPECT).round() as i32;
                    (h, h, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            if let Some(p) = self.picture.borrow().as_ref() {
                p.size_allocate(&gtk::Allocation::new(0, 0, width, height), -1);
            }
        }

        fn snapshot(&self, s: &gtk::Snapshot) {
            let w = self.obj();
            let (width, height) = (w.width() as f32, w.height() as f32);
            if width <= 0.0 || height <= 0.0 {
                return;
            }
            if let Some(p) = self.picture.borrow().as_ref() {
                w.snapshot_child(p, s);
            }
            let bounds = graphene::Rect::new(0.0, 0.0, width, height);
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(bounds, 10.0));
            let f = frame(&self.look.borrow(), width, height);
            let r = f.width() * 0.22;
            let square = gsk::RoundedRect::from_rect(f, r);
            // The rest dimmed, then the square's edge.
            s.append_outset_shadow(
                &square,
                &gdk::RGBA::new(10.0 / 255.0, 6.0 / 255.0, 20.0 / 255.0, 0.55),
                0.0,
                0.0,
                4000.0,
                0.0,
            );
            s.append_outset_shadow(&square, &gdk::RGBA::new(0.0, 0.0, 0.0, 0.4), 0.0, 0.0, 1.0, 0.0);
            let edge =
                if w.has_focus() { gdk::RGBA::new(198.0 / 255.0, 1.0, 31.0 / 255.0, 1.0) } else { gdk::RGBA::WHITE };
            s.append_border(&square, &[2.0; 4], &[edge; 4]);
            s.pop();
        }
    }
}

glib::wrapper! {
    /// A picture as it fills a screen (16:10), the icon's square on it.
    pub struct Crop(ObjectSubclass<imp::Crop>) @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// The square on a `width` × `height` picture: its middle at (x, y), as
/// tall as the picture over the zoom.
fn frame(i: &IconLook, width: f32, height: f32) -> graphene::Rect {
    let (sw, sh) = (width / (ICON_ASPECT as f32 * i.zoom as f32), height / i.zoom as f32);
    graphene::Rect::new(i.x as f32 * width - sw / 2.0, i.y as f32 * height - sh / 2.0, sw, sh)
}

impl Crop {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_picture(&self, w: &Wallpaper) {
        if let Some(p) = self.imp().picture.borrow().as_ref() {
            p.set_wallpaper(w);
        }
    }

    pub fn set_look(&self, i: &IconLook) {
        if !near(&self.imp().look.borrow(), i) {
            *self.imp().look.borrow_mut() = i.clone();
            self.queue_draw();
        }
    }

    /// The square, in its pixels.
    pub fn square(&self) -> graphene::Rect {
        frame(&self.imp().look.borrow(), self.width() as f32, self.height() as f32)
    }
}

// ------------------------------------------------------------ the editor

/// Where a drag of the square began: the square's middle then, and the pointer's (in the picture's widths and heights).
#[derive(Clone, Copy)]
struct Drag {
    base: (f64, f64),
}

pub struct Editor {
    builder: Weak<Builder>,
    me: Weak<Editor>,
    root: gtk::Box,
    src_bg: gtk::Button,
    src_own: gtk::Button,
    picker: Rc<Picker>,
    sub: gtk::Label,
    crop: Crop,
    zoom_row: gtk::Box,
    zoom: gtk::Scale,
    reset: gtk::Button,
    ws: RefCell<Option<Wadspace>>,
    editable: Cell<bool>,
    /// "Its own picture" picked, but no picture yet: its picker shows.
    choosing: Cell<bool>,
    /// What it is while it's changed (none: as wadd has it).
    draft: RefCell<Option<IconLook>>,
    drag: Cell<Option<Drag>>,
    /// The slider's held (sent when it's let go).
    held: Cell<bool>,
    settle: RefCell<Option<glib::SourceId>>,
    /// Bumped each time a draft is sent (a stale one's timeout knows it's old).
    sent: Cell<u32>,
    /// The slider's being set from here (not moved by someone).
    updating: Cell<bool>,
}

impl Editor {
    pub fn new(b: &Rc<Builder>, photos: &Rc<Photos>) -> Rc<Self> {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["cz-icon"]).build();
        let src = gtk::Box::builder().homogeneous(true).spacing(4).css_classes(["cz-src"]).build();
        let src_bg = gtk::Button::builder().label("Its background").css_classes(["cz-src-b"]).build();
        let src_own = gtk::Button::builder().label("Its own picture").css_classes(["cz-src-b"]).build();
        src.append(&src_bg);
        src.append(&src_own);
        root.append(&src);
        let sub = gtk::Label::builder()
            .label("Drag the square to the part it shows, and zoom in.")
            .xalign(0.0)
            .wrap(true)
            .css_classes(["cz-sub"])
            .build();
        let crop = Crop::new();
        crop.set_focusable(true);
        crop.set_cursor_from_name(Some("crosshair"));
        let zoom_row = gtk::Box::builder().spacing(8).css_classes(["cz-zoom-row"]).build();
        zoom_row.append(&wad_gtk::Icon::new("image", 14));
        let zoom = gtk::Scale::with_range(gtk::Orientation::Horizontal, ICON_ZOOM_MIN, ICON_ZOOM_MAX, 0.05);
        zoom.set_draw_value(false);
        zoom.set_hexpand(true);
        zoom.add_css_class("cz-zoom");
        zoom.set_tooltip_text(Some("Zoom"));
        zoom_row.append(&zoom);
        zoom_row.append(&wad_gtk::Icon::new("image", 19));
        let reset = gtk::Button::builder().label("Reset").css_classes(["cz-reset"]).build();
        zoom_row.append(&reset);

        let me = Rc::new_cyclic(|me: &Weak<Editor>| {
            let w = me.clone();
            let fail = Rc::downgrade(b);
            let picker = Picker::new(
                photos,
                false,
                move |wp| {
                    if let Some(e) = w.upgrade() {
                        e.choosing.set(false);
                        e.own(Some(wp));
                    }
                },
                move |why| {
                    if let Some(b) = fail.upgrade() {
                        b.say(Level::Warn, why);
                    }
                },
            );
            Self {
                builder: Rc::downgrade(b),
                me: me.clone(),
                root,
                src_bg,
                src_own,
                picker,
                sub,
                crop,
                zoom_row,
                zoom,
                reset,
                ws: RefCell::default(),
                editable: Cell::new(false),
                choosing: Cell::new(false),
                draft: RefCell::default(),
                drag: Cell::new(None),
                held: Cell::new(false),
                settle: RefCell::default(),
                sent: Cell::new(0),
                updating: Cell::new(false),
            }
        });
        me.root.append(&me.picker.widget());
        me.root.append(&me.sub);
        me.root.append(&me.crop);
        me.root.append(&me.zoom_row);
        me.wire();
        me
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    pub fn picker(&self) -> &Rc<Picker> {
        &self.picker
    }

    fn wire(&self) {
        let w = self.me.clone();
        self.src_bg.connect_clicked(move |_| {
            if let Some(e) = w.upgrade() {
                e.choosing.set(false);
                let has_own = e.ws.borrow().as_ref().is_some_and(|x| own_of(x).is_some());
                if has_own {
                    e.own(None);
                } else {
                    e.redraw();
                }
            }
        });
        let w = self.me.clone();
        self.src_own.connect_clicked(move |_| {
            if let Some(e) = w.upgrade() {
                e.choosing.set(true);
                e.redraw();
            }
        });
        let w = self.me.clone();
        self.reset.connect_clicked(move |_| {
            if let Some(e) = w.upgrade() {
                e.do_reset();
            }
        });

        // The square: dragged, or (pressed off it) put there first.
        let g = gtk::GestureDrag::builder().button(gdk::BUTTON_PRIMARY).build();
        let w = self.me.clone();
        g.connect_drag_begin(move |_, x, y| {
            if let Some(e) = w.upgrade() {
                e.press(x, y);
            }
        });
        let w = self.me.clone();
        g.connect_drag_update(move |_, dx, dy| {
            if let Some(e) = w.upgrade() {
                e.motion(dx, dy);
            }
        });
        let w = self.me.clone();
        g.connect_drag_end(move |_, _, _| {
            if let Some(e) = w.upgrade() {
                e.release();
            }
        });
        self.crop.add_controller(g);

        // The arrow keys nudge it.
        let k = gtk::EventControllerKey::new();
        let w = self.me.clone();
        k.connect_key_pressed(move |_, key, _, _| {
            let Some(e) = w.upgrade() else { return glib::Propagation::Proceed };
            let d = match key {
                gdk::Key::Left => (-1.0, 0.0),
                gdk::Key::Right => (1.0, 0.0),
                gdk::Key::Up => (0.0, -1.0),
                gdk::Key::Down => (0.0, 1.0),
                _ => return glib::Propagation::Proceed,
            };
            e.nudge(d.0 * 0.02, d.1 * 0.02);
            glib::Propagation::Stop
        });
        self.crop.add_controller(k);
        let f = gtk::EventControllerFocus::new();
        let c = self.crop.downgrade();
        f.connect_enter(move |_| c.upgrade().iter().for_each(|c| c.queue_draw()));
        let c = self.crop.downgrade();
        f.connect_leave(move |_| c.upgrade().iter().for_each(|c| c.queue_draw()));
        self.crop.add_controller(f);

        // The slider: drawn at once; sent when it's let go (or, by the keys, when it settles).
        let w = self.me.clone();
        self.zoom.connect_value_changed(move |s| {
            if let Some(e) = w.upgrade()
                && !e.updating.get()
            {
                e.zoomed(s.value());
            }
        });
        let held = gtk::EventControllerLegacy::new();
        held.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = self.me.clone();
        held.connect_event(move |_, ev| {
            if let Some(e) = w.upgrade() {
                match ev.event_type() {
                    gdk::EventType::ButtonPress | gdk::EventType::TouchBegin => e.held.set(true),
                    gdk::EventType::ButtonRelease | gdk::EventType::TouchEnd | gdk::EventType::TouchCancel => {
                        e.held.set(false);
                        // After the slider's own handling of the release.
                        let w = e.me.clone();
                        glib::idle_add_local_once(move || {
                            if let Some(e) = w.upgrade() {
                                e.commit_zoom();
                            }
                        });
                    }
                    _ => {}
                }
            }
            glib::Propagation::Proceed
        });
        self.zoom.add_controller(held);
    }

    // ------------------------------------------------------------ shown

    /// The selected wadspace, as it is now.
    pub fn show(&self, x: &Wadspace, editable: bool) {
        let other = self.ws.borrow().as_ref().is_none_or(|y| y.id != x.id);
        if other {
            self.choosing.set(false);
            self.draft.replace(None);
            self.drag.set(None);
        }
        // wadd's change has come: the draft is what it is now.
        let model = icon_of(x);
        let caught_up = self.draft.borrow().as_ref().is_some_and(|d| near(d, &model));
        if caught_up && self.drag.get().is_none() && !self.held.get() {
            self.draft.replace(None);
        }
        *self.ws.borrow_mut() = Some(x.clone());
        self.editable.set(editable);
        self.redraw();
    }

    fn redraw(&self) {
        let Some(x) = self.ws.borrow().clone() else { return };
        let editable = self.editable.get();
        let own = own_of(&x);
        let choosing = self.choosing.get() && own.is_none();
        let on_own = own.is_some() || choosing;
        set_on(&self.src_bg, !on_own);
        set_on(&self.src_own, on_own);
        self.src_bg.set_sensitive(editable);
        self.src_own.set_sensitive(editable);
        self.picker.widget().set_visible(on_own);
        self.picker.set(own.as_ref(), editable);

        let picture = icon_picture_of(&x);
        let has = has_picture(picture);
        for w in [self.sub.upcast_ref::<gtk::Widget>(), self.crop.upcast_ref(), self.zoom_row.upcast_ref()] {
            w.set_visible(has);
        }
        self.crop.set_picture(picture);
        self.crop.set_sensitive(editable);
        self.crop.set_cursor_from_name(Some(if editable { "crosshair" } else { "default" }));
        let now = self.now();
        self.crop.set_look(&now);
        if !self.held.get() && (self.zoom.value() - now.zoom).abs() > 1e-9 {
            self.updating.set(true);
            self.zoom.set_value(now.zoom);
            self.updating.set(false);
        }
        self.zoom.set_sensitive(editable);
        self.reset.set_sensitive(editable && !at_middle(&icon_of(&x)));
    }

    /// What it is now: the draft, or as wadd has it.
    fn now(&self) -> IconLook {
        if let Some(d) = self.draft.borrow().clone() {
            return d;
        }
        self.ws.borrow().as_ref().map(icon_of).unwrap_or_default()
    }

    fn draw_draft(&self, d: IconLook) {
        self.crop.set_look(&d);
        self.draft.replace(Some(d));
    }

    // ------------------------------------------------------------ the square

    fn at(&self, x: f64, y: f64) -> (f64, f64) {
        let (w, h) = (f64::from(self.crop.width().max(1)), f64::from(self.crop.height().max(1)));
        (x / w, y / h)
    }

    fn press(&self, x: f64, y: f64) {
        if !self.editable.get() {
            return;
        }
        self.crop.grab_focus();
        let now = self.now();
        let on_square = self.crop.square().contains_point(&graphene::Point::new(x as f32, y as f32));
        let d = if on_square {
            now
        } else {
            let (fx, fy) = self.at(x, y);
            clamp_icon(IconLook { x: fx, y: fy, ..now })
        };
        self.drag.set(Some(Drag { base: (d.x, d.y) }));
        self.draw_draft(d);
    }

    fn motion(&self, dx: f64, dy: f64) {
        let Some(drag) = self.drag.get() else { return };
        let (fx, fy) = self.at(dx, dy);
        let now = self.now();
        self.draw_draft(clamp_icon(IconLook { x: drag.base.0 + fx, y: drag.base.1 + fy, ..now }));
    }

    fn release(&self) {
        if self.drag.take().is_none() {
            return;
        }
        let d = self.now();
        let model = self.ws.borrow().as_ref().map(icon_of).unwrap_or_default();
        if near(&d, &model) {
            // A click on the square: nothing moved.
            self.draft.replace(None);
            return;
        }
        self.send(IconPatch { x: Some(Some(d.x)), y: Some(Some(d.y)), ..Default::default() });
    }

    fn nudge(&self, dx: f64, dy: f64) {
        if !self.editable.get() {
            return;
        }
        let now = self.now();
        let d = clamp_icon(IconLook { x: now.x + dx, y: now.y + dy, ..now });
        self.draw_draft(d.clone());
        self.send(IconPatch { x: Some(Some(d.x)), y: Some(Some(d.y)), ..Default::default() });
    }

    // ------------------------------------------------------------ the zoom

    fn zoomed(&self, z: f64) {
        if !self.editable.get() {
            return;
        }
        let z = ((z - ICON_ZOOM_MIN) / ZOOM_STEP).round() * ZOOM_STEP + ICON_ZOOM_MIN;
        let now = self.now();
        self.draw_draft(clamp_icon(IconLook { zoom: z, ..now }));
        if let Some(id) = self.settle.take() {
            id.remove();
        }
        if self.held.get() {
            return;
        }
        let w = self.me.clone();
        let id = glib::timeout_add_local_once(SETTLE, move || {
            if let Some(e) = w.upgrade() {
                e.settle.replace(None);
                e.commit_zoom();
            }
        });
        self.settle.replace(Some(id));
    }

    /// The zoom (and where the square went for it) sent, if it changed.
    fn commit_zoom(&self) {
        if self.held.get() {
            return;
        }
        if let Some(id) = self.settle.take() {
            id.remove();
        }
        let Some(d) = self.draft.borrow().clone() else { return };
        let model = self.ws.borrow().as_ref().map(icon_of).unwrap_or_default();
        if near(&d, &model) {
            return;
        }
        self.send(IconPatch { zoom: Some(Some(d.zoom)), x: Some(Some(d.x)), y: Some(Some(d.y)), ..Default::default() });
    }

    /// The zoom set (`wadsctl custom zoom`): as the slider let go there.
    pub fn set_zoom(&self, z: f64) {
        self.zoomed(z.clamp(ICON_ZOOM_MIN, ICON_ZOOM_MAX));
        self.commit_zoom();
        self.updating.set(true);
        self.zoom.set_value(self.now().zoom);
        self.updating.set(false);
    }

    // ------------------------------------------------------------ edits

    fn send(&self, patch: IconPatch) {
        let Some(id) = self.ws.borrow().as_ref().map(|x| x.id.clone()) else { return };
        let Some(b) = self.builder.upgrade() else { return };
        tracing::debug!("{id}'s icon: {patch:?}");
        b.shell.op(Op::SetIcon { ws: id, patch: Some(patch) }, |_| {});
        // If wadd's change never comes (it said no), the draft goes.
        let n = self.sent.get().wrapping_add(1);
        self.sent.set(n);
        let w = self.me.clone();
        glib::timeout_add_local_once(STALE, move || {
            if let Some(e) = w.upgrade()
                && e.sent.get() == n
                && e.drag.get().is_none()
                && !e.held.get()
                && e.draft.replace(None).is_some()
            {
                e.redraw();
            }
        });
    }

    /// A picture of its own (none: its background's): the square starts in the middle of it.
    fn own(&self, wp: Option<Wallpaper>) {
        self.draft.replace(None);
        self.send(IconPatch { picture: Some(wp), x: Some(None), y: Some(None), zoom: Some(None) });
        self.redraw();
    }

    /// The middle of it, as big as it goes. With no picture of its own that's
    /// no icon look at all; one of its own is kept.
    pub fn do_reset(&self) {
        let Some(x) = self.ws.borrow().clone() else { return };
        if !self.editable.get() {
            return;
        }
        self.draft.replace(None);
        let Some(b) = self.builder.upgrade() else { return };
        let patch =
            own_of(&x).map(|_| IconPatch { x: Some(None), y: Some(None), zoom: Some(None), ..Default::default() });
        b.shell.op(Op::SetIcon { ws: x.id.clone(), patch }, |_| {});
        self.redraw();
    }

    pub fn source(&self, own: bool) -> Result<(), String> {
        let b = if own { &self.src_own } else { &self.src_bg };
        if !b.is_sensitive() {
            return Err("it can't change here".into());
        }
        b.emit_clicked();
        Ok(())
    }

    // ------------------------------------------------------------ wadsctl

    /// What `wadsctl custom` says of it.
    pub fn describe(&self) -> String {
        let now = self.now();
        let own = self.ws.borrow().as_ref().and_then(own_of);
        format!(
            "icon-source={} icon-picture={:?} icon-x={:.4} icon-y={:.4} icon-zoom={:.3} icon-draft={} reset={}",
            if own.is_some() || self.choosing.get() { "own" } else { "background" },
            own.as_ref().map(super::photos::key_of).unwrap_or_else(|| "-".into()),
            now.x,
            now.y,
            now.zoom,
            self.draft.borrow().is_some(),
            if self.reset.is_sensitive() { "on" } else { "off" },
        )
    }

    /// A widget of it by id, and the point on it (its middle, but for the square's and the slider's knob).
    pub fn spot(&self, id: &str) -> Option<(gtk::Widget, f64, f64)> {
        let mid = |w: &gtk::Widget| Some((w.clone(), f64::from(w.width()) / 2.0, f64::from(w.height()) / 2.0));
        match id {
            "icon-source:background" => mid(self.src_bg.upcast_ref()),
            "icon-source:own" => mid(self.src_own.upcast_ref()),
            "icon-upload" => mid(self.picker.upload_button().upcast_ref()),
            "crop" => mid(self.crop.upcast_ref()),
            "frame" => {
                let r = self.crop.square();
                Some((
                    self.crop.clone().upcast(),
                    f64::from(r.x() + r.width() / 2.0),
                    f64::from(r.y() + r.height() / 2.0),
                ))
            }
            "zoom" => {
                // The knob: along the trough as far as the zoom is (the slider's own ends inset by its knob).
                let w = f64::from(self.zoom.width());
                let k = (self.now().zoom - ICON_ZOOM_MIN) / (ICON_ZOOM_MAX - ICON_ZOOM_MIN);
                Some((self.zoom.clone().upcast(), 8.0 + k * (w - 16.0), f64::from(self.zoom.height()) / 2.0))
            }
            "reset" => mid(self.reset.upcast_ref()),
            _ => {
                let key = id.strip_prefix("icon-bg:")?;
                mid(self.picker.find(key)?.upcast_ref())
            }
        }
    }
}

/// Its own picture, if it has one.
fn own_of(x: &Wadspace) -> Option<Wallpaper> {
    x.look.icon.as_ref().and_then(|i| i.picture.clone())
}

fn set_on(b: &gtk::Button, on: bool) {
    if on {
        b.add_css_class("on");
    } else {
        b.remove_css_class("on");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_square_is_square_and_in_the_middle_to_begin_with() {
        let i = clamp_icon(IconLook::default());
        assert!(at_middle(&i));
        // On a 16:10 picture 320 x 200, at 1x it's as tall as the picture.
        let f = frame(&i, 320.0, 200.0);
        assert_eq!((f.x(), f.y(), f.width(), f.height()), (60.0, 0.0, 200.0, 200.0));
        let z = clamp_icon(IconLook { zoom: 2.0, x: 0.9, y: 0.1, picture: None });
        assert!(!at_middle(&z));
        let f = frame(&z, 320.0, 200.0);
        assert_eq!((f.width(), f.height()), (100.0, 100.0));
        // Kept inside the picture.
        assert!(f.x() >= 0.0 && f.y() >= 0.0 && f.x() + f.width() <= 320.0 + 1e-3);
    }
}
