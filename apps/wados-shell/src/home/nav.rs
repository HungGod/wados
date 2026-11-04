//! Where the arrow keys are in the launcher, and what each key does (no
//! GTK): the bar along the top (the machine, then the tabs), the cards, or
//! the buttons under the card in focus. The spike's `keys()` in Home.svelte.
//!
//! - ←/→ along the cards; ↑ to the bar, on the tab shown; ↓ to the buttons,
//!   ↓ again for the desktop; Enter opens the card in focus.
//! - In the bar, ←/→ along it, and a tab shows at once; ← past the first
//!   tab is the machine (Enter there for the next machine); ↓ or Enter back
//!   to the cards.
//! - In the buttons, ←/→ along them, Enter does the one it's on, ↑ (or Esc)
//!   back to the cards, ↓ the desktop.
//! - Tab and ⇧Tab: the next or last tab, from anywhere.

/// Which part the keys are in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Zone {
    Top,
    #[default]
    Cards,
    Actions,
}

impl Zone {
    pub fn id(self) -> &'static str {
        match self {
            Zone::Top => "top",
            Zone::Cards => "cards",
            Zone::Actions => "actions",
        }
    }
}

/// The keys the launcher takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Enter,
    Space,
    Escape,
    Tab,
    BackTab,
}

impl Key {
    pub fn parse(s: &str) -> Option<Key> {
        Some(match s.to_ascii_lowercase().as_str() {
            "left" => Key::Left,
            "right" => Key::Right,
            "up" => Key::Up,
            "down" => Key::Down,
            "enter" | "return" => Key::Enter,
            "space" => Key::Space,
            "escape" | "esc" => Key::Escape,
            "tab" => Key::Tab,
            "backtab" | "shift+tab" => Key::BackTab,
            _ => return None,
        })
    }
}

/// What the launcher has, as the keys need it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Have {
    /// How many tabs.
    pub tabs: usize,
    /// The tab shown (its place).
    pub view: usize,
    /// How many cards, and the one in focus.
    pub cards: usize,
    pub at: usize,
    /// How many buttons the card in focus has.
    pub actions: usize,
}

/// What a key asks the launcher to do (besides moving the keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Do {
    /// Only where the keys are changed.
    Moved,
    /// Show this tab (by its place), from its first card.
    Pick(usize),
    /// This card in focus.
    Focus(usize),
    /// Open the card in focus.
    Open,
    /// The button (by its place).
    Act(usize),
    /// The next machine's wadspaces.
    NextMachine,
    /// Down to the desktop.
    Desktop,
}

/// Where the keys are.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Nav {
    pub zone: Zone,
    /// In the bar: the machine (0), or a tab (1 on, by its place).
    pub top: usize,
    /// The button they're on.
    pub act: usize,
}

impl Nav {
    /// `k` pressed: what it does, or `None` if the launcher doesn't take it.
    pub fn key(&mut self, k: Key, h: &Have) -> Option<Do> {
        if h.tabs == 0 {
            return None;
        }
        if let Key::Tab | Key::BackTab = k {
            let i = (h.view + if k == Key::Tab { 1 } else { h.tabs - 1 }) % h.tabs;
            self.top = i + 1;
            return Some(self.pick(i, h));
        }
        match self.zone {
            Zone::Top => match k {
                Key::Left | Key::Right => {
                    self.top = if k == Key::Left { self.top.saturating_sub(1) } else { (self.top + 1).min(h.tabs) };
                    Some(if self.top > 0 { self.pick(self.top - 1, h) } else { Do::Moved })
                }
                Key::Down => {
                    self.zone = Zone::Cards;
                    Some(Do::Moved)
                }
                Key::Enter | Key::Space if self.top == 0 => Some(Do::NextMachine),
                Key::Enter | Key::Space => {
                    self.zone = Zone::Cards;
                    Some(Do::Moved)
                }
                _ => None,
            },
            Zone::Actions => match k {
                Key::Left => {
                    self.act = self.act.saturating_sub(1);
                    Some(Do::Moved)
                }
                Key::Right => {
                    self.act = (self.act + 1).min(h.actions.saturating_sub(1));
                    Some(Do::Moved)
                }
                Key::Up | Key::Escape => {
                    self.zone = Zone::Cards;
                    Some(Do::Moved)
                }
                Key::Down => Some(Do::Desktop),
                Key::Enter => Some(if self.act < h.actions { Do::Act(self.act) } else { Do::Moved }),
                _ => None,
            },
            Zone::Cards => match k {
                Key::Left => Some(Do::Focus(h.at.saturating_sub(1))),
                Key::Right => Some(Do::Focus((h.at + 1).min(h.cards.saturating_sub(1)))),
                Key::Up => {
                    self.zone = Zone::Top;
                    self.top = h.view + 1;
                    Some(Do::Moved)
                }
                Key::Down if h.actions > 0 => {
                    self.zone = Zone::Actions;
                    self.act = self.act.min(h.actions - 1);
                    Some(Do::Moved)
                }
                Key::Down => Some(Do::Desktop),
                Key::Enter => Some(if h.cards > 0 { Do::Open } else { Do::Moved }),
                _ => None,
            },
        }
    }

    /// Another tab: from its first card, the first button.
    fn pick(&mut self, i: usize, h: &Have) -> Do {
        if i == h.view {
            return Do::Moved;
        }
        self.act = 0;
        Do::Pick(i)
    }

    /// The card in focus changed (or its buttons did): the keys stay on a button there is.
    pub fn settle(&mut self, actions: usize) {
        if self.zone == Zone::Actions {
            if actions == 0 {
                self.zone = Zone::Cards;
            } else {
                self.act = self.act.min(actions - 1);
            }
        }
    }

