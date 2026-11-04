//! Where the desktop's icons sit (the spike's `Desktop.svelte` `place`,
//! `moveTo`, `placeAt` and `inBox`): a grid of spots from the top left, down
//! each column. An icon keeps the spot it was put in (while that's on the
//! screen and nobody earlier has it); the rest fill the free spots in order.
//! Pure, so it's tested here.

use std::collections::HashSet;

use indexmap::IndexMap;

/// A spot's size (an icon's is inside it), and the grid's margin.
pub const CW: f64 = 106.0;
pub const CH: f64 = 116.0;
pub const PAD_X: f64 = 10.0;
pub const PAD_Y: f64 = 12.0;
/// An icon's box, in its spot.
pub const ICON_W: f64 = 100.0;
pub const ICON_H: f64 = 112.0;

/// A column and row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Spot {
    pub c: u32,
    pub r: u32,
}

impl Spot {
    pub const fn new(c: u32, r: u32) -> Self {
        Self { c, r }
    }

    /// Its place in the order the free spots fill (down each column).
    fn index(self, rows: u32) -> u64 {
        self.c as u64 * rows as u64 + self.r as u64
    }

    fn at(i: u64, rows: u32) -> Self {
        Self { c: (i / rows as u64) as u32, r: (i % rows as u64) as u32 }
    }

    /// Its top left, in the desktop's pixels.
    pub fn origin(self) -> (f64, f64) {
        (PAD_X + self.c as f64 * CW, PAD_Y + self.r as f64 * CH)
    }
}

/// Every icon's spot, by its key, in the desktop's order.
pub type Spots = IndexMap<String, Spot>;

/// How many rows fit in a desktop `height` pixels tall.
pub fn rows_for(height: f64) -> u32 {
    (((height - PAD_Y - 16.0) / CH).floor() as i64).max(1) as u32
}

/// The spot under a point (the desktop's pixels), the rows it has.
pub fn spot_at(x: f64, y: f64, rows: u32) -> Spot {
    let c = ((x - PAD_X) / CW).floor().max(0.0) as u32;
    let r = (((y - PAD_Y) / CH).floor().max(0.0) as u32).min(rows.saturating_sub(1));
    Spot { c, r }
}

/// Each key's spot: where it was put (if that's on the screen and free),
/// else the next free one, down each column.
pub fn place(keys: &[String], saved: &Spots, rows: u32) -> Spots {
    let rows = rows.max(1);
    let mut out = Spots::new();
    let mut taken = HashSet::new();
    for k in keys {
        if let Some(&p) = saved.get(k)
            && p.r < rows
            && taken.insert(p)
        {
            out.insert(k.clone(), p);
        }
    }
    let mut i = 0u64;
    let mut placed = Spots::new();
    for k in keys {
        if let Some(&p) = out.get(k) {
            placed.insert(k.clone(), p);
            continue;
        }
        while taken.contains(&Spot::at(i, rows)) {
            i += 1;
        }
        let p = Spot::at(i, rows);
        taken.insert(p);
        placed.insert(k.clone(), p);
    }
    placed
}

/// `place`, with `fresh` ones (new on the desktop: made, untagged, back from
/// the Trash) after everything already there; nothing else moves for them.
pub fn place_fresh(keys: &[String], saved: &Spots, rows: u32, fresh: &[String]) -> Spots {
    let rows = rows.max(1);
    let rest: Vec<String> = keys.iter().filter(|k| !fresh.contains(k)).cloned().collect();
    let mut out = place(&rest, saved, rows);
    let after = out.values().map(|p| p.index(rows) + 1).max().unwrap_or(0);
    for (i, k) in (after..).zip(keys.iter().filter(|k| fresh.contains(k))) {
        out.insert(k.clone(), Spot::at(i, rows));
    }
    // In the desktop's order.
    keys.iter().filter_map(|k| out.get(k).map(|p| (k.clone(), *p))).collect()
}

