//! High level repository facade.
//!
//! [`Repository`] is the only thing the CLI talks to. It hides the details
//! of the on-disk layout (refs, HEAD, SQLite) behind a small set of
//! intent-revealing methods: `init`, `add_node`, `commit`, `log`, `status`,
//! `branch`, `switch`, `rollback`.

use std::fs;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::commit::{commit_id_with_parents, tree_id_for_nodes, CommitStats, MemoryCommit};
use crate::error::{MemoraError, Result};
use crate::export::{render as render_export, ExportFormat};
use crate::merge::{plan_merge, MergePlan, MergeStrategy, NodeDecision};
use crate::node::{MemoryKind, MemoryNode, MemoryStatus, NewNode};
use crate::session::{Session, SessionEvent, SessionEventKind};
use crate::store::{HeadRef, Refs, Store, UnstagedSummary};
use crate::time::{Clock, SystemClock};
use crate::{DEFAULT_BRANCH, FORMAT_VERSION, STORE_DIR};

/// A fully constructed repository, anchored at `<workdir>/.memora/`.
pub struct Repository {
    /// Working directory (the project root).
    workdir: PathBuf,
    /// Path to the `.memora/` directory.
    memora_dir: PathBuf,
    refs: Refs,
    store: Store,
    clock: Box<dyn Clock>,
}

/// Result of a successful `commit`. Returned to callers so they can render
/// human-friendly output.
#[derive(Debug, Clone)]
pub struct CommitOutcome {
    /// The commit just created. `None` means there was nothing to commit.
    pub commit: Option<MemoryCommit>,
    /// Branch the commit landed on (`None` if HEAD was detached).
    pub branch: Option<String>,
}

impl Repository {
    // --- discovery -------------------------------------------------------

    /// Walk up from `start` looking for a `.memora/` directory. Returns the
    /// project root that contains it.
    pub fn discover(start: impl AsRef<Path>) -> Result<PathBuf> {
        let start = start.as_ref();
        let abs = if start.is_absolute() {
            start.to_path_buf()
        } else {
            std::env::current_dir()?.join(start)
        };
        let mut current: Option<&Path> = Some(&abs);
        while let Some(dir) = current {
            if dir.join(STORE_DIR).is_dir() {
                return Ok(dir.to_path_buf());
            }
            current = dir.parent();
        }
        Err(MemoraError::NotARepository)
    }

    /// Open the repository whose `.memora/` is at `<workdir>/.memora/`.
    pub fn open(workdir: impl AsRef<Path>) -> Result<Self> {
        let workdir = workdir.as_ref().to_path_buf();
        let memora_dir = workdir.join(STORE_DIR);
        if !memora_dir.is_dir() {
            return Err(MemoraError::NotARepository);
        }
        let refs = Refs::new(&memora_dir);
        let store = Store::open(memora_dir.join("memora.db"))?;
        Ok(Self {
            workdir,
            memora_dir,
            refs,
            store,
            clock: Box::new(SystemClock),
        })
    }

    /// Like [`Self::open`] but walks up looking for an existing repo.
    pub fn open_from(start: impl AsRef<Path>) -> Result<Self> {
        let root = Self::discover(start)?;
        Self::open(root)
    }

    /// Initialise a fresh repository at `<workdir>/.memora/`. Errors if
    /// one already exists.
    pub fn init(workdir: impl AsRef<Path>) -> Result<Self> {
        let workdir = workdir.as_ref().to_path_buf();
        let memora_dir = workdir.join(STORE_DIR);
        if memora_dir.exists() {
            return Err(MemoraError::AlreadyInitialised { path: memora_dir });
        }
        fs::create_dir_all(&memora_dir)?;
        let refs = Refs::new(&memora_dir);
        refs.init(DEFAULT_BRANCH)?;

        // Write a friendly config file. Pure TOML, hand-written so we
        // don't pull a serialiser in just for this.
        let config = format!(
            "# memora config (format v{FORMAT_VERSION})\n\
             [core]\n\
             format_version = {FORMAT_VERSION}\n\
             default_branch = \"{DEFAULT_BRANCH}\"\n\
             \n\
             [author]\n\
             name = \"human\"\n"
        );
        fs::write(memora_dir.join("config"), config)?;

        let store = Store::open(memora_dir.join("memora.db"))?;
        Ok(Self {
            workdir,
            memora_dir,
            refs,
            store,
            clock: Box::new(SystemClock),
        })
    }

