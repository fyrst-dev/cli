//! Shopware CD dispatch.
//!
//! Database **dump** is owned by `shopware-cli project dump` — this CLI never
//! wraps it. Database **import** is `fyrst-cli shopware db import` (also used
//! by `shopware sync restore --data db`). Other verbs stay stubs until recipe
//! wrappers exist. See docs/ADR-0001-shopware-namespace.md.

mod data;
mod env;
mod envfile;
mod error;
mod import;
mod live;
mod mysql;
mod restore;
mod snapshot;

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
        ShopwareCommand::InitEnv(_) => not_implemented("shopware init-env"),
        ShopwareCommand::Release(_) => not_implemented("shopware release"),
        ShopwareCommand::Rollback(_) => not_implemented("shopware rollback"),
        ShopwareCommand::Sync(SyncCommand::Sync(_)) => not_implemented("shopware sync sync"),
        ShopwareCommand::SyncLocal(_) => not_implemented("shopware sync-local"),
        ShopwareCommand::Backup(BackupCommand::Backup(_)) => {
            not_implemented("shopware backup backup")
        }
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
