//! Three-way merge engine.
//!
//! Given two commits `ours` and `theirs`, the engine:
//!
//! 1. Finds their merge base (the most recent common ancestor across the
//!    full parent DAG, including merge commits).
//! 2. Loads the per-node snapshot of all three (`base`, `ours`, `theirs`).
//! 3. Computes a per-node decision using a small precedence ladder:
//!    confidence → source priority → status priority → recency.
//! 4. Returns a [`MergePlan`] the [`Repository`](crate::repo::Repository)
//!    can apply to its working set.
//!
//! Phase 3 v0.1 detects only **same-id** divergence. Two nodes that
//! describe the same fact under different ids will *not* be flagged as
//! conflicting; that needs semantic-overlap detection (Phase 4+).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::error::Result;
use crate::node::{MemoryNode, MemorySource, MemoryStatus};
use crate::store::Store;

/// Strategy for resolving a same-id divergence between `ours` and `theirs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStrategy {
    /// Score the two sides and pick the winner; mark genuine ties as
    /// `Conflicted` and leave them for the user.
    Auto,
    /// On any divergence, keep the `ours` version.
    Ours,
    /// On any divergence, keep the `theirs` version.
    Theirs,
}

/// What the merge plan decided to do with a single node id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeDecision {
    /// The node is unchanged across all three sides; keep it as-is.
    Unchanged,
    /// One side modified the node, the other didn't — take the modified
    /// version. The string is a short reason for tooling / tests.
    TakeOurs(String),
    /// Symmetric to `TakeOurs`.
    TakeTheirs(String),
    /// Both sides modified the node, auto-resolution picked a winner.
    Auto {
        /// `true` if `ours` won, `false` if `theirs` won.
        ours_won: bool,
        /// Short reason such as "higher confidence" or "code_read > model_inference".
        reason: String,
    },
    /// Both sides modified the node, scores tied, the result is marked
    /// `Conflicted` and surfaced to the user.
    Conflicted {
        /// Reason the tie could not be auto-resolved.
        reason: String,
    },
    /// Node existed in `base` and was deleted by both sides, or by one side
    /// while the other left it unchanged.
    Removed,
}

/// One entry in a [`MergePlan`].
#[derive(Debug, Clone)]
pub struct MergeEntry {
    /// Node id this decision applies to.
    pub id: String,
    /// What we decided.
    pub decision: NodeDecision,
    /// The node value to write into the working set. `None` means delete.
    pub resolved: Option<MemoryNode>,
}

/// What the merge engine wants the repository to do.
#[derive(Debug, Clone)]
pub struct MergePlan {
    /// Resolved id of the merge base, or `None` if there is no common
    /// ancestor (two completely unrelated histories — rare but possible).
    pub base: Option<String>,
    /// `ours` commit id.
    pub ours: String,
    /// `theirs` commit id.
    pub theirs: String,
    /// Per-node decisions.
    pub entries: Vec<MergeEntry>,
    /// True if every entry is `Unchanged` — i.e. the merge is a no-op.
    pub identical: bool,
    /// True if `theirs` is reachable from `ours` (already up-to-date).
    pub already_up_to_date: bool,
    /// True if `ours` is reachable from `theirs` (fast-forward possible).
    pub can_fast_forward: bool,
}

impl MergePlan {
    /// All conflict entries — empty when the merge is clean.
    pub fn conflicts(&self) -> Vec<&MergeEntry> {
        self.entries
            .iter()
            .filter(|e| matches!(e.decision, NodeDecision::Conflicted { .. }))
            .collect()
    }

    /// Return only the entries whose resolved value should land in the
    /// working set (i.e. everything except `Unchanged` and `Removed`-with-no-value).
    pub fn writable(&self) -> impl Iterator<Item = &MergeEntry> {
        self.entries
            .iter()
            .filter(|e| !matches!(e.decision, NodeDecision::Unchanged))
    }

    /// True if any entry is `Conflicted`.
    pub fn has_conflicts(&self) -> bool {
        self.entries
            .iter()
            .any(|e| matches!(e.decision, NodeDecision::Conflicted { .. }))
    }
}

