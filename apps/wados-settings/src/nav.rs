//! Settings' panels (the spike's `nav.ts`): the sidebar's groups, the
//! words its search matches, and the other names the shell (or a
//! notification) may open them by.

/// A panel as the sidebar shows it.
#[derive(Debug, PartialEq, Eq)]
pub struct PanelInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
    pub keywords: &'static [&'static str],
}

const fn p(id: &'static str, label: &'static str, icon: &'static str, keywords: &'static [&'static str]) -> PanelInfo {
    PanelInfo { id, label, icon, keywords }
}

/// The sidebar, in its groups.
pub const GROUPS: &[&[PanelInfo]] = &[
    &[
        p("wifi", "Wi-Fi", "wifi", &["wireless", "network", "ssid", "airplane", "password", "hotspot"]),
        p("network", "Network", "network", &["wired", "ethernet", "cable", "vpn", "tailscale", "proxy"]),
        p("bluetooth", "Bluetooth", "bluetooth", &["headphones", "mouse", "keyboard", "controller", "pair", "devices"]),
    ],
    &[
        p(
            "displays",
            "Displays",
            "monitor",
            &[
                "monitor",
                "screen",
                "resolution",
                "size",
                "scale",
                "refresh",
                "orientation",
                "night light",
                "brightness",
                "arrangement",
            ],
        ),
        p(
            "sound",
            "Sound",
            "volume-2",
            &["audio", "volume", "speakers", "headphones", "microphone", "input", "output", "mute"],
        ),
        p(
            "power",
            "Power",
            "battery-charging",
            &["battery", "power mode", "performance", "saver", "suspend", "blank", "lid", "server mode"],
        ),
    ],
    &[
        p("background", "Background", "image", &["wallpaper", "desktop", "appearance", "dark", "theme", "picture"]),
        p(
            "lock",
            "Lock Screen & Privacy",
            "lock",
            &["password", "lock", "privacy", "auto lock", "notifications", "security", "picture"],
        ),
    ],
    &[
        p(
            "accounts",
            "Accounts",
            "users",
            &["github", "tailscale", "tailnet", "steam", "games", "devices", "trust", "phone"],
        ),
        p("storage", "Storage", "hard-drive", &["disk", "space", "clean up", "base images", "wadspaces", "usage"]),
    ],
    &[
        p(
            "mouse",
            "Mouse & Touchpad",
            "mouse",
            &["pointer", "speed", "scrolling", "natural", "tap to click", "primary button", "touchpad"],
        ),
        p("keyboard", "Keyboard", "keyboard", &["layout", "input", "repeat", "shortcuts", "keys"]),
    ],
    &[p(
        "system",
        "System",
        "info",
        &["about", "updates", "date", "time", "time zone", "users", "version", "hardware", "hostname", "firmware"],
    )],
];

/// Every panel, in the sidebar's order.
pub fn panels() -> impl Iterator<Item = &'static PanelInfo> {
    GROUPS.iter().flat_map(|g| g.iter())
}

pub fn info(id: &str) -> Option<&'static PanelInfo> {
    panels().find(|p| p.id == id)
}

/// System's pages (its `sub`).
pub const SUBS: &[&str] = &["about", "datetime", "users", "updates"];

/// A System page's name.
#[allow(dead_code)]
pub fn sub_label(sub: &str) -> &'static str {
    match sub {
        "about" => "About",
        "datetime" => "Date & Time",
        "users" => "Users",
        "updates" => "Updates",
        _ => "",
    }
}

/// The other names a panel's opened by (and System's page they open).
const ALIASES: &[(&str, &str, Option<&str>)] = &[
    ("display", "displays", None),
    ("monitor", "displays", None),
    ("night-light", "displays", None),
    ("audio", "sound", None),
    ("volume", "sound", None),
    ("battery", "power", None),
    ("wired", "network", None),
    ("vpn", "network", None),
    ("appearance", "background", None),
    ("wallpaper", "background", None),
    ("privacy", "lock", None),
    ("lock-screen", "lock", None),
    ("lockscreen", "lock", None),
    ("password", "lock", None),
    ("tailscale", "accounts", None),
    ("tailnet", "accounts", None),
    ("github", "accounts", None),
    ("steam", "accounts", None),
    ("online", "accounts", None),
    ("disk", "storage", None),
    ("touchpad", "mouse", None),
    ("about", "system", Some("about")),
    ("datetime", "system", Some("datetime")),
    ("date-time", "system", Some("datetime")),
    ("users", "system", Some("users")),
    // The spike's opens System; here it's System's own Updates page.
    ("updates", "system", Some("updates")),
];

