//! The OAuth device flow (RFC 8628, as GitHub has it): how wadd signs in to
//! GitHub without a client secret or a redirect.
//!
//! 1. [`Github::device_code`]: `POST <web>/login/device/code` with the
//!    client ID and the scopes (space-separated) answers a [`DeviceCode`]:
//!    the code the person types (`user_code`, "WDJB-MJHT"), where
//!    (`verification_uri`, `https://github.com/login/device`), for how long
//!    (`expires_in`, 900 s) and how often to ask (`interval`, 5 s).
//! 2. [`Github::poll`]: `POST <web>/login/oauth/access_token` with the
//!    device code every `interval`, until GitHub gives the token
//!    ([`Granted`]) or says it won't. `authorization_pending` asks again;
//!    `slow_down` adds 5 s to the interval (or takes GitHub's new one, if
//!    longer) for every later ask; `expired_token`, `access_denied`,
//!    `incorrect_device_code` and `device_flow_disabled` end it
//!    ([`Error`]'s words say why). A network blip or GitHub's own trouble
//!    doesn't: it asks again at the next interval.
//!
//! Its waits are a [`Clock`]'s, so tests run it without waiting.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde_json::Value;

use crate::{Error, Github, Token, USER_AGENT, split_scopes};

/// The device flow's grant type.
pub const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// What `slow_down` adds to the interval.
pub const SLOW_DOWN: Duration = Duration::from_secs(5);
/// The interval when GitHub doesn't say.
const INTERVAL: u64 = 5;

/// GitHub's answer to `/login/device/code`. Debug leaves the device code out.
#[derive(Clone, PartialEq, Eq, serde::Deserialize)]
pub struct DeviceCode {
    /// What polling asks with (only wadd has it).
    pub device_code: String,
    /// What the person types on GitHub's page ("WDJB-MJHT").
    pub user_code: String,
    /// Where (`https://github.com/login/device`).
    pub verification_uri: String,
    /// Seconds until the codes expire.
    pub expires_in: u64,
    /// Seconds between polls, at least.
    #[serde(default = "interval")]
    pub interval: u64,
}

fn interval() -> u64 {
    INTERVAL
}

impl std::fmt::Debug for DeviceCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceCode")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .field("expires_in", &self.expires_in)
            .field("interval", &self.interval)
            .finish_non_exhaustive()
    }
}

/// The token GitHub gave, and what it may do (its `scope`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Granted {
    pub token: Token,
    pub scopes: Vec<String>,
}

/// One poll's answer, when it isn't an end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    Granted(Granted),
    /// `authorization_pending`: not yet.
    Pending,
    /// `slow_down`: ask less often (GitHub's new interval, if it said).
    SlowDown(Option<u64>),
}

/// Time for the device flow: now, and waiting. [`RealClock`] is tokio's.
pub trait Clock: Send + Sync {
    /// How long since some fixed moment (the clock's start).
    fn now(&self) -> Duration;
    fn sleep(&self, d: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// tokio's time.
#[derive(Debug, Clone, Copy)]
pub struct RealClock(std::time::Instant);

impl Default for RealClock {
    fn default() -> Self {
        Self(std::time::Instant::now())
    }
}

impl Clock for RealClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
    fn sleep(&self, d: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep(d))
    }
}

/// An OAuth error (`error`, `error_description`) that ends the sign-in.
fn ends(error: &str, description: &str) -> Error {
    match error {
        "expired_token" => Error::Expired,
        "access_denied" => Error::Denied,
        "incorrect_device_code" => Error::IncorrectCode,
        "device_flow_disabled" => Error::DeviceFlowDisabled,
        "incorrect_client_credentials" | "unauthorized_client" => Error::UnknownApp,
        _ => Error::OAuth { error: error.into(), description: description.into() },
    }
}

impl Github {
    /// A form to the sign-in's `path`: its JSON answer, and its status.
    async fn oauth(&self, path: &str, form: &[(&str, &str)]) -> Result<(u16, Value), Error> {
        let res = self
            .http
            .post(format!("{}{path}", self.web))
            .header("accept", "application/json")
            .header("user-agent", USER_AGENT)
            .form(form)
            .send()
            .await
            .map_err(Error::Network)?;
        let status = res.status().as_u16();
        let text = res.text().await.map_err(Error::Network)?;
        let body = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !body.is_object() && !(200..300).contains(&status) && status != 404 {
            return Err(Error::Upstream { status, message: text.chars().take(200).collect() });
        }
        Ok((status, body))
    }

    /// Asks GitHub for a device code for the app `client_id`, for `scopes`.
    pub async fn device_code(&self, client_id: &str, scopes: &[String]) -> Result<DeviceCode, Error> {
        let scope = scopes.join(" ");
        let (status, body) = self.oauth("/login/device/code", &[("client_id", client_id), ("scope", &scope)]).await?;
        // No such app: GitHub's 404 for a client ID it doesn't know.
        if status == 404 {
            return Err(Error::UnknownApp);
        }
        if let Some(e) = body.get("error").and_then(Value::as_str) {
            let d = body.get("error_description").and_then(Value::as_str).unwrap_or_default();
            return Err(ends(e, d));
        }
        serde_json::from_value(body)
            .map_err(|e| Error::Upstream { status, message: format!("GitHub's device code: {e}") })
    }

    /// Asks once whether the person has finished: the token, not yet, slow
    /// down, or an end.
    pub async fn poll_once(&self, client_id: &str, device_code: &str) -> Result<Poll, Error> {
        let form = [("client_id", client_id), ("device_code", device_code), ("grant_type", GRANT_TYPE)];
        let (status, body) = self.oauth("/login/oauth/access_token", &form).await?;
        if let Some(t) = body.get("access_token").and_then(Value::as_str).filter(|t| !t.is_empty()) {
            let scopes = body.get("scope").and_then(Value::as_str).map(split_scopes).unwrap_or_default();
            return Ok(Poll::Granted(Granted { token: Token::new(t), scopes }));
        }
        match body.get("error").and_then(Value::as_str) {
            Some("authorization_pending") => Ok(Poll::Pending),
            Some("slow_down") => Ok(Poll::SlowDown(body.get("interval").and_then(Value::as_u64))),
            Some(e) => Err(ends(e, body.get("error_description").and_then(Value::as_str).unwrap_or_default())),
            None => Err(Error::Upstream { status, message: "GitHub gave neither a token nor an error".into() }),
        }
    }

    /// Polls for `code`'s token until GitHub gives it, says it won't, or the
    /// code expires. Dropping the future stops it.
    pub async fn poll(&self, client_id: &str, code: &DeviceCode, clock: &dyn Clock) -> Result<Granted, Error> {
        let deadline = clock.now() + Duration::from_secs(code.expires_in);
        let mut every = Duration::from_secs(code.interval);
        loop {
            clock.sleep(every).await;
            if clock.now() >= deadline {
                return Err(Error::Expired);
            }
            match self.poll_once(client_id, &code.device_code).await {
                Ok(Poll::Granted(g)) => return Ok(g),
                Ok(Poll::Pending) => {}
                Ok(Poll::SlowDown(theirs)) => {
                    every = (every + SLOW_DOWN).max(Duration::from_secs(theirs.unwrap_or(0)));
                    tracing::debug!("GitHub asked to slow down: every {} s now", every.as_secs());
                }
                Err(e) if e.passing() => tracing::info!("asking GitHub for the token again: {e}"),
                Err(e) => return Err(e),
            }
        }
    }
}
