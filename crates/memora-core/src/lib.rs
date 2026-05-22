//! # memora-core
//!
//! Core library for [memora](https://github.com/memora-dev/memora) — the
//! memory layer for AI agents. Provides the typed memory model, content
//! addressed storage, snapshot / commit primitives, and the SQLite-backed
//! store that the `memora` CLI is built on.
//!
//! This crate is intentionally CLI agnostic. Every public type is `Serialize`
//! / `Deserialize` so it can be embedded into other tools, SDKs, or services
//! without pulling in any user-interface code.
//!
//! ## Layout
//!
//! - [`error`]    — the unified [`MemoraError`](error::MemoraError) and
//!   [`Result`](error::Result) types.
//! - [`hash`]     — small helpers around SHA-256 content addressing.
//! - [`time`]     — clock abstraction so tests can pin timestamps.
//! - [`node`]     — [`MemoryNode`](node::MemoryNode) and its companion
//!   enums describing typed memory.
//! - [`commit`]   — commit / tree primitives that snapshot a set of nodes.
//! - [`store`]    — SQLite schema and low level persistence.
//! - [`repo`]     — high level [`Repository`](repo::Repository) facade
//!   that the CLI talks to.
//!
//! See `SPEC.md` at the repo root for the on-disk format.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]

pub mod commit;
pub mod error;
pub mod hash;
pub mod node;
pub mod repo;
pub mod store;
pub mod time;

pub use commit::{CommitStats, MemoryCommit};
pub use error::{MemoraError, Result};
pub use node::{MemoryKind, MemoryNode, MemorySource, MemoryStatus};
pub use repo::Repository;

/// On-disk format version written into `.memora/config`. Bumped whenever the
/// schema or directory layout changes in a non backwards-compatible way.
pub const FORMAT_VERSION: u32 = 1;

/// Name of the directory memora uses inside a project, analogous to `.git/`.
pub const STORE_DIR: &str = ".memora";

/// Default branch name created by `memora init`.
pub const DEFAULT_BRANCH: &str = "main";
