//! wadbrowser: WadOS's web browser.
//!
//! Forked from Cactus (github.com/yohannes-mesay/cactus-browser, commit
//! 8ae5a66, MPL-2.0).
//!
//! The command line is `wadbrowser_core::cli`'s (`wadbrowser --help`). One
//! browser runs per user (or per `--instance`): a later launch hands its
//! arguments to it over D-Bus and exits, so a link opens in a moment.

use anyhow::Result;
use wadbrowser_core::{APP_VERSION, CACTUS_COMMIT, cli, help_text};

fn main() -> Result<()> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| "/".into());
    let req = match cli::parse(std::env::args().skip(1), &cwd) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("wadbrowser: {e}\n{}", cli::USAGE);
            std::process::exit(2);
        }
    };
    if req.help {
        print!("{}", help_text());
        return Ok(());
    }
    if req.version {
        println!("wadbrowser {APP_VERSION} (Cactus {CACTUS_COMMIT})");
        return Ok(());
    }

    let filter = if req.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()))
        // Colours only on a terminal (the journal stores escapes as binary).
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();
    for flag in &req.ignored {
        tracing::debug!(flag, "ignored");
    }
    gpu(req.gpu.as_deref());

    let status = wadbrowser_ui::run(&req)?;
    std::process::exit(status);
}

/// v1's `gpu.rs`: `--gpu off` (or `WADBROWSER_GPU=off`), or no usable
/// render node (a wadspace without /dev/dri), keeps WebKit off its DMA-BUF
/// renderer, which draws nothing without a GPU. Before GTK or any thread.
fn gpu(wanted: Option<&str>) {
    let wanted = wanted.map(str::to_owned).or_else(|| std::env::var("WADBROWSER_GPU").ok());
    let off = match wanted.as_deref() {
        Some("on") => false,
        Some("off") => true,
        _ => !render_node(),
    };
    if off && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // SAFETY: first thing in main, before any other thread exists.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }
}

fn render_node() -> bool {
    let Ok(dir) = std::fs::read_dir("/dev/dri") else { return false };
    dir.flatten().any(|e| {
        e.file_name().to_string_lossy().starts_with("renderD")
            && std::fs::OpenOptions::new().read(true).write(true).open(e.path()).is_ok()
    })
}