    /// Inject a custom clock — used by tests so timestamps are reproducible.
    pub fn with_clock(mut self, clock: Box<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    // --- accessors -------------------------------------------------------

    /// Working directory containing this repository.
    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// The `.memora/` directory itself.
    pub fn memora_dir(&self) -> &Path {
        &self.memora_dir
    }

    /// Borrow the underlying [`Store`].
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Borrow the [`Refs`] manager.
    pub fn refs(&self) -> &Refs {
        &self.refs
    }

    /// Read the parsed HEAD ref.
    pub fn head(&self) -> Result<HeadRef> {
        self.refs.read_head()
    }

    /// Resolve HEAD to a concrete commit id, if any.
    pub fn head_commit_id(&self) -> Result<Option<String>> {
        match self.refs.read_head()? {
            HeadRef::Branch(name) => self.refs.read_branch(&name),
            HeadRef::Detached(id) => Ok(Some(id)),
        }
    }

    // --- node operations -------------------------------------------------

    /// Insert a new typed memory node and return it.
    pub fn add_node(&self, req: NewNode) -> Result<MemoryNode> {
        let now = self.clock.now();
        let node = MemoryNode::from_new(req, now);
        self.store.upsert_node(&node)?;
        self.record_event(
            SessionEventKind::NodeAdded,
            serde_json::json!({
                "node_id": node.id,
                "kind": node.kind.as_str(),
                "status": node.status.as_str(),
                "source": node.source.as_str(),
                "confidence": node.confidence,
                "content": node.content,
            }),
        )?;
        Ok(node)
    }

    // --- commit / status / log -------------------------------------------

    /// Build an unstaged summary against the current HEAD.
    pub fn status(&self) -> Result<UnstagedSummary> {
        let head_commit = self.head_commit_id()?;
        self.store.unstaged_against(head_commit.as_deref())
    }

    /// Create a commit on the current branch with the given message and
    /// author. If there are no changes since HEAD, returns
    /// [`CommitOutcome`] with `commit: None`.
    pub fn commit(&self, message: &str, author: &str) -> Result<CommitOutcome> {
        self.commit_with_parents(message, author, &[])
    }

    /// Same as [`Self::commit`] but with explicit *additional* parent commit
    /// ids. The first parent always comes from HEAD. Used by `merge`.
    pub fn commit_with_parents(
        &self,
        message: &str,
        author: &str,
        extra_parents: &[String],
    ) -> Result<CommitOutcome> {
        let head_ref = self.refs.read_head()?;
        let parent = self.head_commit_id()?;

        // Pull the parent's full per-node snapshot so we can compute proper
        // promotion / modification / removal stats.
        let parent_versions: std::collections::HashMap<String, MemoryNode> = match parent.as_deref()
        {
            Some(p) => self
                .store
                .commit_node_versions(p)?
                .into_iter()
                .map(|n| (n.id.clone(), n))
                .collect(),
            None => std::collections::HashMap::new(),
        };

        let nodes = self.store.all_nodes()?;
        let tree = tree_id_for_nodes(&nodes);

        // Detect "nothing to commit": same tree id as parent, *and* no
        // additional parents (a merge commit always wants to be recorded
        // even if the tree happened to match — e.g. for already-up-to-date
        // we never reach here because the merge code short-circuits earlier).
        let parent_tree = match parent.as_deref() {
            Some(p) => self
                .store
                .get_commit(p)?
                .map(|c| c.tree_id)
                .unwrap_or_default(),
            None => tree_id_for_nodes(&[]),
        };
        if extra_parents.is_empty() && parent.is_some() && tree == parent_tree {
            return Ok(CommitOutcome {
                commit: None,
                branch: head_ref.branch().map(str::to_string),
            });
        }

        let mut stats = CommitStats::default();
        let mut current_ids = std::collections::HashSet::new();
        for node in &nodes {
            current_ids.insert(node.id.clone());
            match parent_versions.get(&node.id) {
                None => stats.added += 1,
                Some(prev) => {
                    if state_differs(prev, node) {
                        stats.modified += 1;
                    }
                    if prev.status != MemoryStatus::Stable && node.status == MemoryStatus::Stable {
                        stats.promoted += 1;
                    }
                    if prev.status != MemoryStatus::Conflicted
                        && node.status == MemoryStatus::Conflicted
                    {
                        stats.conflicted += 1;
                    }
                }
            }
        }
        for id in parent_versions.keys() {
            if !current_ids.contains(id) {
                stats.removed += 1;
            }
        }

        let now = self.clock.now();
        let id = commit_id_with_parents(parent.as_deref(), extra_parents, &tree, author, message, now);
        let commit = MemoryCommit {
            id: id.clone(),
            parent: parent.clone(),
            message: message.to_string(),
            author: author.to_string(),
            timestamp: now,
            tree_id: tree,
            stats,
        };
        self.store.insert_commit(&commit)?;
        let mut node_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
        node_ids.sort();
        self.store.insert_commit_nodes(&id, &node_ids)?;
        self.store.insert_node_versions(&id, &nodes)?;
        if !extra_parents.is_empty() {
            self.store.insert_merge_parents(&id, extra_parents)?;
        }

        let branch_name = head_ref.branch().map(str::to_string);
        match &head_ref {
            HeadRef::Branch(name) => {
                if !self.refs.branch_path(name).exists() {
                    self.refs.create_branch(name, Some(&id))?;
                } else {
                    self.refs.write_branch(name, &id)?;
                }
            }
            HeadRef::Detached(_) => {
                self.refs.write_head_detached(&id)?;
            }
        }

        let outcome = CommitOutcome {
            commit: Some(commit.clone()),
            branch: branch_name,
        };
        self.record_event(
            SessionEventKind::CommitCreated,
            serde_json::json!({
                "commit_id": commit.id,
                "parent": commit.parent,
                "extra_parents": extra_parents,
                "branch": outcome.branch,
                "message": commit.message,
                "tree_id": commit.tree_id,
                "stats": commit.stats,
            }),
        )?;
        Ok(outcome)
    }

    /// Walk the commit history starting from HEAD.
    pub fn log(&self, limit: Option<usize>) -> Result<Vec<MemoryCommit>> {
        match self.head_commit_id()? {
            Some(head) => self.store.walk_commits(&head, limit),
            None => Ok(Vec::new()),
        }
    }

    // --- branching -------------------------------------------------------

    /// Create a new branch pointing at the current HEAD commit.
    pub fn create_branch(&self, name: &str) -> Result<()> {
        let head_commit = self.head_commit_id()?;
        self.refs.create_branch(name, head_commit.as_deref())
    }

    /// List branches.
    pub fn list_branches(&self) -> Result<Vec<String>> {
        self.refs.list_branches()
    }

    /// Switch HEAD to the given branch. The branch must already exist.
    /// The working set is rewritten to match the target branch's tip.
    /// Refuses to switch if there are uncommitted changes; commit them or
    /// stash them by branching first (`memora branch foo`).
    pub fn switch_branch(&self, name: &str) -> Result<()> {
        if !self.refs.branch_path(name).exists() {
            return Err(MemoraError::RefNotFound(name.to_string()));
        }

        // Refuse if the working set has uncommitted changes.
        let summary = self.status()?;
        if !summary.added.is_empty()
            || !summary.modified.is_empty()
            || !summary.removed.is_empty()
        {
            return Err(MemoraError::Invalid(format!(
                "uncommitted changes in working set ({} added, {} modified, {} removed) — commit or branch first",
                summary.added.len(),
                summary.modified.len(),
                summary.removed.len(),
            )));
        }

        // Move HEAD then rewrite the working set from the target's node_versions.
        self.refs.write_head_branch(name)?;
        let target_commit = self.refs.read_branch(name)?;
        let target_nodes = match target_commit.as_deref() {
            Some(c) => self.store.commit_node_versions(c)?,
            None => Vec::new(),
        };
        self.replace_working_set(&target_nodes)?;
        Ok(())
    }

    /// Reset HEAD to a specific commit id, leaving the working set as it
    /// is. We auto-create a `pre-rollback/<short>` checkpoint commit
    /// **before** moving HEAD, so the previous tip is never silently lost.
    ///
    /// This is intentionally conservative for v0.1; a future version will
    /// also reconstruct node tables from the target commit's tree.
    pub fn rollback_to(&self, target_commit: &str, author: &str) -> Result<MemoryCommit> {
        // Validate that target exists.
        let target = self
            .store
            .get_commit(target_commit)?
            .ok_or_else(|| MemoraError::CommitNotFound(target_commit.to_string()))?;

        // Take a checkpoint of the current state first so the user can
        // always undo the rollback.
        let _ = self.commit(
            &format!(
                "pre-rollback checkpoint (rolling to {})",
                &target.id[..7.min(target.id.len())]
            ),
            author,
        )?;

        // Move HEAD. If we're on a branch, point the branch at the target;
        // otherwise update detached HEAD.
        match self.refs.read_head()? {
            HeadRef::Branch(name) => self.refs.write_branch(&name, &target.id)?,
            HeadRef::Detached(_) => self.refs.write_head_detached(&target.id)?,
        }
        Ok(target)
    }

    // --- merge -----------------------------------------------------------

    /// Plan a merge of `their_rev` into the current HEAD. Pure: does not
    /// touch the working set. Useful for `--dry-run` style flows.
    pub fn plan_merge(
        &self,
        their_rev: &str,
        strategy: MergeStrategy,
    ) -> Result<MergePlan> {
        let ours = self
            .head_commit_id()?
            .ok_or_else(|| MemoraError::CommitNotFound("HEAD".into()))?;
        let theirs = self.resolve_revision(their_rev)?;
        plan_merge(&self.store, &ours, &theirs, strategy)
    }

    /// Merge `their_rev` into the current HEAD. The behaviour is:
    ///
    /// - **already up-to-date**: no-op, returns the existing HEAD.
    /// - **fast-forward**: if `--ff` is allowed, just move HEAD's branch.
    /// - **true merge**: rewrite the working set from the merge plan and
    ///   create a merge commit (unless `commit == false`).
    pub fn merge(
        &self,
        their_rev: &str,
        opts: MergeOptions,
    ) -> Result<MergeOutcome> {
        let ours = self
            .head_commit_id()?
            .ok_or_else(|| MemoraError::CommitNotFound("HEAD".into()))?;
        let theirs = self.resolve_revision(their_rev)?;
        let plan = plan_merge(&self.store, &ours, &theirs, opts.strategy)?;

        if plan.already_up_to_date {
            return Ok(MergeOutcome {
                kind: MergeKind::AlreadyUpToDate,
                plan,
                commit: None,
            });
        }

        if plan.can_fast_forward && opts.allow_fast_forward {
            // Fast-forward: just point our branch at theirs and overwrite
            // the working set with theirs's snapshot.
            let their_nodes = self.store.commit_node_versions(&theirs)?;
            self.replace_working_set(&their_nodes)?;
            match self.refs.read_head()? {
                HeadRef::Branch(name) => self.refs.write_branch(&name, &theirs)?,
                HeadRef::Detached(_) => self.refs.write_head_detached(&theirs)?,
            }
            let commit = self.store.get_commit(&theirs)?;
            return Ok(MergeOutcome {
                kind: MergeKind::FastForward,
                plan,
                commit,
            });
        }

        // True merge: apply the plan to the working set.
        self.apply_plan_to_working_set(&plan)?;

        if !opts.commit {
            return Ok(MergeOutcome {
                kind: MergeKind::NoCommit,
                plan,
                commit: None,
            });
        }

        let message = opts.message.clone().unwrap_or_else(|| {
            format!(
                "Merge {} into {}",
                short_for_display(&theirs),
                self.refs
                    .read_head()
                    .ok()
                    .as_ref()
                    .and_then(|h| h.branch().map(str::to_string))
                    .unwrap_or_else(|| "HEAD".into()),
            )
        });
        let outcome = self.commit_with_parents(&message, &opts.author, &[theirs.clone()])?;
        let kind = if plan.has_conflicts() {
            MergeKind::Conflicts
        } else {
            MergeKind::Merged
        };
        let result = MergeOutcome {
            kind,
            plan,
            commit: outcome.commit,
        };
        self.record_event(
            SessionEventKind::MergeCompleted,
            serde_json::json!({
                "ours": result.plan.ours,
                "theirs": result.plan.theirs,
                "base": result.plan.base,
                "kind": match result.kind {
                    MergeKind::AlreadyUpToDate => "already_up_to_date",
                    MergeKind::FastForward => "fast_forward",
                    MergeKind::Merged => "merged",
                    MergeKind::Conflicts => "conflicts",
                    MergeKind::NoCommit => "no_commit",
                },
                "commit_id": result.commit.as_ref().map(|c| c.id.clone()),
                "conflicts": result.plan.conflicts().len(),
            }),
        )?;
        Ok(result)
    }

    /// Replace the live `nodes` table with the contents of `target`.
    /// Used by fast-forward merge.
    fn replace_working_set(&self, target: &[MemoryNode]) -> Result<()> {
        let current = self.store.all_nodes()?;
        let target_ids: std::collections::HashSet<&str> =
            target.iter().map(|n| n.id.as_str()).collect();
        for n in &current {
            if !target_ids.contains(n.id.as_str()) {
                self.store.delete_node(&n.id)?;
            }
        }
        for n in target {
            self.store.upsert_node(n)?;
        }
        Ok(())
    }

    /// Apply a [`MergePlan`] to the working set in place.
    fn apply_plan_to_working_set(&self, plan: &MergePlan) -> Result<()> {
        for entry in &plan.entries {
            match &entry.decision {
                NodeDecision::Unchanged => {}
                NodeDecision::Removed => {
                    self.store.delete_node(&entry.id)?;
                }
                _ => {
                    if let Some(node) = &entry.resolved {
                        self.store.upsert_node(node)?;
                    } else {
                        self.store.delete_node(&entry.id)?;
                    }
                }
            }
        }
        Ok(())
    }

    // --- promotion -------------------------------------------------------

    /// Promote one or more `ephemeral` nodes to `stable`. Returns the
    /// list of node ids that were actually promoted. Idempotent: nodes
    /// that are already `stable` are skipped without error.
    pub fn promote(&self, plan: PromotePlan) -> Result<Vec<String>> {
        let candidate_ids: Vec<String> = match plan {
            PromotePlan::Ids(ids) => {
                let mut out = Vec::with_capacity(ids.len());
                for id in ids {
                    let node = self
                        .store
                        .get_node(&id)?
                        .ok_or_else(|| MemoraError::NodeNotFound(id.clone()))?;
                    if node.status == MemoryStatus::Ephemeral {
                        out.push(id);
                    }
                }
                out
            }
            PromotePlan::Kind(kind) => self.store.find_promotion_candidates(Some(kind), None)?,
            PromotePlan::AllConfirmed { min_confidence } => self
                .store
                .find_promotion_candidates(None, Some(min_confidence.clamp(0.0, 1.0)))?,
        };
        let now = self.clock.now();
        for id in &candidate_ids {
            self.store.set_status(id, MemoryStatus::Stable, now)?;
        }
        if !candidate_ids.is_empty() {
            self.record_event(
                SessionEventKind::NodePromoted,
                serde_json::json!({
                    "node_ids": candidate_ids,
                    "from": "ephemeral",
                    "to": "stable",
                }),
            )?;
        }
        Ok(candidate_ids)
    }

    // --- diff ------------------------------------------------------------

    /// Compute a [`DiffReport`] between two commits (or a commit and the
    /// working set). `from` and `to` are revspecs supported by
    /// [`Self::resolve_revision`]. `to == None` means the working set.
    pub fn diff(&self, from: &str, to: Option<&str>) -> Result<DiffReport> {
        let from_id = self.resolve_revision(from)?;
        let from_nodes: Vec<MemoryNode> = self.store.commit_node_versions(&from_id)?;
        let to_label: String;
        let to_nodes: Vec<MemoryNode> = match to {
            Some(rev) => {
                let to_id = self.resolve_revision(rev)?;
                to_label = to_id.clone();
                self.store.commit_node_versions(&to_id)?
            }
            None => {
                to_label = "(working set)".to_string();
                self.store.all_nodes()?
            }
        };
        Ok(DiffReport::compute(from_id, to_label, &from_nodes, &to_nodes))
    }

    // --- revision parsing ------------------------------------------------

    /// Resolve a revision spec to a full commit id. Supported forms:
    /// - full or short commit id (>=4 hex chars)
    /// - branch name
    /// - `HEAD`, `HEAD~`, `HEAD~N`
    pub fn resolve_revision(&self, rev: &str) -> Result<String> {
        let rev = rev.trim();
        if rev.is_empty() {
            return Err(MemoraError::Invalid("empty revision".into()));
        }

        if rev == "HEAD" {
            return self
                .head_commit_id()?
                .ok_or_else(|| MemoraError::CommitNotFound("HEAD".into()));
        }
        if let Some(rest) = rev.strip_prefix("HEAD") {
            if let Some(n) = parse_tilde(rest) {
                let head = self
                    .head_commit_id()?
                    .ok_or_else(|| MemoraError::CommitNotFound("HEAD".into()))?;
                return self.nth_ancestor(&head, n);
            }
        }

        if rev.chars().all(|c| c.is_ascii_hexdigit()) && rev.len() >= 4 {
            if let Ok(id) = self.store.resolve_commit_prefix(rev) {
                return Ok(id);
            }
        }

        if self.refs.branch_path(rev).exists() {
            return self
                .refs
                .read_branch(rev)?
                .ok_or_else(|| MemoraError::CommitNotFound(rev.to_string()));
        }

        Err(MemoraError::CommitNotFound(rev.to_string()))
    }

    fn nth_ancestor(&self, commit_id: &str, n: usize) -> Result<String> {
        let mut current = commit_id.to_string();
        for step in 0..n {
            let c = self
                .store
                .get_commit(&current)?
                .ok_or_else(|| MemoraError::CommitNotFound(current.clone()))?;
            match c.parent {
                Some(p) => current = p,
                None => {
                    return Err(MemoraError::Invalid(format!(
                        "revision walks past root commit (only {step} ancestors available)"
                    )));
                }
            }
        }
        Ok(current)
    }

    // --- session bracketing ---------------------------------------------

    /// Path to the marker file recording the active session id, if any.
    fn current_session_path(&self) -> PathBuf {
        self.memora_dir.join("sessions").join("CURRENT")
    }

    /// Read the active session id (the contents of `.memora/sessions/CURRENT`).
    pub fn current_session_id(&self) -> Result<Option<String>> {
        let path = self.current_session_path();
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path)?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            Ok(None)
        } else {
            Ok(Some(trimmed.to_string()))
        }
    }

    /// Start a fresh recording session. Sets the `CURRENT` marker so that
    /// subsequent operations append events to it. Returns the new session.
    pub fn start_session(&self, source: &str) -> Result<Session> {
        if let Some(existing) = self.current_session_id()? {
            return Err(MemoraError::Invalid(format!(
                "a session is already active: {existing}; run `memora session end` first"
            )));
        }
        let now = self.clock.now();
        let session = Session {
            id: Uuid::new_v4().to_string(),
            source: source.to_string(),
            started_at: now,
            ended_at: None,
            event_count: 0,
        };
        self.store.insert_session(&session)?;
        // Make sure `.memora/sessions/` exists; init does this but a hand-
        // crafted store might not.
        fs::create_dir_all(self.memora_dir.join("sessions"))?;
        fs::write(self.current_session_path(), &session.id)?;
        // Record the start event itself.
        self.store.append_session_event(
            &session.id,
            now,
            SessionEventKind::SessionStarted,
            &serde_json::json!({ "source": source }),
        )?;
        Ok(session)
    }

    /// End the active session. Returns the closed session, or `None` if
    /// none was active.
    pub fn end_session(&self) -> Result<Option<Session>> {
        let id = match self.current_session_id()? {
            Some(id) => id,
            None => return Ok(None),
        };
        let now = self.clock.now();
        self.store.append_session_event(
            &id,
            now,
            SessionEventKind::SessionEnded,
            &serde_json::json!({}),
        )?;
        let mut session = self
            .store
            .get_session(&id)?
            .ok_or_else(|| MemoraError::Invalid(format!("session not found: {id}")))?;
        session.ended_at = Some(now);
        // event_count was incremented inside append_session_event.
        if let Some(updated) = self.store.get_session(&id)? {
            session.event_count = updated.event_count;
        }
        self.store.update_session(&session)?;
        let _ = fs::remove_file(self.current_session_path());
        Ok(Some(session))
    }

    /// Record an arbitrary event against the active session, if any. No-op
    /// when no session is active.
    pub fn record_event(&self, kind: SessionEventKind, data: serde_json::Value) -> Result<()> {
        let id = match self.current_session_id()? {
            Some(id) => id,
            None => return Ok(()),
        };
        let now = self.clock.now();
        self.store.append_session_event(&id, now, kind, &data)?;
        Ok(())
    }

    /// Read every event from a session, in append order.
    pub fn session_events(&self, session_id: &str) -> Result<Vec<SessionEvent>> {
        let resolved = self.store.resolve_session_prefix(session_id)?;
        self.store.session_events(&resolved)
    }

    /// List sessions, newest first.
    pub fn list_sessions(&self, limit: Option<usize>) -> Result<Vec<Session>> {
        self.store.list_sessions(limit)
    }

    // --- export ---------------------------------------------------------

    /// Score every node in the working set with the standard importance
    /// formula and return them sorted by score (highest first). Used by
    /// [`Self::export`].
    pub fn ranked_nodes(&self, weights: ImportanceWeights) -> Result<Vec<(MemoryNode, f32)>> {
        let nodes = self.store.all_nodes()?;
        let now = self.clock.now();
        let max_age = nodes
            .iter()
            .map(|n| (now - n.last_accessed).max(1))
            .max()
            .unwrap_or(1);
        let max_count = nodes.iter().map(|n| n.access_count).max().unwrap_or(0);

        let mut scored: Vec<(MemoryNode, f32)> = nodes
            .into_iter()
            .map(|n| {
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
                let score = (n.confidence * weights.confidence)
                    + (recency * weights.recency)
                    + (access * weights.access);
                (n, score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored)
    }

    /// Render the working set into the requested export format. The
    /// optional [`ExportFilter`] caps and filters the candidate set first.
    pub fn export(&self, format: ExportFormat, filter: ExportFilter) -> Result<String> {
        let scored = self.ranked_nodes(filter.weights)?;
        let mut nodes: Vec<MemoryNode> = scored
            .into_iter()
            .map(|(n, _)| n)
            .filter(|n| filter.matches(n))
            .collect();
        if let Some(top) = filter.top {
            nodes.truncate(top);
        }
        Ok(render_export(format, &nodes))
    }
}

// ---------------------------------------------------------------------------
// Free helpers + supporting types used by promote/diff above.
// ---------------------------------------------------------------------------

/// Options controlling [`Repository::merge`].
#[derive(Debug, Clone)]
pub struct MergeOptions {
    /// Strategy for resolving same-id divergences. Defaults to `Auto`.
    pub strategy: MergeStrategy,
    /// Allow fast-forward when possible (default: `true`).
    pub allow_fast_forward: bool,
    /// Create a merge commit at the end (default: `true`). When `false`,
    /// the working set is left in a merged state without committing.
    pub commit: bool,
    /// Override commit message. Defaults to `"Merge <theirs> into <branch>"`.
    pub message: Option<String>,
    /// Author for the merge commit. Defaults to `"human"`.
    pub author: String,
}

impl Default for MergeOptions {
    fn default() -> Self {
        Self {
            strategy: MergeStrategy::Auto,
            allow_fast_forward: true,
            commit: true,
            message: None,
            author: "human".into(),
        }
    }
}

/// What `memora merge` actually did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeKind {
    /// `theirs` was already an ancestor of `ours`. Nothing to do.
    AlreadyUpToDate,
    /// HEAD was fast-forwarded to `theirs`.
    FastForward,
    /// A real three-way merge happened, no conflicts.
    Merged,
    /// A real three-way merge happened with at least one conflict.
    Conflicts,
    /// Plan was applied to the working set but no commit was created.
    NoCommit,
}

