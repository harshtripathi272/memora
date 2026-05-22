//! `memora gc` — importance-scored garbage collection of the working set.

use std::env;

use anyhow::Result;

use memora_core::{GcAction, GcOptions, ImportanceWeights, Repository};

use crate::cli::GcArgs;
use crate::ui::{bold, dim, green, red, short_id, yellow};

/// Entry point for the `gc` subcommand.
pub fn run(args: GcArgs) -> Result<()> {
    anyhow::ensure!(
        (0.0..=1.0).contains(&args.threshold),
        "--threshold must be in [0.0, 1.0] (got {})",
        args.threshold
    );

    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;
    let report = repo.gc(GcOptions {
        threshold: args.threshold,
        weights: ImportanceWeights::default(),
        aggressive: args.aggressive,
        dry_run: args.dry_run,
    })?;

    let title = if args.dry_run {
        bold(yellow("gc — dry run"))
    } else {
        bold(green("gc"))
    };
    println!(
        "{} threshold={:.2}{}",
        title,
        report.threshold,
        if report.aggressive {
            dim(" --aggressive").to_string()
        } else {
            String::new()
        }
    );
    println!(
        "  {} swept · {} marked · {} kept",
        report.swept(),
        report.marked(),
        report.kept(),
    );

    let mut sweeps = Vec::new();
    let mut marks = Vec::new();
    for action in &report.actions {
        match action {
            GcAction::Sweep { node } => sweeps.push(node),
            GcAction::Mark { node, score } => marks.push((node, *score)),
            GcAction::Keep { .. } => {}
        }
    }
    if !sweeps.is_empty() {
        println!("\n{}", bold(red("removed:")));
        for n in sweeps {
            println!(
                "  {} [{}] {}",
                dim(short_id(&n.id)),
                n.kind,
                truncate(&n.content, 80)
            );
        }
    }
    if !marks.is_empty() {
        println!("\n{}", bold(yellow("marked deprecated:")));
        for (n, score) in marks {
            println!(
                "  {} [{}] (score {:.2}) {}",
                dim(short_id(&n.id)),
                n.kind,
                score,
                truncate(&n.content, 80)
            );
        }
    }
    if args.dry_run && (report.marked() > 0 || report.swept() > 0) {
        println!(
            "\n{}",
            dim("re-run without --dry-run to apply these changes.")
        );
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
