//! Shopware CD dispatch.
//!
//! `shopware sync snapshot` dumps the local DB via `shopware-cli project dump`
//! (one-shot official CLI image on the Compose network). Other verbs stay stubs
//! until recipe wrappers exist. See docs/ADR-0001-shopware-namespace.md.

mod data;
mod dump;
mod env;
mod envfile;
mod error;
mod snapshot;

use crate::cli::{BackupCommand, ShopwareArgs, ShopwareCommand, SyncCommand};
use std::process::ExitCode;

pub fn run(args: ShopwareArgs) -> ExitCode {
    match args.command {
        ShopwareCommand::Sync(SyncCommand::Snapshot(op)) => match snapshot::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::InitEnv(_) => not_implemented("shopware init-env"),
        ShopwareCommand::Release(_) => not_implemented("shopware release"),
        ShopwareCommand::Rollback(_) => not_implemented("shopware rollback"),
        ShopwareCommand::Sync(SyncCommand::Restore(_)) => not_implemented("shopware sync restore"),
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
