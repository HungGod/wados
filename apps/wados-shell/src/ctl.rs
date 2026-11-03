//! The shell's socket (`$XDG_RUNTIME_DIR/wados/shell.sock`): one line in, one
//! line out. labwc's keybinds reach the shell this way (through `wadsctl`),
//! and so do tests. A thread accepts; each request goes to the main loop,
//! which answers it.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::mpsc;

/// One line from a client, and where its answer goes.
pub struct Request {
    pub line: String,
    pub reply: mpsc::Sender<String>,
}

pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("WADOS_SHELL_SOCKET") {
        return PathBuf::from(p);
    }
    let rt = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    rt.join("wados/shell.sock")
}

/// Listens (replacing a socket left by an earlier shell) and sends every
/// request to `tx`.
pub fn serve(tx: async_channel::Sender<Request>) -> std::io::Result<PathBuf> {
    let path = path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::thread::Builder::new().name("ctl".into()).spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut line = String::new();
                let Ok(reader) = stream.try_clone() else { return };
                if BufReader::new(reader).read_line(&mut line).is_err() {
                    return;
                }
                let (reply, answer) = mpsc::channel();
                if tx.send_blocking(Request { line: line.trim().to_string(), reply }).is_err() {
                    let _ = writeln!(stream, "error the shell is stopping");
                    return;
                }
                let text = answer.recv().unwrap_or_else(|_| "error no answer".into());
                let _ = writeln!(stream, "{text}");
            });
        }
    })?;
    Ok(path)
}
