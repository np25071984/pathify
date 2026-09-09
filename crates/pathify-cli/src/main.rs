//! The `pathify` binary.
//!
//! Thin by design: argument parsing, input/output plumbing, and turning core
//! errors into messages and exit codes. All the real work lives in
//! `pathify-core`.

mod cli;
mod commands;
mod io;
mod units;

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command};

/// Exit codes, so `pathify … && …` and `set -e` scripts can tell the failure
/// modes apart rather than seeing an undifferentiated 1.
mod exit {
    /// Bad input: unparseable, or a format we can't read.
    pub const DATA_ERROR: u8 = 1;
    /// The input could not be read or the output could not be written.
    pub const IO_ERROR: u8 = 2;
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    match run(&cli, &mut out) {
        Ok(()) => {
            // A broken pipe here is normal — `pathify info … | head` closes the
            // reader early — and is not worth an error message.
            let _ = out.flush();
            ExitCode::SUCCESS
        }
        Err(error) => {
            // Diagnostics go to stderr so they never corrupt piped output.
            eprintln!("pathify: {error:#}");
            ExitCode::from(exit_code(&error))
        }
    }
}

fn run(cli: &Cli, out: &mut dyn Write) -> anyhow::Result<()> {
    match &cli.command {
        Command::Info(args) => commands::info::run(args, out),
        Command::Convert(args) => commands::convert::run(args, out),
        Command::Merge(args) => commands::merge::run(args, out),
        Command::Clean(args) => commands::clean::run(args, out),
        Command::View(args) => commands::view::run(args, out),
    }
}

fn exit_code(error: &anyhow::Error) -> u8 {
    use pathify_core::Error as CoreError;

    // An I/O failure anywhere in the chain — a missing file, a closed pipe —
    // is an environment problem, not bad trace data.
    if error.chain().any(|cause| {
        cause.downcast_ref::<std::io::Error>().is_some()
            || matches!(cause.downcast_ref::<CoreError>(), Some(CoreError::Io(_)))
    }) {
        return exit::IO_ERROR;
    }
    exit::DATA_ERROR
}
