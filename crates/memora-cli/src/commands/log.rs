//! `memora log` — show commit history newest-first.

use std::env;

use anyhow::Result;

use memora_core::Repository;

use crate::cli::LogArgs;
use crate::ui::{dim, fmt_timestamp, short_id, yellow};

/// Entry point for the `log` subcommand.
pub fn run(args: LogArgs) -> Result<()> {
    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;
    let commits = repo.log(args.limit)?;
    if commits.is_empty() {
        println!("{}", dim("no commits yet"));
        return Ok(());
    }
    for c in commits {
        if args.oneline {
            println!("{} {}", yellow(short_id(&c.id)), c.message);
        } else {
            println!("{} {}", yellow("commit"), yellow(&c.id));
            if let Some(p) = &c.parent {
                println!("Parent:    {}", short_id(p));
            }
            println!("Author:    {}", c.author);
            println!("Date:      {}", fmt_timestamp(c.timestamp));
            println!("Tree:      {}", short_id(&c.tree_id));
            println!(
                "Stats:     +{} ~{} -{}  promoted {}  conflicted {}",
                c.stats.added,
                c.stats.modified,
                c.stats.removed,
                c.stats.promoted,
                c.stats.conflicted,
            );
            println!();
            println!("    {}", c.message);
            println!();
        }
    }
    Ok(())
}
