//! Commit + tree primitives.
//!
//! A *commit* is a labelled snapshot of the set of node ids that exist in
//! the repository at one moment. Commits form a parent-pointed DAG, just
//! like git, but the objects they reference are typed memory nodes rather
//! than file blobs.
//!
//! For phase 1 we keep the model deliberately simple: a tree is just the
//! sorted set of node ids that were live at the time of the commit. The
//! tree id is the SHA-256 of those ids joined by newlines. This is enough
//! to detect "did anything change since the last commit" without writing
//! a separate object database yet — we can layer that in once the diff
//! engine arrives.

use serde::{Deserialize, Serialize};

use crate::hash::sha256_hex;
use crate::node::MemoryNode;

/// Aggregated counts for the changes a commit introduces, used by
/// `memora commit` and `memora log` for friendly output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitStats {
    /// Number of new nodes introduced.
    pub added: u32,
    /// Number of nodes whose `updated_at` advanced.
    pub modified: u32,
    /// Number of nodes that were dropped between parent and this commit.
    pub removed: u32,
    /// Number of ephemeral → stable transitions in this commit.
    pub promoted: u32,
    /// Number of nodes flipped to `Conflicted` in this commit.
    pub conflicted: u32,
}

/// A snapshot of the repository state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryCommit {
    /// Lowercase hex SHA-256 commit id.
    pub id: String,
    /// Parent commit id, or `None` for the root commit.
    pub parent: Option<String>,
    /// Human-readable commit message.
    pub message: String,
    /// Tool / human that created the commit (e.g. `claude_code`, `human`).
    pub author: String,
    /// Unix-second timestamp.
    pub timestamp: i64,
    /// Tree id — SHA-256 over the sorted node ids in this snapshot.
    pub tree_id: String,
    /// Per-commit stats.
    pub stats: CommitStats,
}

/// Compute the tree id for a slice of node ids. Order independent.
pub fn tree_id_for(node_ids: &[String]) -> String {
    let mut sorted: Vec<&String> = node_ids.iter().collect();
    sorted.sort();
    let canonical = sorted
        .into_iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    sha256_hex(canonical.as_bytes())
}

/// Compute a tree id over the *full state* of a set of nodes, not just
/// their ids. This means a status flip (e.g. `ephemeral` → `stable`) on
/// the same node id produces a different tree, which is what we want for
/// `memora commit` to detect "something actually changed".
///
/// Each node contributes the line `<id>\t<state-digest>`, where the state
/// digest hashes the fields that participate in equality:
/// `kind | status | confidence (3dp) | content | source | evidence`.
pub fn tree_id_for_nodes(nodes: &[MemoryNode]) -> String {
    let mut entries: Vec<(String, String)> = nodes
        .iter()
        .map(|n| (n.id.clone(), node_state_digest(n)))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical: String = entries
        .into_iter()
        .map(|(id, st)| format!("{id}\t{st}"))
        .collect::<Vec<_>>()
        .join("\n");
    sha256_hex(canonical.as_bytes())
}

/// SHA-256 of the comparable fields of a node. Used inside `tree_id_for_nodes`.
fn node_state_digest(n: &MemoryNode) -> String {
    let canonical = format!(
        "kind:{}\nstatus:{}\nconf:{:.3}\ncontent:{}\nsource:{}\nevidence:{}",
        n.kind.as_str(),
        n.status.as_str(),
        n.confidence,
        n.content,
        n.source.as_str(),
        n.evidence.as_deref().unwrap_or(""),
    );
    sha256_hex(canonical.as_bytes())
}

/// Compute a commit id from its component fields. Pure function — exposed
/// so tests can build commits without going through the store.
pub fn commit_id(
    parent: Option<&str>,
    tree_id: &str,
    author: &str,
    message: &str,
    timestamp: i64,
) -> String {
    let canonical = format!(
        "v1\nparent:{}\ntree:{}\nauthor:{}\nts:{}\nmsg:{}",
        parent.unwrap_or(""),
        tree_id,
        author,
        timestamp,
        message,
    );
    sha256_hex(canonical.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_id_is_order_independent() {
        let a = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let b = vec!["c".to_string(), "a".to_string(), "b".to_string()];
        assert_eq!(tree_id_for(&a), tree_id_for(&b));
    }

    #[test]
    fn empty_tree_has_known_id() {
        // Stable across releases — golden value pinned for safety.
        assert_eq!(
            tree_id_for(&[]),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn commit_id_changes_with_parent() {
        let t = tree_id_for(&["a".to_string()]);
        let a = commit_id(None, &t, "human", "init", 0);
        let b = commit_id(Some("deadbeef"), &t, "human", "init", 0);
        assert_ne!(a, b);
    }

    #[test]
    fn tree_id_for_nodes_reflects_status_changes() {
        use crate::node::{MemoryKind, MemorySource, NewNode};
        let mut node = MemoryNode::from_new(
            NewNode::new(MemoryKind::Assumption, "x", MemorySource::ModelInference),
            42,
        );
        let before = tree_id_for_nodes(std::slice::from_ref(&node));
        node.status = crate::node::MemoryStatus::Stable;
        let after = tree_id_for_nodes(std::slice::from_ref(&node));
        assert_ne!(before, after, "status change must alter tree id");
    }
}
