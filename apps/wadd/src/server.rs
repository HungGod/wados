//! Server mode (`GET|POST /v1/server`): the screen goes off and this machine
//! only streams. Wadspaces on the screen or in windows stop; streams keep
//! going. It's v1's, adapted:
//!
//! - **The flag** (`/run/wados/server/on`) says it's on: it outlasts wadd
//!   restarting, and not the machine. greetd won't start while it's there
//!   (its `ExecCondition`), so a stray restart can't bring a session back.
//! - **`wados-console.service`** runs `wadd console` (console.rs) on tty1
//!   under a logind inhibitor, so the machine stays up with its lid shut.
//!   wadd stops one and starts the other. They don't conflict in systemd:
//!   a stray `start greetd`, refused by its condition, would still stop the
//!   console, and leave no way back.
//! - **Coming back** starts greetd, and every start lands on the lock.
//!
//! The kernel gives Ctrl+Alt+Delete to systemd only on a text console, which
//! is why the session's labwc never sees it reach `ctrl-alt-del.target`. That
//! target's one job asks the console for its menu (`menu` beside the flag).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use wad_model::SessionDisplay;
use wad_proto::{ApiError, Event, Server};

use crate::events::Bus;
use crate::focus::MachineFocus;
use crate::runs::Runs;

/// wadd.service's RuntimeDirectory.
pub const RUN_DIR: &str = "/run/wados/server";
pub const SERVER_OFF: &str = "This machine is in server mode: its screen is off";

/// The seat.
#[async_trait]
pub trait Hands: Send + Sync + 'static {
    /// Ends the session, and shows the console.
    async fn seat_off(&self) -> Result<(), String>;
    /// Starts the session, which locks first.
    async fn seat_on(&self) -> Result<(), String>;
}

/// The machine's: systemd and logind.
pub struct SystemHands {
    pub uid: u32,
}

async fn run(prog: &str, args: &[&str]) -> Result<(), String> {
    let out = tokio::process::Command::new(prog).args(args).output().await.map_err(|e| format!("{prog}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{prog} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

#[async_trait]
impl Hands for SystemHands {
    async fn seat_off(&self) -> Result<(), String> {
        run("systemctl", &["stop", "greetd.service"]).await?;
        run("systemctl", &["start", "wados-console.service"]).await?;
        // greetd's gone, so its session is ending anyway: this is to be sure.
        if let Err(e) = run("loginctl", &["terminate-user", &self.uid.to_string()]).await {
            tracing::info!("server mode: {e}");
        }
        Ok(())
    }

    async fn seat_on(&self) -> Result<(), String> {
        run("systemctl", &["stop", "wados-console.service"]).await?;
        run("systemctl", &["start", "greetd.service"]).await
    }
}

/// A laptop's wadd has no seat to give up.
pub struct NoSeat;

#[async_trait]
impl Hands for NoSeat {
    async fn seat_off(&self) -> Result<(), String> {
        Err("Server mode is a machine's own".into())
    }
    async fn seat_on(&self) -> Result<(), String> {
        Ok(())
    }
}

pub struct ServerMode {
    flag: PathBuf,
    on: AtomicBool,
    runs: Arc<Runs>,
    focus: Arc<MachineFocus>,
    hands: Arc<dyn Hands>,
    bus: Bus,
}

impl ServerMode {
    pub fn new(dir: &Path, runs: Arc<Runs>, focus: Arc<MachineFocus>, hands: Arc<dyn Hands>, bus: Bus) -> Arc<Self> {
        let flag = dir.join("on");
        let on = AtomicBool::new(flag.exists());
        let me = Arc::new(Self { flag, on, runs, focus, hands, bus });
        me.publish();
        me
    }

    pub fn get(&self) -> Server {
        Server { on: self.on.load(Ordering::SeqCst) }
    }

    fn publish(&self) {
        self.bus.publish(Event::Server { on: self.get().on });
    }

    /// May a wadspace open this way now? Streams may.
    pub fn check_open(&self, display: SessionDisplay) -> Result<(), ApiError> {
        if self.get().on && display != SessionDisplay::Stream {
            return Err(ApiError::conflict(SERVER_OFF));
        }
        Ok(())
    }

    pub async fn set(self: &Arc<Self>, on: bool) -> Result<Server, ApiError> {
        if on == self.get().on {
            return Ok(self.get());
        }
        if on { self.enter().await } else { self.leave().await }
    }

    async fn enter(self: &Arc<Self>) -> Result<Server, ApiError> {
        self.focus.check_free()?;
        let fail = |e: std::io::Error| ApiError::internal(format!("server mode's flag: {e}"));
        if let Some(d) = self.flag.parent() {
            std::fs::create_dir_all(d).map_err(fail)?;
        }
        std::fs::write(&self.flag, b"").map_err(fail)?;
        self.on.store(true, Ordering::SeqCst);
        self.publish();
        for r in self.runs.states() {
            let shown =
                matches!(r.display, Some(SessionDisplay::Screen | SessionDisplay::Window | SessionDisplay::Background));
            if shown && let Err(e) = self.runs.stop(&r.id).await {
                tracing::warn!("server mode: stopping {}: {}", r.id, e.message);
            }
        }
        // After answering: this ends the session that asked.
        let me = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            if let Err(e) = me.hands.seat_off().await {
                tracing::error!("server mode: {e}");
            }
        });
        Ok(self.get())
    }

    async fn leave(&self) -> Result<Server, ApiError> {
        match std::fs::remove_file(&self.flag) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(ApiError::internal(format!("server mode's flag: {e}")));
            }
            _ => {}
        }
        self.on.store(false, Ordering::SeqCst);
        self.publish();
        // Not in this request: the console that asked is the first thing
        // stopped, and its connection going would cancel what's left of it.
        let hands = self.hands.clone();
        tokio::spawn(async move {
            if let Err(e) = hands.seat_on().await {
                tracing::error!("back from server mode: {e}");
            }
        });
        Ok(self.get())
    }
}
