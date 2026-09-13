# Command matrix

Source of truth for script behaviour is the Flex overlay in
[fyrst-dev/recipes](https://github.com/fyrst-dev/recipes)
(`fyrst/shopware-cd/1.0/root/deploy/`). This CLI mirrors the public interface.

Delegated tools that must stay as external calls (never reimplemented in Rust):

| Concern | Keep calling |
| --- | --- |
| DB snapshot | `shopware-cli project dump` (pinned CLI image; `SYNC_DUMP_ENGINE=mysqldump` escape hatch is still stub in this CLI) |
| DB restore | MySQL/MariaDB client import |
| VPS release / rollback | `docker compose` (`deploy/compose.yaml` + `compose.prod.yaml` + `compose.vps.yaml`) |
| Opt-in URL rewrite after restore | `bin/console fyrst:sales-channel:rewrite-urls` via compose `web` |

## Map

| Overlay script | CLI | Nested verbs | Flags | Status |
| --- | --- | --- | --- | --- |
| `deploy/init-env.sh` | `fyrst-cli shopware init-env` | — | `--shop-id`, `--env`, `--image`, `--vps`, `--generate-app-secret`, `--dry-run` | stub (exit 2) |
| `deploy/vps-release.sh` | `fyrst-cli shopware release` | — | `--dry-run`, `--skip-pull` | stub (exit 2) |
| `deploy/vps-rollback.sh` | `fyrst-cli shopware rollback` | — | `--dry-run`, `--skip-pull` | stub (exit 2) |
| `deploy/sync-runtime.sh` snapshot | `fyrst-cli shopware sync snapshot` | — | `--from`, `--data`, `--snapshot-dir`, `--dry-run`, `--skip-db`, `--skip-volumes` | **local DB dump implemented**; bind-mount volumes and remote SSH `--from` stub (exit 2); `SYNC_DUMP_ENGINE=mysqldump` stub (exit 2) |
| `deploy/sync-runtime.sh` restore / sync | `fyrst-cli shopware sync restore` / `sync sync` | — | same flags | stub (exit 2) |
| `deploy/sync-runtime-local.sh` | `fyrst-cli shopware sync-local` | — | `--from`, `--data`, `--remote-data-root`, `--delete`, `--dry-run` | stub (exit 2) |
| `deploy/backup-runtime.sh` | `fyrst-cli shopware backup` | `backup`, `prune`, `restore` | `--data`, `--dry-run`; restore also `--from`, `--i-understand-this-restores-this-host` | stub (exit 2) |

`shopware sync` with no verb, and `shopware backup` with no verb, require a
subcommand (clap prints help, exit 2). That matches the overlay scripts, which
refuse a missing command.

## `shopware sync snapshot` (implemented subset)

Resolves shop root (`COMPOSE_DIR` or walk from cwd for `.env` + `deploy/`),
loads `.env` then `.env.prod` then `deploy/sync.env` (identity keys from the
process environment win when non-empty). Requires `SHOPWARE_SHOP_ID`. Compose
network is `${COMPOSE_PROJECT_NAME}_default`, with
`COMPOSE_PROJECT_NAME` defaulting to `${SHOPWARE_SHOP_ID}-${SHOPWARE_DEPLOY_ENV}`.
Dump-only therefore needs `SHOPWARE_DEPLOY_ENV` **or** an explicit
`COMPOSE_PROJECT_NAME`.

DB dump (when `--data` includes `db` / `database` / `mysql` and `--skip-db` is
off):

1. Optional `docker compose … up -d --no-build mysql` when a compose `mysql`
   service is present.
2. `docker run --rm --network <project>_default` (or `host` for an external
   `DATABASE_URL`) mounting shop root read-only and the snapshot dir, `-w` shop
   root, image `SYNC_SHOPWARE_CLI_IMAGE` (default
   `ghcr.io/shopware/shopware-cli:0.18.4`).
3. `shopware-cli --no-update-hint project dump --skip-lock-tables
   --compression=gzip` plus `--quick` / `--clean` (default on) and
   `--anonymize` (default off). Write `--output=${SNAPSHOT_DIR}/db.sql.gz`.
4. Connection: `--host mysql` on the Compose network, or host/port from
   `DATABASE_URL`. Credentials from `MYSQL_*` or `DATABASE_URL` — never logged.

`--dry-run` prints that `docker run` line without `--password` and does not
call Docker.

Default `--data` still lists volume trees; those copies are **not
implemented**. If `db` is selected, the dump runs and stderr warns. If only
volumes remain (`--skip-db` or `--data media`), exit 2.

## Environment (not clap flags)

Scripts and this CLI read shop identity and secrets from the environment /
shop-root `.env`. Names match the overlay:

| Area | Variables (non-exhaustive) |
| --- | --- |
| Shop identity | `COMPOSE_DIR`, `SHOPWARE_SHOP_ID`, `SHOPWARE_DEPLOY_ENV`, `SHOPWARE_DATA_BASE`, `SHOPWARE_DATA_ROOT`, `COMPOSE_PROJECT_NAME` |
| Dump | `MYSQL_USER`, `MYSQL_PASSWORD`, `MYSQL_DATABASE`, `MYSQL_ROOT_PASSWORD`, `DATABASE_URL`, `SYNC_SHOPWARE_CLI_IMAGE`, `SYNC_DUMP_ENGINE`, `SYNC_DUMP_QUICK`, `SYNC_DUMP_CLEAN`, `SYNC_DUMP_ANONYMIZE`, `SYNC_SNAPSHOT_DIR` |
| Release | `IMAGE`, `IMAGE_TAG`, `COMPOSE_PROFILES`, `SMOKE_URL`, `PULL_POLICY`, `SKIP_PULL`, `ROLLBACK_ON_SMOKE_FAIL` |
| Sync (future) | other `SYNC_*` (SSH, rewrite URL / map, live-restore guard) |
| Backup (future) | `BACKUP_TARGET`, `BACKUP_KEEP_DAYS`, `BACKUP_SSH_KEY`, `BACKUP_ALLOW_LIVE_RESTORE`, `BACKUP_CONFIRM_RESTORE` |

## Future wrappers (not done here)

When implemented, remaining verbs should exec (or source-equivalent) the
overlay script with the parsed argv, rather than re-coding import / compose /
rewrite / rsync in Rust. Recipe and `shopware-cd` product code stay out of this
repo. Recipe bash wrappers are **not** switched to `fyrst-cli` in this change.
