//! `wadd console`: server mode's text console on tty1 (the spike's
//! `shell/boot/ServerMode.svelte`), under `wados-console.service`. It says
//! what's streaming, and Ctrl+Alt+Delete opens its menu: Back to the
//! desktop, Restart, Power off, Stay in server mode. The kernel blanks the
//! screen after a minute, and a key brings it back. The words drift a little
//! each minute, so nothing burns in.

use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::path::Path;
use std::time::Duration;

use nix::sys::termios::{self, LocalFlags, SetArg};
use wad_model::StreamMode;
use wad_proto::StreamInfo;
use wad_wadd_client::Wadd;

use crate::server::RUN_DIR;

pub const MENU: [&str; 4] = ["Back to the desktop", "Restart", "Power off", "Stay in server mode"];

/// A key the menu knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Esc,
}

/// The keys in what was read (`ESC [ A`, or a lone ESC, …).
pub fn keys(bytes: &[u8]) -> Vec<Key> {
    let mut out = vec![];
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' | b'\n' => out.push(Key::Enter),
            0x1b if bytes.get(i + 1) == Some(&b'[') => {
                match bytes.get(i + 2) {
                    Some(b'A') => out.push(Key::Up),
                    Some(b'B') => out.push(Key::Down),
                    _ => {}
                }
                i += 2;
            }
            0x1b => out.push(Key::Esc),
            _ => {}
        }
        i += 1;
    }
    out
}

fn row(s: &StreamInfo, name: &str) -> String {
    let how = match s.mode {
        StreamMode::Private => "Private",
        StreamMode::Public => "Public",
        m if m.is_public() => "Public · secure",
        _ => "Private · secure",
    };
    format!("{name}  ·  {how}  ·  {} watching", s.viewers)
}

/// What the screen says, a line each.
pub fn screen(machine: &str, clock: &str, rows: &[String], menu: Option<usize>) -> Vec<String> {
    let mut out = vec![format!("{machine} · server mode · {clock}"), String::new()];
    match menu {
        Some(at) => {
            for (i, m) in MENU.iter().enumerate() {
                out.push(format!("{} {m}", if i == at { ">" } else { " " }));
            }
            out.push(String::new());
            out.push("↑ ↓ and Enter · Esc stays".into());
        }
        None => {
            out.push("This machine is streaming. Its screen is off.".into());
            out.push(String::new());
            if rows.is_empty() {
                out.push(
                    "Nothing is streaming right now. Start a stream from the phone remote, or come back to the desktop."
                        .into(),
                );
            }
            out.extend(rows.iter().cloned());
            out.push(String::new());
            out.push("Press Ctrl+Alt+Delete for the menu".into());
        }
    }
    out
}

fn draw(lines: &[String], minute: u64) {
    let (top, left) = (2 + minute % 5, 2 + minute % 9);
    let mut text = String::from("\x1b[2J\x1b[?25l");
    for (i, l) in lines.iter().enumerate() {
        text.push_str(&format!("\x1b[{};{}H{l}", top + i as u64, left));
    }
    let mut out = std::io::stdout();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

async fn act(w: &Wadd, at: usize) -> bool {
    match at {
        0 => match w.server_set(false).await {
            Ok(_) => true,
            Err(e) => {
                tracing::error!("back to the desktop: {}", e.message);
                false
            }
        },
        1 | 2 => {
            let verb = if at == 1 { "reboot" } else { "poweroff" };
            let _ = tokio::process::Command::new("systemctl").arg(verb).status().await;
            true
        }
        _ => false,
    }
}

pub async fn run() -> Result<(), String> {
    let stdin = std::io::stdin();
    let saved = termios::tcgetattr(stdin.as_fd()).map_err(|e| format!("tty1: {e}"))?;
    let mut raw = saved.clone();
    raw.local_flags.remove(LocalFlags::ICANON | LocalFlags::ECHO | LocalFlags::ISIG);
    termios::tcsetattr(stdin.as_fd(), SetArg::TCSANOW, &raw).map_err(|e| format!("tty1: {e}"))?;
    // The kernel's console blanks after a minute with no key.
    print!("\x1b[9;1]");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 32];
        let mut input = std::io::stdin();
        while let Ok(n) = input.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let w = Wadd::here();
    let poke = Path::new(RUN_DIR).join("menu");
    let mut menu: Option<usize> = None;
    let mut rows = vec![];
    let mut machine = String::new();
    let mut ticks = 0u64;
    let mut shown = vec![];
    loop {
        if ticks.is_multiple_of(10) {
            if let Ok(m) = w.get::<wad_proto::MachineInfo>("/v1/machine").await {
                machine = m.name;
            }
            if let (Ok(list), Ok(lib)) = (w.streams().await, w.get::<wad_model::edit::Library>("/v1/library").await) {
                let name = |id: &str| {
                    lib.wadspaces.iter().find(|x| x.id == id).map_or_else(|| id.to_string(), |x| x.name.clone())
                };
                rows = list.streams.iter().filter(|s| s.peer.is_none()).map(|s| row(s, &name(&s.id))).collect();
            }
        }
        if poke.exists() {
            let _ = std::fs::remove_file(&poke);
            menu = Some(0);
        }
        let now = chrono::Local::now();
        let minute = now.timestamp().div_euclid(60).unsigned_abs();
        let lines = screen(&machine, &now.format("%H:%M").to_string(), &rows, menu);
        if lines != shown {
            draw(&lines, minute);
            shown = lines;
        }
        let got = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
        ticks += 1;
        let Ok(Some(bytes)) = got else { continue };
        for k in keys(&bytes) {
            let Some(at) = menu else { continue };
            match k {
                Key::Up => menu = Some((at + MENU.len() - 1) % MENU.len()),
                Key::Down => menu = Some((at + 1) % MENU.len()),
                Key::Esc => menu = None,
                Key::Enter => {
                    menu = None;
                    if act(&w, at).await {
                        print!("\x1b[9;0]\x1b[?25h\x1b[2J");
                        let _ = termios::tcsetattr(std::io::stdin().as_fd(), SetArg::TCSANOW, &saved);
                        return Ok(());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menus_keys_are_read_from_what_a_tty_sends() {
        assert_eq!(keys(b"\x1b[A\x1b[B\r"), [Key::Up, Key::Down, Key::Enter]);
        assert_eq!(keys(b"\x1b"), [Key::Esc]);
        assert_eq!(keys(b"x"), []);
    }

    #[test]
    fn it_says_whats_streaming_or_that_nothing_is() {
        let s = screen("Studio", "10:30", &[], None);
        assert_eq!(s[0], "Studio · server mode · 10:30");
        assert!(s.iter().any(|l| l.starts_with("Nothing is streaming right now")));
        assert!(s.iter().any(|l| l == "Press Ctrl+Alt+Delete for the menu"));
        let s = screen("Studio", "10:30", &["Krita  ·  Public  ·  2 watching".into()], None);
        assert!(!s.iter().any(|l| l.starts_with("Nothing")));
    }

    #[test]
    fn the_menu_marks_where_it_is() {
        let s = screen("Studio", "10:30", &[], Some(1));
        assert!(s.contains(&"> Restart".to_string()) && s.contains(&"  Power off".to_string()));
        assert!(s.iter().any(|l| l == "↑ ↓ and Enter · Esc stays"));
    }
}
