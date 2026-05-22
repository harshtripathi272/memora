//! `memora commit` — snapshot the current working set.

use std::env;

use anyhow::Result;

use memora_core::Repository;

use crate::cli::CommitArgs;
use crate::ui::{bold, cyan, green, red, short_id, yellow};

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
                bold(yellow("Nothing to commit:"))
            );
        }
        Some(c) => {
            let branch = outcome.branch.as_deref().unwrap_or("(detached)");
            println!(
                "[{} {}] {}",
                bold(branch),
                yellow(short_id(&c.id)),
                c.message
            );
            println!(
                "  +{} added · ~{} modified · -{} removed · {} promoted · {} conflicted",
                green(c.stats.added.to_string()),
                cyan(c.stats.modified.to_string()),
                red(c.stats.removed.to_string()),
                green(c.stats.promoted.to_string()),
                yellow(c.stats.conflicted.to_string()),
            );
        }
    }
    Ok(())
}
