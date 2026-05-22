//! `memora replay` — walk through a session's recorded events.

use std::env;
use std::io::{self, BufRead};

use anyhow::Result;

use memora_core::session::SessionEventKind;
use memora_core::Repository;

use crate::cli::ReplayArgs;
use crate::ui::{bold, cyan, dim, fmt_timestamp, green, red, short_id, yellow};

/// Entry point for the `replay` subcommand.
pub fn run(args: ReplayArgs) -> Result<()> {
    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;

    let session_id = match args.session {
        Some(id) => id,
        None => match repo.current_session_id()? {
            Some(id) => id,
            None => {
                // Fall back to the most recent session.
                let recent = repo.list_sessions(Some(1))?;
                recent
                    .into_iter()
                    .next()
                    .map(|s| s.id)
                    .ok_or_else(|| anyhow::anyhow!(
                        "no session id given and no sessions recorded yet; run `memora session start` first"
                    ))?
            }
        },
    };

    let events = repo.session_events(&session_id)?;
    if events.is_empty() {
        println!("{}", dim("session has no recorded events."));
        return Ok(());
    }
    let resolved_id = events.first().unwrap().session_id.clone();
    println!(
        "{} session {} ({} events)",
        bold("replay"),
        bold(short_id(&resolved_id)),
        events.len(),
    );
    println!();

    let stdin = io::stdin();
    let mut lock = stdin.lock();
    let mut buf = String::new();

    for (idx, event) in events.iter().enumerate() {
        let header_color = match event.kind {
            SessionEventKind::SessionStarted => green("▶ session_started"),
            SessionEventKind::SessionEnded => yellow("■ session_ended"),
            SessionEventKind::NodeAdded => green("+ node_added"),
            SessionEventKind::NodePromoted => cyan("⇧ node_promoted"),
            SessionEventKind::CommitCreated => bold("● commit_created"),
            SessionEventKind::MergeCompleted => red("⇆ merge_completed"),
        };
        println!(
            "[{:>3}] {} {} {}",
            idx + 1,
            dim(fmt_timestamp(event.timestamp)),
            header_color,
            describe_event(event),
        );

        if args.step && idx + 1 < events.len() {
            buf.clear();
            // Read one line; if stdin is closed (e.g. piped, no more input)
            // just continue rather than exiting noisily.
            if lock.read_line(&mut buf).is_err() {
                break;
            }
        }
    }
    Ok(())
}

fn describe_event(event: &memora_core::SessionEvent) -> String {
    let data = &event.data;
    match event.kind {
        SessionEventKind::SessionStarted => {
            let source = data.get("source").and_then(|v| v.as_str()).unwrap_or("?");
            format!("source = {source}")
        }
        SessionEventKind::SessionEnded => "session closed".to_string(),
        SessionEventKind::NodeAdded => {
            let kind = data.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
            let id = data.get("node_id").and_then(|v| v.as_str()).unwrap_or("?");
            let content = data
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("[{}] {} {}", kind, short_id(id), truncate(content, 80))
        }
        SessionEventKind::NodePromoted => {
            let ids = data
                .get("node_ids")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| short_id(s).to_string()))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            format!("ephemeral → stable [{ids}]")
        }
        SessionEventKind::CommitCreated => {
            let id = data
                .get("commit_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let msg = data.get("message").and_then(|v| v.as_str()).unwrap_or("");
            format!("{} {}", short_id(id), truncate(msg, 80))
        }
        SessionEventKind::MergeCompleted => {
            let kind = data.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
            let theirs = data.get("theirs").and_then(|v| v.as_str()).unwrap_or("");
            let ours = data.get("ours").and_then(|v| v.as_str()).unwrap_or("");
            format!(
                "{} {} ← {}",
                kind,
                short_id(ours),
                short_id(theirs)
            )
        }
    }
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
