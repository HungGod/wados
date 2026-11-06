//! Writes the labwc config Settings would (for probes): rcxml <dir> [rate delay layout].

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let dir = std::path::PathBuf::from(&a[0]);
    let mut k = wad_desk::settings::Keyboard::default();
    if a.len() >= 4 {
        k.repeat_rate = a[1].parse().unwrap();
        k.repeat_delay = a[2].parse().unwrap();
        k.layout = a[3].clone();
    }
    wad_desk::labwc::write_to(&dir, &wad_desk::settings::Mouse::default(), &k).unwrap();
}
