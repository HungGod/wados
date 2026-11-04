//! "Create app from this page…" (the spike's `CreateAppDialog.svelte`): a
//! name, the address it opens at and the profile it signs in with; then
//! "<name> is an app", with Open it.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use super::{Field, Frame, Tone, Variant, button};
use crate::data::Profile;

/// What the dialog starts with.
#[derive(Debug, Clone, Default)]
pub struct CreateAppInfo {
    /// The page's title: the name, to begin with.
    pub title: String,
    /// The page's address.
    pub url: String,
    pub profiles: Vec<Profile>,
    /// The profile picked to begin with (this window's).
    pub profile: String,
}

/// The app it makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewApp {
    pub name: String,
    pub url: String,
    pub profile: String,
}

/// Told how making it went: an error is said in the dialog (it stays).
pub type Done = Box<dyn FnOnce(Result<(), String>)>;

/// What it does.
#[derive(Clone)]
pub struct CreateAppActions {
    /// Makes it (wadd's `assets.webApp`), then says how that went.
    pub create: Rc<dyn Fn(NewApp, Done)>,
    /// "Open it": the app in a window of its own.
    pub open: Rc<dyn Fn(&NewApp)>,
}

/// An address on the web, from what's typed: `github.com/x` →
/// `https://github.com/x`; words, or one of the browser's own pages, aren't one.
pub fn web_address(input: &str) -> Option<String> {
    let t = input.trim();
    if t.is_empty() || t.contains(char::is_whitespace) {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return (!crate::data::bare_host(t).is_empty()).then(|| t.to_string());
    }
    if lower.contains("://") || lower.starts_with("about:") {
        return None;
    }
    let host = crate::data::bare_host(t);
    let hostish = host == "localhost"
        || host.split('.').count() >= 2
            && host.split('.').all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    if !hostish {
        return None;
    }
    let local = host == "localhost" || host.bytes().next().is_some_and(|b| b.is_ascii_digit());
    Some(format!("{}://{t}", if local { "http" } else { "https" }))
}

/// The dialog open.
pub struct CreateAppDialog {
    pub dialog: adw::Dialog,
    submit: Rc<dyn Fn()>,
}

impl CreateAppDialog {
    /// "Create app", as its button does.
    pub fn submit(&self) {
        (self.submit)()
    }
}

/// "Create app from this page", over `parent`.
pub fn create_app(parent: &impl IsA<gtk::Widget>, info: &CreateAppInfo, actions: CreateAppActions) -> CreateAppDialog {
    let frame = Rc::new(Frame::new("Create app from this page", "app-window", Tone::Accent, 440));
    let name = Field::new("Name", "", &info.title, false);
    let address = Field::new("Opens at", "", &info.url, true);
    frame.body.append(&name.root);
    frame.body.append(&address.root);

    // The profile.
    let pick = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
    pick.append(&gtk::Label::builder().label("Profile").xalign(0.0).css_classes(["wb-field-label"]).build());
    let names: Vec<&str> = info.profiles.iter().map(|p| p.name.as_str()).collect();
    let profiles = gtk::DropDown::from_strings(&names);
    profiles.add_css_class("wb-select");
    profiles.update_property(&[gtk::accessible::Property::Label("Profile")]);
    if let Some(i) = info.profiles.iter().position(|p| p.id == info.profile) {
        profiles.set_selected(i as u32);
    }
    pick.append(&profiles);
    frame.body.append(&pick);

    let cancel = button("Cancel", Variant::Ghost, None);
    let create = button("Create app", Variant::Primary, None);
    frame.footer.append(&cancel);
    frame.footer.append(&create);
    let d = frame.dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(d) = d.upgrade() {
            d.close();
        }
    });

    let ids: Vec<String> = info.profiles.iter().map(|p| p.id.clone()).collect();
    let fallback = info.title.clone();
    let name = Rc::new(name);
    let address = Rc::new(address);
    let busy = Rc::new(RefCell::new(false));
    let submit: Rc<dyn Fn()> = {
        let (frame, name, address, profiles, create) =
            (frame.clone(), name.clone(), address.clone(), profiles.clone(), create.clone());
        let actions = actions.clone();
        Rc::new(move || {
            if *busy.borrow() {
                return;
            }
            let Some(url) = web_address(&address.text()) else {
                address.error("An address on the web, please");
                return;
            };
            let shown = name.text().trim().to_string();
            let app = NewApp {
                name: if shown.is_empty() { fallback.clone() } else { shown },
                url,
                profile: ids.get(profiles.selected() as usize).cloned().unwrap_or_default(),
            };
            *busy.borrow_mut() = true;
            create.set_sensitive(false);
            let (frame, address, create, busy, open) =
                (frame.clone(), address.clone(), create.clone(), busy.clone(), actions.open.clone());
            let made = app.clone();
            (actions.create)(
                app,
                Box::new(move |r| {
                    *busy.borrow_mut() = false;
                    create.set_sensitive(true);
                    match r {
                        Ok(()) => made_it(&frame, made, open),
                        Err(e) => address.error(&e),
                    }
                }),
            );
        })
    };
    let s = submit.clone();
    create.connect_clicked(move |_| s());
    for e in [&name.entry, &address.entry] {
        let s = submit.clone();
        e.connect_activate(move |_| s());
    }
    name.focus_in(&frame.dialog);
    frame.dialog.present(Some(parent));
    CreateAppDialog { dialog: frame.dialog.clone(), submit }
}

