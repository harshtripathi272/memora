//! `memora merge` — three-way merge another branch into HEAD.

use std::env;

use anyhow::Result;

use memora_core::{MergeKind, MergeOptions, MergeStrategy, NodeDecision, Repository};

use crate::cli::MergeArgs;
use crate::ui::{bold, cyan, dim, green, red, short_id, yellow};

/// Entry point for the `merge` subcommand.
pub fn run(args: MergeArgs) -> Result<()> {
    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;
    let strategy: MergeStrategy = args.strategy.into();

    if args.dry_run {
        let plan = repo.plan_merge(&args.branch, strategy)?;
        print_plan_header(&plan, args.no_ff);
        print_plan_body(&plan);
        return Ok(());
    }

    let opts = MergeOptions {
        strategy,
        allow_fast_forward: !args.no_ff,
        commit: !args.no_commit,
        message: args.message,
        author: args.author,
    };
    let outcome = repo.merge(&args.branch, opts)?;

    match outcome.kind {
        MergeKind::AlreadyUpToDate => {
            println!("{}", dim("Already up to date."));
        }
        MergeKind::FastForward => {
            let target = outcome
                .commit
                .as_ref()
                .map(|c| short_id(&c.id).to_string())
                .unwrap_or_else(|| "(unknown)".to_string());
            println!(
                "{} {} → {}",
                bold(green("Fast-forwarded")),
                short_id(&outcome.plan.ours),
                yellow(target),
            );
        }
        MergeKind::Merged => {
            println!(
                "{} {} into {}",
                bold(green("Merged")),
                short_id(&outcome.plan.theirs),
                short_id(&outcome.plan.ours),
            );
            if let Some(c) = &outcome.commit {
                println!("  merge commit: {}", yellow(short_id(&c.id)));
            }
            print_plan_body(&outcome.plan);
        }
        MergeKind::Conflicts => {
            println!(
                "{} merge completed with conflicts",
                bold(red("Conflicts:"))
            );
            if let Some(c) = &outcome.commit {
                println!("  merge commit: {}", yellow(short_id(&c.id)));
            }
            print_plan_body(&outcome.plan);
            println!(
                "{}",
                dim("conflicted nodes were marked Conflicted in the working set; resolve manually then commit.")
            );
        }
        MergeKind::NoCommit => {
            println!("{}", bold(yellow("Plan applied to working set, no commit created.")));
            print_plan_body(&outcome.plan);
        }
    }

    Ok(())
}

fn print_plan_header(plan: &memora_core::MergePlan, no_ff: bool) {
    println!(
        "{} {} ← {}",
        bold("merge plan"),
        short_id(&plan.ours),
        short_id(&plan.theirs),
    );
    if let Some(base) = &plan.base {
        println!("  base: {}", short_id(base));
    } else {
        println!("  base: {}", dim("(unrelated histories)"));
    }
    if plan.already_up_to_date {
        println!("{}", dim("  → already up to date"));
    } else if plan.can_fast_forward && !no_ff {
        println!("{}", dim("  → fast-forward possible"));
    }
}

fn print_plan_body(plan: &memora_core::MergePlan) {
    let mut updates = 0;
    let mut removes = 0;
    let mut conflicts = 0;
    let mut auto_picks = Vec::new();
    let mut conflict_lines = Vec::new();
    for entry in &plan.entries {
        match &entry.decision {
            NodeDecision::Unchanged => {}
            NodeDecision::TakeOurs(_) | NodeDecision::TakeTheirs(_) => updates += 1,
            NodeDecision::Auto { ours_won, reason } => {
                updates += 1;
                auto_picks.push(format!(
                    "  {} {} {} ({})",
                    if *ours_won {
                        bold(green("ours"))
                    } else {
                        bold(cyan("theirs"))
                    },
                    short_id(&entry.id),
                    entry
                        .resolved
                        .as_ref()
                        .map(|n| n.kind.to_string())
                        .unwrap_or_default(),
                    dim(reason),
                ));
            }
            NodeDecision::Conflicted { reason } => {
                conflicts += 1;
                conflict_lines.push(format!(
                    "  {} {} ({})",
                    bold(red("conflict")),
                    short_id(&entry.id),
                    dim(reason),
                ));
            }
            NodeDecision::Removed => removes += 1,
        }
    }

    let summary = format!("{updates} updates · {removes} removed · {conflicts} conflicted");
    println!("  {summary}");

    if !auto_picks.is_empty() {
        println!("\n{}", bold("auto-resolved:"));
        for line in auto_picks {
            println!("{line}");
        }
    }
    if !conflict_lines.is_empty() {
        println!("\n{}", bold("conflicts:"));
        for line in conflict_lines {
            println!("{line}");
        }
    }
}
