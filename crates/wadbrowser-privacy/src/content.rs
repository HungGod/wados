//! WebKit's content filters (its content-blocker JSON, compiled by
//! `webkit6::UserContentFilterStore`): the blocker's, made from the filter
//! lists by the adblock crate's `content-blocking` converter, and the
//! social media blocker's, made from [`SOCIAL`]. WebKit does the blocking;
//! the browser only counts what it blocked.
//!
//! The converter ends the lists' rules with one of its own: an
//! `ignore-previous-rules` for first-party documents (a filter list never
//! blocks the page you went to, only what it loads). It covers the rules
//! before it in the same filter only, which is why the social blocker, whose
//! point is the pages themselves, is a filter of its own.

use anyhow::{Result, anyhow};
use serde_json::json;

/// How lists become rules: in the cache id, so a change here (or a new
/// adblock crate) compiles again.
pub const FORMAT: &str = "wadbrowser-content-1/adblock-0.13";
/// The blocker's filters' ids start so (then the lists' hash).
pub const LISTS_PREFIX: &str = "wadbrowser-lists-";
/// The social blocker's filter's id starts so.
pub const SOCIAL_PREFIX: &str = "wadbrowser-social-";

/// Social media the "Block all social media" blocker stops (the spike's catalog's).
pub const SOCIAL: &[&str] = &[
    "x.com",
    "twitter.com",
    "facebook.com",
    "instagram.com",
    "tiktok.com",
    "reddit.com",
    "threads.net",
    "linkedin.com",
    "snapchat.com",
    "bsky.app",
];

/// The social site `host` is part of, if it's one the blocker stops.
pub fn social_site(host: &str) -> Option<&'static str> {
    let h = host.trim().to_ascii_lowercase();
    let h = h.strip_prefix("www.").unwrap_or(&h);
    SOCIAL.iter().copied().find(|s| h == *s || h.ends_with(&format!(".{s}")))
}

/// The lists as WebKit's rules (JSON), and how many rules there are.
/// CPU-heavy for EasyList (seconds): off the main thread.
pub fn lists_rules(lists: &[String]) -> Result<(String, usize)> {
    // The converter needs the filters' text: a debug FilterSet.
    let mut set = adblock::lists::FilterSet::new(true);
    for l in lists {
        set.add_filter_list(l.clone(), adblock::lists::ParseOptions::default());
    }
    let (rules, _used) = set.into_content_blocking().map_err(|()| anyhow!("the lists didn't convert"))?;
    let n = rules.len();
    // WebKit refuses an empty rule list: a rule that matches nothing stands in.
    let json = if rules.is_empty() { nothing_rules() } else { serde_json::to_string(&rules)? };
    Ok((json, n))
}

/// The social blocker's rules: everything from each social site, its pages too.
pub fn social_rules() -> String {
    let rules: Vec<_> = SOCIAL
        .iter()
        .map(|s| {
            json!({
                "trigger": { "url-filter": format!("^[a-z][a-z0-9.+-]*://([^/]*\\.)?{}[:/]", regex_escape(s)) },
                "action": { "type": "block" }
            })
        })
        .collect();
    serde_json::to_string(&rules).unwrap_or_default()
}

/// The social blocker's filter id (its sites' hash, so a change compiles again).
pub fn social_identifier() -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(FORMAT.as_bytes());
    h.update(social_rules().as_bytes());
    let hex: String = h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{SOCIAL_PREFIX}{hex}")
}

/// A rule list that blocks nothing (WebKit won't compile an empty one).
fn nothing_rules() -> String {
    json!([{ "trigger": { "url-filter": "^wadbrowser-matches-nothing:" }, "action": { "type": "block" } }]).to_string()
}

fn regex_escape(s: &str) -> String {
    s.chars().fold(String::new(), |mut out, c| {
        if !c.is_ascii_alphanumeric() && c != '-' {
            out.push('\\');
        }
        out.push(c);
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_become_webkit_rules_with_the_first_party_exception_last() {
        let (json, n) =
            lists_rules(&["||ads.test^\n##.banner\n".into(), "||track.test^$third-party\n".into()]).expect("converted");
        let rules: serde_json::Value = serde_json::from_str(&json).unwrap();
        let rules = rules.as_array().unwrap();
        assert_eq!(rules.len(), n);
        let actions: Vec<&str> = rules.iter().map(|r| r["action"]["type"].as_str().unwrap()).collect();
        assert!(actions.contains(&"block") && actions.contains(&"css-display-none"), "{actions:?}");
        // The converter's own exception for the page you went to comes last.
        let last = rules.last().unwrap();
        assert_eq!(last["action"]["type"], "ignore-previous-rules");
        assert_eq!(last["trigger"]["resource-type"], serde_json::json!(["document"]));
        assert_eq!(last["trigger"]["load-type"], serde_json::json!(["first-party"]));
        assert!(json.contains("ads\\\\.test"), "{json}");
    }

    #[test]
    fn no_rules_still_compiles() {
        let (json, n) = lists_rules(&["! only a comment\n".into()]).expect("converted");
        assert_eq!(n, 0);
        assert!(json.contains("wadbrowser-matches-nothing"));
    }

    #[test]
    fn the_social_blocker_stops_each_site_and_its_subdomains() {
        assert_eq!(social_site("www.reddit.com"), Some("reddit.com"));
        assert_eq!(social_site("old.reddit.com"), Some("reddit.com"));
        assert_eq!(social_site("notreddit.com"), None);
        assert_eq!(social_site("github.com"), None);
        let rules: serde_json::Value = serde_json::from_str(&social_rules()).unwrap();
        assert_eq!(rules.as_array().unwrap().len(), SOCIAL.len());
        assert_eq!(rules[0]["trigger"]["url-filter"], "^[a-z][a-z0-9.+-]*://([^/]*\\.)?x\\.com[:/]");
        assert!(rules[0]["trigger"].get("resource-type").is_none(), "pages too");
        assert!(social_identifier().starts_with(SOCIAL_PREFIX));
        assert_eq!(social_identifier(), social_identifier());
    }
}
