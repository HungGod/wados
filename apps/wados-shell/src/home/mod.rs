//! The launcher (`shell/home/Home.svelte`), where WadSpaces starts, all of
//! it by the arrow keys. Along the top, its bar: the machine, a divider,
//! Running, Recent (the five launched most lately) and the tags (Work,
//! Gaming, and yours). It opens on Running; with nothing running, on Recent;
//! with nothing launched yet, on Work. Under it, a console-style carousel of
//! square cards of that tab's wadspaces (what's running first), the one in
//! focus big in the middle with its picture behind everything (blurred and
//! shaded). Select it (a click, or Enter) to open it. Under it, its round
//! buttons: Launch (or Show), Focus, the two streams (Stream: publicly,
//! Private: on the tailnet; the stream sheet, or its link when it streams),
//! an update check (or a build), the Trash (Undo's in the toast). A
//! right-click on a card is its menu: Open (Show), Open in window, Stream ▸
//! (its modes: streams.rs's menu), Edit in the Builder (on the desktop,
//! with its group), Duplicate, Move to Trash. A card that streams says so
//! (`LIVE · n`, Starting stream).
//!
//! The machine at the bar's start picks whose wadspaces show: this one's,
//! or another of mine that's online (`peers.rs`; they come from it, over the
//! tailnet). Another machine's cards say whose they are, and their buttons
//! are Launch ("Stream from {machine}": a private stream there, opened here
//! in a window of WadBrowser's), Focus (it runs here, so it asks to copy it
//! first) and Copy here; their menu has Stream from {machine}, Stream in a
//! window, and copies it too.
//!
//! The keys (nav.rs): ←/→ the cards; ↑ the bar (←/→ along it, a tab shows
//! at once; ← past the first is the machine); ↓ the buttons, ↓ again the
//! desktop; Tab/⇧Tab the next or last tab; PageDown (machine.rs's) the
//! desktop.
//!
//! The interface machine.rs uses: `new`, `widget` (sized by machine.rs to
//! the screen above the dock, and slid/faded by it; the backdrop reaches
//! under the dock too), `key` (the chrome's keys, before machine.rs's own),
//! `shown` (each time the page is entered: the keys go back on the cards,
//! and the launcher takes the chrome's keyboard focus, so Enter and Space
//! are its own and not a dock button's).
//!
//! `wadsctl home` says where things are (and `moving=` while the row or the
//! picture behind is still moving); `wadsctl home key <key>…`, `tab <id>`,
//! `focus <id>`, `menu <id>` (a card's, as a right-click; `menu sub <row>`
//! opens the open menu's submenu: `menu sub Stream`), `machine` (its
//! list) and `machine <id>` drive it; `wadsctl home layout` gives its
//! pieces' bounds, in points.
//!
//! Not here yet: Focus (later), whose button shows, dimmed, its tooltip
//! saying when; drops on a card.

mod backdrop;
mod bar;
mod buttons;
mod card;
mod layout;
mod menu;
mod model;
mod nav;
mod stage;
mod tween;

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use gtk::{gdk, glib, prelude::*};
use wad_model::edit::Op;
use wad_model::{Built, SessionDisplay, Wadspace, Wallpaper};
use wad_proto::Level;

use crate::machine::Page;
use crate::peers::{self, Choice};
use crate::shell::Shell;
use crate::store::{Change, Store};
use crate::streams;
use wad_model::StreamMode;

use self::backdrop::Backdrop;
use self::bar::{Bar, TabView};
use self::buttons::Buttons;
use self::card::{Card, CardData, StreamBadge, apps_of};
use self::layout::{BAR_H, Layout};
use self::menu::{Item, Row};
use self::model::{ActId, Action, Space, Tab};
use self::nav::{Do, Have, Key, Nav, Zone};
use self::stage::{Place, Stage};
use self::tween::{self as tweens, Tween};

const CSS: &str = include_str!("home.css");
/// The row's motion: the spike's 0.45 s on cubic-bezier(0.2, 0.85, 0.25, 1).
const MOVE: Duration = Duration::from_millis(450);
/// How long after wadd first sends its library the launcher picks the tab
/// it opens on (what's running comes just after the library).
const SETTLE: Duration = Duration::from_millis(400);
/// A sideways scroll moves one card at most this often (the spike's 380 ms).
const WHEEL_GAP: Duration = Duration::from_millis(380);

pub struct Launcher {
    shell: Rc<Shell>,
    me: OnceCell<Weak<Launcher>>,
    stage: Stage,
    backdrop: Backdrop,
    bar: Bar,
    buttons: Buttons,
    empty: Empty,
    /// The menu open (the machine's list, or a card's), if one is.
    menu: RefCell<Option<gtk::Popover>>,
    cards: RefCell<Vec<(String, Rc<Card>)>>,
    st: RefCell<State>,
    pos: RefCell<Option<Tween>>,
    queued: Cell<bool>,
    /// Watching the chrome's focus (to keep it).
    focus_held: Cell<bool>,
    /// The text sizes that follow the screen's.
    sizes: gtk::CssProvider,
}

#[derive(Default)]
struct State {
    /// The tab picked (none yet: the one it opens on, worked out afresh).
    view: Option<Tab>,
    /// Another machine picked (none: this one).
    machine: Option<String>,
    /// The card in focus, and its place (kept when it goes: the next one takes it).
    focus: Option<String>,
    focus_at: usize,
    nav: Nav,
    /// What the last refresh showed.
    shown: Shown,
    /// A sideways scroll: how far, and when it last moved the row.
    wheel: (f64, Option<Instant>),
}

#[derive(Default, Clone)]
struct Shown {
    tabs: Vec<Tab>,
    counts: Vec<usize>,
    view: Option<Tab>,
    list: Vec<String>,
    at: usize,
    actions: Vec<Action>,
    layout: Option<Layout>,
    /// The machine shown: its name, and its id.
    machine: String,
    machine_id: String,
    /// The machines it can show (this one first).
    machines: Vec<Choice>,
}

