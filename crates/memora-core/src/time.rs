//! Tiny clock abstraction.
//!
//! Snapshot and commit ids include creation timestamps in their digest, so
//! we need a way to pin "now" during tests. Production code just uses
//! [`SystemClock`].

use chrono::Utc;

/// Anything that can produce the current unix timestamp (seconds).
pub trait Clock: Send + Sync {
    /// Return the current time in unix seconds.
    fn now(&self) -> i64;
}

/// Real wall-clock implementation, backed by `chrono::Utc::now`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        Utc::now().timestamp()
    }
}

/// Convenience helper that returns the current unix second timestamp.
pub fn now() -> i64 {
    SystemClock.now()
}
