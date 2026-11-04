//! The plain data the v2 widgets show (menus, the chip, the dialogs), kept
//! apart from the engine's own types: the chrome maps its profiles and keys
//! into these.

/// How a window shows the web: Full, Focus, App.
pub use wadbrowser_core::modes::Mode;

/// Each mode: its label, the hint under it, its icon.
pub const MODES: [(Mode, &str, &str, &str); 3] = [
    (Mode::Full, "Full", "Tabs and an address bar", "globe"),
    (Mode::Focus, "Focus", "Tabs, no address bar", "eye"),
    (Mode::App, "App", "This site in a window of its own", "app-window"),
];

/// A mode's id, as the menu's items are named (`full`).
pub fn mode_id(m: Mode) -> &'static str {
    match m {
        Mode::Full => "full",
        Mode::Focus => "focus",
        Mode::App => "app",
    }
}

/// A browser profile: its own sign-ins, cookies and history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    /// Its colour, `#rrggbb`.
    pub color: String,
}

/// A saved sign-in for the web (a key): the sites it signs in to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebKey {
    pub id: String,
    pub name: String,
    /// Bare hosts ("github.com").
    pub sites: Vec<String>,
    /// How many wadspaces use it.
    pub used_by: u32,
}

/// A URL's host without `www.` or a port ("https://www.GitHub.com:443/x" →
/// "github.com"); empty if it has none.
pub fn bare_host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default();
    let host = if host.starts_with('[') {
        host.split_inclusive(']').next().unwrap_or(host)
    } else {
        host.split(':').next().unwrap_or(host)
    };
    let host = host.to_ascii_lowercase();
    host.strip_prefix("www.").map(str::to_string).unwrap_or(host)
}

/// One of the browser's own pages (cactus://, about:).
pub fn is_cactus(url: &str) -> bool {
    wadbrowser_core::internal::is_internal(url)
}

/// A colour for CSS: `#rgb` or `#rrggbb`, else the accent.
pub fn safe_color(c: &str) -> &str {
    let hex = c.strip_prefix('#').unwrap_or("");
    if matches!(hex.len(), 3 | 6) && hex.bytes().all(|b| b.is_ascii_hexdigit()) { c } else { "#c6ff1f" }
}

/// 1284 → "1.3k" (the spike's `compact`).
pub fn compact(n: u64) -> String {
    if n < 1000 {
        return n.to_string();
    }
    if n < 10_000 {
        let s = format!("{:.1}", n as f64 / 1000.0);
        return format!("{}k", s.strip_suffix(".0").unwrap_or(&s));
    }
    format!("{}k", (n as f64 / 1000.0).round() as u64)
}

/// 1284 → "1,284" (the spike's `toLocaleString('en-AU')`).
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_bare() {
        assert_eq!(bare_host("https://www.GitHub.com:443/HungGod?x#y"), "github.com");
        assert_eq!(bare_host("http://user:pw@old.reddit.com/r"), "old.reddit.com");
        assert_eq!(bare_host("github.com/x"), "github.com");
        assert_eq!(bare_host("cactus://newtab"), "newtab");
        assert_eq!(bare_host("http://[::1]:8080/"), "[::1]");
    }

    #[test]
    fn counts_read_like_the_spikes() {
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1284), "1.3k");
        assert_eq!(compact(2000), "2k");
        assert_eq!(compact(12_600), "13k");
        assert_eq!(thousands(1284), "1,284");
        assert_eq!(thousands(12), "12");
    }

    #[test]
    fn colours_are_checked() {
        assert_eq!(safe_color("#7fd3ff"), "#7fd3ff");
        assert_eq!(safe_color("#fff"), "#fff");
        assert_eq!(safe_color("red; x"), "#c6ff1f");
    }
}
