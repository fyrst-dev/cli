//! Clap definitions for `fyrst-cli`.
//!
//! Flag names and nesting follow `fyrst/shopware-cd` Flex overlay scripts
//! (`deploy/init-env.sh`, `vps-release.sh`, `vps-rollback.sh`,
//! `sync-runtime.sh`, `sync-runtime-local.sh`, `backup-runtime.sh`).
//! Operator-facing verbs prefer `sync pull` / `backup create` / `sync local`;
//! clap aliases keep overlay/script names (`sync sync`, `backup backup`,
//! top-level `sync-local`).

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Printed on `fyrst-cli shopware --help` so nested verbs are visible.
pub const SHOPWARE_COMMAND_TREE: &str = "\
Command tree:
  fyrst-cli shopware init-env
  fyrst-cli shopware release
  fyrst-cli shopware rollback
  fyrst-cli shopware db import
  fyrst-cli shopware sync snapshot
  fyrst-cli shopware sync restore
  fyrst-cli shopware sync pull
  fyrst-cli shopware sync local
  fyrst-cli shopware backup create
  fyrst-cli shopware backup prune
  fyrst-cli shopware backup restore

Aliases (overlay / script parity):
  sync pull     = sync sync
  sync local    = sync-local (top-level)
  backup create = backup backup

sync = between environments / workdir. backup = off-host disaster recovery.
sync snapshot captures bind-mount trees for sync (not a retained backup).
Keep restore on both: sync restore applies a snapshot; backup restore is DR.

Live policy (refuse a live consumer unless):
  db import         --allow-live or SYNC_ALLOW_LIVE_RESTORE=1
  sync restore      SYNC_ALLOW_LIVE_RESTORE=1 only
  sync pull         SYNC_ALLOW_LIVE_RESTORE=1 only
  backup restore    confirm flag + BACKUP_ALLOW_LIVE_RESTORE=1
                    (--i-understand-this-restores-this-host or BACKUP_CONFIRM_RESTORE=1)

Dump = shopware-cli project dump only (fyrst-cli does not dump).
Import = fyrst-cli shopware db import (also used by sync restore --data db and sync pull).
init-env = fyrst-cli shopware init-env (shop-root .env after create + Flex).
Release = fyrst-cli shopware release (VPS compose; never builds).
Rollback = fyrst-cli shopware rollback (IMAGE_TAG from .previous-tag).
Snapshot volumes = fyrst-cli shopware sync snapshot (bind-mount trees; not a dump).
sync restore also restores bind-mount volumes and opt-in rewrite via compose web.
sync pull = rsync remote bind-mounts + import of an already-present dump (does not dump).
sync local = VPS → local project-dev rsync (never DB; not SHOPWARE_DATA_ROOT; --data all refused).
Backup = fyrst-cli shopware backup create (volumes + operator db.sql.gz; live allowed).
Backup prune = stamp-based retention under BACKUP_TARGET (BACKUP_KEEP_DAYS).
backup restore = disaster recovery onto this host (confirmation required).
Other verbs still exit 2 (not implemented). See docs/command-matrix.md.
";

