use std::path::PathBuf;
use thiserror::Error;

/// Errors returned by configuration storage and migration operations.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Filesystem I/O failure associated with a specific path.
    #[error("I/O error at '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// JSON serialization or deserialization failure.
    #[error("JSON serialization/deserialization error: {0}")]
    Json(#[from] serde_json::Error),

    /// Internal mutex was poisoned by a panicking thread.
    #[error("configuration store lock is poisoned")]
    LockPoisoned,
}
