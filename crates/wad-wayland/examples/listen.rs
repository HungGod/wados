//! Probe: serve one sandboxed client on a socket of its own, until Ctrl+C.
//!
//!     cargo run -p wad-wayland --example listen -- <compositor socket> <new socket> <app id>
//!
//! Then `WAYLAND_DISPLAY=<new socket> wayland-info` lists what a wadspace sees.

use std::path::PathBuf;

use wad_wayland::security_context::{Labels, Sandbox, listen};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [compositor, path, app_id] = args.as_slice() else {
        eprintln!("usage: listen <compositor socket> <new socket> <app id>");
        std::process::exit(2);
    };
    let instance = format!("{app_id}-1");
    let sandbox = Sandbox { engine: "io.wadspaces", app_id, instance_id: &instance };
    let path = PathBuf::from(path);
    let _held = match listen(&PathBuf::from(compositor), &path, &sandbox, None, &Labels::default()) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    println!("serving {} to io.wadspaces/{app_id}", path.display());
    loop {
        std::thread::park();
    }
}
