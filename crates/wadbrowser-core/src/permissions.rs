//! Per-site permission preferences stored in SQLite.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    Notifications,
    Geolocation,
    Media,
    Clipboard,
    IdleDetection,
}

impl PermissionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Notifications => "notifications",
            Self::Geolocation => "geolocation",
            Self::Media => "media",
            Self::Clipboard => "clipboard",
            Self::IdleDetection => "idle_detection",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "notifications" => Some(Self::Notifications),
            "geolocation" => Some(Self::Geolocation),
            "media" => Some(Self::Media),
            "clipboard" => Some(Self::Clipboard),
            "idle_detection" => Some(Self::IdleDetection),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    Deny,
    Ask,
}

impl PermissionDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Ask => "ask",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            "ask" => Some(Self::Ask),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SitePermission {
    pub origin: String,
    pub kind: PermissionKind,
    pub decision: PermissionDecision,
}

/// Normalize a page URL down to a scheme://host[:port] origin.
pub fn origin_from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let scheme = parsed.scheme();
    match parsed.port() {
        Some(port) => Some(format!("{scheme}://{host}:{port}")),
        None => Some(format!("{scheme}://{host}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_strips_path() {
        assert_eq!(origin_from_url("https://Example.com:8443/a/b?x=1").as_deref(), Some("https://example.com:8443"));
    }
}
