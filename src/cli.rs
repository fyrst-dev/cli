//! Clap definitions for `fyrst-cli`.
//!
//! Operator verbs are grouped by lifecycle (`env`, `deploy`, `db`, `sync`,
//! `backup`), not overlay script filenames. Overlay scripts in
//! `fyrst/shopware-cd` (`deploy/init-env.sh`, `vps-release.sh`, …) are
//! unchanged. Old clap paths are a hard cut (no aliases). See
//! `docs/ADR-0002-shopware-lifecycle-regroup.md`.

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Printed on `fyrst-cli shopware --help` so nested verbs are visible.
pub const SHOPWARE_COMMAND_TREE: &str = "\
Command tree:
  fyrst-cli shopware env init
  fyrst-cli shopware deploy release
  fyrst-cli shopware deploy rollback
  fyrst-cli shopware db import
  fyrst-cli shopware sync capture
  fyrst-cli shopware sync apply
  fyrst-cli shopware sync pull
  fyrst-cli shopware sync local
  fyrst-cli shopware backup create
  fyrst-cli shopware backup prune
  fyrst-cli shopware backup recover

sync = between environments / workdir. backup = off-host disaster recovery.
sync capture copies bind-mount trees for sync (not a retained backup).
sync apply loads a workdir onto this host; backup recover is off-host DR.

Live policy (refuse a live consumer unless):
  db import         --allow-live or SHOPWARE_ALLOW_LIVE_RESTORE=1
  sync apply        SHOPWARE_ALLOW_LIVE_RESTORE=1 only
  sync pull         SHOPWARE_ALLOW_LIVE_RESTORE=1 only
  backup recover    confirm flag + SHOPWARE_ALLOW_LIVE_RESTORE=1
                    (--i-understand-this-restores-this-host or BACKUP_CONFIRM_RESTORE=1)

Dump = shopware-cli project dump only (fyrst-cli does not dump).
Import = fyrst-cli shopware db import (also used by sync apply --data db and sync pull).
env init = fyrst-cli shopware env init (shop-root .env after create + Flex).
Release = fyrst-cli shopware deploy release (VPS compose; never builds).
Rollback = fyrst-cli shopware deploy rollback (IMAGE_TAG from .previous-tag).
sync capture = bind-mount trees into --snapshot-dir (not a dump).
sync apply also restores bind-mount volumes and opt-in rewrite via compose web.
sync pull = rsync remote bind-mounts + import of an already-present dump (does not dump).
sync local = VPS → local project-dev rsync (never DB; not SHOPWARE_DATA_ROOT; --data all refused).
Backup = fyrst-cli shopware backup create (volumes + operator db.sql.gz; live allowed).
Backup prune = stamp-based retention under BACKUP_TARGET (BACKUP_KEEP_DAYS).
backup recover = disaster recovery onto this host (confirmation required).
See docs/command-matrix.md.
";

