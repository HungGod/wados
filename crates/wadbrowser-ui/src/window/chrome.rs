//! The window's chrome as the v2 design draws it (the spike's
//! `Browser.svelte`): the tab bar above the header bar, each tab its
//! favicon, title and ×; the header bar's back, forward and reload, its
//! middle (Full's address, Focus's page title, an app's offsite pill), the
//! shield, the profile chip and ⋮, and the loading line under it; under
//! that, a GitHub sign-in window's code.
//!
//! Only drawing: what the buttons do is `window.rs`'s and `wire.rs`'s.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use adw::{TabPage, TabView};
use gtk::glib;
use wadbrowser_core::site;
use webkit6::WebView;
use webkit6::prelude::*;

use crate::chip::{ChipInfo, ProfileChip};
use crate::shield::{Shield, ShieldActions};

/// A header bar button: a lucide icon (`.wb-hb`).
pub(super) fn icon_button(icon: &'static str, size: i32, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::builder()
        .child(&wad_gtk::Icon::new(icon, size))
        .css_classes(["wb-hb"])
        .valign(gtk::Align::Center)
        .tooltip_text(tooltip)
        .build();
    b.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    b
}

// ------------------------------------------------------------ the tab bar

/// One tab's widgets, and what it listens to (taken off when it goes).
struct Tab {
    page: TabPage,
    root: gtk::Box,
    handlers: Vec<(glib::Object, glib::SignalHandlerId)>,
}

/// The tab bar (`.tabbar`): the tabs, then +, then room to drag the window
/// by (a double click there opens a new tab), then the window's buttons.
pub(super) struct TabStrip {
    pub root: gtk::WindowHandle,
    /// The view's pages (kept: the view only keeps it while someone does).
    pages: gtk::SelectionModel,
    list: gtk::Box,
    tabs: RefCell<Vec<Tab>>,
    /// The window's buttons, when the tab bar's the top of the window.
    pub controls: gtk::WindowControls,
}

impl TabStrip {
    pub fn new(tab_view: &TabView, new_tab: Rc<dyn Fn()>) -> Rc<TabStrip> {
        let row = gtk::Box::builder().css_classes(["wb-tabbar"]).spacing(4).build();
        let list = gtk::Box::builder().spacing(4).build();
        row.append(&list);
        let plus = icon_button("plus", 16, "New tab");
        plus.add_css_class("wb-new-tab");
        row.append(&plus);
        // The room after the tabs: drags the window; a double click, a new tab.
        let room = gtk::Box::builder().hexpand(true).build();
        let twice = gtk::GestureClick::builder().button(1).build();
        twice.set_propagation_phase(gtk::PropagationPhase::Capture);
        let nt = new_tab.clone();
        twice.connect_pressed(move |g, n, _, _| {
            if n == 2 {
                g.set_state(gtk::EventSequenceState::Claimed);
                nt();
            }
        });
        room.add_controller(twice);
        row.append(&room);
        let controls = gtk::WindowControls::builder().side(gtk::PackType::End).valign(gtk::Align::Center).build();
        row.append(&controls);
        plus.connect_clicked(move |_| new_tab());
        let root = gtk::WindowHandle::builder().child(&row).build();

        let pages = tab_view.pages();
        let strip = Rc::new(TabStrip { root, pages: pages.clone(), list, tabs: RefCell::default(), controls });
        let (s, tv) = (Rc::downgrade(&strip), tab_view.downgrade());
        pages.connect_items_changed(move |_, at, removed, added| {
            if let (Some(s), Some(tv)) = (s.upgrade(), tv.upgrade()) {
                s.changed(&tv, at, removed, added);
            }
        });
        let s = Rc::downgrade(&strip);
        tab_view.connect_selected_page_notify(move |tv| {
            if let Some(s) = s.upgrade() {
                s.mark_selected(tv);
            }
        });
        strip
    }

    /// `removed` tabs gone at `at`, `added` new ones there (the view's pages model).
    fn changed(self: &Rc<Self>, tv: &TabView, at: u32, removed: u32, added: u32) {
        let at = at as usize;
        let gone: Vec<Tab> = {
            let mut tabs = self.tabs.borrow_mut();
            let end = (at + removed as usize).min(tabs.len());
            tabs.drain(at.min(end)..end).collect()
        };
        for t in gone {
            self.list.remove(&t.root);
            for (obj, id) in t.handlers {
                obj.disconnect(id);
            }
        }
        for i in 0..added {
            let Some(page) = self.pages.item(at as u32 + i).and_downcast::<TabPage>() else { continue };
            let tab = self.tab(tv, &page);
            let before = if at + i as usize == 0 {
                None
            } else {
                self.tabs.borrow().get(at + i as usize - 1).map(|t| t.root.clone())
            };
            self.list.insert_child_after(&tab.root, before.as_ref());
            self.tabs.borrow_mut().insert(at + i as usize, tab);
        }
        self.mark_selected(tv);
    }

