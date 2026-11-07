//! The browser's menus as data (the spike's `menus.ts`): right-clicking a
//! page, the main ⋮ menu (with its Mode submenu) and the profile chip's.
//! The chrome passes in what they show and what they do;
//! [`crate::menu_popover::MenuPopover`] draws them.

use std::rc::Rc;

use crate::data::{MODES, Mode, Profile, WebKey, bare_host, is_own_page, mode_id};

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
    pub new_profile: Run,
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
            new_profile: noop(),
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
        Item::new("page-new-profile", "New profile…", "user-round-plus", a.new_profile.clone()).entry(),
        Entry::Sep,
        Item::new("page-inspect", "Inspect", "code", a.inspect.clone())
            .off_if(!m.can_inspect, "The developer tools are off")
            .entry(),
    ]
}

// ------------------------------------------------------------ the ⋮ menu

/// What the main menu shows.
#[derive(Debug, Clone, Default)]
pub struct MainMenu {
    pub mode: Mode,
    pub block_social: bool,
    /// Why the mode can't change (a sign-in window stays one).
    pub mode_off: Option<String>,
}

/// What the main menu does.
#[derive(Clone)]
pub struct MainActions {
    pub new_tab: Run,
    pub new_window: Run,
    pub new_private_window: Run,
    pub new_profile: Run,
    pub set_mode: Rc<dyn Fn(Mode)>,
    pub set_block_social: Rc<dyn Fn(bool)>,
    /// Opens one of the browser's pages (`wadbrowser://history`).
    pub open: Rc<dyn Fn(&str)>,
}

impl Default for MainActions {
    fn default() -> Self {
        Self {
            new_tab: noop(),
            new_window: noop(),
            new_private_window: noop(),
            new_profile: noop(),
            set_mode: Rc::new(|_| {}),
            set_block_social: Rc::new(|_| {}),
            open: Rc::new(|_| {}),
        }
    }
}

/// The ⋮ menu in the header bar.
pub fn main_menu(m: &MainMenu, a: &MainActions) -> Vec<Entry> {
    let modes = MODES
        .iter()
        .map(|&(mode, label, hint, icon)| {
            let set = a.set_mode.clone();
            Item::new(format!("menu-mode-{}", mode_id(mode)), label, icon, Rc::new(move || set(mode)))
                .hint(hint)
                .checked(m.mode == mode)
                .entry()
        })
        .collect();
    let open = |url: &'static str| {
        let open = a.open.clone();
        Rc::new(move || open(url)) as Run
    };
    let social = {
        let (set, on) = (a.set_block_social.clone(), m.block_social);
        Rc::new(move || set(!on)) as Run
    };
    vec![
        Item::new("menu-new-tab", "New tab", "plus", a.new_tab.clone()).entry(),
        Item::new("menu-new-window", "New window", "app-window", a.new_window.clone()).entry(),
        Item::new("menu-new-private", "New private window", "eye-off", a.new_private_window.clone()).entry(),
        Item::new("menu-new-profile", "New profile…", "user-round-plus", a.new_profile.clone()).entry(),
        Entry::Sep,
        Item::new("menu-mode", "Mode", "layout-grid", noop())
            .submenu(modes)
            .off_if(m.mode_off.is_some(), m.mode_off.as_deref().unwrap_or_default())
            .entry(),
        Item::new("menu-block-social", "Block all social media", "shield-ban", social).checked(m.block_social).entry(),
        Entry::Sep,
        Item::new("menu-history", "History", "clock", open("wadbrowser://history")).entry(),
        Item::new("menu-privacy", "Privacy", "shield-check", open("wadbrowser://privacy")).entry(),
        Item::new("menu-settings", "Settings", "settings", open("wadbrowser://settings")).entry(),
    ]
}

// ------------------------------------------------------------ the profile chip

/// What the profile chip's menu shows.
#[derive(Debug, Clone, Default)]
pub struct ProfileMenu {
    pub profiles: Vec<Profile>,
    /// This window's profile.
    pub current: String,
    pub private: bool,
}

/// What it does.
#[derive(Clone)]
pub struct ProfileActions {
    /// A window as profile `id` (this window's own: nothing to do).
    pub open_as: Rc<dyn Fn(&str)>,
    pub new_profile: Run,
    pub new_private_window: Run,
}

