//! `memora pull` — copy a remote branch's commits into the local store.

use std::env;

use anyhow::Result;

use memora_core::Repository;

use crate::cli::PullArgs;
use crate::ui::{bold, dim, green, short_id};

/// Entry point for the `pull` subcommand.
pub fn run(args: PullArgs) -> Result<()> {
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

    let outcome = repo.pull(&args.remote, &branch)?;
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
        "{} {} commit{} from {} (tip {})",
        bold(green("Pulled")),
        outcome.commits_copied,
        if outcome.commits_copied == 1 { "" } else { "s" },
        bold(format!("{}/{}", args.remote, branch)),
        bold(tip),
    );
    println!(
        "{}",
        dim(format!(
            "  next: `memora merge {}/{}` to fold the remote tip into your branch.",
            args.remote, branch
        ))
    );
    Ok(())
}
