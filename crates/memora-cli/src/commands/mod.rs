//! CLI command dispatch.
//!
//! Each variant of [`crate::cli::Command`] is handled by a function in
//! one of the submodules below. Keeping them separate makes them easy to
//! test in isolation.

use anyhow::Result;

use crate::cli::{Cli, Command};

mod add;
mod branch;
mod commit;
mod init;
mod log;
mod rollback;
mod status;
mod switch;

/// Route a parsed [`Cli`] to the right command implementation.
pub fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Init(args) => init::run(args),
        Command::Add(args) => add::run(args),
        Command::Commit(args) => commit::run(args),
        Command::Status => status::run(),
        Command::Log(args) => log::run(args),
        Command::Branch(args) => branch::run(args),
        Command::Switch(args) => switch::run(args),
        Command::Rollback(args) => rollback::run(args),
    }
}