/// `key` let go on `to`. Alone, one already there goes where it was; with
/// others (`group`, a selection it's in), they all move as far, and any in
/// their way are let go (they find free spots). None: it didn't move.
pub fn move_to(spots: &Spots, key: &str, to: Spot, group: &[String], rows: u32) -> Option<Spots> {
    let rows = rows.max(1);
    let to = Spot::new(to.c, to.r.min(rows - 1));
    let from = *spots.get(key)?;
    if from == to {
        return None;
    }
    let mut next = spots.clone();
    let group: Vec<&String> = group.iter().filter(|k| spots.contains_key(*k)).collect();
    if group.len() <= 1 {
        if let Some(there) = spots.iter().find(|(k, p)| k.as_str() != key && **p == to).map(|(k, _)| k.clone()) {
            next.insert(there, from);
        }
        next.insert(key.to_string(), to);
        return Some(next);
    }
    let (dc, dr) = (to.c as i64 - from.c as i64, to.r as i64 - from.r as i64);
    let moved: IndexMap<&String, Spot> = group
        .iter()
        .map(|k| {
            let p = spots[*k];
            let c = (p.c as i64 + dc).max(0) as u32;
            let r = (p.r as i64 + dr).clamp(0, rows as i64 - 1) as u32;
            (*k, Spot::new(c, r))
        })
        .collect();
    let landing: HashSet<Spot> = moved.values().copied().collect();
    for (k, p) in spots {
        if !moved.contains_key(k) && landing.contains(p) {
            next.shift_remove(k);
        }
    }
    let mut used = HashSet::new();
    for (k, p) in moved {
        // Two pushed against an edge onto one spot: the second finds a free one.
        if used.insert(p) {
            next.insert(k.clone(), p);
        } else {
            next.shift_remove(k);
        }
    }
    Some(next)
}

/// One not on the desktop yet (out of a folder) put at `to`; one there finds a free spot.
pub fn place_at(spots: &Spots, key: &str, to: Spot) -> Spots {
    let mut next: Spots = spots.iter().filter(|(_, p)| **p != to).map(|(k, p)| (k.clone(), *p)).collect();
    next.insert(key.to_string(), to);
    next
}

/// The keys whose icons a box (the desktop's pixels, any two corners) touches.
pub fn in_box(spots: &Spots, (x0, y0, x1, y1): (f64, f64, f64, f64)) -> Vec<String> {
    let (l, r) = (x0.min(x1), x0.max(x1));
    let (t, b) = (y0.min(y1), y0.max(y1));
    spots
        .iter()
        .filter(|(_, p)| {
            let (x, y) = p.origin();
            x < r && x + ICON_W > l && y < b && y + ICON_H > t
        })
        .map(|(k, _)| k.clone())
        .collect()
}

