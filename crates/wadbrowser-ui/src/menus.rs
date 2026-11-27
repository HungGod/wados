//! The browser's menus as data (the spike's `menus.ts`): right-clicking a
//! page, and the main ⋮ menu. Neither changes a setting: those are
//! wadbrowser.conf's (the Builder's add-ons, Settings ▸ Browser).
//! The chrome passes in what they show and what they do;
//! [`crate::menu_popover::MenuPopover`] draws them.

use std::rc::Rc;

use crate::data::{WebKey, bare_host, is_own_page};

pub type Run = Rc<dyn Fn()>;

/// A row of a menu.
#[derive(Clone)]
pub enum Entry {
    Item(Item),
    Sep,
    /// A small caps heading.
    Heading(String),
}

/// A menu item: what it says, and what it does.
#[derive(Clone)]
pub struct Item {
    /// A test hook (the spike's `data-menu`): `page-back`, `menu-mode-focus`.
    pub id: String,
    pub label: String,
    /// A lucide icon ("" for none); a check stands in for it when it's on.
    pub icon: &'static str,
    /// A small note under the label.
    pub hint: Option<String>,
    pub shortcut: Option<String>,
    /// Off: why, shown under it (and as its tooltip); `Some("")` says nothing.
    pub disabled: Option<String>,
    pub danger: bool,
    pub checked: bool,
    /// Opens beside it (hover, a click, or →).
    pub submenu: Vec<Entry>,
    pub run: Option<Run>,
}

impl Item {
    pub fn new(id: impl Into<String>, label: impl Into<String>, icon: &'static str, run: Run) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon,
            hint: None,
            shortcut: None,
            disabled: None,
            danger: false,
            checked: false,
            submenu: vec![],
            run: Some(run),
        }
    }

    pub fn hint(mut self, h: impl Into<String>) -> Self {
        self.hint = Some(h.into());
        self
    }

    pub fn key(mut self, k: impl Into<String>) -> Self {
        self.shortcut = Some(k.into());
        self
    }

    /// Off, saying `why` ("" for no reason), when `off`.
    pub fn off_if(mut self, off: bool, why: &str) -> Self {
        if off {
            self.disabled = Some(why.to_string());
        }
        self
    }

    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn submenu(mut self, entries: Vec<Entry>) -> Self {
        self.submenu = entries;
        self.run = None;
        self
    }

    pub fn enabled(&self) -> bool {
        self.disabled.is_none()
    }

    pub fn entry(self) -> Entry {
        Entry::Item(self)
    }
}

/// Item `id`, at any depth.
pub fn find<'a>(entries: &'a [Entry], id: &str) -> Option<&'a Item> {
    entries.iter().find_map(|e| match e {
        Entry::Item(it) if it.id == id => Some(it),
        Entry::Item(it) => find(&it.submenu, id),
        _ => None,
    })
}

fn noop() -> Run {
    Rc::new(|| {})
}

// ------------------------------------------------------------ right-clicking a page

/// What the page menu shows.
#[derive(Debug, Clone, Default)]
pub struct PageMenu {
    pub url: String,
    pub can_back: bool,
    pub can_forward: bool,
    /// Saved sign-ins for the web.
    pub web_keys: Vec<WebKey>,
    /// The developer tools are there (WebKit's inspector).
    pub can_inspect: bool,
    /// Why "Create app from this page" is off on a web page too (in a
    /// wadspace: "Add web apps in the Builder").
    pub create_app_off: Option<String>,
    /// Why saved sign-ins can't change here (in a wadspace they're the
    /// machine's): its keys are shown, each off with this.
    pub keys_off: Option<String>,
}

/// What the page menu does.
#[derive(Clone)]
pub struct PageActions {
    pub back: Run,
    pub forward: Run,
    pub reload: Run,
    pub create_app: Run,
    /// Use this key here (this page's site).
    pub use_key: Rc<dyn Fn(&WebKey)>,
    pub add_key: Run,
    pub edit_keys: Run,
    pub inspect: Run,
}

impl Default for PageActions {
    fn default() -> Self {
        Self {
            back: noop(),
            forward: noop(),
            reload: noop(),
            create_app: noop(),
            use_key: Rc::new(|_| {}),
            add_key: noop(),
            edit_keys: noop(),
            inspect: noop(),
        }
    }
}

