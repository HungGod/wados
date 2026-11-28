//! WadBrowser's settings, as a wadspace's design and the machine's Settings
//! keep them, and `wadbrowser.conf`, the file they reach the browser in.
//!
//! The browser itself has no settings page: a wadspace's come from the
//! Builder's add-ons (wad-gen writes them to
//! `/run/wadspaces-extra/wadbrowser.conf`), the machine's from Settings ▸
//! Browser (`~/.config/wadbrowser/wadbrowser.conf`). `key = value` lines,
//! `#` starts a comment; a later file's key wins, and one it doesn't name
//! is left as it was:
//!
//! ```text
//! default = full          # full (an address bar) or focus (none)
//! blocker = on            # the ad and tracker blocker
//! groups = social-media   # blocker groups: their sites don't open
//! block = x.com reddit.com  # sites of its own that don't open
//! ```
//!
//! **Blocker groups** are the machine's (the Library's `block_groups`): a
//! name and its sites, one tap to block them all in a wadspace or in
//! Settings. Social media's is made at first. A wadspace's file has them
//! counted in already (wad-gen: `block` is every site); the machine's
//! browser reads `groups` against [`MACHINE_GROUPS`], which wadd keeps as the
//! groups are (a group changed counts at once, in every open window).

use serde::{Deserialize, Serialize};

/// The machine's blocker groups, for its own browser (wadd writes them as
/// the Library has them: `id = site site …`).
pub const MACHINE_GROUPS: &str = "/run/wados/browser/groups.conf";

/// Social media's group, made at first: its id.
pub const SOCIAL_MEDIA: &str = "social-media";

/// Social media's sites (its group's at first).
pub const SUGGESTED_BLOCKS: [&str; 12] = [
    "facebook.com",
    "instagram.com",
    "tiktok.com",
    "x.com",
    "twitter.com",
    "reddit.com",
    "youtube.com",
    "snapchat.com",
    "threads.net",
    "linkedin.com",
    "pinterest.com",
    "bsky.app",
];

/// A blocker group: sites blocked together, a tap for them all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockGroup {
    pub id: String,
    pub name: String,
    /// As [`site`] keeps them.
    #[serde(default)]
    pub sites: Vec<String>,
}

impl BlockGroup {
    /// Its sites as [`site`] keeps them, once each.
    pub fn tidied(mut self) -> Self {
        self.sites = tidy(std::mem::take(&mut self.sites));
        self.name = self.name.trim().to_string();
        self
    }
}

/// The groups a machine starts with: social media's.
pub fn default_groups() -> Vec<BlockGroup> {
    vec![BlockGroup {
        id: SOCIAL_MEDIA.into(),
        name: "Social media".into(),
        sites: SUGGESTED_BLOCKS.iter().map(|s| s.to_string()).collect(),
    }]
}

/// The machine's blocker groups: social media's at first (a Library with
/// none written down yet), and whatever's been made, changed or removed since.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlockGroups(pub Vec<BlockGroup>);

impl Default for BlockGroups {
    fn default() -> Self {
        BlockGroups(default_groups())
    }
}

impl std::ops::Deref for BlockGroups {
    type Target = Vec<BlockGroup>;
    fn deref(&self) -> &Vec<BlockGroup> {
        &self.0
    }
}

impl std::ops::DerefMut for BlockGroups {
    fn deref_mut(&mut self) -> &mut Vec<BlockGroup> {
        &mut self.0
    }
}

