//! The machine's windows as the bar's tasks (`MachineApps.svelte`,
//! `RunningItems.svelte`): Settings, Files, WadBrowser and the Builder each
//! its own task once open, as KDE's icons-only task manager does (their
//! windows group under them, never a second button), then the machine's
//! other windows.
//! A wadspace's windows aren't tasks: the running wadspaces stand for them.
//! Pure: the bar draws it.

use crate::wm::Top;

/// One of the machine's own apps the bar has a launcher for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MachineApp {
    Settings,
    Files,
    Browser,
    /// The shell's own window (crate::builder).
    Builder,
}

impl MachineApp {
    pub const ALL: [MachineApp; 4] =
        [MachineApp::Settings, MachineApp::Files, MachineApp::Browser, MachineApp::Builder];

    pub fn name(self) -> &'static str {
        match self {
            MachineApp::Settings => "Settings",
            MachineApp::Files => "Files",
            MachineApp::Browser => "WadBrowser",
            MachineApp::Builder => "Builder",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            MachineApp::Settings => "settings",
            MachineApp::Files => "files",
            MachineApp::Browser => "browser",
            MachineApp::Builder => "builder",
        }
    }

    /// Its glyph (WadBrowser draws its own icon instead).
    pub fn icon(self) -> &'static str {
        match self {
            MachineApp::Settings => "settings",
            MachineApp::Files => "folder",
            MachineApp::Browser => "globe",
            MachineApp::Builder => "blocks",
        }
    }

    /// The app ids its windows have (the browser's fork keeps Cactus's until it's renamed).
    pub fn app_ids(self) -> &'static [&'static str] {
        match self {
            MachineApp::Settings => &["io.wados.Settings"],
            MachineApp::Files => &["io.wados.Files"],
            MachineApp::Browser => &["io.wados.Browser", "io.wados.WadBrowser", "org.cactus.Browser"],
            MachineApp::Builder => &[crate::builder::APP_ID],
        }
    }

    /// What starts it, and the phase it comes in while it isn't on the
    /// machine (none: the Builder, the shell's own window).
    pub fn launch(self) -> Option<(&'static [&'static str], u8)> {
        match self {
            MachineApp::Settings => Some((&["wados-settings"], 8)),
            MachineApp::Files => Some((&["wados-files"], 8)),
            MachineApp::Browser => Some((&["wadbrowser"], 5)),
            MachineApp::Builder => None,
        }
    }
}

/// The machine app a window belongs to, if it's one of theirs.
pub fn app_of(app_id: &str) -> Option<MachineApp> {
    MachineApp::ALL.into_iter().find(|a| a.app_ids().contains(&app_id))
}

/// The windows, as the bar shows them.
#[derive(Debug, Default, PartialEq)]
pub struct Tasks {
    /// Each machine app, with its windows (most recently in front first).
    pub apps: Vec<(MachineApp, Vec<Top>)>,
    /// The machine's other windows (the terminal, a picture, the Builder…), in the order they opened.
    pub others: Vec<Top>,
}

/// `windows` grouped, each app's windows in `recent`'s order.
pub fn group(windows: &[Top], recent: &Recency) -> Tasks {
    let apps = MachineApp::ALL
        .into_iter()
        .map(|a| {
            let mut mine: Vec<Top> =
                windows.iter().filter(|t| app_of(&t.app_id) == Some(a) && t.wadspace().is_none()).cloned().collect();
            mine.sort_by_key(|t| recent.rank(t.id));
            (a, mine)
        })
        .collect();
    let others = windows.iter().filter(|t| t.wadspace().is_none() && app_of(&t.app_id).is_none()).cloned().collect();
    Tasks { apps, others }
}

/// Which windows were in front most lately (foreign-toplevel doesn't say
/// how they're stacked): the one activated goes first.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Recency {
    order: Vec<u32>,
}

impl Recency {
    pub fn update(&mut self, windows: &[Top]) {
        self.order.retain(|id| windows.iter().any(|t| t.id == *id));
        for t in windows {
            if !self.order.contains(&t.id) {
                self.order.push(t.id);
            }
        }
        if let Some(front) = windows.iter().find(|t| t.activated && !t.minimized) {
            self.order.retain(|id| *id != front.id);
            self.order.insert(0, front.id);
        }
    }

    /// Where it is (0: in front most lately).
    pub fn rank(&self, id: u32) -> usize {
        self.order.iter().position(|x| *x == id).unwrap_or(usize::MAX)
    }
}

/// What a click on a task does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Click {
    /// Nothing open: start it.
    Launch,
    Activate(u32),
    /// In front already: put it down.
    Minimize(u32),
}

/// A window in front: activated and not minimized.
pub fn in_front(t: &Top) -> bool {
    t.activated && !t.minimized
}

/// A click on a task whose windows are `mine` (most recently in front
/// first): open it; bring its window up; or, in front already, put it down.
/// With more than one, go round them (the one in front longest ago next).
pub fn click(mine: &[Top]) -> Click {
    match mine {
        [] => Click::Launch,
        [one] if in_front(one) => Click::Minimize(one.id),
        [one] => Click::Activate(one.id),
        many if many.iter().any(in_front) => Click::Activate(many[many.len() - 1].id),
        many => Click::Activate(many[0].id),
    }
}

