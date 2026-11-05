//! wadd's configuration, layered:
//!
//! - **system** (a WadSpaces machine): `/usr/lib/wados/wadd.toml` from the
//!   image, then `/etc/wados/wadd.toml` (the installer's provision writes the
//!   machine's id and public host there; an admin may add more). wadd never
//!   writes either: what it changes at run time is state, under `state_dir`.
//! - **user** (`wadd serve --user`, a laptop): the same vendor file if there
//!   is one, then `~/.config/wados/wadd.toml`; rootless podman, paths under
//!   your home and runtime dir.
//!
//! Later files override earlier ones key by key (tables merge). Unknown keys
//! are ignored, so an older wadd reads a newer file.
//!
//! No secret is ever in here: the TURN key is `streaming.turn`'s file, read
//! when a public stream starts, and the tunnel's token is cloudflared's.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const VENDOR: &str = "/usr/lib/wados/wadd.toml";
pub const ADMIN: &str = "/etc/wados/wadd.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// A WadSpaces machine: root, rootful podman, system paths.
    System,
    /// A laptop or desktop: your user, rootless podman, your paths.
    User,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub machine: Machine,
    pub daemon: Daemon,
    pub log: Log,
    pub display: Display,
    pub streaming: Streaming,
    pub tailnet: Tailnet,
    pub packages: Packages,
    pub github: Github,
}

/// GitHub: wadd's sign-in (the OAuth device flow) and the calls it makes
/// with the machine's token (the podman secret `github_token`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Github {
    /// The OAuth app: its client ID and the first sign-in's scopes.
    pub app: PathBuf,
    /// GitHub's sign-in (the device flow's endpoints).
    pub web: String,
    /// GitHub's REST API.
    pub api: String,
}

impl Default for Github {
    fn default() -> Self {
        Self {
            app: "/usr/lib/wados/github.toml".into(),
            web: "https://github.com".into(),
            api: "https://api.github.com".into(),
        }
    }
}

/// The machine.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Machine {
    /// What it's called (Settings ▸ About changes it).
    pub name: String,
    /// Its stable id, made at install (DNS-safe, unique among the owner's
    /// machines): its public host is `<id>.wadspaces.com`, and renaming the
    /// machine doesn't change it. Empty on a laptop: the hostname stands in.
    pub id: String,
}

/// Packages: an app's, a tool's or an extension's files, built once by CI
/// (wad_gen) and stacked onto the base. Where CI's index of them is, and
/// whether this machine downloads them (else it builds them itself).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Packages {
    /// Download what CI published (ghcr); false: only what's here, or built here.
    pub pull: bool,
    /// CI's index of what it published.
    pub index_url: String,
    /// How long a fetched index is used before it's fetched again.
    pub index_max_age_min: u64,
}

impl Default for Packages {
    fn default() -> Self {
        Self {
            pull: true,
            index_url: "https://github.com/HungGod/wados/releases/download/packages/index.json".into(),
            index_max_age_min: 60,
        }
    }
}