/// Right-click on a page.
pub fn page_menu(m: &PageMenu, a: &PageActions) -> Vec<Entry> {
    let host = bare_host(&m.url);
    let not_web = is_own_page(&m.url);
    let keys: Vec<Entry> = m
        .web_keys
        .iter()
        .map(|k| {
            let key = k.clone();
            let use_key = a.use_key.clone();
            Item::new(
                format!("page-use-{}", k.id),
                format!("Use {} here", k.name),
                "key-round",
                Rc::new(move || use_key(&key)),
            )
            .checked(k.sites.contains(&host))
            .hint(if k.sites.is_empty() { "No sites yet".to_string() } else { k.sites.join(", ") })
            .off_if(m.keys_off.is_some(), m.keys_off.as_deref().unwrap_or_default())
            .entry()
        })
        .collect();
    let mut signins = vec![];
    if !keys.is_empty() {
        signins.push(Entry::Heading(format!("Sign in to {host} with")));
        signins.extend(keys);
        signins.push(Entry::Sep);
    }
    let keys_off = m.keys_off.as_deref();
    signins.push(
        Item::new("page-add-signin", format!("Add a new sign-in for {host}…"), "plus", a.add_key.clone())
            .off_if(keys_off.is_some(), keys_off.unwrap_or_default())
            .entry(),
    );
    signins.push(
        Item::new("page-edit-signins", "Edit saved sign-ins…", "pencil", a.edit_keys.clone())
            .off_if(m.web_keys.is_empty(), "There aren’t any yet")
            .off_if(keys_off.is_some() && !m.web_keys.is_empty(), keys_off.unwrap_or_default())
            .entry(),
    );
    vec![
        Item::new("page-back", "Back", "arrow-left", a.back.clone()).off_if(!m.can_back, "").entry(),
        Item::new("page-forward", "Forward", "arrow-right", a.forward.clone()).off_if(!m.can_forward, "").entry(),
        Item::new("page-reload", "Reload", "refresh-cw", a.reload.clone()).entry(),
        Entry::Sep,
        Item::new("page-create-app", "Create app from this page…", "app-window", a.create_app.clone())
            .off_if(m.create_app_off.is_some(), m.create_app_off.as_deref().unwrap_or_default())
            .off_if(not_web, "Only on web pages")
            .entry(),
        Item::new("page-signins", "Saved sign-ins", "key-round", noop())
            .submenu(signins)
            .off_if(not_web, "Only on web pages")
            .entry(),
        Entry::Sep,
        Item::new("page-inspect", "Inspect", "code", a.inspect.clone())
            .off_if(!m.can_inspect, "The developer tools are off")
            .entry(),
    ]
}

// ------------------------------------------------------------ the ⋮ menu

/// What the main menu does.
#[derive(Clone)]
pub struct MainActions {
    pub new_tab: Run,
    pub new_window: Run,
    pub new_private_window: Run,
    /// Opens one of the browser's pages (`wadbrowser://history`).
    pub open: Rc<dyn Fn(&str)>,
}

impl Default for MainActions {
    fn default() -> Self {
        Self { new_tab: noop(), new_window: noop(), new_private_window: noop(), open: Rc::new(|_| {}) }
    }
}