/// Result returned by [`Repository::merge`].
#[derive(Debug, Clone)]
pub struct MergeOutcome {
    /// What kind of merge happened.
    pub kind: MergeKind,
    /// The plan that was computed.
    pub plan: MergePlan,
    /// The commit that was created, if any (None for `AlreadyUpToDate`,
    /// `FastForward` returns `theirs`, `NoCommit` returns `None`).
    pub commit: Option<MemoryCommit>,
}

fn short_for_display(id: &str) -> String {
    id.chars().take(7).collect()
}

/// Weights for the importance score used by [`Repository::ranked_nodes`].
/// They should sum to 1.0 for a normalised score, but no validation is
/// performed.
#[derive(Debug, Clone, Copy)]
pub struct ImportanceWeights {
    /// Weight of `confidence` (0.0 – 1.0).
    pub confidence: f32,
    /// Weight of recency (0.0 – 1.0, freshest = 1.0).
    pub recency: f32,
    /// Weight of access frequency (0.0 – 1.0, hottest = 1.0).
    pub access: f32,
}

impl Default for ImportanceWeights {
    fn default() -> Self {
        // Matches the formula in `docs/MEMORY_TYPES.md`.
        Self {
            confidence: 0.4,
            recency: 0.3,
            access: 0.3,
        }
    }
}

