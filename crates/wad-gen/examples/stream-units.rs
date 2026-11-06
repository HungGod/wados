//! A streamed wadspace's units, as wadd writes them, for
//! images/stream/try-stream.sh: the spike's `retro` wadspace (as `trystream`,
//! on a plain colour) on this machine with rootless podman. Writes its units
//! and its extra folder (`display`: `stream` when it's streamed from the
//! start); prints nothing.
//!
//!   cargo run -q -p wad-gen --example stream-units -- --units <dir> --state <dir>
//!       --guid <guid> --web-port <port> --auth <dir> --wallpaper <#rrggbb>
//!       [--mux-port <port>]       a public stream (WebRTC); else private
//!       [--screen-dir <dir>]      the machine's screen folders (else none)
//!       [--moving]                moving_units: both displays, the sidecar unbound
//!       [--no-stream]             (with --moving) not streamed: no sidecar

use std::collections::HashMap;
use std::fs;

use wad_gen::Mounts;
use wad_gen::extras;
use wad_gen::units::{self, Machine, Run, Sidecar, Transport};
use wad_model::{Library, Wallpaper};

fn main() {
    let mut opts: HashMap<String, String> = HashMap::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let Some(name) = a.strip_prefix("--") else { usage() };
        let value =
            if matches!(name, "moving" | "no-stream") { String::new() } else { args.next().unwrap_or_else(|| usage()) };
        opts.insert(name.into(), value);
    }
    let get = |k: &str| opts.get(k).cloned().unwrap_or_else(|| usage());
    let port = |p: &str| p.parse::<u16>().unwrap_or_else(|_| panic!("not a port: {p}"));
    let (units_dir, state_dir, guid, auth) = (get("units"), get("state"), get("guid"), get("auth"));

    let lib: Library = serde_json::from_str(include_str!("../../wad-model/tests/data/populated.json"))
        .expect("populated.json is a library");
    let mut ws = lib.wadspace("retro").expect("the sample has retro").clone();
    ws.id = "trystream".into();
    ws.name = "Try Stream".into();
    ws.look.wallpaper = Wallpaper::color(get("wallpaper"));
    let machine = Machine {
        signins_dir: format!("{state_dir}/keys"),
        state_dir: state_dir.clone(),
        tz: "Etc/UTC".into(),
        rootless_uid: Some(1000),
        devices: if fs::metadata("/dev/dri").is_ok() { vec!["/dev/dri".into()] } else { vec![] },
        screen_dir: opts.get("screen-dir").cloned(),
        screens: 1,
        ..Machine::default()
    };
    let transport = match opts.get("mux-port") {
        Some(mux) => Transport::WebRtc { mux_port: port(mux) },
        None => Transport::WebSockets,
    };
    let sidecar = Sidecar {
        image: "localhost/wados-stream:44",
        guid: &guid,
        web_port: port(&get("web-port")),
        transport,
        dir: &auth,
        origin: None,
        players: 0,
    };
    let stream = (!opts.contains_key("no-stream")).then_some(&sidecar);
    let mounts = Mounts::new();
    let run = Run { image: "localhost/wados-base:44", mounts: &mounts, wallpaper: None };
    let all = if opts.contains_key("moving") {
        units::moving_units(&ws, &run, &machine, stream)
    } else {
        units::units(&ws, &run, &machine, stream)
    };
    fs::create_dir_all(&units_dir).expect("the units folder");
    for (name, text) in all {
        fs::write(format!("{units_dir}/{name}"), text).expect("a unit");
    }
    let extra = extras::dir(&state_dir, &ws.id);
    fs::create_dir_all(&extra).expect("the extra folder");
    for (name, text) in extras::files(&ws, &lib.keys, &mounts) {
        if let Some(text) = text {
            fs::write(format!("{extra}/{name}"), text).expect("an extra file");
        }
    }
    // Streamed from the start: its desktop starts on the stream. (Moving, the
    // try script says which, as wadd does before each move.)
    if !opts.contains_key("moving") && stream.is_some() {
        fs::write(format!("{extra}/{}", units::DISPLAY_FILE), "stream\n").expect("the display file");
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: stream-units --units <dir> --state <dir> --guid <guid> --web-port <port> --auth <dir> \
         --wallpaper <#rrggbb> [--mux-port <port>] [--screen-dir <dir>] [--moving] [--no-stream]"
    );
    std::process::exit(2)
}