impl Shown {
    fn view_at(&self) -> usize {
        self.view.as_ref().and_then(|v| self.tabs.iter().position(|t| t == v)).unwrap_or(0)
    }
}

impl Launcher {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let sizes = gtk::CssProvider::new();
        if let Some(d) = gdk::Display::default() {
            wad_gtk::theme::add_css(&d, CSS);
            gtk::style_context_add_provider_for_display(&d, &sizes, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
        }
        let stage = Stage::default();
        // It holds the chrome's keyboard focus (hold_focus), so Enter and
        // Space are the launcher's, not a dock button's.
        stage.set_focusable(true);
        let backdrop = Backdrop::new();
        stage.put(&backdrop, Place::at(0.0, 0.0));
        let empty = Empty::new();
        stage.put(&empty.root, Place::at(0.0, 0.0));
        let buttons = Buttons::new();
        stage.put(&buttons.root, Place::at(0.0, 0.0));
        let bar = Bar::new();
        stage.put(&bar.root, Place::at(0.0, 0.0));

        let me = Rc::new(Self {
            shell: shell.clone(),
            me: OnceCell::new(),
            stage,
            backdrop,
            bar,
            buttons,
            empty,
            menu: RefCell::default(),
            cards: RefCell::default(),
            st: RefCell::default(),
            pos: RefCell::default(),
            queued: Cell::new(false),
            focus_held: Cell::new(false),
            sizes,
        });
        let _ = me.me.set(Rc::downgrade(&me));

        let w = Rc::downgrade(&me);
        let row = Tween::new(&me.stage, MOVE, tweens::MOVE, 0.0, move |p| {
            if let Some(m) = w.upgrade() {
                m.lay_cards(p);
            }
        });
        *me.pos.borrow_mut() = Some(row);

        me.connect();
        me.refresh();
        me
    }

    pub fn widget(&self) -> gtk::Widget {
        self.stage.clone().upcast()
    }