/// The ⋮ menu in the header bar.
pub fn main_menu(a: &MainActions) -> Vec<Entry> {
    let open = |url: &'static str| {
        let open = a.open.clone();
        Rc::new(move || open(url)) as Run
    };
    vec![
        Item::new("menu-new-tab", "New tab", "plus", a.new_tab.clone()).key("Ctrl+T").entry(),
        Item::new("menu-new-window", "New window", "app-window", a.new_window.clone()).key("Ctrl+N").entry(),
        Item::new("menu-new-private", "New private window", "eye-off", a.new_private_window.clone())
            .key("Ctrl+Shift+N")
            .entry(),
        Entry::Sep,
        Item::new("menu-history", "History", "clock", open("wadbrowser://history")).entry(),
        Item::new("menu-privacy", "Privacy", "shield-check", open("wadbrowser://privacy")).entry(),
    ]
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn keys() -> Vec<WebKey> {
        vec![
            WebKey {
                id: "work-web".into(),
                name: "Work".into(),
                sites: vec!["claude.ai".into(), "mail.google.com".into()],
                used_by: 1,
            },
            WebKey { id: "hunggod".into(), name: "hunggod".into(), sites: vec!["github.com".into()], used_by: 1 },
            WebKey { id: "empty".into(), name: "Spare".into(), sites: vec![], used_by: 0 },
        ]
    }

    fn page(url: &str, keys: Vec<WebKey>) -> Vec<Entry> {
        page_menu(
            &PageMenu { url: url.into(), can_forward: true, web_keys: keys, ..Default::default() },
            &PageActions::default(),
        )
    }

    fn ids(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match e {
                Entry::Item(it) => it.id.clone(),
                Entry::Sep => "|".into(),
                Entry::Heading(h) => format!("[{h}]"),
            })
            .collect()
    }

    #[test]
    fn a_pages_menu_offers_each_saved_sign_in_ticking_the_ones_this_site_uses() {
        let items = page("https://www.github.com/HungGod/Writing", keys());
        assert_eq!(
            ids(&items),
            ["page-back", "page-forward", "page-reload", "|", "page-create-app", "page-signins", "|", "page-inspect"]
        );
        assert_eq!(find(&items, "page-back").unwrap().disabled.as_deref(), Some(""));
        assert!(find(&items, "page-forward").unwrap().enabled());
        assert!(find(&items, "page-create-app").unwrap().enabled());
        let signins = &find(&items, "page-signins").unwrap().submenu;
        assert_eq!(
            ids(signins),
            [
                "[Sign in to github.com with]",
                "page-use-work-web",
                "page-use-hunggod",
                "page-use-empty",
                "|",
                "page-add-signin",
                "page-edit-signins"
            ]
        );
        for k in keys() {
            let row = find(signins, &format!("page-use-{}", k.id)).unwrap();
            assert_eq!(row.label, format!("Use {} here", k.name));
            assert_eq!(row.checked, k.sites.iter().any(|s| s == "github.com"));
        }
        assert_eq!(find(signins, "page-use-work-web").unwrap().hint.as_deref(), Some("claude.ai, mail.google.com"));
        assert_eq!(find(signins, "page-use-empty").unwrap().hint.as_deref(), Some("No sites yet"));
        assert_eq!(find(signins, "page-add-signin").unwrap().label, "Add a new sign-in for github.com…");
        assert!(find(signins, "page-edit-signins").unwrap().enabled());
        assert_eq!(find(&items, "page-inspect").unwrap().disabled.as_deref(), Some("The developer tools are off"));
    }

    #[test]
    fn with_no_keys_there_is_nothing_to_edit() {
        let items = page("https://docs.rs/gtk4", vec![]);
        let signins = &find(&items, "page-signins").unwrap().submenu;
        assert_eq!(ids(signins), ["page-add-signin", "page-edit-signins"]);
        assert_eq!(find(signins, "page-edit-signins").unwrap().disabled.as_deref(), Some("There aren’t any yet"));
    }

    #[test]
    fn cactuss_own_pages_cant_become_apps_or_keep_sign_ins() {
        let items = page("wadbrowser://privacy", keys());
        assert_eq!(find(&items, "page-create-app").unwrap().disabled.as_deref(), Some("Only on web pages"));
        assert_eq!(find(&items, "page-signins").unwrap().disabled.as_deref(), Some("Only on web pages"));
        assert!(find(&items, "page-reload").unwrap().enabled());
    }

    #[test]
    fn in_a_wadspace_apps_and_keys_are_the_machines() {
        let m = PageMenu {
            url: "https://github.com/x".into(),
            web_keys: keys(),
            create_app_off: Some("Add web apps in the Builder".into()),
            keys_off: Some("Sign-ins change on the machine".into()),
            ..Default::default()
        };
        let items = page_menu(&m, &PageActions::default());
        assert_eq!(find(&items, "page-create-app").unwrap().disabled.as_deref(), Some("Add web apps in the Builder"));
        let signins = &find(&items, "page-signins").unwrap().submenu;
        let hunggod = find(signins, "page-use-hunggod").unwrap();
        assert!(hunggod.checked, "still ticked: this site uses it");
        assert_eq!(hunggod.disabled.as_deref(), Some("Sign-ins change on the machine"));
        for id in ["page-add-signin", "page-edit-signins"] {
            assert_eq!(find(signins, id).unwrap().disabled.as_deref(), Some("Sign-ins change on the machine"), "{id}");
        }
        // A page of the browser's own says that first.
        let own = page_menu(&PageMenu { url: "wadbrowser://newtab".into(), ..m }, &PageActions::default());
        assert_eq!(find(&own, "page-create-app").unwrap().disabled.as_deref(), Some("Only on web pages"));
    }

    #[test]
    fn using_a_key_passes_it_on() {
        let used = Rc::new(RefCell::new(None));
        let u = used.clone();
        let a = PageActions {
            use_key: Rc::new(move |k: &WebKey| *u.borrow_mut() = Some(k.id.clone())),
            ..Default::default()
        };
        let items =
            page_menu(&PageMenu { url: "https://claude.ai".into(), web_keys: keys(), ..Default::default() }, &a);
        let row = find(&items, "page-use-work-web").unwrap();
        assert!(row.checked);
        (row.run.as_ref().unwrap())();
        assert_eq!(used.borrow().as_deref(), Some("work-web"));
    }

    #[test]
    fn the_main_menu_opens_windows_and_the_browsers_pages_but_no_settings() {
        let opened = Rc::new(RefCell::new(vec![]));
        let o = opened.clone();
        let a = MainActions { open: Rc::new(move |u: &str| o.borrow_mut().push(u.to_string())), ..Default::default() };
        let items = main_menu(&a);
        assert_eq!(
            ids(&items),
            ["menu-new-tab", "menu-new-window", "menu-new-private", "|", "menu-history", "menu-privacy"]
        );
        for id in ["menu-history", "menu-privacy"] {
            (find(&items, id).unwrap().run.as_ref().unwrap())();
        }
        assert_eq!(*opened.borrow(), ["wadbrowser://history", "wadbrowser://privacy"]);
    }
}