/// A new group's id, from its name (`My Distractions` → `my-distractions`),
/// not one of `taken`.
pub fn group_id(name: &str, taken: &[String]) -> String {
    let mut base: String = name
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if base.is_empty() {
        base = "group".into();
    }
    let mut id = base.clone();
    let mut n = 2;
    while taken.contains(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// A browser's settings (a wadspace's, or the machine's).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BrowserPrefs {
    /// Tabs and an address bar (off: its tabs alone, no address to type).
    pub address_bar: bool,
    /// The ad and tracker blocker. (It searches with DuckDuckGo: that's
    /// not a setting; a `search` an older file has is skipped.)
    pub ad_blocker: bool,
    /// The blocker groups on (their ids): their sites don't open.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    /// Sites an older design blocked one by one: read, then made a group of
    /// its own by [`upgrade`]; never written.
    #[serde(rename = "blocked", skip_serializing)]
    pub old_sites: Vec<String>,
}

impl Default for BrowserPrefs {
    fn default() -> Self {
        BrowserPrefs { address_bar: true, ad_blocker: true, groups: vec![], old_sites: vec![] }
    }
}

impl BrowserPrefs {
    pub fn is_default(&self) -> bool {
        *self == BrowserPrefs::default()
    }

    /// Each group once.
    pub fn tidied(mut self) -> Self {
        let mut groups: Vec<String> = vec![];
        for g in std::mem::take(&mut self.groups) {
            let g = g.trim().to_string();
            if !g.is_empty() && !groups.contains(&g) {
                groups.push(g);
            }
        }
        self.groups = groups;
        self
    }

    /// The group `id` on, or off: whether it's on now.
    pub fn toggle_group(&mut self, id: &str) -> bool {
        if let Some(i) = self.groups.iter().position(|g| g == id) {
            self.groups.remove(i);
            false
        } else {
            self.groups.push(id.to_string());
            true
        }
    }

    /// Every site that doesn't open: the sites of its groups (those of
    /// `groups` it has on), once each.
    pub fn sites(&self, groups: &[BlockGroup]) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for s in groups.iter().filter(|g| self.groups.contains(&g.id)).flat_map(|g| g.sites.iter()) {
            if !out.contains(s) {
                out.push(s.clone());
            }
        }
        out
    }
}

impl BlockGroup {
    /// `text` (made a [`site`] first) in the group too: why not, if it can't be.
    pub fn add(&mut self, text: &str) -> Result<(), String> {
        let Some(s) = site(text) else {
            return Err("That isn’t a website: type one like example.com".into());
        };
        if self.sites.contains(&s) {
            return Err(format!("{s} is in it already"));
        }
        self.sites.push(s);
        Ok(())
    }

    /// `site` out of the group: whether it was in it.
    pub fn remove(&mut self, site: &str) -> bool {
        let before = self.sites.len();
        self.sites.retain(|s| s != site);
        self.sites.len() != before
    }
}

/// An older design's browser settings, as they are now: WadBrowser's Focus
/// add-on (no address bar) made [`BrowserPrefs::address_bar`] off, and sites
/// a wadspace blocked one by one made a group of its own, on in it. The
/// wadspaces changed (their ids); `lib.block_groups` may have changed too.
pub fn upgrade(lib: &mut crate::edit::Library) -> Vec<String> {
    let mut changed = vec![];
    for i in 0..lib.wadspaces.len() {
        let mut touched = false;
        let w = &mut lib.wadspaces[i];
        let focus = |e: &crate::types::Ext| e.app == crate::AppId::Wadbrowser && e.id == "focus" && e.url.is_none();
        if w.exts.iter().any(focus) {
            w.exts.retain(|e| !focus(e));
            w.secrets.by_slot.shift_remove("ext:focus");
            w.browser.address_bar = false;
            touched = true;
        }
        if !w.browser.old_sites.is_empty() {
            let sites = tidy(std::mem::take(&mut w.browser.old_sites));
            if !sites.is_empty() {
                let name = format!("{}’s sites", w.name);
                let taken: Vec<String> = lib.block_groups.iter().map(|g| g.id.clone()).collect();
                let id = group_id(&name, &taken);
                let w = &mut lib.wadspaces[i];
                w.browser.groups.push(id.clone());
                lib.block_groups.push(BlockGroup { id, name, sites });
            }
            touched = true;
        }
        if touched {
            changed.push(lib.wadspaces[i].id.clone());
        }
    }
    changed
}

