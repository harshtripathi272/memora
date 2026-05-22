//! Command-line argument parsing.
//!
//! Defining the full surface up-front (with comments) keeps `--help`
//! self-documenting and gives us a single place to evolve the CLI.

use clap::{Parser, Subcommand};

/// Top-level CLI.
#[derive(Debug, Parser)]
#[command(
    name = "memora",
    version,
    about = "The memory layer for AI agents — versioned, typed, portable.",
    long_about = "memora version-controls, types, and tracks provenance on the \
                  memory of your AI coding agents. Run `memora init` in a project, \
                  then `memora add` and `memora commit` to capture beliefs as they \
                  evolve."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level subcommand.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a new memora store in the current directory.
    Init(InitArgs),

    /// Add a new typed memory node to the working set.
    Add(AddArgs),

    /// Snapshot the current working set as a commit on the active branch.
    Commit(CommitArgs),

    /// Show what has changed since the last commit.
    Status,

    /// List commits, newest first.
    Log(LogArgs),

    /// Manage branches (list / create).
    Branch(BranchArgs),

    /// Switch HEAD to an existing branch.
    Switch(SwitchArgs),

    /// Roll HEAD back to a specified commit, with auto checkpoint.
    Rollback(RollbackArgs),

    /// Promote ephemeral nodes to stable.
    Promote(PromoteArgs),

    /// Show what changed between two commits (or commit vs working set).
    Diff(DiffArgs),

    /// Merge another branch (or commit) into HEAD.
    Merge(MergeArgs),
}

/// Arguments for `memora init`.
#[derive(Debug, clap::Args)]
pub struct InitArgs {
    /// Initialise the store at this directory instead of the current one.
    #[arg(value_name = "DIR")]
    pub path: Option<std::path::PathBuf>,
}

/// Arguments for `memora add`.
#[derive(Debug, clap::Args)]
pub struct AddArgs {
    /// Memory category. One of: episodic, semantic, procedural, assumption,
    /// project, preference.
    #[arg(long = "type", short = 't', value_name = "KIND")]
    pub kind: String,

    /// The memory content to record.
    #[arg(long, short = 'c', value_name = "TEXT")]
    pub content: String,

    /// Optional confidence in the range `[0.0, 1.0]`. Defaults to a
    /// source-specific prior (e.g. 1.0 for `code-read`, 0.6 for
    /// `model-inference`).
    #[arg(long)]
    pub confidence: Option<f32>,

    /// Provenance: where did this belief come from? Examples:
    /// `claude-code`, `cursor`, `code-read`, `test-result`,
    /// `model-inference`, `manual`.
    #[arg(long, default_value = "manual")]
    pub source: String,

    /// Optional evidence pointer, e.g. `src/auth/jwt.rs:L42`.
    #[arg(long)]
    pub evidence: Option<String>,

    /// Repeatable tag.
    #[arg(long = "tag", value_name = "TAG")]
    pub tags: Vec<String>,
}

/// Arguments for `memora commit`.
#[derive(Debug, clap::Args)]
pub struct CommitArgs {
    /// Commit message.
    #[arg(short = 'm', long, value_name = "MESSAGE")]
    pub message: String,

    /// Override the author. Defaults to `human`.
    #[arg(long, default_value = "human")]
    pub author: String,
}

/// Arguments for `memora log`.
#[derive(Debug, clap::Args)]
pub struct LogArgs {
    /// One commit per line, git-style oneline output.
    #[arg(long)]
    pub oneline: bool,

    /// Limit the number of commits shown.
    #[arg(long, short = 'n')]
    pub limit: Option<usize>,
}

/// Arguments for `memora branch`.
#[derive(Debug, clap::Args)]
pub struct BranchArgs {
    /// List all branches (default when no name is given).
    #[arg(short = 'l', long)]
    pub list: bool,

    /// New branch name. When provided, creates the branch.
    #[arg(value_name = "NAME")]
    pub name: Option<String>,
}

