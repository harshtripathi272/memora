//! `memora status` — show what has changed since HEAD.

use std::collections::BTreeMap;
use std::env;

use anyhow::Result;
use owo_colors::OwoColorize;

use memora_core::node::MemoryKind;
use memora_core::Repository;

use crate::ui::short_id;

/// Entry point for the `status` subcommand.
pub fn run() -> Result<()> {
    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;
    let head = repo.head()?;
    let summary = repo.status()?;

    match head.branch() {
        Some(b) => println!("On branch {}", b.bold()),
        None => println!("HEAD is detached"),
    }
    println!("{} nodes in working set", summary.total);

    if summary.added.is_empty() && summary.modified.is_empty() && summary.removed.is_empty() {
        println!("{}", "Nothing to commit; working set matches HEAD.".dimmed());
        return Ok(());
    }

    if !summary.added.is_empty() {
        println!("\n{}", "Added since HEAD:".green().bold());
        let by_kind = group_by_kind(&summary.added);
        for (kind, nodes) in by_kind {
            println!("  {}", kind.to_string().bold());
            for n in nodes {
                println!(
                    "    {} {}  {}",
                    short_id(&n.id).dimmed(),
                    format!("[{}]", n.status).dimmed(),
                    truncate(&n.content, 80)
                );
            }
        }
    }

    if !summary.modified.is_empty() {
        println!("\n{}", "Modified since HEAD:".cyan().bold());
        for n in &summary.modified {
            println!(
                "  {} {} {}",
                short_id(&n.id).dimmed(),
                format!("[{}]", n.kind).dimmed(),
                truncate(&n.content, 80)
            );
        }
    }

    if !summary.removed.is_empty() {
        println!("\n{}", "Removed since HEAD:".red().bold());
        for id in &summary.removed {
            println!("  {}", short_id(id).red());
        }
    }
    Ok(())
}

fn group_by_kind(
    nodes: &[memora_core::MemoryNode],
) -> BTreeMap<MemoryKind, Vec<&memora_core::MemoryNode>> {
    let mut map: BTreeMap<MemoryKind, Vec<&memora_core::MemoryNode>> = BTreeMap::new();
    for n in nodes {
        map.entry(n.kind).or_default().push(n);
    }
    map
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}
