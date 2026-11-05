//! systemd over D-Bus, for wadd's workspace units (quadlet's
//! wad-<id>.service): the system manager on a machine, your user manager on
//! a laptop (`wadd serve --user`). Each operation waits for its job to finish
//! (JobRemoved) and says how it ended, instead of shelling out to systemctl.
//! Apps on the machine itself (the terminal, WadBrowser) run in transient
//! units of the same name, made on the spot ([`Systemd::start_transient`]).

use std::time::Duration;

mod power;
pub use power::Power;

use futures_util::StreamExt;
use zbus::zvariant::{OwnedObjectPath, Value};

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
trait Manager {
    fn start_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn stop_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn restart_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn reload(&self) -> zbus::Result<()>;
    fn subscribe(&self) -> zbus::Result<()>;
    fn load_unit(&self, name: &str) -> zbus::Result<OwnedObjectPath>;
    fn reset_failed_unit(&self, name: &str) -> zbus::Result<()>;
    fn start_transient_unit(
        &self,
        name: &str,
        mode: &str,
        properties: &[(&str, Value<'_>)],
        aux: &[(&str, &[(&str, Value<'_>)])],
    ) -> zbus::Result<OwnedObjectPath>;
    #[zbus(signal)]
    fn job_removed(&self, id: u32, job: OwnedObjectPath, unit: String, result: String) -> zbus::Result<()>;
}

#[zbus::proxy(interface = "org.freedesktop.systemd1.Unit", default_service = "org.freedesktop.systemd1")]
trait Unit {
    #[zbus(property)]
    fn active_state(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn load_state(&self) -> zbus::Result<String>;
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("systemd: {0}")]
    Bus(#[from] zbus::Error),
    /// The job ended, but not well: "failed", "timeout", "dependency", ...
    #[error("{op} {unit}: {result}")]
    Job { op: &'static str, unit: String, result: String },
    #[error("{op} {unit}: no answer from systemd in {secs} s")]
    Timeout { op: &'static str, unit: String, secs: u64 },
}

/// A transient service: a program run as a unit without a unit file. It is
/// gone once it stops (CollectMode=inactive-or-failed), so a later start
/// makes it afresh.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TransientSpec {
    pub description: String,
    /// The program (an absolute path) and its arguments.
    pub exec: Vec<String>,
    /// Who it runs as (the system manager only; a user manager runs it as you).
    pub user: Option<String>,
    pub group: Option<String>,
    pub working_directory: Option<String>,
    /// NAME=value
    pub environment: Vec<String>,
}

impl TransientSpec {
    fn properties(&self) -> Vec<(&'static str, Value<'_>)> {
        let program = self.exec.first().cloned().unwrap_or_default();
        let exec_start = vec![(program, self.exec.clone(), false)];
        let mut p: Vec<(&'static str, Value<'_>)> = vec![
            ("Description", Value::from(self.description.as_str())),
            // "exec": the start fails when the program can't be run, rather
            // than the unit dying a moment later.
            ("Type", Value::from("exec")),
            ("ExecStart", Value::new(exec_start)),
            ("CollectMode", Value::from("inactive-or-failed")),
            ("Environment", Value::new(self.environment.clone())),
        ];
        if let Some(u) = &self.user {
            p.push(("User", Value::from(u.as_str())));
        }
        if let Some(g) = &self.group {
            p.push(("Group", Value::from(g.as_str())));
        }
        if let Some(d) = &self.working_directory {
            p.push(("WorkingDirectory", Value::from(d.as_str())));
        }
        p
    }
}

#[derive(Clone)]
pub struct Systemd {
    conn: zbus::Connection,
}

#[derive(Clone, Copy)]
enum Op<'a> {
    Start,
    Stop,
    Restart,
    Transient(&'a TransientSpec),
}

impl Op<'_> {
    fn name(self) -> &'static str {
        match self {
            Op::Start | Op::Transient(_) => "start",
            Op::Stop => "stop",
            Op::Restart => "restart",
        }
    }
}

impl Systemd {
    /// The system manager (a machine; wadd runs as root).
    pub async fn system() -> Result<Self, Error> {
        Ok(Self { conn: zbus::Connection::system().await? })
    }

    /// Your user manager (a laptop).
    pub async fn user() -> Result<Self, Error> {
        Ok(Self { conn: zbus::Connection::session().await? })
    }

    async fn manager(&self) -> Result<ManagerProxy<'_>, Error> {
        Ok(ManagerProxy::new(&self.conn).await?)
    }

    /// Re-reads unit files (and runs generators: quadlet turns
    /// wad-<id>.container into wad-<id>.service here). Returns once done.
    pub async fn reload(&self) -> Result<(), Error> {
        Ok(self.manager().await?.reload().await?)
    }

    pub async fn start(&self, unit: &str, timeout: Duration) -> Result<(), Error> {
        self.job(Op::Start, unit, timeout).await
    }

    pub async fn stop(&self, unit: &str, timeout: Duration) -> Result<(), Error> {
        self.job(Op::Stop, unit, timeout).await
    }

    pub async fn restart(&self, unit: &str, timeout: Duration) -> Result<(), Error> {
        self.job(Op::Restart, unit, timeout).await
    }

    /// Makes the transient unit `unit` and starts it. Refused while a unit
    /// of that name is still loaded (running, or stopping).
    pub async fn start_transient(&self, unit: &str, spec: &TransientSpec, timeout: Duration) -> Result<(), Error> {
        self.job(Op::Transient(spec), unit, timeout).await
    }

    /// Forgets a unit's failure (`systemctl reset-failed`), so one whose
    /// file is gone unloads.
    pub async fn reset_failed(&self, unit: &str) -> Result<(), Error> {
        Ok(self.manager().await?.reset_failed_unit(unit).await?)
    }

    /// "active", "inactive", "failed", "activating", ...
    pub async fn active_state(&self, unit: &str) -> Result<String, Error> {
        let path = self.manager().await?.load_unit(unit).await?;
        let u = UnitProxy::builder(&self.conn).path(path)?.build().await?;
        Ok(u.active_state().await?)
    }

    async fn job(&self, op: Op<'_>, unit: &str, timeout: Duration) -> Result<(), Error> {
        let m = self.manager().await?;
        m.subscribe().await.ok(); // already subscribed is fine
        // Listen before asking, so a quick job's end isn't missed.
        let mut removed = m.receive_job_removed().await?;
        let job = match op {
            Op::Start => m.start_unit(unit, "replace").await?,
            Op::Stop => m.stop_unit(unit, "replace").await?,
            Op::Restart => m.restart_unit(unit, "replace").await?,
            Op::Transient(spec) => m.start_transient_unit(unit, "fail", &spec.properties(), &[]).await?,
        };
        let wait = async {
            while let Some(sig) = removed.next().await {
                let Ok(args) = sig.args() else { continue };
                if args.job == job {
                    return Some(args.result.clone());
                }
            }
            None
        };
        let secs = timeout.as_secs();
        match tokio::time::timeout(timeout, wait).await {
            Ok(Some(r)) if r == "done" => Ok(()),
            Ok(Some(r)) => Err(Error::Job { op: op.name(), unit: unit.into(), result: r }),
            Ok(None) => Err(Error::Job { op: op.name(), unit: unit.into(), result: "the signal stream ended".into() }),
            Err(_) => Err(Error::Timeout { op: op.name(), unit: unit.into(), secs }),
        }
    }
}
