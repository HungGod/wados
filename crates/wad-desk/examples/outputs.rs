//! Output management by hand (for probes against a headless labwc):
//!
//!     outputs list
//!     outputs move <connector> <x> <y> [scale] [WxH]
//!     outputs off <connector> | on <connector>
//!     outputs night <kelvin> <seconds>

use std::sync::mpsc;
use std::time::Duration;

use wad_desk::outputs::{self, Said};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (tx, rx) = mpsc::channel();
    let o = outputs::start(move |s| {
        let _ = tx.send(s);
    })
    .unwrap_or_else(|e| panic!("{e}"));
    let heads = loop {
        if let Said::Heads(h) = rx.recv_timeout(Duration::from_secs(5)).expect("heads") {
            break h;
        }
    };
    let mut confs: Vec<_> = heads.iter().map(|h| h.conf()).collect();
    let find =
        |c: &mut Vec<wad_desk::layout::HeadConf>, n: &str| c.iter().position(|x| x.name == n).expect("no such display");
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["list"] | [] => {
            for h in &heads {
                println!(
                    "{} key={} label={:?} enabled={} mode={:?} at={},{} scale={} logical={:?} modes={}",
                    h.name,
                    h.key(),
                    h.label(),
                    h.enabled,
                    h.mode.map(|m| (m.width, m.height, m.refresh)),
                    h.x,
                    h.y,
                    h.scale,
                    h.logical(),
                    h.modes.len()
                );
            }
            return;
        }
        ["move", n, x, y, rest @ ..] => {
            let i = find(&mut confs, n);
            confs[i].x = x.parse().unwrap();
            confs[i].y = y.parse().unwrap();
            if let Some(s) = rest.first() {
                confs[i].scale = s.parse().unwrap();
            }
            if let Some(sz) = rest.get(1) {
                let (w, h) = sz.split_once('x').unwrap();
                confs[i].mode = Some(wad_desk::layout::Mode {
                    width: w.parse().unwrap(),
                    height: h.parse().unwrap(),
                    refresh: 0,
                    preferred: false,
                });
            }
        }
        ["off", n] => {
            let i = find(&mut confs, n);
            confs[i].enabled = false;
        }
        ["on", n, rest @ ..] => {
            let i = find(&mut confs, n);
            confs[i].enabled = true;
            // A headless output has no modes of its own once it's off.
            let (w, h) = rest.first().and_then(|s| s.split_once('x')).unwrap_or(("1280", "720"));
            if confs[i].mode.is_none() {
                confs[i].mode = Some(wad_desk::layout::Mode {
                    width: w.parse().unwrap(),
                    height: h.parse().unwrap(),
                    refresh: 0,
                    preferred: false,
                });
            }
        }
        ["night", k, secs] => {
            o.night(Some(k.parse().unwrap()));
            let end = std::time::Instant::now() + Duration::from_secs(secs.parse().unwrap());
            while std::time::Instant::now() < end {
                if let Ok(s) = rx.recv_timeout(Duration::from_millis(200)) {
                    println!("{s:?}");
                }
            }
            return;
        }
        other => panic!("what's {other:?}?"),
    }
    let id = o.apply(confs, false);
    loop {
        match rx.recv_timeout(Duration::from_secs(5)).expect("an answer") {
            Said::Applied(i, r) if i == id => {
                println!("{r:?}");
                std::process::exit(if r.is_ok() { 0 } else { 1 });
            }
            _ => {}
        }
    }
}
