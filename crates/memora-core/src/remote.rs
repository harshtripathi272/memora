//! Remote sync — push / pull a branch between two filesystem `.memora/`
//! stores.
//!
//! Phase 5 keeps the wire format trivial: a "remote" is another
//! `.memora/`-bearing project on the filesystem. To `push`, we open the
//! remote's SQLite alongside ours and copy every commit (plus its
//! companion rows in `commit_nodes`, `node_versions`, `merge_parents`)
//! that the remote is missing. The remote's branch ref is then advanced
//! to our tip. `pull` is the symmetric operation.
//!
//! The transport boundary is intentionally narrow: a single function
//! [`copy_commits_between`] does all the row-level copying. Once we add
//! a real network protocol (HTTP, Git smart-protocol, etc.) it will
//! plug in here without changing the public API.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::commit::MemoryCommit;
use crate::error::{MemoraError, Result};
use crate::store::{Refs, Store};
use crate::STORE_DIR;

/// Direction of a sync. Only used for nicer errors / event payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDirection {
    /// Local → remote.
    Push,
    /// Remote → local.
    Pull,
}

impl SyncDirection {
    /// Lowercase wire string (`push` / `pull`).
    pub fn as_str(self) -> &'static str {
        match self {
            SyncDirection::Push => "push",
            SyncDirection::Pull => "pull",
        }
    }
}

/// What happened during a `push` or `pull`.
#[derive(Debug, Clone)]
pub struct SyncOutcome {
    /// Direction.
    pub direction: SyncDirection,
    /// Branch synced.
    pub branch: String,
    /// Number of commits copied across the boundary.
    pub commits_copied: usize,
    /// Tip commit id of the destination branch after the sync.
    pub new_tip: Option<String>,
    /// True if the destination branch was already at our tip (no work).
    pub already_synced: bool,
    /// True if we refused to push because the remote's branch tip is
    /// not an ancestor of our tip (would lose remote commits).
    pub rejected_non_fast_forward: bool,
}

/// Open another `.memora/` directory's `Store` and `Refs` together.
/// `path` may either be the project root (containing `.memora/`) or the
/// `.memora/` directory itself; we accept both for friendliness.
pub fn open_remote(path: impl AsRef<Path>) -> Result<(PathBuf, Store, Refs)> {
    let raw = path.as_ref();
    let memora_dir = if raw.ends_with(STORE_DIR) {
        raw.to_path_buf()
    } else if raw.join(STORE_DIR).is_dir() {
        raw.join(STORE_DIR)
    } else if raw.is_dir() {
        // Maybe the user passed the .memora dir name relative form.
        raw.to_path_buf()
    } else {
        return Err(MemoraError::Invalid(format!(
            "remote does not look like a memora store: {}",
            raw.display()
        )));
    };
    if !memora_dir.is_dir() {
        return Err(MemoraError::Invalid(format!(
            "remote .memora directory not found at {}",
            memora_dir.display()
        )));
    }
    let store = Store::open(memora_dir.join("memora.db"))?;
    let refs = Refs::new(&memora_dir);
    Ok((memora_dir, store, refs))
}

/// Copy the commits reachable from `tip` in `src` that don't yet exist
/// in `dst`, preserving parent ordering. Returns the list of commit ids
/// actually copied (newest first).
pub fn copy_commits_between(src: &Store, dst: &Store, tip: &str) -> Result<Vec<String>> {
    // Topologically order the commits we need to copy: each commit
    // appears after all of its parents in the output list. We do this
    // by walking ancestors with a visit-set, then emitting in
    // dependency order.
    let mut needed = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![tip.to_string()];
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if dst.get_commit(&id)?.is_some() {
            // Already present; no need to descend further.
            continue;
        }
        let commit = src
            .get_commit(&id)?
            .ok_or_else(|| MemoraError::CommitNotFound(id.clone()))?;
        // Push parents so they're processed before we emit this commit.
        for p in src.all_parents(&id)? {
            stack.push(p);
        }
        needed.push(commit);
    }

    // Now emit in topological order: a commit is safe to write once
    // all of its parents are present (or already in `dst`).
    let mut copied = Vec::new();
    let mut written: HashSet<String> = HashSet::new();
    while !needed.is_empty() {
        let mut progress = false;
        let mut still_needed = Vec::new();
        for c in needed.drain(..) {
            let parents = src.all_parents(&c.id)?;
            let parents_ready = parents
                .iter()
                .all(|p| written.contains(p) || dst_has(dst, p).unwrap_or(false));
            if !parents_ready {
                still_needed.push(c);
                continue;
            }
            write_commit_with_companions(src, dst, &c)?;
            written.insert(c.id.clone());
            copied.push(c.id.clone());
            progress = true;
        }
        needed = still_needed;
        if !progress {
            return Err(MemoraError::Invalid(
                "cycle or missing parent detected while copying commits".into(),
            ));
        }
    }
    Ok(copied)
}

fn dst_has(dst: &Store, id: &str) -> Result<bool> {
    Ok(dst.get_commit(id)?.is_some())
}

/// Write a single commit and all of its companion rows to `dst`.
fn write_commit_with_companions(src: &Store, dst: &Store, commit: &MemoryCommit) -> Result<()> {
    dst.insert_commit(commit)?;
    let node_ids = src.commit_node_ids(&commit.id)?;
    if !node_ids.is_empty() {
        dst.insert_commit_nodes(&commit.id, &node_ids)?;
    }
    let versions = src.commit_node_versions(&commit.id)?;
    if !versions.is_empty() {
        dst.insert_node_versions(&commit.id, &versions)?;
    }
    let extra_parents = src.merge_parents(&commit.id)?;
    if !extra_parents.is_empty() {
        dst.insert_merge_parents(&commit.id, &extra_parents)?;
    }
    Ok(())
}

/// Returns true if `ancestor` is reachable from `tip` via the parent DAG.
pub fn is_ancestor(store: &Store, ancestor: &str, tip: &str) -> Result<bool> {
    if ancestor == tip {
        return Ok(true);
    }
    let mut stack = vec![tip.to_string()];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if id == ancestor {
            return Ok(true);
        }
        for p in store.all_parents(&id)? {
            stack.push(p);
        }
    }
    Ok(false)
}
