//! `memora rollback` — move HEAD back to a previous commit.
//!
//! Always takes a checkpoint commit of the current state first so the
//! action is reversible.

use std::env;

use anyhow::Result;
use owo_colors::OwoColorize;

use memora_core::Repository;

use crate::cli::RollbackArgs;
use crate::ui::short_id;

/// Entry point for the `rollback` subcommand.
pub fn run(args: RollbackArgs) -> Result<()> {
    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;

    let target_id = repo.store().resolve_commit_prefix(&args.to)?;
    let target = repo.rollback_to(&target_id, &args.author)?;
    println!(
        "{} HEAD → {} ({})",
        "Rolled back".yellow().bold(),
        short_id(&target.id).yellow(),
        target.message
    );
    println!(
        "{}",
        "  (a pre-rollback checkpoint commit was recorded if there were uncommitted changes)"
            .dimmed()
    );
    Ok(())
}
