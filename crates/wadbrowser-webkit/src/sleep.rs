//! Tab sleeping (v1's hibernation): a tab not looked at for a while has its
//! page unloaded (its web process ended, its history kept by WebKit), and
//! reloads where it was when it's shown again. Not the tab shown, one
//! playing sound, one downloading, nor a popup or its opener (they share a
//! web process). The UI keeps the clock: [`touch`] when a tab is shown or
//! left, [`sleepy`] each minute, [`wake`] when one is shown.

use crate::view::state;
use std::time::{Duration, Instant};
use webkit6::WebView;
use webkit6::prelude::*;

/// Whether `view` has been asleep.
pub fn is_asleep(view: &WebView) -> bool {
    state(view).is_some_and(|s| s.asleep.get())
}

/// It's being looked at (shown, or just left).
pub fn touch(view: &WebView) {
    if let Some(s) = state(view) {
        s.last_seen.set(Instant::now());
    }
}

/// Whether it could sleep now, `after` since it was last looked at.
pub fn sleepy(view: &WebView, after: Duration) -> bool {
    let Some(s) = state(view) else { return false };
    !s.asleep.get()
        && !s.related.get()
        && s.last_seen.get().elapsed() >= after
        && !view.is_playing_audio()
        && !view.is_loading()
        && !crate::downloads::downloading(view)
        && view.uri().is_some_and(|u| u.starts_with("http"))
}

/// Unloads its page; its history stays.
pub fn sleep(view: &WebView) {
    let Some(s) = state(view) else { return };
    if s.asleep.replace(true) {
        return;
    }
    tracing::debug!(uri = ?view.uri(), "a tab sleeps");
    crate::testlog::line("sleep", &[&s.profile, &view.uri().unwrap_or_default()]);
    view.terminate_web_process();
}

/// Loads its page again, where it was.
pub fn wake(view: &WebView) {
    let Some(s) = state(view) else { return };
    s.last_seen.set(Instant::now());
    if !s.asleep.replace(false) {
        return;
    }
    crate::testlog::line("wake", &[&s.profile, &view.uri().unwrap_or_default()]);
    view.reload();
}