    fn tab(&self, tv: &TabView, page: &TabPage) -> Tab {
        let root = gtk::Box::builder().css_classes(["wb-tab"]).spacing(8).hexpand(false).build();
        root.set_size_request(64, -1);
        let fav = gtk::Box::builder().css_classes(["wb-tab-fav"]).valign(gtk::Align::Center).build();
        fav.set_size_request(16, 16);
        let label = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .width_chars(22)
            .max_width_chars(22)
            .css_classes(["wb-tab-title"])
            .build();
        let x = gtk::Button::builder()
            .child(&wad_gtk::Icon::new("x", 13))
            .css_classes(["wb-tab-x"])
            .valign(gtk::Align::Center)
            .tooltip_text("Close tab")
            .build();
        x.update_property(&[gtk::accessible::Property::Label("Close tab")]);
        root.append(&fav);
        root.append(&label);
        root.append(&x);

        // A click shows it; a middle click closes it.
        let click = gtk::GestureClick::builder().button(0).build();
        let (p, t) = (page.downgrade(), tv.downgrade());
        click.connect_pressed(move |g, _, _, _| {
            if g.current_button() == 1
                && let (Some(p), Some(t)) = (p.upgrade(), t.upgrade())
            {
                t.set_selected_page(&p);
            }
        });
        let (p, t) = (page.downgrade(), tv.downgrade());
        click.connect_released(move |g, _, _, _| {
            if g.current_button() == 2
                && let (Some(p), Some(t)) = (p.upgrade(), t.upgrade())
            {
                t.close_page(&p);
            }
        });
        root.add_controller(click);
        let (p, t) = (page.downgrade(), tv.downgrade());
        x.connect_clicked(move |_| {
            if let (Some(p), Some(t)) = (p.upgrade(), t.upgrade()) {
                t.close_page(&p);
            }
        });

        let mut handlers = vec![];
        let l = label.downgrade();
        let id = page.connect_title_notify(move |p| {
            if let Some(l) = l.upgrade() {
                set_title(&l, p);
            }
        });
        handlers.push((page.clone().upcast::<glib::Object>(), id));
        set_title(&label, page);
        if let Ok(view) = page.child().downcast::<WebView>() {
            for prop in ["uri", "favicon", "title"] {
                let f = fav.downgrade();
                let id = view.connect_notify_local(Some(prop), move |v, _| {
                    if let Some(f) = f.upgrade() {
                        set_favicon(&f, v);
                    }
                });
                handlers.push((view.clone().upcast::<glib::Object>(), id));
            }
            set_favicon(&fav, &view);
        }
        Tab { page: page.clone(), root, handlers }
    }

    fn mark_selected(&self, tv: &TabView) {
        let selected = tv.selected_page();
        for t in self.tabs.borrow().iter() {
            if Some(&t.page) == selected.as_ref() {
                t.root.add_css_class("active");
            } else {
                t.root.remove_css_class("active");
            }
        }
    }

    /// Shown, unless it's an app's window with one tab.
    pub fn sync(&self, tv: &TabView, autohide: bool) {
        self.root.set_visible(!(autohide && tv.n_pages() <= 1));
    }
}

fn set_title(label: &gtk::Label, page: &TabPage) {
    let t = page.title();
    label.set_label(&t);
    if let Some(tab) = label.parent() {
        tab.set_tooltip_text(Some(&t));
    }
}

/// A tab's icon: the browser's own pages', the site's favicon (or the
/// catalog's icon for it), else its initial (`ui/AppIcon`).
fn set_favicon(slot: &gtk::Box, view: &WebView) {
    while let Some(c) = slot.first_child() {
        slot.remove(&c);
    }
    slot.append(&favicon(view, 16));
}

/// A page's icon, `size` px: the browser's own pages', its favicon, else
/// its site's (the catalog's, or its initial).
pub(super) fn favicon(view: &WebView, size: i32) -> gtk::Widget {
    let uri = view.uri().map(String::from).unwrap_or_default();
    if uri.is_empty() || uri.starts_with("about:") || site::is_cactus(&uri) {
        let page = uri.strip_prefix("cactus://").unwrap_or("").split(['/', '?', '#']).next().unwrap_or("");
        let icon = match page {
            "privacy" | "shield" => "shield-check",
            "settings" => "settings",
            "history" => "clock",
            _ => "compass",
        };
        let i = wad_gtk::Icon::new(icon, size - 2);
        i.add_css_class("wb-fav-ic");
        i.set_size_request(size, size);
        return i.upcast();
    }
    if let Some(t) = view.favicon() {
        let img = gtk::Image::from_paintable(Some(&t));
        img.set_pixel_size(size);
        return img.upcast();
    }
    let host = site::bare_host(&site::host_of(&uri));
    let name = view.title().map(String::from).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| host.clone());
    wad_gtk::app_icon::site(&site::site_of(&host), Some(&name), Some(&uri), size)
}

