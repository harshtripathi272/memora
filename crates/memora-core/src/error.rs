//! Unified error type for memora-core.
//!
//! Every fallible API in this crate returns [`Result<T>`], which is just
//! `std::result::Result<T, MemoraError>`. The CLI converts these into pretty
//! human-readable messages; library consumers can match on variants.

use std::path::PathBuf;

use thiserror::Error;

/// Convenience alias used throughout memora-core.
pub type Result<T> = std::result::Result<T, MemoraError>;

/// Every error memora-core can produce.
#[derive(Debug, Error)]
pub enum MemoraError {
    /// `memora init` was run but a `.memora/` directory already exists.
    #[error("a memora store already exists at {path}")]
    AlreadyInitialised {
        /// Path of the existing `.memora/` directory.
        path: PathBuf,
    },

    /// Tried to operate on a repository but no `.memora/` was found while
    /// walking up from the working directory.
    #[error("not inside a memora repository (run `memora init` first)")]
    NotARepository,

    /// A reference (branch / HEAD pointer) we expected to exist did not.
    #[error("reference not found: {0}")]
    RefNotFound(String),

    /// A branch name was not valid (empty, contains whitespace, etc).
    #[error("invalid branch name: {0}")]
    InvalidBranchName(String),

    /// User asked to create a branch that already exists.
    #[error("branch already exists: {0}")]
    BranchAlreadyExists(String),

    /// A node id was provided that does not match anything in the store.
    #[error("node not found: {0}")]
    NodeNotFound(String),

    /// A commit id was provided that does not match anything in the store.
    #[error("commit not found: {0}")]
    CommitNotFound(String),

    /// Generic invalid input from a caller — used for things like bad enum
    /// strings coming back out of SQLite.
    #[error("invalid value: {0}")]
    Invalid(String),

    /// Wraps any underlying SQLite error.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// Wraps any underlying I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Wraps a `serde_json` error, used when (de)serialising tag / related
    /// arrays stored as JSON in SQLite.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}
