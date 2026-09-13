# Command matrix

Source of truth for script behaviour is the Flex overlay in
[fyrst-dev/recipes](https://github.com/fyrst-dev/recipes)
(`fyrst/shopware-cd/1.0/root/deploy/`). This CLI mirrors the public interface.
**Wrappers are not implemented yet** — every verb below except `--help` exits
2 with `not implemented`.

Delegated tools that must stay as external calls (never reimplemented in Rust):

| Concern | Keep calling |
| --- | --- |
| DB snapshot | `shopware-cli project dump` (pinned CLI image; `SYNC_DUMP_ENGINE=mysqldump` escape hatch) |
| DB restore | MySQL/MariaDB client import |
| VPS release / rollback | `docker compose` (`deploy/compose.yaml` + `compose.prod.yaml` + `compose.vps.yaml`) |
| Opt-in URL rewrite after restore | `bin/console fyrst:sales-channel:rewrite-urls` via compose `web` |

## Map

| Overlay script | CLI | Nested verbs | Flags (clap skeleton) | Status |
| --- | --- | --- | --- | --- |
| `deploy/init-env.sh` | `fyrst-cli shopware init-env` | — | `--shop-id`, `--env`, `--image`, `--vps`, `--generate-app-secret`, `--dry-run` | stub |
| `deploy/vps-release.sh` | `fyrst-cli shopware release` | — | `--dry-run`, `--skip-pull` | stub |
| `deploy/vps-rollback.sh` | `fyrst-cli shopware rollback` | — | `--dry-run`, `--skip-pull` | stub |
| `deploy/sync-runtime.sh` | `fyrst-cli shopware sync` | `snapshot`, `restore`, `sync` | `--from`, `--data`, `--snapshot-dir`, `--dry-run`, `--skip-db`, `--skip-volumes` | stub |
| `deploy/sync-runtime-local.sh` | `fyrst-cli shopware sync-local` | — | `--from`, `--data`, `--remote-data-root`, `--delete`, `--dry-run` | stub |
| `deploy/backup-runtime.sh` | `fyrst-cli shopware backup` | `backup`, `prune`, `restore` | `--data`, `--dry-run`; restore also `--from`, `--i-understand-this-restores-this-host` | stub |

`shopware sync` with no verb, and `shopware backup` with no verb, require a
subcommand (clap prints help, exit 2). That matches the overlay scripts, which
refuse a missing command.

## Environment (not clap flags)

Scripts read shop identity and secrets from the environment / shop-root
`.env`. The skeleton does not load them. Wrappers must keep the same names:

| Area | Variables (non-exhaustive) |
| --- | --- |
| Shop identity | `COMPOSE_DIR`, `SHOPWARE_SHOP_ID`, `SHOPWARE_DEPLOY_ENV`, `SHOPWARE_DATA_BASE`, `SHOPWARE_DATA_ROOT`, `COMPOSE_PROJECT_NAME` |
| Release | `IMAGE`, `IMAGE_TAG`, `COMPOSE_PROFILES`, `SMOKE_URL`, `PULL_POLICY`, `SKIP_PULL`, `ROLLBACK_ON_SMOKE_FAIL` |
| Sync | `SYNC_*` (SSH, dump engine, rewrite URL / map, live-restore guard) |
| Backup | `BACKUP_TARGET`, `BACKUP_KEEP_DAYS`, `BACKUP_SSH_KEY`, `BACKUP_ALLOW_LIVE_RESTORE`, `BACKUP_CONFIRM_RESTORE` |

## Future wrappers (not done here)

When implemented, each verb should exec (or source-equivalent) the overlay
script with the parsed argv, rather than re-coding dump/import/compose/rewrite
logic in Rust. Recipe and `shopware-cd` product code stay out of this repo.