/// Streaming a wadspace: it draws on its Selkies sidecar, and wadd's proxy
/// serves the sidecar. Privately on the tailnet (Tailscale serve), or
/// publicly at `https://<host>/ws-<guid>/` through the machine's Cloudflare
/// tunnel, with WebRTC's media over Cloudflare's STUN and TURN.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Streaming {
    pub enabled: bool,
    /// The machine's public host (`<id>.wadspaces.com`, made by the installer
    /// stick with its tunnel). Empty: no public streams.
    pub host: String,
    /// Where the proxy listens: the tunnel's origin, on loopback only (both
    /// families). One that can't be bound is skipped.
    pub listen: Vec<String>,
    /// The sidecar (images/stream).
    pub image: String,
    /// Each stream's sidecar web port (on 127.0.0.1) and WebRTC mux port
    /// come from these ranges: `ports` ports from each first one.
    pub first_web_port: u16,
    pub first_mux_port: u16,
    /// A private stream's port: wadd's proxy for it on 127.0.0.1 (its own
    /// listener), served on the tailnet at the same number (`tailscale serve
    /// --https=<port>`).
    pub first_private_port: u16,
    pub ports: u16,
    /// A stream nobody has watched or touched for this long is stopped.
    pub idle_minutes: u64,
    /// Cloudflare TURN's key (`[cloudflare.turn]` token_id, api_token): the
    /// installer stick puts it here, 0600. Missing: STUN only (viewers with
    /// a direct path connect; the stream says so).
    pub turn: PathBuf,
    /// Cloudflare's TURN API (Realtime): a stream's credentials are minted
    /// at `<turn_api>/turn/keys/<token_id>/credentials/generate-ice-servers`.
    /// A test points it at a mock.
    pub turn_api: String,
    /// How long a stream's TURN credentials last (the `ttl` asked for). wadd
    /// writes new ones to the sidecar before they run out.
    pub turn_ttl_s: u64,
    /// The OCI runtime a secure stream's wadspace runs in (`podman
    /// --runtime=<it>`: a VM of its own). Empty: no secure streams.
    pub secure_runtime: String,
    /// A secure stream's VM: its vCPUs and its memory (MiB, taken from the
    /// machine as the VM uses it). libkrun's annotations `krun.cpus` and
    /// `krun.ram_mib` (wad_gen::units::secure_units).
    pub secure_cpus: u32,
    pub secure_ram_mib: u32,
    /// cloudflared's metrics server (its /ready says whether the tunnel is up).
    pub metrics: String,
    /// What carries `https://<host>` to `listen`: "cloudflared", the
    /// machine's own tunnel (public is available with its token in
    /// /etc/cloudflared, cloudflared.service active and its `/ready`
    /// connected); or "external", **for tests only**: something wadd doesn't
    /// run does, e.g. a quick tunnel (`cloudflared tunnel --url
    /// http://localhost:8085`, a random `*.trycloudflare.com` host) that a
    /// test harness starts, and public is available whenever `host` is set.
    pub tunnel: String,
}

impl Default for Streaming {
    fn default() -> Self {
        Self {
            enabled: true,
            host: String::new(),
            listen: vec!["127.0.0.1:8085".into(), "[::1]:8085".into()],
            image: "localhost/wados-stream:44".into(),
            first_web_port: 48100,
            first_mux_port: 48200,
            first_private_port: 48300,
            ports: 100,
            idle_minutes: 30,
            turn: "/etc/wados/turn.toml".into(),
            turn_api: "https://rtc.live.cloudflare.com/v1".into(),
            turn_ttl_s: 86_400,
            secure_runtime: "krun".into(),
            secure_cpus: 2,
            secure_ram_mib: 2048,
            metrics: "127.0.0.1:20241".into(),
            tunnel: "cloudflared".into(),
        }
    }
}

/// The owner's tailnet (Tailscale): the machine joins it, the owner's other
/// machines and the devices they trust reach wadd's remote API over it.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Tailnet {
    pub enabled: bool,
    /// tailscaled's LocalAPI.
    pub socket: PathBuf,
    /// The `tailscale` CLI (joining, leaving, serve, drive).
    pub cli: PathBuf,
    /// The remote API's port, on the machine's tailnet addresses.
    pub port: u16,
    /// Listen here instead (a laptop's trial: tailscaled in userspace mode
    /// forwards the tailnet's connections to loopback). Empty: the machine's
    /// tailnet addresses, with `port`.
    pub listen: Vec<String>,
    /// The interface the remote API is bound to, so nothing from another
    /// network reaches it. Empty: none.
    pub bind_device: String,
    /// The machine's name on the tailnet. Empty: from [machine] name.
    pub hostname: String,
    /// How often tailscaled's status is read.
    pub poll_s: u64,
    /// The control server a login uses (a Headscale's: `tailscale up
    /// --login-server`). Empty: the one tailscaled has (Tailscale's, unless
    /// it was pointed elsewhere).
    pub login_server: String,
    /// Share the person's home on the tailnet (Taildrive, as them) once it's running.
    pub drive: bool,
    /// What runs Taildrive's commands as the person.
    pub runuser: PathBuf,
}

