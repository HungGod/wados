//! Locking before the machine sleeps: logind's delay-inhibitor pattern, as
//! logic only (`mod.rs` wires it to logind and the lock screen; the tests
//! feed it fake signals).
//!
//! While it's awake the shell holds a "sleep" delay inhibitor. When logind
//! says it's going to sleep (`PrepareForSleep(true)`: the lid, the suspend
//! key, an idle suspend), the shell locks the screen and lets the inhibitor
//! go once the lock is showing; logind waits for that (up to its
//! `InhibitDelayMaxSec`). Woken (`PrepareForSleep(false)`), it takes one again.

/// What to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Do {
    /// Lock the screen, then call [`Sleep::locked`].
    Lock,
    /// Ask logind for the inhibitor (if [`Sleep::ask`] says so).
    Inhibit,
}

/// The inhibitor (`T`: logind's fd; dropping it lets sleep go on) and where
/// sleep is.
#[derive(Debug)]
pub struct Sleep<T> {
    held: Option<T>,
    asking: bool,
    going: bool,
    /// Why logind refused the last time.
    refused: Option<String>,
}

impl<T> Default for Sleep<T> {
    fn default() -> Self {
        Self { held: None, asking: false, going: false, refused: None }
    }
}

impl<T> Sleep<T> {
    /// Whether to ask logind for an inhibitor now: not while one's held or
    /// asked for, nor while the machine is going to sleep.
    pub fn ask(&mut self) -> bool {
        if self.held.is_some() || self.asking || self.going {
            return false;
        }
        self.asking = true;
        true
    }

    /// logind gave it. If sleep started meanwhile, it goes at once.
    pub fn inhibited(&mut self, fd: T) {
        self.asking = false;
        self.refused = None;
        if !self.going {
            self.held = Some(fd);
        }
    }

    /// logind refused it (or didn't answer).
    pub fn refused(&mut self, why: String) {
        self.asking = false;
        self.refused = Some(why);
    }

    /// logind's `PrepareForSleep`: `going` to sleep, or woken.
    pub fn prepare(&mut self, going: bool) -> Do {
        self.going = going;
        if going { Do::Lock } else { Do::Inhibit }
    }

    /// The screen is locked (or it couldn't be): sleep can go on.
    pub fn locked(&mut self) {
        if self.going {
            self.held = None;
        }
    }

    /// For `wadsctl session`.
    pub fn state(&self) -> String {
        match (&self.held, self.asking, self.going, &self.refused) {
            (Some(_), ..) => "held".into(),
            (None, _, true, _) => "released".into(),
            (None, true, ..) => "asking".into(),
            (None, false, false, Some(why)) => format!("refused({why})"),
            (None, false, false, None) => "none".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gtk::glib::{self, prelude::*};

    use super::*;
    use crate::session::logind::prepare_for_sleep;

    /// An inhibitor that says when it's let go (as closing logind's fd would).
    struct Fd(Rc<Cell<bool>>);
    impl Drop for Fd {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    fn fd() -> (Fd, Rc<Cell<bool>>) {
        let gone = Rc::new(Cell::new(false));
        (Fd(gone.clone()), gone)
    }

    /// The signal as it comes off the bus: `PrepareForSleep(b)`.
    fn signal(going: bool) -> bool {
        prepare_for_sleep(&(going,).to_variant()).expect("a PrepareForSleep signal")
    }

    #[test]
    fn reads_logind_s_signal() {
        assert_eq!(prepare_for_sleep(&(true,).to_variant()), Some(true));
        assert_eq!(prepare_for_sleep(&(false,).to_variant()), Some(false));
        assert_eq!(prepare_for_sleep(&("nope",).to_variant()), None);
        assert_eq!(prepare_for_sleep(&glib::Variant::from(true)), None);
    }

    #[test]
    fn holds_it_until_the_screen_is_locked() {
        let mut s = Sleep::default();
        assert_eq!(s.state(), "none");
        assert!(s.ask());
        assert!(!s.ask(), "one request at a time");
        assert_eq!(s.state(), "asking");
        let (first, first_gone) = fd();
        s.inhibited(first);
        assert_eq!(s.state(), "held");
        assert!(!s.ask(), "it's held");

        // The lid closes.
        assert_eq!(s.prepare(signal(true)), Do::Lock);
        assert!(!first_gone.get(), "held while the lock screen comes up");
        assert!(!s.ask(), "not while it's going to sleep");
        s.locked();
        assert!(first_gone.get(), "let go once it's locked");
        assert_eq!(s.state(), "released");

        // It wakes: a new one.
        assert_eq!(s.prepare(signal(false)), Do::Inhibit);
        assert!(s.ask());
        let (second, second_gone) = fd();
        s.inhibited(second);
        assert_eq!(s.state(), "held");
        assert!(!second_gone.get());
    }

    #[test]
    fn a_lock_with_no_sleep_coming_keeps_it() {
        let mut s = Sleep::default();
        assert!(s.ask());
        let (f, gone) = fd();
        s.inhibited(f);
        // Super+L, or the idle lock: nothing to do with sleep.
        s.locked();
        assert!(!gone.get());
        assert_eq!(s.state(), "held");
    }

    #[test]
    fn one_that_comes_after_sleep_started_goes_at_once() {
        let mut s = Sleep::default();
        assert!(s.ask());
        assert_eq!(s.prepare(signal(true)), Do::Lock);
        let (late, gone) = fd();
        s.inhibited(late);
        assert!(gone.get(), "it mustn't hold up a sleep that's already started");
        assert_eq!(s.state(), "released");
    }

    #[test]
    fn refused_then_asked_again_on_waking() {
        let mut s: Sleep<Fd> = Sleep::default();
        assert!(s.ask());
        s.refused("Access denied".into());
        assert_eq!(s.state(), "refused(Access denied)");
        // Sleep comes anyway: it still locks.
        assert_eq!(s.prepare(signal(true)), Do::Lock);
        s.locked();
        assert_eq!(s.prepare(signal(false)), Do::Inhibit);
        assert!(s.ask());
    }
}
