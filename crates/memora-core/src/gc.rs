//! Garbage collection — importance-scored pruning of the working set.
//!
//! GC is deliberately conservative and reversible. We never delete nodes
//! from `node_versions` (the per-commit snapshots), so every GC'd node
//! can still be resurrected by checking out a historical commit. What
//! we *do* prune is the live `nodes` table — the agent's current working
//! memory.
//!
//! The two-phase model:
//!
//! 1. **Mark phase.** Compute an importance score for every live node.
//!    Anything below the threshold and not already `Deprecated` is
//!    flipped to `Deprecated`. This keeps the node reachable but signals
//!    it as a removal candidate.
//! 2. **Sweep phase.** Anything currently `Deprecated` gets physically
//!    deleted from the live table.
//!
//! Calling GC twice in a row therefore has the desired effect: the first
//! run marks, the second run sweeps. A single run with `--aggressive`
//! does both at once.

use crate::error::Result;
use crate::node::{MemoryNode, MemoryStatus};
use crate::repo::ImportanceWeights;
use crate::store::Store;

/// Per-node decision a GC run came to.
#[derive(Debug, Clone, PartialEq)]
pub enum GcAction {
    /// Already `Deprecated`, scheduled for physical deletion.
    Sweep {
        /// The deprecated node we're about to delete.
        node: MemoryNode,
    },
    /// Score is below the threshold and the node was live; flip to
    /// `Deprecated` and revisit on the next pass.
    Mark {
        /// The node we'd flip.
        node: MemoryNode,
        /// Computed importance score.
        score: f32,
    },
    /// Score is at or above the threshold; keep the node as-is.
    Keep {
        /// The surviving node.
        node: MemoryNode,
        /// Computed importance score.
        score: f32,
    },
}

/// Aggregate result of a GC run.
#[derive(Debug, Clone, Default)]
pub struct GcReport {
    /// Importance threshold used.
    pub threshold: f32,
    /// Whether `--aggressive` was set (mark + sweep in one pass).
    pub aggressive: bool,
    /// Whether `--dry-run` was set (no mutation actually applied).
    pub dry_run: bool,
    /// Decisions, one per live node.
    pub actions: Vec<GcAction>,
}

impl GcReport {
    /// Number of nodes that would be marked `Deprecated`.
    pub fn marked(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a, GcAction::Mark { .. }))
            .count()
    }

    /// Number of nodes that would be physically deleted.
    pub fn swept(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a, GcAction::Sweep { .. }))
            .count()
    }

    /// Number of nodes kept.
    pub fn kept(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a, GcAction::Keep { .. }))
            .count()
    }
}

/// Options controlling [`run_gc`].
#[derive(Debug, Clone, Copy)]
pub struct GcOptions {
    /// Importance threshold in `[0.0, 1.0]`. Nodes scoring below this
    /// get marked `Deprecated`.
    pub threshold: f32,
    /// Importance score weights — same as for export.
    pub weights: ImportanceWeights,
    /// If `true`, sweep marked nodes in the same pass.
    pub aggressive: bool,
    /// If `true`, never mutate the store; just return the report.
    pub dry_run: bool,
}

impl Default for GcOptions {
    fn default() -> Self {
        Self {
            threshold: 0.3,
            weights: ImportanceWeights::default(),
            aggressive: false,
            dry_run: false,
        }
    }
}

