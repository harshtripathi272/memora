//! `memora branch` — list / create branches.

use std::env;

use anyhow::Result;

use memora_core::Repository;

use crate::cli::BranchArgs;
use crate::ui::{bold, dim, green};

/// Entry point for the `branch` subcommand.
pub fn run(args: BranchArgs) -> Result<()> {
    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;

    // No name + --list (or no flags at all) → list branches.
    if args.name.is_none() {
        let head = repo.head()?;
        let current = head.branch();
        let branches = repo.list_branches()?;
        if branches.is_empty() {
            println!("{}", dim("no branches"));
            return Ok(());
        }
        for b in branches {
            if Some(b.as_str()) == current {
                println!("* {}", bold(green(&b)));
            } else {
                println!("  {b}");
            }
        }
        return Ok(());
    }

    // Otherwise create a new branch from HEAD.
    let name = args.name.unwrap();
    repo.create_branch(&name)?;
    println!("{} branch {}", bold(green("Created")), bold(&name));
    Ok(())
}