    /// One of the chrome's keys: true if the launcher took it.
    pub fn key(&self, key: gdk::Key, mods: gdk::ModifierType) -> bool {
        if mods.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK
                | gdk::ModifierType::META_MASK,
        ) {
            return false;
        }
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        let k = match key {
            gdk::Key::Left | gdk::Key::KP_Left => Key::Left,
            gdk::Key::Right | gdk::Key::KP_Right => Key::Right,
            gdk::Key::Up | gdk::Key::KP_Up => Key::Up,
            gdk::Key::Down | gdk::Key::KP_Down => Key::Down,
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter => Key::Enter,
            gdk::Key::space => Key::Space,
            gdk::Key::Escape => Key::Escape,
            gdk::Key::ISO_Left_Tab => Key::BackTab,
            gdk::Key::Tab | gdk::Key::KP_Tab => {
                if shift {
                    Key::BackTab
                } else {
                    Key::Tab
                }
            }
            _ => return false,
        };
        self.press(k)
    }

    /// Each time the page is entered: the keys are on the cards (another
    /// machine's wadspaces are asked for again, if they're old).
    pub fn shown(&self) {
        self.st.borrow_mut().nav.shown();
        self.hold_focus();
        self.refresh();
        let peer = self.peer_shown();
        if let Some(p) = peer {
            peers::refresh(&self.shell, &p);
        }
    }

    /// Another machine, if it's the one shown.
    fn peer_shown(&self) -> Option<String> {
        let st = self.st.borrow();
        let s = &st.shown;
        s.machines.iter().find(|c| c.id == s.machine_id && !c.here).map(|c| c.id.clone())
    }

    /// The chrome's keyboard focus on the launcher, and back on it whenever
    /// something else takes it (a click on a dock button): the window gives
    /// it to its first button otherwise, and Enter and Space would press it
    /// before the launcher saw them.
    fn hold_focus(&self) {
        let Some(win) = self.stage.root().and_downcast::<gtk::Window>() else { return };
        if !self.focus_held.replace(true) {
            let w = self.weak();
            win.connect_focus_widget_notify(move |win| {
                let Some(m) = w.upgrade() else { return };
                let mine = GtkWindowExt::focus(win).is_some_and(|f| f == *m.stage.upcast_ref::<gtk::Widget>());
                if !mine && m.shell.page() == Page::Launcher {
                    m.stage.grab_focus();
                }
            });
        }
        self.stage.grab_focus();
    }

    // ------------------------------------------------------------ wiring

    fn weak(&self) -> Weak<Launcher> {
        self.me.get().cloned().unwrap_or_default()
    }

    fn connect(self: &Rc<Self>) {
        // What wadd says: shown on the next idle (a burst of events is one refresh).
        let w = Rc::downgrade(self);
        let first = Cell::new(true);
        self.shell.store.watch(move |c| {
            let Some(m) = w.upgrade() else { return };
            if let Change::Notice { .. } = c {
                return;
            }
            if *c == Change::Library && first.replace(false) {
                let w = Rc::downgrade(&m);
                glib::timeout_add_local_once(SETTLE, move || {
                    if let Some(m) = w.upgrade() {
                        m.open_on_first();
                    }
                });
            }
            m.queue_refresh();
        });

        // The machine: its list.
        let click = gtk::GestureClick::new();
        let w = Rc::downgrade(self);
        click.connect_released(move |g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(m) = w.upgrade() {
                {
                    let mut st = m.st.borrow_mut();
                    st.nav.zone = Zone::Top;
                    st.nav.top = 0;
                }
                m.refresh();
                m.open_menu();
            }
        });
        self.bar.machine.add_controller(click);

        // Nothing running: a way to Recent.
        let click = gtk::GestureClick::new();
        let w = Rc::downgrade(self);
        click.connect_released(move |g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(m) = w.upgrade() {
                m.pick(Tab::Recent);
            }
        });
        self.empty.recent.add_controller(click);

        // A sideways scroll moves along the cards (down is machine.rs's: the desktop).
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let w = Rc::downgrade(self);
        scroll.connect_scroll(move |c, dx, dy| {
            let Some(m) = w.upgrade() else { return glib::Propagation::Proceed };
            let shift = c.current_event_state().contains(gdk::ModifierType::SHIFT_MASK);
            if dx.abs() <= dy.abs() && !shift {
                return glib::Propagation::Proceed;
            }
            let pixels = c.unit() == gdk::ScrollUnit::Surface;
            m.wheel(if dx != 0.0 { dx } else { dy }, if pixels { 24.0 } else { 1.0 });
            glib::Propagation::Stop
        });
        self.stage.add_controller(scroll);

        // wadsctl home …
        let w = Rc::downgrade(self);
        self.shell.on_ctl("home", move |args| match w.upgrade() {
            Some(m) => m.ctl(args),
            None => "error the launcher is gone".into(),
        });
    }

    fn queue_refresh(&self) {
        if self.queued.replace(true) {
            return;
        }
        let w = self.weak();
        glib::idle_add_local_once(move || {
            if let Some(m) = w.upgrade() {
                m.refresh();
            }
        });
    }

    /// Opened: the tab it opens on stays (until another's picked).
    fn open_on_first(&self) {
        self.refresh();
        let mut st = self.st.borrow_mut();
        if st.view.is_none() {
            st.view = st.shown.view.clone();
            st.nav.zone = Zone::Cards;
        }
    }

    // ------------------------------------------------------------ what it shows

    fn refresh(&self) {
        self.queued.set(false);
        let store = &self.shell.store;
        let this_id = store.machine_id();
        let this_name = store.machine().map(|m| m.name).unwrap_or_else(|| "This machine".into());
        let machines = peers::machines(&this_id, &this_name, &store.peers());
        // Another machine picked shows while it's online; then this one again.
        let picked = self.st.borrow().machine.clone();
        let choice =
            picked.and_then(|p| machines.iter().find(|c| c.id == p).cloned()).unwrap_or_else(|| machines[0].clone());
        let (live, waiting) = if choice.here {
            (store.live(), None)
        } else {
            let got = store.peer_spaces(&choice.id).unwrap_or_default();
            let waiting = (got.at == 0).then(|| model::peer_wait_text(&choice.name, got.error.as_deref()));
            (got.list, waiting)
        };
        let tags = store.library().tags.clone();
        let spaces: Vec<Space> = live.iter().map(|x| space_of(store, x, &tags, choice.here)).collect();
        let tabs = model::tabs(&tags);
        let (w, h) = self.shell.size();
        let layout = Layout::new(w as f64, h as f64);
        let machine = choice.name.clone();
        let machine_id = choice.id.clone();

        let (was, shown, nav) = {
            let mut st = self.st.borrow_mut();
            let view = model::shown(st.view.as_ref().unwrap_or(&model::opens_on(&spaces)), &tabs);
            let list: Vec<String> = model::spaces_of(&view, &spaces).iter().map(|x| x.id.clone()).collect();
            let at = st
                .focus
                .as_ref()
                .and_then(|f| list.iter().position(|x| x == f))
                .unwrap_or(st.focus_at.min(list.len().saturating_sub(1)));
            st.focus_at = at;
            st.focus = list.get(at).cloned();
            let actions = st
                .focus
                .as_ref()
                .and_then(|f| live.iter().find(|x| &x.id == f))
                .map(|x| {
                    if choice.here {
                        let (live, avail) = (store.stream(&x.id), store.stream_modes());
                        model::actions(
                            &x.name,
                            store.is_running(&x.id),
                            store.build(&x.id).map(|b| b.state.built),
                            streams::card_button(StreamMode::Public, live.as_ref(), avail.as_ref()),
                            streams::card_button(StreamMode::Private, live.as_ref(), avail.as_ref()),
                        )
                    } else {
                        model::peer_actions(&x.name, &choice.name)
                    }
                })
                .unwrap_or_default();
            st.nav.settle(actions.len());
            let counts = tabs.iter().map(|t| model::spaces_of(t, &spaces).len()).collect();
            let shown = Shown {
                tabs,
                counts,
                view: Some(view),
                list,
                at,
                actions,
                layout: Some(layout),
                machine,
                machine_id,
                machines: machines.clone(),
            };
            let was = std::mem::replace(&mut st.shown, shown.clone());
            (was, shown, st.nav)
        };

        if was.layout.map(|l| (l.w, l.h)) != Some((layout.w, layout.h)) {
            self.sizes
                .load_from_string(&format!(".home-machine, .home-tab {{ font-size: {:.1}px; }}", layout.tab_font()));
        }

        // The bar.
        let views: Vec<TabView> = shown
            .tabs
            .iter()
            .zip(&shown.counts)
            .map(|(t, &count)| TabView { tab: t.clone(), label: t.label(&tags), count })
            .collect();
        let view = shown.view.clone().unwrap_or(Tab::Work);
        self.bar.set_machine(&shown.machine, choice.icon, machines.len() > 1);
        self.bar.set(&views, &view, (nav.zone == Zone::Top).then_some(nav.top), &self.on_tab());

        // The cards, the buttons, the picture behind.
        let badge = (!choice.here).then(|| {
            let kind = store.peer(&choice.id).map(|p| p.peer.kind).unwrap_or(wad_model::PeerKind::Desktop);
            (peers::kind_icon(kind), choice.name.clone())
        });
        self.sync_cards(&was, &shown, &live, nav, badge);
        self.buttons.set(&shown.actions, (nav.zone == Zone::Actions).then_some(nav.act), &self.on_act());
        let focused = shown.list.get(shown.at).and_then(|id| live.iter().find(|x| &x.id == id));
        let picture = focused.map(|x| x.look.wallpaper.clone()).unwrap_or_else(wad_model::wallpapers::default_desktop);
        self.backdrop.set(&picture, layout.w, layout.h);
        let on = (!choice.here).then_some(choice.name.as_str());
        self.empty.set(&view, &tags, shown.list.is_empty(), on, waiting);

        // Where the pieces go.
        let l = layout;
        self.stage.place(&self.backdrop, Place::at(0.0, 0.0).sized(l.w as i32, l.h as i32));
        self.stage.place(&self.bar.root, Place::anchored(l.pad - 14.0, l.bar_top + BAR_H / 2.0, 0.0, 0.5));
        self.stage.place(&self.buttons.root, Place::anchored(l.w / 2.0, l.acts_top(), 0.5, 0.0));
        self.stage.place(&self.empty.root, Place::anchored(l.w / 2.0, l.row_top + l.side / 2.0, 0.5, 0.5));

        // The row: another tab shows at once; another card in focus, it moves there.
        let spring = self.pos.borrow().clone();
        if let Some(s) = spring {
            let at = shown.at as f64;
            if was.view != shown.view || was.layout != shown.layout || was.machine_id != shown.machine_id {
                s.jump(at);
            } else if s.target() != at {
                s.to(at);
            } else {
                self.lay_cards(s.value());
            }
        }
    }

    fn sync_cards(
        &self,
        was: &Shown,
        shown: &Shown,
        live: &[Wadspace],
        nav: Nav,
        machine: Option<(&'static str, String)>,
    ) {
        let side = shown.layout.map(|l| l.side as i32).unwrap_or(380);
        let mut cards = self.cards.borrow_mut();
        let mut old: HashMap<String, Rc<Card>> = cards.drain(..).collect();
        let mut next = Vec::with_capacity(shown.list.len());
        for id in &shown.list {
            let Some(x) = live.iter().find(|x| &x.id == id) else { continue };
            let card = match old.remove(id) {
                Some(c) => c,
                None => self.make_card(id, side),
            };
            card.set(&card_data(&self.shell.store, x, machine.clone()));
            card.set_side(side);
            next.push((id.clone(), card));
        }
        for (_, c) in old {
            self.stage.remove(&c.root);
        }
        for (i, (_, c)) in next.iter().enumerate() {
            c.set_focus(i == shown.at, i == shown.at && nav.zone == Zone::Cards);
        }
        // The nearer the one in focus, the higher.
        if was.list != shown.list || was.at != shown.at || next.len() != cards.len() {
            let mut order: Vec<usize> = (0..next.len()).collect();
            order.sort_by_key(|&i| std::cmp::Reverse(i.abs_diff(shown.at)));
            for i in order {
                next[i].1.root.insert_before(&self.stage, Some(&self.empty.root));
            }
        }
        *cards = next;
    }

    fn make_card(&self, id: &str, side: i32) -> Rc<Card> {
        let card = Card::new(side);
        let click = gtk::GestureClick::new();
        let (w, card_id) = (self.weak(), id.to_string());
        let opened = Cell::new(false);
        click.connect_released(move |g, n, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            let Some(m) = w.upgrade() else { return };
            if n == 1 {
                opened.set(m.click_card(&card_id));
            } else if n == 2 && !opened.get() {
                // A double click on one beside: the first brought it to the middle.
                m.open(&card_id);
            }
        });
        card.root.add_controller(click);
        // Its menu, on a right-click.
        let menu = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let (w, id, c) = (self.weak(), id.to_string(), Rc::downgrade(&card));
        menu.connect_pressed(move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let (Some(m), Some(c)) = (w.upgrade(), c.upgrade()) {
                m.st.borrow_mut().nav.zone = Zone::Cards;
                m.card_menu(&id, &c, x, y);
            }
        });
        card.root.add_controller(menu);
        self.stage.put_under(&card.root, Some(self.empty.root.upcast_ref()), Place::at(0.0, 0.0));
        card
    }

    /// Every card where it is with the row at `pos` (the place in focus, fractional while it moves).
    fn lay_cards(&self, pos: f64) {
        let Some(l) = self.st.borrow().shown.layout else { return };
        let side = l.side as i32;
        for (i, (_, c)) in self.cards.borrow().iter().enumerate() {
            let p = l.place(i as f64 - pos);
            let shown = p.opacity > 0.0;
            if c.root.is_visible() != shown {
                c.root.set_visible(shown);
            }
            c.root.set_opacity(p.opacity);
            c.root.set_can_target(p.opacity > 0.05);
            self.stage.place(
                &c.root,
                Place::anchored(l.w / 2.0 + p.x, l.row_top + l.side / 2.0, 0.5, 0.5).sized(side, side).scaled(p.scale),
            );
        }
    }

    fn on_tab(&self) -> Rc<dyn Fn(usize)> {
        let w = self.weak();
        Rc::new(move |i| {
            let Some(m) = w.upgrade() else { return };
            let tab = {
                let mut st = m.st.borrow_mut();
                st.nav.zone = Zone::Top;
                st.nav.top = i + 1;
                st.shown.tabs.get(i).cloned()
            };
            if let Some(t) = tab {
                m.pick(t);
            }
        })
    }

    fn on_act(&self) -> Rc<dyn Fn(usize)> {
        let w = self.weak();
        Rc::new(move |i| {
            let Some(m) = w.upgrade() else { return };
            {
                let mut st = m.st.borrow_mut();
                st.nav.zone = Zone::Actions;
                st.nav.act = i;
            }
            m.refresh();
            m.act(i);
        })
    }

    // ------------------------------------------------------------ doing things

    /// A key, as the person pressed it (or wadsctl did).
    fn press(&self, k: Key) -> bool {
        let done = {
            let mut st = self.st.borrow_mut();
            let s = &st.shown;
            let have =
                Have { tabs: s.tabs.len(), view: s.view_at(), cards: s.list.len(), at: s.at, actions: s.actions.len() };
            st.nav.key(k, &have)
        };
        tracing::debug!(?k, ?done, "launcher key");
        let Some(done) = done else { return false };
        match done {
            Do::Moved => self.refresh(),
            Do::NextMachine => {
                let next = {
                    let st = self.st.borrow();
                    let m = &st.shown.machines;
                    let i = m.iter().position(|c| c.id == st.shown.machine_id).unwrap_or(0);
                    m.get((i + 1) % m.len().max(1)).map(|c| c.id.clone())
                };
                match next {
                    Some(id) => self.pick_machine(&id),
                    None => self.refresh(),
                }
            }
            Do::Pick(i) => {
                let tab = self.st.borrow().shown.tabs.get(i).cloned();
                if let Some(t) = tab {
                    self.pick(t);
                }
            }
            Do::Focus(i) => {
                {
                    let mut st = self.st.borrow_mut();
                    let id = st.shown.list.get(i).cloned();
                    if id.is_some() {
                        st.focus = id;
                        st.focus_at = i;
                    }
                }
                self.refresh();
            }
            Do::Open => {
                if let Some(id) = self.focused() {
                    self.open(&id);
                }
            }
            Do::Act(i) => self.act(i),
            Do::Desktop => self.shell.go(Page::Desktop),
        }
        true
    }

    /// Shows `id`'s wadspaces (of the tab picked), from its first; another
    /// machine's are asked for again if they're old.
    fn pick_machine(&self, id: &str) {
        let peer = {
            let mut st = self.st.borrow_mut();
            let here = st.shown.machines.iter().find(|c| c.id == id).is_none_or(|c| c.here);
            let want = (!here).then(|| id.to_string());
            if st.machine != want {
                st.machine = want.clone();
                st.focus = None;
                st.focus_at = 0;
                st.nav.act = 0;
            }
            want
        };
        self.refresh();
        if let Some(p) = peer {
            peers::refresh(&self.shell, &p);
        }
    }

    /// Shows `tab`'s wadspaces, from its first.
    fn pick(&self, tab: Tab) {
        {
            let mut st = self.st.borrow_mut();
            if st.shown.view.as_ref() != Some(&tab) {
                st.focus = None;
                st.focus_at = 0;
                st.nav.act = 0;
            }
            st.view = Some(tab);
        }
        self.refresh();
    }

    /// A click on a card: one beside comes to the middle; the one there opens. True if it opened.
    fn click_card(&self, id: &str) -> bool {
        let focused = {
            let mut st = self.st.borrow_mut();
            st.nav.zone = Zone::Cards;
            let focused = st.focus.as_deref() == Some(id);
            if !focused {
                st.focus = Some(id.to_string());
            }
            focused
        };
        self.refresh();
        if focused {
            self.open(id);
        }
        focused
    }

    fn focused(&self) -> Option<String> {
        self.st.borrow().focus.clone()
    }

    /// Opens it full screen (built first, if it has to be); one running is
    /// shown so. Another machine's streams from there ("Stream from
    /// {machine}": privately, into a window of WadBrowser's here).
    fn open(&self, id: &str) {
        if self.peer_shown().is_some() {
            return streams::stream_from(id, false);
        }
        self.shell.open(id, SessionDisplay::Screen);
    }

    /// The card in focus's button `i`.
    fn act(&self, i: usize) {
        let Some(a) = self.st.borrow().shown.actions.get(i).cloned() else { return };
        let Some(id) = self.focused() else { return };
        if !a.enabled {
            return;
        }
        match a.id {
            ActId::Launch => self.open(&id),
            ActId::Update => {
                let built = self.shell.store.build(&id).is_none_or(|b| b.state.built == Built::Built);
                if built {
                    self.check_for_updates(&id);
                } else {
                    self.shell.build(&id);
                }
            }
            ActId::Trash => self.shell.op(Op::Trash { ws: id }, |_| {}),
            // Another machine's cards are its wadspaces' keys (`<peer>:<id>`).
            ActId::Copy => {
                if self.peer_shown().is_some() {
                    peers::copy_here(&self.shell, &id);
                }
            }
            ActId::Focus => match self.peer_shown() {
                Some(p) => self.focus_elsewhere(&p, &id),
                None => self.shell.focus_ui(|f| f.setup(&id)),
            },
            ActId::Stream | ActId::Private => {
                let mode = if a.id == ActId::Stream { StreamMode::Public } else { StreamMode::Private };
                let store = &self.shell.store;
                let b = streams::card_button(mode, store.stream(&id).as_ref(), store.stream_modes().as_ref());
                streams::pick(&id, b.pick);
            }
        }
    }

    /// Focus on another machine's (`id`: its key): it runs here, so it asks
    /// to copy it first (the spike's).
    fn focus_elsewhere(&self, peer: &str, id: &str) {
        let store = &self.shell.store;
        let name = store.peer_wadspace(id).map(|(_, x)| x.name).unwrap_or_else(|| id.to_string());
        let from = store.machine_name(peer).unwrap_or_else(|| "another machine".into());
        let (shell, key) = (self.shell.clone(), id.to_string());
        crate::overlays::confirm(crate::overlays::Confirm {
            title: format!("Focus in {name}?"),
            icon: "timer",
            body: format!(
                "Focus runs a wadspace on this machine, and {name} lives on {from}. Copy it here, then focus in the copy."
            ),
            ok: "Copy it here".into(),
            run: Rc::new(move || peers::copy_here(&shell, &key)),
        });
    }

    /// Built: a build of what it's made of now, which says it's up to date when there's nothing to do.
    fn check_for_updates(&self, id: &str) {
        let name = self.shell.store.wadspace(id).map(|x| x.name).unwrap_or_else(|| id.to_string());
        let shell = self.shell.clone();
        self.shell.link.build(id, move |a| {
            if a.job.is_none() {
                shell.toast(Level::Ok, &format!("{name} is up to date"), None);
            }
        });
    }

    fn wheel(&self, d: f64, step: f64) {
        let dir = {
            let mut st = self.st.borrow_mut();
            if st.wheel.1.is_some_and(|t| t.elapsed() < WHEEL_GAP) {
                return;
            }
            st.wheel.0 += d;
            if st.wheel.0.abs() < step {
                return;
            }
            let dir = st.wheel.0.signum();
            st.wheel = (0.0, Some(Instant::now()));
            dir
        };
        let at = self.st.borrow().shown.at;
        let i = if dir > 0.0 { at + 1 } else { at.saturating_sub(1) };
        let n = self.st.borrow().shown.list.len();
        if i < n {
            let mut st = self.st.borrow_mut();
            st.focus = st.shown.list.get(i).cloned();
            st.focus_at = i;
            drop(st);
            self.refresh();
        }
    }

    /// The machines' list: this one, and my others that are online.
    fn open_menu(&self) {
        let m = &self.bar.machine;
        let (machines, at) = {
            let st = self.st.borrow();
            (st.shown.machines.clone(), st.shown.machine_id.clone())
        };
        let mut items = vec![Item::Heading("Wadspaces on".into())];
        for c in machines {
            let (w, id) = (self.weak(), c.id.clone());
            let row = Row::new(c.icon, peers::choice_label(&c), move || {
                if let Some(m) = w.upgrade() {
                    m.pick_machine(&id);
                }
            });
            items.push(Item::Row(if c.id == at { row.checked() } else { row }));
        }
        let rect = gdk::Rectangle::new(0, 0, m.width(), m.height());
        m.add_css_class("open");
        let machine = m.clone();
        self.show_menu(menu::popover(m, items), rect, move || machine.remove_css_class("open"));
    }

    /// A card's right-click menu (the spike's wadspaceMenu, what works today), at (`x`, `y`) on it.
    fn card_menu(&self, id: &str, card: &Rc<Card>, x: f64, y: f64) {
        if let Some(p) = self.peer_shown() {
            return self.peer_card_menu(&p, id, card, x, y);
        }
        let Some(ws) = self.shell.store.wadspace(id) else { return };
        let running = self.shell.store.is_running(id);
        let shell = &self.shell;
        let open = {
            let (shell, id) = (shell.clone(), id.to_string());
            move || shell.open(&id, SessionDisplay::Screen)
        };
        let window = {
            let (shell, id) = (shell.clone(), id.to_string());
            move || shell.open(&id, SessionDisplay::Window)
        };
        // The Builder's a window on the desktop (the spike's showSpace): there, with its group.
        let edit = {
            let (shell, id) = (shell.clone(), id.to_string());
            move || {
                shell.go(Page::Desktop);
                shell.edit(&id);
            }
        };
        let duplicate = {
            let (shell, id) = (shell.clone(), id.to_string());
            move || shell.op(Op::Duplicate { ws: id.clone(), machine: None }, |_| {})
        };
        let trash = {
            let (shell, id) = (shell.clone(), id.to_string());
            move || shell.op(Op::Trash { ws: id.clone() }, |_| {})
        };
        let here = ws.machine == shell.store.machine_id();
        let mut items = vec![
            Item::Heading(ws.name.clone()),
            Item::Row(Row::new("monitor-play", if running { "Show" } else { "Open" }, open)),
            Item::Row(Row::new("app-window", "Open in window", window)),
        ];
        for d in shell.displays().into_iter().skip(1) {
            let (s, i) = (shell.clone(), id.to_string());
            let what = if running { "Show" } else { "Open" };
            let label = format!("{what} on {}: {}", d.n, d.name);
            items.push(Item::Row(Row::new("monitor-up", label, move || s.open_on(&i, &d.connector))));
        }
        items.push(Item::Row(Row::new("radio", "Stream", || {}).submenu(stream_rows(&shell.store, id))));
        // Another machine's shows in the Builder but doesn't change there: as the spike's, it's only this machine's.
        if here {
            items.push(Item::Row(Row::new("blocks", "Edit in the Builder", edit).hint("What's in it, to change")));
        }
        items.extend([
            Item::Row(Row::new("copy-plus", "Duplicate", duplicate).hint("A copy beside it, to change")),
            Item::Line,
            Item::Row(Row::new("trash", "Move to Trash", trash).danger()),
        ]);
        let p = card
            .root
            .compute_point(&self.stage, &gtk::graphene::Point::new(x as f32, y as f32))
            .unwrap_or_else(|| gtk::graphene::Point::new(x as f32, y as f32));
        card.root.add_css_class("menu-for");
        let root = card.root.clone();
        self.show_menu(
            menu::popover(&self.stage, items),
            gdk::Rectangle::new(p.x() as i32, p.y() as i32, 1, 1),
            move || root.remove_css_class("menu-for"),
        );
    }

    /// Another machine's card's menu (the spike's wadspaceMenu, not here):
    /// streams from there (full screen, or in a window), the Builder (to see
    /// what's in it), Copy to this machine (from there: it has to be online).
    fn peer_card_menu(&self, peer: &str, id: &str, card: &Rc<Card>, x: f64, y: f64) {
        let Some((_, ws)) = self.shell.store.peer_wadspace(id) else { return };
        let shell = &self.shell;
        let at = shell.store.machine_name(peer).unwrap_or_else(|| "another machine".into());
        let streamed = |window: bool| {
            let key = id.to_string();
            move || streams::stream_from(&key, window)
        };
        let show = {
            let (shell, id) = (shell.clone(), id.to_string());
            move || {
                shell.go(Page::Desktop);
                if let Some(b) = shell.builder() {
                    b.open(&id, None);
                }
            }
        };
        let copy = {
            let (shell, key) = (shell.clone(), id.to_string());
            move || peers::copy_here(&shell, &key)
        };
        let items = vec![
            Item::Heading(format!("{} · on {at}", ws.name)),
            Item::Row(
                Row::new("monitor-play", format!("Stream from {at}"), streamed(false))
                    .hint("Privately, over your tailnet"),
            ),
            Item::Row(Row::new("app-window", "Stream in a window", streamed(true))),
            Item::Row(Row::new("blocks", "Show in the Builder", show).hint("What’s in it: copy it here to change it")),
            Item::Row(Row::new("download", "Copy to this machine", copy).hint(format!("From {at}, while it’s online"))),
            Item::Line,
            Item::Row(Row::new("trash", "Move to Trash", || {}).later(format!("It lives on {at}"))),
        ];
        let p = card
            .root
            .compute_point(&self.stage, &gtk::graphene::Point::new(x as f32, y as f32))
            .unwrap_or_else(|| gtk::graphene::Point::new(x as f32, y as f32));
        card.root.add_css_class("menu-for");
        let root = card.root.clone();
        self.show_menu(
            menu::popover(&self.stage, items),
            gdk::Rectangle::new(p.x() as i32, p.y() as i32, 1, 1),
            move || root.remove_css_class("menu-for"),
        );
    }

    /// Opens `menu` at `rect` (in its parent); `closed` when it goes (it's let go then).
    fn show_menu(&self, menu: gtk::Popover, rect: gdk::Rectangle, closed: impl Fn() + 'static) {
        if let Some(old) = self.menu.borrow_mut().take() {
            old.popdown();
        }
        menu.set_offset(0, 8);
        menu.set_pointing_to(Some(&rect));
        let w = self.weak();
        menu.connect_closed(move |p| {
            closed();
            let p = p.clone();
            let w = w.clone();
            // Let go of it once it's closed (not in its own signal).
            glib::idle_add_local_once(move || {
                if let Some(m) = w.upgrade() {
                    let mut open = m.menu.borrow_mut();
                    if open.as_ref() == Some(&p) {
                        open.take();
                    }
                }
                p.unparent();
            });
        });
        *self.menu.borrow_mut() = Some(menu.clone());
        menu.popup();
    }

    // ------------------------------------------------------------ wadsctl

    fn ctl(&self, args: &[&str]) -> String {
        match args {
            [] | ["state"] => self.state(),
            ["key", keys @ ..] if !keys.is_empty() => {
                // As the keyboard's: only while the launcher's the page.
                if self.shell.page() != Page::Launcher {
                    return "error the launcher isn't the page (wadsctl page home)".into();
                }
                for k in keys {
                    let Some(k) = Key::parse(k) else { return format!("error no key {k:?}") };
                    self.press(k);
                }
                self.state()
            }
            ["tab", id] => match Tab::parse(id) {
                Some(t) if self.st.borrow().shown.tabs.contains(&t) => {
                    self.pick(t);
                    self.state()
                }
                _ => format!("error no tab {id:?}"),
            },
            ["focus", id] => {
                let i = self.st.borrow().shown.list.iter().position(|x| x == id);
                match i {
                    Some(i) => {
                        {
                            let mut st = self.st.borrow_mut();
                            st.focus = Some(id.to_string());
                            st.focus_at = i;
                        }
                        self.refresh();
                        self.state()
                    }
                    None => format!("error {id:?} isn't on this tab"),
                }
            }
            ["layout"] => self.layout(),
            ["menu", "sub", label @ ..] if !label.is_empty() => {
                if menu::open_sub(&label.join(" ")) {
                    "ok".into()
                } else {
                    format!("error no open menu has a {:?} submenu", label.join(" "))
                }
            }
            ["menu", "close"] => {
                if let Some(m) = self.menu.borrow().clone() {
                    m.popdown();
                }
                "ok".into()
            }
            ["menu", id] => {
                let card = self.cards.borrow().iter().find(|(x, _)| x == id).map(|(_, c)| c.clone());
                match card {
                    Some(c) => {
                        let side = c.root.width() as f64;
                        self.card_menu(id, &c, side / 2.0, side / 2.0);
                        "ok".into()
                    }
                    None => format!("error {id:?} isn't on this tab"),
                }
            }
            ["menu"] => {
                let open = self.menu.borrow().as_ref().is_some_and(|m| m.is_visible());
                format!("ok {}", if open { "open" } else { "closed" })
            }
            ["machine"] => {
                self.open_menu();
                "ok".into()
            }
            ["machine", id] => {
                if !self.st.borrow().shown.machines.iter().any(|c| c.id == *id) {
                    return format!("error {id:?} isn't one to pick (this machine, or mine online)");
                }
                self.pick_machine(id);
                self.state()
            }
            ["act", n] => match n.parse::<usize>() {
                Ok(i) if i < self.st.borrow().shown.actions.len() => {
                    {
                        let mut st = self.st.borrow_mut();
                        st.nav.zone = Zone::Actions;
                        st.nav.act = i;
                    }
                    self.refresh();
                    self.act(i);
                    self.state()
                }
                _ => format!("error no button {n}"),
            },
            _ => "error home [state] | key <left|right|up|down|enter|space|esc|tab|backtab>… | tab <id> | focus <id> | layout | menu [<id>: its card's, as a right-click | sub <row> | close] | machine [<id>] | act <n>".into(),
        }
    }

    /// Where the keys are and what shows: `wadsctl home`.
    fn state(&self) -> String {
        let st = self.st.borrow();
        let s = &st.shown;
        let tabs: Vec<String> = s.tabs.iter().zip(&s.counts).map(|(t, c)| format!("{}:{c}", t.id())).collect();
        let actions: Vec<String> = s
            .actions
            .iter()
            .map(|a| a.label.to_lowercase().replace(' ', "-"))
            .zip(s.actions.iter())
            .map(|(l, a)| if a.enabled { l } else { format!("{l}(off)") })
            .collect();
        let backdrop = match self.backdrop.wallpaper() {
            Some(Wallpaper::Image { value }) => value,
            Some(Wallpaper::Color { value }) => value,
            _ => "none".into(),
        };
        format!(
            "ok tab={} zone={} top={} act={} at={} focus={} busy={} machine={:?} machine_id={} machines={} tabs={} cards={} actions={} backdrop={} moving={}",
            s.view.as_ref().map(|v| v.id()).unwrap_or_default(),
            st.nav.zone.id(),
            st.nav.top,
            st.nav.act,
            s.at,
            st.focus.as_deref().unwrap_or("none"),
            self.cards
                .borrow()
                .get(s.at)
                .and_then(|(_, c)| c.busy())
                .map(|b| format!("{b:.2}"))
                .unwrap_or_else(|| "none".into()),
            s.machine,
            s.machine_id,
            s.machines.iter().map(|c| c.id.as_str()).collect::<Vec<_>>().join(","),
            tabs.join(","),
            s.list.join(","),
            actions.join(","),
            backdrop,
            self.pos.borrow().as_ref().is_some_and(|t| t.moving()) || self.backdrop.fading(),
        )
    }

    /// The pieces' bounds on the screen, in points: `wadsctl home layout`.
    fn layout(&self) -> String {
        let at = |w: &gtk::Widget| -> String {
            match w.compute_bounds(&self.stage) {
                Some(r) => format!("{:.0},{:.0},{:.0},{:.0}", r.x(), r.y(), r.width(), r.height()),
                None => "none".into(),
            }
        };
        let st = self.st.borrow();
        let cards: Vec<String> = self.cards.borrow().iter().map(|(_, c)| at(c.root.upcast_ref())).collect();
        let tabs: Vec<String> = self.bar.tab_widgets().iter().map(|t| at(t.upcast_ref())).collect();
        let acts: Vec<String> = self.buttons.widgets().iter().map(|b| at(b.upcast_ref())).collect();
        let focused = self.cards.borrow().get(st.shown.at).map(|(_, c)| at(c.root.upcast_ref()));
        format!(
            "ok card={} machine={} bar={} tabs={} acts={} buttons={} cards={}",
            focused.unwrap_or_else(|| "none".into()),
            at(self.bar.machine.upcast_ref()),
            at(self.bar.root.upcast_ref()),
            tabs.join(";"),
            at(self.buttons.root.upcast_ref()),
            acts.join(";"),
            cards.join(";"),
        )
    }
}

