//! `memora` — the memory layer for AI agents.
//!
//! This binary is a thin layer over [`memora_core`]. Each subcommand lives
//! in its own module under `commands/` so we can unit-test argument parsing
//! and behaviour independently.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use clap::Parser;

mod cli;
mod commands;
mod ui;

fn main() -> ExitCode {
    // Decide colour once. We disable when NO_COLOR is set or stdout is
    // not a real terminal (so `memora ... | grep ...` is clean).
    let stdout_is_tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let colour = stdout_is_tty && std::env::var_os("NO_COLOR").is_none();
    ui::set_color(colour);

    let args = cli::Cli::parse();
    match commands::dispatch(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            ui::print_error(&err);
            ExitCode::FAILURE
        }
    }
}