/// Plan a merge of `theirs` into `ours`. Pure: does not mutate the store.
pub fn plan_merge(
    store: &Store,
    ours: &str,
    theirs: &str,
    strategy: MergeStrategy,
) -> Result<MergePlan> {
    if ours == theirs {
        return Ok(MergePlan {
            base: Some(ours.to_string()),
            ours: ours.to_string(),
            theirs: theirs.to_string(),
            entries: Vec::new(),
            identical: true,
            already_up_to_date: true,
            can_fast_forward: false,
        });
    }

    // Reachability from ours and theirs (used both for ff detection and
    // for finding the merge base).
    let ours_anc = ancestors(store, ours)?;
    let theirs_anc = ancestors(store, theirs)?;

    let already_up_to_date = ours_anc.contains(theirs);
    let can_fast_forward = theirs_anc.contains(ours);

    let base = merge_base(store, ours, theirs, &ours_anc, &theirs_anc)?;

    if already_up_to_date {
        return Ok(MergePlan {
            base,
            ours: ours.to_string(),
            theirs: theirs.to_string(),
            entries: Vec::new(),
            identical: true,
            already_up_to_date: true,
            can_fast_forward: false,
        });
    }

    // Load the three snapshots (base may be missing for unrelated histories).
    let base_nodes = match base.as_deref() {
        Some(b) => store.commit_node_versions(b)?,
        None => Vec::new(),
    };
    let ours_nodes = store.commit_node_versions(ours)?;
    let theirs_nodes = store.commit_node_versions(theirs)?;

    let base_map: HashMap<String, MemoryNode> =
        base_nodes.into_iter().map(|n| (n.id.clone(), n)).collect();
    let ours_map: HashMap<String, MemoryNode> =
        ours_nodes.into_iter().map(|n| (n.id.clone(), n)).collect();
    let theirs_map: HashMap<String, MemoryNode> =
        theirs_nodes.into_iter().map(|n| (n.id.clone(), n)).collect();

    let mut all_ids: HashSet<&String> = HashSet::new();
    all_ids.extend(base_map.keys());
    all_ids.extend(ours_map.keys());
    all_ids.extend(theirs_map.keys());

    let mut sorted: Vec<&String> = all_ids.into_iter().collect();
    sorted.sort();

    let mut entries = Vec::with_capacity(sorted.len());
    for id in sorted {
        let b = base_map.get(id);
        let o = ours_map.get(id);
        let t = theirs_map.get(id);
        let entry = decide(id, b, o, t, strategy);
        entries.push(entry);
    }

    Ok(MergePlan {
        base,
        ours: ours.to_string(),
        theirs: theirs.to_string(),
        entries,
        identical: false,
        already_up_to_date: false,
        can_fast_forward,
    })
}

/// Per-node decision logic for the three-way merge.
fn decide(
    id: &str,
    base: Option<&MemoryNode>,
    ours: Option<&MemoryNode>,
    theirs: Option<&MemoryNode>,
    strategy: MergeStrategy,
) -> MergeEntry {
    use NodeDecision::*;

    match (base, ours, theirs) {
        // Existed nowhere → impossible to reach; skip safely.
        (None, None, None) => MergeEntry {
            id: id.to_string(),
            decision: Unchanged,
            resolved: None,
        },
        // Same on both sides (whether existing or both deleted) — unchanged.
        (_, None, None) => MergeEntry {
            id: id.to_string(),
            decision: Removed,
            resolved: None,
        },
        // Only ours has it: either a new add on our side, or theirs deleted.
        (b, Some(o), None) => match b {
            None => MergeEntry {
                id: id.to_string(),
                decision: TakeOurs("added by ours".into()),
                resolved: Some(o.clone()),
            },
            Some(bn) if state_eq(bn, o) => MergeEntry {
                id: id.to_string(),
                // theirs deleted, ours unchanged — accept the deletion.
                decision: Removed,
                resolved: None,
            },
            Some(_) => {
                // ours modified, theirs deleted — modify-vs-delete is a conflict.
                conflict_or_strategy(id, ours, theirs, strategy, "modify on ours, delete on theirs")
            }
        },
        // Symmetric to the previous arm.
        (b, None, Some(t)) => match b {
            None => MergeEntry {
                id: id.to_string(),
                decision: TakeTheirs("added by theirs".into()),
                resolved: Some(t.clone()),
            },
            Some(bn) if state_eq(bn, t) => MergeEntry {
                id: id.to_string(),
                decision: Removed,
                resolved: None,
            },
            Some(_) => {
                conflict_or_strategy(id, ours, theirs, strategy, "delete on ours, modify on theirs")
            }
        },
        // In both ours and theirs.
        (b, Some(o), Some(t)) => {
            if state_eq(o, t) {
                return MergeEntry {
                    id: id.to_string(),
                    decision: Unchanged,
                    resolved: Some(o.clone()),
                };
            }
            // Different on the two sides — did each side change?
            let ours_changed = b.map(|bn| !state_eq(bn, o)).unwrap_or(true);
            let theirs_changed = b.map(|bn| !state_eq(bn, t)).unwrap_or(true);
            if ours_changed && !theirs_changed {
                return MergeEntry {
                    id: id.to_string(),
                    decision: TakeOurs("only ours changed".into()),
                    resolved: Some(o.clone()),
                };
            }
            if theirs_changed && !ours_changed {
                return MergeEntry {
                    id: id.to_string(),
                    decision: TakeTheirs("only theirs changed".into()),
                    resolved: Some(t.clone()),
                };
            }
            // Both changed.
            match strategy {
                MergeStrategy::Ours => MergeEntry {
                    id: id.to_string(),
                    decision: TakeOurs("strategy=ours".into()),
                    resolved: Some(o.clone()),
                },
                MergeStrategy::Theirs => MergeEntry {
                    id: id.to_string(),
                    decision: TakeTheirs("strategy=theirs".into()),
                    resolved: Some(t.clone()),
                },
                MergeStrategy::Auto => auto_resolve(id, o, t),
            }
        }
    }
}

