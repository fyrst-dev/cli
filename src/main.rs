//! fyrst.dev global CLI (`fyrst-cli`).
//!
//! Database dumps are owned by `shopware-cli project dump`. This binary does
//! not wrap dump. `shopware init-env` finishes shop-root `.env`. Database
//! import is `fyrst-cli shopware db import` (MySQL client via Compose exec or
//! a one-shot client image). VPS release is `fyrst-cli shopware release`.
//! Rollback is `fyrst-cli shopware rollback` (IMAGE_TAG from `.previous-tag`).
//! `shopware sync snapshot` copies bind-mount trees (not a dump).
//! `sync restore` restores DB, volumes, and opt-in rewrite.
//! `shopware sync sync` pulls bind-mounts over SSH and imports an existing dump
//! (does not dump). `shopware sync-local` rsyncs VPS upload trees into a local
//! project-dev checkout (never DB). `shopware backup backup` copies bind-mount
//! trees (live allowed) and an operator-provided dump file.
//! `shopware backup prune` is stamp-based retention under BACKUP_TARGET.
//! `shopware backup restore` is disaster recovery onto this host.

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
