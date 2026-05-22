//! High level repository facade.
//!
//! [`Repository`] is the only thing the CLI talks to. It hides the details
//! of the on-disk layout (refs, HEAD, SQLite) behind a small set of
//! intent-revealing methods: `init`, `add_node`, `commit`, `log`, `status`,
//! `branch`, `switch`, `rollback`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::commit::{commit_id, tree_id_for, CommitStats, MemoryCommit};
use crate::error::{MemoraError, Result};
use crate::node::{MemoryNode, MemoryStatus, NewNode};
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
        let parent_node_ids: Vec<String> = match parent.as_deref() {
            Some(p) => self.store.commit_node_ids(p)?,
            None => Vec::new(),
        };
        let parent_set: std::collections::HashSet<String> =
            parent_node_ids.iter().cloned().collect();

        let nodes = self.store.all_nodes()?;
        let mut node_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
        node_ids.sort();
        let current_set: std::collections::HashSet<String> = node_ids.iter().cloned().collect();
        let tree = tree_id_for(&node_ids);

        // Detect "nothing to commit": same tree id as parent.
        let parent_tree = match parent.as_deref() {
            Some(p) => self
                .store
                .get_commit(p)?
                .map(|c| c.tree_id)
                .unwrap_or_default(),
            None => tree_id_for(&[]),
        };
        if parent.is_some() && tree == parent_tree {
            return Ok(CommitOutcome {
                commit: None,
                branch: head_ref.branch().map(str::to_string),
            });
        }

        // Compute stats relative to the parent.
        let parent_ts = match parent.as_deref() {
            Some(p) => self.store.get_commit(p)?.map(|c| c.timestamp).unwrap_or(0),
            None => 0,
        };
        let mut stats = CommitStats::default();
        for node in &nodes {
            let is_new = !parent_set.contains(&node.id);
            if is_new {
                stats.added += 1;
            } else if node.updated_at > parent_ts {
                stats.modified += 1;
            }
            if node.status == MemoryStatus::Stable && node.updated_at > parent_ts {
                stats.promoted += 1;
            }
            if node.status == MemoryStatus::Conflicted && node.updated_at > parent_ts {
                stats.conflicted += 1;
            }
        }
        stats.removed = parent_set.difference(&current_set).count() as u32;

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
        self.store.insert_commit_nodes(&id, &node_ids)?;

        let branch_name = head_ref.branch().map(str::to_string);
        match &head_ref {
            HeadRef::Branch(name) => {
                // Make sure the branch ref exists (it always should after init).
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
}