#[derive(Debug, Parser)]
#[command(
    name = "fyrst-cli",
    version,
    about = "fyrst.dev global CLI",
    long_about = "Home of the fyrst.dev global CLI (`fyrst-cli`).\n\n\
Shopware CD ops live under `shopware`. Other fyrst namespaces can be added later.\n\n\
Database dumps are owned by `shopware-cli project dump`; fyrst-cli does not wrap dump. \
`shopware init-env` finishes shop-root .env after create + Flex. \
`shopware db import` loads a .sql / .sql.gz via the MySQL/MariaDB client. \
`shopware release` pulls IMAGE:IMAGE_TAG and recreates the VPS Compose stack (never builds). \
`shopware rollback` re-deploys IMAGE using IMAGE_TAG from `.previous-tag`. \
`shopware sync snapshot` copies bind-mount / volume trees; it is not a dump command. \
`shopware sync restore` loads --snapshot-dir (same import module, bind-mount volumes, \
opt-in rewrite via compose web). \
`shopware sync pull` (alias: `sync sync`) pulls from `--from` (rsync bind-mounts + import of an already-present dump) and does not dump. \
`shopware sync local` (alias: `sync-local`) rsyncs VPS upload trees into a local project-dev checkout (never DB). \
`shopware backup create` (alias: `backup backup`) copies bind-mount trees into BACKUP_TARGET (live allowed) \
and never wraps dump. \
`shopware backup prune` deletes stamp-named artifacts under BACKUP_TARGET. \
`shopware backup restore` is disaster recovery onto this host (confirmation required). \
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
deploy/ in fyrst-dev/recipes (`fyrst/shopware-cd`), with operator-facing aliases \
documented in `shopware --help` (after_help).\n\n\
`init-env` is implemented: finish shop-root `.env` after create + Flex (merge missing \
keys from `.env.example`, shop id / deploy env, optional IMAGE / APP_SECRET, `--vps` \
comments COMPOSE_PROJECT_NAME). `--dry-run` prints the plan and does not write. \
Passwords and APP_SECRET are never printed.\n\n\
`db import` is implemented: MySQL/MariaDB client import (Compose `mysql` exec, else a \
one-shot client image for DATABASE_URL). `sync restore` uses that same import module, \
restores bind-mount volumes from --snapshot-dir, stops/starts web/worker/scheduler, and \
runs opt-in `bin/console fyrst:sales-channel:rewrite-urls` via compose `web`. \
`sync pull` (alias `sync sync`) is the cron/operator pull path: rsync remote bind-mounts onto this host, then \
import `db.sql.gz` from `--snapshot-dir` if `--data` includes db (does not dump). \
`--from local` is a snapshot+restore pipeline check, not the staging cron path. \
`release` is implemented: VPS `docker compose` pull + recreate \
(`deploy/compose.yaml` + `compose.prod.yaml` + `compose.vps.yaml`). Never builds images. \
`sync snapshot` copies bind-mount trees into --snapshot-dir/data/<item>/; it does not dump. \
`sync local` (alias `sync-local`) rsyncs VPS bind-mount trees into ./public/media, ./files, … (never the \
database, never local SHOPWARE_DATA_ROOT). `--data all` is refused (all includes db on sync pull / backup create). \
Dumps stay with `shopware-cli project dump` — this CLI does not wrap dump.\n\n\
`rollback` is implemented: same compose files and order as `vps-release.sh`, with \
IMAGE_TAG only from `.previous-tag` (process-env IMAGE_TAG is ignored).\n\n\
`backup create` (alias `backup backup`) copies bind-mount trees (and an operator-provided db.sql.gz) into \
BACKUP_TARGET; it is allowed on live. \
`backup prune` is implemented: stamp-based retention under BACKUP_TARGET \
(BACKUP_KEEP_DAYS, default 14, 0 = keep forever). \
`backup restore` fetches an artifact (`--artifact` / `--stamp`, alias `--from`) and applies it onto this host. It is not \
live→staging sync. Confirmation is required (`--i-understand-this-restores-this-host` \
or BACKUP_CONFIRM_RESTORE=1). Live needs BACKUP_ALLOW_LIVE_RESTORE=1.",
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
    /// Import a SQL dump (shopware-cli has no import)
    #[command(subcommand)]
    Db(DbCommand),
    /// Snapshot / restore / pull / local runtime data (deploy/sync-runtime.sh)
    #[command(subcommand)]
    Sync(SyncCommand),
    /// Alias of `sync local`: VPS upload trees → local project-dev (never DB)
    #[command(name = "sync-local")]
    SyncLocal(SyncLocalArgs),
    /// Create / prune / restore runtime artifacts (deploy/backup-runtime.sh)
    #[command(subcommand)]
    Backup(BackupCommand),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum DeployEnv {
    Live,
    Staging,
    Playground,
    Dev,
}

impl DeployEnv {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Staging => "staging",
            Self::Playground => "playground",
            Self::Dev => "dev",
        }
    }
}

#[derive(Debug, Args)]
#[command(
    after_help = "Does not overwrite the whole .env. Does not invent MYSQL passwords or APP_URL.\n\n\
Environment:\n  \
  COMPOSE_DIR    Shop checkout (default: walk from cwd for .env / .env.example + deploy/)\n\n\
Examples:\n  \
  fyrst-cli shopware init-env --shop-id acme\n  \
  fyrst-cli shopware init-env --shop-id acme --env live --vps --image ghcr.io/example/acme\n  \
  fyrst-cli shopware init-env --shop-id acme --generate-app-secret\n  \
  fyrst-cli shopware init-env --shop-id acme --vps --dry-run\n"
)]
pub struct InitEnvArgs {
    /// Shop slug (required unless SHOPWARE_SHOP_ID is already non-empty in `.env`)
    #[arg(long = "shop-id", value_name = "SLUG")]
    pub shop_id: Option<String>,

    /// live | staging | playground | dev (default live when unset/empty; keep existing non-empty)
    #[arg(long = "env", value_enum, value_name = "NAME")]
    pub env: Option<DeployEnv>,

    /// Set IMAGE (registry/repo). Unset leaves IMAGE as-is
    #[arg(long = "image", value_name = "REPO")]
    pub image: Option<String>,

    /// Comment out COMPOSE_PROJECT_NAME=… lines (create footgun on a VPS)
    #[arg(long)]
    pub vps: bool,

