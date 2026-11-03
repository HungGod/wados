// Tags organize wadspaces on the desktop: each wadspace has one (its
// `tag`), and the desktop shows a group per tag: General (the default, for
// the rest) first, then the tags made, in order. A new tag is an empty group
// to drag wadspaces into.

use crate::types::{Tag, Wadspace};

/// General: the tag of every wadspace without one (it isn't one of the library's tags).
pub const GENERAL_ID: &str = "general";
pub const GENERAL_NAME: &str = "General";
pub const GENERAL_COLOR: &str = "#9a93ad";

pub fn general() -> Tag {
    Tag { id: GENERAL_ID.to_string(), name: GENERAL_NAME.to_string(), color: GENERAL_COLOR.to_string() }
}

/// The colours new tags take, in turn.
pub const TAG_COLORS: [&str; 8] =
    ["#7fd3ff", "#c6ff1f", "#ff3d81", "#ffb36b", "#b48cff", "#5ef2c6", "#ffe36b", "#ff7a7a"];

/// Its tag's id (General's, if it has none, or one that's gone).
pub fn tag_of<'a>(tags: &[Tag], x: &'a Wadspace) -> &'a str {
    match x.tag.as_deref() {
        Some(t) if tags.iter().any(|y| y.id == t) => t,
        _ => GENERAL_ID,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TagGroup<'a> {
    pub tag: Tag,
    pub spaces: Vec<&'a Wadspace>,
}

/// `list` by tag: General first, then every tag (empty ones too), each in order.
pub fn tag_groups<'a>(tags: &[Tag], list: &[&'a Wadspace]) -> Vec<TagGroup<'a>> {
    std::iter::once(general())
        .chain(tags.iter().cloned())
        .map(|tag| {
            let mut spaces: Vec<&Wadspace> = list.iter().copied().filter(|x| tag_of(tags, x) == tag.id).collect();
            spaces.sort_by_key(|x| x.order);
            TagGroup { tag, spaces }
        })
        .collect()
}
