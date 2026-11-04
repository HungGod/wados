use thiserror::Error;

#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("invalid url: {0}")]
    InvalidUrl(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("{0}")]
    Other(String),
}

pub type BrowserResult<T> = Result<T, BrowserError>;