/// The second step: it's an app now.
fn made_it(frame: &Rc<Frame>, app: NewApp, open: Rc<dyn Fn(&NewApp)>) {
    frame.clear();
    frame.retitle(&format!("{} is an app", app.name), "app-window");
    let row = gtk::Box::builder().spacing(16).build();
    let icon = wad_gtk::app_icon::site(&app.name.to_lowercase(), Some(&app.name), Some(&app.url), 48);
    icon.set_valign(gtk::Align::Center);
    icon.set_hexpand(false);
    row.append(&icon);
    let text = gtk::Label::builder()
        .label(
            "It’s in the launcher and on the new tab page, and opens in a window of its own, without an address bar.",
        )
        .xalign(0.0)
        .wrap(true)
        .hexpand(true)
        .css_classes(["wb-dialog-text"])
        .build();
    text.set_max_width_chars(1);
    row.append(&text);
    frame.body.append(&row);
    let done = button("Done", Variant::Ghost, None);
    let go = button("Open it", Variant::Primary, Some("external-link"));
    frame.footer.append(&done);
    frame.footer.append(&go);
    let d = frame.dialog.downgrade();
    done.connect_clicked(move |_| {
        if let Some(d) = d.upgrade() {
            d.close();
        }
    });
    let d = frame.dialog.downgrade();
    go.connect_clicked(move |_| {
        open(&app);
        if let Some(d) = d.upgrade() {
            d.close();
        }
    });
    frame.dialog.set_focus(Some(&go));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apps_open_at_addresses_on_the_web() {
        assert_eq!(web_address("github.com").as_deref(), Some("https://github.com"));
        assert_eq!(web_address(" github.com/HungGod/x ").as_deref(), Some("https://github.com/HungGod/x"));
        assert_eq!(web_address("https://mail.google.com/mail").as_deref(), Some("https://mail.google.com/mail"));
        assert_eq!(web_address("localhost:5199").as_deref(), Some("http://localhost:5199"));
        assert_eq!(web_address("192.168.1.4:8080").as_deref(), Some("http://192.168.1.4:8080"));
        assert_eq!(web_address("cactus://newtab"), None);
        assert_eq!(web_address("about:blank"), None);
        assert_eq!(web_address("rust gtk"), None);
        assert_eq!(web_address("hello"), None);
        assert_eq!(web_address(""), None);
        assert_eq!(web_address("https://"), None);
    }
}
