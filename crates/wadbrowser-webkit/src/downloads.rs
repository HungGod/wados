//! Downloads: one handler per network session (Cactus connected one per
//! window, so each download was handled once per window, and closed windows
//! were kept alive by it). A download goes straight to the Downloads folder
//! under the name WebKit suggests (the server's, else the address's), a
//! number added when it's taken; the store keeps a record (not a private
//! window's). The UI hears of each through [`connect`].

use crate::engine::engine;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use webkit6::prelude::*;
use webkit6::{Download, DownloadError, NetworkSession, WebView, glib};

/// Build a non-colliding destination path under the downloads directory.
pub fn unique_download_path(dir: &Path, filename: &str) -> PathBuf {
    let safe = sanitize_filename(filename);
    let mut path = dir.join(&safe);
    if !path.exists() {
        return path;
    }
    let stem = Path::new(&safe).file_stem().and_then(|s| s.to_str()).unwrap_or("download");
    let ext = Path::new(&safe).extension().and_then(|s| s.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
    for i in 1..10_000 {
        path = dir.join(format!("{stem}-{i}{ext}"));
        if !path.exists() {
            break;
        }
    }
    path
}

/// A file name with no folder in it ("../../etc/passwd" → "passwd").
pub fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base.chars().map(|c| if c == '\0' || c.is_control() { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() { "download".into() } else { cleaned }
}

pub fn filename_from_url(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.path_segments().and_then(|mut s| s.next_back()).map(|s| s.to_string()))
        .filter(|s| !s.is_empty())
        .map(|s| sanitize_filename(&percent_decode(&s)))
        .unwrap_or_else(|| "download".into())
}

/// A path as a `file://` address, percent-encoded ("a b#.pdf" → "file:///…/a%20b%23.pdf").
pub fn file_uri_for_path(path: &Path) -> String {
    glib::filename_to_uri(path, None).map(String::from).unwrap_or_else(|_| format!("file://{}", path.display()))
}

/// A destination WebKit gave back (a path, or a `file://` address) as a path.
pub fn path_from_destination(dest: &str) -> PathBuf {
    if dest.starts_with("file://") {
        glib::filename_from_uri(dest).map(|(p, _)| p).unwrap_or_else(|_| PathBuf::from(&dest[7..]))
    } else {
        PathBuf::from(dest)
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What happened to a download, for the UI (a toast, the downloads panel).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadEvent {
    Started { id: u64, name: String, path: PathBuf },
    Finished { id: u64, name: String, path: PathBuf },
    Failed { id: u64, name: String, cancelled: bool, error: String },
}

type Listener = Rc<dyn Fn(Option<&WebView>, &DownloadEvent)>;

thread_local! {
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
    static NEXT: Cell<u64> = const { Cell::new(1) };
    /// The downloads under way, and the views they came from.
    static ACTIVE: RefCell<Vec<(u64, glib::WeakRef<WebView>)>> = const { RefCell::new(Vec::new()) };
}

/// Whether a download from `view` is under way (it doesn't sleep).
pub(crate) fn downloading(view: &WebView) -> bool {
    ACTIVE.with_borrow(|a| a.iter().any(|(_, w)| w.upgrade().as_ref() == Some(view)))
}

fn done(id: u64) {
    ACTIVE.with_borrow_mut(|a| a.retain(|(i, _)| *i != id));
}

/// Called for each download's start and end, with the view it came from (if any).
pub fn connect(f: impl Fn(Option<&WebView>, &DownloadEvent) + 'static) {
    LISTENERS.with_borrow_mut(|l| l.push(Rc::new(f)));
}

fn tell(d: &Download, ev: DownloadEvent) {
    crate::testlog::line("download", &[&format!("{ev:?}")]);
    let view = d.web_view();
    for f in LISTENERS.with_borrow(|l| l.clone()) {
        f(view.as_ref(), &ev);
    }
}

/// Where downloads go: the person's Downloads folder (made if it's missing).
pub fn downloads_dir() -> PathBuf {
    let d = match engine() {
        Some(e) => e.paths.downloads_dir(),
        None => {
            glib::user_special_dir(glib::UserDirectory::Downloads).unwrap_or_else(|| glib::home_dir().join("Downloads"))
        }
    };
    let _ = std::fs::create_dir_all(&d);
    d
}

/// The session's one download handler (`session.rs` connects it as it makes the session).
pub(crate) fn watch(session: &NetworkSession, profile: &str) {
    let keep = !session.is_ephemeral();
    let profile = profile.to_string();
    session.connect_download_started(move |_, d| started(d, keep, &profile));
}

fn started(d: &Download, keep: bool, profile: &str) {
    let id = NEXT.get();
    NEXT.set(id + 1);
    d.set_allow_overwrite(false);
    if let Some(v) = d.web_view() {
        ACTIVE.with_borrow_mut(|a| a.push((id, v.downgrade())));
    }
    let url = d.request().and_then(|r| r.uri()).map(String::from).unwrap_or_default();
    let record: Rc<Cell<Option<i64>>> = Rc::default();
    let name: Rc<RefCell<String>> = Rc::default();
    tracing::info!(%url, profile, "a download");
    {
        let (record, name, url) = (record.clone(), name.clone(), url.clone());
        d.connect_decide_destination(move |d, suggested| {
            let wanted =
                if suggested.trim().is_empty() { filename_from_url(&url) } else { sanitize_filename(suggested) };
            let path = unique_download_path(&downloads_dir(), &wanted);
            let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(wanted);
            // WebKitGTK 6 takes a path here (an absolute one), not a file:// address.
            d.set_destination(&path.to_string_lossy());
            *name.borrow_mut() = file.clone();
            if keep && let Some(e) = engine() {
                record.set(e.store.add_download(&url, &file, &path, "started").ok().map(|r| r.id));
            }
            tell(d, DownloadEvent::Started { id, name: file, path });
            true
        });
    }
    {
        let (record, name) = (record.clone(), name.clone());
        d.connect_finished(move |d| {
            done(id);
            let path = d.destination().map(|p| path_from_destination(&p)).unwrap_or_default();
            if let (Some(rid), Some(e)) = (record.get(), engine()) {
                let _ = e.store.update_download_status(rid, "finished");
                wadbrowser_core::metrics::record_download_completed(&e.store);
            }
            // `finished` comes after `failed` too: only a download that didn't fail is done.
            if record.get() != Some(-1) {
                tell(d, DownloadEvent::Finished { id, name: name.borrow().clone(), path });
            }
        });
    }
    d.connect_failed(move |d, err| {
        done(id);
        let cancelled = err.matches(DownloadError::CancelledByUser);
        if let (Some(rid), Some(e)) = (record.get(), engine()) {
            let _ = e.store.update_download_status(rid, if cancelled { "cancelled" } else { "failed" });
        }
        record.set(Some(-1));
        tell(d, DownloadEvent::Failed { id, name: name.borrow().clone(), cancelled, error: err.message().to_string() });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_path_separators() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("a\\b"), "b");
        assert_eq!(sanitize_filename(".."), "download");
        assert_eq!(sanitize_filename(".bashrc"), "bashrc");
        assert_eq!(sanitize_filename("  "), "download");
    }

    #[test]
    fn filename_from_typical_url() {
        assert_eq!(filename_from_url("https://example.com/files/report.pdf?x=1"), "report.pdf");
        assert_eq!(filename_from_url("https://example.com/files/my%20report.pdf"), "my report.pdf");
        assert_eq!(filename_from_url("https://example.com/"), "download");
    }

    #[test]
    fn file_addresses_are_percent_encoded() {
        let p = Path::new("/tmp/a b#1.pdf");
        let uri = file_uri_for_path(p);
        assert_eq!(uri, "file:///tmp/a%20b%231.pdf");
        assert_eq!(path_from_destination(&uri), p);
        assert_eq!(path_from_destination("/tmp/x"), Path::new("/tmp/x"));
    }

    #[test]
    fn a_taken_name_gets_a_number() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(unique_download_path(d.path(), "a.pdf"), d.path().join("a.pdf"));
        std::fs::write(d.path().join("a.pdf"), "").unwrap();
        assert_eq!(unique_download_path(d.path(), "a.pdf"), d.path().join("a-1.pdf"));
    }
}