impl Default for ProfileActions {
    fn default() -> Self {
        Self { open_as: Rc::new(|_| {}), new_profile: noop(), new_private_window: noop() }
    }
}

/// The profile chip: this window's profile, a window as another, a new one.
pub fn profile_menu(m: &ProfileMenu, a: &ProfileActions) -> Vec<Entry> {
    let mut out = vec![Entry::Heading(if m.private { "A private window keeps nothing" } else { "Profiles" }.into())];
    for p in &m.profiles {
        let this = !m.private && p.id == m.current;
        let (open_as, id) = (a.open_as.clone(), p.id.clone());
        out.push(
            Item::new(format!("profile-{}", p.id), &p.name, "user", Rc::new(move || open_as(&id)))
                .checked(this)
                .hint(if this { "This window" } else { "Open a window" })
                .entry(),
        );
    }
    out.push(Entry::Sep);
    out.push(Item::new("profile-new", "New profile…", "user-round-plus", a.new_profile.clone()).entry());
    out.push(Item::new("profile-private", "New private window", "eye-off", a.new_private_window.clone()).entry());
    out
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
            [
                "page-back",
                "page-forward",
                "page-reload",
                "|",
                "page-create-app",
                "page-signins",
                "page-new-profile",
                "|",
                "page-inspect"
            ]
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
    fn the_main_menu_ticks_the_mode_and_the_social_blocker() {
        let set = Rc::new(RefCell::new(vec![]));
        let s = set.clone();
        let social = Rc::new(RefCell::new(None));
        let so = social.clone();
        let a = MainActions {
            set_mode: Rc::new(move |m| s.borrow_mut().push(m)),
            set_block_social: Rc::new(move |on| *so.borrow_mut() = Some(on)),
            ..Default::default()
        };
        let items = main_menu(&MainMenu { mode: Mode::Focus, block_social: true, mode_off: None }, &a);
        let modes = &find(&items, "menu-mode").unwrap().submenu;
        let ticked: Vec<&str> = modes
            .iter()
            .filter_map(|e| match e {
                Entry::Item(it) if it.checked => Some(it.id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(ticked, ["menu-mode-focus"]);
        assert_eq!(find(modes, "menu-mode-app").unwrap().hint.as_deref(), Some("This site in a window of its own"));
        (find(&items, "menu-mode-app").unwrap().run.as_ref().unwrap())();
        assert_eq!(*set.borrow(), [Mode::App]);
        let block = find(&items, "menu-block-social").unwrap();
        assert!(block.checked);
        (block.run.as_ref().unwrap())();
        assert_eq!(*social.borrow(), Some(false), "it turns it off");
    }

    #[test]
    fn the_main_menu_opens_the_browsers_pages() {
        let opened = Rc::new(RefCell::new(vec![]));
        let o = opened.clone();
        let a = MainActions { open: Rc::new(move |u: &str| o.borrow_mut().push(u.to_string())), ..Default::default() };
        let items = main_menu(&MainMenu { mode: Mode::Full, block_social: false, mode_off: None }, &a);
        for id in ["menu-history", "menu-privacy", "menu-settings"] {
            (find(&items, id).unwrap().run.as_ref().unwrap())();
        }
        assert_eq!(*opened.borrow(), ["wadbrowser://history", "wadbrowser://privacy", "wadbrowser://settings"]);
    }

    #[test]
    fn the_profile_menu_ticks_this_windows_profile_unless_its_private() {
        let profiles = vec![
            Profile { id: "default".into(), name: "Personal".into(), color: "#c6ff1f".into() },
            Profile { id: "work".into(), name: "Work".into(), color: "#7fd3ff".into() },
        ];
        let m = ProfileMenu { profiles: profiles.clone(), current: "default".into(), private: false };
        let items = profile_menu(&m, &ProfileActions::default());
        assert_eq!(
            ids(&items),
            ["[Profiles]", "profile-default", "profile-work", "|", "profile-new", "profile-private"]
        );
        let me = find(&items, "profile-default").unwrap();
        assert!(me.checked);
        assert_eq!(me.hint.as_deref(), Some("This window"));
        assert_eq!(find(&items, "profile-work").unwrap().hint.as_deref(), Some("Open a window"));

        let private = profile_menu(&ProfileMenu { private: true, ..m }, &ProfileActions::default());
        assert_eq!(ids(&private)[0], "[A private window keeps nothing]");
        assert!(!find(&private, "profile-default").unwrap().checked);
    }
}
