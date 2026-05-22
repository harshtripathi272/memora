//! `memora diff` — show belief changes between two commits or vs working set.

use std::env;

use anyhow::Result;

use memora_core::{NodeChange, Repository};

use crate::cli::DiffArgs;
use crate::ui::{bold, cyan, dim, green, red, short_id, yellow};

/// Entry point for the `diff` subcommand.
pub fn run(args: DiffArgs) -> Result<()> {
    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;

    let to = if args.working { None } else { Some(args.to.as_str()) };
    let report = repo.diff(&args.from, to)?;

    let to_label = if to.is_none() {
        dim("(working set)")
    } else {
        yellow(short_id(&report.to_label))
    };
    println!(
        "{} {} → {}",
        bold("diff"),
        yellow(short_id(&report.from_id)),
        to_label,
    );

    if report.is_empty() {
        println!("{}", dim("no belief changes between the two states"));
        return Ok(());
    }

    if !report.added.is_empty() {
        println!("\n{}", bold(green("Added:")));
        for n in &report.added {
            println!(
                "  + {} [{}] {}",
                dim(short_id(&n.id)),
                bold(n.kind.to_string()),
                truncate(&n.content, 96),
            );
        }
    }

    if !report.modified.is_empty() {
        println!("\n{}", bold(cyan("Changed:")));
        for m in &report.modified {
            println!(
                "  ~ {} [{}] {}",
                dim(short_id(&m.after.id)),
                bold(m.after.kind.to_string()),
                truncate(&m.after.content, 96),
            );
            for ch in &m.changes {
                let label = match ch {
                    NodeChange::Status { from, to } => format!("status: {from} → {to}"),
                    NodeChange::Content => "content updated".to_string(),
                    NodeChange::Confidence { from, to } => {
                        format!("confidence: {from:.2} → {to:.2}")
                    }
                    NodeChange::Source => "source updated".to_string(),
                    NodeChange::Evidence => "evidence updated".to_string(),
                };
                println!("      {}", dim(&label));
            }
        }
    }

    if !report.removed.is_empty() {
        println!("\n{}", bold(red("Removed:")));
        for n in &report.removed {
            println!(
                "  - {} [{}] {}",
                dim(short_id(&n.id)),
                bold(n.kind.to_string()),
                truncate(&n.content, 96),
            );
        }
    }

    if args.semantic {
        let lines = report.semantic_lines();
        if !lines.is_empty() {
            println!("\n{}", bold("Semantic summary:"));
            for l in lines {
                println!("  {l}");
            }
        }
    }
    Ok(())
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
