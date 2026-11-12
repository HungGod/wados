//! A network that wants a sign-in first (a hotel's, a café's captive
//! portal). wadd's `network` says so (`portal`: NetworkManager's
//! connectivity check found a page in the way), and its sign-in page opens
//! in WadBrowser at once, as a phone opens it, with a notice whose button
//! opens it again. Once each time a network turns out to want it: signed
//! in, the check finds the internet and it's over.

use std::cell::Cell;
use std::rc::Rc;

use wad_proto::{Event, Level, NoticeAction};

use crate::shell::Shell;
use crate::store::Change;

/// What's opened: NetworkManager's own check (Fedora's, plain HTTP), which
/// a portal answers with its sign-in page; signed in, the page says "OK".
pub const PAGE: &str = "http://fedoraproject.org/static/hotspot.txt";

/// The notice's button (verbs.rs).
pub const VERB: &str = "network.signin";

/// Whether a network event is a portal newly found: it wants a sign-in now,
/// and didn't before.
pub fn newly(was: bool, now: bool) -> bool {
    now && !was
}

/// Follows wadd's network for the shell's session.
pub fn start(shell: &Rc<Shell>) {
    let s = Rc::downgrade(shell);
    let was = Cell::new(false);
    shell.store.watch(move |c| {
        let (Some(shell), Change::Network) = (s.upgrade(), c) else { return };
        let net = shell.store.network();
        let now = net.as_ref().is_some_and(|n| n.portal);
        if newly(was.replace(now), now) {
            let name = net.and_then(|n| n.connected).unwrap_or_else(|| "This network".into());
            tracing::info!("network: {name} wants a sign-in: opening its page");
            open(&shell);
            shell.store.apply(Event::Notice {
                level: Level::Info,
                title: format!("Sign in to {name}"),
                undo: None,
                body: Some("It asks you to sign in before it lets you online: its page is open in WadBrowser.".into()),
                icon: Some("wifi".into()),
                action: Some(NoticeAction::new("Sign in", VERB, &[])),
                toast: true,
            });
        }
    });
}

/// The network's sign-in page, in WadBrowser.
pub fn open(shell: &Shell) {
    crate::peers::open_app(shell, &["wadbrowser", PAGE]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_portal_newly_found_opens_its_page() {
        assert!(newly(false, true), "found");
        assert!(!newly(true, true), "still there: opened once");
        assert!(!newly(true, false) && !newly(false, false), "signed in, or none");
    }
}
