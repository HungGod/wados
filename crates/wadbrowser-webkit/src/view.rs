//! A tab's web view: in its profile's network session, with a
//! UserContentManager of its own (Cactus shared one per session and added
//! its scripts again for every tab). Each view's has:
//!
//! - the Do Not Track / Global Privacy Control script, once;
//! - the content filters for its page's site (`filters`);
//! - the cosmetic CSS for its current page only, replaced on each navigation
//!   (Cactus piled every site's rules onto every later page);
//! - the shield's counter: a script in a world of its own that tells the
//!   browser which of the page's loads failed, of which those the lists block
//!   are counted, once each (WebKit's filters block in the web process and
//!   say nothing); frames the lists block are counted at their navigation;
//! - the "cactus" message handler, while it shows a `cactus://` page.
//!
//! Its signals: popups keep `window.opener` (a view related to their
//! opener's, so in its session and web process), a page that closes itself
//! closes its tab, a top-level page on a key's site moves into the key's
//! session, and only real failures get an error page.

use crate::engine::{engine, hooks, later};
use crate::session::{SessionBundle, signins};
use crate::{filters, permissions, scheme, testlog, zoom};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Instant;
use wadbrowser_core::Preferences;
use wadbrowser_core::profiles::profile_for;
use wadbrowser_privacy::social_site;
use webkit6::prelude::*;
use webkit6::{
    HardwareAccelerationPolicy, LoadEvent, NavigationPolicyDecision, NavigationType, NetworkError, PolicyDecisionType,
    PolicyError, ResponsePolicyDecision, Settings as WebSettings, URIRequest, UserContentInjectedFrames,
    UserContentManager, UserScript, UserScriptInjectionTime, UserStyleLevel, UserStyleSheet,
    WebProcessTerminationReason, WebView, gio, glib,
};

/// The shield's counter's world, and its message handler.
const SHIELD_WORLD: &str = "wadbrowser";
const SHIELD_HANDLER: &str = "wadbrowserBlocked";
/// `cactus://` pages' message handler (wadbrowser-core's `internal::PageAction`).
pub const PAGE_HANDLER: &str = "cactus";
const STATE: &str = "wadbrowser-view";

/// Tells the browser each load on the page that failed (WebKit's content
/// filter blocking it is a failure): elements' `error` events, which a
/// listener in this world gets without the page being able to touch it.
const SHIELD_JS: &str = r#"(function () {
  if (window.__wadbrowserShield) return;
  window.__wadbrowserShield = true;
  var seen = {};
  addEventListener('error', function (e) {
    var t = e.target;
    if (!t || t === window || !t.tagName) return;
    var url = t.currentSrc || t.src || t.href || '';
    if (!url || seen[url] || !/^https?:/.test(url)) return;
    seen[url] = true;
    try {
      webkit.messageHandlers.wadbrowserBlocked.postMessage({ url: String(url), tag: t.tagName.toLowerCase(), page: String(location.href) });
    } catch (_) {}
  }, true);
})();"#;

/// What a view is to the engine.
pub struct ViewState {
    /// Its profile ("default", "signin-<key>", "private-3"…).
    pub profile: String,
    pub private: bool,
    pub(crate) ucm: UserContentManager,
    pub(crate) applied: Cell<filters::Applied>,
    blocked: Cell<u64>,
    seen: RefCell<HashSet<String>>,
    /// The page the count is of.
    page: RefCell<String>,
    css: RefCell<Option<UserStyleSheet>>,
    page_handler: Cell<bool>,
    dnt: Cell<bool>,
    pub(crate) asleep: Cell<bool>,
    pub(crate) last_seen: Cell<Instant>,
    /// It opened a popup, or is one: they share a web process, so neither sleeps.
    pub(crate) related: Cell<bool>,
}

impl ViewState {
    fn new(profile: &str, private: bool, ucm: UserContentManager) -> Self {
        Self {
            profile: profile.to_string(),
            private,
            ucm,
            applied: Cell::default(),
            blocked: Cell::new(0),
            seen: RefCell::default(),
            page: RefCell::default(),
            css: RefCell::default(),
            page_handler: Cell::new(false),
            dnt: Cell::new(false),
            asleep: Cell::new(false),
            last_seen: Cell::new(Instant::now()),
            related: Cell::new(false),
        }
    }

    /// What the shield counted on the page it shows.
    pub fn blocked(&self) -> u64 {
        self.blocked.get()
    }

