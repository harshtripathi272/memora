//! Pretty-printing helpers for the CLI.
//!
//! Centralising colours and formatting here means we can later add a
//! `--no-color` flag in one place. For now we lean on `owo_colors` to
//! auto-detect tty support.

use owo_colors::OwoColorize;

/// Print an error to stderr in red. Errors come back as `anyhow::Error`
/// from the command dispatcher so we can also show their cause chain.
pub fn print_error(err: &anyhow::Error) {
    eprintln!("{} {}", "error:".red().bold(), err);
    let mut source = err.source();
    while let Some(s) = source {
        eprintln!("  {} {}", "caused by:".dimmed(), s);
        source = s.source();
    }
}

/// Format a unix-second timestamp as a short local-ish date string for
/// log output. We use UTC to keep tests deterministic.
pub fn fmt_timestamp(ts: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| ts.to_string())
}

/// Abbreviate a SHA-256 to 7 hex chars (git-style).
pub fn short_id(id: &str) -> &str {
    &id[..7.min(id.len())]
}
