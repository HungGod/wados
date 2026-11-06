//! Open with's memory on disk: the app that last opened each kind of file
//! ([`wad_model::open_with::Remembered`]), the session's, at
//! `~/.config/wados/open-with.json` (the shell and Files both read and write it).

use std::path::PathBuf;

use gtk::glib;
use wad_model::AppId;
use wad_model::open_with::Remembered;

pub fn path() -> PathBuf {
    glib::user_config_dir().join("wados").join("open-with.json")
}

/// What's remembered (nothing, if it's not there or not readable).
pub fn load() -> Remembered {
    std::fs::read_to_string(path()).map(|s| Remembered::parse(&s)).unwrap_or_default()
}

/// `app` opened a file of `mime`: remembered for the next.
pub fn remember(mime: &str, app: AppId) {
    let mut r = load();
    if r.get(mime) == Some(app) {
        return;
    }
    r.set(mime, app);
    let p = path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = p.with_extension("json.new");
    if let Err(e) = std::fs::write(&tmp, r.to_json()).and_then(|()| std::fs::rename(&tmp, &p)) {
        tracing::warn!("{}: {e}", p.display());
    }
}
