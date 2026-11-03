// Ids and names that aren't taken yet, as the spike makes them: slugs for
// ids ("cosmic-bodybuilding-2"), "Name (copy)" for a wadspace's or a tag's
// name, "Name 2" for a variant's. A name that grows past the longest a name
// can be is cut short first, so it stays one.

use std::cmp::Ordering;

use crate::check::MAX_NAME;

/// `s` lowercased, with each run of anything but a-z and 0-9 a '-', none at the ends ("Mario Kart: Double Dash!!" → "mario-kart-double-dash").
pub fn slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut dash = false;
    for c in s.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c);
        } else {
            dash = true;
        }
    }
    out
}

/// `base`, or `base-2`, `base-3`… whichever isn't taken.
pub fn new_id<S: AsRef<str>>(taken: &[S], base: &str) -> String {
    let has = |s: &str| taken.iter().any(|t| t.as_ref() == s);
    if !has(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base}-{i}")).find(|s| !has(s)).unwrap_or_default()
}

/// A slug id from a name that isn't taken ("cosmic-bodybuilding-2"): at most 40 characters before its number; 'wadspace' if the name has none.
pub fn free_id<S: AsRef<str>>(taken: &[S], name: &str) -> String {
    let mut base: String = slug(name).chars().take(40).collect();
    if base.is_empty() {
        base = "wadspace".to_string();
    }
    new_id(taken, &base)
}

/// A name that isn't taken: "Name", then "Name (copy)", "Name (copy 2)" (a copy's copy is "Name (copy 2)", not "Name (copy) (copy)").
pub fn copy_name<S: AsRef<str>>(taken: &[S], name: &str) -> String {
    let has = |s: &str| taken.iter().any(|t| t.as_ref() == s);
    if !has(name) {
        return name.to_string();
    }
    let base = strip_copy(name);
    (1..)
        .map(|i| if i == 1 { " (copy)".to_string() } else { format!(" (copy {i})") })
        .map(|suffix| fit(base, &suffix))
        .find(|s| !has(s))
        .unwrap_or_default()
}

/// A name that isn't taken among `taken`: "Name", then "Name 2", "Name 3".
pub fn numbered_name<S: AsRef<str>>(taken: &[S], name: &str) -> String {
    let has = |s: &str| taken.iter().any(|t| t.as_ref() == s);
    if !has(name) {
        return name.to_string();
    }
    (2..).map(|i| fit(name, &format!(" {i}"))).find(|s| !has(s)).unwrap_or_default()
}

/// "Name (copy)" or "Name (copy 3)" → "Name".
fn strip_copy(name: &str) -> &str {
    let Some(head) = name.strip_suffix(')') else { return name };
    let Some(at) = head.rfind(" (copy") else { return name };
    let rest = &head[at + " (copy".len()..];
    let numbered = rest.strip_prefix(' ').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    if rest.is_empty() || numbered { &name[..at] } else { name }
}

/// `base` then `suffix`, `base` cut short so the two are a name still.
fn fit(base: &str, suffix: &str) -> String {
    let room = MAX_NAME.saturating_sub(suffix.chars().count());
    let cut: String = base.chars().take(room).collect();
    format!("{}{suffix}", cut.trim_end())
}

/// At most `n` characters of `s`.
pub fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Names in the order a person reads them (roughly `localeCompare`): letters before case.
pub fn locale_cmp(a: &str, b: &str) -> Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| b.cmp(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_ids() {
        assert_eq!(slug("Mario Kart: Double Dash!!"), "mario-kart-double-dash");
        assert_eq!(slug("--Hello  World--"), "hello-world");
        assert_eq!(slug("!!!"), "");
        assert_eq!(free_id(&["cosmic"], "Cosmic"), "cosmic-2");
        assert_eq!(free_id(&["cosmic", "cosmic-2"], "Cosmic"), "cosmic-3");
        assert_eq!(free_id::<&str>(&[], "!!!"), "wadspace");
        assert_eq!(free_id::<&str>(&[], &"a".repeat(60)).len(), 40);
        assert_eq!(new_id(&["variant"], "variant"), "variant-2");
    }

    #[test]
    fn copies_and_numbers() {
        assert_eq!(copy_name(&["VanuaAcademy"], "VanuaAcademy"), "VanuaAcademy (copy)");
        assert_eq!(copy_name(&["A", "A (copy)"], "A"), "A (copy 2)");
        assert_eq!(copy_name(&["A", "A (copy)", "A (copy 2)"], "A (copy)"), "A (copy 3)");
        assert_eq!(copy_name(&["New tag"], "New"), "New");
        assert_eq!(numbered_name(&["Mario Kart"], "Mario Kart"), "Mario Kart 2");
        // Never longer than a name can be.
        let long = "x".repeat(80);
        assert_eq!(copy_name(&[long.as_str()], &long).chars().count(), 80);
        assert!(copy_name(&[long.as_str()], &long).ends_with(" (copy)"));
    }

    #[test]
    fn reads_like_locale_compare() {
        let mut names = vec!["Work", "hunggod", "Steam", "Notes vault"];
        names.sort_by(|a, b| locale_cmp(a, b));
        assert_eq!(names, ["hunggod", "Notes vault", "Steam", "Work"]);
    }
}
