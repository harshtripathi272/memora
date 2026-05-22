//! Typed memory nodes — the atomic unit of memory in memora.
//!
//! Every belief, fact, observation, or preference an agent writes into a
//! memora store is a [`MemoryNode`]. Nodes are content-addressed (the `id`
//! is a SHA-256 of canonical content) and carry a full provenance record:
//! who wrote it, what evidence backs it, and how confident we are.
//!
//! The six [`MemoryKind`] variants — Episodic, Semantic, Procedural,
//! Assumption, Project, Preference — are the project's headline feature.
//! They drive different storage rules, expiry behaviour, and merge
//! semantics. See `docs/MEMORY_TYPES.md`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::MemoraError;
use crate::hash::sha256_hex;

/// The six typed categories memora tracks. Lowercase string forms (used in
/// the CLI, SQLite, and JSON) are the canonical wire representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    /// What happened in a session — conversation turns, tool calls, decisions.
    Episodic,
    /// Stable facts the agent believes about the world or codebase.
    Semantic,
    /// Reusable workflows, skills, learned how-to patterns.
    Procedural,
    /// Unverified beliefs the agent is operating on (lowest trust by default).
    Assumption,
    /// Codebase entities, architecture, conventions, file structure.
    Project,
    /// User or team preferences about style, tooling, communication.
    Preference,
}

impl MemoryKind {
    /// Canonical lowercase string used in SQLite and on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryKind::Episodic => "episodic",
            MemoryKind::Semantic => "semantic",
            MemoryKind::Procedural => "procedural",
            MemoryKind::Assumption => "assumption",
            MemoryKind::Project => "project",
            MemoryKind::Preference => "preference",
        }
    }

    /// All six variants, in the canonical display order.
    pub const ALL: [MemoryKind; 6] = [
        MemoryKind::Episodic,
        MemoryKind::Semantic,
        MemoryKind::Procedural,
        MemoryKind::Assumption,
        MemoryKind::Project,
        MemoryKind::Preference,
    ];
}

impl fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MemoryKind {
    type Err = MemoraError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "episodic" => Ok(MemoryKind::Episodic),
            "semantic" => Ok(MemoryKind::Semantic),
            "procedural" => Ok(MemoryKind::Procedural),
            "assumption" => Ok(MemoryKind::Assumption),
            "project" => Ok(MemoryKind::Project),
            "preference" => Ok(MemoryKind::Preference),
            other => Err(MemoraError::Invalid(format!(
                "unknown memory kind '{other}' (expected one of: episodic, semantic, procedural, assumption, project, preference)"
            ))),
        }
    }
}

/// Lifecycle state of a memory node. The state machine is:
///
/// ```text
///  Ephemeral ─promote─▶ Stable ──gc──▶ Deprecated
///      │                  │
///      └────conflict──────┴──▶ Conflicted
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryStatus {
    /// Just observed — provisional, low trust.
    Ephemeral,
    /// Confirmed, promoted — high trust.
    Stable,
    /// Stale, GC candidate.
    Deprecated,
    /// Contradicts another node, awaiting resolution.
    Conflicted,
}

impl MemoryStatus {
    /// Canonical lowercase string used in SQLite and on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryStatus::Ephemeral => "ephemeral",
            MemoryStatus::Stable => "stable",
            MemoryStatus::Deprecated => "deprecated",
            MemoryStatus::Conflicted => "conflicted",
        }
    }
}

impl fmt::Display for MemoryStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MemoryStatus {
    type Err = MemoraError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ephemeral" => Ok(MemoryStatus::Ephemeral),
            "stable" => Ok(MemoryStatus::Stable),
            "deprecated" => Ok(MemoryStatus::Deprecated),
            "conflicted" => Ok(MemoryStatus::Conflicted),
            other => Err(MemoraError::Invalid(format!(
                "unknown memory status '{other}'"
            ))),
        }
    }
}

/// Where a memory node came from. Distinguishing model guesses from direct
/// code reads is the foundation of memora's trust model.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "name")]
pub enum MemorySource {
    /// Anthropic Claude Code.
    ClaudeCode,
    /// Cursor editor / agent.
    Cursor,
    /// Cline VS Code agent.
    Cline,
    /// All-Hands OpenHands.
    OpenHands,
    /// A direct read of project source code.
    CodeRead,
    /// Output from running a test or test suite.
    TestResult,
    /// A model inference / guess (no external evidence).
    ModelInference,
    /// Manually entered by a human operator.
    Manual,
    /// Anything else, identified by name.
    Unknown(String),
}

