//! `memora switch` — move HEAD to an existing branch.

use std::env;

use anyhow::Result;
use owo_colors::OwoColorize;

use memora_core::Repository;

use crate::cli::SwitchArgs;

/// Entry point for the `switch` subcommand.
pub fn run(args: SwitchArgs) -> Result<()> {
    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;
    repo.switch_branch(&args.name)?;
    println!("Switched to branch {}", args.name.bold());
    Ok(())
}