fn conflict_or_strategy(
    id: &str,
    ours: Option<&MemoryNode>,
    theirs: Option<&MemoryNode>,
    strategy: MergeStrategy,
    reason: &str,
) -> MergeEntry {
    match strategy {
        MergeStrategy::Ours => MergeEntry {
            id: id.to_string(),
            decision: NodeDecision::TakeOurs(format!("{reason} (strategy=ours)")),
            resolved: ours.cloned(),
        },
        MergeStrategy::Theirs => MergeEntry {
            id: id.to_string(),
            decision: NodeDecision::TakeTheirs(format!("{reason} (strategy=theirs)")),
            resolved: theirs.cloned(),
        },
        MergeStrategy::Auto => MergeEntry {
            id: id.to_string(),
            decision: NodeDecision::Conflicted {
                reason: reason.to_string(),
            },
            // Surface the conflict by promoting the surviving side's body
            // (whichever exists) and flipping its status to Conflicted.
            resolved: ours
                .or(theirs)
                .cloned()
                .map(|mut n| {
                    n.status = MemoryStatus::Conflicted;
                    n
                }),
        },
    }
}

/// Score-based auto-resolution for both-sides-changed.
fn auto_resolve(id: &str, ours: &MemoryNode, theirs: &MemoryNode) -> MergeEntry {
    use std::cmp::Ordering;
    let oc = ours.confidence;
    let tc = theirs.confidence;
    if (oc - tc).abs() > 0.001 {
        return if oc > tc {
            MergeEntry {
                id: id.to_string(),
                decision: NodeDecision::Auto {
                    ours_won: true,
                    reason: format!("higher confidence ({oc:.2} > {tc:.2})"),
                },
                resolved: Some(ours.clone()),
            }
        } else {
            MergeEntry {
                id: id.to_string(),
                decision: NodeDecision::Auto {
                    ours_won: false,
                    reason: format!("higher confidence ({tc:.2} > {oc:.2})"),
                },
                resolved: Some(theirs.clone()),
            }
        };
    }

    let op = source_priority(&ours.source);
    let tp = source_priority(&theirs.source);
    match op.cmp(&tp) {
        Ordering::Greater => {
            return MergeEntry {
                id: id.to_string(),
                decision: NodeDecision::Auto {
                    ours_won: true,
                    reason: format!("source priority ({} > {})", ours.source, theirs.source),
                },
                resolved: Some(ours.clone()),
            };
        }
        Ordering::Less => {
            return MergeEntry {
                id: id.to_string(),
                decision: NodeDecision::Auto {
                    ours_won: false,
                    reason: format!("source priority ({} > {})", theirs.source, ours.source),
                },
                resolved: Some(theirs.clone()),
            };
        }
        Ordering::Equal => {}
    }

    let os = status_priority(ours.status);
    let ts = status_priority(theirs.status);
    match os.cmp(&ts) {
        Ordering::Greater => {
            return MergeEntry {
                id: id.to_string(),
                decision: NodeDecision::Auto {
                    ours_won: true,
                    reason: format!("status priority ({} > {})", ours.status, theirs.status),
                },
                resolved: Some(ours.clone()),
            };
        }
        Ordering::Less => {
            return MergeEntry {
                id: id.to_string(),
                decision: NodeDecision::Auto {
                    ours_won: false,
                    reason: format!("status priority ({} > {})", theirs.status, ours.status),
                },
                resolved: Some(theirs.clone()),
            };
        }
        Ordering::Equal => {}
    }

    if ours.updated_at != theirs.updated_at {
        return if ours.updated_at > theirs.updated_at {
            MergeEntry {
                id: id.to_string(),
                decision: NodeDecision::Auto {
                    ours_won: true,
                    reason: "more recent (ours)".into(),
                },
                resolved: Some(ours.clone()),
            }
        } else {
            MergeEntry {
                id: id.to_string(),
                decision: NodeDecision::Auto {
                    ours_won: false,
                    reason: "more recent (theirs)".into(),
                },
                resolved: Some(theirs.clone()),
            }
        };
    }

    // Genuine tie — surface it as a conflict.
    MergeEntry {
        id: id.to_string(),
        decision: NodeDecision::Conflicted {
            reason: "auto-resolution tied on confidence, source, status and recency".into(),
        },
        resolved: Some({
            let mut n = ours.clone();
            n.status = MemoryStatus::Conflicted;
            n
        }),
    }
}

