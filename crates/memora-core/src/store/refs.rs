//! On-disk ref + HEAD management.
//!
//! Branches live in `.memora/refs/heads/<name>` and contain a single
//! commit id (or are empty for a branch with no commits yet). `HEAD`
//! contains either `ref: refs/heads/<name>` (the normal case) or a raw
//! commit id (detached state, used after `memora rollback` to a specific
//! hash before the user names a new branch).
//!
//! Keeping these as plain files is intentional: the format is meant to
//! be inspectable with `cat`, just like git.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{MemoraError, Result};

/// Filesystem helper that knows how to read and write HEAD plus refs.
pub struct Refs {
    /// Path to the `.memora/` directory.
    root: PathBuf,
}

impl Refs {
    /// Construct a new ref manager rooted at the given `.memora/` directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Return the path to `.memora/HEAD`.
    pub fn head_path(&self) -> PathBuf {
        self.root.join("HEAD")
    }

    /// Return the path to a branch ref, e.g. `.memora/refs/heads/main`.
    pub fn branch_path(&self, name: &str) -> PathBuf {
        self.root.join("refs").join("heads").join(name)
    }

    /// Validate a branch name. Rules are intentionally close to git's:
    /// no whitespace, no control chars, no leading/trailing dots, no `..`.
    pub fn validate_branch_name(name: &str) -> Result<()> {
        if name.is_empty() {
            return Err(MemoraError::InvalidBranchName("(empty)".into()));
        }
        if name.starts_with('.') || name.ends_with('.') {
            return Err(MemoraError::InvalidBranchName(name.to_string()));
        }
        if name.contains("..") {
            return Err(MemoraError::InvalidBranchName(name.to_string()));
        }
        for ch in name.chars() {
            if ch.is_whitespace() || ch.is_control() {
                return Err(MemoraError::InvalidBranchName(name.to_string()));
            }
            if matches!(ch, '~' | '^' | ':' | '?' | '*' | '[' | '\\') {
                return Err(MemoraError::InvalidBranchName(name.to_string()));
            }
        }
        Ok(())
    }

    /// Initialise the on-disk ref structure. Idempotent.
    pub fn init(&self, default_branch: &str) -> Result<()> {
        Self::validate_branch_name(default_branch)?;
        fs::create_dir_all(self.root.join("refs").join("heads"))?;
        fs::create_dir_all(self.root.join("refs").join("remotes"))?;
        fs::create_dir_all(self.root.join("objects"))?;
        fs::create_dir_all(self.root.join("sessions"))?;

        // HEAD points at the default branch even if the branch file is empty
        // (no commits yet), mirroring how git behaves immediately after init.
        let head = format!("ref: refs/heads/{default_branch}\n");
        fs::write(self.head_path(), head)?;
        let branch_file = self.branch_path(default_branch);
        if !branch_file.exists() {
            fs::write(&branch_file, "")?;
        }
        Ok(())
    }

    /// Read raw HEAD contents (whatever is in the file).
    pub fn read_head_raw(&self) -> Result<String> {
        let raw = fs::read_to_string(self.head_path())?;
        Ok(raw.trim().to_string())
    }

    /// Read HEAD as a parsed [`HeadRef`].
    pub fn read_head(&self) -> Result<HeadRef> {
        let raw = self.read_head_raw()?;
        if let Some(rest) = raw.strip_prefix("ref:") {
            let r = rest.trim();
            let name = r
                .strip_prefix("refs/heads/")
                .ok_or_else(|| MemoraError::Invalid(format!("malformed HEAD ref: {raw}")))?;
            Ok(HeadRef::Branch(name.to_string()))
        } else if !raw.is_empty() {
            Ok(HeadRef::Detached(raw))
        } else {
            Err(MemoraError::Invalid("HEAD is empty".into()))
        }
    }

    /// Write HEAD as a symbolic ref to a branch.
    pub fn write_head_branch(&self, branch: &str) -> Result<()> {
        Self::validate_branch_name(branch)?;
        fs::write(self.head_path(), format!("ref: refs/heads/{branch}\n"))?;
        Ok(())
    }

    /// Write HEAD as a detached commit pointer.
    pub fn write_head_detached(&self, commit_id: &str) -> Result<()> {
        fs::write(self.head_path(), format!("{commit_id}\n"))?;
        Ok(())
    }

