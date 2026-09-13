//! fyrst.dev global CLI (`fyrst-cli`).
//!
//! `shopware sync snapshot` dumps the local database by delegating to
//! `shopware-cli project dump` (one-shot official CLI image). Other shopware
//! verbs still exit 2. This binary does not reimplement dump, MySQL import,
//! Compose release, or `fyrst:sales-channel:rewrite-urls`.

mod cli;
mod shopware;

use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = cli::Cli::parse();
    match cli.command {
        cli::Command::Shopware(args) => shopware::run(args),
    }
}
