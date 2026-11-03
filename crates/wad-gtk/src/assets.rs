//! WadSpaces' pictures and marks: installed under /usr/share/wados/assets on
//! the machine, or the repo's assets/ on a laptop's build.

use std::path::PathBuf;

pub fn path(rel: &str) -> Option<PathBuf> {
    if rel.split('/').any(|p| p == "..") {
        return None;
    }
    let dirs =
        [PathBuf::from("/usr/share/wados/assets"), PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"))];
    dirs.into_iter().map(|d| d.join(rel)).find(|p| p.exists())
}
