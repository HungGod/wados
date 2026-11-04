//! The `cactus://` scheme: WadBrowser's own pages, served by WebKit from a
//! callback the UI gives ([`set_page_source`]), in place of Cactus's
//! `load_html` with a made-up base address. It's registered once, on the
//! web context every view shares, as secure (the pages are like https: no
//! mixed-content warnings) and local (a web page can't link to or load
//! them; only the browser opens them).
//!
//! `cactus://res/<file>` is the pages' shared files (wadbrowser-core's
//! `internal::resource`), with the fonts read from where WadOS installs
//! them (`/usr/share/wados/assets/fonts`) when they're there.
//!
//! A page that couldn't load is shown in place of it (its address stays the
//! one that failed, so Reload tries it again), drawn by the same callback
//! from `cactus://error?…` ([`show_error`]); its base is `cactus://error/`,
//! so it can use the shared files. A page the social blocker stopped is
//! `cactus://blocked?site=…&uri=…` ([`show_blocked`]), at that address, so
//! its button's message reaches the browser.

use std::cell::RefCell;
use std::rc::Rc;
use webkit6::prelude::*;
use webkit6::{URISchemeRequest, WebContext, WebView, gio, glib};

pub const SCHEME: &str = "cactus";
/// The base address of a page shown in place of one that failed.
pub const ERROR_BASE: &str = "cactus://error/";

/// What a `cactus://` page is asked for.
pub struct PageRequest<'a> {
    /// The whole address ("cactus://history?q=rust").
    pub uri: &'a str,
    /// The view it's for (none for one WebKit asks for itself).
    pub view: Option<&'a WebView>,
}

/// A page (or file) to answer with.
pub struct Page {
    pub bytes: Vec<u8>,
    pub mime: String,
}

impl Page {
    pub fn html(html: impl Into<String>) -> Self {
        Self { bytes: html.into().into_bytes(), mime: "text/html".into() }
    }

    pub fn new(bytes: impl Into<Vec<u8>>, mime: impl Into<String>) -> Self {
        Self { bytes: bytes.into(), mime: mime.into() }
    }
}

type Source = Rc<dyn Fn(&PageRequest) -> Option<Page>>;

thread_local! {
    static SOURCE: RefCell<Option<Source>> = const { RefCell::new(None) };
    static REGISTERED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The UI's pages: `cactus://<page>` (and `cactus://error?…`,
/// `cactus://blocked?…`) → the page, or None for one there isn't
/// (WebKit shows its "not found"). Called on the main thread, as WebKit asks.
pub fn set_page_source(f: impl Fn(&PageRequest) -> Option<Page> + 'static) {
    SOURCE.set(Some(Rc::new(f)));
    register();
}

/// Registers the scheme on the shared web context (once).
pub fn register() {
    if REGISTERED.replace(true) {
        return;
    }
    let Some(context) = WebContext::default() else {
        tracing::warn!("no web context: cactus:// pages won't load");
        return;
    };
    context.register_uri_scheme(SCHEME, serve);
    if let Some(sec) = context.security_manager() {
        sec.register_uri_scheme_as_secure(SCHEME);
        sec.register_uri_scheme_as_local(SCHEME);
        // The pages, their stylesheet and its fonts are on different
        // cactus:// hosts, and @font-face needs CORS.
        sec.register_uri_scheme_as_cors_enabled(SCHEME);
    }
}

fn serve(req: &URISchemeRequest) {
    let uri = req.uri().map(String::from).unwrap_or_default();
    let page = if let Some(file) = uri.strip_prefix(wadbrowser_core::internal::RES) {
        resource(file)
    } else {
        let view = req.web_view();
        let source = SOURCE.with_borrow(|s| s.clone());
        source.and_then(|f| f(&PageRequest { uri: &uri, view: view.as_ref() }))
    };
    match page {
        Some(p) => {
            let bytes = glib::Bytes::from_owned(p.bytes);
            let len = bytes.len() as i64;
            req.finish(&gio::MemoryInputStream::from_bytes(&bytes), len, Some(&p.mime));
        }
        None => {
            let mut e = glib::Error::new(gio::IOErrorEnum::NotFound, &format!("There's no page {uri}"));
            req.finish_error(&mut e);
        }
    }
}

/// A shared file: a font from where WadOS installs them, else the pages'.
fn resource(file: &str) -> Option<Page> {
    let file = file.split(['?', '#']).next().unwrap_or_default().trim_start_matches('/');
    if let Some(font) = file.strip_prefix("fonts/").filter(|f| !f.contains('/') && !f.starts_with('.'))
        && let Some(bytes) = font_dirs().into_iter().find_map(|d| std::fs::read(d.join(font)).ok())
    {
        return Some(Page::new(bytes, "font/ttf"));
    }
    wadbrowser_core::internal::resource(file).map(|(bytes, mime)| Page::new(bytes, mime))
}

/// Where the fonts are: as WadOS installs them, then the repo's (wad-gtk's `fonts.rs`).
fn font_dirs() -> Vec<std::path::PathBuf> {
    vec![
        "/usr/share/wados/assets/fonts".into(),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts"),
    ]
}

/// A `cactus://` address with its query ("cactus://error?title=…").
pub fn page_uri(page: &str, query: &[(&str, &str)]) -> String {
    let q: Vec<String> = query.iter().map(|(k, v)| format!("{k}={}", encode(v))).collect();
    if q.is_empty() { format!("cactus://{page}") } else { format!("cactus://{page}?{}", q.join("&")) }
}

/// A query's value ("a%20b" → "a b").
pub fn query_value(uri: &str, key: &str) -> Option<String> {
    let q = uri.split_once('?')?.1.split('#').next()?;
    q.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (decode(k) == key).then(|| decode(v))
    })
}

