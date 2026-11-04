//! Navigation / response policy hooks for real request blocking.

use std::rc::Rc;
use tracing::debug;
use wadbrowser_privacy::PrivacyEngine;
use webkit6::prelude::*;
use webkit6::{PolicyDecisionType, ResponsePolicyDecision, WebView};

/// Attach decide-policy handler that ignores blocked subresource responses
/// and keeps document navigations usable.
pub fn attach_blocking_policy(view: &WebView, privacy: Rc<PrivacyEngine>) {
    view.connect_decide_policy(move |web_view, decision, decision_type| {
        match decision_type {
            PolicyDecisionType::Response => {
                if let Ok(resp) = decision.clone().downcast::<ResponsePolicyDecision>() {
                    let url = resp.request().and_then(|r| r.uri()).unwrap_or_default().to_string();
                    let mime = resp.response().and_then(|r| r.mime_type()).map(|m| m.to_string());
                    let source = web_view.uri().unwrap_or_else(|| "https://example.invalid".into()).to_string();

                    // Don't block the main document response.
                    if !url.is_empty() && url == source {
                        return false;
                    }

                    let decision_result = privacy.should_block_auto(&url, &source, mime.as_deref());
                    if decision_result.matched {
                        debug!("policy-block {url} ({:?})", decision_result.reason);
                        decision.ignore();
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    });
}

/// Re-export helper used by UI when wiring a freshly created tab.
pub fn wire_privacy(view: &WebView, privacy: Rc<PrivacyEngine>) {
    attach_blocking_policy(view, privacy.clone());
    crate::view::attach_request_filter(view, privacy);
}