/// Filter / cap applied to the working set before [`Repository::export`]
/// renders it.
#[derive(Debug, Clone, Default)]
pub struct ExportFilter {
    /// Importance score weights.
    pub weights: ImportanceWeights,
    /// Keep at most this many nodes after ranking.
    pub top: Option<usize>,
    /// Restrict to specific kinds. Empty means "all kinds".
    pub kinds: Vec<MemoryKind>,
    /// Restrict to specific statuses. Empty means "all statuses except
    /// deprecated".
    pub statuses: Vec<MemoryStatus>,
    /// Drop nodes with confidence below this threshold.
    pub min_confidence: Option<f32>,
}

impl ExportFilter {
    /// Decide whether a single node passes the filter.
    pub fn matches(&self, node: &MemoryNode) -> bool {
        if !self.kinds.is_empty() && !self.kinds.contains(&node.kind) {
            return false;
        }
        if self.statuses.is_empty() {
            if node.status == MemoryStatus::Deprecated {
                return false;
            }
        } else if !self.statuses.contains(&node.status) {
            return false;
        }
        if let Some(min) = self.min_confidence {
            if node.confidence < min {
                return false;
            }
        }
        true
    }
}

/// Caller intent for [`Repository::promote`].
#[derive(Debug, Clone)]
pub enum PromotePlan {
    /// Promote a specific list of node ids.
    Ids(Vec<String>),
    /// Promote every ephemeral node of a given kind.
    Kind(MemoryKind),
    /// Promote every ephemeral node whose confidence is at least
    /// `min_confidence`.
    AllConfirmed {
        /// Confidence floor (clamped into `[0.0, 1.0]`).
        min_confidence: f32,
    },
}

