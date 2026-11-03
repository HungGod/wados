// What the spike's own logic made of its populated sample
// (tests/data/parity.json: every live wadspace's chips, suggestions and
// folders, and two Alt+Space searches, from the spike's chipsOf,
// suggestions, foldersOf and search), made again here from the flattened
// sample. The two have to agree.

mod common;

use serde_json::Value;
use wad_model::chips::chips_of;
use wad_model::folders::folders_of;
use wad_model::suggest::suggestions;
use wad_model::variants::search;

fn strings(v: &Value) -> Vec<String> {
    v.as_array().expect("a list").iter().map(|s| s.as_str().expect("a string").to_string()).collect()
}

#[test]
fn chips_suggestions_and_folders_are_the_spikes() {
    let lib = common::library();
    let facts: Value = serde_json::from_str(include_str!("data/parity.json")).expect("tests/data/parity.json");
    let mut seen = 0;
    for x in lib.live() {
        let f = &facts[&x.id];
        let chips: Vec<String> = chips_of(x)
            .iter()
            .map(|c| {
                format!(
                    "{}:{}{}",
                    serde_json::to_value(c.kind).expect("a kind").as_str().unwrap_or(""),
                    c.id,
                    if c.launch { "*" } else { "" }
                )
            })
            .collect();
        assert_eq!(chips, strings(&f["chips"]), "{}'s chips", x.id);
        let sugg: Vec<String> = suggestions(x, 3)
            .iter()
            .map(|s| match &s.why {
                Some(why) => format!("{} ({why})", s.item),
                None => s.item.clone(),
            })
            .collect();
        assert_eq!(sugg, strings(&f["sugg"]), "{}'s suggestions", x.id);
        let folders: Vec<Value> = x
            .apps
            .iter()
            .map(|a| {
                let fs: Vec<Value> = folders_of(x, a.app)
                    .iter()
                    .map(|fo| {
                        serde_json::json!([
                            fo.home,
                            fo.key.as_ref().map(|k| k.name.clone()),
                            fo.items.iter().map(|v| v.id.clone()).collect::<Vec<_>>()
                        ])
                    })
                    .collect();
                serde_json::json!([a.app.as_str(), fs])
            })
            .collect();
        assert_eq!(Value::Array(folders), f["folders"], "{}'s folders", x.id);
        seen += 1;
    }
    assert_eq!(seen, 10);
}

#[test]
fn alt_space_finds_what_the_spikes_did() {
    let lib = common::library();
    let facts: Value = serde_json::from_str(include_str!("data/parity.json")).expect("tests/data/parity.json");
    for (q, key) in [("", "search:"), ("git", "search:git")] {
        let found: Vec<String> = search(&lib.wadspaces, common::HERE, q, 40)
            .iter()
            .map(|f| format!("{}/{}", f.ws.id, f.variant.id))
            .collect();
        assert_eq!(found, strings(&facts[key]), "{q:?}");
    }
}

#[test]
fn the_sample_is_a_good_library() {
    assert_eq!(wad_model::check::library(&common::library()), Ok(()));
}
