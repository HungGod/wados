//! Small helpers for cleaning titles/URLs before persistence or display.

/// Collapse whitespace and trim; limit length for UI/DB friendliness.
pub fn clean_title(title: &str, max_chars: usize) -> String {
    let collapsed: String = title.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&collapsed, max_chars)
}

pub fn clean_url_for_display(url: &str, max_chars: usize) -> String {
    let trimmed = url.trim();
    truncate_chars(trimmed, max_chars)
}

pub fn host_label(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.to_string())).unwrap_or_else(|| url.to_string())
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Reject obviously dangerous local schemes for bookmarking / history display.
pub fn is_persistable_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    if lower.starts_with("wadbrowser://")
        || lower.starts_with("about:")
        || lower.starts_with("data:")
        || lower.starts_with("blob:")
        || lower.starts_with("javascript:")
    {
        return false;
    }
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("file://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_title_whitespace() {
        assert_eq!(clean_title("  hello   rust\n", 32), "hello rust");
    }

    #[test]
    fn persistable_http_only() {
        assert!(is_persistable_url("https://example.com"));
        assert!(!is_persistable_url("wadbrowser://newtab"));
        assert!(!is_persistable_url("javascript:alert(1)"));
    }
}
