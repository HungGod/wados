use serde::{Deserialize, Serialize};

/// What kind of failure an error is. The UI picks its wording and whether to
/// offer a retry from this; `message` is for people.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request was wrong (a bad id, a missing field).
    BadRequest,
    /// Needs a sign-in that isn't there (no GitHub token).
    Unauthorized,
    /// Not allowed.
    Forbidden,
    NotFound,
    /// Already running, already exists, or busy with something else.
    Conflict,
    /// The machine is offline, or a service it needs can't be reached.
    Offline,
    /// A service answered with an error (GitHub, podman).
    Upstream,
    /// Gave up waiting.
    Timeout,
    /// The user stopped it (or let it expire).
    Cancelled,
    /// A bug or something unexpected; the message has details.
    Internal,
}

/// The one error shape: wadd's API returns it as the body of a failed request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    /// A wadspace moving between the screen and its stream: the windows
    /// that didn't close (unsaved work, say), with code Conflict. Asked
    /// again with `force`, they close without saving.
    #[serde(rename = "stillOpen", default, skip_serializing_if = "Vec::is_empty")]
    pub still_open: Vec<String>,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), still_open: vec![] }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::BadRequest, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    /// Windows that didn't close for a move (code Conflict).
    pub fn still_open(message: impl Into<String>, windows: Vec<String>) -> Self {
        Self { code: ErrorCode::Conflict, message: message.into(), still_open: windows }
    }
}

impl From<wad_model::edit::EditError> for ApiError {
    fn from(e: wad_model::edit::EditError) -> Self {
        use wad_model::edit::EditError;
        let code = match &e {
            EditError::NotFound { .. } => ErrorCode::NotFound,
            EditError::Refused(_) => ErrorCode::Conflict,
            EditError::Command(_) | EditError::Check(_) => ErrorCode::BadRequest,
        };
        Self::new(code, e.to_string())
    }
}
