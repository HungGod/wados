//! Omnibox suggestion popover helpers.

use gtk::prelude::*;
use gtk::{Box as GtkBox, Label, ListBox, ListBoxRow, Orientation, Popover, PositionType, SelectionMode};
use wadbrowser_core::{Suggestion, SuggestionKind, suggest};

pub fn build_suggestion_popover() -> (Popover, ListBox) {
    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::Single);
    list.add_css_class("boxed-list");

    let pop = Popover::new();
    pop.set_position(PositionType::Bottom);
    pop.set_autohide(true);
    pop.set_child(Some(&list));
    (pop, list)
}

pub fn refill_suggestions(list: &ListBox, suggestions: &[Suggestion]) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for suggestion in suggestions {
        let row = ListBoxRow::new();
        let box_ = GtkBox::new(Orientation::Vertical, 2);
        box_.set_margin_top(6);
        box_.set_margin_bottom(6);
        box_.set_margin_start(10);
        box_.set_margin_end(10);

        let title = Label::new(Some(&suggestion.title));
        title.set_halign(gtk::Align::Start);
        title.set_xalign(0.0);
        title.add_css_class("heading");

        let subtitle = Label::new(Some(&format!("{} · {}", kind_label(suggestion.kind), suggestion.url)));
        subtitle.set_halign(gtk::Align::Start);
        subtitle.set_xalign(0.0);
        subtitle.add_css_class("dim-label");
        subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);

        box_.append(&title);
        box_.append(&subtitle);
        row.set_child(Some(&box_));
        // Stash URL on widget name for activation.
        row.set_widget_name(&suggestion.url);
        list.append(&row);
    }
}

pub fn compute_suggestions(
    query: &str,
    history: &[wadbrowser_core::HistoryEntry],
    bookmarks: &[wadbrowser_core::Bookmark],
    search_url: impl Fn(&str) -> String,
    limit: usize,
) -> Vec<Suggestion> {
    suggest(query, history, bookmarks, search_url, limit)
}

fn kind_label(kind: SuggestionKind) -> &'static str {
    match kind {
        SuggestionKind::History => "History",
        SuggestionKind::Bookmark => "Bookmark",
        SuggestionKind::Search => "Search",
        SuggestionKind::Url => "URL",
    }
}
