//! `sw`: the command-line frontend of `sw-core`.
//!
//! All IRIX distribution knowledge lives in `sw-core`; this binary only
//! parses arguments, calls the library and renders results.

mod cli;
mod commands;
mod output;

use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = cli::Cli::parse();
    match commands::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