#[derive(Debug, Parser)]
#[command(
    name = "fyrst-cli",
    version,
    about = "fyrst.dev global CLI",
    long_about = "Home of the fyrst.dev global CLI (`fyrst-cli`).\n\n\
Shopware CD ops live under `shopware`. Other fyrst namespaces can be added later.\n\n\
Database dumps are owned by `shopware-cli project dump`; fyrst-cli does not wrap dump. \
`shopware env init` finishes shop-root .env after create + Flex. \
`shopware db import` loads a .sql / .sql.gz via the MySQL/MariaDB client. \
`shopware deploy release` pulls IMAGE:IMAGE_TAG and recreates the VPS Compose stack (never builds). \
`shopware deploy rollback` re-deploys IMAGE using IMAGE_TAG from `.previous-tag`. \
`shopware sync capture` copies bind-mount / volume trees; it is not a dump command. \
`shopware sync apply` loads --snapshot-dir (same import module, bind-mount volumes, \
opt-in rewrite via compose web). \
`shopware sync pull` pulls from `--from` (rsync bind-mounts + import of an already-present dump) and does not dump. \
`shopware sync local` rsyncs VPS upload trees into a local project-dev checkout (never DB). \
`shopware backup create` copies bind-mount trees into BACKUP_TARGET (live allowed) \
and never wraps dump. \
`shopware backup prune` deletes stamp-named artifacts under BACKUP_TARGET. \
`shopware backup recover` is disaster recovery onto this host (confirmation required).",
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
    long_about = "Shopware CD operations grouped by lifecycle: env (shop identity), \
deploy (VPS compose), db (SQL import), sync (between environments / workdir), \
backup (off-host disaster recovery).\n\n\
`env init` is implemented: finish shop-root `.env` after create + Flex (merge missing \
keys from `.env.example`, shop id, optional IMAGE). Comments out COMPOSE_PROJECT_NAME \
and leftover SHOPWARE_DEPLOY_ENV in committed `.env`; writes SHOPWARE_DEPLOY_ENV and \
COMPOSE_PROJECT_NAME=<shop-id>-<env> to `.env.local` and upserts Compose `name:` in \
gitignored `compose.override.yaml`. Does not generate APP_SECRET. `--dry-run` prints the \
plan and does not write. Passwords are never printed.\n\n\
`db import` is implemented: MySQL/MariaDB client import (Compose `mysql` exec, else a \
one-shot client image for DATABASE_URL). `sync apply` uses that same import module, \
restores bind-mount volumes from --snapshot-dir, stops/starts web/worker/scheduler, and \
runs opt-in `bin/console fyrst:sales-channel:rewrite-urls` via compose `web`. \
`sync pull` is the cron/operator pull path: rsync remote bind-mounts onto this host, then \
import `db.sql.gz` from `--snapshot-dir` if `--data` includes db (does not dump). \
`--from local` is a capture+apply pipeline check, not the staging cron path. \
`deploy release` is implemented: VPS `docker compose` pull + recreate \
(`deploy/compose.yaml` + `compose.prod.yaml` + `compose.vps.yaml`). Never builds images. \
`sync capture` copies bind-mount trees into --snapshot-dir/data/<item>/; it does not dump. \
`sync local` rsyncs VPS bind-mount trees into ./public/media, ./files, … (never the \
database, never local SHOPWARE_DATA_ROOT). `--data all` is refused (all includes db on sync pull / backup create). \
Dumps stay with `shopware-cli project dump` — this CLI does not wrap dump.\n\n\
`deploy rollback` is implemented: same compose files and order as `vps-release.sh`, with \
IMAGE_TAG only from `.previous-tag` (process-env IMAGE_TAG is ignored).\n\n\
`backup create` copies bind-mount trees (and an operator-provided db.sql.gz) into \
BACKUP_TARGET; it is allowed on live. \
`backup prune` is implemented: stamp-based retention under BACKUP_TARGET \
(BACKUP_KEEP_DAYS, default 14, 0 = keep forever). \
`backup recover` fetches an artifact (`--artifact` / `--stamp`, alias `--from`) and applies it onto this host. It is not \
live→staging sync. Confirmation is required (`--i-understand-this-restores-this-host` \
or BACKUP_CONFIRM_RESTORE=1). Live needs SHOPWARE_ALLOW_LIVE_RESTORE=1.",
    after_help = SHOPWARE_COMMAND_TREE
)]
pub struct ShopwareArgs {
    #[command(subcommand)]
    pub command: ShopwareCommand,
}

#[derive(Debug, Subcommand)]
pub enum ShopwareCommand {
    /// Shop identity / `.env` setup
    #[command(subcommand)]
    Env(EnvCommand),
    /// Image pull + Compose recreate on the VPS
    #[command(subcommand)]
    Deploy(DeployCommand),
    /// Import a SQL dump (shopware-cli has no import)
    #[command(subcommand)]
    Db(DbCommand),
    /// Between-env / laptop data movement (workdir, not retention)
    #[command(subcommand)]
    Sync(SyncCommand),
    /// Off-host retention + disaster recovery
    #[command(subcommand)]
    Backup(BackupCommand),
}

#[derive(Debug, Subcommand)]
pub enum EnvCommand {
    /// Finish shop-root .env after create + Flex (deploy/init-env.sh)
    Init(InitEnvArgs),
}

