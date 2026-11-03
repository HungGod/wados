// The lucide icons the app draws (assets/lucide/*.svg), compiled in:
// LUCIDE: [(name, svg)].
use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/lucide");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("assets/lucide")
        .filter_map(|e| e.ok()?.file_name().into_string().ok()?.strip_suffix(".svg").map(String::from))
        .collect();
    names.sort();
    let mut out = String::from("pub static LUCIDE: &[(&str, &str)] = &[\n");
    for n in &names {
        out.push_str(&format!(
            "    ({n:?}, include_str!({:?})),\n",
            dir.join(format!("{n}.svg")).display().to_string()
        ));
    }
    out.push_str("];\n");
    std::fs::write(Path::new(&std::env::var("OUT_DIR").unwrap()).join("lucide.rs"), out).unwrap();
}
