//! fyrst.dev global CLI (`fyrst-cli`).
//!
//! Database dumps are owned by `shopware-cli project dump`. This binary does
//! not wrap dump. `shopware env init` finishes committed `.env` (shop id / IMAGE)
//! and host `.env.local` (`SHOPWARE_DEPLOY_ENV`, `COMPOSE_PROJECT_NAME`) plus
//! gitignored `compose.override.yaml` (`name: <shop-id>-<env>`). Database
//! import is `fyrst-cli shopware db import` (MySQL client via Compose exec or
//! a one-shot client image). VPS release is `fyrst-cli shopware deploy release`.
//! Rollback is `fyrst-cli shopware deploy rollback` (IMAGE_TAG from
//! `.previous-tag`). `shopware sync capture` copies bind-mount trees (not a
//! dump). `sync apply` restores DB, volumes, and opt-in rewrite.
//! `shopware sync pull` pulls bind-mounts over SSH and imports an existing dump
//! (does not dump). `shopware sync local` rsyncs VPS upload trees into a local
//! project-dev checkout (never DB). `shopware backup create` copies bind-mount
//! trees (live allowed) and an operator-provided dump file.
//! `shopware backup prune` is stamp-based retention under BACKUP_TARGET.
//! `shopware backup recover` is disaster recovery onto this host.

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