/// A window's glyph, by its app id.
pub fn glyph(app_id: &str) -> &'static str {
    let id = app_id.to_ascii_lowercase();
    if let Some(a) = app_of(app_id) {
        return a.icon();
    }
    match id.as_str() {
        "io.wados.shell" => "blocks",
        _ if id.contains("terminal") || id.contains("foot") || id.contains("ptyxis") || id.contains("konsole") => {
            "square-terminal"
        }
        _ if id.contains("image") || id.contains("loupe") || id.contains("eog") => "image",
        _ if id.contains("text") || id.contains("edit") => "file-text",
        _ => "app-window",
    }
}

/// A window's name in the bar: its title, or its app's.
pub fn title(t: &Top) -> String {
    if !t.title.trim().is_empty() {
        return t.title.clone();
    }
    match t.app_id.as_str() {
        "io.wados.Terminal" => "Terminal".into(),
        "io.wados.Shell" => "Builder".into(),
        "" => "Window".into(),
        id => id.rsplit('.').next().unwrap_or(id).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top(id: u32, app: &str, activated: bool) -> Top {
        Top { id, app_id: app.into(), title: format!("w{id}"), activated, ..Top::default() }
    }

    #[test]
    fn the_machine_apps_take_their_windows() {
        let windows = vec![
            top(1, "io.wados.Files", false),
            top(2, "io.wadspaces.studio", true),
            top(3, "io.wados.Terminal", false),
            top(4, "org.cactus.Browser", false),
            top(5, "io.wados.Files", false),
            top(6, "io.wados.Shell", false),
            top(7, "io.wados.Builder", false),
        ];
        let mut r = Recency::default();
        r.update(&windows);
        let t = group(&windows, &r);
        let ids = |a: MachineApp| -> Vec<u32> {
            t.apps.iter().find(|(x, _)| *x == a).unwrap().1.iter().map(|w| w.id).collect()
        };
        assert_eq!(ids(MachineApp::Settings), Vec::<u32>::new());
        assert_eq!(ids(MachineApp::Files), [1, 5]);
        assert_eq!(ids(MachineApp::Browser), [4]);
        assert_eq!(ids(MachineApp::Builder), [7]);
        // The wadspace's window isn't a task; the terminal and the shell's other window are the others.
        assert_eq!(t.others.iter().map(|w| w.id).collect::<Vec<_>>(), [3, 6]);
    }

    #[test]
    fn a_wadspace_cannot_pass_for_a_machine_app() {
        // A wadspace's app id is io.wadspaces.<id>, whatever its window says it is.
        assert_eq!(app_of("io.wadspaces.io.wados.Files"), None);
        assert_eq!(app_of("io.wados.Settings"), Some(MachineApp::Settings));
    }

    #[test]
    fn recency_puts_the_one_in_front_first() {
        let mut r = Recency::default();
        r.update(&[top(1, "a", true), top(2, "a", false), top(3, "a", false)]);
        assert_eq!(r.order, [1, 2, 3]);
        r.update(&[top(1, "a", false), top(2, "a", false), top(3, "a", true)]);
        assert_eq!(r.order, [3, 1, 2]);
        // Gone ones go; a new one comes last until it's in front.
        r.update(&[top(1, "a", false), top(3, "a", false), top(4, "a", false)]);
        assert_eq!(r.order, [3, 1, 4]);
        // Minimized isn't in front, even if it was activated.
        let mut min = top(4, "a", true);
        min.minimized = true;
        r.update(&[top(1, "a", false), top(3, "a", false), min]);
        assert_eq!(r.order, [3, 1, 4]);
    }

    #[test]
    fn a_click_opens_raises_lowers_or_goes_round() {
        assert_eq!(click(&[]), Click::Launch);
        assert_eq!(click(&[top(1, "a", false)]), Click::Activate(1));
        assert_eq!(click(&[top(1, "a", true)]), Click::Minimize(1));
        let mut min = top(1, "a", true);
        min.minimized = true;
        assert_eq!(click(&[min]), Click::Activate(1));
        // Several, one in front: the one in front longest ago next, so clicks go round all of them.
        assert_eq!(click(&[top(1, "a", true), top(2, "a", false), top(3, "a", false)]), Click::Activate(3));
        // Several, none in front: the last one in front.
        assert_eq!(click(&[top(2, "a", false), top(3, "a", false)]), Click::Activate(2));
    }

    #[test]
    fn going_round_three_windows_visits_each() {
        let mut r = Recency::default();
        let mut windows =
            vec![top(1, "io.wados.Files", true), top(2, "io.wados.Files", false), top(3, "io.wados.Files", false)];
        r.update(&windows);
        let mut seen = vec![];
        for _ in 0..3 {
            let mine = group(&windows, &r).apps.into_iter().find(|(a, _)| *a == MachineApp::Files).unwrap().1;
            let Click::Activate(id) = click(&mine) else { panic!("not activate") };
            seen.push(id);
            for w in &mut windows {
                w.activated = w.id == id;
            }
            r.update(&windows);
        }
        seen.sort();
        assert_eq!(seen, [1, 2, 3]);
    }

    #[test]
    fn glyphs_and_titles() {
        assert_eq!(glyph("io.wados.Terminal"), "square-terminal");
        assert_eq!(glyph("io.wados.Shell"), "blocks");
        assert_eq!(glyph("org.cactus.Browser"), "globe");
        assert_eq!(glyph("io.wados.Builder"), "blocks");
        assert_eq!(glyph("com.example.Thing"), "app-window");
        let mut t = top(1, "io.wados.Terminal", false);
        t.title = " ".into();
        assert_eq!(title(&t), "Terminal");
        t.app_id = "org.gnome.Loupe".into();
        assert_eq!(title(&t), "Loupe");
    }
}