/// Detail-level shape of a node change between two snapshots.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeChange {
    /// Memory status flipped (e.g. Ephemeral → Stable).
    Status {
        /// Previous status.
        from: MemoryStatus,
        /// New status.
        to: MemoryStatus,
    },
    /// Free-form content was edited.
    Content,
    /// Confidence value changed by more than 0.001.
    Confidence {
        /// Previous confidence.
        from: f32,
        /// New confidence.
        to: f32,
    },
    /// Source / provenance changed.
    Source,
    /// Evidence pointer changed.
    Evidence,
}

/// One entry in a [`DiffReport`] for a node that exists in both snapshots
/// but with different state.
#[derive(Debug, Clone)]
pub struct ModifiedNode {
    /// Snapshot of the node from the `from` side.
    pub before: MemoryNode,
    /// Snapshot of the node from the `to` side.
    pub after: MemoryNode,
    /// One or more concrete changes between the two.
    pub changes: Vec<NodeChange>,
}

/// Result of [`Repository::diff`]. Friendly buckets the CLI can render.
#[derive(Debug, Clone)]
pub struct DiffReport {
    /// Resolved id of the `from` side.
    pub from_id: String,
    /// Identifier of the `to` side (commit id, or `"(working set)"`).
    pub to_label: String,
    /// Nodes present in `to` but not in `from`.
    pub added: Vec<MemoryNode>,
    /// Nodes present in `from` but not in `to`.
    pub removed: Vec<MemoryNode>,
    /// Nodes present in both sides but with different state.
    pub modified: Vec<ModifiedNode>,
}

impl DiffReport {
    fn compute(
        from_id: String,
        to_label: String,
        from_nodes: &[MemoryNode],
        to_nodes: &[MemoryNode],
    ) -> Self {
        use std::collections::HashMap;
        let from_map: HashMap<&str, &MemoryNode> =
            from_nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let to_map: HashMap<&str, &MemoryNode> =
            to_nodes.iter().map(|n| (n.id.as_str(), n)).collect();

        let mut added = Vec::new();
        let mut modified = Vec::new();
        let mut removed = Vec::new();

        for n in to_nodes {
            match from_map.get(n.id.as_str()) {
                None => added.push(n.clone()),
                Some(prev) => {
                    let changes = diff_node(prev, n);
                    if !changes.is_empty() {
                        modified.push(ModifiedNode {
                            before: (*prev).clone(),
                            after: n.clone(),
                            changes,
                        });
                    }
                }
            }
        }
        for n in from_nodes {
            if !to_map.contains_key(n.id.as_str()) {
                removed.push(n.clone());
            }
        }

        added.sort_by(|a, b| a.id.cmp(&b.id));
        removed.sort_by(|a, b| a.id.cmp(&b.id));
        modified.sort_by(|a, b| a.after.id.cmp(&b.after.id));

        DiffReport {
            from_id,
            to_label,
            added,
            removed,
            modified,
        }
    }

    /// True if `added`, `removed`, and `modified` are all empty.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }

    /// Render a short, human-readable summary line per change. Used by
    /// `memora diff --semantic`.
    pub fn semantic_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for n in &self.added {
            out.push(format!(
                "+ [{}] new {} memory: {}",
                n.kind,
                n.status,
                short(&n.content, 80),
            ));
        }
        for m in &self.modified {
            for ch in &m.changes {
                let line = match ch {
                    NodeChange::Status { from, to } => format!(
                        "~ [{}] {} → {}: {}",
                        m.after.kind,
                        from,
                        to,
                        short(&m.after.content, 80),
                    ),
                    NodeChange::Content => format!(
                        "~ [{}] content updated: {}",
                        m.after.kind,
                        short(&m.after.content, 80),
                    ),
                    NodeChange::Confidence { from, to } => format!(
                        "~ [{}] confidence {:.2} → {:.2}: {}",
                        m.after.kind,
                        from,
                        to,
                        short(&m.after.content, 80),
                    ),
                    NodeChange::Source => format!(
                        "~ [{}] source changed: {}",
                        m.after.kind,
                        short(&m.after.content, 80),
                    ),
                    NodeChange::Evidence => format!(
                        "~ [{}] evidence updated: {}",
                        m.after.kind,
                        short(&m.after.content, 80),
                    ),
                };
                out.push(line);
            }
        }
        for n in &self.removed {
            out.push(format!(
                "- [{}] removed: {}",
                n.kind,
                short(&n.content, 80),
            ));
        }
        out
    }
}

