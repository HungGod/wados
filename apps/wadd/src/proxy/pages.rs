//! The proxy's own pages, in the site's look (`stream-pages/`): the
//! sign-in card at a stream's `/ws-<guid>/`, the expired page (an unknown
//! link), and "isn't answering yet" (a sidecar that doesn't answer). Each
//! is self-contained: its CSS, logo and script inline, under a CSP that
//! lets only those run (their hashes), and never framed.

use std::sync::LazyLock;

use base64::Engine;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::header::{self, HeaderValue};
use hyper::{Response, StatusCode};
use sha2::{Digest, Sha256};
use wad_model::StreamMode;

use super::Body;

const CSS: &str = include_str!("../stream-pages/stream.css");
const SIGN_IN_JS: &str = include_str!("../stream-pages/sign-in.js");
const LOGO_SVG: &str = include_str!("../stream-pages/logo.svg");

/// The spike's text (PublicViewer.svelte), and v1's for a sidecar not answering.
pub const EXPIRED_TITLE: &str = "This link has expired";
pub const EXPIRED: &str = "The stream it opened has stopped. Ask whoever sent it for a new one.";
pub const SIGN_IN: &str = "Sign in with the user and password you were sent.";
pub const WRONG: &str = "That’s not it: check the user and password you were sent.";
pub const TOO_MANY: &str = "Too many tries: wait a minute, then try again.";
pub const OLD_LINK: &str = "That one-tap link is from an older stream: sign in instead.";
pub const PRIVATE_SIGN_IN: &str = "Open it with the one-tap link you were sent: it signs you in.";
pub const PRIVATE_OLD_LINK: &str = "That one-tap link is from an older stream: ask for a new one.";
pub const NOT_ANSWERING_TITLE: &str = "This stream isn't answering yet";
pub const NOT_ANSWERING: &str = "Try again in a moment.";

/// Lucide's lock and triangle-alert, as the spike's footer and warning show them.
const LOCK: &str = r#"<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>"#;
const ALERT: &str = r#"<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4"/><path d="M12 17h.01"/></svg>"#;

/// Only the pages' own style and script, by their hashes; nothing framed, nothing from elsewhere.
static CSP: LazyLock<String> = LazyLock::new(|| {
    let hash = |s: &str| base64::engine::general_purpose::STANDARD.encode(Sha256::digest(s.as_bytes()));
    format!(
        "default-src 'none'; style-src 'sha256-{}'; script-src 'sha256-{}'; connect-src 'self'; \
         form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
        hash(CSS),
        hash(SIGN_IN_JS)
    )
});

/// The sign-in card's state.
pub struct Card<'a> {
    pub guid: &'a str,
    pub name: &'a str,
    /// The address the viewer came by (the request's Host), for the footer.
    pub host: &'a str,
    pub mode: StreamMode,
    /// The user they typed, kept after a wrong try.
    pub user: &'a str,
    /// Under the password: [`WRONG`], [`TOO_MANY`].
    pub error: Option<&'a str>,
}

/// The mode as the footer says it (the spike's MODE_LABEL).
pub fn mode_label(m: StreamMode) -> &'static str {
    match m {
        StreamMode::Private => "Private",
        StreamMode::PrivateSecure => "Private (secure)",
        StreamMode::Public => "Public",
        StreamMode::PublicSecure => "Public (secure)",
    }
}

