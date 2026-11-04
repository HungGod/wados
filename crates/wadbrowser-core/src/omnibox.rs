//! Omnibox suggestion ranking from history and bookmarks.

use crate::store::{Bookmark, HistoryEntry};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionKind {
    History,
    Bookmark,
    Search,
    Url,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub title: String,
    pub url: String,
    pub kind: SuggestionKind,
    pub score: i64,
}

/// Rank suggestions for the current omnibox text.
pub fn suggest(
    query: &str,
    history: &[HistoryEntry],
    bookmarks: &[Bookmark],
    search_template: impl Fn(&str) -> String,
    limit: usize,
) -> Vec<Suggestion> {
    let q = query.trim();
    if q.is_empty() {
        return recent_defaults(history, bookmarks, limit);
    }

    let q_lower = q.to_ascii_lowercase();
    let mut out = Vec::new();

    for b in bookmarks {
        if let Some(score) = match_score(&q_lower, &b.title, &b.url, 120, b.position) {
            out.push(Suggestion { title: b.title.clone(), url: b.url.clone(), kind: SuggestionKind::Bookmark, score });
        }
    }

    for h in history {
        let boost = h.visit_count.min(50);
        if let Some(score) = match_score(&q_lower, &h.title, &h.url, 100, boost) {
            out.push(Suggestion {
                title: if h.title.is_empty() { h.url.clone() } else { h.title.clone() },
                url: h.url.clone(),
                kind: SuggestionKind::History,
                score,
            });
        }
    }

    if looks_like_typed_url(q) {
        let url = if q.contains("://") { q.to_string() } else { format!("https://{q}") };
        out.push(Suggestion { title: format!("Go to {q}"), url, kind: SuggestionKind::Url, score: 90 });
    }

    out.push(Suggestion {
        title: format!("Search “{q}”"),
        url: search_template(q),
        kind: SuggestionKind::Search,
        score: 80,
    });

    out.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.title.cmp(&b.title)));
    out.dedup_by(|a, b| a.url == b.url);
    out.truncate(limit);
    out
}

fn recent_defaults(history: &[HistoryEntry], bookmarks: &[Bookmark], limit: usize) -> Vec<Suggestion> {
    let mut out = Vec::new();
    for b in bookmarks.iter().take(limit / 2) {
        out.push(Suggestion { title: b.title.clone(), url: b.url.clone(), kind: SuggestionKind::Bookmark, score: 50 });
    }
    for h in history.iter().take(limit.saturating_sub(out.len())) {
        out.push(Suggestion {
            title: if h.title.is_empty() { h.url.clone() } else { h.title.clone() },
            url: h.url.clone(),
            kind: SuggestionKind::History,
            score: 40 + h.visit_count.min(20),
        });
    }
    out.truncate(limit);
    out
}

fn match_score(q: &str, title: &str, url: &str, base: i64, extra: i64) -> Option<i64> {
    let title_l = title.to_ascii_lowercase();
    let url_l = url.to_ascii_lowercase();
    let mut score = None;
    if title_l.starts_with(q) {
        score = Some(base + 40 + extra);
    } else if url_l.contains(q) {
        score = Some(base + 25 + extra);
    } else if title_l.contains(q) {
        score = Some(base + 15 + extra);
    }
    score
}

fn looks_like_typed_url(q: &str) -> bool {
    if q.contains(' ') {
        return false;
    }
    q.contains('.') || q.starts_with("localhost") || q.contains("://")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn search_suggestion_always_present() {
        let suggestions = suggest("hello world", &[], &[], |q| format!("https://ddg/?q={q}"), 8);
        assert!(suggestions.iter().any(|s| s.kind == SuggestionKind::Search));
    }

    #[test]
    fn bookmark_outranks_weak_history() {
        let bookmarks = vec![Bookmark {
            id: 1,
            title: "Rust Language".into(),
            url: "https://www.rust-lang.org/".into(),
            folder: "Bar".into(),
            position: 0,
            created_at: Utc::now(),
        }];
        let history = vec![HistoryEntry {
            id: 1,
            title: "other".into(),
            url: "https://example.com/rust".into(),
            visit_count: 1,
            last_visit: Utc::now(),
        }];
        let suggestions = suggest("rust", &history, &bookmarks, |q| format!("s?{q}"), 8);
        assert_eq!(suggestions[0].kind, SuggestionKind::Bookmark);
    }
}