#[derive(Debug, Subcommand)]
pub enum DeployCommand {
    /// Pull IMAGE:IMAGE_TAG and recreate the VPS stack (deploy/vps-release.sh)
    Release(ReleaseArgs),
    /// Re-deploy IMAGE using IMAGE_TAG from .previous-tag (deploy/vps-rollback.sh)
    Rollback(RollbackArgs),
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
    after_help = "Does not overwrite the whole .env. Does not invent MYSQL passwords or APP_URL. \
Does not generate APP_SECRET (shopware-cli project create writes that). \
Shared .env is git-committed: comments out uncommented COMPOSE_PROJECT_NAME=… \
(create writes COMPOSE_PROJECT_NAME=sw-…) and leftover SHOPWARE_DEPLOY_ENV. \
Does not set COMPOSE_PROJECT_NAME or SHOPWARE_DEPLOY_ENV in shared .env. \
Writes SHOPWARE_DEPLOY_ENV and COMPOSE_PROJECT_NAME=<shop-id>-<env> to .env.local \
(gitignored; created if missing). Upserts top-level Compose name: <shop-id>-<env> \
in gitignored compose.override.yaml so shopware-cli project dev uses that name \
(Compose reads COMPOSE_PROJECT_NAME only from project-directory .env, not .env.local). \
VPS compose interpolates the same <shop-id>-<env> from deploy/compose.yaml name: plus host \
env files; docker compose -p is a matching pin.\n\n\
Environment:\n  \
  COMPOSE_DIR    Shop checkout (default: walk from cwd for .env / .env.example + deploy/)\n\n\
Examples:\n  \
  fyrst-cli shopware env init --shop-id acme\n  \
  fyrst-cli shopware env init --shop-id acme --env live --image ghcr.io/example/acme\n  \
  fyrst-cli shopware env init --shop-id acme --dry-run\n"
)]
pub struct InitEnvArgs {
    /// Shop slug (required unless SHOPWARE_SHOP_ID is already non-empty in `.env`)
    #[arg(long = "shop-id", value_name = "SLUG")]
    pub shop_id: Option<String>,

    /// live | staging | playground | dev (written to .env.local with COMPOSE_PROJECT_NAME; default live when unset and no host value)
    #[arg(long = "env", value_enum, value_name = "NAME")]
    pub env: Option<DeployEnv>,

    /// Set IMAGE (registry/repo). Unset leaves IMAGE as-is
    #[arg(long = "image", value_name = "REPO")]
    pub image: Option<String>,

    /// Print the summary; do not write .env, .env.local, or compose.override.yaml
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

    /// Allow importing onto a live host (or set SHOPWARE_ALLOW_LIVE_RESTORE=1)
    #[arg(long = "allow-live")]
    pub allow_live: bool,
}

#[derive(Debug, Subcommand)]
pub enum SyncCommand {
    /// Bind-mount trees into --snapshot-dir; dump stays shopware-cli
    Capture(SyncOpArgs),
    /// Load --snapshot-dir onto this host (DB import + bind-mount volumes)
    Apply(SyncOpArgs),
    /// Pull from --from then apply locally (rsync + import; does not dump)
    Pull(SyncOpArgs),
    /// VPS upload trees → local project-dev checkout (never DB)
    Local(SyncLocalArgs),
}

#[derive(Debug, Args)]
pub struct SyncOpArgs {
    /// Source host. "local" = this machine (default for capture). Other aliases use SSH
    #[arg(long = "from", value_name = "ALIAS")]
    pub from: Option<String>,

    /// Comma-separated subset, or "all" (default: db,media,files,thumbnail,theme,sitemap)
    #[arg(long = "data", value_name = "LIST")]
    pub data: Option<String>,

    /// Work directory (default: <shop>/var/runtime-sync)
    #[arg(long = "snapshot-dir", value_name = "DIR")]
    pub snapshot_dir: Option<String>,

    /// Print actions; do not copy or apply (capture does not dump)
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
    Create(BackupOpArgs),
    /// Delete stamp-named artifacts older than BACKUP_KEEP_DAYS (also runs after backup)
    Prune(BackupOpArgs),
    /// Recover one artifact onto this host (disaster recovery; not sync)
    Recover(BackupRecoverArgs),
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
pub struct BackupRecoverArgs {
    #[command(flatten)]
    pub common: BackupOpArgs,

    /// Artifact timestamp or path under the shop/env prefix
    #[arg(
        long = "artifact",
        visible_aliases = ["stamp", "from"],
        value_name = "STAMP_OR_DIR"
    )]
    pub artifact: Option<String>,

    /// Required confirmation for recover (or BACKUP_CONFIRM_RESTORE=1)
    #[arg(long = "i-understand-this-restores-this-host")]
    pub confirm_restore: bool,
}

impl BackupRecoverArgs {
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
    fn sync_capture_parses_flags() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "sync",
            "capture",
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
                command: ShopwareCommand::Sync(SyncCommand::Capture(op)),
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
        for name in ["env", "deploy", "sync", "backup", "db"] {
            assert!(
                shopware.find_subcommand(name).is_some(),
                "missing shopware {name}",
            );
        }
        for gone in ["init-env", "release", "rollback", "sync-local"] {
            assert!(
                shopware.find_subcommand(gone).is_none(),
                "old primary name still present: shopware {gone}",
            );
        }

        let env = shopware.find_subcommand("env").unwrap();
        assert!(env.find_subcommand("init").is_some(), "missing env init");

