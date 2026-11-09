//! Where each wadspace's window goes, from how wadd says it's shown (its
//! run's display): **screen** is full screen on the primary display (the
//! launcher and desktop are under it), **window** an ordinary window,
//! **background** minimized (it keeps running). wadd starts the container;
//! the shell places its window (docs/decisions/0002). Open on 2 and To 2 say
//! another display ([`Placer::set_output`]). A window is a
//! wadspace's when its app id is `io.wadspaces.<id>` (0001's labwc patch).
//!
//! **Every display.** A wadspace opened on the screen draws one window per
//! display (svc-de's `screens`, wadd's `WLR_WL_OUTPUTS`), so its desktop
//! spans them all: each of its windows goes full screen on a display of its
//! own, in the shell's order (the primary first). One window (a stream, an
//! older image) is full screen on [`Placer::output_of`]'s, as before.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use wad_model::SessionDisplay;

use crate::store::Store;
use crate::wm::{Cmd, Top, Which, Wm};

pub struct Placer {
    wm: Option<Rc<Wm>>,
    store: Rc<Store>,
    /// The primary display's connector ("eDP-1", "HEADLESS-1").
    primary: RefCell<String>,
    /// Where a full-screen wadspace goes when that isn't the primary (Open on 2, To 2).
    outputs: RefCell<HashMap<String, String>>,
    /// The displays, primary first (their connectors), as the shell has them.
    displays: RefCell<Vec<String>>,
    windows: RefCell<Vec<Top>>,
    /// What each window was last told (by its toplevel id), so it's told once:
    /// how it's shown, and the display it was put on.
    applied: RefCell<HashMap<u32, (SessionDisplay, Option<String>)>>,
    /// Told each time the windows change (`on_windows`).
    on_windows: RefCell<Vec<OnWindows>>,
}

type OnWindows = Box<dyn Fn(&[Top])>;

/// `first` and then the rest of `displays`, each once: the order a
/// wadspace's windows go in, its own display first.
fn order_displays(displays: &[String], first: &str) -> Vec<String> {
    let mut all = displays.to_vec();
    all.retain(|c| c != first);
    all.insert(0, first.to_string());
    all
}

impl Placer {
    pub fn new(wm: Option<Rc<Wm>>, store: Rc<Store>, primary: &str) -> Rc<Self> {
        Rc::new(Self {
            wm,
            store,
            primary: RefCell::new(primary.to_string()),
            outputs: RefCell::default(),
            displays: RefCell::default(),
            windows: RefCell::default(),
            applied: RefCell::default(),
            on_windows: RefCell::default(),
        })
    }

    /// Calls `f` with every window each time they change (after they're placed).
    /// The primary display is another now (monitors.rs).
    pub fn set_primary(&self, connector: &str) {
        *self.primary.borrow_mut() = connector.to_string();
    }

    /// The displays, primary first (their connectors), as the shell has them:
    /// a wadspace on the screen draws one window per display.
    pub fn set_displays(&self, connectors: Vec<String>) {
        let was = self.displays.borrow().clone();
        if was == connectors {
            return;
        }
        *self.displays.borrow_mut() = connectors;
        // Everything goes where it's meant to be again (their order changed).
        self.applied.borrow_mut().clear();
        self.place_all();
    }

    /// The displays, primary first: the one `ws` goes to, then the others.
    fn from_output(&self, ws: &str) -> Vec<String> {
        order_displays(&self.displays.borrow(), &self.output_of(ws))
    }

    /// `ws` full screen on this display from now on (none: the primary).
    pub fn set_output(&self, ws: &str, connector: Option<&str>) {
        let mut o = self.outputs.borrow_mut();
        match connector {
            Some(c) => o.insert(ws.to_string(), c.to_string()),
            None => o.remove(ws),
        };
    }

    /// The display `ws` is full screen on.
    pub fn output_of(&self, ws: &str) -> String {
        self.outputs.borrow().get(ws).cloned().unwrap_or_else(|| self.primary.borrow().clone())
    }

    /// The wadspaces sent to `connector`: they're forgotten, for it's gone.
    pub fn take_output(&self, connector: &str) -> Vec<String> {
        let mut o = self.outputs.borrow_mut();
        let on: Vec<String> = o.iter().filter(|(_, c)| *c == connector).map(|(w, _)| w.clone()).collect();
        o.retain(|_, c| c != connector);
        on
    }

    pub fn on_windows(&self, f: impl Fn(&[Top]) + 'static) {
        self.on_windows.borrow_mut().push(Box::new(f));
    }

    /// Every window on the machine, as foreign-toplevel last said.
    pub fn windows(&self) -> Vec<Top> {
        self.windows.borrow().clone()
    }

