//! fyrst.dev global CLI (`fyrst-cli`).
//!
//! Database dumps are owned by `shopware-cli project dump`. This binary does
//! not wrap dump. Database import is `fyrst-cli shopware db import` (MySQL
//! client via Compose exec or a one-shot client image). Other shopware verbs
//! still exit 2 except `sync restore` for the DB path.

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
