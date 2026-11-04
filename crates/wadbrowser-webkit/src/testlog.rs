//! What the end-to-end checks read back (`tools/wadbrowser-e2e.sh`): with
//! `WADBROWSER_TEST_LOG=<file>`, a line per thing that happened (a page's
//! title, a tab opened or closed, the content filter compiled or loaded…).
//! Without it, nothing.

use std::io::Write;
use std::sync::OnceLock;

pub const ENV: &str = "WADBROWSER_TEST_LOG";

fn file() -> Option<&'static std::path::PathBuf> {
    static FILE: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    FILE.get_or_init(|| std::env::var_os(ENV).filter(|v| !v.is_empty()).map(Into::into)).as_ref()
}

/// Whether there's a test log.
pub fn on() -> bool {
    file().is_some()
}

/// One line: `event` and its words, tab-separated (newlines in them become spaces).
pub fn line(event: &str, words: &[&str]) {
    let Some(path) = file() else { return };
    let mut text = event.to_string();
    for w in words {
        text.push('\t');
        text.push_str(&w.replace(['\n', '\t'], " "));
    }
    text.push('\n');
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(text.as_bytes());
    }
}
