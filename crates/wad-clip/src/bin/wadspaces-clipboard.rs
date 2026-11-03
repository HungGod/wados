//! A wadspace's clipboard (labwc's) and the one it's drawn on (sway's, or
//! its stream's) as one. A copy on either side is copied on the other.
//!
//!   wadspaces-clipboard [--host <socket>]    (else $WADSPACES_HOST_WAYLAND)
//!       Inside a wadspace (labwc's autostart): labwc is $WAYLAND_DISPLAY.
//!   wadspaces-clipboard --here-fd <fd> --host-fd <fd> --name <name>
//!       On the machine, for a wadspace on the screen (wadd starts it, as a
//!       user of its own): two connections wadd made, to its labwc and to
//!       sway, which serves the wadspace itself no clipboard control.
//!
//! It ends when either side does (labwc, or the display going away); labwc's
//! autostart (or wadd) starts it again with the next labwc.

use std::process::ExitCode;
use std::sync::{Arc, mpsc};

use wad_clip::{Change, wl};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Side {
    Here,
    Machine,
}

/// One side: a socket to connect to, or a connection made for us.
enum End {
    Socket(std::path::PathBuf),
    Stream(std::os::unix::net::UnixStream),
}

/// Both sides and the name to copy as, from the arguments.
fn ends(args: &[String]) -> Option<(End, End, String)> {
    let fd = |v: &str| -> Option<End> {
        let n: i32 = v.parse().ok().filter(|n| *n > 2)?;
        // SAFETY: the fd was handed to this process for it to own (wadd's
        // spawn); nothing else here uses it.
        Some(End::Stream(unsafe { std::os::fd::FromRawFd::from_raw_fd(n) }))
    };
    match args {
        [h, here, m, host, n, name] if h == "--here-fd" && m == "--host-fd" && n == "--name" => {
            Some((fd(here)?, fd(host)?, name.clone()))
        }
        _ => {
            let host = match args {
                [flag, socket] if flag == "--host" => Some(socket.clone()),
                [] => std::env::var("WADSPACES_HOST_WAYLAND").ok().filter(|s| !s.is_empty()),
                _ => None,
            }?;
            let host = End::Socket(wl::socket_path(&host));
            let name = nix::unistd::gethostname().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
            Some((End::Socket(wl::env_socket()), host, format!("ws-{name}")))
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("wadspaces-clipboard {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let Some((here, host, name)) = ends(&args) else {
        eprintln!(
            "usage: wadspaces-clipboard [--host <wayland socket>]  (or WADSPACES_HOST_WAYLAND)\n       \
             wadspaces-clipboard --here-fd <fd> --host-fd <fd> --name <name>"
        );
        return ExitCode::from(2);
    };
    // Unique per run: a new bridge doesn't take an old one's copies for its own.
    let me = format!("{name}-{}", std::process::id());

    let (tx, rx) = mpsc::channel();
    let watch = |end: End, side: Side| {
        let tx = tx.clone();
        let on_change = move |c| {
            let _ = tx.send((side, c));
        };
        match end {
            End::Socket(socket) => wl::watch(&socket, &me, on_change),
            End::Stream(stream) => wl::watch_stream(stream, &me, on_change),
        }
        .map_err(|e| format!("{side:?}: {e}"))
    };
    let (here, machine) = match (watch(here, Side::Here), watch(host, Side::Machine)) {
        (Ok(h), Ok(m)) => (h, m),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("wadspaces-clipboard: not shared: {e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("wadspaces-clipboard: this wadspace's clipboard is the machine's");

    // The last copy carried either way: it comes back from the other side
    // (another wadspace's bridge, the HUD), and needn't go round again.
    let mut last = 0u64;
    // Where the last copy came from, if it was a password.
    let mut secret_from = None;
    let other = |side| match side {
        Side::Here => &machine,
        Side::Machine => &here,
    };
    for (side, change) in rx {
        match change {
            Change::Copied(c) => {
                let digest = c.digest();
                if digest == last {
                    continue;
                }
                last = digest;
                secret_from = c.secret().then_some(side);
                other(side).set(Arc::new(c));
            }
            // A password manager clearing its copy clears it everywhere.
            Change::Cleared if secret_from == Some(side) => {
                secret_from = None;
                last = 0;
                other(side).clear();
            }
            // Otherwise a copy outlives its app: the other side keeps ours.
            Change::Cleared => {}
            Change::Closed(why) => {
                eprintln!("wadspaces-clipboard: {side:?} ended: {why}");
                return ExitCode::SUCCESS;
            }
        }
    }
    ExitCode::SUCCESS
}