/// How many columns the icons take.
#[cfg(test)]
fn cols(spots: &Spots) -> u32 {
    spots.values().map(|p| p.c + 1).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn spots(v: &[(&str, u32, u32)]) -> Spots {
        v.iter().map(|(k, c, r)| (k.to_string(), Spot::new(*c, *r))).collect()
    }

    #[test]
    fn rows_fit_the_height() {
        // The spike's 800 px window less its bar: five rows.
        assert_eq!(rows_for(722.0), 5);
        // The Surface Book 2 less the bar.
        assert_eq!(rows_for(922.0), 7);
        assert_eq!(rows_for(10.0), 1);
    }

    #[test]
    fn unplaced_ones_fill_down_each_column() {
        let out = place(&keys(&["a", "b", "c", "d"]), &Spots::new(), 3);
        assert_eq!(out, spots(&[("a", 0, 0), ("b", 0, 1), ("c", 0, 2), ("d", 1, 0)]));
    }

    #[test]
    fn placed_ones_stay_and_the_rest_go_around_them() {
        let saved = spots(&[("c", 0, 0), ("a", 2, 1)]);
        let out = place(&keys(&["a", "b", "c", "d"]), &saved, 3);
        assert_eq!(out["c"], Spot::new(0, 0));
        assert_eq!(out["a"], Spot::new(2, 1));
        assert_eq!(out["b"], Spot::new(0, 1));
        assert_eq!(out["d"], Spot::new(0, 2));
        // In the desktop's order, whatever their spots.
        assert_eq!(out.keys().cloned().collect::<Vec<_>>(), keys(&["a", "b", "c", "d"]));
    }

    #[test]
    fn a_spot_off_the_screen_or_taken_is_let_go() {
        // Row 5 doesn't fit three rows; b's spot is a's (a comes first).
        let saved = spots(&[("a", 1, 0), ("b", 1, 0), ("c", 0, 5)]);
        let out = place(&keys(&["a", "b", "c"]), &saved, 3);
        assert_eq!(out["a"], Spot::new(1, 0));
        assert_eq!(out["b"], Spot::new(0, 0));
        assert_eq!(out["c"], Spot::new(0, 1));
    }

    #[test]
    fn fresh_ones_go_after_everything() {
        // The Trash at the end of column 1; a new wadspace goes after it, not
        // into the hole at (0, 1).
        let saved = spots(&[("ws:a", 0, 0), ("trash", 1, 2)]);
        let out = place_fresh(&keys(&["ws:a", "ws:new", "trash"]), &saved, 3, &keys(&["ws:new"]));
        assert_eq!(out["ws:a"], Spot::new(0, 0));
        assert_eq!(out["trash"], Spot::new(1, 2));
        assert_eq!(out["ws:new"], Spot::new(2, 0));
        assert_eq!(out.keys().cloned().collect::<Vec<_>>(), keys(&["ws:a", "ws:new", "trash"]));
    }

    #[test]
    fn one_moved_onto_another_swaps_with_it() {
        let s = spots(&[("a", 0, 0), ("b", 0, 1), ("c", 0, 2)]);
        let out = move_to(&s, "a", Spot::new(0, 2), &[], 3).unwrap();
        assert_eq!(out["a"], Spot::new(0, 2));
        assert_eq!(out["c"], Spot::new(0, 0));
        assert_eq!(out["b"], Spot::new(0, 1));
    }

    #[test]
    fn one_moved_to_a_free_spot_stays_there() {
        let s = spots(&[("a", 0, 0), ("b", 0, 1)]);
        let out = move_to(&s, "a", Spot::new(4, 1), &[], 3).unwrap();
        assert_eq!(out["a"], Spot::new(4, 1));
        assert_eq!(out["b"], Spot::new(0, 1));
        assert!(move_to(&s, "a", Spot::new(0, 0), &[], 3).is_none());
        // Below the last row: the last row.
        assert_eq!(move_to(&s, "a", Spot::new(2, 9), &[], 3).unwrap()["a"], Spot::new(2, 2));
    }

    #[test]
    fn a_selection_moves_together_and_bumps_what_is_in_its_way() {
        let s = spots(&[("a", 0, 0), ("b", 0, 1), ("c", 1, 1), ("d", 0, 2)]);
        let group = keys(&["a", "b"]);
        let out = move_to(&s, "a", Spot::new(1, 0), &group, 3).unwrap();
        assert_eq!(out["a"], Spot::new(1, 0));
        assert_eq!(out["b"], Spot::new(1, 1));
        // c was where b went: it's let go, to find a free spot.
        assert!(!out.contains_key("c"));
        assert_eq!(out["d"], Spot::new(0, 2));
        let placed = place(&keys(&["a", "b", "c", "d"]), &out, 3);
        assert_eq!(placed["c"], Spot::new(0, 0));
    }

    #[test]
    fn a_selection_pushed_against_the_edge_keeps_one_per_spot() {
        let s = spots(&[("a", 1, 0), ("b", 1, 1)]);
        // Up one row: a can't go above row 0, so b lands on a's spot.
        let out = move_to(&s, "b", Spot::new(1, 0), &keys(&["a", "b"]), 3).unwrap();
        assert_eq!(out["a"], Spot::new(1, 0));
        assert!(!out.contains_key("b"));
        let placed = place(&keys(&["a", "b"]), &out, 3);
        assert_eq!(placed["b"], Spot::new(0, 0));
    }

    #[test]
    fn out_of_a_folder_onto_a_spot_takes_it() {
        let s = spots(&[("a", 0, 0), ("b", 0, 1)]);
        let out = place_at(&s, "ws:x", Spot::new(0, 1));
        assert_eq!(out["ws:x"], Spot::new(0, 1));
        assert!(!out.contains_key("b"));
        let placed = place(&keys(&["a", "b", "ws:x"]), &out, 3);
        assert_eq!(placed["b"], Spot::new(0, 2));
    }

    #[test]
    fn points_and_boxes() {
        assert_eq!(spot_at(5.0, 5.0, 3), Spot::new(0, 0));
        assert_eq!(spot_at(PAD_X + CW * 2.5, PAD_Y + CH * 1.2, 3), Spot::new(2, 1));
        assert_eq!(spot_at(50.0, 5000.0, 3), Spot::new(0, 2));
        let s = spots(&[("a", 0, 0), ("b", 0, 1), ("c", 1, 0), ("d", 3, 3)]);
        // A box from inside a to inside b: those two.
        let (ax, ay) = Spot::new(0, 0).origin();
        let mut got = in_box(&s, (ax + 50.0, ay + 50.0, ax + 60.0, ay + CH + 10.0));
        got.sort();
        assert_eq!(got, keys(&["a", "b"]));
        // Corners either way round.
        let mut got = in_box(&s, (ax + CW + 90.0, ay + 10.0, ax + 5.0, ay + 5.0));
        got.sort();
        assert_eq!(got, keys(&["a", "c"]));
        assert!(in_box(&s, (2000.0, 2000.0, 2100.0, 2100.0)).is_empty());
        assert_eq!(cols(&s), 4);
    }
}
