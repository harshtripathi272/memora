//! Round-trip helpers for `.memora/config`.
//!
//! The on-disk format is plain TOML. Phase 1 hand-wrote the file once at
//! `init` and never touched it again; Phase 5 needs to add and remove
//! `[remote.<name>]` sections, so we now load → mutate → dump through a
//! typed `Config` struct.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{MemoraError, Result};
use crate::FORMAT_VERSION;

/// Top-level config layout.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Core fields.
    #[serde(default)]
    pub core: CoreConfig,
    /// Default author identity.
    #[serde(default)]
    pub author: AuthorConfig,
    /// Named remotes. Keyed by remote name.
    #[serde(default)]
    pub remote: BTreeMap<String, RemoteConfig>,
}

/// `[core]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreConfig {
    /// Format version this store was written with.
    pub format_version: u32,
    /// Default branch name created at init.
    pub default_branch: String,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            default_branch: crate::DEFAULT_BRANCH.to_string(),
        }
    }
}

/// `[author]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorConfig {
    /// Default author name attached to new commits.
    pub name: String,
}

impl Default for AuthorConfig {
    fn default() -> Self {
        Self {
            name: "human".to_string(),
        }
    }
}

/// One `[remote.<name>]` section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteConfig {
    /// URL of the remote. For Phase 5 this is always a filesystem path
    /// pointing at another `.memora/`-bearing project.
    pub url: String,
}

impl Config {
    /// Load `<memora_dir>/config`. If the file is missing, return defaults.
    pub fn load(memora_dir: &Path) -> Result<Self> {
        let path = memora_dir.join("config");
        if !path.exists() {
            return Ok(Config::default());
        }
        let raw = fs::read_to_string(&path)?;
        toml::from_str(&raw)
            .map_err(|e| MemoraError::Invalid(format!("malformed .memora/config: {e}")))
    }

    /// Persist to `<memora_dir>/config`.
    pub fn save(&self, memora_dir: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self)
            .map_err(|e| MemoraError::Invalid(format!("failed to serialise config: {e}")))?;
        let header = format!("# memora config (format v{})\n", self.core.format_version);
        fs::write(memora_dir.join("config"), format!("{header}{body}"))?;
        Ok(())
    }

    /// Convenience: get a remote by name.
    pub fn remote(&self, name: &str) -> Option<&RemoteConfig> {
        self.remote.get(name)
    }

    /// Add a remote, overwriting any existing entry of the same name.
    pub fn set_remote(&mut self, name: &str, url: impl Into<String>) {
        self.remote
            .insert(name.to_string(), RemoteConfig { url: url.into() });
    }

    /// Remove a remote. Returns whether anything was removed.
    pub fn remove_remote(&mut self, name: &str) -> bool {
        self.remote.remove(name).is_some()
    }
}

/// Validate a remote URL string. Phase 5 only accepts filesystem paths
/// (absolute, or relative to `cwd`); we keep the validator separate so
/// future schemes (`https://`, `git@`) plug in cleanly.
pub fn validate_remote_url(url: &str, cwd: &Path) -> Result<PathBuf> {
    if url.is_empty() {
        return Err(MemoraError::Invalid("empty remote url".into()));
    }
    let p = Path::new(url);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    };
    if !abs.exists() {
        return Err(MemoraError::Invalid(format!(
            "remote path does not exist: {}",
            abs.display()
        )));
    }
    Ok(abs)
}
