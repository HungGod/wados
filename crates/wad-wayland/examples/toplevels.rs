//! Probe: every window the compositor has, as a privileged client (the
//! shell) sees it through wlr-foreign-toplevel: app id, title, states.
//!
//!     WAYLAND_DISPLAY=… cargo run -p wad-wayland --example toplevels

use std::collections::BTreeMap;

use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child, protocol::wl_registry};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

#[derive(Default, Debug)]
struct Top {
    app_id: String,
    title: String,
    states: Vec<u32>,
}

#[derive(Default)]
struct State {
    tops: BTreeMap<u32, Top>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        _: &mut Self,
        r: &wl_registry::WlRegistry,
        e: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = e
            && interface == "zwlr_foreign_toplevel_manager_v1"
        {
            r.bind::<ZwlrForeignToplevelManagerV1, _, _>(name, version.min(3), qh, ());
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(
        s: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        e: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = e {
            s.tops.insert(toplevel.id().protocol_id(), Top::default());
        }
    }
    event_created_child!(State, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(
        s: &mut Self,
        h: &ZwlrForeignToplevelHandleV1,
        e: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let t = s.tops.entry(h.id().protocol_id()).or_default();
        match e {
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => t.app_id = app_id,
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => t.title = title,
            zwlr_foreign_toplevel_handle_v1::Event::State { state } => {
                t.states = state.as_chunks::<4>().0.iter().map(|c| u32::from_ne_bytes(*c)).collect()
            }
            _ => {}
        }
    }
}

fn main() {
    let conn = Connection::connect_to_env().expect("WAYLAND_DISPLAY");
    let mut q = conn.new_event_queue();
    let qh = q.handle();
    conn.display().get_registry(&qh, ());
    let mut s = State::default();
    for _ in 0..3 {
        q.roundtrip(&mut s).expect("roundtrip");
    }
    for (id, t) in &s.tops {
        println!("#{id} app_id={:?} title={:?} states={:?}", t.app_id, t.title, t.states);
    }
    if s.tops.is_empty() {
        println!("(no windows)");
    }
}
