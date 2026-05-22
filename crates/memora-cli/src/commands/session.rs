//! `memora session` — start, end, query recording sessions.

use std::env;

use anyhow::Result;

use memora_core::Repository;

use crate::cli::{SessionArgs, SessionCommand, SessionListArgs, SessionStartArgs};
use crate::ui::{bold, dim, green, short_id, yellow};

/// Entry point for the `session` subcommand.
pub fn run(args: SessionArgs) -> Result<()> {
    let cwd = env::current_dir()?;
    let repo = Repository::open_from(&cwd)?;
    match args.command {
        SessionCommand::Start(a) => start(&repo, a),
        SessionCommand::End => end(&repo),
        SessionCommand::Current => current(&repo),
        SessionCommand::List(a) => list(&repo, a),
    }
}

fn start(repo: &Repository, args: SessionStartArgs) -> Result<()> {
    let session = repo.start_session(&args.source)?;
    println!(
        "{} session {} (source: {})",
        bold(green("Started")),
        bold(short_id(&session.id)),
        session.source,
    );
    println!(
        "{}",
        dim("subsequent add / commit / promote / merge will be recorded; run `memora session end` when done")
    );
    Ok(())
}

fn end(repo: &Repository) -> Result<()> {
    match repo.end_session()? {
        None => println!("{}", dim("no active session.")),
        Some(s) => {
            println!(
                "{} session {} ({} events)",
                bold(yellow("Ended")),
                bold(short_id(&s.id)),
                s.event_count
            );
        }
    }
    Ok(())
}

fn current(repo: &Repository) -> Result<()> {
    match repo.current_session_id()? {
        None => println!("{}", dim("no active session.")),
        Some(id) => {
            let session = repo.store().get_session(&id)?;
            match session {
                None => println!("{id}"),
                Some(s) => println!(
                    "{} ({} events, source {})",
                    bold(short_id(&s.id)),
                    s.event_count,
                    s.source
                ),
            }
        }
    }
    Ok(())
}

fn list(repo: &Repository, args: SessionListArgs) -> Result<()> {
    let sessions = repo.list_sessions(args.limit)?;
    if sessions.is_empty() {
        println!("{}", dim("no sessions recorded yet."));
        return Ok(());
    }
    for s in sessions {
        let active = s.ended_at.is_none();
        let marker = if active { yellow("●") } else { dim(" ") };
        println!(
            "{} {} {} {} events  source={}",
            marker,
            bold(short_id(&s.id)),
            crate::ui::fmt_timestamp(s.started_at),
            s.event_count,
            s.source
        );
    }
    Ok(())
}
