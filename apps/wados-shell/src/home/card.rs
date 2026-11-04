//! One of the carousel's square cards (Home.svelte's `.card`): the
//! wadspace's picture, cut out as its icon says (glass when it has none),
//! shaded toward the bottom, its badges at the top left (Running, Not built
//! yet, Rebuilds, Didn't open; another machine's, its machine), a ring
//! while it builds, starts or copies here, and at the bottom its apps and
//! its name. The launcher scales it (62% beside the one
//! in focus) and rings the one in focus.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use wad_model::{AppId, Built, IconLook, Wadspace, Wallpaper};

/// Its corners.
const RADIUS: f32 = 26.0;
/// How many apps show before "+N".
const APPS: usize = 4;
const APP_SIZE: i32 = 28;

/// One app on a card: a catalog app's icon, or a web app's site.
#[derive(Debug, Clone, PartialEq)]
pub enum AppTile {
    App(AppId),
    Web { id: String, name: Option<String>, url: Option<String> },
}

/// Its apps, as the spike's `appsOf`: the apps (not the terminal), the web apps, then the terminal.
pub fn apps_of(x: &Wadspace) -> Vec<AppTile> {
    let mut out: Vec<AppTile> =
        x.apps.iter().filter(|a| a.app != AppId::Terminal).map(|a| AppTile::App(a.app)).collect();
    out.extend(x.exts.iter().filter(|e| e.app == AppId::Wadbrowser && e.url.is_some()).map(|e| AppTile::Web {
        id: e.id.clone(),
        name: e.name.clone(),
        url: e.url.clone(),
    }));
    if x.has_app(AppId::Terminal) {
        out.push(AppTile::App(AppId::Terminal));
    }
    out
}

/// What a card shows.
#[derive(Debug, Clone, PartialEq)]
pub struct CardData {
    pub name: String,
    pub picture: Wallpaper,
    pub icon: IconLook,
    pub apps: Vec<AppTile>,
    pub running: bool,
    /// Its image here (none: not said yet).
    pub built: Option<Built>,
    pub failed: bool,
    /// Building or starting: how far (0–1).
    pub busy: Option<f64>,
    /// Another machine's: its glyph and name.
    pub machine: Option<(&'static str, String)>,
}

pub struct Card {
    pub root: gtk::Overlay,
    picture: wad_gtk::Picture,
    running: gtk::Label,
    unbuilt: gtk::Box,
    unbuilt_label: gtk::Label,
    failed: gtk::Label,
    machine: gtk::Box,
    machine_icon: wad_gtk::Icon,
    machine_name: gtk::Label,
    busy: gtk::Box,
    ring: gtk::DrawingArea,
    ring_value: Rc<Cell<f64>>,
    apps: gtk::Box,
    name: gtk::Label,
    data: RefCell<Option<CardData>>,
}

impl Card {
    pub fn new(side: i32) -> Rc<Self> {
        let root = gtk::Overlay::builder().css_classes(["home-card"]).overflow(gtk::Overflow::Hidden).build();
        root.set_size_request(side, side);
        let face = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_child(Some(&face));

        let picture = wad_gtk::Picture::new(&Wallpaper::Empty, RADIUS);
        picture.set_can_target(false);
        root.add_overlay(&picture);
        let shade = gtk::Box::builder().css_classes(["home-card-shade"]).can_target(false).build();
        root.add_overlay(&shade);

        let badges = gtk::Box::builder()
            .spacing(5)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .margin_top(14)
            .margin_start(14)
            .can_target(false)
            .build();
        let running = gtk::Label::builder().label("Running").css_classes(["home-badge", "run"]).visible(false).build();
        let unbuilt = gtk::Box::builder().spacing(4).css_classes(["home-badge", "unbuilt"]).visible(false).build();
        unbuilt.append(&wad_gtk::Icon::new("hammer", 11));
        let unbuilt_label = gtk::Label::new(None);
        unbuilt.append(&unbuilt_label);
        let failed =
            gtk::Label::builder().label("Didn’t open").css_classes(["home-badge", "failed"]).visible(false).build();
        let machine = gtk::Box::builder().spacing(4).css_classes(["home-badge", "machine"]).visible(false).build();
        let machine_icon = wad_gtk::Icon::new("monitor", 11);
        machine_icon.set_valign(gtk::Align::Center);
        machine.append(&machine_icon);
        let machine_name = gtk::Label::new(None);
        machine.append(&machine_name);
        badges.append(&running);
        badges.append(&unbuilt);
        badges.append(&failed);
        badges.append(&machine);
        root.add_overlay(&badges);

        let ring_value = Rc::new(Cell::new(0.0));
        let ring_px = (side as f64 * 0.22).round() as i32;
        let ring = gtk::DrawingArea::builder()
            .content_width(ring_px)
            .content_height(ring_px)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        let v = ring_value.clone();
        ring.set_draw_func(move |_, cr, w, h| draw_ring(cr, w as f64, h as f64, v.get()));
        let busy = gtk::Box::builder().css_classes(["home-busy"]).visible(false).can_target(false).build();
        ring.set_hexpand(true);
        busy.append(&ring);
        root.add_overlay(&busy);

        let foot = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .valign(gtk::Align::End)
            .margin_start(18)
            .margin_end(18)
            .margin_bottom(18)
            .can_target(false)
            .build();
        let apps = gtk::Box::builder().spacing(6).halign(gtk::Align::Start).build();
        let name = gtk::Label::builder()
            .css_classes(["home-card-name"])
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .single_line_mode(true)
            .build();
        foot.append(&apps);
        foot.append(&name);
        root.add_overlay(&foot);

        Rc::new(Self {
            root,
            picture,
            running,
            unbuilt,
            unbuilt_label,
            failed,
            machine,
            machine_icon,
            machine_name,
            busy,
            ring,
            ring_value,
            apps,
            name,
            data: RefCell::default(),
        })
    }

