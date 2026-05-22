//! `memora promote` — move ephemeral nodes to stable.

use std::env;
use std::str::FromStr;

use anyhow::{Context, Result};

use memora_core::node::MemoryKind;
use memora_core::{PromotePlan, Repository};

use crate::cli::PromoteArgs;
use crate::ui::{bold, dim, green, short_id, yellow};

/// Entry point for the `promote` subcommand.
pub fn run(args: PromoteArgs) -> Result<()> {
    let plan = build_plan(&args)?;
    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;

    let plan = match plan {
        PromotePlan::Ids(ids) => {
            let resolved = ids
                .into_iter()
                .map(|id| resolve_node_id(&repo, &id))
                .collect::<Result<Vec<_>>>()?;
            PromotePlan::Ids(resolved)
        }
        other => other,
    };

    let promoted = repo.promote(plan)?;

    if promoted.is_empty() {
        println!(
            "{} no ephemeral nodes matched the promotion plan.",
            bold(yellow("Nothing to promote:"))
        );
        return Ok(());
    }

    println!(
        "{} {} node{}",
        bold(green("Promoted")),
        promoted.len(),
        if promoted.len() == 1 { "" } else { "s" }
    );
    for id in &promoted {
        println!("  {} {}", bold(short_id(id)), dim("ephemeral → stable"));
    }
    Ok(())
}

fn build_plan(args: &PromoteArgs) -> Result<PromotePlan> {
    match (&args.id, &args.kind, args.all_confirmed) {
        (Some(id), None, None) => Ok(PromotePlan::Ids(vec![id.clone()])),
        (None, Some(k), None) => {
            let kind = MemoryKind::from_str(k)
                .with_context(|| format!("invalid --type value '{k}'"))?;
            Ok(PromotePlan::Kind(kind))
        }
        (None, None, Some(threshold)) => {
            anyhow::ensure!(
                (0.0..=1.0).contains(&threshold),
                "--all-confirmed threshold must be between 0.0 and 1.0 (got {threshold})"
            );
            Ok(PromotePlan::AllConfirmed {
                min_confidence: threshold,
            })
        }
        _ => anyhow::bail!(
            "specify exactly one of --id <NODE>, --type <KIND>, or --all-confirmed [THRESHOLD]"
        ),
    }
}

/// Allow short ids on the CLI by scanning the working set.
fn resolve_node_id(repo: &Repository, candidate: &str) -> Result<String> {
    if repo.store().get_node(candidate)?.is_some() {
        return Ok(candidate.to_string());
    }
    let trimmed = candidate.trim();
    anyhow::ensure!(
        trimmed.len() >= 4 && trimmed.chars().all(|c| c.is_ascii_hexdigit()),
        "node id '{candidate}' must be at least 4 hex characters"
    );
    let mut matches = Vec::new();
    for n in repo.store().all_nodes()? {
        if n.id.starts_with(trimmed) {
            matches.push(n.id);
        }
    }
    match matches.len() {
        0 => anyhow::bail!("no node matches id prefix '{candidate}'"),
        1 => Ok(matches.pop().unwrap()),
        _ => anyhow::bail!(
            "ambiguous node id prefix '{candidate}' (matched {} nodes)",
            matches.len()
        ),
    }
}
