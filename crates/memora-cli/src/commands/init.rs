//! `memora init` — create a new memora store in the current (or given) directory.

use std::env;

use anyhow::Result;
use owo_colors::OwoColorize;

use memora_core::Repository;

use crate::cli::InitArgs;

/// Entry point for the `init` subcommand.
pub fn run(args: InitArgs) -> Result<()> {
    let target = match args.path {
        Some(p) => p,
        None => env::current_dir()?,
    };
    let repo = Repository::init(&target)?;
    println!(
        "{} memora store at {}",
        "Initialised".green().bold(),
        repo.memora_dir().display()
    );
    println!("HEAD now points at branch {}.", "main".bold());
    println!("Next steps:");
    println!("  {}", "memora add --type=semantic --content=\"...\"".dimmed());
    println!("  {}", "memora commit -m \"first memory\"".dimmed());
    Ok(())
}
