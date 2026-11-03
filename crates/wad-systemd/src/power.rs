//! Shutting down and restarting through logind (the kiosk has no desktop
//! menu). wadd is root on a machine, so it isn't asked; interactive is off,
//! so it never waits on a polkit prompt. Server mode uses it too: ending the
//! kiosk user's session, and an inhibitor so the lid doesn't suspend.

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login {
    fn power_off(&self, interactive: bool) -> zbus::Result<()>;
    fn reboot(&self, interactive: bool) -> zbus::Result<()>;
    fn terminate_user(&self, uid: u32) -> zbus::Result<()>;
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<zbus::zvariant::OwnedFd>;
}

#[derive(Clone)]
pub struct Power {
    conn: zbus::Connection,
}

impl Power {
    pub async fn system() -> Result<Self, zbus::Error> {
        Ok(Self::on(zbus::Connection::system().await?))
    }

    /// On this bus (tests: a private one).
    pub fn on(conn: zbus::Connection) -> Self {
        Self { conn }
    }

    pub async fn power_off(&self) -> Result<(), zbus::Error> {
        LoginProxy::new(&self.conn).await?.power_off(false).await
    }

    pub async fn reboot(&self) -> Result<(), zbus::Error> {
        LoginProxy::new(&self.conn).await?.reboot(false).await
    }

    /// Ends every session of a user and kills what's left of them (their
    /// user manager too).
    pub async fn terminate_user(&self, uid: u32) -> Result<(), zbus::Error> {
        LoginProxy::new(&self.conn).await?.terminate_user(uid).await
    }

    /// An inhibitor lock ("sleep:idle:handle-lid-switch", mode "block"):
    /// held until the descriptor is closed.
    pub async fn inhibit(
        &self,
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> Result<std::os::fd::OwnedFd, zbus::Error> {
        Ok(LoginProxy::new(&self.conn).await?.inhibit(what, who, why, mode).await?.into())
    }
}
