//! Shopware CD dispatch.
//!
//! Database **dump** is owned by `shopware-cli project dump` — this CLI never
//! wraps it. Database **import** is `fyrst-cli shopware db import` (also used
//! by `shopware sync restore --data db` and `shopware sync sync` when a dump
//! is already present). `init-env` finishes shop-root `.env`
//! (recipes `deploy/init-env.sh`). VPS **release** is
//! `fyrst-cli shopware release`. **Rollback** is
//! `fyrst-cli shopware rollback` (IMAGE_TAG from `.previous-tag`). `sync snapshot`
//! copies bind-mount trees (not a dump). `sync restore` also restores
//! bind-mount volumes and opt-in rewrite via compose `web`. `shopware sync sync`
//! pulls bind-mounts over SSH and imports an existing dump (does not dump).
//! `shopware sync-local` rsyncs VPS upload trees into a local project-dev
//! checkout (never DB). `shopware backup backup` copies bind-mount trees and
//! an operator-provided dump file (live allowed). Other verbs stay stubs. See
//! docs/ADR-0001-shopware-namespace.md.

mod app;
mod backup;
mod compose;
mod data;
mod env;
mod envfile;
mod error;
mod import;
mod init_env;
mod live;
mod lock;
mod mysql;
mod prune;
mod release;
mod restore;
mod rewrite;
mod rollback;
mod rollout;
mod snapshot;
mod ssh;
mod sync;
mod sync_local;
mod target;
mod volumes;

use crate::cli::{BackupCommand, DbCommand, ShopwareArgs, ShopwareCommand, SyncCommand};
use std::process::ExitCode;

pub fn run(args: ShopwareArgs) -> ExitCode {
    match args.command {
        ShopwareCommand::Db(DbCommand::Import(op)) => match import::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::Sync(SyncCommand::Snapshot(op)) => match snapshot::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::Sync(SyncCommand::Restore(op)) => match restore::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::InitEnv(op) => match init_env::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::Release(op) => match release::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::Rollback(op) => match rollback::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::Sync(SyncCommand::Sync(op)) => match sync::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::SyncLocal(op) => match sync_local::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::Backup(BackupCommand::Backup(op)) => match backup::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::Backup(BackupCommand::Prune(_)) => {
            not_implemented("shopware backup prune")
        }
        ShopwareCommand::Backup(BackupCommand::Restore(_)) => {
            not_implemented("shopware backup restore")
        }
    }
}

fn not_implemented(command: &str) -> ExitCode {
    eprintln!("not implemented: {command}");
    ExitCode::from(2)
}
