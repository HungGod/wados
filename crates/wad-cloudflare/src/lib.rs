//! A machine's public address, set up once, from the installer stick, before
//! its disk is touched ([`Cloudflare::provision`]): a tunnel of its own
//! (`wadspaces-<id>`), whose ingress sends `<id>.<domain>` to wadd's proxy,
//! and one proxied record, `<id>.<domain>` → `<tunnel>.cfargotunnel.com`.
//! There's no wildcard, and no record per stream: streams are paths.
//!
//! Only the stick (and the build computer, for [`Cloudflare::remove`]) has
//! [`Keys`]. What reaches the machine is the tunnel's own token and the TURN
//! key; neither the tunnel creator's token nor the DNS token ever does.
//!
//! Every call is safe to repeat, so an install can be run again.

use std::fmt;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

pub const API: &str = "https://api.cloudflare.com/client/v4";
/// What the machine's tunnel reaches: wadd's proxy.
pub const SERVICE: &str = "http://localhost:8085";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("A machine called {0} is connected already: give this one another name")]
    Taken(String),
    #[error("Cloudflare didn't answer: {0}")]
    Network(String),
    #[error("Cloudflare said: {0}")]
    Refused(String),
    #[error("Cloudflare has no zone {0}")]
    NoZone(String),
    #[error("the stick's Cloudflare file: {0}")]
    File(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// `cloudflare.toml`: the keys, as the secrets folder has them.
#[derive(Clone, Deserialize)]
pub struct Keys {
    pub cloudflare: Sections,
}

#[derive(Clone, Deserialize)]
pub struct Sections {
    pub tunnel: TunnelKeys,
    pub turn: Option<TurnKeys>,
}

#[derive(Clone, Deserialize)]
pub struct TunnelKeys {
    pub dns_edit_api_token: String,
    pub tunnel_creator_api_token: String,
}

#[derive(Clone, Deserialize)]
pub struct TurnKeys {
    pub token_id: String,
    pub api_token: String,
}

impl fmt::Debug for Keys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Keys(…)")
    }
}

impl Keys {
    pub fn parse(text: &str) -> Result<Self> {
        // toml's error quotes the line it's about: never that, here.
        toml::from_str(text).map_err(|e| Error::File(e.message().to_string()))
    }

    /// `/etc/wados/turn.toml`, or none.
    pub fn turn_file(&self) -> Option<String> {
        let t = self.cloudflare.turn.as_ref()?;
        let table = json!({ "cloudflare": { "turn": { "token_id": t.token_id, "api_token": t.api_token } } });
        toml::to_string(&table).ok()
    }
}

/// What the machine gets.
#[derive(Clone, PartialEq, Eq)]
pub struct Address {
    pub host: String,
    pub tunnel_id: String,
    /// cloudflared's token.
    pub token: String,
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Address").field("host", &self.host).field("tunnel_id", &self.tunnel_id).finish_non_exhaustive()
    }
}

pub struct Cloudflare {
    http: reqwest::Client,
    api: String,
    domain: String,
    tunnel_token: String,
    dns_token: String,
}

struct Zone {
    id: String,
    account: String,
}

#[derive(Deserialize)]
struct Tunnel {
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    token: Option<String>,
}

#[derive(Deserialize)]
struct Record {
    id: String,
}

fn tunnel_name(id: &str) -> String {
    format!("wadspaces-{id}")
}

impl Cloudflare {
    pub fn new(api: &str, domain: &str, keys: &Keys) -> Self {
        let t = &keys.cloudflare.tunnel;
        Self {
            http: reqwest::Client::new(),
            api: api.trim_end_matches('/').to_string(),
            domain: domain.to_string(),
            tunnel_token: t.tunnel_creator_api_token.clone(),
            dns_token: t.dns_edit_api_token.clone(),
        }
    }