    /// Its top-level page's address (the one it's going to, while it goes).
    pub(crate) fn page_uri(&self, view: &WebView) -> String {
        view.uri().map(String::from).unwrap_or_else(|| self.page.borrow().clone())
    }

    fn new_page(&self, uri: &str) {
        if *self.page.borrow() != uri {
            *self.page.borrow_mut() = uri.to_string();
            self.seen.borrow_mut().clear();
            self.blocked.set(0);
        }
    }
}

thread_local! {
    static VIEWS: RefCell<Vec<glib::WeakRef<WebView>>> = const { RefCell::new(Vec::new()) };
}

/// The engine's state of `view` (any view `create_web_view` made has one).
pub fn state(view: &WebView) -> Option<Rc<ViewState>> {
    // SAFETY: only `setup` sets this key, and always to an `Rc<ViewState>`.
    unsafe { view.data::<Rc<ViewState>>(STATE).map(|p| p.as_ref().clone()) }
}

/// Every view that's still there.
pub fn live_views() -> Vec<WebView> {
    VIEWS.with_borrow_mut(|all| {
        all.retain(|w| w.upgrade().is_some());
        all.iter().filter_map(|w| w.upgrade()).collect()
    })
}

fn settings(prefs: &Preferences) -> WebSettings {
    let s = WebSettings::new();
    s.set_enable_developer_extras(true);
    s.set_enable_smooth_scrolling(prefs.enable_smooth_scrolling);
    s.set_enable_back_forward_navigation_gestures(true);
    // Sign-in popups (OAuth) often open after an await, past the click's
    // gesture: allowed, as v1 did. The lists block the pop-under networks.
    s.set_javascript_can_open_windows_automatically(true);
    s.set_allow_modal_dialogs(false);
    s.set_enable_page_cache(true);
    s.set_enable_javascript(prefs.enable_javascript);
    s.set_enable_media_stream(true);
    s.set_enable_webrtc(true);
    s.set_enable_encrypted_media(true);
    s.set_media_playback_requires_user_gesture(false);
    if !prefs.hardware_acceleration {
        s.set_hardware_acceleration_policy(HardwareAccelerationPolicy::Never);
    }
    s
}

/// GTK's font DPI, if it has none: without a settings portal (GDK's
/// `no-portals`, a wadspace with no portal) `gtk-xft-dpi` is -1, and
/// WebKitGTK then gives every view made after the first a zoom level of NaN:
/// its pages draw hugely scaled (their background, their text off screen).
/// 96 dpi, as GTK's own default, before a view is made.
fn sane_dpi() {
    if let Some(s) = gtk::Settings::default()
        && s.gtk_xft_dpi() <= 0
    {
        tracing::info!("GTK has no font DPI (no settings portal?): 96 for WebKit's views");
        s.set_gtk_xft_dpi(96 * 1024);
    }
}

/// A tab's view, in `session`'s profile.
pub fn create_web_view(session: &SessionBundle, prefs: &Preferences) -> WebView {
    sane_dpi();
    let ucm = UserContentManager::new();
    let view = WebView::builder()
        .network_session(&session.network)
        .user_content_manager(&ucm)
        .settings(&settings(prefs))
        .hexpand(true)
        .vexpand(true)
        .build();
    setup(&view, ViewState::new(&session.profile, session.private, ucm), prefs);
    view
}

/// A popup's view: related to its opener's (its session, its web process,
/// and `window.opener`), with a UserContentManager of its own.
pub fn create_related_view(opener: &WebView, prefs: &Preferences) -> WebView {
    sane_dpi();
    let ucm = UserContentManager::new();
    let view = WebView::builder()
        .related_view(opener)
        .user_content_manager(&ucm)
        .settings(&settings(prefs))
        .hexpand(true)
        .vexpand(true)
        .build();
    let (profile, private) = match state(opener) {
        Some(o) => {
            o.related.set(true);
            (o.profile.clone(), o.private)
        }
        None => ("default".to_string(), false),
    };
    let st = ViewState::new(&profile, private, ucm);
    st.related.set(true);
    setup(&view, st, prefs);
    view
}

