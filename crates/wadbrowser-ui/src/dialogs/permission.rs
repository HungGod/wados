//! A site asking for something (WebKit's permission requests), in the
//! dialogs' look: what it wants, Allow or Don't allow, and whether to
//! remember that for the site (Cactus's per-site permissions).

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use wadbrowser_core::PermissionKind;

use super::{Frame, Tone, Variant, button};

/// What's asked (the engine's `permissions::Ask`, as plain data).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionInfo {
    /// Who's asking ("meet.example.com").
    pub site: String,
    /// What for (the camera and the microphone can come together).
    pub kinds: Vec<PermissionKind>,
    /// "Remember for this site" can be offered (not in a private window).
    pub can_remember: bool,
}

/// The answer: allowed or not, and whether to remember it for the site.
/// Closing the dialog is "Don't allow", not remembered.
pub type Decide = Rc<dyn Fn(bool, bool)>;

/// Its icon, title and what it means.
pub fn ask_text(info: &PermissionInfo) -> (&'static str, String, &'static str) {
    use PermissionKind::*;
    let site = &info.site;
    let has = |k: PermissionKind| info.kinds.contains(&k);
    let camera = has(Camera) || has(Media);
    let microphone = has(Microphone) || has(Media);
    let media = info.kinds.iter().all(|k| matches!(k, Camera | Microphone | Media));
    if media && camera && microphone {
        return (
            "camera",
            format!("{site} wants to use your camera and microphone"),
            "It can see and hear you while the page is open.",
        );
    }
    if media && camera {
        return ("camera", format!("{site} wants to use your camera"), "It can see you while the page is open.");
    }
    if media && microphone {
        return ("mic", format!("{site} wants to use your microphone"), "It can hear you while the page is open.");
    }
    match info.kinds.as_slice() {
        [Notifications] => (
            "bell",
            format!("{site} wants to send you notifications"),
            "They show on this machine, even when you’re on another page.",
        ),
        [Clipboard] => (
            "clipboard",
            format!("{site} wants to see what you’ve copied"),
            "It can read the clipboard while it’s open.",
        ),
        [Geolocation] => (
            "map-pin",
            format!("{site} wants to know where you are"),
            "It’s told this machine’s location while it’s open.",
        ),
        [IdleDetection] => (
            "clock",
            format!("{site} wants to know when you’re away"),
            "It’s told when this machine has been idle a while.",
        ),
        kinds => {
            let what: Vec<String> = kinds.iter().map(|k| k.asking().to_lowercase()).collect();
            ("shield", format!("{site} wants to {}", what.join(" and ")), "Only allow it if you trust this site.")
        }
    }
}

/// The question, over `parent`.
pub fn permission(parent: &impl IsA<gtk::Widget>, info: &PermissionInfo, decide: Decide) -> adw::Dialog {
    let (icon, title, what) = ask_text(info);
    let frame = Frame::new(&title, icon, Tone::Warn, 440);
    frame.say(what);
    let remember = gtk::CheckButton::builder().label("Remember for this site").css_classes(["wb-check"]).build();
    remember.set_visible(info.can_remember);
    frame.body.append(&remember);
    let no = button("Don’t allow", Variant::Ghost, None);
    let yes = button("Allow", Variant::Primary, None);
    frame.footer.append(&no);
    frame.footer.append(&yes);

    let answered = Rc::new(Cell::new(false));
    for (b, allow) in [(&no, false), (&yes, true)] {
        let (decide, answered, remember, d) =
            (decide.clone(), answered.clone(), remember.clone(), frame.dialog.downgrade());
        b.connect_clicked(move |_| {
            if !answered.replace(true) {
                decide(allow, remember.get_visible() && remember.is_active());
            }
            if let Some(d) = d.upgrade() {
                d.close();
            }
        });
    }
    frame.dialog.connect_closed(move |_| {
        if !answered.replace(true) {
            decide(false, false);
        }
    });
    frame.dialog.set_focus(Some(&no));
    frame.dialog.present(Some(parent));
    frame.dialog
}

#[cfg(test)]
mod tests {
    use super::*;
    use PermissionKind::*;

    fn ask(kinds: &[PermissionKind]) -> (&'static str, String, &'static str) {
        ask_text(&PermissionInfo { site: "meet.example.com".into(), kinds: kinds.to_vec(), can_remember: true })
    }

    #[test]
    fn it_says_what_the_site_wants() {
        assert_eq!(ask(&[Camera, Microphone]).1, "meet.example.com wants to use your camera and microphone");
        assert_eq!(ask(&[Media]).1, "meet.example.com wants to use your camera and microphone");
        assert_eq!(ask(&[Camera]).0, "camera");
        assert_eq!(ask(&[Microphone]).0, "mic");
        assert_eq!(ask(&[Notifications]).1, "meet.example.com wants to send you notifications");
        assert_eq!(ask(&[Geolocation]).0, "map-pin");
        assert_eq!(
            ask(&[Clipboard, Notifications]).1,
            "meet.example.com wants to see what you copied and show notifications"
        );
        for kinds in [
            &[Notifications][..],
            &[Geolocation],
            &[Camera, Microphone],
            &[Microphone],
            &[Clipboard],
            &[IdleDetection],
            &[Clipboard, Notifications],
        ] {
            let icon = ask(kinds).0;
            assert!(wad_gtk::icon::svg(icon).is_some(), "{icon} is one of ours");
        }
    }
}