/// One of the machine's wadspaces, for the tabs (`here`: this machine's;
/// another's runs aren't known here).
fn space_of(store: &Store, x: &Wadspace, tags: &[wad_model::Tag], here: bool) -> Space {
    let running = here && store.is_running(&x.id);
    // A run's start is a launch too (wadd doesn't set lastLaunched yet).
    let since = store.run(&x.id).filter(|_| running).map(|r| r.state.since).unwrap_or(0);
    Space {
        id: x.id.clone(),
        category: x.category_or_default(),
        tag: wad_model::tags::tag_of(tags, x).to_string(),
        launched: x.last_launched.unwrap_or(0).max(since),
        running,
    }
}

fn card_data(store: &Store, x: &Wadspace, machine: Option<(&'static str, String)>) -> CardData {
    if machine.is_some() {
        // Another machine's: what it's made of, and a ring while it copies here.
        let copying = store_copying(store, &x.id);
        return CardData {
            name: x.name.clone(),
            picture: wad_model::look::icon_picture_of(x).clone(),
            icon: wad_model::look::icon_of(x),
            apps: apps_of(x),
            running: false,
            built: None,
            failed: false,
            busy: copying,
            machine,
            stream: store.stream(&x.id).as_ref().map(StreamBadge::of),
        };
    }
    let state = store.tile_state(&x.id);
    let stream = store.stream(&x.id);
    // Its Stream job, while it starts: the ring too.
    let streaming = stream.as_ref().and_then(|s| s.job.as_deref()).and_then(|j| store.job(j)).and_then(|j| {
        matches!(j.job.status, wad_model::JobStatus::Queued | wad_model::JobStatus::Running).then_some(j.job.progress)
    });
    CardData {
        name: x.name.clone(),
        picture: wad_model::look::icon_picture_of(x).clone(),
        icon: wad_model::look::icon_of(x),
        apps: apps_of(x),
        running: state.running,
        built: store.build(&x.id).map(|b| b.state.built),
        failed: state.failed,
        busy: state.busy.or(streaming),
        machine: None,
        stream: stream.as_ref().map(StreamBadge::of),
    }
}

/// How far a copy of another machine's `ws` here has got, while one runs.
fn store_copying(store: &Store, ws: &str) -> Option<f64> {
    store.copying(ws)
}

/// A wadspace's Stream ▸ rows (streams.rs's menu): each mode, the one it
/// streams in ticked (its link), the others off saying why.
fn stream_rows(store: &Store, key: &str) -> Vec<Item> {
    streams::menu(store, key)
        .into_iter()
        .map(|i| {
            let (k, pick) = (key.to_string(), i.pick);
            let row = Row::new(i.icon, i.label, move || streams::pick(&k, pick)).checked_if(i.checked);
            Item::Row(match i.disabled {
                Some(why) => row.later(why),
                None => row.hint(i.hint),
            })
        })
        .collect()
}

/// What a tab with nothing in it says.
struct Empty {
    root: gtk::Box,
    title: gtk::Label,
    body: gtk::Label,
    recent: gtk::Box,
}

impl Empty {
    fn new() -> Self {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .halign(gtk::Align::Center)
            .visible(false)
            .build();
        let title = gtk::Label::builder().css_classes(["home-empty-title"]).build();
        let body = gtk::Label::builder().css_classes(["home-empty-body"]).build();
        let recent = gtk::Box::builder().spacing(8).halign(gtk::Align::Center).css_classes(["home-pill"]).build();
        recent.append(&wad_gtk::Icon::new("history", 16));
        recent.append(&gtk::Label::new(Some("Recent")));
        root.append(&title);
        root.append(&body);
        root.append(&recent);
        Self { root, title, body, recent }
    }

    /// `on`: another machine's name (none: this one's); `waiting`: its
    /// wadspaces haven't come (what it says meanwhile).
    fn set(
        &self,
        view: &Tab,
        tags: &[wad_model::Tag],
        empty: bool,
        on: Option<&str>,
        waiting: Option<(String, String)>,
    ) {
        self.root.set_visible(empty);
        if !empty {
            return;
        }
        let (title, body, recent) = match waiting {
            Some((t, b)) => (t, b, false),
            None => {
                let (t, b) = model::empty_text(view, tags, on);
                (t, b.to_string(), *view == Tab::Running)
            }
        };
        self.title.set_label(&title);
        self.body.set_label(&body);
        self.recent.set_visible(recent);
    }
}
