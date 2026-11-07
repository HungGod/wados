//! What site a page is on, for web app windows: their own pages, or
//! "offsite" once you leave the app's site. Ported from the spike's
//! `src/apps/browser/web.ts` (`hostOf`, `bareHost`, `siteOf`, `sameSite`).

/// `wadbrowser://` pages (WadBrowser's own: the new tab, settings, history…).
pub fn is_own_page(url: &str) -> bool {
    url.trim().split_once("://").is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("wadbrowser"))
}

/// The host, lowercased and without its port or credentials
/// ("https://User@GitHub.com:443/x" → "github.com"); empty when there's none.
pub fn host_of(url: &str) -> String {
    let url = url.trim();
    let Some((_, rest)) = url.split_once("://") else { return String::new() };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(v6) = host.strip_prefix('[') {
        // [::1]:8080 → ::1
        v6.split(']').next().unwrap_or("")
    } else {
        host.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map_or(host, |(h, _)| h)
    };
    host.to_ascii_lowercase()
}

/// The host and its port as written ("http://127.0.0.1:8080/x" → "127.0.0.1:8080").
pub fn authority_of(url: &str) -> String {
    let url = url.trim();
    let Some((_, rest)) = url.split_once("://") else { return String::new() };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    authority.rsplit_once('@').map_or(authority, |(_, h)| h).to_ascii_lowercase()
}

/// The path, without the query or hash ("/a/b"; "/" when there's none).
pub fn path_of(url: &str) -> String {
    let url = url.trim();
    let Some((_, rest)) = url.split_once("://") else { return "/".into() };
    let after = rest.find(['/', '?', '#']).map_or("", |i| &rest[i..]);
    let path = after.split(['?', '#']).next().unwrap_or("");
    if path.is_empty() { "/".into() } else { path.into() }
}

/// "www.reddit.com" → "reddit.com".
pub fn bare_host(host: &str) -> String {
    let h = host.to_ascii_lowercase();
    h.strip_prefix("www.").map(str::to_owned).unwrap_or(h)
}

/// Second-level domains under which a site is three labels ("bbc.co.uk").
const TWO_PART: &[&str] = &["co.uk", "org.uk", "ac.uk", "com.au", "net.au", "org.au", "co.nz", "co.jp"];

/// The site a host belongs to ("mail.google.com" → "google.com"; "bbc.co.uk"
/// stays; an IP address stays whole).
pub fn site_of(host: &str) -> String {
    let bare = bare_host(host);
    let labels: Vec<&str> = bare.split('.').collect();
    let numeric = labels.last().is_some_and(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_digit()));
    if labels.len() <= 2 || numeric || bare.contains(':') {
        return bare;
    }
    let last2 = labels[labels.len() - 2..].join(".");
    if TWO_PART.contains(&last2.as_str()) { labels[labels.len() - 3..].join(".") } else { last2 }
}

/// Are two URLs on the same site (an app's own pages, not offsite)?
pub fn same_site(a: &str, b: &str) -> bool {
    if is_own_page(a) || is_own_page(b) {
        return is_own_page(a) && is_own_page(b);
    }
    site_of(&host_of(a)) == site_of(&host_of(b))
}

/// A web app's window is offsite at `url`: a web page on another site than
/// its home's. WadBrowser's own pages (a new tab), a blank page and a page
/// still on its way (no address yet) aren't leaving the app.
pub fn offsite(url: &str, home: &str) -> bool {
    let url = url.trim();
    if url.is_empty() || url.starts_with("about:") || is_own_page(url) || host_of(url).is_empty() {
        return false;
    }
    !same_site(url, home)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_and_paths() {
        assert_eq!(host_of("https://GitHub.com/HungGod/x?tab=1"), "github.com");
        assert_eq!(host_of("http://localhost:5199/"), "localhost");
        assert_eq!(host_of("https://user:pw@mail.google.com:443/x"), "mail.google.com");
        assert_eq!(host_of("http://[::1]:8080/x"), "::1");
        assert_eq!(host_of("wadbrowser://newtab"), "newtab");
        assert_eq!(host_of("not a url"), "");
        assert_eq!(authority_of("http://127.0.0.1:8766/login/device"), "127.0.0.1:8766");
        assert_eq!(path_of("https://github.com/login/device?x=1#y"), "/login/device");
        assert_eq!(path_of("https://github.com"), "/");
        assert_eq!(path_of("https://github.com?q"), "/");
    }

    #[test]
    fn sites() {
        assert_eq!(bare_host("WWW.Reddit.com"), "reddit.com");
        assert_eq!(site_of("mail.google.com"), "google.com");
        assert_eq!(site_of("www.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(site_of("news.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(site_of("github.com"), "github.com");
        assert_eq!(site_of("localhost"), "localhost");
        assert_eq!(site_of("192.168.1.20"), "192.168.1.20");
        assert_eq!(site_of("::1"), "::1");
    }

    #[test]
    fn same_sites_as_the_spike_has_them() {
        assert!(same_site("https://mail.google.com/mail/u/0", "https://accounts.google.com/signin"));
        assert!(same_site("https://claude.ai/new", "https://claude.ai"));
        assert!(!same_site("https://claude.ai", "https://github.com"));
        // WadBrowser's own pages are a site of their own.
        assert!(same_site("wadbrowser://newtab", "wadbrowser://settings"));
        assert!(!same_site("wadbrowser://newtab", "https://claude.ai"));
        assert!(!same_site("http://127.0.0.1:8765/a", "http://localhost:8765/a"));
    }

    #[test]
    fn offsite_is_another_web_site() {
        let home = "https://claude.ai";
        assert!(!offsite("https://claude.ai/chat/1", home));
        assert!(!offsite("https://www.claude.ai/", home));
        assert!(offsite("https://github.com/anthropics", home));
        // A new tab, a blank page, a page on its way: still the app.
        assert!(!offsite("wadbrowser://newtab", home));
        assert!(!offsite("about:blank", home));
        assert!(!offsite("", home));
        // Gmail's sign-in is Google's.
        assert!(!offsite("https://accounts.google.com/x", "https://mail.google.com"));
    }

    #[test]
    fn own_pages() {
        assert!(is_own_page("wadbrowser://newtab"));
        assert!(is_own_page("WADBROWSER://about"));
        assert!(!is_own_page("https://cactus.example"));
        assert!(!is_own_page("wadbrowser"));
    }
}