/// `sites` as [`site`] keeps them, once each, in order.
fn tidy(sites: Vec<String>) -> Vec<String> {
    let mut seen = vec![];
    for s in sites {
        if let Some(s) = site(&s)
            && !seen.contains(&s)
        {
            seen.push(s);
        }
    }
    seen
}

/// The machine's groups file ([`MACHINE_GROUPS`]): `id = site site …` a line.
pub fn groups_text(groups: &[BlockGroup]) -> String {
    let mut out = String::from("# WadBrowser's blocker groups on this machine (wadd's, as the Library has them).\n");
    for g in groups {
        out.push_str(&format!("# {}\n{} = {}\n", g.name.replace('\n', " "), g.id, g.sites.join(" ")));
    }
    out
}

/// What a groups file says: each group's id and sites (names aren't kept).
pub fn parse_groups(text: &str) -> Vec<BlockGroup> {
    let mut out: Vec<BlockGroup> = vec![];
    for line in text.lines() {
        let line = line.split_once('#').map_or(line, |(l, _)| l).trim();
        let Some((id, sites)) = line.split_once('=') else { continue };
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        let sites: Vec<String> = sites.split([' ', ',', '\t']).filter(|s| !s.is_empty()).map(String::from).collect();
        out.retain(|g| g.id != id);
        out.push(BlockGroup { id: id.into(), name: id.into(), sites: tidy(sites) });
    }
    out
}

/// What someone typed as a site to block, as the blocker keeps it: its host,
/// lower case, without `www.`, a scheme, a port or a path
/// (`https://www.Reddit.com/r/x` → `reddit.com`). None if it isn't a host.
pub fn site(text: &str) -> Option<String> {
    let t = text.trim().to_ascii_lowercase();
    let t = t.split_once("://").map_or(t.as_str(), |(_, rest)| rest);
    let host = t.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split(':').next().unwrap_or("");
    let host = host.trim_end_matches('.');
    let host = host.strip_prefix("www.").unwrap_or(host);
    let ok = host.contains('.')
        && host.len() <= 253
        && host.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    ok.then(|| host.to_string())
}

/// Whether `host` is `blocked` or under one of them (`m.reddit.com` under `reddit.com`).
pub fn blocked_site<'a>(blocked: &'a [String], host: &str) -> Option<&'a str> {
    let h = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let h = h.strip_prefix("www.").unwrap_or(&h);
    blocked.iter().map(String::as_str).find(|s| h == *s || h.ends_with(&format!(".{s}")))
}

/// What a `wadbrowser.conf` says (each key it has; the rest are None).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfText {
    /// `default`: the address bar (true) or not (focus).
    pub address_bar: Option<bool>,
    pub ad_blocker: Option<bool>,
    pub groups: Option<Vec<String>>,
    /// `block`: the sites that don't open (a wadspace's: its groups'
    /// already; an older machine file's: sites of its own).
    pub blocked: Option<Vec<String>>,
}

impl ConfText {
    /// Reads one file's lines; ones it can't read are skipped.
    pub fn parse(text: &str) -> ConfText {
        let mut c = ConfText::default();
        for line in text.lines() {
            let line = line.split_once('#').map_or(line, |(l, _)| l).trim();
            let Some((key, value)) = line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) else { continue };
            match key {
                "default" => match value {
                    "full" => c.address_bar = Some(true),
                    "focus" | "lite" => c.address_bar = Some(false),
                    _ => {}
                },
                "blocker" => match value {
                    "on" | "true" | "yes" => c.ad_blocker = Some(true),
                    "off" | "false" | "no" => c.ad_blocker = Some(false),
                    _ => {}
                },
                "groups" => {
                    c.groups =
                        Some(value.split([' ', ',', '\t']).filter(|s| !s.is_empty()).map(String::from).collect());
                }
                "block" => {
                    c.blocked =
                        Some(tidy(value.split([' ', ',', '\t']).filter(|s| !s.is_empty()).map(String::from).collect()));
                }
                _ => {}
            }
        }
        c
    }

    /// These keys over `prefs` (`block` isn't a setting: see [`ConfText::blocked`]).
    pub fn over(&self, prefs: &mut BrowserPrefs) {
        if let Some(a) = self.address_bar {
            prefs.address_bar = a;
        }
        if let Some(b) = self.ad_blocker {
            prefs.ad_blocker = b;
        }
        if let Some(g) = &self.groups {
            prefs.groups = g.clone();
        }
    }
}

