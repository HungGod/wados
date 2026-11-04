//! Built-in search engine catalog.

use crate::prefs::SearchEngine;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEngineInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub url_template: &'static str,
}

pub const BUILTIN_ENGINES: &[SearchEngineInfo] = &[
    SearchEngineInfo { id: "duckduckgo", name: "DuckDuckGo", url_template: "https://duckduckgo.com/?q={searchTerms}" },
    SearchEngineInfo {
        id: "startpage",
        name: "Startpage",
        url_template: "https://www.startpage.com/sp/search?query={searchTerms}",
    },
    SearchEngineInfo {
        id: "wikipedia",
        name: "Wikipedia",
        url_template: "https://en.wikipedia.org/w/index.php?search={searchTerms}",
    },
    SearchEngineInfo {
        id: "brave",
        name: "Brave Search",
        url_template: "https://search.brave.com/search?q={searchTerms}",
    },
    SearchEngineInfo { id: "mojeek", name: "Mojeek", url_template: "https://www.mojeek.com/search?q={searchTerms}" },
];

pub fn engine_from_id(id: &str) -> Option<SearchEngine> {
    match id {
        "duckduckgo" => Some(SearchEngine::DuckDuckGo),
        "startpage" => Some(SearchEngine::Startpage),
        "wikipedia" => Some(SearchEngine::Wikipedia),
        "brave" => Some(SearchEngine::Custom {
            name: "Brave Search".into(),
            url_template: "https://search.brave.com/search?q={searchTerms}".into(),
        }),
        "mojeek" => Some(SearchEngine::Custom {
            name: "Mojeek".into(),
            url_template: "https://www.mojeek.com/search?q={searchTerms}".into(),
        }),
        _ => None,
    }
}

pub fn id_for_engine(engine: &SearchEngine) -> String {
    match engine {
        SearchEngine::DuckDuckGo => "duckduckgo".into(),
        SearchEngine::Startpage => "startpage".into(),
        SearchEngine::Wikipedia => "wikipedia".into(),
        SearchEngine::Custom { name, .. } => {
            let lower = name.to_ascii_lowercase();
            if lower.contains("brave") {
                "brave".into()
            } else if lower.contains("mojeek") {
                "mojeek".into()
            } else {
                "custom".into()
            }
        }
    }
}

pub fn build_search_url(template: &str, query: &str) -> String {
    let encoded = crate::prefs::encode_query(query);
    template.replace("{searchTerms}", &encoded).replace("%s", &encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_ids_roundtrip() {
        for info in BUILTIN_ENGINES {
            assert!(engine_from_id(info.id).is_some(), "{}", info.id);
        }
    }
}