    /// Set APP_SECRET with openssl rand -hex 32 if empty (value is never printed)
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
pub enum DbCommand {
    /// Import a .sql or .sql.gz dump into this shop's database
    Import(DbImportArgs),
}

#[derive(Debug, Args)]
pub struct DbImportArgs {
    /// Path to a .sql or .sql.gz dump
    #[arg(long = "file", value_name = "PATH")]
    pub file: String,

    /// Print the import plan; do not import
    #[arg(long)]
    pub dry_run: bool,

    /// Allow importing onto a live host (or set SYNC_ALLOW_LIVE_RESTORE=1)
    #[arg(long = "allow-live")]
    pub allow_live: bool,
}

#[derive(Debug, Subcommand)]
pub enum SyncCommand {
    /// Bind-mount trees into --snapshot-dir; dump stays shopware-cli
    Snapshot(SyncOpArgs),
    /// Load --snapshot-dir onto this host (DB import + bind-mount volumes)
    Restore(SyncOpArgs),
    /// Pull from --from then apply locally (rsync + import; does not dump)
    #[command(visible_alias = "sync")]
    Pull(SyncOpArgs),
    /// VPS upload trees → local project-dev checkout (never DB)
    Local(SyncLocalArgs),
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

    /// Print actions; do not copy or restore (snapshot does not dump)
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

    /// Comma-separated subset (default: media,files,thumbnail,theme,sitemap; not db). `--data all` is refused
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
    /// Copy bind-mount trees (+ operator db.sql.gz) into BACKUP_TARGET (timestamped; live allowed)
    #[command(visible_alias = "backup")]
    Create(BackupOpArgs),
    /// Delete stamp-named artifacts older than BACKUP_KEEP_DAYS (also runs after backup)
    Prune(BackupOpArgs),
    /// Restore one artifact onto this host (disaster recovery; not sync)
    Restore(BackupRestoreArgs),
}

#[derive(Debug, Args)]
pub struct BackupOpArgs {
    /// db,media,files,thumbnail,theme,sitemap (default all). prune: ignored (stamp-based)
    #[arg(long = "data", value_name = "LIST")]
    pub data: Option<String>,

    /// Print snapshot/copy/prune actions; do not copy, dump, or delete
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct BackupRestoreArgs {
    #[command(flatten)]
    pub common: BackupOpArgs,

    /// Artifact timestamp or path under the shop/env prefix
    #[arg(
        long = "artifact",
        visible_aliases = ["stamp", "from"],
        value_name = "STAMP_OR_DIR"
    )]
    pub artifact: Option<String>,

    /// Required confirmation for restore (or BACKUP_CONFIRM_RESTORE=1)
    #[arg(long = "i-understand-this-restores-this-host")]
    pub confirm_restore: bool,
}

