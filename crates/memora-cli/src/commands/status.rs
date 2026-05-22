//! `memora status` — show what has changed since HEAD.

use std::collections::BTreeMap;
use std::env;

use anyhow::Result;

use memora_core::node::MemoryKind;
use memora_core::Repository;

use crate::ui::{bold, cyan, dim, green, red, short_id};

/// Entry point for the `status` subcommand.
pub fn run() -> Result<()> {
    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;
    let head = repo.head()?;
    let summary = repo.status()?;

    match head.branch() {
        Some(b) => println!("On branch {}", bold(b)),
        None => println!("HEAD is detached"),
    }
    println!("{} nodes in working set", summary.total);

    if summary.added.is_empty() && summary.modified.is_empty() && summary.removed.is_empty() {
        println!("{}", dim("Nothing to commit; working set matches HEAD."));
        return Ok(());
    }

    if !summary.added.is_empty() {
        println!("\n{}", bold(green("Added since HEAD:")));
        let by_kind = group_by_kind(&summary.added);
        for (kind, nodes) in by_kind {
            println!("  {}", bold(kind.to_string()));
            for n in nodes {
                println!(
                    "    {} {}  {}",
                    dim(short_id(&n.id)),
                    dim(format!("[{}]", n.status)),
                    truncate(&n.content, 80)
                );
            }
        }
    }

    if !summary.modified.is_empty() {
        println!("\n{}", bold(cyan("Modified since HEAD:")));
        for n in &summary.modified {
            println!(
                "  {} {} {}",
                dim(short_id(&n.id)),
                dim(format!("[{}]", n.kind)),
                truncate(&n.content, 80)
            );
        }
    }

    if !summary.removed.is_empty() {
        println!("\n{}", bold(red("Removed since HEAD:")));
        for id in &summary.removed {
            println!("  {}", red(short_id(id)));
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
