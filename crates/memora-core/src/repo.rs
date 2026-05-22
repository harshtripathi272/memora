//! High level repository facade.
//!
//! [`Repository`] is the only thing the CLI talks to. It hides the details
//! of the on-disk layout (refs, HEAD, SQLite) behind a small set of
//! intent-revealing methods: `init`, `add_node`, `commit`, `log`, `status`,
//! `branch`, `switch`, `rollback`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::commit::{commit_id, tree_id_for_nodes, CommitStats, MemoryCommit};
use crate::error::{MemoraError, Result};
use crate::node::{MemoryKind, MemoryNode, MemoryStatus, NewNode};
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

        // Detect "nothing to commit": same tree id as parent.
        let parent_tree = match parent.as_deref() {
            Some(p) => self
                .store
                .get_commit(p)?
                .map(|c| c.tree_id)
                .unwrap_or_default(),
            None => tree_id_for_nodes(&[]),
        };
        if parent.is_some() && tree == parent_tree {
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
        let id = commit_id(parent.as_deref(), &tree, author, message, now);
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

        Ok(CommitOutcome {
            commit: Some(commit),
            branch: branch_name,
        })
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
    pub fn switch_branch(&self, name: &str) -> Result<()> {
        if !self.refs.branch_path(name).exists() {
            return Err(MemoraError::RefNotFound(name.to_string()));
        }
        self.refs.write_head_branch(name)
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
}

// ---------------------------------------------------------------------------
// Free helpers + supporting types used by promote/diff above.
// ---------------------------------------------------------------------------

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
}
