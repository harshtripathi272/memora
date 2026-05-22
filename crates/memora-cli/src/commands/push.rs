//! `memora push` — copy a local branch's commits to a configured remote.

use std::env;

use anyhow::Result;

use memora_core::Repository;

use crate::cli::PushArgs;
use crate::ui::{bold, dim, green, short_id};

/// Entry point for the `push` subcommand.
pub fn run(args: PushArgs) -> Result<()> {
    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;
    let branch = match args.branch {
        Some(b) => b,
        None => repo
            .head()?
            .branch()
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("HEAD is detached; specify a branch explicitly"))?,
    };

    let outcome = repo.push(&args.remote, &branch)?;
    if outcome.rejected_non_fast_forward {
        anyhow::bail!(
            "rejected non-fast-forward push: remote '{}' already has commits not in the local branch '{}'. Pull first, merge, then push.",
            args.remote,
            branch
        );
    }
    if outcome.already_synced {
        println!(
            "{} {} already in sync.",
            bold(green("up-to-date")),
            dim(format!("{}/{}", args.remote, branch))
        );
        return Ok(());
    }
    let tip = outcome
        .new_tip
        .as_deref()
        .map(short_id)
        .unwrap_or("?")
        .to_string();
    println!(
        "{} {} commit{} → {} (tip {})",
        bold(green("Pushed")),
        outcome.commits_copied,
        if outcome.commits_copied == 1 { "" } else { "s" },
        bold(format!("{}/{}", args.remote, branch)),
        bold(tip),
    );
    Ok(())
}