fn parse_tilde(rest: &str) -> Option<usize> {
    if rest.is_empty() {
        return None;
    }
    let stripped = rest.strip_prefix('~')?;
    if stripped.is_empty() {
        Some(1)
    } else {
        stripped.parse::<usize>().ok()
    }
}

fn state_differs(a: &MemoryNode, b: &MemoryNode) -> bool {
    !diff_node(a, b).is_empty()
}

fn diff_node(a: &MemoryNode, b: &MemoryNode) -> Vec<NodeChange> {
    let mut out = Vec::new();
    if a.status != b.status {
        out.push(NodeChange::Status {
            from: a.status,
            to: b.status,
        });
    }
    if a.content != b.content {
        out.push(NodeChange::Content);
    }
    if (a.confidence - b.confidence).abs() > 0.001 {
        out.push(NodeChange::Confidence {
            from: a.confidence,
            to: b.confidence,
        });
    }
    if a.source != b.source {
        out.push(NodeChange::Source);
    }
    if a.evidence != b.evidence {
        out.push(NodeChange::Evidence);
    }
    out
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{MemoryKind, MemorySource, NewNode};
    use std::sync::atomic::{AtomicI64, Ordering};
    use tempfile::tempdir;

    /// A clock that advances by one second every time it's read — useful
    /// so successive node ids and commits are always distinct in tests.
    struct StepClock(AtomicI64);
    impl Clock for StepClock {
        fn now(&self) -> i64 {
            self.0.fetch_add(1, Ordering::SeqCst)
        }
    }

    fn new_repo(path: &Path) -> Repository {
        Repository::init(path)
            .unwrap()
            .with_clock(Box::new(StepClock(AtomicI64::new(1_000))))
    }

    #[test]
    fn init_creates_directory_layout() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        assert!(repo.memora_dir().exists());
        assert!(repo.memora_dir().join("HEAD").exists());
        assert!(repo.memora_dir().join("config").exists());
        assert!(repo.memora_dir().join("memora.db").exists());
        assert_eq!(repo.head().unwrap().branch(), Some("main"));
    }

    #[test]
    fn double_init_is_an_error() {
        let tmp = tempdir().unwrap();
        let _ = new_repo(tmp.path());
        match Repository::init(tmp.path()) {
            Err(MemoraError::AlreadyInitialised { .. }) => {}
            Err(other) => panic!("expected AlreadyInitialised, got {other:?}"),
            Ok(_) => panic!("expected init to fail"),
        }
    }

    #[test]
    fn add_commit_and_log_round_trip() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        repo.add_node(NewNode::new(
            MemoryKind::Semantic,
            "auth uses JWT",
            MemorySource::CodeRead,
        ))
        .unwrap();
        let outcome = repo.commit("first memory", "human").unwrap();
        assert!(outcome.commit.is_some());
        let log = repo.log(None).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].message, "first memory");
        assert_eq!(log[0].stats.added, 1);
    }

    #[test]
    fn empty_commit_is_a_noop() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        repo.add_node(NewNode::new(
            MemoryKind::Project,
            "rust",
            MemorySource::CodeRead,
        ))
        .unwrap();
        repo.commit("first", "human").unwrap();
        let again = repo.commit("nothing changed", "human").unwrap();
        assert!(again.commit.is_none());
    }

    #[test]
    fn branch_and_switch() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        repo.add_node(NewNode::new(
            MemoryKind::Project,
            "x",
            MemorySource::CodeRead,
        ))
        .unwrap();
        repo.commit("c1", "human").unwrap();
        repo.create_branch("feature/x").unwrap();
        repo.switch_branch("feature/x").unwrap();
        assert_eq!(repo.head().unwrap().branch(), Some("feature/x"));
        let branches = repo.list_branches().unwrap();
        assert!(branches.contains(&"main".to_string()));
        assert!(branches.contains(&"feature/x".to_string()));
    }

    #[test]
    fn rollback_creates_checkpoint_then_moves_head() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        repo.add_node(NewNode::new(
            MemoryKind::Project,
            "v1",
            MemorySource::CodeRead,
        ))
        .unwrap();
        let first = repo.commit("c1", "human").unwrap().commit.unwrap();
        repo.add_node(NewNode::new(
            MemoryKind::Project,
            "v2",
            MemorySource::CodeRead,
        ))
        .unwrap();
        let _second = repo.commit("c2", "human").unwrap().commit.unwrap();
        let rolled = repo.rollback_to(&first.id, "human").unwrap();
        assert_eq!(rolled.id, first.id);
        assert_eq!(repo.head_commit_id().unwrap().as_deref(), Some(first.id.as_str()));
    }

    #[test]
    fn promote_by_id_marks_node_stable() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        let node = repo
            .add_node(NewNode::new(
                MemoryKind::Assumption,
                "redis is the cache",
                MemorySource::ModelInference,
            ))
            .unwrap();
        let promoted = repo.promote(PromotePlan::Ids(vec![node.id.clone()])).unwrap();
        assert_eq!(promoted, vec![node.id.clone()]);
        let after = repo.store().get_node(&node.id).unwrap().unwrap();
        assert_eq!(after.status, MemoryStatus::Stable);
    }

    #[test]
    fn promote_by_kind_only_touches_matching_ephemeral_nodes() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        repo.add_node(NewNode::new(
            MemoryKind::Assumption,
            "a",
            MemorySource::ModelInference,
        ))
        .unwrap();
        let other = repo
            .add_node(NewNode::new(
                MemoryKind::Project,
                "p",
                MemorySource::CodeRead,
            ))
            .unwrap();
        let promoted = repo.promote(PromotePlan::Kind(MemoryKind::Assumption)).unwrap();
        assert_eq!(promoted.len(), 1);
        assert_eq!(
            repo.store().get_node(&other.id).unwrap().unwrap().status,
            MemoryStatus::Ephemeral
        );
    }

    #[test]
    fn promote_all_confirmed_respects_threshold() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        let high = repo
            .add_node(NewNode::new(
                MemoryKind::Project,
                "high",
                MemorySource::CodeRead,
            ))
            .unwrap();
        let low = repo
            .add_node(NewNode {
                confidence: Some(0.4),
                ..NewNode::new(
                    MemoryKind::Assumption,
                    "low",
                    MemorySource::ModelInference,
                )
            })
            .unwrap();
        let promoted = repo
            .promote(PromotePlan::AllConfirmed { min_confidence: 0.8 })
            .unwrap();
        assert_eq!(promoted, vec![high.id.clone()]);
        assert_eq!(
            repo.store().get_node(&low.id).unwrap().unwrap().status,
            MemoryStatus::Ephemeral
        );
    }

    #[test]
    fn promote_then_commit_records_promotion_stat() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        let n = repo
            .add_node(NewNode::new(
                MemoryKind::Assumption,
                "x",
                MemorySource::ModelInference,
            ))
            .unwrap();
        repo.commit("first", "human").unwrap();
        repo.promote(PromotePlan::Ids(vec![n.id.clone()])).unwrap();
        let c = repo.commit("promote it", "human").unwrap().commit.unwrap();
        assert_eq!(c.stats.promoted, 1);
        assert_eq!(c.stats.modified, 1);
        assert_eq!(c.stats.added, 0);
    }

    #[test]
    fn diff_between_commits_reports_added_and_promoted() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        let n = repo
            .add_node(NewNode::new(
                MemoryKind::Assumption,
                "redis is the cache",
                MemorySource::ModelInference,
            ))
            .unwrap();
        let c1 = repo.commit("first", "human").unwrap().commit.unwrap();
        repo.promote(PromotePlan::Ids(vec![n.id.clone()])).unwrap();
        repo.add_node(NewNode::new(
            MemoryKind::Project,
            "rust workspace",
            MemorySource::CodeRead,
        ))
        .unwrap();
        let c2 = repo.commit("promote and add", "human").unwrap().commit.unwrap();

        let diff = repo.diff(&c1.id, Some(&c2.id)).unwrap();
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.modified.len(), 1);
        assert!(matches!(
            diff.modified[0].changes[0],
            NodeChange::Status {
                from: MemoryStatus::Ephemeral,
                to: MemoryStatus::Stable
            }
        ));
    }

    #[test]
    fn diff_understands_head_tilde() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        repo.add_node(NewNode::new(
            MemoryKind::Project,
            "v1",
            MemorySource::CodeRead,
        ))
        .unwrap();
        let _c1 = repo.commit("c1", "human").unwrap();
        repo.add_node(NewNode::new(
            MemoryKind::Project,
            "v2",
            MemorySource::CodeRead,
        ))
        .unwrap();
        let _c2 = repo.commit("c2", "human").unwrap();
        let diff = repo.diff("HEAD~1", Some("HEAD")).unwrap();
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.modified.len(), 0);
        assert_eq!(diff.removed.len(), 0);
    }

    // -------------------------- merge tests --------------------------

    #[test]
    fn merge_already_up_to_date_is_noop() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        repo.add_node(NewNode::new(MemoryKind::Project, "v1", MemorySource::CodeRead))
            .unwrap();
        repo.commit("c1", "human").unwrap();
        repo.create_branch("feature/x").unwrap();
        // ours == theirs.
        let outcome = repo.merge("feature/x", MergeOptions::default()).unwrap();
        assert_eq!(outcome.kind, MergeKind::AlreadyUpToDate);
    }

    #[test]
    fn merge_fast_forward_advances_branch() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        // base commit on main.
        repo.add_node(NewNode::new(MemoryKind::Project, "v1", MemorySource::CodeRead))
            .unwrap();
        repo.commit("c1", "human").unwrap();
        repo.create_branch("feature").unwrap();
        // advance feature.
        repo.switch_branch("feature").unwrap();
        repo.add_node(NewNode::new(MemoryKind::Project, "v2", MemorySource::CodeRead))
            .unwrap();
        let c2 = repo.commit("c2", "human").unwrap().commit.unwrap();
        // back to main and merge feature.
        repo.switch_branch("main").unwrap();
        let outcome = repo.merge("feature", MergeOptions::default()).unwrap();
        assert_eq!(outcome.kind, MergeKind::FastForward);
        assert_eq!(repo.head_commit_id().unwrap().as_deref(), Some(c2.id.as_str()));
    }

    #[test]
    fn merge_clean_three_way_picks_up_disjoint_changes() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        repo.add_node(NewNode::new(
            MemoryKind::Project,
            "shared",
            MemorySource::CodeRead,
        ))
        .unwrap();
        repo.commit("base", "human").unwrap();
        // feature: add a new node only on this side.
        repo.create_branch("feature").unwrap();
        repo.switch_branch("feature").unwrap();
        repo.add_node(NewNode::new(
            MemoryKind::Semantic,
            "auth uses jwt",
            MemorySource::CodeRead,
        ))
        .unwrap();
        repo.commit("feat", "human").unwrap();
        // main: add a different node.
        repo.switch_branch("main").unwrap();
        repo.add_node(NewNode::new(
            MemoryKind::Preference,
            "verbose errors",
            MemorySource::Manual,
        ))
        .unwrap();
        repo.commit("pref", "human").unwrap();
        // merge feature into main.
        let outcome = repo.merge("feature", MergeOptions::default()).unwrap();
        assert_eq!(outcome.kind, MergeKind::Merged);
        let nodes = repo.store().all_nodes().unwrap();
        let contents: std::collections::HashSet<String> =
            nodes.into_iter().map(|n| n.content).collect();
        assert!(contents.contains("shared"));
        assert!(contents.contains("auth uses jwt"));
        assert!(contents.contains("verbose errors"));
        assert!(!outcome.plan.has_conflicts());
        assert!(outcome.commit.is_some());
        let commit = outcome.commit.unwrap();
        let merge_parents = repo.store().merge_parents(&commit.id).unwrap();
        assert_eq!(merge_parents.len(), 1);
    }

    #[test]
    fn merge_resolves_same_node_change_by_confidence() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        let base_node = repo
            .add_node(NewNode::new(
                MemoryKind::Assumption,
                "redis is the cache",
                MemorySource::ModelInference,
            ))
            .unwrap();
        repo.commit("base", "human").unwrap();
        repo.create_branch("feature").unwrap();
        // ours promote with model-inference baseline confidence (0.6).
        repo.store()
            .set_status(&base_node.id, MemoryStatus::Stable, 1234)
            .unwrap();
        repo.commit("ours promote", "human").unwrap();
        // theirs: switch to feature *before* mutating, then promote with
        // a higher confidence so the merge has a reason to pick theirs.
        repo.switch_branch("feature").unwrap();
        let mut node = repo.store().get_node(&base_node.id).unwrap().unwrap();
        node.confidence = 0.95;
        node.status = MemoryStatus::Stable;
        node.updated_at += 1;
        repo.store().upsert_node(&node).unwrap();
        repo.commit("theirs promote with high confidence", "human")
            .unwrap();
        // back to main and merge feature.
        repo.switch_branch("main").unwrap();
        let outcome = repo.merge("feature", MergeOptions::default()).unwrap();
        assert_eq!(outcome.kind, MergeKind::Merged);
        let merged = repo.store().get_node(&base_node.id).unwrap().unwrap();
        assert_eq!(merged.status, MemoryStatus::Stable);
        assert!(merged.confidence > 0.9);
    }

    #[test]
    fn merge_with_strategy_ours_keeps_our_side() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        let base_node = repo
            .add_node(NewNode::new(
                MemoryKind::Project,
                "rust",
                MemorySource::CodeRead,
            ))
            .unwrap();
        repo.commit("base", "human").unwrap();
        repo.create_branch("feature").unwrap();
        // ours edits content (still on main).
        let mut ours = repo.store().get_node(&base_node.id).unwrap().unwrap();
        ours.content = "rust+wasm".into();
        ours.updated_at += 1;
        repo.store().upsert_node(&ours).unwrap();
        repo.commit("ours", "human").unwrap();
        // switch to feature, edit differently.
        repo.switch_branch("feature").unwrap();
        let mut theirs = repo.store().get_node(&base_node.id).unwrap().unwrap();
        theirs.content = "rust+ts".into();
        theirs.updated_at += 2;
        repo.store().upsert_node(&theirs).unwrap();
        repo.commit("theirs", "human").unwrap();
        repo.switch_branch("main").unwrap();
        let outcome = repo
            .merge(
                "feature",
                MergeOptions {
                    strategy: MergeStrategy::Ours,
                    ..MergeOptions::default()
                },
            )
            .unwrap();
        assert_eq!(outcome.kind, MergeKind::Merged);
        let merged = repo.store().get_node(&base_node.id).unwrap().unwrap();
        assert_eq!(merged.content, "rust+wasm");
    }

    #[test]
    fn merge_marks_conflict_when_strategy_auto_ties() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        let base_node = repo
            .add_node(NewNode::new(
                MemoryKind::Project,
                "rust",
                MemorySource::CodeRead,
            ))
            .unwrap();
        repo.commit("base", "human").unwrap();
        repo.create_branch("feature").unwrap();
        // ours edits content with a fixed updated_at so it stays equal to theirs.
        let ts = 5_000;
        let mut ours = repo.store().get_node(&base_node.id).unwrap().unwrap();
        ours.content = "rust+wasm".into();
        ours.updated_at = ts;
        repo.store().upsert_node(&ours).unwrap();
        repo.commit("ours", "human").unwrap();
        // switch to feature, edit different content with the same ts.
        repo.switch_branch("feature").unwrap();
        let mut theirs = repo.store().get_node(&base_node.id).unwrap().unwrap();
        theirs.content = "rust+ts".into();
        theirs.updated_at = ts;
        repo.store().upsert_node(&theirs).unwrap();
        repo.commit("theirs", "human").unwrap();
        repo.switch_branch("main").unwrap();
        let outcome = repo.merge("feature", MergeOptions::default()).unwrap();
        assert_eq!(outcome.kind, MergeKind::Conflicts);
        assert!(outcome.plan.has_conflicts());
        let merged = repo.store().get_node(&base_node.id).unwrap().unwrap();
        assert_eq!(merged.status, MemoryStatus::Conflicted);
    }
}


