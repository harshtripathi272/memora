//! SQLite-backed persistence layer.
//!
//! The store owns the `.memora/memora.db` connection plus a couple of
//! plain-text files in `.memora/` (HEAD, refs/, config). It does *not*
//! own any high-level workflow logic — that lives in [`crate::repo`].
//!
//! Splitting concerns this way means we can unit-test storage in
//! isolation by pointing it at a temp directory.

mod db;
pub mod refs;

pub use db::{Store, UnstagedSummary};
pub use refs::{HeadRef, Refs};
