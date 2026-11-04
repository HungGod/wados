//! wadbrowser: WadOS's web browser.
//!
//! Forked from Cactus (github.com/yohannes-mesay/cactus-browser, commit
//! 8ae5a66, MPL-2.0).
//!
//!     wadbrowser [--private] [--restore | --no-restore] [-v] [URL...]

use anyhow::Result;
use wadbrowser_core::LaunchArgs;

fn main() -> Result<()> {
    let args = LaunchArgs::parse(std::env::args());
    let filter = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()))
        // Colours only on a terminal (the journal stores escapes as binary).
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();

    wadbrowser_ui::run(args)
}
