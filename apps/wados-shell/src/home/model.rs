//! What the launcher shows, worked out from what wadd says (no GTK): its tabs
//! (Running, Recent, Work, Gaming, then the tags), each tab's wadspaces (in
//! every one, what's running comes first), which tab it opens on, and the
//! buttons under the card in focus. The spike's `Home.svelte` (its derived
//! state) and `sim/recent.ts`.

use wad_model::{Built, Category, Tag};

use crate::streams::Button as StreamButton;

/// Recent's length: the five launched most lately.
pub const RECENT: usize = 5;

/// One of the launcher's tabs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Tab {
    /// What's running now.
    Running,
    /// The five launched most lately.
    Recent,
    Work,
    Gaming,
    /// One of the library's tags (its id).
    Tag(String),
}

impl Tab {
    /// Its id, as `wadsctl home` says it (the spike's: `tag:<id>` for a tag).
    pub fn id(&self) -> String {
        match self {
            Tab::Running => "running".into(),
            Tab::Recent => "recent".into(),
            Tab::Work => "work".into(),
            Tab::Gaming => "gaming".into(),
            Tab::Tag(t) => format!("tag:{t}"),
        }
    }

    pub fn parse(s: &str) -> Option<Tab> {
        Some(match s {
            "running" => Tab::Running,
            "recent" => Tab::Recent,
            "work" => Tab::Work,
            "gaming" => Tab::Gaming,
            _ => Tab::Tag(s.strip_prefix("tag:")?.to_string()),
        })
    }

    /// What it says on the bar.
    pub fn label(&self, tags: &[Tag]) -> String {
        match self {
            Tab::Running => "Running".into(),
            Tab::Recent => "Recent".into(),
            Tab::Work => "Work".into(),
            Tab::Gaming => "Gaming".into(),
            Tab::Tag(id) => tags.iter().find(|t| &t.id == id).map(|t| t.name.clone()).unwrap_or_else(|| id.clone()),
        }
    }
}

/// One of the machine's wadspaces, as the launcher sorts them.
#[derive(Debug, Clone, PartialEq)]
pub struct Space {
    pub id: String,
    /// Its launcher tab (Work or Gaming).
    pub category: Category,
    /// Its tag's id (General's when it has none).
    pub tag: String,
    /// When it was last launched (its run's start, if that's later), ms; 0 never.
    pub launched: i64,
    pub running: bool,
}

/// The tabs: Running and Recent, then Work, Gaming and the tags made (General isn't one).
pub fn tabs(tags: &[Tag]) -> Vec<Tab> {
    let mut out = vec![Tab::Running, Tab::Recent, Tab::Work, Tab::Gaming];
    out.extend(tags.iter().filter(|t| t.id != wad_model::tags::GENERAL_ID).map(|t| Tab::Tag(t.id.clone())));
    out
}

fn latest_first(list: &mut [&Space]) {
    // Stable: ones launched at the same time keep their order.
    list.sort_by_key(|a| std::cmp::Reverse(a.launched));
}

/// What's running, the latest launched first.
pub fn running(spaces: &[Space]) -> Vec<&Space> {
    let mut out: Vec<&Space> = spaces.iter().filter(|x| x.running).collect();
    latest_first(&mut out);
    out
}

/// The five launched most lately (running or not), the latest first.
pub fn recent(spaces: &[Space]) -> Vec<&Space> {
    let mut out: Vec<&Space> = spaces.iter().filter(|x| x.launched > 0).collect();
    latest_first(&mut out);
    out.truncate(RECENT);
    out
}

/// The running ones first, then the rest, each in the order they came.
pub fn running_first(list: Vec<&Space>) -> Vec<&Space> {
    let (mut on, off): (Vec<&Space>, Vec<&Space>) = list.into_iter().partition(|x| x.running);
    on.extend(off);
    on
}

/// A tab's wadspaces (`spaces` in the library's order), running ones first.
pub fn spaces_of<'a>(tab: &Tab, spaces: &'a [Space]) -> Vec<&'a Space> {
    match tab {
        Tab::Running => running(spaces),
        Tab::Recent => running_first(recent(spaces)),
        Tab::Work => running_first(spaces.iter().filter(|x| x.category == Category::Work).collect()),
        Tab::Gaming => running_first(spaces.iter().filter(|x| x.category == Category::Gaming).collect()),
        Tab::Tag(t) => running_first(spaces.iter().filter(|x| &x.tag == t).collect()),
    }
}

/// Where it opens: on Running; with nothing running, Recent; with nothing launched yet, Work.
pub fn opens_on(spaces: &[Space]) -> Tab {
    if spaces.iter().any(|x| x.running) {
        Tab::Running
    } else if spaces.iter().any(|x| x.launched > 0) {
        Tab::Recent
    } else {
        Tab::Work
    }
}

