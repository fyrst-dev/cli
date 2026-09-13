//! Shopware CD dispatch. Commands are stubs until recipe wrappers exist.
//!
//! Implementation rule (see docs/ADR-0001-shopware-namespace.md):
//! wrap the Flex overlay scripts; keep calling `shopware-cli project dump`,
//! the MySQL/MariaDB client for import, Compose for release/rollback, and
//! `bin/console fyrst:sales-channel:rewrite-urls`. Do not reimplement those.

use crate::cli::{BackupCommand, ShopwareArgs, ShopwareCommand, SyncCommand};
use std::process::ExitCode;

pub fn run(args: ShopwareArgs) -> ExitCode {
    let name = match args.command {
        ShopwareCommand::InitEnv(_) => "shopware init-env",
        ShopwareCommand::Release(_) => "shopware release",
        ShopwareCommand::Rollback(_) => "shopware rollback",
        ShopwareCommand::Sync(SyncCommand::Snapshot(_)) => "shopware sync snapshot",
        ShopwareCommand::Sync(SyncCommand::Restore(_)) => "shopware sync restore",
        ShopwareCommand::Sync(SyncCommand::Sync(_)) => "shopware sync sync",
        ShopwareCommand::SyncLocal(_) => "shopware sync-local",
        ShopwareCommand::Backup(BackupCommand::Backup(_)) => "shopware backup backup",
        ShopwareCommand::Backup(BackupCommand::Prune(_)) => "shopware backup prune",
        ShopwareCommand::Backup(BackupCommand::Restore(_)) => "shopware backup restore",
    };
    not_implemented(name)
}

fn not_implemented(command: &str) -> ExitCode {
    eprintln!("not implemented: {command}");
    ExitCode::from(2)
}
