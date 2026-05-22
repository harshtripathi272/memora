//! `memora rollback` — move HEAD back to a previous commit.
//!
//! Always takes a checkpoint commit of the current state first so the
//! action is reversible.

use std::env;

use anyhow::Result;

use memora_core::Repository;

use crate::cli::RollbackArgs;
use crate::ui::{bold, dim, short_id, yellow};

/// Entry point for the `rollback` subcommand.
pub fn run(args: RollbackArgs) -> Result<()> {
    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;

    let target_id = repo.store().resolve_commit_prefix(&args.to)?;
    let target = repo.rollback_to(&target_id, &args.author)?;
    println!(
        "{} HEAD → {} ({})",
        bold(yellow("Rolled back")),
        yellow(short_id(&target.id)),
        target.message
    );
    println!(
        "{}",
        dim("  (a pre-rollback checkpoint commit was recorded if there were uncommitted changes)")
    );
    Ok(())
}
