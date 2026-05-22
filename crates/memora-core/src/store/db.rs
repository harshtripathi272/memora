//! SQLite-backed object store.
//!
//! All memory nodes, commits, and branch metadata live in a single
//! `memora.db` file. We use `rusqlite` with the `bundled` feature so the
//! binary has no external dependencies.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use rusqlite::{params, Connection, OptionalExtension};

use crate::commit::{CommitStats, MemoryCommit};
use crate::error::{MemoraError, Result};
use crate::node::{MemoryKind, MemoryNode, MemorySource, MemoryStatus};

const SCHEMA_SQL: &str = include_str!("schema.sql");

/// A summary of nodes that are not yet captured by HEAD's snapshot — used by
/// `memora status`.
#[derive(Debug, Clone, Default)]
pub struct UnstagedSummary {
    /// Nodes that exist now but were not in HEAD's tree.
    pub added: Vec<MemoryNode>,
    /// Nodes that existed in HEAD's tree but have a newer `updated_at`.
    pub modified: Vec<MemoryNode>,
    /// Nodes that existed in HEAD's tree but no longer exist (rare for now).
    pub removed: Vec<String>,
    /// Total nodes currently in the working set.
    pub total: usize,
}

/// Owns the SQLite connection and exposes typed CRUD over the schema.
pub struct Store {
    conn: Connection,
    /// Path to the database file.
    path: PathBuf,
}

