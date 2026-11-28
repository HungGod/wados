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

/// How WebKit draws, set before GTK or any thread (`--gpu on|off`, or
/// `WADBROWSER_GPU`; else by the render node). WebKit always composites
/// with OpenGL: a page that needs compositing (YouTube's watch pages start a
/// view transition, which turns it on) has its process aborted by WebKitGTK
/// 2.54 when it can't. So it's never turned off; what changes is the GL:
///
/// - no usable render node (a wadspace without /dev/dri), or `off`: Mesa's
///   software GL (llvmpipe), rather than WebKit without compositing;
/// - nouveau's render node: Mesa's nouveau GL driver, not Zink on NVK, which
///   Mesa 26.2 picks for Turing and later and which crashed WebKit's page
///   process (a recursion in `zink_render_attachment_shadow`). Older cards
///   already have nouveau's own.
///
/// Anything already in the environment wins.
fn gpu(wanted: Option<&str>) {
    let wanted = wanted.map(str::to_owned).or_else(|| std::env::var("WADBROWSER_GPU").ok());
    let node = render_node();
    for (key, value) in gl_env(wanted.as_deref(), node.as_deref()) {
        if std::env::var_os(key).is_none() {
            tracing::info!("GPU: {key}={value} (render node: {})", node.as_deref().unwrap_or("none"));
            // SAFETY: first thing in main, before any other thread exists.
            unsafe { std::env::set_var(key, value) };
        }
    }
}

/// The environment for `wanted` (`on`, `off`, else by the node) on a render
/// node whose kernel driver is `driver` (None: no usable node).
fn gl_env(wanted: Option<&str>, driver: Option<&str>) -> Vec<(&'static str, &'static str)> {
    let soft = match wanted {
        Some("on") => false,
        Some("off") => true,
        _ => driver.is_none(),
    };
    if soft {
        vec![("LIBGL_ALWAYS_SOFTWARE", "1")]
    } else if driver == Some("nouveau") {
        vec![("NOUVEAU_USE_ZINK", "0")]
    } else {
        vec![]
    }
}

/// The kernel driver of the first render node that opens (`nouveau`,
/// `amdgpu`, `i915`…; "" if it can't be told), else None.
fn render_node() -> Option<String> {
    let dir = std::fs::read_dir("/dev/dri").ok()?;
    let mut nodes: Vec<_> = dir
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("renderD"))
        .collect();
    nodes.sort();
    nodes.into_iter().find_map(|n| {
        std::fs::OpenOptions::new().read(true).write(true).open(format!("/dev/dri/{n}")).ok()?;
        let driver = std::fs::read_link(format!("/sys/class/drm/{n}/device/driver"))
            .ok()
            .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
            .unwrap_or_default();
        Some(driver)
    })
}

#[cfg(test)]
mod tests {
    use super::gl_env;

    #[test]
    fn webkit_always_has_a_gl() {
        // No GPU, or turned off: software GL, never WebKit without compositing.
        assert_eq!(gl_env(None, None), [("LIBGL_ALWAYS_SOFTWARE", "1")]);
        assert_eq!(gl_env(Some("off"), Some("amdgpu")), [("LIBGL_ALWAYS_SOFTWARE", "1")]);
        // nouveau: its own GL driver, not Zink.
        assert_eq!(gl_env(None, Some("nouveau")), [("NOUVEAU_USE_ZINK", "0")]);
        assert_eq!(gl_env(Some("on"), Some("nouveau")), [("NOUVEAU_USE_ZINK", "0")]);
        // Others as Mesa has them.
        assert!(gl_env(None, Some("amdgpu")).is_empty());
        assert!(gl_env(Some("on"), None).is_empty());
    }
}