fn setup(view: &WebView, st: ViewState, prefs: &Preferences) {
    let st = Rc::new(st);
    // SAFETY: the key is this module's, read back only as `Rc<ViewState>` (`state`).
    unsafe { view.set_data(STATE, st.clone()) };
    VIEWS.with_borrow_mut(|all| all.push(view.downgrade()));

    apply_privacy_headers(view, prefs);
    let shield = UserScript::for_world(
        SHIELD_JS,
        UserContentInjectedFrames::AllFrames,
        UserScriptInjectionTime::Start,
        SHIELD_WORLD,
        &[],
        &[],
    );
    st.ucm.add_script(&shield);
    st.ucm.register_script_message_handler(SHIELD_HANDLER, Some(SHIELD_WORLD));
    let weak = view.downgrade();
    st.ucm.connect_script_message_received(Some(SHIELD_HANDLER), move |_, value| {
        let Some(view) = weak.upgrade() else { return };
        let Some(json) = value.to_json(0) else { return };
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(&json) else { return };
        let url = msg["url"].as_str().unwrap_or_default();
        let kind = match msg["tag"].as_str().unwrap_or_default() {
            "img" | "image" | "picture" | "input" => "image",
            "script" => "script",
            "link" => "stylesheet",
            "iframe" | "frame" => "sub_frame",
            "video" | "audio" | "source" | "track" => "media",
            "object" | "embed" => "object",
            _ => "other",
        };
        count_if_blocked(&view, url, kind);
    });
    let weak = view.downgrade();
    st.ucm.connect_script_message_received(Some(PAGE_HANDLER), move |_, value| {
        let Some(view) = weak.upgrade() else { return };
        // Only the browser's own pages have the handler; check again all the same.
        if !view.uri().is_some_and(|u| u.starts_with("cactus://")) {
            return;
        }
        let json = if value.is_string() {
            value.to_str().to_string()
        } else {
            value.to_json(0).map(String::from).unwrap_or_default()
        };
        let v = view.clone();
        later(move |h| (h.page_action)(&v, &json));
    });

    view.connect_decide_policy(decide_policy);
    view.connect_load_changed(load_changed);
    view.connect_load_failed(load_failed);
    view.connect_web_process_terminated(crashed);
    view.connect_permission_request(permissions::request);
    view.connect_close(|v| {
        testlog::line("page-closed", &[&profile_of(v), &v.uri().unwrap_or_default()]);
        let v = v.clone();
        later(move |h| (h.close_view)(&v));
    });
    view.connect_create(|opener, _action| {
        hooks()?;
        let prefs = engine().map(|e| e.prefs.borrow().clone()).unwrap_or_default();
        let popup = create_related_view(opener, &prefs);
        let opener = opener.downgrade();
        popup.connect_ready_to_show(move |p| {
            let Some(o) = opener.upgrade() else { return };
            let p = p.clone();
            testlog::line("popup", &[&profile_of(&p), &p.uri().unwrap_or_default()]);
            later(move |h| (h.open_popup)(&o, &p));
        });
        Some(popup.upcast())
    });
    if testlog::on() {
        view.connect_title_notify(|v| {
            if let Some(t) = v.title().filter(|t| !t.is_empty()) {
                testlog::line("title", &[&profile_of(v), &t, &v.uri().unwrap_or_default()]);
            }
        });
    }
}

fn profile_of(view: &WebView) -> String {
    state(view).map(|s| s.profile.clone()).unwrap_or_default()
}

/// The DNT and GPC script, if the settings send them (once per view).
pub fn apply_privacy_headers(view: &WebView, prefs: &Preferences) {
    let Some(st) = state(view) else { return };
    if st.dnt.get() || !(prefs.send_dnt || prefs.send_gpc) {
        return;
    }
    let mut js = String::from("(function(){");
    if prefs.send_dnt {
        js.push_str("try{Object.defineProperty(Navigator.prototype,'doNotTrack',{get:function(){return '1';},configurable:true});}catch(e){}");
    }
    if prefs.send_gpc {
        js.push_str("try{Object.defineProperty(Navigator.prototype,'globalPrivacyControl',{get:function(){return true;},configurable:true});}catch(e){}");
    }
    js.push_str("})();");
    let script = UserScript::new(&js, UserContentInjectedFrames::AllFrames, UserScriptInjectionTime::Start, &[], &[]);
    st.ucm.add_script(&script);
    st.dnt.set(true);
}

/// A script of the window's for this view (a sign-in window's): `js` at the
/// start or end of each page (`at_start`), in every frame or the top one;
/// only on pages matching `allow` (WebKit's URL patterns, "https://github.com/*"),
/// or all when it's empty.
pub fn add_user_script(view: &WebView, js: &str, at_start: bool, all_frames: bool, allow: &[&str]) {
    let Some(st) = state(view) else { return };
    let script = UserScript::new(
        js,
        if all_frames { UserContentInjectedFrames::AllFrames } else { UserContentInjectedFrames::TopFrame },
        if at_start { UserScriptInjectionTime::Start } else { UserScriptInjectionTime::End },
        allow,
        &[],
    );
    st.ucm.add_script(&script);
}

