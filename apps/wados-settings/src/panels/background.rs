//! Settings ▸ Background (the spike's `panels/BackgroundPanel.svelte`): being
//! built. Replace this file with the panel: `make` gives a
//! [`Panel`](crate::panel::Panel) (see `panels/accounts.rs`), and until
//! then Settings shows a placeholder here.

use std::rc::Rc;

use crate::panel::{Ctx, Panel, Placeholder};

pub fn make(_: &Ctx) -> Rc<dyn Panel> {
    Placeholder::make("background")
}