    /// A wadspace's windows (one per display its labwc draws on).
    pub fn windows_of(&self, ws: &str) -> Vec<Top> {
        self.windows.borrow().iter().filter(|t| t.wadspace() == Some(ws)).cloned().collect()
    }

    pub fn wm(&self) -> Option<&Rc<Wm>> {
        self.wm.as_ref()
    }

    /// foreign-toplevel's list changed.
    pub fn windows_changed(&self, list: Vec<Top>) {
        let ids: Vec<u32> = list.iter().map(|t| t.id).collect();
        self.applied.borrow_mut().retain(|id, _| ids.contains(id));
        *self.windows.borrow_mut() = list;
        self.place_all();
        let list = self.windows();
        for f in self.on_windows.borrow().iter() {
            f(&list);
        }
    }

    /// A wadspace's run changed (it may be shown another way now).
    pub fn run_changed(&self, ws: &str) {
        {
            let mut applied = self.applied.borrow_mut();
            for t in self.windows_of(ws) {
                applied.remove(&t.id);
            }
        }
        // What it is now, which may be more or fewer windows than before.
        self.place_all();
    }

    /// Puts `ws` in front again (as it's shown: full screen or its window).
    pub fn raise(&self, ws: &str) {
        let Some(wm) = &self.wm else { return };
        for t in self.windows_of(ws) {
            wm.send(Cmd::Activate(Which(format!("#{}", t.id))));
        }
    }

    fn place_all(&self) {
        // A wadspace's windows: one per display, in a stable order (their ids),
        // so each display gets one of them.
        let mut by_ws: HashMap<String, Vec<u32>> = HashMap::new();
        for t in self.windows() {
            if let Some(ws) = t.wadspace().map(str::to_string) {
                by_ws.entry(ws).or_default().push(t.id);
            }
        }
        for ids in by_ws.values_mut() {
            ids.sort_unstable();
        }
        for t in self.windows() {
            let want = self.store.display(t.wadspace().unwrap_or_default());
            // Its place among its wadspace's windows (0: the first).
            let at = t
                .wadspace()
                .and_then(|ws| by_ws.get(ws))
                .and_then(|ids| ids.iter().position(|id| *id == t.id))
                .unwrap_or(0);
            let one = t.wadspace().map(|ws| by_ws.get(ws).map_or(1, Vec::len)).unwrap_or(1);
            self.place(&t, want, at, one);
        }
    }

    fn place(&self, t: &Top, want: Option<SessionDisplay>, at: usize, of: usize) {
        let (Some(wm), Some(ws)) = (&self.wm, t.wadspace()) else { return };
        let Some(want) = want else { return };
        let output = (want == SessionDisplay::Screen).then(|| {
            let outs = self.from_output(ws);
            outs[at % outs.len()].clone()
        });
        let told = (want, output.clone());
        if self.applied.borrow().get(&t.id) == Some(&told) {
            return;
        }
        let which = || Which(format!("#{}", t.id));
        match want {
            SessionDisplay::Screen => {
                wm.send(Cmd::Fullscreen(which(), output.clone()));
                wm.send(Cmd::Activate(which()));
            }
            SessionDisplay::Window => {
                wm.send(Cmd::Unfullscreen(which()));
                wm.send(Cmd::Activate(which()));
            }
            SessionDisplay::Background => wm.send(Cmd::Minimize(which())),
            // A stream's desktop isn't on this screen.
            SessionDisplay::Stream => {}
        }
        tracing::debug!("{ws}: its window #{} is {want:?} ({}/{of} on {output:?})", t.id, at + 1);
        self.applied.borrow_mut().insert(t.id, told);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outs(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_wadspaces_windows_go_one_per_display_its_own_first() {
        let displays = outs(&["eDP-1", "DP-1", "HDMI-1"]);
        // On the primary: the primary, then the others in the shell's order.
        assert_eq!(order_displays(&displays, "eDP-1"), outs(&["eDP-1", "DP-1", "HDMI-1"]));
        // On another (Open on 2): that one first, then the rest.
        assert_eq!(order_displays(&displays, "DP-1"), outs(&["DP-1", "eDP-1", "HDMI-1"]));
        // Its display isn't known to the shell (a connector that went): the rest, each once.
        assert_eq!(order_displays(&displays, "DP-9"), outs(&["DP-9", "eDP-1", "DP-1", "HDMI-1"]));
        // One display: every window on it.
        assert_eq!(order_displays(&outs(&["eDP-1"]), "eDP-1"), outs(&["eDP-1"]));
    }

    #[test]
    fn more_windows_than_displays_go_round_again() {
        let ordered = order_displays(&outs(&["eDP-1", "DP-1"]), "eDP-1");
        let at = |i: usize| ordered[i % ordered.len()].clone();
        assert_eq!((at(0).as_str(), at(1).as_str(), at(2).as_str()), ("eDP-1", "DP-1", "eDP-1"));
    }
}