    /// Back on the page (from the desktop, say): the keys are on the cards.
    pub fn shown(&mut self) {
        self.zone = Zone::Cards;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seven tabs, Work (2) shown, six cards with the third in focus, six buttons.
    fn have() -> Have {
        Have { tabs: 7, view: 2, cards: 6, at: 2, actions: 6 }
    }

    #[test]
    fn left_and_right_move_along_the_cards_and_stop_at_the_ends() {
        let mut n = Nav::default();
        assert_eq!(n.key(Key::Left, &have()), Some(Do::Focus(1)));
        assert_eq!(n.key(Key::Right, &have()), Some(Do::Focus(3)));
        assert_eq!(n.key(Key::Left, &Have { at: 0, ..have() }), Some(Do::Focus(0)));
        assert_eq!(n.key(Key::Right, &Have { at: 5, ..have() }), Some(Do::Focus(5)));
        assert_eq!(n.key(Key::Enter, &have()), Some(Do::Open));
        assert_eq!(n.key(Key::Enter, &Have { cards: 0, at: 0, actions: 0, ..have() }), Some(Do::Moved));
        // Not the launcher's: Esc and Space on the cards.
        assert_eq!(n.key(Key::Escape, &have()), None);
        assert_eq!(n.key(Key::Space, &have()), None);
    }

    #[test]
    fn up_goes_to_the_bar_on_the_tab_shown_and_a_tab_shows_at_once() {
        let mut n = Nav::default();
        assert_eq!(n.key(Key::Up, &have()), Some(Do::Moved));
        assert_eq!((n.zone, n.top), (Zone::Top, 3));
        assert_eq!(n.key(Key::Right, &have()), Some(Do::Pick(3)));
        assert_eq!(n.top, 4);
        // Past the last, it stays.
        let mut end = Nav { zone: Zone::Top, top: 7, act: 0 };
        assert_eq!(end.key(Key::Right, &Have { view: 6, ..have() }), Some(Do::Moved));
        assert_eq!(end.top, 7);
        // ← past the first tab is the machine; Enter there for the next machine.
        let mut n = Nav { zone: Zone::Top, top: 1, act: 0 };
        assert_eq!(n.key(Key::Left, &Have { view: 0, ..have() }), Some(Do::Moved));
        assert_eq!(n.top, 0);
        assert_eq!(n.key(Key::Left, &Have { view: 0, ..have() }), Some(Do::Moved));
        assert_eq!(n.key(Key::Enter, &have()), Some(Do::NextMachine));
        assert_eq!(n.zone, Zone::Top);
        // On a tab, Enter (or ↓) goes back to the cards.
        n.top = 2;
        assert_eq!(n.key(Key::Enter, &have()), Some(Do::Moved));
        assert_eq!(n.zone, Zone::Cards);
        n.zone = Zone::Top;
        assert_eq!(n.key(Key::Down, &have()), Some(Do::Moved));
        assert_eq!(n.zone, Zone::Cards);
    }

    #[test]
    fn down_goes_to_the_buttons_then_the_desktop() {
        let mut n = Nav::default();
        assert_eq!(n.key(Key::Down, &have()), Some(Do::Moved));
        assert_eq!((n.zone, n.act), (Zone::Actions, 0));
        assert_eq!(n.key(Key::Right, &have()), Some(Do::Moved));
        assert_eq!(n.key(Key::Right, &have()), Some(Do::Moved));
        assert_eq!(n.key(Key::Enter, &have()), Some(Do::Act(2)));
        for _ in 0..9 {
            n.key(Key::Right, &have());
        }
        assert_eq!(n.act, 5);
        assert_eq!(n.key(Key::Down, &have()), Some(Do::Desktop));
        assert_eq!(n.key(Key::Up, &have()), Some(Do::Moved));
        assert_eq!(n.zone, Zone::Cards);
        // Back down: on the button it was on.
        n.key(Key::Down, &have());
        assert_eq!(n.act, 5);
        assert_eq!(n.key(Key::Escape, &have()), Some(Do::Moved));
        assert_eq!(n.zone, Zone::Cards);
        // No card, no buttons: ↓ is the desktop.
        assert_eq!(n.key(Key::Down, &Have { cards: 0, at: 0, actions: 0, ..have() }), Some(Do::Desktop));
    }

    #[test]
    fn tab_and_shift_tab_go_round_the_tabs_from_anywhere() {
        let mut n = Nav { zone: Zone::Actions, top: 0, act: 4 };
        assert_eq!(n.key(Key::Tab, &have()), Some(Do::Pick(3)));
        // The keys stay where they were, on the first button.
        assert_eq!((n.zone, n.top, n.act), (Zone::Actions, 4, 0));
        assert_eq!(n.key(Key::Tab, &Have { view: 6, ..have() }), Some(Do::Pick(0)));
        assert_eq!(n.key(Key::BackTab, &Have { view: 0, ..have() }), Some(Do::Pick(6)));
        assert_eq!(n.top, 7);
    }

    #[test]
    fn another_card_keeps_the_keys_on_a_button_there_is() {
        let mut n = Nav { zone: Zone::Actions, top: 0, act: 5 };
        n.settle(3);
        assert_eq!(n.act, 2);
        n.settle(0);
        assert_eq!(n.zone, Zone::Cards);
        n.zone = Zone::Top;
        n.shown();
        assert_eq!(n.zone, Zone::Cards);
    }

    #[test]
    fn key_names_for_wadsctl() {
        assert_eq!(Key::parse("Left"), Some(Key::Left));
        assert_eq!(Key::parse("shift+tab"), Some(Key::BackTab));
        assert_eq!(Key::parse("f5"), None);
    }
}