/// In place of `failed_uri`'s page: the UI's `cactus://error?title=&message=&uri=`.
pub fn show_error(view: &WebView, failed_uri: &str, title: &str, message: &str) {
    let uri = page_uri("error", &[("title", title), ("message", message), ("uri", failed_uri)]);
    show_in_place(view, failed_uri, &uri);
}

/// A page the social blocker stopped: `cactus://blocked?site=&uri=`, loaded
/// at that address (not in the page's place) so its button can ask the
/// browser to let the site through; `uri` is the page it stopped.
pub fn show_blocked(view: &WebView, failed_uri: &str, site: &str) {
    let uri = page_uri("blocked", &[("site", site), ("uri", failed_uri)]);
    view.load_uri(&uri);
}

fn show_in_place(view: &WebView, failed_uri: &str, page: &str) {
    let source = SOURCE.with_borrow(|s| s.clone());
    let html = source
        .and_then(|f| f(&PageRequest { uri: page, view: Some(view) }))
        .map(|p| String::from_utf8_lossy(&p.bytes).into_owned())
        .unwrap_or_else(|| fallback(page));
    view.load_alternate_html(&html, failed_uri, Some(ERROR_BASE));
}

fn fallback(page: &str) -> String {
    let title = query_value(page, "title").unwrap_or_else(|| "This page didn't load".into());
    let message = query_value(page, "message").unwrap_or_default();
    format!(
        "<!doctype html><meta charset=utf-8><title>{t}</title><body style=\"background:#0a0614;color:#f5f2fa;font:15px system-ui;margin:18vh auto;max-width:520px\"><h1>{t}</h1><p>{m}</p>",
        t = esc(&title),
        m = esc(&message)
    )
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(s: &str) -> String {
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
            out.push(if b[i] == b'+' { b' ' } else { b[i] });
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_with_queries_both_ways() {
        let u = page_uri("error", &[("title", "No page"), ("uri", "https://x.test/a?b=1&c")]);
        assert_eq!(u, "cactus://error?title=No%20page&uri=https%3A%2F%2Fx.test%2Fa%3Fb%3D1%26c");
        assert_eq!(query_value(&u, "uri").as_deref(), Some("https://x.test/a?b=1&c"));
        assert_eq!(query_value(&u, "title").as_deref(), Some("No page"));
        assert_eq!(query_value(&u, "none"), None);
        assert_eq!(query_value("cactus://history?q=caf%C3%A9+au+lait", "q").as_deref(), Some("café au lait"));
        assert_eq!(query_value("cactus://x?q=100%", "q").as_deref(), Some("100%"));
        assert_eq!(page_uri("newtab", &[]), "cactus://newtab");
    }

    #[test]
    fn the_shared_files_and_the_fonts() {
        let css = resource("pages.css").expect("the stylesheet");
        assert_eq!(css.mime, "text/css");
        let font = resource("fonts/Inter-Variable.ttf").expect("a font");
        assert_eq!(font.mime, "font/ttf");
        assert!(font.bytes.len() > 10_000);
        assert!(resource("fonts/../../etc/passwd").is_none());
        assert!(resource("nothing.css").is_none());
    }
}
