# Command matrix

Source of truth for script behaviour is the Flex overlay in
[fyrst-dev/recipes](https://github.com/fyrst-dev/recipes)
(`fyrst/shopware-cd/1.0/root/deploy/`). This CLI mirrors the public interface.

Delegated tools:

| Concern | Owner |
| --- | --- |
| **DB dump** | **`shopware-cli project dump` only.** fyrst-cli does not dump, wrap dump, or shell out to shopware-cli. |
| **DB import** | **`fyrst-cli shopware db import`** (MySQL/MariaDB client). shopware-cli has no import. |
| VPS release | **`fyrst-cli shopware release`** (`docker compose` with `deploy/compose.yaml` + `compose.prod.yaml` + `compose.vps.yaml`) |
| VPS rollback | **`fyrst-cli shopware rollback`** (same compose files; `IMAGE_TAG` from `.previous-tag`) |
| Opt-in URL rewrite after restore | `bin/console fyrst:sales-channel:rewrite-urls` via compose `web` |

## Map

| Overlay script | CLI | Nested verbs | Flags | Status |
| --- | --- | --- | --- | --- |
| `deploy/init-env.sh` | `fyrst-cli shopware init-env` | — | `--shop-id`, `--env`, `--image`, `--vps`, `--generate-app-secret`, `--dry-run` | **implemented** |
| `deploy/vps-release.sh` | `fyrst-cli shopware release` | — | `--dry-run`, `--skip-pull` | **implemented** |
| `deploy/vps-rollback.sh` | `fyrst-cli shopware rollback` | — | `--dry-run`, `--skip-pull` | **implemented** |
| `restore_db_*` (sync-runtime) | `fyrst-cli shopware db import` | `import` | `--file`, `--dry-run`, `--allow-live` | **implemented** |
| `deploy/sync-runtime.sh` snapshot | `fyrst-cli shopware sync snapshot` | — | `--from`, `--data`, `--snapshot-dir`, `--dry-run`, `--skip-db`, `--skip-volumes` | **volumes implemented** (bind-mount trees → `--snapshot-dir/data/<item>/`; named-volume tar fallback). **Not a dump command:** `--data db` exits 2 and tells operators to run `shopware-cli project dump` |
| `deploy/sync-runtime.sh` restore | `fyrst-cli shopware sync restore` | — | same flags | **DB path implemented** (same import module; `--snapshot-dir/db.sql.gz`); volumes stub; live refuse unless `SYNC_ALLOW_LIVE_RESTORE=1` |
| `deploy/sync-runtime.sh` sync | `fyrst-cli shopware sync sync` | — | same flags | stub (exit 2) |
| `deploy/sync-runtime-local.sh` | `fyrst-cli shopware sync-local` | — | `--from`, `--data`, `--remote-data-root`, `--delete`, `--dry-run` | stub (exit 2) |
| `deploy/backup-runtime.sh` | `fyrst-cli shopware backup` | `backup`, `prune`, `restore` | `--data`, `--dry-run`; restore also `--from`, `--i-understand-this-restores-this-host` | stub (exit 2) |

`shopware sync` with no verb, `shopware backup` with no verb, and
`shopware db` with no verb, require a subcommand (clap prints help, exit 2).

## Exact init-env CLI

```text
fyrst-cli shopware init-env [--shop-id SLUG] [--env live|staging|playground|dev] [--image REPO] [--vps] [--generate-app-secret] [--dry-run]
```

## Exact release CLI

```text
fyrst-cli shopware release [--dry-run] [--skip-pull]
```

## `shopware release` (implemented)

Resolves shop root (`COMPOSE_DIR` or walk from cwd), loads `.env` then `.env.prod`
(not `deploy/sync.env`). Process-env `IMAGE` / `IMAGE_TAG` (and other VPS knobs)
win when non-empty — CI tags beat `.env` `IMAGE_TAG=latest`. Requires `IMAGE`,
`IMAGE_TAG`, `SHOPWARE_SHOP_ID`, `SHOPWARE_DEPLOY_ENV`, and all three compose files.

Compose is always:

```text
docker compose --env-file .env -f deploy/compose.yaml -f deploy/compose.prod.yaml -f deploy/compose.vps.yaml
```

Never `--build`. `compose run` uses `--pull never` (not `--no-build`). `compose up`
uses `--no-build` (and `--pull never` when skipping registry pull). Theme/asset
compile flags stay in the setup helper image.

1. If `.deployed-tag` exists, copy it to `.previous-tag`.
2. `compose pull` unless `--skip-pull` / `SKIP_PULL=1` / `PULL_POLICY=never`.
3. Start bundled `mysql` if present; start `redis` if present / profiled.
4. One-shot `--profile setup run --rm --pull never setup`.
5. `up -d --no-build` (optional `--pull never`) `--remove-orphans web`.
6. Extra `COMPOSE_PROFILES` (must not include `setup`).
7. Optional `SMOKE_URL` (curl, 30 attempts). Write `.deployed-tag` only after
   setup/web succeed and smoke passes.
8. On smoke failure: print
   `IMAGE_TAG=$(cat .previous-tag) fyrst-cli shopware rollback`. Auto-run the
   shared rollout helper at the previous tag when `ROLLBACK_ON_SMOKE_FAIL` is
   on (unset → **on for live**, off otherwise). Release still **exits 1** after
   a successful auto-rollback. First deploy with no `.previous-tag` cannot
   auto-rollback.

`--dry-run` prints that sequence and does not pull, recreate, or write tag files.
`--skip-pull` sets `PULL_POLICY=never`.

Loud warning (does not auto-enable) when `SHOPWARE_DEPLOY_ENV=live` and
`COMPOSE_PROFILES` is empty. Recommended live: `redis,worker,scheduler`.

Passwords / `DATABASE_URL` are never logged. Auto-rollback on smoke uses the
shared compose/rollout helper (`fyrst-cli shopware rollback`).


## Exact import CLI

```text
fyrst-cli shopware db import --file <path.sql|.sql.gz> [--dry-run] [--allow-live]
```

Also:

```text
fyrst-cli shopware sync restore --data db [--snapshot-dir DIR] [--dry-run]
```

looks for `<snapshot-dir>/db.sql.gz` then `db.sql` and calls the same import
module. Default `--snapshot-dir` is `<shop>/var/runtime-sync`.

## `shopware init-env` (implemented)

Finish shop-root `.env` after `shopware-cli project create` + Flex. Maps to
recipes `deploy/init-env.sh`. Does **not** overwrite the whole file, invent
`MYSQL_*` passwords, or set `APP_URL`. This is **not** a dump command.

Resolves shop root (`COMPOSE_DIR` if set, else walk from cwd for `.env` or
`.env.example` + `deploy/`). `.env` is `KEY=VALUE` (quotes stripped, **no
shell expansion**), same parser as import (`src/shopware/envfile.rs`).

1. Copy `.env.example` → `.env` when `.env` is missing; merge **missing** keys
   from `.env.example` without clobbering existing values.
2. `--shop-id` is required unless `SHOPWARE_SHOP_ID` is already non-empty in
   `.env`. Slug: lowercase `[a-z0-9]([a-z0-9-]*[a-z0-9])?`.
3. `--env` is `live` | `staging` | `playground` | `dev`. Default `live` when
   unset/empty; keep an existing non-empty value.
4. `--image` sets `IMAGE` (no whitespace). Unset leaves `IMAGE` as-is.
5. `--vps` comments out uncommented `COMPOSE_PROJECT_NAME=…` lines (create
   footgun on a VPS). Does not leave an empty `COMPOSE_PROJECT_NAME=`.
6. `--generate-app-secret`: `openssl rand -hex 32` only if `APP_SECRET` is
   empty; the value is never printed.
7. `--dry-run` prints the summary and does not write `.env`.
8. After a real write: `chmod 600 .env`.

Refuses a bash xtrace equivalent (`SHELLOPTS=xtrace` / `BASH_XTRACEFD`) so
credentials in `.env` cannot leak via trace. Summary never prints secret
values (MYSQL passwords, `APP_SECRET`).

## Exact rollback CLI

```text
fyrst-cli shopware rollback [--dry-run] [--skip-pull]
```

## `shopware rollback` (implemented)

Resolves shop root (`COMPOSE_DIR` or walk from cwd for `.env` + `deploy/`),
loads `.env` then `.env.prod`. Process-env wins for `IMAGE`, `SMOKE_URL`,
`COMPOSE_PROFILES`, `SKIP_PULL`, `PULL_POLICY` when non-empty. **`IMAGE_TAG`
from the process environment is ignored** — the only tag is shop-root
`.previous-tag` (overlay `vps_read_previous_tag`). Missing or empty
`.previous-tag` exits 1 and does not guess `IMAGE_TAG`.

Requires `IMAGE`, `SHOPWARE_SHOP_ID`, `SHOPWARE_DEPLOY_ENV`, and the same
compose files as release:

`deploy/compose.yaml` + `compose.prod.yaml` + `compose.vps.yaml`

Rollout order reuses the release compose/rollout helper (never `--build`;
`run` uses `--pull never`, `up` uses `--no-build`):

1. Pull unless `--skip-pull` / `SKIP_PULL=1` / `PULL_POLICY=never`.
2. Start bundled `mysql` if present; start `redis` if present / profiled.
3. One-shot `compose --profile setup run --rm --pull never setup`.
4. `up -d --no-build` (optional `--pull never`) `--remove-orphans web`.
5. Extra `COMPOSE_PROFILES` (must not include `setup`).
6. Optional `SMOKE_URL`. Write `.deployed-tag` **only after** success (and
   smoke, if set). Smoke failure does not update `.deployed-tag`.

`--dry-run` prints that sequence and does not pull or recreate containers.
`--skip-pull` sets `PULL_POLICY=never` (same-host / air-gap).

## `shopware db import` (implemented)

Resolves shop root (`COMPOSE_DIR` or walk from cwd for `.env` + `deploy/`),
loads `.env` then `.env.prod` then `deploy/sync.env` (identity keys from the
process environment win when non-empty). Requires `SHOPWARE_SHOP_ID`.

1. If a compose `mysql` service is present: on execute,
   `docker compose … up -d --no-build mysql`, wait until pingable, then
   `gzip -dc FILE | docker compose --env-file .env -f … exec -T mysql sh -c
   '<mysql|mariadb>'` (plain `.sql` is stdin, not gzip).
2. Else `DATABASE_URL` to a real host: `gzip -dc FILE | docker run --rm -i
   --network host <mysql:8.4|mariadb:11.4> mysql <database>`. Override image
   with `SYNC_MYSQL_CLIENT_IMAGE`. Host `mysql` without a bundled service is
   refused.
3. `--dry-run` prints that pipeline **without** passwords / `MYSQL_PWD` /
   `DATABASE_URL` and does not call Docker.

Live consumer (`SHOPWARE_DEPLOY_ENV`, `SYNC_ENV`, checkout basename, or
hostname equal to `live`, case-insensitive):

| Command | Default | Override |
| --- | --- | --- |
| `db import` | refuse | `--allow-live` or `SYNC_ALLOW_LIVE_RESTORE=1` |
| `sync restore` | refuse (overlay rules) | `SYNC_ALLOW_LIVE_RESTORE=1` only |

Staging / playground / dev need no extra flag.

## `shopware sync snapshot` (volumes implemented; not a dump)

Copies bind-mount trees (`media`, `files`, `thumbnail`, `theme`, `sitemap`)
into `--snapshot-dir/data/<item>/`. Default `--snapshot-dir` is
`<shop>/var/runtime-sync`. Default `--from` is `local`. Default `--data` is
`db,media,files,thumbnail,theme,sitemap`.

This is **not** a dump command. fyrst-cli never runs
`docker run … shopware-cli project dump` and never wraps
`SYNC_DUMP_ENGINE=mysqldump`.

```text
fyrst-cli shopware sync snapshot [--from ALIAS] [--data LIST] [--snapshot-dir DIR] [--dry-run] [--skip-db] [--skip-volumes]
```

| `--data` | Behaviour |
| --- | --- |
| volumes only (`--skip-db` or `--data media,files,…`) | Copy trees; exit 0. `--dry-run` prints the rsync/tar plan and does not copy. |
| `db` only / `--skip-volumes` | Exit 2. Message to run `shopware-cli project dump` and import with `fyrst-cli shopware db import`. Not a silent success. |
| `all` (db + volumes) | Copy volumes **and** remind the operator to dump with shopware-cli into `--snapshot-dir/db.sql.gz`. Must not wrap dump. |
| `mysql_data` / `redis_data` | Refused (exit 1). |

Local: rsync (or a tree copy if rsync is missing) from
`$SYNC_DATA_ROOT` / `$SHOPWARE_DATA_ROOT` / derived
`$SHOPWARE_DATA_BASE/$SHOPWARE_SHOP_ID/$SHOPWARE_DEPLOY_ENV/<item>`
into `--snapshot-dir/data/<item>/`. If the bind-mount directory is missing,
named-volume fallback `${COMPOSE_PROJECT_NAME}_<item>` via
`SYNC_ARCHIVE_IMAGE` (default `alpine:3.20`) writes
`--snapshot-dir/volumes/<item>.tar.gz`.

Remote `--from <alias>`: SSH + rsync (or tar over SSH) from
`SYNC_REMOTE_DATA_ROOT` / `SYNC_<ALIAS>_DATA_ROOT` / derived
`$SHOPWARE_DATA_BASE/$SHOPWARE_SHOP_ID/$SYNC_SOURCE_ENV`. Requires
`SYNC_REMOTE_PATH` (or `SYNC_<ALIAS>_REMOTE_PATH`). Does **not** SSH-run
the overlay snapshot script (that would dump). Does not dump on the remote.

Object storage (S3) is out of scope. Dump remains shopware-cli.

## Environment (not clap flags)

Scripts and this CLI read shop identity and secrets from the environment /
shop-root `.env`. Names match the overlay:

| Area | Variables (non-exhaustive) |
| --- | --- |
| Shop identity | `COMPOSE_DIR`, `SHOPWARE_SHOP_ID`, `SHOPWARE_DEPLOY_ENV`, `SHOPWARE_DATA_BASE`, `SHOPWARE_DATA_ROOT`, `COMPOSE_PROJECT_NAME`, `SYNC_ENV` |
| Init-env | `APP_SECRET` (optional `--generate-app-secret`; never logged), `IMAGE` |
| Import | `MYSQL_DATABASE`, `DATABASE_URL`, `SYNC_MYSQL_CLIENT_IMAGE`, `SYNC_SNAPSHOT_DIR`, `SYNC_ALLOW_LIVE_RESTORE` |
| Dump (shopware-cli / overlay only) | `MYSQL_USER`, `MYSQL_PASSWORD`, `MYSQL_ROOT_PASSWORD`, `SYNC_SHOPWARE_CLI_IMAGE`, `SYNC_DUMP_ENGINE`, `SYNC_DUMP_QUICK`, `SYNC_DUMP_CLEAN`, `SYNC_DUMP_ANONYMIZE` |
| Release | `IMAGE`, `IMAGE_TAG` (release / `.env`; **ignored on rollback**), `COMPOSE_PROFILES`, `SMOKE_URL`, `PULL_POLICY`, `SKIP_PULL`, `ROLLBACK_ON_SMOKE_FAIL` |
| Rollback | `.previous-tag` is the only `IMAGE_TAG`; same optional env as release except `ROLLBACK_ON_SMOKE_FAIL` |
| Snapshot volumes | `SYNC_DATA_ROOT`, `SYNC_REMOTE_DATA_ROOT`, `SYNC_SOURCE_ENV`, `SYNC_REMOTE_PATH`, `SYNC_SSH_HOST`, `SYNC_SSH_USER`, `SYNC_SSH_PORT`, `SYNC_SSH_KEY`, `SYNC_ARCHIVE_IMAGE`, per-alias `SYNC_<ALIAS>_*` |
| Sync (future) | other `SYNC_*` (rewrite URL / map) |
| Backup (future) | `BACKUP_TARGET`, `BACKUP_KEEP_DAYS`, `BACKUP_SSH_KEY`, `BACKUP_ALLOW_LIVE_RESTORE`, `BACKUP_CONFIRM_RESTORE` |

## Future wrappers (not done here)

When implemented, remaining verbs should exec (or source-equivalent) the
overlay script with the parsed argv, rather than re-coding rewrite / rsync in
Rust. Recipe and `shopware-cd` product code stay out of this repo. Recipe bash
wrappers are **not** switched to `fyrst-cli` in this change. Dump remains
shopware-cli even after wrappers land. Rollback reuses the release
compose/rollout helper.
