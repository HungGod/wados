//! wadsctl: tells the shell what to do, one line over its socket
//! (`$XDG_RUNTIME_DIR/wados/shell.sock`), and prints its answer.
//!
//! labwc's keybinds run it (`<action name="Execute" command="wadsctl switcher next"/>`),
//! and so do tests:
//!
//!     wadsctl page desktop
//!     wadsctl switcher next
//!     wadsctl ping
//!
//! Exits 0 when the shell says `ok …`, 1 when it says anything else, 2 when
//! there's no shell to ask.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

fn socket() -> PathBuf {
    if let Some(p) = std::env::var_os("WADOS_SHELL_SOCKET") {
        return PathBuf::from(p);
    }
    let rt = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    rt.join("wados/shell.sock")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!("usage: wadsctl <command> [args…]   (wadsctl help: what the shell takes)");
        std::process::exit(if args.is_empty() { 2 } else { 0 });
    }
    let line = args.join(" ");
    if line.contains('\n') {
        eprintln!("wadsctl: one line only");
        std::process::exit(2);
    }
    let path = socket();
    let mut stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("wadsctl: {}: {e}", path.display());
            std::process::exit(2);
        }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    if let Err(e) = writeln!(stream, "{line}") {
        eprintln!("wadsctl: {e}");
        std::process::exit(2);
    }
    let mut reply = String::new();
    match BufReader::new(stream).read_line(&mut reply) {
        Ok(_) => {
            let reply = reply.trim_end();
            println!("{reply}");
            std::process::exit(if reply == "ok" || reply.starts_with("ok ") { 0 } else { 1 });
        }
        Err(e) => {
            eprintln!("wadsctl: no answer: {e}");
            std::process::exit(2);
        }
    }
}