// ------------------------------------------------------------ the address

/// Full's address field (`.entry`): the address shown (its icon, the host,
/// the rest of it faint; the placeholder on a new tab) until it's clicked,
/// then an entry with it selected.
pub(super) struct Address {
    pub root: adw::Clamp,
    stack: gtk::Stack,
    shown: gtk::Button,
    icon: wad_gtk::Icon,
    host: gtk::Label,
    rest: gtk::Label,
    placeholder: gtk::Label,
    pub entry: gtk::Entry,
}

impl Address {
    pub fn new() -> Address {
        let icon = wad_gtk::Icon::new("compass", 13);
        let host = gtk::Label::builder().css_classes(["wb-addr-host"]).build();
        let rest = gtk::Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["wb-addr-rest"])
            .xalign(0.0)
            .build();
        let placeholder = gtk::Label::builder().css_classes(["wb-addr-placeholder"]).build();
        let inner = gtk::Box::builder().spacing(7).halign(gtk::Align::Center).build();
        inner.append(&icon);
        inner.append(&placeholder);
        let addr = gtk::Box::builder().build();
        addr.append(&host);
        addr.append(&rest);
        inner.append(&addr);
        let shown = gtk::Button::builder().child(&inner).css_classes(["wb-addr-shown"]).hexpand(true).build();
        shown.update_property(&[gtk::accessible::Property::Label("Address")]);
        let entry = gtk::Entry::builder()
            .css_classes(["wb-addr-entry"])
            .hexpand(true)
            .input_purpose(gtk::InputPurpose::Url)
            .build();
        entry.update_property(&[gtk::accessible::Property::Label("Address")]);
        let stack = gtk::Stack::builder().css_classes(["wb-addr"]).hhomogeneous(true).vhomogeneous(true).build();
        stack.add_named(&shown, Some("shown"));
        stack.add_named(&entry, Some("edit"));
        let root = adw::Clamp::builder()
            .maximum_size(720)
            .tightening_threshold(720)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .child(&stack)
            .build();
        Address { root, stack, shown, icon, host, rest, placeholder, entry }
    }

    /// Shows `uri` (a new tab: `placeholder`).
    pub fn show(&self, uri: &str, newtab: bool, placeholder: &str) {
        let scheme = uri.split_once("://").map(|(s, _)| s.to_ascii_lowercase()).unwrap_or_default();
        self.icon.set(match scheme.as_str() {
            "cactus" | "" => "compass",
            "https" => "lock",
            _ => "globe",
        });
        self.placeholder.set_visible(newtab);
        self.placeholder.set_label(placeholder);
        self.host.set_visible(!newtab);
        self.rest.set_visible(!newtab);
        if newtab {
            return;
        }
        let (host, rest) = match uri.split_once("://") {
            Some((_, after)) => {
                let i = after.find(['/', '?', '#']).unwrap_or(after.len());
                let host = after[..i].to_ascii_lowercase();
                let host = host.rsplit_once('@').map_or(host.clone(), |(_, h)| h.to_string());
                let rest = &after[i..];
                let host = if scheme == "cactus" { format!("cactus://{host}") } else { host };
                (host, if rest == "/" { String::new() } else { rest.to_string() })
            }
            None => (uri.to_string(), String::new()),
        };
        self.host.set_label(&host);
        self.rest.set_label(&rest);
        self.shown.set_tooltip_text(Some(uri));
    }

    /// The entry, with `text` in it, selected.
    pub fn edit(&self, text: &str) {
        self.entry.set_text(text);
        self.stack.set_visible_child_name("edit");
        self.stack.add_css_class("editing");
        self.entry.grab_focus();
        self.entry.select_region(0, -1);
    }

    /// Back to the address shown.
    pub fn done(&self) {
        self.stack.set_visible_child_name("shown");
        self.stack.remove_css_class("editing");
    }

    pub fn editing(&self) -> bool {
        self.stack.visible_child_name().as_deref() == Some("edit")
    }

    pub fn connect_clicked(&self, f: impl Fn() + 'static) {
        self.shown.connect_clicked(move |_| f());
    }
}

// ------------------------------------------------------------ a sign-in's code

/// A GitHub sign-in window's code, in a bar under the header bar
/// (`.wb-code`): "Your code: WADS-0601", and Copy. GitHub's device page is
/// behind its sign-in and its form isn't documented, so the code is typed
/// in for you where the page lets it be (signin::device_code_script) and
/// shown here either way, to type in by hand. Hidden in every other window.
pub(super) struct CodeBar {
    pub root: gtk::Box,
    code: gtk::Label,
    pub copy: gtk::Button,
}