impl MemorySource {
    /// Default confidence floor for nodes from this source. Callers may
    /// override on a per-node basis but this is the prior used when no
    /// explicit confidence is provided.
    pub fn default_confidence(&self) -> f32 {
        match self {
            MemorySource::CodeRead => 1.0,
            MemorySource::TestResult => 0.9,
            MemorySource::Manual => 0.8,
            MemorySource::ClaudeCode | MemorySource::Cursor | MemorySource::Cline | MemorySource::OpenHands => 0.7,
            MemorySource::ModelInference => 0.6,
            MemorySource::Unknown(_) => 0.3,
        }
    }

    /// Stable wire representation written to SQLite.
    pub fn as_str(&self) -> String {
        match self {
            MemorySource::ClaudeCode => "claude_code".to_string(),
            MemorySource::Cursor => "cursor".to_string(),
            MemorySource::Cline => "cline".to_string(),
            MemorySource::OpenHands => "openhands".to_string(),
            MemorySource::CodeRead => "code_read".to_string(),
            MemorySource::TestResult => "test_result".to_string(),
            MemorySource::ModelInference => "model_inference".to_string(),
            MemorySource::Manual => "manual".to_string(),
            MemorySource::Unknown(name) => format!("unknown:{name}"),
        }
    }
}

impl fmt::Display for MemorySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

impl FromStr for MemorySource {
    type Err = MemoraError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if let Some(name) = trimmed.strip_prefix("unknown:") {
            return Ok(MemorySource::Unknown(name.to_string()));
        }
        match trimmed.to_ascii_lowercase().as_str() {
            "claude_code" | "claude-code" | "claudecode" => Ok(MemorySource::ClaudeCode),
            "cursor" => Ok(MemorySource::Cursor),
            "cline" => Ok(MemorySource::Cline),
            "openhands" | "open_hands" => Ok(MemorySource::OpenHands),
            "code_read" | "code-read" => Ok(MemorySource::CodeRead),
            "test_result" | "test-result" => Ok(MemorySource::TestResult),
            "model_inference" | "model-inference" => Ok(MemorySource::ModelInference),
            "manual" | "human" => Ok(MemorySource::Manual),
            other => Ok(MemorySource::Unknown(other.to_string())),
        }
    }
}

/// A single typed memory record.
///
/// `id` is derived deterministically from `(kind, content, source,
/// created_at)` so that re-adding the same observation in the same instant
/// from the same source collapses into one node, but two genuine
/// observations stay distinct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryNode {
    /// Content-addressed id. Lowercase hex SHA-256.
    pub id: String,
    /// Which of the six typed categories this node belongs to.
    pub kind: MemoryKind,
    /// Free-form text content of the memory.
    pub content: String,
    /// Confidence in the content, clamped into `[0.0, 1.0]`.
    pub confidence: f32,
    /// Lifecycle status.
    pub status: MemoryStatus,
    /// Where this node came from (provenance).
    pub source: MemorySource,
    /// Optional pointer to evidence (e.g. `src/auth/jwt.rs:L42`).
    pub evidence: Option<String>,
    /// User-assigned tags.
    pub tags: Vec<String>,
    /// Ids of related memory nodes, forming a lightweight graph.
    pub related_to: Vec<String>,
    /// Unix seconds when this node was first created.
    pub created_at: i64,
    /// Unix seconds when this node was last modified.
    pub updated_at: i64,
    /// Unix seconds when this node was last read by `query` etc.
    pub last_accessed: i64,
    /// Read counter — used by importance scoring during GC.
    pub access_count: u32,
    /// Optional unix-second TTL. After this time the node is GC eligible.
    pub expires_at: Option<i64>,
}

