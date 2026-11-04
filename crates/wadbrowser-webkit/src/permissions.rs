//! A page asking for the camera, the microphone, notifications or the
//! clipboard: the person is asked (the UI's `ask_permission` hook), and the
//! answer can be kept for the site (the store's `site_permissions`; a
//! private window's are never kept). v1's rules for the rest: location is
//! never given; listing the devices only to a site let at the camera or the
//! microphone; pointer lock and storage access always.

use crate::engine::{engine, later};
use crate::view::state;
use wadbrowser_core::{PermissionDecision, PermissionKind, origin_from_url};
use webkit6::prelude::*;
use webkit6::{
    ClipboardPermissionRequest, DeviceInfoPermissionRequest, NotificationPermissionRequest, PermissionRequest,
    PointerLockPermissionRequest, UserMediaPermissionRequest, WebView, WebsiteDataAccessPermissionRequest,
};

/// A question for the person: may `host` do `kinds`? Answer it once;
/// dropped unanswered, it's a no.
pub struct Ask {
    /// The site's origin ("https://meet.example").
    pub origin: String,
    /// Its host, to show ("meet.example").
    pub host: String,
    /// What it asks for (the camera and the microphone can come together).
    pub kinds: Vec<PermissionKind>,
    /// Whether "remember" can be offered (not in a private window).
    pub can_remember: bool,
    request: Option<PermissionRequest>,
}

impl Ask {
    /// What to say: "meet.example wants to use your camera".
    pub fn question(&self) -> String {
        let what: Vec<String> = self.kinds.iter().map(|k| k.asking().to_lowercase()).collect();
        format!("{} wants to {}", self.host, what.join(" and "))
    }

    /// The person's answer; `remember` keeps it for the site.
    pub fn answer(mut self, allow: bool, remember: bool) {
        let Some(req) = self.request.take() else { return };
        if allow {
            req.allow()
        } else {
            req.deny()
        }
        if remember
            && self.can_remember
            && let Some(e) = engine()
        {
            let d = if allow { PermissionDecision::Allow } else { PermissionDecision::Deny };
            for k in &self.kinds {
                if let Err(err) = e.store.set_permission(&self.origin, *k, d) {
                    tracing::warn!("keeping a permission: {err}");
                }
            }
        }
    }
}

impl Drop for Ask {
    fn drop(&mut self) {
        if let Some(req) = self.request.take() {
            req.deny();
        }
    }
}

fn stored(origin: &str, kind: PermissionKind) -> Option<PermissionDecision> {
    engine()?.store.get_permission(origin, kind).ok().flatten()
}

/// The `permission-request` signal.
pub(crate) fn request(view: &WebView, req: &PermissionRequest) -> bool {
    let uri = view.uri().map(String::from).unwrap_or_default();
    let origin = origin_from_url(&uri).unwrap_or_default();
    let kinds = if let Some(m) = req.downcast_ref::<UserMediaPermissionRequest>() {
        let mut k = vec![];
        if m.is_for_video_device() {
            k.push(PermissionKind::Camera);
        }
        if m.is_for_audio_device() {
            k.push(PermissionKind::Microphone);
        }
        if k.is_empty() {
            k.push(PermissionKind::Media);
        }
        k
    } else if req.is::<NotificationPermissionRequest>() {
        vec![PermissionKind::Notifications]
    } else if req.is::<ClipboardPermissionRequest>() {
        vec![PermissionKind::Clipboard]
    } else if req.is::<DeviceInfoPermissionRequest>() {
        // Listing the devices: only for a site already let at one.
        let ok = [PermissionKind::Camera, PermissionKind::Microphone, PermissionKind::Media]
            .iter()
            .any(|k| stored(&origin, *k) == Some(PermissionDecision::Allow));
        if ok {
            req.allow()
        } else {
            req.deny()
        }
        return true;
    } else if req.is::<PointerLockPermissionRequest>() || req.is::<WebsiteDataAccessPermissionRequest>() {
        req.allow();
        return true;
    } else {
        // Location, DRM (MediaKeySystem…) and anything newer: no.
        req.deny();
        return true;
    };
    if origin.is_empty() {
        req.deny();
        return true;
    }
    let decisions: Vec<Option<PermissionDecision>> = kinds.iter().map(|k| stored(&origin, *k)).collect();
    if decisions.contains(&Some(PermissionDecision::Deny)) {
        req.deny();
        return true;
    }
    if decisions.iter().all(|d| *d == Some(PermissionDecision::Allow)) {
        req.allow();
        return true;
    }
    let private = state(view).is_some_and(|s| s.private);
    let host = wadbrowser_core::site::host_of(&uri);
    let ask = Ask { origin, host, kinds, can_remember: !private, request: Some(req.clone()) };
    let v = view.clone();
    later(move |h| (h.ask_permission)(&v, ask));
    true
}