impl BackupRestoreArgs {
    /// Stamp or directory path (`--artifact` / `--stamp` / `--from`).
    pub fn artifact_spec(&self) -> Option<&str> {
        self.artifact
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
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
    fn sync_pull_parses_flags() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "sync",
            "pull",
            "--from",
            "live",
            "--data",
            "media,files",
            "--dry-run",
            "--skip-db",
        ])
        .unwrap();
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::Sync(SyncCommand::Pull(op)),
            }) => {
                assert_eq!(op.from.as_deref(), Some("live"));
                assert_eq!(op.data.as_deref(), Some("media,files"));
                assert!(op.dry_run);
                assert!(op.skip_db);
                assert!(!op.skip_volumes);
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn sync_sync_alias_parses_as_pull() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "sync",
            "sync",
            "--from",
            "live",
            "--data",
            "media,files",
            "--dry-run",
            "--skip-db",
        ])
        .unwrap();
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::Sync(SyncCommand::Pull(op)),
            }) => {
                assert_eq!(op.from.as_deref(), Some("live"));
                assert_eq!(op.data.as_deref(), Some("media,files"));
                assert!(op.dry_run);
                assert!(op.skip_db);
            }
            other => panic!("alias should parse as pull: {other:?}"),
        }
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
            "db",
        ] {
            assert!(
                shopware.find_subcommand(name).is_some(),
                "missing shopware {name}",
            );
        }

        let sync = shopware.find_subcommand("sync").unwrap();
        for name in ["snapshot", "restore", "pull", "local"] {
            assert!(sync.find_subcommand(name).is_some(), "missing sync {name}");
        }
        assert!(
            sync.find_subcommand("sync").is_some(),
            "missing overlay alias sync sync",
        );

        let backup = shopware.find_subcommand("backup").unwrap();
        for name in ["create", "prune", "restore"] {
            assert!(
                backup.find_subcommand(name).is_some(),
                "missing backup {name}",
            );
        }
        assert!(
            backup.find_subcommand("backup").is_some(),
            "missing overlay alias backup backup",
        );

        let db = shopware.find_subcommand("db").unwrap();
        assert!(
            db.find_subcommand("import").is_some(),
            "missing shopware db import",
        );
    }

    #[test]
    fn init_env_parses_flags() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "init-env",
            "--shop-id",
            "acme",
            "--env",
            "staging",
            "--image",
            "ghcr.io/example/acme",
            "--vps",
            "--generate-app-secret",
            "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::InitEnv(op),
            }) => {
                assert_eq!(op.shop_id.as_deref(), Some("acme"));
                assert_eq!(op.env, Some(DeployEnv::Staging));
                assert_eq!(op.image.as_deref(), Some("ghcr.io/example/acme"));
                assert!(op.vps);
                assert!(op.generate_app_secret);
                assert!(op.dry_run);
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn rollback_parses_flags() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "rollback",
            "--dry-run",
            "--skip-pull",
        ])
        .unwrap();
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::Rollback(op),
            }) => {
                assert!(op.dry_run);
                assert!(op.skip_pull);
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn db_import_parses_flags() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "db",
            "import",
            "--file",
            "/tmp/db.sql.gz",
            "--dry-run",
            "--allow-live",
        ])
        .unwrap();
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::Db(DbCommand::Import(op)),
            }) => {
                assert_eq!(op.file, "/tmp/db.sql.gz");
                assert!(op.dry_run);
                assert!(op.allow_live);
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn release_parses_flags() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "release",
            "--dry-run",
            "--skip-pull",
        ])
        .unwrap();
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::Release(op),
            }) => {
                assert!(op.dry_run);
                assert!(op.skip_pull);
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn sync_local_nested_parses_flags() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "sync",
            "local",
            "--from",
            "live",
            "--data",
            "media,files",
            "--remote-data-root",
            "/data",
            "--delete",
            "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::Sync(SyncCommand::Local(op)),
            }) => {
                assert_eq!(op.from.as_deref(), Some("live"));
                assert_eq!(op.data.as_deref(), Some("media,files"));
                assert_eq!(op.remote_data_root.as_deref(), Some("/data"));
                assert!(op.delete);
                assert!(op.dry_run);
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn sync_local_top_level_alias_parses_flags() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "sync-local",
            "--from",
            "live",
            "--data",
            "media,files",
            "--remote-data-root",
            "/data",
            "--delete",
            "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::SyncLocal(op),
            }) => {
                assert_eq!(op.from.as_deref(), Some("live"));
                assert_eq!(op.data.as_deref(), Some("media,files"));
                assert_eq!(op.remote_data_root.as_deref(), Some("/data"));
                assert!(op.delete);
                assert!(op.dry_run);
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn backup_create_parses_flags() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "backup",
            "create",
            "--data",
            "media,db",
            "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::Backup(BackupCommand::Create(op)),
            }) => {
                assert_eq!(op.data.as_deref(), Some("media,db"));
                assert!(op.dry_run);
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn backup_backup_alias_parses_as_create() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "backup",
            "backup",
            "--data",
            "media,db",
            "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::Backup(BackupCommand::Create(op)),
            }) => {
                assert_eq!(op.data.as_deref(), Some("media,db"));
                assert!(op.dry_run);
            }
            other => panic!("alias should parse as create: {other:?}"),
        }
    }

    #[test]
    fn backup_prune_parses_flags() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "backup",
            "prune",
            "--data",
            "db",
            "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::Backup(BackupCommand::Prune(op)),
            }) => {
                assert_eq!(op.data.as_deref(), Some("db"));
                assert!(op.dry_run);
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    fn assert_backup_restore_artifact(cli: Cli, spec: &str) {
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::Backup(BackupCommand::Restore(op)),
            }) => {
                assert_eq!(op.artifact_spec(), Some(spec));
                assert_eq!(op.common.data.as_deref(), Some("db"));
                assert!(op.common.dry_run);
                assert!(op.confirm_restore);
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn backup_restore_parses_artifact_flag() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "backup",
            "restore",
            "--artifact",
            "20260912T020000Z",
            "--data",
            "db",
            "--dry-run",
            "--i-understand-this-restores-this-host",
        ])
        .unwrap();
        assert_backup_restore_artifact(cli, "20260912T020000Z");
    }

    #[test]
    fn backup_restore_parses_stamp_and_from_aliases() {
        for flag in ["--stamp", "--from"] {
            let cli = Cli::try_parse_from([
                "fyrst-cli",
                "shopware",
                "backup",
                "restore",
                flag,
                "20260912T020000Z",
                "--data",
                "db",
                "--dry-run",
                "--i-understand-this-restores-this-host",
            ])
            .unwrap();
            assert_backup_restore_artifact(cli, "20260912T020000Z");
        }
    }
}