/// The page's cosmetic CSS, in place of the last page's.
pub fn inject_cosmetic_css(view: &WebView, css: &str) {
    let Some(st) = state(view) else { return };
    if let Some(old) = st.css.borrow_mut().take() {
        st.ucm.remove_style_sheet(&old);
    }
    if css.trim().is_empty() {
        return;
    }
    let sheet = UserStyleSheet::new(css, UserContentInjectedFrames::AllFrames, UserStyleLevel::User, &[], &[]);
    st.ucm.add_style_sheet(&sheet);
    *st.css.borrow_mut() = Some(sheet);
}

pub fn load_html(view: &WebView, html: &str, base_uri: &str) {
    view.load_html(html, Some(base_uri));
}

/// Loads `uri` as the person asked for it (the address bar, a bookmark, the
/// command line): with `DNT: 1` and `Sec-GPC: 1` when the settings send
/// them. (WebKitGTK only lets the browser set the headers of loads it starts
/// itself: a page's own links and what it loads send the JS ones only.)
pub fn navigate(view: &WebView, uri: &str) {
    if let Some(st) = state(view) {
        prepare_for(view, &st, uri);
    }
    let prefs = engine().map(|e| e.prefs.borrow().clone()).unwrap_or_default();
    let web = uri.starts_with("http://") || uri.starts_with("https://");
    if web && (prefs.send_dnt || prefs.send_gpc) {
        let req = URIRequest::new(uri);
        if let Some(h) = req.http_headers() {
            if prefs.send_dnt {
                h.append("DNT", "1");
            }
            if prefs.send_gpc {
                h.append("Sec-GPC", "1");
            }
            view.load_request(&req);
            return;
        }
    }
    view.load_uri(uri);
}

/// The profile a top-level page at `uri` should be in, if not `profile`:
/// a key's whose sites have its host (in a wadspace). None: it stays.
pub fn profile_for_uri(profile: &str, uri: &str) -> Option<String> {
    if !(uri.starts_with("http://") || uri.starts_with("https://")) {
        return None;
    }
    let host = wadbrowser_core::site::host_of(uri);
    // HOOK(keys-on-machine): on the machine there's no signins.json; a key's
    // sites come from wadd's library (wadbrowser_core::wadd::Wadd::web_keys),
    // but its folder is root's: using its cookies here is phase 6's.
    profile_for(&host, profile, &signins().site_keys())
}

/// Before a top-level page starts loading in `view`: its filters, the
/// "cactus" handler if it's one of ours.
fn prepare_for(view: &WebView, st: &ViewState, uri: &str) {
    filters::apply(view, st, uri);
    let ours = uri.starts_with("cactus://");
    if ours != st.page_handler.get() {
        if ours {
            st.ucm.register_script_message_handler(PAGE_HANDLER, None);
        } else {
            st.ucm.unregister_script_message_handler(PAGE_HANDLER, None);
        }
        st.page_handler.set(ours);
    }
}

/// Counts `url` (loaded by `view`'s page as `kind`) if a filter blocked it:
/// once per page and address.
fn count_if_blocked(view: &WebView, url: &str, kind: &str) {
    let (Some(e), Some(st)) = (engine(), state(view)) else { return };
    let page = st.page_uri(view);
    st.new_page(&page);
    if url.is_empty() || url == page || !st.seen.borrow_mut().insert(url.to_string()) {
        return;
    }
    let applied = st.applied.get();
    let lists = applied.lists && e.privacy.should_block(url, &page, kind).matched;
    let social = applied.social && social_site(&wadbrowser_core::site::host_of(url)).is_some();
    if !(lists || social) {
        return;
    }
    st.blocked.set(st.blocked.get() + 1);
    e.privacy.record_blocked(1);
    e.unsaved_blocks.set(e.unsaved_blocks.get() + 1);
    testlog::line("blocked", &[&st.profile, &page, url, &st.blocked.get().to_string()]);
    crate::shield::changed(Some(view));
}

fn web_scheme(scheme: &str) -> bool {
    matches!(scheme, "http" | "https" | "about" | "data" | "blob" | "file" | "javascript" | "webkit" | "cactus")
}

