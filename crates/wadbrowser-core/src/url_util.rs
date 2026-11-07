use crate::prefs::Preferences;
use url::Url;

/// Turn omnibox input into a navigable URI.
pub fn normalize_navigation_input(input: &str, prefs: &Preferences) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return prefs.homepage.clone();
    }

    if trimmed.starts_with("wadbrowser://")
        || trimmed.starts_with("about:")
        || trimmed.starts_with("file://")
        || trimmed.starts_with("data:")
        || trimmed.starts_with("blob:")
    {
        return trimmed.to_string();
    }

    if let Some(rest) = trimmed.strip_prefix("http://") {
        if prefs.https_first && !looks_like_local(rest) {
            return format!("https://{rest}");
        }
        return trimmed.to_string();
    }

    if trimmed.starts_with("https://") {
        return trimmed.to_string();
    }

    // Bare domain / host:port / path-looking host
    if looks_like_url(trimmed) {
        let candidate = if prefs.https_first && !looks_like_local(trimmed) {
            format!("https://{trimmed}")
        } else {
            format!("http://{trimmed}")
        };
        if Url::parse(&candidate).is_ok() {
            return candidate;
        }
    }

    prefs.search_url(trimmed)
}

fn looks_like_local(hostish: &str) -> bool {
    let host = hostish.split('/').next().unwrap_or(hostish);
    let host = host.split(':').next().unwrap_or(host);
    host == "localhost"
        || host.starts_with("127.")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.ends_with(".local")
}

fn looks_like_url(s: &str) -> bool {
    if s.contains(' ') {
        return false;
    }
    if s.starts_with("localhost") || s.starts_with("127.0.0.1") {
        return true;
    }
    // has a dot and TLD-ish, or host:port
    if let Some((host, _)) = s.split_once('/') {
        return host_looks_valid(host);
    }
    host_looks_valid(s)
}

fn host_looks_valid(host: &str) -> bool {
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() || host.contains(' ') {
        return false;
    }
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return true;
    }
    host.contains('.') && !host.starts_with('.') && !host.ends_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_first_upgrades() {
        let prefs = Preferences::default();
        assert_eq!(normalize_navigation_input("example.com", &prefs), "https://example.com");
        assert_eq!(normalize_navigation_input("http://example.com", &prefs), "https://example.com");
    }

    #[test]
    fn search_fallback() {
        let prefs = Preferences::default();
        let u = normalize_navigation_input("rust ownership", &prefs);
        assert!(u.contains("duckduckgo.com"));
    }

    #[test]
    fn own_scheme_passthrough() {
        let prefs = Preferences::default();
        assert_eq!(normalize_navigation_input("wadbrowser://settings", &prefs), "wadbrowser://settings");
    }
}