/// Builder-style request used to create a new node. The store fills in
/// timestamps, the id, and clamps confidence.
#[derive(Debug, Clone)]
pub struct NewNode {
    /// Memory category.
    pub kind: MemoryKind,
    /// Free-form text content.
    pub content: String,
    /// Optional caller-supplied confidence. If `None`, the source's default
    /// is used.
    pub confidence: Option<f32>,
    /// Initial lifecycle status (defaults to `Ephemeral`).
    pub status: Option<MemoryStatus>,
    /// Provenance / source.
    pub source: MemorySource,
    /// Optional evidence pointer.
    pub evidence: Option<String>,
    /// Tags.
    pub tags: Vec<String>,
    /// Related node ids.
    pub related_to: Vec<String>,
    /// Optional TTL (unix seconds).
    pub expires_at: Option<i64>,
}

impl NewNode {
    /// Construct a minimal new-node request.
    pub fn new(kind: MemoryKind, content: impl Into<String>, source: MemorySource) -> Self {
        Self {
            kind,
            content: content.into(),
            confidence: None,
            status: None,
            source,
            evidence: None,
            tags: Vec::new(),
            related_to: Vec::new(),
            expires_at: None,
        }
    }
}

impl MemoryNode {
    /// Build a fully-formed node from a [`NewNode`] plus a creation timestamp.
    ///
    /// This is the canonical constructor used by the store; the id is
    /// derived from the canonical-form bytes returned by [`Self::digest_input`].
    pub fn from_new(req: NewNode, now: i64) -> Self {
        let confidence = req
            .confidence
            .unwrap_or_else(|| req.source.default_confidence())
            .clamp(0.0, 1.0);
        let status = req.status.unwrap_or(MemoryStatus::Ephemeral);
        let id = Self::digest_input(req.kind, &req.content, &req.source, now);

        MemoryNode {
            id,
            kind: req.kind,
            content: req.content,
            confidence,
            status,
            source: req.source,
            evidence: req.evidence,
            tags: req.tags,
            related_to: req.related_to,
            created_at: now,
            updated_at: now,
            last_accessed: now,
            access_count: 0,
            expires_at: req.expires_at,
        }
    }

    /// Compute the content-address for a (kind, content, source, timestamp)
    /// tuple. Exposed mostly so tests can sanity check ids.
    pub fn digest_input(kind: MemoryKind, content: &str, source: &MemorySource, ts: i64) -> String {
        let canonical = format!(
            "v1\nkind:{}\nsource:{}\nts:{}\ncontent:{}",
            kind.as_str(),
            source.as_str(),
            ts,
            content
        );
        sha256_hex(canonical.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_round_trip() {
        for k in MemoryKind::ALL {
            assert_eq!(MemoryKind::from_str(k.as_str()).unwrap(), k);
        }
    }

    #[test]
    fn unknown_kind_errors() {
        assert!(MemoryKind::from_str("nope").is_err());
    }

    #[test]
    fn source_round_trip_and_unknown() {
        let s = MemorySource::CodeRead;
        assert_eq!(MemorySource::from_str(&s.as_str()).unwrap(), s);
        // Unknowns survive a round-trip.
        let u = MemorySource::Unknown("vimscript".into());
        assert_eq!(MemorySource::from_str(&u.as_str()).unwrap(), u);
        // Anything we don't recognise becomes Unknown(...).
        match MemorySource::from_str("brand-new-tool").unwrap() {
            MemorySource::Unknown(name) => assert_eq!(name, "brand-new-tool"),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn confidence_is_clamped() {
        let node = MemoryNode::from_new(
            NewNode {
                confidence: Some(2.5),
                ..NewNode::new(MemoryKind::Semantic, "x", MemorySource::ModelInference)
            },
            42,
        );
        assert!(node.confidence <= 1.0 && node.confidence >= 0.0);
    }

    #[test]
    fn id_is_deterministic_for_same_input() {
        let req = || NewNode::new(MemoryKind::Project, "uses Rust", MemorySource::CodeRead);
        let a = MemoryNode::from_new(req(), 100);
        let b = MemoryNode::from_new(req(), 100);
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn id_differs_when_timestamp_differs() {
        let req = || NewNode::new(MemoryKind::Project, "uses Rust", MemorySource::CodeRead);
        let a = MemoryNode::from_new(req(), 100);
        let b = MemoryNode::from_new(req(), 101);
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn default_confidence_priors() {
        assert_eq!(MemorySource::CodeRead.default_confidence(), 1.0);
        assert!(MemorySource::ModelInference.default_confidence() < 1.0);
        assert!(MemorySource::Unknown("x".into()).default_confidence() < 0.5);
    }
}