    /// Shows `d` (only what changed).
    pub fn set(&self, d: &CardData) {
        let was = self.data.borrow().clone();
        if was.as_ref() == Some(d) {
            return;
        }
        if was.as_ref().is_none_or(|w| w.picture != d.picture || w.icon != d.icon) {
            let pictured = wad_model::wallpapers::has_picture(&d.picture);
            self.picture.set_wallpaper(&d.picture);
            self.picture.set_crop(Some(&d.icon));
            self.picture.set_visible(pictured);
            if pictured {
                self.root.remove_css_class("bare");
            } else {
                self.root.add_css_class("bare");
            }
        }
        if was.as_ref().is_none_or(|w| w.apps != d.apps) {
            while let Some(c) = self.apps.first_child() {
                self.apps.remove(&c);
            }
            for t in d.apps.iter().take(APPS) {
                let icon = match t {
                    AppTile::App(a) => wad_gtk::app_icon::app(*a, APP_SIZE),
                    AppTile::Web { id, name, url } => {
                        wad_gtk::app_icon::site(id, name.as_deref(), url.as_deref(), APP_SIZE)
                    }
                };
                icon.add_css_class("home-app");
                // A site's card expands what's in it: the row keeps each to its size.
                icon.set_hexpand(false);
                icon.set_halign(gtk::Align::Start);
                self.apps.append(&icon);
            }
            if d.apps.len() > APPS {
                let more =
                    gtk::Label::builder().label(format!("+{}", d.apps.len() - APPS)).css_classes(["home-more"]).build();
                self.apps.append(&more);
            }
            self.apps.set_visible(!d.apps.is_empty());
        }
        self.name.set_label(&d.name);
        self.running.set_visible(d.running);
        let unbuilt = match d.built {
            Some(Built::Never) => Some("Not built yet"),
            Some(Built::Update) => Some("Rebuilds"),
            _ => None,
        };
        self.unbuilt_label.set_label(unbuilt.unwrap_or(""));
        self.unbuilt.set_visible(unbuilt.is_some() && !d.failed);
        self.failed.set_visible(d.failed);
        match &d.machine {
            Some((icon, name)) => {
                self.machine_icon.set(icon);
                self.machine_name.set_label(name);
                self.machine.set_visible(true);
                self.root.add_css_class("remote");
            }
            None => {
                self.machine.set_visible(false);
                self.root.remove_css_class("remote");
            }
        }
        self.busy.set_visible(d.busy.is_some());
        self.ring_value.set(d.busy.unwrap_or(0.0));
        self.ring.queue_draw();
        *self.data.borrow_mut() = Some(d.clone());
    }

    /// How far what it's busy with has got (building, starting, copying here), if it is.
    pub fn busy(&self) -> Option<f64> {
        self.data.borrow().as_ref().and_then(|d| d.busy)
    }

    /// The one in focus (`keys`: and the keys are on the cards).
    pub fn set_focus(&self, focus: bool, keys: bool) {
        for (class, on) in [("focus", focus && !keys), ("keys", keys)] {
            if on {
                self.root.add_css_class(class);
            } else {
                self.root.remove_css_class(class);
            }
        }
    }

    pub fn set_side(&self, side: i32) {
        self.root.set_size_request(side, side);
        let px = (side as f64 * 0.22).round() as i32;
        self.ring.set_content_width(px);
        self.ring.set_content_height(px);
    }
}

/// Ring.svelte: a track, and the part done in the accent from the top, clockwise.
fn draw_ring(cr: &gtk::cairo::Context, w: f64, h: f64, value: f64) {
    let stroke = 3.0;
    let r = (w.min(h) - stroke) / 2.0;
    let (cx, cy) = (w / 2.0, h / 2.0);
    cr.set_line_width(stroke);
    cr.set_source_rgb(0x24 as f64 / 255.0, 0x1a as f64 / 255.0, 0x39 as f64 / 255.0);
    cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
    let _ = cr.stroke();
    let v = value.clamp(0.0, 1.0);
    if v > 0.0 {
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.set_source_rgb(0xc6 as f64 / 255.0, 1.0, 0x1f as f64 / 255.0);
        let start = -std::f64::consts::FRAC_PI_2;
        cr.arc(cx, cy, r, start, start + v * std::f64::consts::TAU);
        let _ = cr.stroke();
    }
}
