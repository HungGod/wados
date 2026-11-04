//! Saved sign-ins (web keys), from a page's menu (the spike's
//! `SignInDialogs.svelte`): a new one for this site, or all of them, each
//! with its sites, any of which can go.

use std::rc::Rc;

use adw::prelude::*;

use super::{Field, Frame, Tone, Variant, button};
use crate::data::WebKey;

/// "Add a new sign-in for <host>…", over `parent`; `save` gets its name.
pub fn add_signin(parent: &impl IsA<gtk::Widget>, host: &str, save: Rc<dyn Fn(&str)>) -> adw::Dialog {
    let frame = Frame::new(&format!("A new sign-in for {host}"), "key-round", Tone::Magenta, 440);
    frame.say(
        "A saved sign-in keeps this site’s cookies on its own, so any wadspace (or web app) given it is signed in too.",
    );
    let name = Field::new("Name", "Work GitHub, Personal…", "", false);
    frame.body.append(&name.root);
    let cancel = button("Cancel", Variant::Ghost, None);
    let ok = button("Save sign-in", Variant::Primary, None);
    ok.set_sensitive(false);
    frame.footer.append(&cancel);
    frame.footer.append(&ok);
    let d = frame.dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(d) = d.upgrade() {
            d.close();
        }
    });
    let o = ok.downgrade();
    name.entry.connect_changed(move |e| {
        if let Some(o) = o.upgrade() {
            o.set_sensitive(!e.text().trim().is_empty());
        }
    });
    let submit: Rc<dyn Fn()> = {
        let (entry, d) = (name.entry.clone(), frame.dialog.downgrade());
        Rc::new(move || {
            let n = entry.text().trim().to_string();
            if n.is_empty() {
                return;
            }
            save(&n);
            if let Some(d) = d.upgrade() {
                d.close();
            }
        })
    };
    let s = submit.clone();
    ok.connect_clicked(move |_| s());
    name.entry.connect_activate(move |_| submit());
    name.focus_in(&frame.dialog);
    frame.dialog.present(Some(parent));
    frame.dialog
}

/// "Used by 1 wadspace", "used by no wadspaces".
pub fn used_by(n: u32) -> String {
    match n {
        0 => "used by no wadspaces".into(),
        1 => "used by 1 wadspace".into(),
        n => format!("used by {n} wadspaces"),
    }
}

/// A key's id, and a site it no longer signs in to.
pub type RemoveSite = Rc<dyn Fn(&str, &str)>;

/// "Edit saved sign-ins…", over `parent`: each key and its sites (this
/// page's `host` ringed); `remove_site` gets a key's id and the site it no
/// longer signs in to.
pub fn edit_signins(
    parent: &impl IsA<gtk::Widget>,
    keys: &[WebKey],
    host: &str,
    remove_site: RemoveSite,
) -> adw::Dialog {
    let frame = Frame::new("Saved sign-ins", "key-round", Tone::Magenta, 520);
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
    for k in keys {
        let card =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["wb-key"]).build();
        let head = gtk::Box::builder().spacing(8).build();
        let ic = wad_gtk::Icon::new("key-round", 15);
        ic.add_css_class("wb-key-icon");
        head.append(&ic);
        head.append(&gtk::Label::builder().label(&k.name).css_classes(["wb-key-name"]).build());
        head.append(&gtk::Label::builder().label(used_by(k.used_by)).css_classes(["wb-small-faint"]).build());
        card.append(&head);
        let sites = wad_gtk::Wrap::new(6, 6);
        let none = gtk::Label::builder().label("No sites").xalign(0.0).css_classes(["wb-small-faint"]).build();
        none.set_visible(k.sites.is_empty());
        for site in &k.sites {
            let pill = gtk::Box::builder().spacing(4).css_classes(["wb-site"]).build();
            if site == host {
                pill.add_css_class("here");
            }
            pill.append(&gtk::Label::new(Some(site)));
            let x = gtk::Button::builder()
                .child(&wad_gtk::Icon::new("x", 12))
                .css_classes(["wb-site-x"])
                .tooltip_text(format!("Remove {site} from {}", k.name))
                .valign(gtk::Align::Center)
                .build();
            x.update_property(&[gtk::accessible::Property::Label(&format!("Remove {site} from {}", k.name))]);
            pill.append(&x);
            sites.append(&pill);
            let (key, site, remove) = (k.id.clone(), site.clone(), remove_site.clone());
            let (pill, wrap, none) = (pill.downgrade(), sites.downgrade(), none.downgrade());
            x.connect_clicked(move |_| {
                remove(&key, &site);
                if let (Some(p), Some(w)) = (pill.upgrade(), wrap.upgrade()) {
                    p.unparent();
                    w.queue_resize();
                    if w.first_child().is_none()
                        && let Some(n) = none.upgrade()
                    {
                        n.set_visible(true);
                    }
                }
            });
        }
        card.append(&sites);
        card.append(&none);
        list.append(&card);
    }
    if keys.is_empty() {
        let card = gtk::Box::builder().css_classes(["wb-key"]).build();
        card.append(
            &gtk::Label::builder().label("No saved sign-ins yet.").xalign(0.0).css_classes(["wb-small-faint"]).build(),
        );
        list.append(&card);
    }
    frame.body.append(&list);
    let done = button("Done", Variant::Primary, None);
    frame.footer.append(&done);
    let d = frame.dialog.downgrade();
    done.connect_clicked(move |_| {
        if let Some(d) = d.upgrade() {
            d.close();
        }
    });
    frame.dialog.set_focus(Some(&done));
    frame.dialog.present(Some(parent));
    frame.dialog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_says_how_many_wadspaces_use_a_key() {
        assert_eq!(used_by(0), "used by no wadspaces");
        assert_eq!(used_by(1), "used by 1 wadspace");
        assert_eq!(used_by(3), "used by 3 wadspaces");
    }
}
