//! `memora remote` — add / list / remove remotes.

use std::env;

use anyhow::Result;

use memora_core::Repository;

use crate::cli::{RemoteArgs, RemoteCommand};
use crate::ui::{bold, dim, green, yellow};

/// Entry point for the `remote` subcommand.
pub fn run(args: RemoteArgs) -> Result<()> {
    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;
    match args.command {
        RemoteCommand::Add { name, url } => add(&repo, &name, &url),
        RemoteCommand::List => list(&repo),
        RemoteCommand::Remove { name } => remove(&repo, &name),
    }
}

fn add(repo: &Repository, name: &str, url: &str) -> Result<()> {
    repo.add_remote(name, url)?;
    println!(
        "{} remote {} → {}",
        bold(green("Added")),
        bold(name),
        url
    );
    Ok(())
}

fn list(repo: &Repository) -> Result<()> {
    let remotes = repo.list_remotes()?;
    if remotes.is_empty() {
        println!("{}", dim("no remotes configured."));
        return Ok(());
    }
    for (name, cfg) in remotes {
        println!("{} {}", bold(&name), cfg.url);
    }
    Ok(())
}

fn remove(repo: &Repository, name: &str) -> Result<()> {
    if repo.remove_remote(name)? {
        println!("{} remote {}", bold(yellow("Removed")), bold(name));
    } else {
        println!("{}", dim("nothing to remove."));
    }
    Ok(())
}