impl Tailnet {
    pub fn for_profile(profile: Profile) -> Self {
        Self {
            // A laptop has its own Tailscale, if any: a trial turns it on.
            enabled: profile == Profile::System,
            socket: "/run/tailscale/tailscaled.sock".into(),
            cli: "/usr/bin/tailscale".into(),
            port: 8087,
            listen: vec![],
            bind_device: "tailscale0".into(),
            hostname: String::new(),
            poll_s: 10,
            login_server: String::new(),
            drive: true,
            runuser: "/usr/sbin/runuser".into(),
        }
    }
}

impl Default for Tailnet {
    fn default() -> Self {
        Self::for_profile(Profile::System)
    }
}

/// The screen: the user's labwc session, which the shell reports to wadd
/// (its Wayland socket) when it starts. Until it does, wadd looks here.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Display {
    pub enabled: bool,
    /// The session's runtime dir (the newest `wayland-*` socket in it).
    pub runtime_dir: PathBuf,
    /// A fixed socket instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socket: Option<PathBuf>,
}

impl Display {
    pub fn for_profile(profile: Profile) -> Self {
        Self {
            enabled: true,
            // The account made at first boot (uid 1000); on a laptop, yours.
            runtime_dir: match profile {
                Profile::System => "/run/user/1000".into(),
                Profile::User => runtime_dir(),
            },
            socket: None,
        }
    }
}