fn source_priority(source: &MemorySource) -> u8 {
    match source {
        MemorySource::CodeRead => 9,
        MemorySource::TestResult => 8,
        MemorySource::Manual => 7,
        MemorySource::ClaudeCode
        | MemorySource::Cursor
        | MemorySource::Cline
        | MemorySource::OpenHands => 6,
        MemorySource::ModelInference => 4,
        MemorySource::Unknown(_) => 1,
    }
}

fn status_priority(status: MemoryStatus) -> u8 {
    match status {
        MemoryStatus::Stable => 4,
        MemoryStatus::Ephemeral => 3,
        MemoryStatus::Conflicted => 2,
        MemoryStatus::Deprecated => 1,
    }
}

fn state_eq(a: &MemoryNode, b: &MemoryNode) -> bool {
    a.kind == b.kind
        && a.content == b.content
        && (a.confidence - b.confidence).abs() <= 0.001
        && a.status == b.status
        && a.source == b.source
        && a.evidence == b.evidence
}

// ---------------------------------------------------------------------------
// Ancestry / merge-base computation.
// ---------------------------------------------------------------------------

/// Set of all ancestors of `commit_id` (inclusive). Walks both first and
/// extra parents, so merge commits are handled correctly.
fn ancestors(store: &Store, commit_id: &str) -> Result<HashSet<String>> {
    let mut out = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(commit_id.to_string());
    while let Some(id) = queue.pop_front() {
        if !out.insert(id.clone()) {
            continue;
        }
        for p in store.all_parents(&id)? {
            if !out.contains(&p) {
                queue.push_back(p);
            }
        }
    }
    Ok(out)
}

/// Find the *best* merge base — i.e. a common ancestor with no descendants
/// among the common-ancestor set. For most practical cases this is unique;
/// when multiple candidates exist we return the one that is reachable
/// from the most others (a virtual best-of) — sufficient for v0.1.
fn merge_base(
    store: &Store,
    ours: &str,
    theirs: &str,
    ours_anc: &HashSet<String>,
    theirs_anc: &HashSet<String>,
) -> Result<Option<String>> {
    let common: HashSet<String> = ours_anc.intersection(theirs_anc).cloned().collect();
    if common.is_empty() {
        return Ok(None);
    }

    // Walk *down* from ours and theirs and pick the first commit that is
    // in `common` — that is the LCA on a single straight line. For DAGs
    // with criss-cross merges we narrow further by removing any candidate
    // that is an ancestor of another candidate.
    let mut candidates: HashSet<String> = common.clone();
    let candidate_list: Vec<String> = candidates.iter().cloned().collect();
    for cand in candidate_list {
        let anc = ancestors(store, &cand)?;
        for other in &common {
            if other != &cand && anc.contains(other) {
                candidates.remove(other);
            }
        }
    }

    // From the remaining set, pick the one with the highest timestamp
    // (the "newest" common ancestor) for determinism.
    let mut best: Option<(String, i64)> = None;
    for c in candidates {
        let ts = store.get_commit(&c)?.map(|cm| cm.timestamp).unwrap_or(0);
        match &best {
            None => best = Some((c, ts)),
            Some((_, b_ts)) if ts > *b_ts => best = Some((c, ts)),
            _ => {}
        }
    }
    // Suppress unused-variable lints from the closure-unfriendly local code.
    let _ = (ours, theirs);
    Ok(best.map(|(id, _)| id))
}
