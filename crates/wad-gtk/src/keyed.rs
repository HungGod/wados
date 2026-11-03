//! A list of widgets kept in step with a list of items by key: an item
//! that stays keeps its widget (and whatever it's animating), one that
//! changed is updated, new ones are made and gone ones dropped. What React
//! does for a `.map()` with keys.

use std::collections::HashMap;
use std::hash::Hash;

use gtk::prelude::*;

pub struct Keyed<K, T, W> {
    container: gtk::Box,
    rows: Vec<(K, T, W)>,
}

impl<K: Eq + Hash + Clone, T: Clone + PartialEq, W: IsA<gtk::Widget>> Keyed<K, T, W> {
    pub fn new(container: gtk::Box) -> Self {
        Self { container, rows: Vec::new() }
    }

    pub fn container(&self) -> &gtk::Box {
        &self.container
    }

    pub fn widgets(&self) -> impl Iterator<Item = (&K, &W)> {
        self.rows.iter().map(|(k, _, w)| (k, w))
    }

    /// Shows `items` in order: `make` for a new one, `update` for one that changed.
    pub fn set(&mut self, items: &[(K, T)], make: impl Fn(&T) -> W, update: impl Fn(&W, &T)) {
        let mut old: HashMap<K, (T, W)> = self.rows.drain(..).map(|(k, t, w)| (k, (t, w))).collect();
        let mut rows = Vec::with_capacity(items.len());
        for (k, t) in items {
            let w = match old.remove(k) {
                Some((was, w)) => {
                    if was != *t {
                        update(&w, t);
                    }
                    w
                }
                None => make(t),
            };
            rows.push((k.clone(), t.clone(), w));
        }
        for (_, (_, w)) in old {
            self.container.remove(&w);
        }
        // In order: each after the one before it.
        let mut prev: Option<gtk::Widget> = None;
        for (_, _, w) in &rows {
            let w = w.upcast_ref::<gtk::Widget>();
            if w.parent().is_none() {
                self.container.insert_child_after(w, prev.as_ref());
            } else {
                self.container.reorder_child_after(w, prev.as_ref());
            }
            prev = Some(w.clone());
        }
        self.rows = rows;
    }
}