impl Default for Display {
    fn default() -> Self {
        Self::for_profile(Profile::System)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Daemon {
    /// The API's Unix socket (when systemd doesn't hand one over).
    pub socket: PathBuf,
    /// Where wadd keeps its state: the wadspaces, tags, keys, runs.
    pub state_dir: PathBuf,
    /// Who may use the API, besides root and wadd's own user: these users...
    pub allow_users: Vec<String>,
    /// ...and members of these groups.
    pub allow_groups: Vec<String>,
    /// Podman's API socket.
    pub podman_socket: PathBuf,
    /// Where wadd writes the wadspaces' quadlet units (rewritten at every
    /// start, so they're never stale; quadlet reads this directory).
    pub units_dir: PathBuf,
    /// Project folders, one per project.
    pub projects_dir: PathBuf,
    /// Keys (saved sign-ins), a folder each, mounted into the wadspaces that use them.
    pub signins_dir: PathBuf,
    /// How long a wadspace may take to come up.
    pub ready_timeout_s: u64,
    /// How often wadd checks the wadspaces against podman.
    pub reconcile_s: u64,
    /// Who owns project folders: the user inside the images (abc). git runs
    /// as them when wadd is root.
    pub projects_uid: u32,
    /// Folder projects and the folder browser stay inside these.
    pub folder_roots: Vec<PathBuf>,
    /// A build won't start with less free disk than this.
    pub build_min_free_gb: u64,
    /// The wadspaces' screen folders, one each (`<screen_dir>/<id>`: the
    /// Wayland socket wadd has the compositor serve it alone, and sound's
    /// folder), mounted at /run/wadspaces-display.
    pub screen_dir: PathBuf,
    /// How much of your home a wadspace's COPY steps may take into a build, in MB.
    pub copy_max_mb: u64,
}

/// Home folders, and where drives get mounted.
pub const FOLDER_ROOTS: [&str; 6] = ["/var/home", "/home", "/mnt", "/media", "/run/media", "/run/wadspaces-drives"];

impl Daemon {
    pub fn for_profile(profile: Profile) -> Self {
        let common = |socket, state_dir: PathBuf, podman_socket, units_dir, screen_dir| Self {
            socket,
            podman_socket,
            units_dir,
            screen_dir,
            copy_max_mb: 256,
            projects_dir: state_dir.join("projects"),
            signins_dir: state_dir.join("keys"),
            state_dir,
            allow_users: vec![],
            allow_groups: vec![],
            ready_timeout_s: 900,
            reconcile_s: 5,
            projects_uid: 1000,
            folder_roots: FOLDER_ROOTS.iter().map(PathBuf::from).collect(),
            build_min_free_gb: 8,
        };
        match profile {
            Profile::System => Self {
                // The machine's people: the account made at first boot is in
                // group wados (and wheel, as its admin).
                allow_groups: vec!["wados".into(), "wheel".into()],
                ..common(
                    "/run/wadd/wadd.sock".into(),
                    "/var/lib/wados".into(),
                    "/run/podman/podman.sock".into(),
                    "/run/containers/systemd".into(),
                    // wadd.service's RuntimeDirectory.
                    "/run/wados/screen".into(),
                )
            },
            Profile::User => common(
                runtime_dir().join("wadd/wadd.sock"),
                home().join(".local/state/wados"),
                runtime_dir().join("podman/podman.sock"),
                runtime_dir().join("containers/systemd"),
                runtime_dir().join("wados/screen"),
            ),
        }
    }
}

impl Default for Daemon {
    fn default() -> Self {
        Self::for_profile(Profile::System)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Log {
    /// `tracing` filter, e.g. "info" or "wadd=debug,info".
    pub level: String,
    /// Lines kept in memory for /v1/logs.
    pub buffer_lines: usize,
}

impl Default for Machine {
    fn default() -> Self {
        Self { name: hostname().unwrap_or_else(|| "wados".into()), id: String::new() }
    }
}

impl Default for Log {
    fn default() -> Self {
        Self { level: "info".into(), buffer_lines: 2000 }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::defaults(Profile::System)
    }
}

impl Config {
    pub fn defaults(profile: Profile) -> Self {
        Self {
            machine: Machine::default(),
            daemon: Daemon::for_profile(profile),
            log: Log::default(),
            display: Display::for_profile(profile),
            streaming: Streaming::default(),
            tailnet: Tailnet::for_profile(profile),
            packages: Packages::default(),
            github: Github::default(),
        }
    }

    /// The files read for a profile, in order (missing ones are skipped).
    pub fn files(profile: Profile) -> Vec<PathBuf> {
        match profile {
            Profile::System => vec![VENDOR.into(), ADMIN.into()],
            Profile::User => vec![VENDOR.into(), config_home().join("wados/wadd.toml")],
        }
    }

    /// The profile's defaults, then each file that exists, merged in order.
    pub fn load(profile: Profile, files: &[PathBuf]) -> Result<Self, Error> {
        let mut merged = toml::Table::new();
        for f in files {
            match std::fs::read_to_string(f) {
                Ok(text) => {
                    let t: toml::Table = text.parse().map_err(|e| Error::Parse(f.clone(), Box::new(e)))?;
                    merge(&mut merged, t);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::Read(f.clone(), e)),
            }
        }
        Self::from_table(profile, merged).map_err(|e| Error::Invalid(Box::new(e)))
    }

    /// Defaults overlaid with a (merged) table.
    pub fn from_table(profile: Profile, table: toml::Table) -> Result<Self, toml::de::Error> {
        let mut base = toml::Table::try_from(Self::defaults(profile)).expect("defaults serialize");
        merge(&mut base, table);
        base.try_into()
    }

    /// The machine's id: [machine] id, else its hostname made DNS-safe.
    pub fn machine_id(&self) -> String {
        if !self.machine.id.is_empty() {
            return self.machine.id.clone();
        }
        let id: String = self
            .machine
            .name
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let id = id.trim_matches('-').to_string();
        if id.is_empty() { "wados".into() } else { id }
    }
}

/// `over` into `base`: tables merge, anything else replaces.
fn merge(base: &mut toml::Table, over: toml::Table) {
    for (k, v) in over {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}: {1}")]
    Read(PathBuf, #[source] std::io::Error),
    #[error("{0}: {1}")]
    Parse(PathBuf, #[source] Box<toml::de::Error>),
    #[error("configuration: {0}")]
    Invalid(#[source] Box<toml::de::Error>),
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn home() -> PathBuf {
    env_path("HOME").unwrap_or_else(|| "/".into())
}

fn runtime_dir() -> PathBuf {
    env_path("XDG_RUNTIME_DIR").unwrap_or_else(std::env::temp_dir)
}

fn config_home() -> PathBuf {
    env_path("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config"))
}

/// The configured hostname, else the kernel's (a transient one has no file).
fn hostname() -> Option<String> {
    ["/etc/hostname", "/proc/sys/kernel/hostname"]
        .iter()
        .find_map(|f| std::fs::read_to_string(f).ok().map(|h| h.trim().to_string()).filter(|h| !h.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_file_overrides_key_by_key() {
        let dir = tempfile::tempdir().unwrap();
        let (vendor, admin) = (dir.path().join("vendor.toml"), dir.path().join("admin.toml"));
        std::fs::write(&vendor, "[streaming]\nidle_minutes = 10\n[machine]\nname = \"Surface\"\n").unwrap();
        std::fs::write(&admin, "[streaming]\nhost = \"surface.wadspaces.com\"\n[machine]\nid = \"surface\"\n").unwrap();
        let c = Config::load(Profile::System, &[vendor, admin, dir.path().join("missing.toml")]).unwrap();
        assert_eq!(c.streaming.idle_minutes, 10);
        assert_eq!(c.streaming.host, "surface.wadspaces.com");
        assert_eq!((c.machine.name.as_str(), c.machine.id.as_str()), ("Surface", "surface"));
        // What the files didn't say is the profile's.
        assert_eq!(c.daemon.state_dir, PathBuf::from("/var/lib/wados"));
        assert_eq!(c.streaming.turn, PathBuf::from("/etc/wados/turn.toml"));
    }

    #[test]
    fn without_a_host_there_are_no_public_streams_and_no_secret_in_the_config() {
        let c = Config::defaults(Profile::System);
        assert!(c.streaming.host.is_empty());
        let text = toml::to_string(&c).unwrap();
        // The TURN key is its file's (`turn`), never a value here; v1's [cloud] (Firebase) is gone.
        for word in ["token", "api_key", "password", "secret", "firebase", "[cloud]"] {
            assert!(!text.contains(word), "{word} in the defaults");
        }
        // Cloudflare's TURN API is only where to ask, with the key from that file.
        assert_eq!(c.streaming.turn_api, "https://rtc.live.cloudflare.com/v1");
        assert_eq!((c.streaming.turn_ttl_s, c.streaming.secure_runtime.as_str()), (86_400, "krun"));
        assert_eq!((c.streaming.secure_cpus, c.streaming.secure_ram_mib), (2, 2048));
    }

    #[test]
    fn a_machine_without_an_id_uses_its_name_made_dns_safe() {
        let mut c = Config::defaults(Profile::User);
        c.machine.name = "Sam's Surface Book".into();
        assert_eq!(c.machine_id(), "sam-s-surface-book");
        c.machine.id = "surface".into();
        assert_eq!(c.machine_id(), "surface");
    }

    #[test]
    fn turns_api_points_at_a_mock_and_secure_streams_turn_off() {
        let t: toml::Table =
            "[streaming]\nturn_api = \"http://127.0.0.1:9/v1\"\nturn_ttl_s = 600\nsecure_runtime = \"\"\n"
                .parse()
                .unwrap();
        let c = Config::from_table(Profile::System, t).unwrap();
        assert_eq!((c.streaming.turn_api.as_str(), c.streaming.turn_ttl_s), ("http://127.0.0.1:9/v1", 600));
        assert!(c.streaming.secure_runtime.is_empty());
        assert_eq!(c.streaming.first_private_port, 48300);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let t: toml::Table = "[cloud]\nproject_id = \"old\"\n[keys]\nhome = [\"KEY_0\"]\n".parse().unwrap();
        assert!(Config::from_table(Profile::System, t).is_ok());
    }
}