impl Store {
    /// Open (or create) the SQLite database at `path` and apply the schema.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let conn = Connection::open(&path)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA_SQL)?;
        Ok(Self { conn, path })
    }

    /// Database file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    // --- node CRUD --------------------------------------------------------

    /// Insert a node, replacing any existing row with the same id. The
    /// caller is expected to have already filled in timestamps and id.
    pub fn upsert_node(&self, node: &MemoryNode) -> Result<()> {
        let tags = serde_json::to_string(&node.tags)?;
        let related = serde_json::to_string(&node.related_to)?;
        self.conn.execute(
            "INSERT INTO nodes (
                id, kind, content, confidence, status, source,
                evidence, tags_json, related_to_json,
                created_at, updated_at, last_accessed, access_count, expires_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
             ON CONFLICT(id) DO UPDATE SET
                kind = excluded.kind,
                content = excluded.content,
                confidence = excluded.confidence,
                status = excluded.status,
                source = excluded.source,
                evidence = excluded.evidence,
                tags_json = excluded.tags_json,
                related_to_json = excluded.related_to_json,
                updated_at = excluded.updated_at,
                last_accessed = excluded.last_accessed,
                access_count = excluded.access_count,
                expires_at = excluded.expires_at",
            params![
                node.id,
                node.kind.as_str(),
                node.content,
                node.confidence as f64,
                node.status.as_str(),
                node.source.as_str(),
                node.evidence,
                tags,
                related,
                node.created_at,
                node.updated_at,
                node.last_accessed,
                node.access_count,
                node.expires_at,
            ],
        )?;
        Ok(())
    }

    /// Fetch a node by id.
    pub fn get_node(&self, id: &str) -> Result<Option<MemoryNode>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, content, confidence, status, source, evidence,
                    tags_json, related_to_json, created_at, updated_at,
                    last_accessed, access_count, expires_at
             FROM nodes WHERE id = ?1",
        )?;
        let node = stmt
            .query_row(params![id], row_to_node)
            .optional()?;
        Ok(node)
    }

    /// Return *all* nodes currently in the store. Order is undefined.
    pub fn all_nodes(&self) -> Result<Vec<MemoryNode>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, content, confidence, status, source, evidence,
                    tags_json, related_to_json, created_at, updated_at,
                    last_accessed, access_count, expires_at
             FROM nodes",
        )?;
        let rows = stmt.query_map([], row_to_node)?;
        let mut nodes = Vec::new();
        for n in rows {
            nodes.push(n?);
        }
        Ok(nodes)
    }

    /// Count how many nodes exist by kind. Returns counts for *all*
    /// six kinds even when zero, in canonical order.
    pub fn count_by_kind(&self) -> Result<Vec<(MemoryKind, u32)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT kind, COUNT(*) FROM nodes GROUP BY kind")?;
        let rows = stmt.query_map([], |r| {
            let kind: String = r.get(0)?;
            let count: u32 = r.get(1)?;
            Ok((kind, count))
        })?;
        let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for r in rows {
            let (k, c) = r?;
            counts.insert(k, c);
        }
        let mut out = Vec::with_capacity(MemoryKind::ALL.len());
        for k in MemoryKind::ALL {
            out.push((k, counts.get(k.as_str()).copied().unwrap_or(0)));
        }
        Ok(out)
    }

    /// Count how many nodes exist with the given status.
    pub fn count_by_status(&self, status: MemoryStatus) -> Result<u32> {
        let mut stmt = self
            .conn
            .prepare("SELECT COUNT(*) FROM nodes WHERE status = ?1")?;
        let count: u32 = stmt.query_row(params![status.as_str()], |r| r.get(0))?;
        Ok(count)
    }

    // --- commit CRUD ------------------------------------------------------

    /// Persist a commit row.
    pub fn insert_commit(&self, commit: &MemoryCommit) -> Result<()> {
        let stats = serde_json::to_string(&commit.stats)?;
        self.conn.execute(
            "INSERT INTO commits (id, parent_id, message, author, timestamp, tree_id, stats_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                commit.id,
                commit.parent,
                commit.message,
                commit.author,
                commit.timestamp,
                commit.tree_id,
                stats,
            ],
        )?;
        Ok(())
    }

    /// Persist the (commit_id, node_id) membership rows for a snapshot.
    pub fn insert_commit_nodes(&self, commit_id: &str, node_ids: &[String]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt =
                tx.prepare("INSERT OR IGNORE INTO commit_nodes (commit_id, node_id) VALUES (?1, ?2)")?;
            for nid in node_ids {
                stmt.execute(params![commit_id, nid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Fetch a commit by id.
    pub fn get_commit(&self, id: &str) -> Result<Option<MemoryCommit>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, parent_id, message, author, timestamp, tree_id, stats_json
             FROM commits WHERE id = ?1",
        )?;
        let commit = stmt
            .query_row(params![id], row_to_commit)
            .optional()?;
        Ok(commit)
    }

    /// Resolve a possibly-abbreviated commit id to the full id.
    /// Returns `Err(CommitNotFound)` if zero matches and `Err(Invalid)` on
    /// ambiguity.
    pub fn resolve_commit_prefix(&self, prefix: &str) -> Result<String> {
        if prefix.len() < 4 {
            return Err(MemoraError::Invalid(
                "commit id must be at least 4 characters".into(),
            ));
        }
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM commits WHERE id LIKE ?1 || '%' LIMIT 2")?;
        let rows = stmt.query_map(params![prefix], |r| r.get::<_, String>(0))?;
        let mut matches = Vec::new();
        for r in rows {
            matches.push(r?);
        }
        match matches.len() {
            0 => Err(MemoraError::CommitNotFound(prefix.to_string())),
            1 => Ok(matches.pop().unwrap()),
            _ => Err(MemoraError::Invalid(format!(
                "ambiguous commit id '{prefix}' (matched multiple commits)"
            ))),
        }
    }

    /// Walk the commit chain from `head` toward the root, in newest-first
    /// order. Stops after `limit` entries if specified.
    pub fn walk_commits(&self, head: &str, limit: Option<usize>) -> Result<Vec<MemoryCommit>> {
        let mut out = Vec::new();
        let mut current = Some(head.to_string());
        while let Some(id) = current {
            if let Some(max) = limit {
                if out.len() >= max {
                    break;
                }
            }
            match self.get_commit(&id)? {
                Some(c) => {
                    current = c.parent.clone();
                    out.push(c);
                }
                None => return Err(MemoraError::CommitNotFound(id)),
            }
        }
        Ok(out)
    }

    /// Return the node ids belonging to a particular commit's tree.
    pub fn commit_node_ids(&self, commit_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT node_id FROM commit_nodes WHERE commit_id = ?1")?;
        let rows = stmt.query_map(params![commit_id], |r| r.get::<_, String>(0))?;
        let mut ids = Vec::new();
        for r in rows {
            ids.push(r?);
        }
        Ok(ids)
    }

    /// Compute an [`UnstagedSummary`] comparing the current node table to
    /// the given baseline commit (typically HEAD). Pass `None` for the
    /// "no commits yet" case — every existing node is reported as added.
    pub fn unstaged_against(&self, head_commit: Option<&str>) -> Result<UnstagedSummary> {
        let current = self.all_nodes()?;
        let baseline_ids: std::collections::HashSet<String> = match head_commit {
            Some(id) => self.commit_node_ids(id)?.into_iter().collect(),
            None => std::collections::HashSet::new(),
        };
        let mut summary = UnstagedSummary {
            total: current.len(),
            ..Default::default()
        };
        let baseline_commit_ts = match head_commit {
            Some(id) => self.get_commit(id)?.map(|c| c.timestamp).unwrap_or(0),
            None => 0,
        };
        let mut current_ids = std::collections::HashSet::new();
        for node in current {
            current_ids.insert(node.id.clone());
            if !baseline_ids.contains(&node.id) {
                summary.added.push(node);
            } else if node.updated_at > baseline_commit_ts {
                summary.modified.push(node);
            }
        }
        for id in baseline_ids.difference(&current_ids) {
            summary.removed.push(id.clone());
        }
        Ok(summary)
    }
}

fn row_to_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryNode> {
    let kind_str: String = row.get(1)?;
    let status_str: String = row.get(4)?;
    let source_str: String = row.get(5)?;
    let tags_json: String = row.get(7)?;
    let related_json: String = row.get(8)?;

    // The conversions below shouldn't fail unless someone hand-edited the DB,
    // but we map them through rusqlite's error type to keep the API tidy.
    let kind = MemoryKind::from_str(&kind_str).map_err(to_sqlite_err)?;
    let status = MemoryStatus::from_str(&status_str).map_err(to_sqlite_err)?;
    let source = MemorySource::from_str(&source_str).map_err(to_sqlite_err)?;
    let tags: Vec<String> = serde_json::from_str(&tags_json).map_err(serde_to_sqlite_err)?;
    let related: Vec<String> =
        serde_json::from_str(&related_json).map_err(serde_to_sqlite_err)?;

    Ok(MemoryNode {
        id: row.get(0)?,
        kind,
        content: row.get(2)?,
        confidence: row.get::<_, f64>(3)? as f32,
        status,
        source,
        evidence: row.get(6)?,
        tags,
        related_to: related,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        last_accessed: row.get(11)?,
        access_count: row.get(12)?,
        expires_at: row.get(13)?,
    })
}

fn row_to_commit(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryCommit> {
    let stats_json: String = row.get(6)?;
    let stats: CommitStats = serde_json::from_str(&stats_json).map_err(serde_to_sqlite_err)?;
    Ok(MemoryCommit {
        id: row.get(0)?,
        parent: row.get(1)?,
        message: row.get(2)?,
        author: row.get(3)?,
        timestamp: row.get(4)?,
        tree_id: row.get(5)?,
        stats,
    })
}

fn to_sqlite_err(err: MemoraError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
}

fn serde_to_sqlite_err(err: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{MemoryNode, NewNode};

    fn fresh_store() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::open(tmp.path().join("memora.db")).unwrap();
        (tmp, s)
    }

    #[test]
    fn can_round_trip_a_node() {
        let (_tmp, store) = fresh_store();
        let node = MemoryNode::from_new(
            NewNode::new(MemoryKind::Project, "uses Rust", MemorySource::CodeRead),
            42,
        );
        store.upsert_node(&node).unwrap();
        let fetched = store.get_node(&node.id).unwrap().unwrap();
        assert_eq!(fetched, node);
    }

    #[test]
    fn count_by_kind_returns_all_six() {
        let (_tmp, store) = fresh_store();
        let counts = store.count_by_kind().unwrap();
        assert_eq!(counts.len(), 6);
        for (_, c) in counts {
            assert_eq!(c, 0);
        }
    }

    #[test]
    fn unstaged_reports_new_nodes_when_no_head() {
        let (_tmp, store) = fresh_store();
        let n = MemoryNode::from_new(
            NewNode::new(MemoryKind::Semantic, "hello", MemorySource::Manual),
            10,
        );
        store.upsert_node(&n).unwrap();
        let s = store.unstaged_against(None).unwrap();
        assert_eq!(s.added.len(), 1);
        assert_eq!(s.total, 1);
    }
}
