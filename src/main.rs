//! fyrst.dev global CLI (`fyrst-cli`).
//!
//! Shopware CD commands under `shopware` are a clap skeleton in this commit.
//! They exit 2 with "not implemented". Future work wraps Flex recipe scripts;
//! it does not reimplement `shopware-cli project dump`, MySQL import, Compose
//! release, or `fyrst:sales-channel:rewrite-urls`.

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
