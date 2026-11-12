//! `wadd`: the WadSpaces machine daemon.
//!
//!   wadd serve            as a machine's system service (wadd.service)
//!   wadd serve --user     on a laptop, as you (rootless podman, your paths)
//!   wadd serve --sample   the tailnet, peers, GitHub and streams as sample data (to build the UI against)
//!   wadd version

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};
use wad_config::{Config, Profile};
use wadd::logbuf::{BufferLayer, LogBuffer};
use wadd::{Listen, Parts, Server, systemd};

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
        /// Answer the tailnet, peers, GitHub and streams with sample data (two
        /// peers, a running tailnet with an ask, a signed-in GitHub, a public
        /// stream live and a private one starting), to build the UI against.
        #[arg(long)]
        sample: bool,
    },
    /// Server mode's console on tty1 (wados-console.service).
    Console,
    /// A machine's public address, on the build computer, with the secrets.
    Tunnel {
        #[command(subcommand)]
        cmd: TunnelCmd,
    },
    /// Print the version.
    Version,
}

#[derive(Subcommand)]
enum TunnelCmd {
    /// A machine's public address made (or found again: the same id reuses
    /// its tunnel and record), and what its first boot puts in place written
    /// to `--out`: `cloudflared-token`, `wadd.toml` (`[streaming] host`) and
    /// `turn.toml` (when the keys have TURN's). host/build.sh installer
    /// gives WadSpaces Live its own this way. The address's name isn't the
    /// machine's id: every stick of one name shares it, and each stick's
    /// first boot gives itself an id of its own (host/usr/libexec/wados/provision).
    Provision {
        /// The machine's id (`<id>.wadspaces.com`).
        id: String,
        /// The secrets folder's cloudflare.toml.
        #[arg(long)]
        secrets: PathBuf,
        /// A folder (made if it isn't there) for the three files.
        #[arg(long)]
        out: PathBuf,
    },
    /// Retires a machine: its DNS record and tunnel go.
    Remove {
        /// The machine's id (`<id>.wadspaces.com`).
        id: String,
        /// The secrets folder's cloudflare.toml.
        #[arg(long)]
        secrets: PathBuf,
    },
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Version => {
            println!("wadd {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Cmd::Serve { user, config, socket, state_dir, sample } => serve(user, config, socket, state_dir, sample),
        Cmd::Console => console(),
        Cmd::Tunnel { cmd: TunnelCmd::Remove { id, secrets } } => tunnel_remove(&id, &secrets),
        Cmd::Tunnel { cmd: TunnelCmd::Provision { id, secrets, out } } => tunnel_provision(&id, &secrets, &out),
    }
}

fn console() -> ExitCode {
    let r = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
        .and_then(|rt| rt.block_on(wadd::console::run()));
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("wadd: {e}");
            ExitCode::FAILURE
        }
    }
}

fn tunnel_provision(id: &str, secrets: &std::path::Path, out: &std::path::Path) -> ExitCode {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let run = || -> Result<String, String> {
        if let Some(p) = wad_model::install_names::computer_name_problem(id) {
            return Err(format!("{id}: {p}"));
        }
        let text = std::fs::read_to_string(secrets).map_err(|e| format!("{}: {e}", secrets.display()))?;
        let keys = wad_cloudflare::Keys::parse(&text).map_err(|e| e.to_string())?;
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
        let cf = wad_cloudflare::Cloudflare::new(wad_cloudflare::API, "wadspaces.com", &keys);
        let a = rt.block_on(cf.provision(id)).map_err(|e| e.to_string())?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(out)
            .map_err(|e| format!("{}: {e}", out.display()))?;
        let write = |name: &str, text: &str, mode: u32| -> Result<(), String> {
            let path = out.join(name);
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(mode)
                .open(&path)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            std::io::Write::write_all(&mut f, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
        };
        write("cloudflared-token", &a.token, 0o600)?;
        write("wadd.toml", &format!("[streaming]\nhost = \"{}\"\n", a.host), 0o644)?;
        if let Some(turn) = keys.turn_file() {
            write("turn.toml", &turn, 0o600)?;
        }
        Ok(a.host)
    };
    match run() {
        Ok(host) => {
            println!("{host}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("wadd: {e}");
            ExitCode::FAILURE
        }
    }
}

fn tunnel_remove(id: &str, secrets: &std::path::Path) -> ExitCode {
    let keys = std::fs::read_to_string(secrets)
        .map_err(|e| format!("{}: {e}", secrets.display()))
        .and_then(|t| wad_cloudflare::Keys::parse(&t).map_err(|e| e.to_string()));
    let keys = match keys {
        Ok(k) => k,
        Err(e) => {
            eprintln!("wadd: {e}");
            return ExitCode::FAILURE;
        }
    };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build();
    let cf = wad_cloudflare::Cloudflare::new(wad_cloudflare::API, "wadspaces.com", &keys);
    match rt.map_err(|e| e.to_string()).and_then(|rt| rt.block_on(cf.remove(id)).map_err(|e| e.to_string())) {
        Ok(true) => {
            println!("{id}: its record and tunnel are gone");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("{id}: it has no tunnel (its record, if it had one, is gone)");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("wadd: {e}");
            ExitCode::FAILURE
        }
    }
}

fn serve(
    user: bool,
    config: Option<PathBuf>,
    socket: Option<PathBuf>,
    state_dir: Option<PathBuf>,
    sample: bool,
) -> ExitCode {
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
    let mut parts = Parts::real(&config, profile);
    if sample {
        tracing::warn!("the tailnet, peers, GitHub and streams are sample data");
        parts.sample = Some(Duration::from_millis(800));
    }
    let r = rt.block_on(async {
        let server = Server::with_parts(profile, config, listen, logs, parts)?;
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
