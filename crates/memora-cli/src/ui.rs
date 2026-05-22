//! Pretty-printing helpers for the CLI.
//!
//! The colour helpers here check a process-wide `COLOR_ENABLED` flag set
//! by `main.rs` based on `NO_COLOR` and TTY detection. Each helper takes a
//! `&str` and returns a `String` so callers can interpolate them naturally
//! with `format!` / `println!`. When colour is disabled the helpers simply
//! pass the string through unchanged.

use std::sync::atomic::{AtomicBool, Ordering};

use owo_colors::OwoColorize;

static COLOR_ENABLED: AtomicBool = AtomicBool::new(true);

/// Enable or disable colour for the rest of the process. Called once from
/// `main.rs` after parsing `NO_COLOR` and checking whether stdout is a tty.
pub fn set_color(enabled: bool) {
    COLOR_ENABLED.store(enabled, Ordering::SeqCst);
}

fn enabled() -> bool {
    COLOR_ENABLED.load(Ordering::SeqCst)
}

/// Apply the *bold* attribute.
pub fn bold(s: impl AsRef<str>) -> String {
    if enabled() {
        s.as_ref().bold().to_string()
    } else {
        s.as_ref().to_string()
    }
}

/// Apply the *dim* attribute.
pub fn dim(s: impl AsRef<str>) -> String {
    if enabled() {
        s.as_ref().dimmed().to_string()
    } else {
        s.as_ref().to_string()
    }
}

/// Green text.
pub fn green(s: impl AsRef<str>) -> String {
    if enabled() {
        s.as_ref().green().to_string()
    } else {
        s.as_ref().to_string()
    }
}

/// Red text.
pub fn red(s: impl AsRef<str>) -> String {
    if enabled() {
        s.as_ref().red().to_string()
    } else {
        s.as_ref().to_string()
    }
}

/// Yellow text.
pub fn yellow(s: impl AsRef<str>) -> String {
    if enabled() {
        s.as_ref().yellow().to_string()
    } else {
        s.as_ref().to_string()
    }
}

/// Cyan text.
pub fn cyan(s: impl AsRef<str>) -> String {
    if enabled() {
        s.as_ref().cyan().to_string()
    } else {
        s.as_ref().to_string()
    }
}

/// Print an error to stderr in red. Errors come back as `anyhow::Error`
/// from the command dispatcher so we can also show their cause chain.
pub fn print_error(err: &anyhow::Error) {
    eprintln!("{} {}", bold(red("error:")), err);
    let mut source = err.source();
    while let Some(s) = source {
        eprintln!("  {} {}", dim("caused by:"), s);
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