    /// Read the commit id a branch points at, or `None` if the branch has
    /// no commits yet.
    pub fn read_branch(&self, name: &str) -> Result<Option<String>> {
        let path = self.branch_path(name);
        if !path.exists() {
            return Err(MemoraError::RefNotFound(name.to_string()));
        }
        let raw = fs::read_to_string(path)?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            Ok(None)
        } else {
            Ok(Some(trimmed.to_string()))
        }
    }

    /// Update a branch ref to point at the given commit id.
    pub fn write_branch(&self, name: &str, commit_id: &str) -> Result<()> {
        Self::validate_branch_name(name)?;
        let path = self.branch_path(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, format!("{commit_id}\n"))?;
        Ok(())
    }

    /// Create a new branch pointing at `commit_id` (which may be `None`).
    pub fn create_branch(&self, name: &str, commit_id: Option<&str>) -> Result<()> {
        Self::validate_branch_name(name)?;
        let path = self.branch_path(name);
        if path.exists() {
            return Err(MemoraError::BranchAlreadyExists(name.to_string()));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = commit_id.map(|c| format!("{c}\n")).unwrap_or_default();
        fs::write(path, body)?;
        Ok(())
    }

    /// List all local branches in lexical order.
    pub fn list_branches(&self) -> Result<Vec<String>> {
        let dir = self.root.join("refs").join("heads");
        let mut out = Vec::new();
        if !dir.exists() {
            return Ok(out);
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            collect_branches(&dir, &entry.path(), &mut out)?;
        }
        out.sort();
        Ok(out)
    }
}

fn collect_branches(root: &Path, path: &Path, out: &mut Vec<String>) -> Result<()> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            collect_branches(root, &entry.path(), out)?;
        }
    } else if path.is_file() {
        if let Ok(rel) = path.strip_prefix(root) {
            let name = rel.to_string_lossy().replace('\\', "/");
            out.push(name);
        }
    }
    Ok(())
}

/// What HEAD currently points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadRef {
    /// Normal case: HEAD is a symbolic ref to a branch by name.
    Branch(String),
    /// Detached: HEAD points directly at a commit id.
    Detached(String),
}

impl HeadRef {
    /// If this is a branch ref, return the branch name.
    pub fn branch(&self) -> Option<&str> {
        if let HeadRef::Branch(name) = self {
            Some(name)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn refs_in(dir: &std::path::Path) -> Refs {
        Refs::new(dir.join(".memora"))
    }

    #[test]
    fn validates_branch_names() {
        assert!(Refs::validate_branch_name("main").is_ok());
        assert!(Refs::validate_branch_name("feature/x").is_ok());
        assert!(Refs::validate_branch_name("").is_err());
        assert!(Refs::validate_branch_name("bad name").is_err());
        assert!(Refs::validate_branch_name(".hidden").is_err());
        assert!(Refs::validate_branch_name("a..b").is_err());
        assert!(Refs::validate_branch_name("with:colon").is_err());
    }

    #[test]
    fn init_creates_layout_and_head() {
        let tmp = tempdir().unwrap();
        let refs = refs_in(tmp.path());
        refs.init("main").unwrap();
        assert!(tmp.path().join(".memora/HEAD").exists());
        assert!(tmp.path().join(".memora/refs/heads/main").exists());
        assert_eq!(refs.read_head().unwrap(), HeadRef::Branch("main".into()));
        assert_eq!(refs.read_branch("main").unwrap(), None);
    }

    #[test]
    fn create_and_list_branches() {
        let tmp = tempdir().unwrap();
        let refs = refs_in(tmp.path());
        refs.init("main").unwrap();
        refs.create_branch("dev", Some("abcdef")).unwrap();
        refs.create_branch("feature/x", None).unwrap();
        let mut branches = refs.list_branches().unwrap();
        branches.sort();
        assert_eq!(branches, vec!["dev", "feature/x", "main"]);
        assert_eq!(refs.read_branch("dev").unwrap().as_deref(), Some("abcdef"));
    }

    #[test]
    fn cannot_recreate_branch() {
        let tmp = tempdir().unwrap();
        let refs = refs_in(tmp.path());
        refs.init("main").unwrap();
        assert!(refs.create_branch("main", None).is_err());
    }
}
