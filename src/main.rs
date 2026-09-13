//! fyrst.dev global CLI (`fyrst-cli`).
//!
//! Database dumps are owned by `shopware-cli project dump`. This binary does
//! not wrap dump. `shopware init-env` finishes shop-root `.env`. Database
//! import is `fyrst-cli shopware db import` (MySQL client via Compose exec or
//! a one-shot client image). VPS release is `fyrst-cli shopware release`.
//! Rollback is `fyrst-cli shopware rollback` (IMAGE_TAG from `.previous-tag`).
//! `shopware sync snapshot` copies bind-mount trees (not a dump).
//! `sync restore` restores DB, volumes, and opt-in rewrite.

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
