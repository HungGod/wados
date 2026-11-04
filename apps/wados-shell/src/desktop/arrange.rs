//! Where the icons were put (the spike's `machine.desk`, `desk.arrange`),
//! kept in `~/.config/wados/desk.json`: a column and row per icon, by key
//! (`ws:<id>`, `folder:<tag>`, `file:<name>`, `machines`, `trash`).
//!
//!     { "ws:wados": { "c": 0, "r": 0 }, "trash": { "c": 2, "r": 0 } }
//!
//! Written whole each time (to a temporary file, then renamed over it), so
//! it's never half there. Ones it doesn't have fill the free spots.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::grid::{Spot, Spots};

/// `~/.config/wados/desk.json`.
pub fn path() -> PathBuf {
    gtk::glib::user_config_dir().join("wados").join("desk.json")
}

/// The spots in `text` (bad entries are skipped; a bad file is none).
pub fn decode(text: &str) -> Spots {
    let Ok(Value::Object(m)) = serde_json::from_str::<Value>(text) else { return Spots::new() };
    m.into_iter()
        .filter_map(|(k, v)| {
            let c = v.get("c")?.as_u64()?;
            let r = v.get("r")?.as_u64()?;
            Some((k, Spot::new(u32::try_from(c).ok()?, u32::try_from(r).ok()?)))
        })
        .collect()
}

pub fn encode(spots: &Spots) -> String {
    let m: Map<String, Value> = spots.iter().map(|(k, p)| (k.clone(), json!({ "c": p.c, "r": p.r }))).collect();
    let mut s = serde_json::to_string_pretty(&Value::Object(m)).unwrap_or_else(|_| "{}".into());
    s.push('\n');
    s
}

pub fn load(path: &Path) -> Spots {
    std::fs::read_to_string(path).map(|t| decode(&t)).unwrap_or_default()
}

/// Writes them atomically: a temporary file beside it, then renamed over it.
pub fn save(path: &Path, spots: &Spots) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("desk"),
        std::process::id()
    ));
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(encode(spots).as_bytes())?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut s = Spots::new();
        s.insert("ws:wados".into(), Spot::new(0, 0));
        s.insert("folder:studio".into(), Spot::new(0, 1));
        s.insert("file:Pitch deck \"v2\".pdf".into(), Spot::new(3, 4));
        s.insert("machines".into(), Spot::new(1, 5));
        s.insert("trash".into(), Spot::new(2, 0));
        let back = decode(&encode(&s));
        assert_eq!(back, s);
        // In the order they were.
        assert_eq!(back.keys().collect::<Vec<_>>(), s.keys().collect::<Vec<_>>());
    }

    #[test]
    fn bad_entries_and_files_are_skipped() {
        let s = decode(r#"{"ws:a": {"c": 1, "r": 2}, "ws:b": {"c": -1, "r": 0}, "ws:c": "x", "ws:d": {"c": 1}}"#);
        assert_eq!(s.len(), 1);
        assert_eq!(s["ws:a"], Spot::new(1, 2));
        assert!(decode("not json").is_empty());
        assert!(decode("[1, 2]").is_empty());
    }

    #[test]
    fn saved_atomically_and_read_back() {
        let dir = std::env::temp_dir().join(format!("wados-desk-test-{}", std::process::id()));
        let p = dir.join("wados").join("desk.json");
        let mut s = Spots::new();
        s.insert("trash".into(), Spot::new(4, 2));
        save(&p, &s).unwrap();
        assert_eq!(load(&p), s);
        s.insert("machines".into(), Spot::new(4, 1));
        save(&p, &s).unwrap();
        assert_eq!(load(&p), s);
        // Nothing left beside it.
        let left: Vec<_> = std::fs::read_dir(p.parent().unwrap()).unwrap().collect();
        assert_eq!(left.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(load(&p).is_empty());
    }
}
