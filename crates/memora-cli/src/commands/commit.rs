//! `memora commit` — snapshot the current working set.

use std::env;

use anyhow::Result;
use owo_colors::OwoColorize;

use memora_core::Repository;

use crate::cli::CommitArgs;
use crate::ui::short_id;

/// Entry point for the `commit` subcommand.
pub fn run(args: CommitArgs) -> Result<()> {
    anyhow::ensure!(!args.message.trim().is_empty(), "commit message is empty");
    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;
    let outcome = repo.commit(&args.message, &args.author)?;

    match outcome.commit {
        None => {
            println!(
                "{} no changes since the last commit.",
                "Nothing to commit:".yellow().bold()
            );
        }
        Some(c) => {
            let branch = outcome.branch.as_deref().unwrap_or("(detached)");
            println!(
                "[{} {}] {}",
                branch.bold(),
                short_id(&c.id).yellow(),
                c.message
            );
            println!(
                "  +{} added · ~{} modified · -{} removed · {} promoted · {} conflicted",
                c.stats.added.green(),
                c.stats.modified.cyan(),
                c.stats.removed.red(),
                c.stats.promoted.green(),
                c.stats.conflicted.yellow(),
            );
        }
    }
    Ok(())
}
