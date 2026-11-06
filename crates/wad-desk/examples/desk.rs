//! wad-desk by hand, for the VM's checks (tools/vm/check.sh's session steps):
//!
//!     desk sound                 the outputs and inputs, as Settings ▸ Sound reads them
//!     desk sound-test            plays Settings' speaker test (pw-play), to its end
//!     desk level SECONDS         the input level, as Sound's meter reads it
//!     desk check PASSWORD        does it open this account (PAM, wados-lock)
//!     desk passwd CURRENT NEW    changes it, as Lock Screen & Privacy does

use wad_desk::{pam, sound};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let login = std::env::var("USER").unwrap_or_default();
    match a.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["sound"] => match sound::status() {
            Ok(s) => {
                for d in s.outputs.iter().chain(&s.inputs) {
                    println!(
                        "{}{} {} {:.2}{}",
                        if d.default { "*" } else { " " },
                        d.id,
                        d.name,
                        d.volume,
                        if d.muted { " muted" } else { "" }
                    );
                }
                println!("outputs={} inputs={}", s.outputs.len(), s.inputs.len());
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        ["sound-test"] => {
            let f = std::env::temp_dir().join("wados-sound-test.wav");
            let mut c = sound::play_test(&f).unwrap_or_else(|e| panic!("{e}"));
            let ok = c.wait().map(|s| s.success()).unwrap_or(false);
            println!("played={ok}");
        }
        ["level", secs] => {
            let (tx, rx) = std::sync::mpsc::channel();
            let _l = sound::listen(move |v| {
                let _ = tx.send(v);
            })
            .unwrap_or_else(|e| panic!("{e}"));
            let end = std::time::Instant::now() + std::time::Duration::from_secs(secs.parse().unwrap());
            let (mut n, mut top) = (0, 0.0f64);
            while std::time::Instant::now() < end {
                if let Ok(v) = rx.recv_timeout(std::time::Duration::from_millis(200)) {
                    n += 1;
                    top = top.max(v);
                }
            }
            println!("readings={n} loudest={top:.2}");
        }
        ["check", pw] => match pam::check("wados-lock", &login, pw) {
            Ok(()) => println!("ok"),
            Err(e) => println!("no: {e}"),
        },
        ["passwd", current, new] => match pam::change("wados-lock", &login, current, new) {
            Ok(()) => println!("changed"),
            Err(e) => println!("refused: {e:?}"),
        },
        _ => {
            eprintln!("desk sound | sound-test | level SECONDS | check PASSWORD | passwd CURRENT NEW");
            std::process::exit(2);
        }
    }
}
