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
    let args = cli::Cli::parse();
    match commands::dispatch(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            ui::print_error(&err);
            ExitCode::FAILURE
        }
    }
}
