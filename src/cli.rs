//! Clap definitions for `fyrst-cli`.
//!
//! Flag names and nesting follow `fyrst/shopware-cd` Flex overlay scripts
//! (`deploy/init-env.sh`, `vps-release.sh`, `vps-rollback.sh`,
//! `sync-runtime.sh`, `sync-runtime-local.sh`, `backup-runtime.sh`).

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Printed on `fyrst-cli shopware --help` so nested verbs are visible.
pub const SHOPWARE_COMMAND_TREE: &str = "\
Command tree:
  fyrst-cli shopware init-env
  fyrst-cli shopware release
  fyrst-cli shopware rollback
  fyrst-cli shopware sync snapshot
  fyrst-cli shopware sync restore
  fyrst-cli shopware sync sync
  fyrst-cli shopware sync-local
  fyrst-cli shopware backup backup
  fyrst-cli shopware backup prune
  fyrst-cli shopware backup restore

`shopware sync snapshot` dumps the local DB via shopware-cli.
Other verbs still exit 2 (not implemented). See docs/command-matrix.md.
";

#[derive(Debug, Parser)]
#[command(
    name = "fyrst-cli",
    version,
    about = "fyrst.dev global CLI",
    long_about = "Home of the fyrst.dev global CLI (`fyrst-cli`).\n\n\
Shopware CD ops live under `shopware`. Other fyrst namespaces can be added later.\n\n\
`shopware sync snapshot` (local) dumps the database via `shopware-cli project dump`. \
Other shopware subcommands still exit 2 with \"not implemented\".",
    arg_required_else_help = true,
    subcommand_required = true,
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Shopware CD operations (Flex overlay deploy/ scripts)
    Shopware(ShopwareArgs),
}

#[derive(Debug, Args)]
#[command(
    arg_required_else_help = true,
    subcommand_required = true,
    about = "Shopware CD operations",
    long_about = "Shopware CD operations. Names match Flex overlay scripts under \
deploy/ in fyrst-dev/recipes (`fyrst/shopware-cd`).\n\n\
`sync snapshot` (local, DB) is implemented: it runs `shopware-cli project dump` in a \
one-shot container. Bind-mount volume copy and remote SSH snapshot are still stub.\n\n\
Other verbs will keep calling:\n  \
- mysql/mariadb client import (DB restore)\n  \
- docker compose (release / rollback)\n  \
- bin/console fyrst:sales-channel:rewrite-urls (opt-in after restore)\n\n\
Those tools are not reimplemented here.",
    after_help = SHOPWARE_COMMAND_TREE
)]
pub struct ShopwareArgs {
    #[command(subcommand)]
    pub command: ShopwareCommand,
}

#[derive(Debug, Subcommand)]
pub enum ShopwareCommand {
    /// Finish shop-root .env after create + Flex (deploy/init-env.sh)
    #[command(name = "init-env")]
    InitEnv(InitEnvArgs),
    /// Pull IMAGE:IMAGE_TAG and recreate the VPS stack (deploy/vps-release.sh)
    Release(ReleaseArgs),
    /// Re-deploy IMAGE using IMAGE_TAG from .previous-tag (deploy/vps-rollback.sh)
    Rollback(RollbackArgs),
    /// Snapshot / restore / sync runtime data (deploy/sync-runtime.sh)
    #[command(subcommand)]
    Sync(SyncCommand),
    /// Pull VPS upload trees into a local project-dev checkout (deploy/sync-runtime-local.sh)
    #[command(name = "sync-local")]
    SyncLocal(SyncLocalArgs),
    /// Backup / prune / restore runtime artifacts (deploy/backup-runtime.sh)
    #[command(subcommand)]
    Backup(BackupCommand),
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "lower")]
pub enum DeployEnv {
    Live,
    Staging,
    Playground,
    Dev,
}

#[derive(Debug, Args)]
pub struct InitEnvArgs {
    /// Shop slug (required unless SHOPWARE_SHOP_ID is already non-empty)
    #[arg(long = "shop-id", value_name = "SLUG")]
    pub shop_id: Option<String>,

    /// live | staging | playground | dev (default live when unset/empty)
    #[arg(long = "env", value_enum, value_name = "NAME")]
    pub env: Option<DeployEnv>,

    /// Set IMAGE (registry/repo). Unset leaves IMAGE as-is
    #[arg(long = "image", value_name = "REPO")]
    pub image: Option<String>,

    /// Comment out COMPOSE_PROJECT_NAME=… lines (create footgun on a VPS)
    #[arg(long)]
    pub vps: bool,

    /// Set APP_SECRET with openssl rand -hex 32 if empty
    #[arg(long = "generate-app-secret")]
    pub generate_app_secret: bool,

    /// Print the summary; do not write .env
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct ReleaseArgs {
    /// Print the compose sequence; do not pull or recreate containers
    #[arg(long)]
    pub dry_run: bool,

    /// Same-host / air-gap: skip registry pull, PULL_POLICY=never
    #[arg(long = "skip-pull")]
    pub skip_pull: bool,
}

#[derive(Debug, Args)]
pub struct RollbackArgs {
    /// Print the compose sequence; do not pull or recreate containers
    #[arg(long)]
    pub dry_run: bool,

