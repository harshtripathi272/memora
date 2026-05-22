//! `memora export` — render the working set into another tool's format.

use std::env;
use std::fs;
use std::str::FromStr;

use anyhow::{Context, Result};

use memora_core::node::{MemoryKind, MemoryStatus};
use memora_core::{ExportFilter, ExportFormat, ImportanceWeights, Repository};

use crate::cli::ExportArgs;
use crate::ui::{bold, dim, green};

/// Entry point for the `export` subcommand.
pub fn run(args: ExportArgs) -> Result<()> {
    let format = ExportFormat::parse(&args.to)
        .with_context(|| format!("unknown export format '{}'", args.to))?;

    let mut filter = ExportFilter {
        weights: ImportanceWeights::default(),
        top: args.top,
        kinds: Vec::new(),
        statuses: Vec::new(),
        min_confidence: args.min_confidence,
    };
    for k in &args.kinds {
        filter
            .kinds
            .push(MemoryKind::from_str(k).with_context(|| format!("invalid --kind '{k}'"))?);
    }
    for s in &args.statuses {
        filter
            .statuses
            .push(MemoryStatus::from_str(s).with_context(|| format!("invalid --status '{s}'"))?);
    }
    if let Some(c) = filter.min_confidence {
        anyhow::ensure!(
            (0.0..=1.0).contains(&c),
            "--min-confidence must be between 0.0 and 1.0 (got {c})"
        );
    }

    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;
    let body = repo.export(format, filter)?;

    if args.stdout {
        print!("{body}");
        return Ok(());
    }

    let path = match args.output {
        Some(p) => p,
        None => cwd.join(format.default_filename()),
    };
    fs::write(&path, &body).with_context(|| format!("writing {}", path.display()))?;
    println!(
        "{} {} ({} bytes) → {}",
        bold(green("Exported")),
        args.to,
        body.len(),
        dim(path.display().to_string())
    );
    Ok(())
}