/// Where a page asked for is (`resolvePanel`): a panel's id or another
/// name for it, and System's page (`sub`, or the alias's); none when it's
/// no panel's.
pub fn resolve(page: &str, sub: Option<&str>) -> Option<(&'static str, Option<&'static str>)> {
    let raw = page.trim().to_lowercase();
    let (id, alias_sub) = match info(&raw) {
        Some(p) => (p.id, None),
        None => ALIASES.iter().find(|(a, _, _)| *a == raw).map(|(_, id, s)| (*id, *s))?,
    };
    let sub = sub.and_then(|s| SUBS.iter().find(|x| **x == s).copied()).or(alias_sub);
    Some((id, if id == "system" { sub } else { None }))
}

/// The panel a window opens on when it isn't told (or is told something no panel is).
pub const FIRST: &str = "wifi";

/// True if `p` matches the search (its name, or what's in it).
pub fn matches(p: &PanelInfo, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || p.label.to_lowercase().contains(&q) || p.keywords.iter().any(|k| k.contains(&q))
}

/// The sidebar's groups for a search: each group's panels that match, empty groups gone.
pub fn filter(query: &str) -> Vec<Vec<&'static PanelInfo>> {
    GROUPS
        .iter()
        .map(|g| g.iter().filter(|p| matches(p, query)).collect::<Vec<_>>())
        .filter(|g| !g.is_empty())
        .collect()
}

/// What Enter in the search opens.
pub fn first_match(query: &str) -> Option<&'static PanelInfo> {
    filter(query).first().and_then(|g| g.first().copied())
}

/// What the sidebar says when nothing does.
pub fn nothing_matches(query: &str) -> String {
    format!("Nothing matches “{}”.", query.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sidebar_is_the_spikes() {
        let ids: Vec<_> = panels().map(|p| p.id).collect();
        assert_eq!(
            ids,
            [
                "wifi",
                "network",
                "bluetooth",
                "displays",
                "sound",
                "power",
                "background",
                "lock",
                "accounts",
                "storage",
                "mouse",
                "keyboard",
                "system"
            ]
        );
        assert_eq!(GROUPS.len(), 6);
        assert_eq!(info("lock").unwrap().label, "Lock Screen & Privacy");
    }

    #[test]
    fn pages_resolve_by_id_or_alias() {
        assert_eq!(resolve("accounts", None), Some(("accounts", None)));
        assert_eq!(resolve("Accounts", None), Some(("accounts", None)));
        assert_eq!(resolve("tailscale", None), Some(("accounts", None)));
        assert_eq!(resolve("night-light", None), Some(("displays", None)));
        assert_eq!(resolve("updates", None), Some(("system", Some("updates"))));
        assert_eq!(resolve("date-time", None), Some(("system", Some("datetime"))));
        assert_eq!(resolve("system", Some("users")), Some(("system", Some("users"))));
        assert_eq!(resolve("system", Some("nope")), Some(("system", None)));
        // A sub only means something to System.
        assert_eq!(resolve("wifi", Some("about")), Some(("wifi", None)));
        assert_eq!(resolve("bogus", None), None);
        assert!(ALIASES.iter().all(|(_, id, _)| info(id).is_some()));
        assert!(ALIASES.iter().filter_map(|(_, _, s)| *s).all(|s| SUBS.contains(&s) && !sub_label(s).is_empty()));
    }

    #[test]
    fn search_matches_names_and_keywords() {
        let ids = |q: &str| filter(q).iter().flatten().map(|p| p.id).collect::<Vec<_>>();
        assert_eq!(ids("").len(), 13);
        assert_eq!(ids("blue"), ["bluetooth"]);
        assert_eq!(ids("  WALLpaper "), ["background"]);
        assert_eq!(ids("headphones"), ["bluetooth", "sound"]);
        assert_eq!(ids("password"), ["wifi", "lock"]);
        assert_eq!(ids("update"), ["system"]);
        assert!(ids("zzz").is_empty());
        // Empty groups go, so no separator's left alone.
        assert_eq!(filter("headphones").len(), 2);
        assert_eq!(first_match("tail").map(|p| p.id), Some("network"));
        assert_eq!(first_match("zzz"), None);
        assert_eq!(nothing_matches(" zzz "), "Nothing matches “zzz”.");
    }
}
