//! Probe: a mouse that isn't there (wlr-virtual-pointer), for tests that drag.
//!
//!     WAYLAND_DISPLAY=… pointer <width> <height> <step>…
//!
//! `width`×`height` is the output layout's size in logical pixels; each step
//! is one of `move X Y`, `down`, `up`, `wait MS`, `drag X1 Y1 X2 Y2` (press,
//! move there in small steps, release), `click X Y`, `rclick X Y` (the right
//! button: menus), `dblclick X Y`, `side back|forward` (the mouse's side
//! buttons), or `scroll DY` (wheel notches: + down).

use std::thread::sleep;
use std::time::{Duration, Instant};

use wayland_client::{Connection, Dispatch, QueueHandle, protocol::wl_registry};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1, zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_SIDE: u32 = 0x113;
const BTN_EXTRA: u32 = 0x114;

#[derive(Default)]
struct State {
    manager: Option<ZwlrVirtualPointerManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        s: &mut Self,
        r: &wl_registry::WlRegistry,
        e: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, .. } = e
            && interface == "zwlr_virtual_pointer_manager_v1"
        {
            s.manager = Some(r.bind(name, 1, qh, ()));
        }
    }
}
wayland_client::delegate_noop!(State: ignore ZwlrVirtualPointerManagerV1);
wayland_client::delegate_noop!(State: ignore ZwlrVirtualPointerV1);

struct Mouse<'a> {
    p: ZwlrVirtualPointerV1,
    conn: &'a Connection,
    w: u32,
    h: u32,
    start: Instant,
    at: (f64, f64),
}

impl Mouse<'_> {
    fn t(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }
    fn to(&mut self, x: f64, y: f64) {
        self.p.motion_absolute(self.t(), x as u32, y as u32, self.w, self.h);
        self.p.frame();
        self.at = (x, y);
        let _ = self.conn.flush();
    }
    fn button(&self, down: bool) {
        self.press(BTN_LEFT, down);
    }
    fn press(&self, button: u32, down: bool) {
        use wayland_client::protocol::wl_pointer::ButtonState;
        self.p.button(self.t(), button, if down { ButtonState::Pressed } else { ButtonState::Released });
        self.p.frame();
        let _ = self.conn.flush();
    }
    fn click(&self, button: u32) {
        self.press(button, true);
        sleep(Duration::from_millis(40));
        self.press(button, false);
    }
    fn scroll(&self, notches: f64) {
        use wayland_client::protocol::wl_pointer::{Axis, AxisSource};
        self.p.axis_source(AxisSource::Wheel);
        self.p.axis(self.t(), Axis::VerticalScroll, notches * 15.0);
        self.p.axis_discrete(self.t(), Axis::VerticalScroll, notches * 15.0, notches as i32);
        self.p.frame();
        let _ = self.conn.flush();
    }
    fn glide(&mut self, x: f64, y: f64) {
        let (x0, y0) = self.at;
        for i in 1..=30 {
            let k = i as f64 / 30.0;
            self.to(x0 + (x - x0) * k, y0 + (y - y0) * k);
            sleep(Duration::from_millis(16));
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let num =
        |i: usize| -> f64 { args.get(i).and_then(|a| a.parse().ok()).unwrap_or_else(|| panic!("a number at {i}")) };
    let (w, h) = (num(0) as u32, num(1) as u32);
    let conn = Connection::connect_to_env().expect("WAYLAND_DISPLAY");
    let mut q = conn.new_event_queue();
    let qh = q.handle();
    conn.display().get_registry(&qh, ());
    let mut st = State::default();
    q.roundtrip(&mut st).expect("roundtrip");
    let manager = st.manager.take().expect("the compositor offers no zwlr_virtual_pointer_manager_v1");
    let p = manager.create_virtual_pointer(None, &qh, ());
    let mut m = Mouse { p, conn: &conn, w, h, start: Instant::now(), at: (0.0, 0.0) };
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "move" => {
                m.to(num(i + 1), num(i + 2));
                i += 3;
            }
            "down" => {
                m.button(true);
                i += 1;
            }
            "up" => {
                m.button(false);
                i += 1;
            }
            "wait" => {
                sleep(Duration::from_millis(num(i + 1) as u64));
                i += 2;
            }
            "side" => {
                m.click(if args.get(i + 1).map(String::as_str) == Some("back") { BTN_SIDE } else { BTN_EXTRA });
                i += 2;
            }
            "drag" => {
                m.to(num(i + 1), num(i + 2));
                sleep(Duration::from_millis(100));
                m.button(true);
                sleep(Duration::from_millis(100));
                m.glide(num(i + 3), num(i + 4));
                sleep(Duration::from_millis(150));
                m.button(false);
                i += 5;
            }
            "click" | "rclick" | "dblclick" => {
                m.to(num(i + 1), num(i + 2));
                sleep(Duration::from_millis(80));
                let b = if args[i] == "rclick" { BTN_RIGHT } else { BTN_LEFT };
                m.click(b);
                if args[i] == "dblclick" {
                    sleep(Duration::from_millis(80));
                    m.click(b);
                }
                i += 3;
            }
            "scroll" => {
                m.scroll(num(i + 1));
                i += 2;
            }
            other => panic!("unknown step {other}"),
        }
        q.roundtrip(&mut st).expect("roundtrip");
    }
    sleep(Duration::from_millis(100));
}
