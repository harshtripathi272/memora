//! `memora init` — create a new memora store in the current (or given) directory.

use std::env;

use anyhow::Result;

use memora_core::Repository;

use crate::cli::InitArgs;
use crate::ui::{bold, dim, green};

/// Entry point for the `init` subcommand.
pub fn run(args: InitArgs) -> Result<()> {
    let target = match args.path {
        Some(p) => p,
        None => env::current_dir()?,
    };
    let repo = Repository::init(&target)?;
    println!(
        "{} memora store at {}",
        bold(green("Initialised")),
        repo.memora_dir().display()
    );
    println!("HEAD now points at branch {}.", bold("main"));
    println!("Next steps:");
    println!("  {}", dim("memora add --type=semantic --content=\"...\""));
    println!("  {}", dim("memora commit -m \"first memory\""));
    Ok(())
}