/// The tab shown: `view` while it's still one of `tabs` (a tag deleted: Work).
pub fn shown(view: &Tab, tabs: &[Tab]) -> Tab {
    if tabs.contains(view) { view.clone() } else { Tab::Work }
}

// ------------------------------------------------------------ its buttons

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActId {
    /// Launch (Show, if it's running).
    Launch,
    Focus,
    /// Stream it publicly.
    Stream,
    /// Stream it privately, on your tailnet.
    Private,
    /// Check for updates (or build it).
    Update,
    Trash,
    /// Another machine's: copy it to this one.
    Copy,
}

/// One of the round buttons under the card in focus (named in its tooltip).
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub id: ActId,
    /// Its name ("Show", "Build"…), for wadsctl.
    pub label: &'static str,
    /// Its tooltip.
    pub hint: String,
    /// A lucide icon, or `tailscale` (Tailscale's mark).
    pub icon: &'static str,
    pub primary: bool,
    pub danger: bool,
    /// What isn't here yet (Focus) or can't be now (a stream that can't
    /// start: its tooltip says why) shows, but does nothing.
    pub enabled: bool,
}

/// When Focus comes, for its tooltip.
pub const FOCUS_LATER: &str = "Focus comes in a later phase";

/// Another machine's card's buttons (`peer` its name): Launch (it streams
/// from there, privately, into a window here), Focus (which runs here: it
/// asks to copy it first) and Copy here.
pub fn peer_actions(name: &str, peer: &str) -> Vec<Action> {
    let act =
        |id, label, hint: String, icon| Action { id, label, hint, icon, primary: false, danger: false, enabled: true };
    vec![
        Action { primary: true, ..act(ActId::Launch, "Launch", format!("Stream {name} here from {peer}"), "play") },
        act(ActId::Focus, "Focus", format!("Focus runs here: copy {name} to this machine first"), "timer"),
        act(ActId::Copy, "Copy here", "Copy it to this machine".into(), "download"),
    ]
}

/// What a tab with nothing in it says: its title and what to do. `on`:
/// another machine's name (none: this one's).
pub fn empty_text(view: &Tab, tags: &[Tag], on: Option<&str>) -> (String, &'static str) {
    match (view, on) {
        (Tab::Running, None) => ("Nothing’s running".into(), "Launch one from Recent or a tag."),
        (Tab::Running, Some(m)) => (format!("Nothing’s running from {m}"), "Launch one from Recent or a tag."),
        (Tab::Recent, None) => ("Nothing launched yet".into(), "The ones you launch show here."),
        (Tab::Recent, Some(m)) => (format!("Nothing launched from {m} yet"), "The ones you launch show here."),
        (t, None) => {
            (format!("No {} wadspaces yet", t.label(tags)), "Make one on the desktop: right-click it, or Ctrl+N.")
        }
        (t, Some(m)) => (format!("No {} wadspaces on {m}", t.label(tags)), "Pick another tag, or this machine."),
    }
}

/// What the launcher says while another machine's wadspaces haven't come:
/// asking it, or why it didn't answer.
pub fn peer_wait_text(peer: &str, error: Option<&str>) -> (String, String) {
    match error {
        None => (format!("Asking {peer} for its wadspaces"), "Over your tailnet.".into()),
        Some(why) => (format!("{peer} didn’t answer"), format!("{why}. Pick this machine, or try again later.")),
    }
}