impl CodeBar {
    pub fn new() -> CodeBar {
        let row = gtk::Box::builder().spacing(10).halign(gtk::Align::Center).build();
        row.append(&gtk::Label::builder().label("Your code:").css_classes(["wb-code-label"]).build());
        let code = gtk::Label::builder().css_classes(["wb-code-value"]).selectable(true).build();
        code.update_property(&[gtk::accessible::Property::Label("Your code")]);
        row.append(&code);
        let inner = gtk::Box::builder().spacing(6).build();
        inner.append(&wad_gtk::Icon::new("copy", 14));
        inner.append(&gtk::Label::new(Some("Copy")));
        let copy = gtk::Button::builder()
            .child(&inner)
            .css_classes(["wb-code-copy"])
            .valign(gtk::Align::Center)
            .tooltip_text("Copy the code")
            .build();
        copy.update_property(&[gtk::accessible::Property::Label("Copy the code")]);
        row.append(&copy);
        let hint = gtk::Label::builder()
            .label("Type it on GitHub's page if it isn't filled in")
            .css_classes(["wb-code-hint"])
            .build();
        row.append(&hint);
        let root = gtk::Box::builder().css_classes(["wb-code"]).visible(false).build();
        row.set_hexpand(true);
        root.append(&row);
        CodeBar { root, code, copy }
    }

    /// Shows `code` (none: hidden).
    pub fn show(&self, code: Option<&str>) {
        self.code.set_label(code.unwrap_or_default());
        self.root.set_visible(code.is_some_and(|c| !c.is_empty()));
    }

    /// The code shown ("" when it's hidden).
    pub fn code(&self) -> String {
        if self.root.is_visible() { self.code.label().to_string() } else { String::new() }
    }
}

// ------------------------------------------------------------ the header bar

/// The header bar (`.headerbar`) and what's on it.
pub(super) struct Header {
    pub root: gtk::WindowHandle,
    pub back: gtk::Button,
    pub forward: gtk::Button,
    pub reload: gtk::Button,
    pub address: Address,
    pub shield: Rc<Shield>,
    pub chip: ProfileChip,
    pub menu: gtk::Button,
    pub loading: gtk::ProgressBar,
    /// The window's buttons, when the tab bar's hidden.
    pub controls: gtk::WindowControls,
    /// A GitHub sign-in's code, under it.
    pub code: CodeBar,
}

impl Header {
    /// `identity`, `title` and `offsite` are the window kind's (ModeChrome's).
    pub fn new(
        identity: &gtk::Box,
        title: &gtk::Label,
        offsite: &gtk::Box,
        shield: ShieldActions,
        chip: ChipInfo,
        chip_clicked: impl Fn(&gtk::Button) + 'static,
    ) -> Header {
        let row = gtk::Box::builder().css_classes(["wb-header"]).spacing(8).build();
        row.append(identity);
        let nav = gtk::Box::builder().spacing(2).css_classes(["wb-nav"]).build();
        let back = icon_button("arrow-left", 16, "Back");
        let forward = icon_button("arrow-right", 16, "Forward");
        let reload = icon_button("refresh-cw", 15, "Reload");
        nav.append(&back);
        nav.append(&forward);
        nav.append(&reload);
        row.append(&nav);

        let middle = gtk::Box::builder().hexpand(true).spacing(10).halign(gtk::Align::Fill).build();
        let address = Address::new();
        middle.append(&address.root);
        title.set_hexpand(true);
        middle.append(title);
        offsite.set_hexpand(true);
        middle.append(offsite);
        row.append(&middle);

        let end = gtk::Box::builder().spacing(6).css_classes(["wb-end"]).build();
        let shield = Shield::new(shield);
        end.append(shield.button());
        let chip = ProfileChip::new(&chip, chip_clicked);
        end.append(chip.button());
        let menu = icon_button("ellipsis-vertical", 16, "Main menu");
        end.append(&menu);
        row.append(&end);
        let controls = gtk::WindowControls::builder().side(gtk::PackType::End).valign(gtk::Align::Center).build();
        row.append(&controls);

        let loading = gtk::ProgressBar::builder()
            .css_classes(["wb-loading"])
            .valign(gtk::Align::End)
            .can_target(false)
            .visible(false)
            .build();
        let overlay = gtk::Overlay::builder().child(&row).build();
        overlay.add_overlay(&loading);
        let code = CodeBar::new();
        let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        column.append(&overlay);
        column.append(&code.root);
        let root = gtk::WindowHandle::builder().child(&column).build();
        Header { root, back, forward, reload, address, shield, chip, menu, loading, controls, code }
    }
}