/// Run garbage collection against `store`. `now` should be a unix-second
/// timestamp from the caller's [`crate::time::Clock`].
pub fn run_gc(store: &Store, now: i64, opts: GcOptions) -> Result<GcReport> {
    let nodes = store.all_nodes()?;
    let max_age = nodes
        .iter()
        .map(|n| (now - n.last_accessed).max(1))
        .max()
        .unwrap_or(1);
    let max_count = nodes.iter().map(|n| n.access_count).max().unwrap_or(0);

    let mut actions = Vec::with_capacity(nodes.len());
    for n in nodes {
        if n.status == MemoryStatus::Deprecated {
            actions.push(GcAction::Sweep { node: n });
            continue;
        }
        let recency = if max_age > 0 {
            1.0 - ((now - n.last_accessed).max(0) as f32 / max_age as f32)
        } else {
            1.0
        };
        let access = if max_count == 0 {
            0.0
        } else {
            n.access_count as f32 / max_count as f32
        };
        let score = (n.confidence * opts.weights.confidence)
            + (recency * opts.weights.recency)
            + (access * opts.weights.access);
        if score < opts.threshold {
            actions.push(GcAction::Mark { node: n, score });
        } else {
            actions.push(GcAction::Keep { node: n, score });
        }
    }

    let report = GcReport {
        threshold: opts.threshold,
        aggressive: opts.aggressive,
        dry_run: opts.dry_run,
        actions,
    };

    if !opts.dry_run {
        for action in &report.actions {
            match action {
                GcAction::Sweep { node } => {
                    store.delete_node(&node.id)?;
                }
                GcAction::Mark { node, .. } => {
                    store.set_status(&node.id, MemoryStatus::Deprecated, now)?;
                    if opts.aggressive {
                        store.delete_node(&node.id)?;
                    }
                }
                GcAction::Keep { .. } => {}
            }
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{MemoryKind, MemorySource, NewNode};

    fn fresh_store() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::open(tmp.path().join("memora.db")).unwrap();
        (tmp, s)
    }

    #[test]
    fn marks_low_importance_then_sweeps_on_second_pass() {
        let (_tmp, store) = fresh_store();
        let low = MemoryNode::from_new(
            NewNode {
                confidence: Some(0.05),
                ..NewNode::new(
                    MemoryKind::Assumption,
                    "weak",
                    MemorySource::ModelInference,
                )
            },
            100,
        );
        let high = MemoryNode::from_new(
            NewNode::new(MemoryKind::Project, "rust", MemorySource::CodeRead),
            100,
        );
        store.upsert_node(&low).unwrap();
        store.upsert_node(&high).unwrap();

        // First pass: mark low.
        let r1 = run_gc(&store, 200, GcOptions::default()).unwrap();
        assert_eq!(r1.marked(), 1);
        assert_eq!(r1.swept(), 0);
        let after = store.get_node(&low.id).unwrap().unwrap();
        assert_eq!(after.status, MemoryStatus::Deprecated);

        // Second pass: sweep low.
        let r2 = run_gc(&store, 300, GcOptions::default()).unwrap();
        assert_eq!(r2.marked(), 0);
        assert_eq!(r2.swept(), 1);
        assert!(store.get_node(&low.id).unwrap().is_none());
        // high survives.
        assert!(store.get_node(&high.id).unwrap().is_some());
    }

    #[test]
    fn dry_run_does_not_mutate() {
        let (_tmp, store) = fresh_store();
        let n = MemoryNode::from_new(
            NewNode {
                confidence: Some(0.05),
                ..NewNode::new(
                    MemoryKind::Assumption,
                    "weak",
                    MemorySource::ModelInference,
                )
            },
            100,
        );
        store.upsert_node(&n).unwrap();
        let report = run_gc(
            &store,
            200,
            GcOptions {
                dry_run: true,
                ..GcOptions::default()
            },
        )
        .unwrap();
        assert_eq!(report.marked(), 1);
        let after = store.get_node(&n.id).unwrap().unwrap();
        assert_eq!(after.status, MemoryStatus::Ephemeral);
    }

    #[test]
    fn aggressive_marks_and_sweeps_in_one_pass() {
        let (_tmp, store) = fresh_store();
        let n = MemoryNode::from_new(
            NewNode {
                confidence: Some(0.01),
                ..NewNode::new(
                    MemoryKind::Assumption,
                    "weak",
                    MemorySource::ModelInference,
                )
            },
            100,
        );
        store.upsert_node(&n).unwrap();
        let report = run_gc(
            &store,
            200,
            GcOptions {
                aggressive: true,
                ..GcOptions::default()
            },
        )
        .unwrap();
        assert_eq!(report.marked(), 1);
        assert!(store.get_node(&n.id).unwrap().is_none());
    }
}