/// The card's buttons: Launch (or Show), Focus, the two streams (`public`,
/// `private`: streams.rs's), an update check (or a build) and the Trash.
/// `built`: its image here (none: not said yet).
pub fn actions(
    name: &str,
    running: bool,
    built: Option<Built>,
    public: StreamButton,
    private: StreamButton,
) -> Vec<Action> {
    let act =
        |id, label, hint: String, icon| Action { id, label, hint, icon, primary: false, danger: false, enabled: true };
    let update = match built {
        Some(Built::Never) => act(ActId::Update, "Build", "Build it".into(), "hammer"),
        Some(Built::Update) => act(ActId::Update, "Build", "Rebuild it (it’s out of date)".into(), "hammer"),
        Some(Built::Built) | None => act(ActId::Update, "Update", "Check for updates".into(), "refresh-cw"),
    };
    vec![
        Action {
            primary: true,
            ..act(
                ActId::Launch,
                if running { "Show" } else { "Launch" },
                if running { format!("Show {name}") } else { format!("Open {name}") },
                "play",
            )
        },
        Action {
            enabled: false,
            ..act(
                ActId::Focus,
                "Focus",
                format!("Focus: {name} full screen for a time you set, with no leaving it until then ({FOCUS_LATER})"),
                "timer",
            )
        },
        Action { enabled: public.enabled, ..act(ActId::Stream, "Stream", public.hint, "radio") },
        Action { enabled: private.enabled, ..act(ActId::Private, "Private", private.hint, "tailscale") },
        update,
        Action { danger: true, ..act(ActId::Trash, "Trash", "Move it to the Trash".into(), "trash") },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(id: &str, category: Category, tag: &str, launched: i64, running: bool) -> Space {
        Space { id: id.into(), category, tag: tag.into(), launched, running }
    }

    /// The spike's sample on the Surface, in its library order.
    fn sample() -> Vec<Space> {
        use Category::*;
        vec![
            sp("wadspaces", Work, "general", 1793133000000, false),
            sp("cosmic-bodybuilding", Work, "writing", 1792621800000, false),
            sp("cosmic-bodybuilding-training", Work, "writing", 1792881000000, false),
            sp("vanua-academy", Work, "studio", 1793136600000, false),
            sp("vanua-academy-client-demo", Work, "studio", 1792708200000, false),
            sp("intelligencequest", Work, "studio", 1793053800000, false),
            sp("couch-co-op", Gaming, "play", 1792967400000, false),
        ]
    }

    fn tags() -> Vec<Tag> {
        ["studio", "writing", "play"]
            .iter()
            .map(|t| Tag {
                id: t.to_string(),
                name: format!("{}{}", t[..1].to_uppercase(), &t[1..]),
                color: "#fff".into(),
            })
            .collect()
    }

    fn ids<'a>(list: &[&'a Space]) -> Vec<&'a str> {
        list.iter().map(|x| x.id.as_str()).collect()
    }

    #[test]
    fn the_tabs_are_running_recent_work_gaming_then_the_tags() {
        let t = tabs(&tags());
        let shown: Vec<String> = t.iter().map(|x| x.label(&tags())).collect();
        assert_eq!(shown, ["Running", "Recent", "Work", "Gaming", "Studio", "Writing", "Play"]);
        assert_eq!(t[4].id(), "tag:studio");
        for x in &t {
            assert_eq!(Tab::parse(&x.id()).as_ref(), Some(x));
        }
        // General is never a tab of its own.
        let mut with_general = tags();
        with_general.insert(0, wad_model::tags::general());
        assert_eq!(tabs(&with_general).len(), 7);
    }

    #[test]
    fn the_counts_match_the_spikes_launcher() {
        let s = sample();
        let counts: Vec<usize> = tabs(&tags()).iter().map(|t| spaces_of(t, &s).len()).collect();
        // Running 0, Recent 5, Work 6, Gaming 1, Studio 3, Writing 2, Play 1.
        assert_eq!(counts, [0, 5, 6, 1, 3, 2, 1]);
    }

    #[test]
    fn recent_is_the_five_launched_most_lately() {
        let s = sample();
        assert_eq!(
            ids(&recent(&s)),
            ["vanua-academy", "wadspaces", "intelligencequest", "couch-co-op", "cosmic-bodybuilding-training"]
        );
        // Never launched isn't recent.
        let fresh = vec![sp("a", Category::Work, "general", 0, false)];
        assert!(recent(&fresh).is_empty());
    }

    #[test]
    fn whats_running_comes_first_in_every_tab() {
        let mut s = sample();
        // Training log opened just now, Client demo earlier today (a run's start counts as a launch).
        (s[2].running, s[2].launched) = (true, 1793200000000);
        (s[4].running, s[4].launched) = (true, 1793100000000);
        assert_eq!(ids(&spaces_of(&Tab::Running, &s)), ["cosmic-bodybuilding-training", "vanua-academy-client-demo"]);
        assert_eq!(
            ids(&spaces_of(&Tab::Work, &s)),
            [
                "cosmic-bodybuilding-training",
                "vanua-academy-client-demo",
                "wadspaces",
                "cosmic-bodybuilding",
                "vanua-academy",
                "intelligencequest"
            ]
        );
        assert_eq!(
            ids(&spaces_of(&Tab::Tag("studio".into()), &s)),
            ["vanua-academy-client-demo", "vanua-academy", "intelligencequest"]
        );
        // Recent: the five latest (Couch Co-op's sixth now), the running ones at its front.
        assert_eq!(
            ids(&spaces_of(&Tab::Recent, &s)),
            [
                "cosmic-bodybuilding-training",
                "vanua-academy-client-demo",
                "vanua-academy",
                "wadspaces",
                "intelligencequest"
            ]
        );
        // Nothing running in a tab: the library's order.
        assert_eq!(ids(&spaces_of(&Tab::Gaming, &s)), ["couch-co-op"]);
    }

    #[test]
    fn it_opens_on_running_else_recent_else_work() {
        let mut s = sample();
        assert_eq!(opens_on(&s), Tab::Recent);
        s[6].running = true;
        assert_eq!(opens_on(&s), Tab::Running);
        let never: Vec<Space> = sample().into_iter().map(|x| Space { launched: 0, ..x }).collect();
        assert_eq!(opens_on(&never), Tab::Work);
        assert_eq!(opens_on(&[]), Tab::Work);
    }

    #[test]
    fn a_tag_thats_gone_shows_work() {
        let t = tabs(&tags());
        assert_eq!(shown(&Tab::Tag("play".into()), &t), Tab::Tag("play".into()));
        assert_eq!(shown(&Tab::Tag("gone".into()), &t), Tab::Work);
        assert_eq!(shown(&Tab::Running, &t), Tab::Running);
    }

    #[test]
    fn another_machines_card_streams_from_there_or_copies_here() {
        let a = peer_actions("Pixel Art", "Desk");
        let names = a.iter().map(|x| x.label).collect::<Vec<_>>();
        assert_eq!(names, ["Launch", "Focus", "Copy here"]);
        assert_eq!(a[0].hint, "Stream Pixel Art here from Desk");
        assert!(a[0].primary && a[0].enabled);
        assert_eq!(a[1].hint, "Focus runs here: copy Pixel Art to this machine first");
        assert!(a[1].enabled && a[2].enabled);
        assert_eq!((a[2].id, a[2].hint.as_str(), a[2].icon), (ActId::Copy, "Copy it to this machine", "download"));
    }

    #[test]
    fn an_empty_tab_says_whose_it_is() {
        let t = tags();
        assert_eq!(empty_text(&Tab::Running, &t, None).0, "Nothing’s running");
        assert_eq!(empty_text(&Tab::Running, &t, Some("Desk")).0, "Nothing’s running from Desk");
        assert_eq!(empty_text(&Tab::Recent, &t, Some("Desk")).0, "Nothing launched from Desk yet");
        assert_eq!(
            empty_text(&Tab::Work, &t, None),
            ("No Work wadspaces yet".into(), "Make one on the desktop: right-click it, or Ctrl+N.")
        );
        assert_eq!(
            empty_text(&Tab::Tag("studio".into()), &t, Some("Desk")),
            ("No Studio wadspaces on Desk".into(), "Pick another tag, or this machine.")
        );
        assert_eq!(peer_wait_text("Desk", None).0, "Asking Desk for its wadspaces");
        assert_eq!(
            peer_wait_text("Desk", Some("Desk is offline")).1,
            "Desk is offline. Pick this machine, or try again later."
        );
    }

    fn can(mode: wad_model::StreamMode) -> StreamButton {
        let all = wad_proto::StreamsAvailability::of(true, true, false);
        crate::streams::card_button(mode, None, Some(&all))
    }

    #[test]
    fn the_buttons_say_launch_or_show_and_build_or_update() {
        use wad_model::StreamMode::{Private, Public};
        let names = |a: &[Action]| a.iter().map(|x| x.label).collect::<Vec<_>>();
        let a = actions("WadOS", false, Some(Built::Built), can(Public), can(Private));
        assert_eq!(names(&a), ["Launch", "Focus", "Stream", "Private", "Update", "Trash"]);
        assert_eq!(a[0].hint, "Open WadOS");
        assert!(a[0].primary && a[5].danger);
        let a = actions("WadOS", true, Some(Built::Never), can(Public), can(Private));
        assert_eq!(names(&a), ["Show", "Focus", "Stream", "Private", "Build", "Trash"]);
        assert_eq!((a[0].hint.as_str(), a[4].hint.as_str(), a[4].icon), ("Show WadOS", "Build it", "hammer"));
        assert_eq!(
            actions("X", false, Some(Built::Update), can(Public), can(Private))[4].hint,
            "Rebuild it (it’s out of date)"
        );
        // Focus isn't here yet: shown, not working, and its tooltip says when.
        let off: Vec<ActId> = a.iter().filter(|x| !x.enabled).map(|x| x.id).collect();
        assert_eq!(off, [ActId::Focus]);
        assert!(a[1].hint.contains(FOCUS_LATER));
        // The streams: publicly (radio) and privately (Tailscale's mark).
        assert_eq!((a[2].hint.as_str(), a[2].icon), ("Stream it publicly", "radio"));
        assert_eq!((a[3].hint.as_str(), a[3].icon), ("Stream it privately, on your tailnet", "tailscale"));
        // One that can't start here is off, its tooltip saying why.
        let none = wad_proto::StreamsAvailability::of(true, false, false);
        let a = actions("WadOS", false, None, crate::streams::card_button(Public, None, Some(&none)), can(Private));
        assert!(!a[2].enabled && a[2].hint.contains("Cloudflare tunnel"), "{}", a[2].hint);
    }
}
