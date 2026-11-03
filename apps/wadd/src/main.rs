//! `wadd`: the WadSpaces machine daemon.
//!
//!   wadd serve            as a machine's system service (wadd.service)
//!   wadd serve --user     on a laptop, as you (rootless podman, your paths)
//!   wadd version

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};
use wad_config::{Config, Profile};
use wadd::logbuf::{BufferLayer, LogBuffer};
use wadd::{Listen, Server, systemd};

#[derive(Parser)]
#[command(name = "wadd", about = "The WadSpaces machine daemon")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the daemon.
    Serve {
        /// As you, on a laptop: rootless podman, your paths, your socket.
        #[arg(long)]
        user: bool,
        /// Read this file after the defaults, instead of the usual ones.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Listen here (when systemd doesn't hand over a socket).
        #[arg(long)]
        socket: Option<PathBuf>,
        /// Keep state here (a trial's own).
        #[arg(long)]
        state_dir: Option<PathBuf>,
    },
    /// Print the version.
    Version,
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Version => {
            println!("wadd {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Cmd::Serve { user, config, socket, state_dir } => serve(user, config, socket, state_dir),
    }
}

fn serve(user: bool, config: Option<PathBuf>, socket: Option<PathBuf>, state_dir: Option<PathBuf>) -> ExitCode {
    let profile = if user { Profile::User } else { Profile::System };
    let files = config.map(|c| vec![c]).unwrap_or_else(|| Config::files(profile));
    let mut config = match Config::load(profile, &files) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("wadd: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(d) = state_dir {
        config.daemon.state_dir = d;
    }
    let logs = LogBuffer::new(config.log.buffer_lines);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log.level));
    tracing_subscriber::registry()
        .with(BufferLayer { buffer: logs.clone(), to_stderr: true }.with_filter(filter))
        .init();

    let listen = match (systemd::listener_from_systemd(), socket) {
        (Some(l), _) => Listen::Activated(l),
        (None, Some(p)) => Listen::Path(p),
        (None, None) => Listen::Path(config.daemon.socket.clone()),
    };
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("wadd: {e}");
            return ExitCode::FAILURE;
        }
    };
    let r = rt.block_on(async {
        let server = Server::new(profile, config, listen, logs)?;
        server.serve().await
    });
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}