        let deploy = shopware.find_subcommand("deploy").unwrap();
        for name in ["release", "rollback"] {
            assert!(
                deploy.find_subcommand(name).is_some(),
                "missing deploy {name}",
            );
        }

        let sync = shopware.find_subcommand("sync").unwrap();
        for name in ["capture", "apply", "pull", "local"] {
            assert!(sync.find_subcommand(name).is_some(), "missing sync {name}");
        }
        for gone in ["snapshot", "restore", "sync"] {
            assert!(
                sync.find_subcommand(gone).is_none(),
                "old sync name still present: {gone}",
            );
        }

        let backup = shopware.find_subcommand("backup").unwrap();
        for name in ["create", "prune", "recover"] {
            assert!(
                backup.find_subcommand(name).is_some(),
                "missing backup {name}",
            );
        }
        for gone in ["backup", "restore"] {
            assert!(
                backup.find_subcommand(gone).is_none(),
                "old backup name still present: {gone}",
            );
        }

        let db = shopware.find_subcommand("db").unwrap();
        assert!(
            db.find_subcommand("import").is_some(),
            "missing shopware db import",
        );
    }

    #[test]
    fn env_init_parses_flags() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "env",
            "init",
            "--shop-id",
            "acme",
            "--env",
            "staging",
            "--image",
            "ghcr.io/example/acme",
            "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::Env(EnvCommand::Init(op)),
            }) => {
                assert_eq!(op.shop_id.as_deref(), Some("acme"));
                assert_eq!(op.env, Some(DeployEnv::Staging));
                assert_eq!(op.image.as_deref(), Some("ghcr.io/example/acme"));
                assert!(op.dry_run);
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn env_init_rejects_vps() {
        let err = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "env",
            "init",
            "--shop-id",
            "acme",
            "--vps",
        ])
        .unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("unexpected argument") || text.contains("unexpected"),
            "{text}"
        );
    }

    #[test]
    fn env_init_rejects_generate_app_secret() {
        let err = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "env",
            "init",
            "--shop-id",
            "acme",
            "--generate-app-secret",
        ])
        .unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("unexpected argument") || text.contains("unexpected"),
            "{text}"
        );
    }

    #[test]
    fn deploy_rollback_parses_flags() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "deploy",
            "rollback",
            "--dry-run",
            "--skip-pull",
        ])
        .unwrap();
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::Deploy(DeployCommand::Rollback(op)),
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
    fn deploy_release_parses_flags() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "deploy",
            "release",
            "--dry-run",
            "--skip-pull",
        ])
        .unwrap();
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::Deploy(DeployCommand::Release(op)),
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

    fn assert_backup_recover_artifact(cli: Cli, spec: &str) {
        match cli.command {
            Command::Shopware(ShopwareArgs {
                command: ShopwareCommand::Backup(BackupCommand::Recover(op)),
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
    fn backup_recover_parses_artifact_flag() {
        let cli = Cli::try_parse_from([
            "fyrst-cli",
            "shopware",
            "backup",
            "recover",
            "--artifact",
            "20260912T020000Z",
            "--data",
            "db",
            "--dry-run",
            "--i-understand-this-restores-this-host",
        ])
        .unwrap();
        assert_backup_recover_artifact(cli, "20260912T020000Z");
    }

    #[test]
    fn backup_recover_parses_stamp_and_from_aliases() {
        for flag in ["--stamp", "--from"] {
            let cli = Cli::try_parse_from([
                "fyrst-cli",
                "shopware",
                "backup",
                "recover",
                flag,
                "20260912T020000Z",
                "--data",
                "db",
                "--dry-run",
                "--i-understand-this-restores-this-host",
            ])
            .unwrap();
            assert_backup_recover_artifact(cli, "20260912T020000Z");
        }
    }

    #[test]
    fn old_command_paths_are_hard_cut() {
        let old: &[&[&str]] = &[
            &["fyrst-cli", "shopware", "init-env"],
            &["fyrst-cli", "shopware", "release"],
            &["fyrst-cli", "shopware", "rollback"],
            &["fyrst-cli", "shopware", "sync-local"],
            &["fyrst-cli", "shopware", "sync", "snapshot"],
            &["fyrst-cli", "shopware", "sync", "restore"],
            &["fyrst-cli", "shopware", "sync", "sync"],
            &["fyrst-cli", "shopware", "backup", "backup"],
            &["fyrst-cli", "shopware", "backup", "restore"],
        ];
        for args in old {
            assert!(
                Cli::try_parse_from(*args).is_err(),
                "old path should not parse: {args:?}"
            );
        }
    }
}