/// Arguments for `memora switch`.
#[derive(Debug, clap::Args)]
pub struct SwitchArgs {
    /// Branch to switch HEAD to.
    #[arg(value_name = "BRANCH")]
    pub name: String,
}

/// Arguments for `memora rollback`.
#[derive(Debug, clap::Args)]
pub struct RollbackArgs {
    /// Commit id to roll HEAD back to. A short id (>=4 chars) is accepted.
    #[arg(long = "to", value_name = "COMMIT")]
    pub to: String,

    /// Author name to attach to the auto-checkpoint commit.
    #[arg(long, default_value = "human")]
    pub author: String,
}

/// Arguments for `memora promote`.
///
/// Exactly one of `--id`, `--type`, or `--all-confirmed` must be provided.
#[derive(Debug, clap::Args)]
pub struct PromoteArgs {
    /// Promote a single node by id (full or short hex).
    #[arg(long = "id", value_name = "NODE")]
    pub id: Option<String>,

    /// Promote every ephemeral node of the given memory type.
    #[arg(long = "type", value_name = "KIND", conflicts_with = "id")]
    pub kind: Option<String>,

    /// Promote every ephemeral node whose confidence is >= the threshold
    /// (default 0.8 when the flag is given without a value).
    #[arg(
        long = "all-confirmed",
        value_name = "THRESHOLD",
        num_args = 0..=1,
        default_missing_value = "0.8",
        conflicts_with_all = ["id", "kind"],
    )]
    pub all_confirmed: Option<f32>,
}

/// Arguments for `memora diff`.
#[derive(Debug, clap::Args)]
pub struct DiffArgs {
    /// `from` revision (commit, branch, HEAD, HEAD~N). Defaults to HEAD~1.
    #[arg(value_name = "FROM", default_value = "HEAD~1")]
    pub from: String,

    /// `to` revision. Defaults to HEAD; pass `--working` to compare
    /// against the uncommitted working set instead.
    #[arg(value_name = "TO", default_value = "HEAD")]
    pub to: String,

    /// Compare `from` to the current uncommitted working set rather than
    /// to a second commit.
    #[arg(long)]
    pub working: bool,

    /// Include a natural-language summary of belief changes.
    #[arg(long)]
    pub semantic: bool,
}

/// Arguments for `memora merge`.
#[derive(Debug, clap::Args)]
pub struct MergeArgs {
    /// Branch (or commit) whose memory to merge into HEAD.
    #[arg(value_name = "BRANCH")]
    pub branch: String,

    /// Strategy for resolving same-id divergences.
    #[arg(long, value_enum, default_value_t = MergeStrategyArg::Auto)]
    pub strategy: MergeStrategyArg,

    /// Disable fast-forward; always create a merge commit.
    #[arg(long = "no-ff")]
    pub no_ff: bool,

    /// Apply the merge to the working set without committing.
    #[arg(long = "no-commit")]
    pub no_commit: bool,

    /// Override the auto-generated merge commit message.
    #[arg(short = 'm', long, value_name = "MESSAGE")]
    pub message: Option<String>,

    /// Plan only — print what would happen and exit without changing anything.
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Author for the merge commit.
    #[arg(long, default_value = "human")]
    pub author: String,
}

/// Convenience clap enum mirroring [`memora_core::MergeStrategy`].
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum MergeStrategyArg {
    /// Score the two sides; mark genuine ties as conflicts.
    Auto,
    /// On any divergence, keep `ours`.
    Ours,
    /// On any divergence, keep `theirs`.
    Theirs,
}

impl From<MergeStrategyArg> for memora_core::MergeStrategy {
    fn from(v: MergeStrategyArg) -> Self {
        match v {
            MergeStrategyArg::Auto => memora_core::MergeStrategy::Auto,
            MergeStrategyArg::Ours => memora_core::MergeStrategy::Ours,
            MergeStrategyArg::Theirs => memora_core::MergeStrategy::Theirs,
        }
    }
}
