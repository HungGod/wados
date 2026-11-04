use crate::session::SessionBundle;
use std::rc::Rc;
use tracing::debug;
use wadbrowser_core::Preferences;
use wadbrowser_privacy::PrivacyEngine;
use webkit6::prelude::*;
use webkit6::{Settings as WebSettings, UserStyleLevel, UserStyleSheet, WebView};

pub fn create_web_view(session: &SessionBundle, prefs: &Preferences) -> WebView {
    let settings = WebSettings::new();
    settings.set_enable_developer_extras(true);
    settings.set_enable_smooth_scrolling(prefs.enable_smooth_scrolling);
    settings.set_enable_back_forward_navigation_gestures(true);
    settings.set_javascript_can_open_windows_automatically(false);
    settings.set_allow_modal_dialogs(false);
    settings.set_enable_page_cache(true);
    settings.set_enable_javascript(prefs.enable_javascript);

    let view = WebView::builder()
        .network_session(&session.network)
        .user_content_manager(&session.content)
        .settings(&settings)
        .hexpand(true)
        .vexpand(true)
        .build();

    apply_privacy_headers(&view, prefs);
    view.set_zoom_level(prefs.default_zoom);
    view
}

pub fn apply_privacy_headers(view: &WebView, prefs: &Preferences) {
    let mut js = String::from("(function(){");
    if prefs.send_dnt {
        js.push_str("try{Object.defineProperty(navigator,'doNotTrack',{get:()=> '1'});}catch(e){}");
    }
    if prefs.send_gpc {
        js.push_str("try{Object.defineProperty(navigator,'globalPrivacyControl',{get:()=> true});}catch(e){}");
    }
    js.push_str("})();");

    if let Some(manager) = view.user_content_manager() {
        let script = webkit6::UserScript::new(
            &js,
            webkit6::UserContentInjectedFrames::AllFrames,
            webkit6::UserScriptInjectionTime::Start,
            &[],
            &[],
        );
        manager.add_script(&script);
    }
}

pub fn inject_cosmetic_css(view: &WebView, css: &str) {
    if css.trim().is_empty() {
        return;
    }
    if let Some(manager) = view.user_content_manager() {
        let sheet =
            UserStyleSheet::new(css, webkit6::UserContentInjectedFrames::AllFrames, UserStyleLevel::User, &[], &[]);
        manager.add_style_sheet(&sheet);
    }
}

pub fn load_html(view: &WebView, html: &str, base_uri: &str) {
    view.load_html(html, Some(base_uri));
}

pub fn navigate(view: &WebView, uri: &str) {
    view.load_uri(uri);
}

/// Resource-load observer — pairs with policy blocking for visibility/stats.
pub fn attach_request_filter(view: &WebView, privacy: Rc<PrivacyEngine>) {
    view.connect_resource_load_started(move |web_view, _resource, request| {
        let url = request.uri().unwrap_or_default().to_string();
        let source = web_view.uri().unwrap_or_else(|| "https://example.invalid".into());
        let decision = privacy.should_block_auto(&url, &source, None);
        if decision.matched {
            debug!("resource matched block list: {url}");
        }
    });
}

pub fn webkit_version_string() -> String {
    format!(
        "{}.{}.{}",
        webkit6::functions::major_version(),
        webkit6::functions::minor_version(),
        webkit6::functions::micro_version()
    )
}
