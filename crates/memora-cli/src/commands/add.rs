//! `memora add` — record a new typed memory node.

use std::env;
use std::str::FromStr;

use anyhow::{Context, Result};
use owo_colors::OwoColorize;

use memora_core::node::{MemoryKind, MemorySource, NewNode};
use memora_core::Repository;

use crate::cli::AddArgs;
use crate::ui::short_id;

/// Entry point for the `add` subcommand.
pub fn run(args: AddArgs) -> Result<()> {
    let kind = MemoryKind::from_str(&args.kind)
        .with_context(|| format!("invalid --type value '{}'", args.kind))?;
    let source = MemorySource::from_str(&args.source)
        .with_context(|| format!("invalid --source value '{}'", args.source))?;

    if let Some(c) = args.confidence {
        anyhow::ensure!(
            (0.0..=1.0).contains(&c),
            "--confidence must be between 0.0 and 1.0 (got {c})"
        );
    }

    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;
    let node = repo.add_node(NewNode {
        kind,
        content: args.content,
        confidence: args.confidence,
        status: None,
        source,
        evidence: args.evidence,
        tags: args.tags,
        related_to: Vec::new(),
        expires_at: None,
    })?;

    println!(
        "{} [{}] node {} (confidence {:.2}, status {})",
        "Added".green().bold(),
        node.kind,
        short_id(&node.id).bold(),
        node.confidence,
        node.status,
    );
    Ok(())
}