    /// Same-host / air-gap: skip registry pull, PULL_POLICY=never
    #[arg(long = "skip-pull")]
    pub skip_pull: bool,
}

#[derive(Debug, Subcommand)]
pub enum SyncCommand {
    /// Dump DB via shopware-cli (local); bind-mount copy is not implemented
    Snapshot(SyncOpArgs),
    /// Load --snapshot-dir into this host's DB and/or SHOPWARE_DATA_ROOT
    Restore(SyncOpArgs),
    /// Pull from --from then apply locally (cron path: rsync trees + DB)
    Sync(SyncOpArgs),
}

#[derive(Debug, Args)]
pub struct SyncOpArgs {
    /// Source host. "local" = this machine (default for snapshot). Other aliases use SSH
    #[arg(long = "from", value_name = "ALIAS")]
    pub from: Option<String>,

    /// Comma-separated subset, or "all" (default: db,media,files,thumbnail,theme,sitemap)
    #[arg(long = "data", value_name = "LIST")]
    pub data: Option<String>,

    /// Work directory (default: <shop>/var/runtime-sync)
    #[arg(long = "snapshot-dir", value_name = "DIR")]
    pub snapshot_dir: Option<String>,

    /// Print actions; do not dump, copy, or restore
    #[arg(long)]
    pub dry_run: bool,

    /// Drop db from --data
    #[arg(long = "skip-db")]
    pub skip_db: bool,

    /// Drop files/media/thumbnail/theme/sitemap from --data
    #[arg(long = "skip-volumes")]
    pub skip_volumes: bool,
}

#[derive(Debug, Args)]
pub struct SyncLocalArgs {
    /// SSH host alias (default: live)
    #[arg(long = "from", value_name = "ALIAS")]
    pub from: Option<String>,

    /// Comma-separated subset, or "all" (media,files,thumbnail,theme,sitemap; not db)
    #[arg(long = "data", value_name = "LIST")]
    pub data: Option<String>,

    /// Bind-mount root on the SSH source
    #[arg(long = "remote-data-root", value_name = "PATH")]
    pub remote_data_root: Option<String>,

    /// Pass rsync --delete (off by default so extra local files are kept)
    #[arg(long)]
    pub delete: bool,

    /// Print rsync actions; do not copy
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Subcommand)]
pub enum BackupCommand {
    /// Snapshot DB + bind-mount trees into BACKUP_TARGET (timestamped)
    Backup(BackupOpArgs),
    /// Delete artifacts older than BACKUP_KEEP_DAYS (also runs after backup)
    Prune(BackupOpArgs),
    /// Restore one artifact onto this host (disaster recovery)
    Restore(BackupRestoreArgs),
}

#[derive(Debug, Args)]
pub struct BackupOpArgs {
    /// db,media,files,thumbnail,theme,sitemap (default all)
    #[arg(long = "data", value_name = "LIST")]
    pub data: Option<String>,

    /// Print actions; do not dump, copy, prune, or restore
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct BackupRestoreArgs {
    #[command(flatten)]
    pub common: BackupOpArgs,

    /// Artifact timestamp or path under the shop/env prefix
    #[arg(long = "from", value_name = "STAMP_OR_DIR")]
    pub from: Option<String>,

    /// Required confirmation for restore (or BACKUP_CONFIRM_RESTORE=1)
    #[arg(long = "i-understand-this-restores-this-host")]
    pub confirm_restore: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn clap_debug_assert() {
        Cli::command().debug_assert();
    }

    #[test]
    fn snapshot_parses_flags() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "sync",
            "snapshot",
            "--from",
            "local",
            "--data",
            "db",
            "--snapshot-dir",
            "/tmp/s",
            "--dry-run",
            "--skip-volumes",
        ])
        .unwrap();
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::Sync(SyncCommand::Snapshot(op)),
            }) => {
                assert_eq!(op.from.as_deref(), Some("local"));
                assert_eq!(op.data.as_deref(), Some("db"));
                assert_eq!(op.snapshot_dir.as_deref(), Some("/tmp/s"));
                assert!(op.dry_run);
                assert!(op.skip_volumes);
                assert!(!op.skip_db);
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn shopware_help_lists_command_tree() {
        let cmd = Cli::command();
        let shopware = cmd.find_subcommand("shopware").expect("shopware");
        for name in [
            "init-env",
            "release",
            "rollback",
            "sync",
            "sync-local",
            "backup",
        ] {
            assert!(
                shopware.find_subcommand(name).is_some(),
                "missing shopware {name}",
            );
        }

        let sync = shopware.find_subcommand("sync").unwrap();
        for name in ["snapshot", "restore", "sync"] {
            assert!(sync.find_subcommand(name).is_some(), "missing sync {name}");
        }

        let backup = shopware.find_subcommand("backup").unwrap();
        for name in ["backup", "prune", "restore"] {
            assert!(
                backup.find_subcommand(name).is_some(),
                "missing backup {name}",
            );
        }
    }
}