#[cfg(test)]
mod phase4_tests {
    use super::*;
    use crate::node::{MemoryKind, MemorySource, NewNode};
    use std::path::Path;
    use std::sync::atomic::{AtomicI64, Ordering};
    use tempfile::tempdir;

    struct StepClock(AtomicI64);
    impl Clock for StepClock {
        fn now(&self) -> i64 {
            self.0.fetch_add(1, Ordering::SeqCst)
        }
    }

    fn new_repo(path: &Path) -> Repository {
        Repository::init(path)
            .unwrap()
            .with_clock(Box::new(StepClock(AtomicI64::new(2_000))))
    }

    #[test]
    fn session_records_add_and_commit_events() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        let session = repo.start_session("claude_code").unwrap();

        repo.add_node(NewNode::new(
            MemoryKind::Project,
            "uses Rust",
            MemorySource::CodeRead,
        ))
        .unwrap();
        repo.commit("first", "human").unwrap();

        let ended = repo.end_session().unwrap().expect("session was active");
        let events = repo.session_events(&session.id).unwrap();
        // Expect: started, node_added, commit_created, ended.
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].kind, SessionEventKind::SessionStarted);
        assert_eq!(events[1].kind, SessionEventKind::NodeAdded);
        assert_eq!(events[2].kind, SessionEventKind::CommitCreated);
        assert_eq!(events[3].kind, SessionEventKind::SessionEnded);
        assert_eq!(ended.event_count, 4);
        // After end_session, CURRENT marker is gone.
        assert!(repo.current_session_id().unwrap().is_none());
    }

    #[test]
    fn cannot_start_two_sessions() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        let _s = repo.start_session("manual").unwrap();
        let again = repo.start_session("manual");
        assert!(again.is_err());
    }

    #[test]
    fn record_event_is_noop_when_no_session() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        // Adding a node without a session should still succeed and not blow up.
        repo.add_node(NewNode::new(
            MemoryKind::Project,
            "no session",
            MemorySource::CodeRead,
        ))
        .unwrap();
        let sessions = repo.list_sessions(None).unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn promote_emits_promotion_event() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        let n = repo
            .add_node(NewNode::new(
                MemoryKind::Assumption,
                "redis",
                MemorySource::ModelInference,
            ))
            .unwrap();
        let session = repo.start_session("claude_code").unwrap();
        repo.promote(PromotePlan::Ids(vec![n.id.clone()])).unwrap();
        repo.end_session().unwrap();
        let events = repo.session_events(&session.id).unwrap();
        assert!(events
            .iter()
            .any(|e| e.kind == SessionEventKind::NodePromoted));
    }

    #[test]
    fn export_json_round_trip() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        repo.add_node(NewNode::new(
            MemoryKind::Project,
            "uses Rust",
            MemorySource::CodeRead,
        ))
        .unwrap();
        repo.add_node(NewNode {
            confidence: Some(0.4),
            ..NewNode::new(
                MemoryKind::Assumption,
                "redis is the cache",
                MemorySource::ModelInference,
            )
        })
        .unwrap();
        let body = repo
            .export(ExportFormat::Json, ExportFilter::default())
            .unwrap();
        let parsed: Vec<MemoryNode> = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn export_filters_drop_below_threshold_and_deprecated() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        let _high = repo
            .add_node(NewNode::new(
                MemoryKind::Project,
                "high",
                MemorySource::CodeRead,
            ))
            .unwrap();
        let _low = repo
            .add_node(NewNode {
                confidence: Some(0.2),
                ..NewNode::new(
                    MemoryKind::Assumption,
                    "low",
                    MemorySource::ModelInference,
                )
            })
            .unwrap();
        let body = repo
            .export(
                ExportFormat::Json,
                ExportFilter {
                    min_confidence: Some(0.5),
                    ..ExportFilter::default()
                },
            )
            .unwrap();
        let parsed: Vec<MemoryNode> = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].content, "high");
    }

    #[test]
    fn export_claude_code_groups_by_kind() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        repo.add_node(NewNode::new(
            MemoryKind::Semantic,
            "auth uses jwt",
            MemorySource::CodeRead,
        ))
        .unwrap();
        repo.add_node(NewNode::new(
            MemoryKind::Project,
            "rust workspace",
            MemorySource::CodeRead,
        ))
        .unwrap();
        let body = repo
            .export(ExportFormat::ClaudeCode, ExportFilter::default())
            .unwrap();
        assert!(body.contains("# Project memory"));
        assert!(body.contains("## Project"));
        assert!(body.contains("## Semantic"));
        assert!(body.contains("auth uses jwt"));
        assert!(body.contains("rust workspace"));
    }

    #[test]
    fn export_top_caps_results_after_ranking() {
        let tmp = tempdir().unwrap();
        let repo = new_repo(tmp.path());
        for i in 0..5 {
            repo.add_node(NewNode::new(
                MemoryKind::Project,
                format!("entry-{i}"),
                MemorySource::CodeRead,
            ))
            .unwrap();
        }
        let body = repo
            .export(
                ExportFormat::Json,
                ExportFilter {
                    top: Some(2),
                    ..ExportFilter::default()
                },
            )
            .unwrap();
        let parsed: Vec<MemoryNode> = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed.len(), 2);
    }
}