    pub fn host(&self, id: &str) -> String {
        format!("{id}.{}", self.domain)
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        token: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<T> {
        let mut req = self.http.request(method, format!("{}{path}", self.api)).bearer_auth(token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await.map_err(|e| Error::Network(e.without_url().to_string()))?;
        let status = res.status();
        let v: Value = res.json().await.map_err(|e| Error::Network(e.without_url().to_string()))?;
        if v["success"].as_bool() != Some(true) {
            let says: Vec<&str> =
                v["errors"].as_array().into_iter().flatten().filter_map(|e| e["message"].as_str()).collect();
            return Err(Error::Refused(if says.is_empty() { status.to_string() } else { says.join("; ") }));
        }
        serde_json::from_value(v["result"].clone()).map_err(|e| Error::Refused(e.to_string()))
    }

    async fn zone(&self) -> Result<Zone> {
        #[derive(Deserialize)]
        struct Found {
            id: String,
            account: Id,
        }
        #[derive(Deserialize)]
        struct Id {
            id: String,
        }
        let path = format!("/zones?name={}", self.domain);
        let zones: Vec<Found> = self.call(reqwest::Method::GET, &self.dns_token, &path, None).await?;
        let z = zones.into_iter().next().ok_or_else(|| Error::NoZone(self.domain.clone()))?;
        Ok(Zone { id: z.id, account: z.account.id })
    }

    async fn find_tunnel(&self, account: &str, id: &str) -> Result<Option<Tunnel>> {
        let path = format!("/accounts/{account}/cfd_tunnel?name={}&is_deleted=false", tunnel_name(id));
        let found: Vec<Tunnel> = self.call(reqwest::Method::GET, &self.tunnel_token, &path, None).await?;
        Ok(found.into_iter().next())
    }

    /// The tunnel and its token: an idle one of that name, or a new one.
    async fn tunnel(&self, account: &str, id: &str) -> Result<(String, String)> {
        if let Some(t) = self.find_tunnel(account, id).await? {
            if matches!(t.status.as_str(), "healthy" | "degraded") {
                return Err(Error::Taken(id.to_string()));
            }
            let path = format!("/accounts/{account}/cfd_tunnel/{}/token", t.id);
            let token: String = self.call(reqwest::Method::GET, &self.tunnel_token, &path, None).await?;
            return Ok((t.id, token));
        }
        let path = format!("/accounts/{account}/cfd_tunnel");
        let body = json!({ "name": tunnel_name(id), "config_src": "cloudflare" });
        let t: Tunnel = self.call(reqwest::Method::POST, &self.tunnel_token, &path, Some(body)).await?;
        let token = t.token.ok_or_else(|| Error::Refused("the new tunnel came without a token".into()))?;
        Ok((t.id, token))
    }

    async fn ingress(&self, account: &str, tunnel: &str, host: &str) -> Result<()> {
        let path = format!("/accounts/{account}/cfd_tunnel/{tunnel}/configurations");
        let body = json!({ "config": { "ingress": [
            { "hostname": host, "service": SERVICE },
            { "service": "http_status:404" },
        ]}});
        let _: Value = self.call(reqwest::Method::PUT, &self.tunnel_token, &path, Some(body)).await?;
        Ok(())
    }

    async fn records(&self, zone: &str, host: &str) -> Result<Vec<Record>> {
        let path = format!("/zones/{zone}/dns_records?type=CNAME&name={host}");
        self.call(reqwest::Method::GET, &self.dns_token, &path, None).await
    }

    async fn record(&self, zone: &str, host: &str, tunnel: &str) -> Result<()> {
        let body = json!({
            "type": "CNAME",
            "name": host,
            "content": format!("{tunnel}.cfargotunnel.com"),
            "proxied": true,
            "ttl": 1,
        });
        let path = format!("/zones/{zone}/dns_records");
        let found = self.records(zone, host).await?;
        let _: Value = match found.first() {
            Some(r) => {
                self.call(reqwest::Method::PUT, &self.dns_token, &format!("{path}/{}", r.id), Some(body)).await?
            }
            None => self.call(reqwest::Method::POST, &self.dns_token, &path, Some(body)).await?,
        };
        Ok(())
    }

    /// `<id>.<domain>`: everything a machine's public address needs.
    pub async fn provision(&self, id: &str) -> Result<Address> {
        let zone = self.zone().await?;
        let host = self.host(id);
        let (tunnel_id, token) = self.tunnel(&zone.account, id).await?;
        self.ingress(&zone.account, &tunnel_id, &host).await?;
        self.record(&zone.id, &host, &tunnel_id).await?;
        Ok(Address { host, tunnel_id, token })
    }

    /// A machine retired: its record and tunnel gone. True if it had a tunnel.
    pub async fn remove(&self, id: &str) -> Result<bool> {
        let zone = self.zone().await?;
        let host = self.host(id);
        for r in self.records(&zone.id, &host).await? {
            let path = format!("/zones/{}/dns_records/{}", zone.id, r.id);
            let _: Value = self.call(reqwest::Method::DELETE, &self.dns_token, &path, None).await?;
        }
        let Some(t) = self.find_tunnel(&zone.account, id).await? else {
            return Ok(false);
        };
        let path = format!("/accounts/{}/cfd_tunnel/{}", zone.account, t.id);
        let _: Value =
            self.call(reqwest::Method::DELETE, &self.tunnel_token, &format!("{path}/connections"), None).await?;
        let _: Value = self.call(reqwest::Method::DELETE, &self.tunnel_token, &path, None).await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests;
