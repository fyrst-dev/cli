//! Shopware CD dispatch.
//!
//! Database **dump** is owned by `shopware-cli project dump` — this CLI never
//! wraps it. Database **import** is `fyrst-cli shopware db import` (also used
//! by `shopware sync apply --data db`, `shopware sync pull` when a dump is
//! already present, and `shopware backup recover`). `env init` finishes
//! shop-root `.env` (recipes `deploy/init-env.sh`). VPS **release** is
//! `fyrst-cli shopware deploy release`. **Rollback** is
//! `fyrst-cli shopware deploy rollback` (IMAGE_TAG from `.previous-tag`).
//! `sync capture` copies bind-mount trees (not a dump). `sync apply` also
//! restores bind-mount volumes and opt-in rewrite via compose `web`.
//! `shopware sync pull` pulls bind-mounts over SSH and imports an existing dump
//! (does not dump). `shopware sync local` rsyncs VPS upload trees into a local
//! project-dev checkout (never DB). `shopware backup create` copies bind-mount
//! trees and an operator-provided dump file (live allowed). `shopware backup
//! prune` deletes stamp-named artifacts under BACKUP_TARGET.
//! `shopware backup recover` is disaster recovery onto this host.
//! See docs/ADR-0001-shopware-namespace.md and
//! docs/ADR-0002-shopware-lifecycle-regroup.md.

mod app;
mod backup;
mod backup_restore;
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

use crate::cli::{
    BackupCommand, DbCommand, DeployCommand, EnvCommand, ShopwareArgs, ShopwareCommand, SyncCommand,
};
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
        ShopwareCommand::Sync(SyncCommand::Capture(op)) => match snapshot::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::Sync(SyncCommand::Apply(op)) => match restore::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::Env(EnvCommand::Init(op)) => match init_env::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::Deploy(DeployCommand::Release(op)) => match release::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::Deploy(DeployCommand::Rollback(op)) => match rollback::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::Sync(SyncCommand::Pull(op)) => match sync::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::Sync(SyncCommand::Local(op)) => match sync_local::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::Backup(BackupCommand::Create(op)) => match backup::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::Backup(BackupCommand::Prune(op)) => match prune::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
        ShopwareCommand::Backup(BackupCommand::Recover(op)) => match backup_restore::run(op) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                e.print();
                e.exit_code()
            }
        },
    }
}
