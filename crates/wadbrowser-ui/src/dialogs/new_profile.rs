//! "New profile…" (the spike's `NewProfileDialog.svelte`): a name and a
//! colour; then it opens in a new window.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use super::{Field, Frame, Tone, Variant, button};

/// The colours a profile can be.
pub const SWATCHES: [&str; 8] =
    ["#c6ff1f", "#7fd3ff", "#ff3d81", "#ffb36b", "#b58cff", "#5cf2c4", "#e8c36a", "#f5f2fa"];

/// The first colour no profile has (else the first).
pub fn first_free(taken: &[String]) -> &'static str {
    SWATCHES.iter().find(|c| !taken.iter().any(|t| t.eq_ignore_ascii_case(c))).copied().unwrap_or(SWATCHES[0])
}

/// "Create and open": its name and colour.
pub type Create = Rc<dyn Fn(&str, &str)>;

/// What it does.
#[derive(Clone)]
pub struct NewProfileActions {
    pub create: Create,
}

/// "New profile", over `parent`; `taken`: the colours profiles have (shown dimmer).
pub fn new_profile(parent: &impl IsA<gtk::Widget>, taken: &[String], actions: NewProfileActions) -> adw::Dialog {
    let frame = Frame::new("New profile", "user-round-plus", Tone::Accent, 440);
    frame.say("A profile keeps its own sign-ins, cookies and history, apart from the others.");
    let name = Field::new("Name", "School, Side project…", "", false);
    frame.body.append(&name.root);

    let colours = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
    colours.append(&gtk::Label::builder().label("Colour").xalign(0.0).css_classes(["wb-field-label"]).build());
    let row = gtk::Box::builder().spacing(10).build();
    row.update_property(&[gtk::accessible::Property::Label("Colour")]);
    let chosen = Rc::new(RefCell::new(first_free(taken).to_string()));
    let mut swatches = vec![];
    for c in SWATCHES {
        let b = gtk::Button::builder()
            .css_classes(["wb-swatch", &format!("wb-swatch-{}", &c[1..])])
            .tooltip_text(c)
            .valign(gtk::Align::Center)
            .build();
        b.update_property(&[gtk::accessible::Property::Label(c)]);
        if taken.iter().any(|t| t.eq_ignore_ascii_case(c)) {
            b.add_css_class("used");
        }
        if *chosen.borrow() == c {
            b.add_css_class("on");
        }
        row.append(&b);
        swatches.push((c, b));
    }
    let swatches = Rc::new(swatches);
    for (c, b) in swatches.iter() {
        let (c, all, chosen) = (*c, swatches.clone(), chosen.clone());
        b.connect_clicked(move |_| {
            *chosen.borrow_mut() = c.to_string();
            for (o, b) in all.iter() {
                if *o == c {
                    b.add_css_class("on");
                } else {
                    b.remove_css_class("on");
                }
            }
        });
    }
    colours.append(&row);
    frame.body.append(&colours);

    let cancel = button("Cancel", Variant::Ghost, None);
    let create = button("Create and open", Variant::Primary, None);
    create.set_sensitive(false);
    frame.footer.append(&cancel);
    frame.footer.append(&create);
    let d = frame.dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(d) = d.upgrade() {
            d.close();
        }
    });
    let c = create.downgrade();
    name.entry.connect_changed(move |e| {
        if let Some(c) = c.upgrade() {
            c.set_sensitive(!e.text().trim().is_empty());
        }
    });
    let submit: Rc<dyn Fn()> = {
        let (entry, d) = (name.entry.clone(), frame.dialog.downgrade());
        Rc::new(move || {
            let n = entry.text().trim().to_string();
            if n.is_empty() {
                return;
            }
            (actions.create)(&n, &chosen.borrow());
            if let Some(d) = d.upgrade() {
                d.close();
            }
        })
    };
    let s = submit.clone();
    create.connect_clicked(move |_| s());
    name.entry.connect_activate(move |_| submit());
    name.focus_in(&frame.dialog);
    frame.dialog.present(Some(parent));
    frame.dialog
}

/// The swatches' colours (they're fixed: one rule each).
pub(crate) fn css() -> String {
    SWATCHES.iter().map(|c| format!("button.wb-swatch.wb-swatch-{} {{ background-color: {c}; }}\n", &c[1..])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_profile_gets_a_colour_no_other_has() {
        assert_eq!(first_free(&[]), "#c6ff1f");
        assert_eq!(first_free(&["#C6FF1F".into(), "#7fd3ff".into()]), "#ff3d81");
        let all: Vec<String> = SWATCHES.iter().map(|s| s.to_string()).collect();
        assert_eq!(first_free(&all), "#c6ff1f");
    }
}
