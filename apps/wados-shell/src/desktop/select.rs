//! What's selected on the desktop, and how a press, a click, a box or a key
//! changes it (the spike's `pressIcon`, `clickIcon`, `pickFor`, `startBand`).
//! Icons by key, in the order they were picked. Pure, so it's tested here.

/// The modifiers a press came with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    /// Ctrl (or Super): toggles one.
    pub ctrl: bool,
    /// Shift: adds one.
    pub shift: bool,
}

impl Mods {
    pub fn any(self) -> bool {
        self.ctrl || self.shift
    }
}

/// An icon pressed: Ctrl toggles it, Shift adds it; one already selected
/// keeps the selection (to drag them all); else it alone.
pub fn press(sel: &[String], key: &str, m: Mods) -> Vec<String> {
    let has = sel.iter().any(|k| k == key);
    if m.ctrl {
        if has { sel.iter().filter(|k| *k != key).cloned().collect() } else { with(sel, key) }
    } else if m.shift || has {
        if has { sel.to_vec() } else { with(sel, key) }
    } else {
        vec![key.to_string()]
    }
}

/// Clicked (let go where it was pressed, no Ctrl or Shift): it alone.
pub fn click(sel: &[String], key: &str, m: Mods, moved: bool) -> Vec<String> {
    if m.any() || moved { sel.to_vec() } else { vec![key.to_string()] }
}

/// Right-clicked: one not selected is selected alone (its menu is for it).
pub fn pick_for(sel: &[String], key: &str) -> Vec<String> {
    if sel.iter().any(|k| k == key) { sel.to_vec() } else { vec![key.to_string()] }
}

/// What a drag from `key` carries: the selection (it among them, first), or it alone.
pub fn carried(sel: &[String], key: &str) -> Vec<String> {
    if sel.iter().any(|k| k == key) {
        std::iter::once(key.to_string()).chain(sel.iter().filter(|k| *k != key).cloned()).collect()
    } else {
        vec![key.to_string()]
    }
}

/// What a box across the background starts from: with Ctrl or Shift, what's
/// selected; else nothing.
pub fn band_base(sel: &[String], m: Mods) -> Vec<String> {
    if m.any() { sel.to_vec() } else { Vec::new() }
}

/// The box's selection: what it started from and what it touches.
pub fn band(base: &[String], touched: &[String]) -> Vec<String> {
    let mut out = base.to_vec();
    for k in touched {
        if !out.contains(k) {
            out.push(k.clone());
        }
    }
    out
}

/// Ones gone from the desktop (trashed, tagged into a folder) aren't selected.
pub fn prune(sel: &[String], keys: &[String]) -> Vec<String> {
    sel.iter().filter(|k| keys.contains(k)).cloned().collect()
}

fn with(sel: &[String], key: &str) -> Vec<String> {
    let mut v = sel.to_vec();
    v.push(key.to_string());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    const NONE: Mods = Mods { ctrl: false, shift: false };
    const CTRL: Mods = Mods { ctrl: true, shift: false };
    const SHIFT: Mods = Mods { ctrl: false, shift: true };

    #[test]
    fn a_plain_press_picks_it_alone_unless_selected() {
        assert_eq!(press(&v(&["a", "b"]), "c", NONE), v(&["c"]));
        // Already selected: they stay, to be dragged together.
        assert_eq!(press(&v(&["a", "b"]), "b", NONE), v(&["a", "b"]));
        // Let go without moving: it alone.
        assert_eq!(click(&v(&["a", "b"]), "b", NONE, false), v(&["b"]));
        // Dragged: they stay.
        assert_eq!(click(&v(&["a", "b"]), "b", NONE, true), v(&["a", "b"]));
    }

    #[test]
    fn ctrl_toggles_and_shift_adds() {
        assert_eq!(press(&v(&["a"]), "b", CTRL), v(&["a", "b"]));
        assert_eq!(press(&v(&["a", "b"]), "a", CTRL), v(&["b"]));
        assert_eq!(press(&v(&["a"]), "b", SHIFT), v(&["a", "b"]));
        assert_eq!(press(&v(&["a", "b"]), "a", SHIFT), v(&["a", "b"]));
        // A click with them doesn't narrow it.
        assert_eq!(click(&v(&["a", "b"]), "b", CTRL, false), v(&["a", "b"]));
    }

    #[test]
    fn right_click_and_drags() {
        assert_eq!(pick_for(&v(&["a", "b"]), "b"), v(&["a", "b"]));
        assert_eq!(pick_for(&v(&["a", "b"]), "c"), v(&["c"]));
        assert_eq!(carried(&v(&["a", "b", "c"]), "b"), v(&["b", "a", "c"]));
        assert_eq!(carried(&v(&["a", "b"]), "z"), v(&["z"]));
    }

    #[test]
    fn boxes_add_to_the_base() {
        assert_eq!(band_base(&v(&["a"]), NONE), Vec::<String>::new());
        assert_eq!(band_base(&v(&["a"]), SHIFT), v(&["a"]));
        assert_eq!(band(&v(&["a"]), &v(&["b", "a", "c"])), v(&["a", "b", "c"]));
        assert_eq!(prune(&v(&["a", "gone", "c"]), &v(&["a", "b", "c"])), v(&["a", "c"]));
    }
}