/// A `wadbrowser.conf` for `prefs`, headed by `about`: `sites` are what
/// `block` says (a wadspace's: its groups' sites; the machine's: none, its
/// groups being read from [`MACHINE_GROUPS`]).
pub fn conf(about: &str, prefs: &BrowserPrefs, sites: &[String]) -> String {
    let mut out = String::new();
    for l in about.lines() {
        out.push_str(if l.is_empty() { "#" } else { "# " });
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("#\n# Tabs and an address bar (full), or tabs alone (focus).\n");
    out.push_str(&format!("default = {}\n", if prefs.address_bar { "full" } else { "focus" }));
    out.push_str("# The ad and tracker blocker.\n");
    out.push_str(&format!("blocker = {}\n", if prefs.ad_blocker { "on" } else { "off" }));
    out.push_str("# Blocker groups on (their sites don't open): the machine's, by id.\n");
    out.push_str(&format!("groups = {}\n", prefs.groups.join(" ")));
    out.push_str("# Sites that don't open (and their subdomains).\n");
    out.push_str(&format!("block = {}\n", sites.join(" ")));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_is_its_bare_host() {
        assert_eq!(site("https://www.Reddit.com/r/rust?x#y").as_deref(), Some("reddit.com"));
        assert_eq!(site(" x.com ").as_deref(), Some("x.com"));
        assert_eq!(site("user@m.facebook.com:443/").as_deref(), Some("m.facebook.com"));
        assert_eq!(site("localhost"), None);
        assert_eq!(site("not a site.com"), None);
        assert_eq!(site(""), None);
        assert_eq!(site("-bad.com"), None);
    }

    #[test]
    fn subdomains_are_blocked_with_their_site() {
        let b = vec!["reddit.com".to_string()];
        assert_eq!(blocked_site(&b, "old.reddit.com"), Some("reddit.com"));
        assert_eq!(blocked_site(&b, "www.reddit.com"), Some("reddit.com"));
        assert_eq!(blocked_site(&b, "notreddit.com"), None);
    }

    #[test]
    fn a_groups_sites_are_added_and_removed_one_of_each() {
        let mut g = BlockGroup { id: "news".into(), name: "News".into(), sites: vec![] };
        assert_eq!(g.add("https://www.BBC.co.uk/news"), Ok(()));
        assert_eq!(g.add("bbc.co.uk"), Err("bbc.co.uk is in it already".into()));
        assert!(g.add("nonsense").is_err());
        assert_eq!(g.sites, vec!["bbc.co.uk"]);
        assert!(g.remove("bbc.co.uk"));
        assert!(!g.remove("bbc.co.uk"));
    }

    #[test]
    fn a_conf_reads_back_what_was_written() {
        let prefs = BrowserPrefs {
            address_bar: false,
            ad_blocker: false,
            groups: vec![SOCIAL_MEDIA.into(), "mine".into()],
            ..Default::default()
        };
        let text = conf("WadBrowser's settings\n\nfor tests", &prefs, &["x.com".into(), "reddit.com".into()]);
        assert!(text.starts_with("# WadBrowser's settings\n#\n# for tests\n"));
        let mut back = BrowserPrefs::default();
        let read = ConfText::parse(&text);
        read.over(&mut back);
        assert_eq!(back, prefs);
        assert_eq!(read.blocked, Some(vec!["x.com".into(), "reddit.com".into()]));
    }

    #[test]
    fn a_later_file_changes_only_what_it_names() {
        let mut p = BrowserPrefs::default();
        ConfText::parse("blocker = off\nblock = x.com, tiktok.com\n").over(&mut p);
        ConfText::parse("default = focus # mine\nsearch = wikipedia\nnonsense\n").over(&mut p);
        assert!(!p.ad_blocker && !p.address_bar, "an older file's search is skipped");
        assert_eq!(ConfText::parse("block = https://www.X.com/ x.com\n").blocked, Some(vec!["x.com".into()]));
    }

    #[test]
    fn an_older_designs_sites_are_read_never_written() {
        assert!(BrowserPrefs::default().is_default() && BrowserPrefs::default().address_bar);
        let p: BrowserPrefs = serde_json::from_str(r#"{"blocked":["x.com"]}"#).unwrap();
        assert_eq!(p.old_sites, vec!["x.com"]);
        assert!(!serde_json::to_string(&p).unwrap().contains("x.com"));
    }

    #[test]
    fn a_groups_sites_count_while_its_on() {
        let groups = default_groups();
        assert_eq!(groups[0].id, SOCIAL_MEDIA);
        assert!(groups[0].sites.iter().any(|s| s == "reddit.com"));
        let mut p = BrowserPrefs::default();
        assert!(p.sites(&groups).is_empty());
        assert!(p.toggle_group(SOCIAL_MEDIA));
        assert_eq!(p.sites(&groups).len(), SUGGESTED_BLOCKS.len());
        assert!(!p.toggle_group(SOCIAL_MEDIA));
        // A group that's gone counts for nothing.
        p.groups = vec!["gone".into()];
        assert!(p.sites(&groups).is_empty());
    }

    #[test]
    fn the_groups_file_reads_back() {
        let groups = vec![
            default_groups().remove(0),
            BlockGroup { id: "news".into(), name: "News # and = more".into(), sites: vec!["bbc.co.uk".into()] },
        ];
        let back = parse_groups(&groups_text(&groups));
        assert_eq!(
            back.iter().map(|g| (&g.id, &g.sites)).collect::<Vec<_>>(),
            groups.iter().map(|g| (&g.id, &g.sites)).collect::<Vec<_>>()
        );
        assert!(parse_groups("# nothing\n= x.com\nbad\n").is_empty());
    }

    #[test]
    fn a_new_groups_id_is_its_name() {
        assert_eq!(group_id("My Distractions!", &[]), "my-distractions");
        assert_eq!(group_id("Social media", &[SOCIAL_MEDIA.into()]), "social-media-2");
        assert_eq!(group_id("  ", &[]), "group");
    }

    #[test]
    fn a_library_without_groups_has_social_medias() {
        let groups: BlockGroups = serde_json::from_str("[]").unwrap();
        assert!(groups.is_empty(), "removed ones stay removed");
        assert_eq!(BlockGroups::default().len(), 1);
    }

    #[test]
    fn an_older_design_is_upgraded() {
        let mut lib = crate::edit::Library::default();
        let mut w = crate::ops::blank("iq", "IntelligenceQuest", "m", 0, 0);
        w.exts.push(crate::types::Ext::new(crate::AppId::Wadbrowser, "focus").named("Focus"));
        w.browser.old_sites = vec!["https://www.News.com".into(), "news.com".into()];
        lib.wadspaces.push(w);
        lib.wadspaces.push(crate::ops::blank("plain", "Plain", "m", 0, 0));
        assert_eq!(upgrade(&mut lib), vec!["iq"]);
        let w = &lib.wadspaces[0];
        assert!(!w.browser.address_bar && w.exts.is_empty(), "Focus is the address bar off");
        let g = lib.block_groups.iter().find(|g| g.name == "IntelligenceQuest’s sites").expect("its sites' group");
        assert_eq!(g.sites, vec!["news.com"]);
        assert_eq!(w.browser.groups, vec![g.id.clone()]);
        assert!(upgrade(&mut lib).is_empty(), "once");
    }
}