fn decide_policy(view: &WebView, decision: &webkit6::PolicyDecision, kind: PolicyDecisionType) -> bool {
    match kind {
        PolicyDecisionType::NavigationAction | PolicyDecisionType::NewWindowAction => {
            let Some(nav) = decision.downcast_ref::<NavigationPolicyDecision>() else { return false };
            let Some(action) = nav.navigation_action() else { return false };
            let Some(uri) = action.request().and_then(|r| r.uri()).map(String::from) else { return false };
            let scheme = uri.split(':').next().unwrap_or_default().to_ascii_lowercase();
            // Links to other apps (mailto:, zoommtg:…) go to them.
            if !web_scheme(&scheme) {
                decision.ignore();
                if let Err(e) = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>) {
                    tracing::info!(%uri, %e, "no app for this link");
                }
                return true;
            }
            // A frame the lists block is stopped by WebKit's filter after this:
            // counted here, as nothing else sees it. (A top-level page is never
            // the lists' to block, so a match here is a frame's.)
            if kind == PolicyDecisionType::NavigationAction
                && action.navigation_type() == NavigationType::Other
                && !action.is_user_gesture()
                && (scheme == "http" || scheme == "https")
            {
                count_if_blocked(view, &uri, "sub_frame");
            }
            false
        }
        PolicyDecisionType::Response => {
            let Some(r) = decision.downcast_ref::<ResponsePolicyDecision>() else { return false };
            if !r.is_mime_type_supported() {
                decision.download();
                return true;
            }
            if r.is_main_frame_main_resource() {
                let uri = r.request().and_then(|q| q.uri()).map(String::from).unwrap_or_default();
                let Some(st) = state(view) else { return false };
                if let Some(target) = profile_for_uri(&st.profile, &uri)
                    && let Some(e) = engine()
                {
                    // A key's site: into the key's session, in this tab's place.
                    decision.ignore();
                    tracing::info!(from = %st.profile, to = %target, %uri, "a key's site: into its profile");
                    testlog::line("moved", &[&st.profile, &target, &uri]);
                    let session = SessionBundle::for_profile(&e.paths, &target);
                    let v = view.clone();
                    later(move |h| (h.move_view)(&v, &session, &uri));
                    return true;
                }
                prepare_for(view, &st, &uri);
            }
            false
        }
        _ => false,
    }
}

fn load_changed(view: &WebView, event: LoadEvent) {
    let Some(st) = state(view) else { return };
    let uri = view.uri().map(String::from).unwrap_or_default();
    match event {
        LoadEvent::Started | LoadEvent::Redirected => {
            st.new_page(&uri);
            prepare_for(view, &st, &uri);
            crate::shield::changed(Some(view));
        }
        LoadEvent::Committed => {
            st.new_page(&uri);
            prepare_for(view, &st, &uri);
            let css = engine().map(|e| e.privacy.cosmetic_styles(&uri)).unwrap_or_default();
            inject_cosmetic_css(view, &css);
            zoom::apply(view);
        }
        LoadEvent::Finished => {
            testlog::line("loaded", &[&st.profile, &uri, &view.title().unwrap_or_default()]);
        }
        _ => {}
    }
}

/// A page that didn't load: its error page. Cancelled loads, a load a
/// download took over, and ones the browser stopped itself aren't failures.
fn load_failed(view: &WebView, _event: LoadEvent, uri: &str, err: &glib::Error) -> bool {
    if err.matches(NetworkError::Cancelled) || err.matches(PolicyError::FrameLoadInterruptedByPolicyChange) {
        return false;
    }
    let host = wadbrowser_core::site::host_of(uri);
    testlog::line("load-failed", &[&profile_of(view), uri, err.message()]);
    if let Some(e) = engine()
        && e.prefs.borrow().block_social
        && !e.privacy.allowlist().contains_url(uri)
        && let Some(site) = social_site(&host)
    {
        scheme::show_blocked(view, uri, site);
        return true;
    }
    let what = if host.is_empty() { uri.to_string() } else { host };
    scheme::show_error(view, uri, "This page didn't load", &format!("{what} didn't answer: {}", err.message()));
    true
}

/// The page's process ended: a crash, or too much memory (not a tab put to sleep).
fn crashed(view: &WebView, reason: WebProcessTerminationReason) {
    let why = match reason {
        WebProcessTerminationReason::TerminatedByApi => return,
        WebProcessTerminationReason::ExceededMemoryLimit => "It was using too much memory.",
        _ => "Something went wrong in it.",
    };
    let uri = view.uri().map(String::from).unwrap_or_default();
    scheme::show_error(view, &uri, "This tab stopped", why);
}

pub fn webkit_version_string() -> String {
    format!(
        "{}.{}.{}",
        webkit6::functions::major_version(),
        webkit6::functions::minor_version(),
        webkit6::functions::micro_version()
    )
}
