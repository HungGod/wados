//! A securely streamed wadspace's units (wad_gen::units::secure_units), as
//! wadd writes them, for tools/krun/prototype.sh: the spike's `retro`
//! wadspace (as `krunproto`, on a plain colour) on a machine's rootful
//! podman. Writes its units and its extra folder (`display`: `stream`);
//! prints nothing. `--root`: the extra folder goes under it (its path in the
//! units stays `--state`'s), for a machine the files are copied to.
//!
//!   cargo run -q -p wad-gen --example secure-units -- --units <dir> --state <dir>
//!       --guid <guid> --web-port <port> --auth <dir> --wallpaper <#rrggbb>
//!       [--image <wadspace image>] [--stream-image <image>] [--runtime krun]
//!       [--cpus 2] [--ram-mib 2048] [--root <dir>]

use std::collections::HashMap;
use std::fs;

use wad_gen::Mounts;
use wad_gen::extras;
use wad_gen::units::{self, Machine, Run, Sidecar, Transport, Vm};
use wad_model::{Library, Wallpaper};

fn main() {
    let mut opts: HashMap<String, String> = HashMap::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let Some(name) = a.strip_prefix("--") else { usage() };
        opts.insert(name.into(), args.next().unwrap_or_else(|| usage()));
    }
    let get = |k: &str| opts.get(k).cloned().unwrap_or_else(|| usage());
    let or = |k: &str, d: &str| opts.get(k).cloned().unwrap_or_else(|| d.to_string());
    let num = |k: &str, d: &str| or(k, d).parse::<u32>().unwrap_or_else(|_| panic!("--{k}: not a number"));
    let (units_dir, state_dir, guid, auth) = (get("units"), get("state"), get("guid"), get("auth"));
    let web_port = get("web-port").parse::<u16>().unwrap_or_else(|_| panic!("--web-port: not a port"));

    let lib: Library = serde_json::from_str(include_str!("../../wad-model/tests/data/populated.json"))
        .expect("populated.json is a library");
    let mut ws = lib.wadspace("retro").expect("the sample has retro").clone();
    ws.id = "krunproto".into();
    ws.name = "Secure Prototype".into();
    ws.look.wallpaper = Wallpaper::color(get("wallpaper"));
    let machine = Machine {
        signins_dir: format!("{state_dir}/keys"),
        agents_dir: "/run/wados/agents".into(),
        state_dir: state_dir.clone(),
        tz: "Etc/UTC".into(),
        userns_base: Some(units::USERNS_BASE),
        screens: 1,
        ..Machine::default()
    };
    let (image, stream_image, runtime) = (
        or("image", "localhost/wados-base:44"),
        or("stream-image", "localhost/wados-stream:44"),
        or("runtime", "krun"),
    );
    let sidecar = Sidecar {
        image: &stream_image,
        guid: &guid,
        web_port,
        transport: Transport::WebSockets,
        dir: &auth,
        origin: None,
        players: 0,
    };
    let vm = Vm { runtime: &runtime, cpus: num("cpus", "2"), ram_mib: num("ram-mib", "2048") };
    let mounts = Mounts::new();
    let run = Run { image: &image, mounts: &mounts, wallpaper: None };
    let all = units::secure_units(&ws, &run, &machine, &sidecar, &vm).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(&units_dir).expect("the units folder");
    for (name, text) in all {
        fs::write(format!("{units_dir}/{name}"), text).expect("a unit");
    }
    let extra = format!("{}{}", or("root", ""), extras::dir(&state_dir, &ws.id));
    fs::create_dir_all(&extra).expect("the extra folder");
    for (name, text) in extras::files(&ws, &lib.keys, &mounts) {
        if let Some(text) = text {
            fs::write(format!("{extra}/{name}"), text).expect("an extra file");
        }
    }
    // Streamed: its desktop draws on the stream's display (in the VM).
    fs::write(format!("{extra}/{}", units::DISPLAY_FILE), "stream\n").expect("the display file");
}

fn usage() -> ! {
    eprintln!(
        "usage: secure-units --units <dir> --state <dir> --guid <guid> --web-port <port> --auth <dir> \
         --wallpaper <#rrggbb> [--image <image>] [--stream-image <image>] [--runtime <rt>] [--cpus <n>] \
         [--ram-mib <n>] [--root <dir>]"
    );
    std::process::exit(2)
}
