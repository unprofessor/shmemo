//! Error types for the memo application

use std::io;
use thiserror::Error;

/// The main error type for memo operations
#[derive(Debug, Error)]
pub enum MemoError {
    /// An I/O error occurred
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Failed to compute digest
    #[error("Failed to compute digest: {0}")]
    Digest(String),

    /// JSON serialization/deserialization error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Home directory not found
    #[error("Cannot find home directory")]
    HomeNotFound,

    /// Invalid command
    #[error("Invalid command: {0}")]
    InvalidCommand(String),

    /// Invalid TTL format
    #[error("Invalid TTL: {0}")]
    InvalidTtl(String),
}

/// Result type alias for memo operations
pub type Result<T> = std::result::Result<T, MemoError>;

impl From<MemoError> for io::Error {
    fn from(err: MemoError) -> Self {
        match err {
            MemoError::Io(io_err) => io_err,
            other => io::Error::other(other.to_string()),
        }
    }
}