/// The card at `/ws-<guid>/`: a public stream's user and password, or a
/// private one's "open it with the link". Its script opens a one-tap link.
pub fn sign_in(status: StatusCode, c: &Card) -> Response<Body> {
    let name = escape(c.name);
    let footer = match c.host.is_empty() {
        true => format!("<footer>{LOCK} {} stream</footer>", mode_label(c.mode)),
        false => format!("<footer>{LOCK} {} · {} stream</footer>", escape(c.host), mode_label(c.mode)),
    };
    let ready = if c.mode.is_public() {
        let (class, error) = match c.error {
            Some(e) => (" has-error", format!(r#"<span class="error" role="alert">{}</span>"#, escape(e))),
            None => ("", String::new()),
        };
        format!(
            r#"<section class="when-ready">
<h1>{name} is shared with you</h1>
<p>{SIGN_IN}</p>
<p class="warn when-bad-link">{ALERT}<span>{OLD_LINK}</span></p>
<form method="post" action="/ws-{guid}/">
<label class="field"><span class="label">User</span><input name="user" value="{user}" autocomplete="off" autocapitalize="none" spellcheck="false" required></label>
<label class="field{class}"><span class="label">Password</span><input name="password" type="password" autocomplete="off" required{autofocus}>{error}</label>
<button type="submit">Sign in</button>
</form>
</section>"#,
            guid = c.guid,
            user = escape(c.user),
            autofocus = if c.error.is_some() { " autofocus" } else { "" },
        )
    } else {
        format!(
            r#"<section class="when-ready">
<h1>{name} is shared with you</h1>
<p>{PRIVATE_SIGN_IN}</p>
<p class="warn when-bad-link">{ALERT}<span>{PRIVATE_OLD_LINK}</span></p>
</section>"#
        )
    };
    let body = format!(
        r#"{ready}
<section class="when-opening">
<h1 class="row"><span class="spin"></span>Opening {name}</h1>
<p>One moment.</p>
</section>
<section class="when-offline">
<h1>Couldn't reach the stream</h1>
<p>Check your connection, then <a id="again" href="">try again</a>.</p>
</section>
{footer}"#
    );
    page(status, &format!("{name} is shared with you"), &body, Some(SIGN_IN_JS), false)
}

/// An unknown link (a stream that's stopped, or never was): 404.
pub fn expired() -> Response<Body> {
    let body = format!("<h1>{EXPIRED_TITLE}</h1>\n<p>{EXPIRED}</p>");
    page(StatusCode::NOT_FOUND, EXPIRED_TITLE, &body, None, false)
}

/// The sidecar didn't answer: 502, and the page tries again by itself.
pub fn not_answering() -> Response<Body> {
    let body = format!("<h1>{NOT_ANSWERING_TITLE}</h1>\n<p>{NOT_ANSWERING}</p>");
    page(StatusCode::BAD_GATEWAY, NOT_ANSWERING_TITLE, &body, None, true)
}

fn page(status: StatusCode, title: &str, body: &str, script: Option<&str>, retry: bool) -> Response<Body> {
    let script = script.map(|s| format!("<script>{s}</script>\n")).unwrap_or_default();
    let refresh = if retry { "<meta http-equiv=\"refresh\" content=\"5\">\n" } else { "" };
    let html = format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex, nofollow">
<meta name="referrer" content="no-referrer">
<meta name="theme-color" content="#0a0614">
{refresh}<title>{title}</title>
<style>{CSS}</style>
{script}</head>
<body>
<main class="card">
<span class="logo">{LOGO_SVG}<span class="word"><b>WAD</b><small>SPACES</small></span></span>
{body}
</main>
</body>
</html>
"##
    );
    let mut r = Response::new(Full::new(Bytes::from(html)).map_err(|e| match e {}).boxed());
    *r.status_mut() = status;
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    if let Ok(v) = HeaderValue::from_str(&CSP) {
        h.insert(header::CONTENT_SECURITY_POLICY, v);
    }
    no_store(h);
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    r
}

/// Never cached, never sniffed, never a referrer.
pub fn no_store(h: &mut hyper::HeaderMap) {
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
}

/// Text as HTML (an element's content or a quoted attribute's).
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn html(r: Response<Body>) -> String {
        String::from_utf8(r.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
    }

    fn card<'a>(mode: StreamMode, error: Option<&'a str>) -> Card<'a> {
        Card { guid: "g1", name: "<Studio> & co", host: "h.example", mode, user: "\"me\"", error }
    }

    #[tokio::test]
    async fn the_card_says_the_spikes_words_and_escapes_what_it_shows() {
        let r = sign_in(StatusCode::OK, &card(StreamMode::Public, None));
        let csp = r.headers()[header::CONTENT_SECURITY_POLICY].to_str().unwrap().to_string();
        assert!(csp.contains("frame-ancestors 'none'") && csp.contains("script-src 'sha256-"), "{csp}");
        let h = html(r).await;
        assert!(h.contains("&lt;Studio&gt; &amp; co is shared with you"), "{h}");
        assert!(!h.contains("<Studio>"));
        assert!(h.contains(SIGN_IN) && h.contains(r#"action="/ws-g1/""#));
        assert!(h.contains(r#"value="&quot;me&quot;""#));
        assert!(h.contains("h.example · Public stream"));
        assert!(!h.contains(WRONG) && !h.contains(r#"class="field has-error""#));

        let h = html(sign_in(StatusCode::FORBIDDEN, &card(StreamMode::PublicSecure, Some(WRONG)))).await;
        assert!(h.contains(WRONG) && h.contains(r#"class="field has-error""#) && h.contains("Public (secure) stream"));

        // A private stream has no password to ask for.
        let h = html(sign_in(StatusCode::OK, &card(StreamMode::Private, None))).await;
        assert!(h.contains(PRIVATE_SIGN_IN) && !h.contains("<form") && h.contains("Private stream"));
    }

    #[tokio::test]
    async fn the_expired_and_waiting_pages() {
        let r = expired();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        let h = html(r).await;
        assert!(h.contains(EXPIRED_TITLE) && h.contains(EXPIRED) && !h.contains("<script"));
        let r = not_answering();
        assert_eq!(r.status(), StatusCode::BAD_GATEWAY);
        let h = html(r).await;
        assert!(h.contains(NOT_ANSWERING_TITLE) && h.contains("http-equiv=\"refresh\""));
    }
}
