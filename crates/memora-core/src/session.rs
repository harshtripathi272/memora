//! Session recording — the "flight recorder" for agent memory.
//!
//! A *session* brackets a sequence of operations performed by one tool
//! (Claude Code, Cursor, a script, a human) so that later, `memora
//! replay --session <id>` can walk through what happened in the order it
//! happened: which nodes were added, which beliefs were promoted, which
//! commits landed, which merges resolved which way.
//!
//! Sessions are completely optional. If no session is active when a
//! command runs, no events are recorded; the command behaves exactly as
//! it always has.

use serde::{Deserialize, Serialize};

/// Tool / actor that owns the session. Stored as a string so we can
/// accept new sources without a schema change.
pub type SessionSource = String;

/// One row in the `sessions` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Stable id (UUIDv4). Short forms like the first 8 chars are fine
    /// for display.
    pub id: String,
    /// Tool / actor that started the session, e.g. `claude_code`.
    pub source: SessionSource,
    /// Unix-second timestamp when the session was started.
    pub started_at: i64,
    /// Unix-second timestamp when the session was ended, or `None` if
    /// it's still active.
    pub ended_at: Option<i64>,
    /// Number of events recorded against this session.
    pub event_count: u32,
}

/// Categorical kind of a [`SessionEvent`]. The string form on the wire
/// matches the lowercase variant name (e.g. `node_added`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    /// Session was started.
    SessionStarted,
    /// Session was ended.
    SessionEnded,
    /// A new memory node was added.
    NodeAdded,
    /// An existing node was promoted (e.g. ephemeral → stable).
    NodePromoted,
    /// A commit was created (regular or merge).
    CommitCreated,
    /// A merge completed (with or without conflicts).
    MergeCompleted,
}

impl SessionEventKind {
    /// Lowercase wire string used in SQLite.
    pub fn as_str(self) -> &'static str {
        match self {
            SessionEventKind::SessionStarted => "session_started",
            SessionEventKind::SessionEnded => "session_ended",
            SessionEventKind::NodeAdded => "node_added",
            SessionEventKind::NodePromoted => "node_promoted",
            SessionEventKind::CommitCreated => "commit_created",
            SessionEventKind::MergeCompleted => "merge_completed",
        }
    }

    /// Parse the wire string. Unknown values become `None` so callers can
    /// guard against schema drift.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "session_started" => SessionEventKind::SessionStarted,
            "session_ended" => SessionEventKind::SessionEnded,
            "node_added" => SessionEventKind::NodeAdded,
            "node_promoted" => SessionEventKind::NodePromoted,
            "commit_created" => SessionEventKind::CommitCreated,
            "merge_completed" => SessionEventKind::MergeCompleted,
            _ => return None,
        })
    }
}

/// One row in `session_events`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    /// Auto-increment row id.
    pub id: i64,
    /// Owning session id.
    pub session_id: String,
    /// Unix-second timestamp.
    pub timestamp: i64,
    /// Categorical kind.
    pub kind: SessionEventKind,
    /// Free-form JSON payload. The shape depends on `kind`; see
    /// [`crate::repo::Repository::record_event`] for the canonical
    /// fields per kind.
    pub data: serde_json::Value,
}
