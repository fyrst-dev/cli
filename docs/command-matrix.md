# Command matrix

Source of truth for script behaviour is the Flex overlay in
[fyrst-dev/recipes](https://github.com/fyrst-dev/recipes)
(`fyrst/shopware-cd/1.0/root/deploy/`). This CLI mirrors the public interface.

Delegated tools:

| Concern | Owner |
| --- | --- |
| **DB dump** | **`shopware-cli project dump` only.** fyrst-cli does not dump, wrap dump, or shell out to shopware-cli. |
| **DB import** | **`fyrst-cli shopware db import`** (MySQL/MariaDB client). shopware-cli has no import. |
| VPS release | **`fyrst-cli shopware deploy release`** (`docker compose` with `deploy/compose.yaml` + `compose.prod.yaml` + `compose.vps.yaml`) |
| VPS rollback | **`fyrst-cli shopware deploy rollback`** (same compose files; `IMAGE_TAG` from `.previous-tag`) |
| Opt-in URL rewrite after restore | `bin/console fyrst:sales-channel:rewrite-urls` via compose `web` |

## Map

| Overlay script | CLI | Nested verbs | Flags | Status |
| --- | --- | --- | --- | --- |
| `deploy/init-env.sh` | `fyrst-cli shopware env init` | `init` | `--shop-id`, `--env`, `--image`, `--vps`, `--generate-app-secret`, `--dry-run` | **implemented** |
| `deploy/vps-release.sh` | `fyrst-cli shopware deploy release` | `release` | `--dry-run`, `--skip-pull` | **implemented** |
| `deploy/vps-rollback.sh` | `fyrst-cli shopware deploy rollback` | `rollback` | `--dry-run`, `--skip-pull` | **implemented** |
| `restore_db_*` (sync-runtime) | `fyrst-cli shopware db import` | `import` | `--file`, `--dry-run`, `--allow-live` | **implemented** |
| `deploy/sync-runtime.sh` snapshot | `fyrst-cli shopware sync capture` | `capture` | `--from`, `--data`, `--snapshot-dir`, `--dry-run`, `--skip-db`, `--skip-volumes` | **volumes implemented** (bind-mount trees → `--snapshot-dir/data/<item>/`; named-volume tar fallback). **Not a dump command:** `--data db` exits 2 and tells operators to run `shopware-cli project dump` |
| `deploy/sync-runtime.sh` restore | `fyrst-cli shopware sync apply` | `apply` | same flags | **implemented**: DB via same import module; bind-mount volumes from `data/<item>/` or `volumes/<item>.tar.gz`; stop/start `web`/`worker`/`scheduler`; opt-in rewrite via compose `web`; live refuse unless `SYNC_ALLOW_LIVE_RESTORE=1`; rewrite never on live |
| `deploy/sync-runtime.sh` sync | `fyrst-cli shopware sync pull` | `pull` | same flags | **implemented** (pull orchestration): rsync remote bind-mounts; DB imports an already-present dump (does **not** dump). Live refuse unless `SYNC_ALLOW_LIVE_RESTORE=1` |
| `deploy/sync-runtime-local.sh` | `fyrst-cli shopware sync local` | `local` | `--from`, `--data`, `--remote-data-root`, `--delete`, `--dry-run` | **implemented** (VPS → local project-dev rsync; never DB; never `SHOPWARE_DATA_ROOT`; `--data all` refused) |
| `deploy/backup-runtime.sh` backup | `fyrst-cli shopware backup create` | `create` | `--data`, `--dry-run` | **implemented** (volumes + operator `db.sql.gz`; **not** a dump wrap; live allowed) |
| `deploy/backup-runtime.sh` prune | `fyrst-cli shopware backup prune` | `prune` | `--data` (ignored; stamp-based), `--dry-run` | **implemented** (retention under `BACKUP_TARGET`; not a dump) |
| `deploy/backup-runtime.sh` restore | `fyrst-cli shopware backup recover` | `recover` | `--data`, `--dry-run`, `--artifact` (aliases `--stamp`, `--from`), `--i-understand-this-restores-this-host` | **implemented** (fetch artifact; inner apply = sync apply module; no dump) |

Hard cut (old clap names removed): `init-env` → `env init`; top-level `release`/`rollback` → `deploy {release\|rollback}`; `sync snapshot` → `sync capture`; `sync restore` → `sync apply`; `sync sync` / `sync-local` / `backup backup` / `backup restore` → `sync pull` / `sync local` / `backup create` / `backup recover`. See [ADR-0002](ADR-0002-shopware-lifecycle-regroup.md).

`shopware env`, `shopware deploy`, `shopware sync`, `shopware backup`, and
`shopware db` with no verb require a subcommand (clap prints help, exit 2).

## Live policy

Refuse a live consumer unless the gate below is set. Three dialects on
purpose; this table is the operator cheat-sheet (also in `shopware --help`).

**sync = between environments / workdir. backup = off-host disaster recovery.**

| Command | Gate |
| --- | --- |
| `db import` | `--allow-live` or `SYNC_ALLOW_LIVE_RESTORE=1` |
| `sync apply` | `SYNC_ALLOW_LIVE_RESTORE=1` only |
| `sync pull` | `SYNC_ALLOW_LIVE_RESTORE=1` only |
| `backup recover` | confirm flag (`--i-understand-this-restores-this-host` or `BACKUP_CONFIRM_RESTORE=1`) **and** `BACKUP_ALLOW_LIVE_RESTORE=1` when `SHOPWARE_DEPLOY_ENV=live` |

Opt-in URL rewrite after apply is **never** allowed on
live, even with `SYNC_ALLOW_LIVE_RESTORE=1`. Staging / playground / dev need no
extra live flag for backup recover (still need confirmation).

## Exact env init CLI

```text
fyrst-cli shopware env init [--shop-id SLUG] [--env live|staging|playground|dev] [--image REPO] [--vps] [--generate-app-secret] [--dry-run]
```

## Exact deploy release CLI

```text
fyrst-cli shopware deploy release [--dry-run] [--skip-pull]
```

## `shopware deploy release` (implemented)

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
   `IMAGE_TAG=$(cat .previous-tag) fyrst-cli shopware deploy rollback`. Auto-run the
   shared rollout helper at the previous tag when `ROLLBACK_ON_SMOKE_FAIL` is
   on (unset → **on for live**, off otherwise). Release still **exits 1** after
   a successful auto-rollback. First deploy with no `.previous-tag` cannot
   auto-rollback.

`--dry-run` prints that sequence and does not pull, recreate, or write tag files.
`--skip-pull` sets `PULL_POLICY=never`.

Loud warning (does not auto-enable) when `SHOPWARE_DEPLOY_ENV=live` and
`COMPOSE_PROFILES` is empty. Recommended live: `redis,worker,scheduler`.

Passwords / `DATABASE_URL` are never logged. Auto-rollback on smoke uses the
shared compose/rollout helper (`fyrst-cli shopware deploy rollback`).


## Exact import CLI

```text
fyrst-cli shopware db import --file <path.sql|.sql.gz> [--dry-run] [--allow-live]
```

Also:

```text
fyrst-cli shopware sync apply --data db [--snapshot-dir DIR] [--dry-run]
```

looks for `<snapshot-dir>/db.sql.gz` then `db.sql` and calls the same import
module. Default `--snapshot-dir` is `<shop>/var/runtime-sync`. Bind-mount
volumes (`media`, `files`, `thumbnail`, `theme`, `sitemap`) restore from
`<snapshot-dir>/data/<item>/` (preferred) or
`<snapshot-dir>/volumes/<item>.tar.gz`.

## `shopware env init` (implemented)

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

## Exact deploy rollback CLI

```text
fyrst-cli shopware deploy rollback [--dry-run] [--skip-pull]
```

## `shopware deploy rollback` (implemented)

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

Cron/operator pull (run on the consumer, typically staging):

```text
fyrst-cli shopware sync pull --from live --data all
fyrst-cli shopware sync pull --from live --skip-db --data media,files,thumbnail,theme,sitemap --dry-run
```

`--from local` snapshots then restores the same host (pipeline check). Prefer
`--from <live-alias>` on staging.

## Exact backup recover CLI

```text
fyrst-cli shopware backup recover [--data LIST] [--dry-run] --artifact STAMP_OR_DIR \
  --i-understand-this-restores-this-host
```

`--artifact` (aliases `--stamp`, `--from`) is required (artifact timestamp under
`$BACKUP_TARGET/$SHOPWARE_SHOP_ID/$SHOPWARE_DEPLOY_ENV/` or a directory path).
Confirmation via `--i-understand-this-restores-this-host` **or**
`BACKUP_CONFIRM_RESTORE=1`. This overwrites DB and bind mounts on **this host**.
It is not `sync apply` (clone live→staging).

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
| `sync apply` | refuse (overlay rules) | `SYNC_ALLOW_LIVE_RESTORE=1` only |
| `sync pull` | refuse (overlay `assert_not_live_restore`) | `SYNC_ALLOW_LIVE_RESTORE=1` only |
| opt-in rewrite after apply | refuse on live | **none** — `SYNC_ALLOW_LIVE_RESTORE=1` does not bypass |
| `backup recover` | refuse when `SHOPWARE_DEPLOY_ENV=live` | `BACKUP_ALLOW_LIVE_RESTORE=1` (then inner apply sets `SYNC_ALLOW_LIVE_RESTORE=1`) |

Staging / playground / dev need no extra live flag for backup recover (still
need confirmation). Quarterly drill: restore onto staging, not live.

## `shopware backup recover` (implemented)

Disaster recovery from a backup artifact onto this host. Overlay
`deploy/backup-runtime.sh restore`. fyrst-cli does **not** dump.

1. Require `--artifact` (aliases `--stamp`, `--from`) and confirmation (`--i-understand-this-restores-this-host`
   or `BACKUP_CONFIRM_RESTORE=1`).
2. If `SHOPWARE_DEPLOY_ENV=live`, refuse unless `BACKUP_ALLOW_LIVE_RESTORE=1`.
3. Fetch the artifact:
   - `--artifact` directory path, or
   - local `$BACKUP_TARGET/<shop>/<env>/<stamp>/`, or
   - SSH `BACKUP_TARGET`: rsync into `<shop>/var/backup-work/restore-<stamp>`.
4. Set `SYNC_ALLOW_LIVE_RESTORE=1` and apply via the **same restore module** as
   `shopware sync apply` (`--data db` → existing import of `db.sql.gz` /
   `db.sql`; volumes from artifact `data/<item>/` then `volumes/<item>.tar.gz`).
5. `--dry-run` prints the fetch + inner restore plan and does not overwrite DB
   or bind mounts.
6. After a real restore, hint to rewrite sales-channel URLs if this is not a
   same-host drill (rewrite itself is still refused on live).

Loads `.env`, `.env.prod`, `deploy/sync.env`, then `deploy/backup.env`.
Process-env `BACKUP_*` and shop identity keys win when non-empty.

## `shopware sync apply` (volumes + orchestration)

Applies `--snapshot-dir` onto **this host**. `--from` is unused for local
apply (kept for clap compatibility with `sync capture` / `sync pull`).

1. Refuse a live consumer unless `SYNC_ALLOW_LIVE_RESTORE=1` (covers
   volume-only restore, not only db).
2. If `SYNC_REWRITE_APP_URL` or `SYNC_REWRITE_URL_MAP` is set, refuse when
   the consumer is live. Rewrite on live is impossible.
3. Stop running compose services `web`, `worker`, `scheduler`.
4. If `--data` includes db: existing import module (`db.sql.gz` then `db.sql`).
5. If rewrite was requested **and** db was restored: `docker compose … run
   --rm --pull never --entrypoint php web bin/console
   fyrst:sales-channel:rewrite-urls …`. If db was skipped, rewrite is skipped.
6. Restore selected bind-mount items into `SHOPWARE_DATA_ROOT` /
   `SYNC_DATA_ROOT` / derived `$SHOPWARE_DATA_BASE/$SHOPWARE_SHOP_ID/$SHOPWARE_DEPLOY_ENV`
   (`SHOPWARE_DATA_BASE` defaults to `/var/lib/shopware/data`). Prefer rsync
   from `data/<item>/`, else extract `volumes/<item>.tar.gz`, then `chown 82:82`.
   `--dry-run` prints rsync/extract/chown and does not copy.
7. Start previously stopped app services.
8. Non-fatal `cache:clear` when `IMAGE` is set; optional non-fatal
   `SYNC_POST_RESTORE_CMD`; hints when rewrite was off.

Passwords are never logged. Rewrite is not a Rust SQL rewriter.

## `shopware sync capture` (volumes implemented; not a dump)

Copies bind-mount trees (`media`, `files`, `thumbnail`, `theme`, `sitemap`)
into `--snapshot-dir/data/<item>/`. Default `--snapshot-dir` is
`<shop>/var/runtime-sync`. Default `--from` is `local`. Default `--data` is
`db,media,files,thumbnail,theme,sitemap`.

This is **not** a dump command. fyrst-cli never runs
`docker run … shopware-cli project dump` and never wraps
`SYNC_DUMP_ENGINE=mysqldump`.

```text
fyrst-cli shopware sync capture [--from ALIAS] [--data LIST] [--snapshot-dir DIR] [--dry-run] [--skip-db] [--skip-volumes]
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

## `shopware sync pull` (implemented)

Overlay `do_sync` minus dump wrap. Run on the consumer (staging / playground /
dev). Typical cron: `--from live --data all`.

1. Live refuse (same as overlay `assert_not_live_restore`) unless
   `SYNC_ALLOW_LIVE_RESTORE=1`. Rewrite-on-live is impossible even with that
   flag (`SYNC_REWRITE_APP_URL` / `SYNC_REWRITE_URL_MAP` must be unset on live).
2. SSH probe (`SYNC_SSH_*`; dry-run prints the probe and does not connect).
3. Stop `web` / `worker` / `scheduler` if running (dry-run prints the plan).
4. **DB:** fyrst-cli **does not dump** and does not SSH a dump wrapper. If
   `--data` includes `db`, import `<snapshot-dir>/db.sql.gz` (then `db.sql`)
   with the existing import module. If no dump is present, exit 1 with
   operator instructions to run `shopware-cli project dump` on the source and
   place `db.sql.gz` for import (`fyrst-cli shopware db import --file` is the
   same module). `--skip-db` skips this leg.
5. Opt-in `fyrst:sales-channel:rewrite-urls` when rewrite env is set **and** a
   dump was imported. Skipped when `--skip-db` / no dump imported.
6. Volume leg: rsync `SYNC_REMOTE_DATA_ROOT/<item>/` → local
   `$DATA_ROOT/<item>/` (tar/docker fallback if rsync cannot write), then
   `chown 82:82`, matching `sync_bind_from_remote`. `--dry-run` prints rsync
   without copying.
7. Start previously stopped app services; non-fatal cache:clear /
   `SYNC_POST_RESTORE_CMD` hints.

`--from local` is documented as a capture+apply pipeline check (rsync
through `--snapshot-dir`), not the cron path.

SSH env (not clap): `SYNC_SSH_HOST` (default `--from` alias), `SYNC_SSH_USER`,
`SYNC_SSH_PORT`, `SYNC_SSH_KEY`, `SYNC_REMOTE_PATH`, `SYNC_REMOTE_DATA_ROOT`,
per-alias `SYNC_<ALIAS>_SSH_*` / `SYNC_<ALIAS>_DATA_ROOT`. Passwords are never
logged.

## Exact `sync local` CLI

```text
fyrst-cli shopware sync local [--from ALIAS] [--data LIST] [--remote-data-root PATH] [--delete] [--dry-run]
```

## `shopware sync local` (implemented)

Pull live VPS upload trees into a **laptop `shopware-cli` project-dev**
checkout. Matches recipes `deploy/sync-runtime-local.sh`. This command
**never** restores the database and **never** writes into local
`SHOPWARE_DATA_ROOT` / `SYNC_DATA_ROOT` (those are VPS bind-mount roots).
VPS→VPS including DB is `shopware sync pull`, not this command.

Default `--from` is `live`. Default `--data` (when omitted) is
`media,files,thumbnail,theme,sitemap`. `--data all` is **refused** so `all`
never implies db on this verb (on `sync pull` / `backup create`, `all`
includes db).

Path remap (remote `$REMOTE_DATA_ROOT` → local shopware-cli project tree):

| Remote | Local |
| --- | --- |
| `media/` | `./public/media/` |
| `files/` | `./files/` |
| `thumbnail/` | `./public/thumbnail/` |
| `theme/` | `./public/theme/` |
| `sitemap/` | `./public/sitemap/` |

1. Resolve shop root (`COMPOSE_DIR` or walk from cwd for `.env` + `deploy/`).
   Require `public/` or `composer.json`.
2. Load shop-root `.env` then `deploy/sync.env` if present (identity keys from
   the process environment win when non-empty). Does not load `.env.prod`.
3. Remote root: `--remote-data-root` / `SYNC_REMOTE_DATA_ROOT` /
   per-alias `SYNC_<ALIAS>_DATA_ROOT`, else
   `$SHOPWARE_DATA_BASE/$SHOPWARE_SHOP_ID/$SYNC_SOURCE_ENV` with
   `SYNC_SOURCE_ENV` default `live` and `SHOPWARE_DATA_BASE` default
   `/var/lib/shopware/data`. `SHOPWARE_SHOP_ID` is required unless an
   explicit remote root is set.
4. SSH: BatchMode, no password prompts. Host/user/port/key from
   `SYNC_SSH_*` / `SYNC_<ALIAS>_SSH_*` (host falls back to `--from`).
5. `--data db` / `database` / `mysql` is refused (dump/import separately:
   `shopware-cli project dump` + `fyrst-cli shopware db import`).
   `--data all` is refused (on `sync pull` / `backup create`, `all` includes
   db). `--data mysql_data` / `redis_data` refused.
6. `--delete` is opt-in (`rsync --delete`). Default keeps extra local files.
   rsync is `-azH` and **omits `--numeric-ids`** so files are owned by the
   local user, not VPS uid 82.
7. `--dry-run` prints each `host:remote/item/ → ./public/…` (or `./files/`)
   mapping and the rsync line; skips SSH probe / copy. Still requires
   `rsync` and `ssh` on PATH.
8. Warns if the checkout directory is named `live`. After success (and
   dry-run): reminder `shopware-cli project console cache:clear` (not
   implemented here).

Requires `rsync` and an OpenSSH `ssh` client.

## `shopware sync pull` (implemented)

Overlay `do_sync` minus dump wrap. Run on the consumer (staging / playground /
dev). Typical cron: `--from live --data all`.

1. Live refuse (same as overlay `assert_not_live_restore`) unless
   `SYNC_ALLOW_LIVE_RESTORE=1`. Rewrite-on-live is impossible even with that
   flag (`SYNC_REWRITE_APP_URL` / `SYNC_REWRITE_URL_MAP` must be unset on live).
2. SSH probe (`SYNC_SSH_*`; dry-run prints the probe and does not connect).
3. Stop `web` / `worker` / `scheduler` if running (dry-run prints the plan).
4. **DB:** fyrst-cli **does not dump** and does not SSH a dump wrapper. If
   `--data` includes `db`, import `<snapshot-dir>/db.sql.gz` (then `db.sql`)
   with the existing import module. If no dump is present, exit 1 with
   operator instructions to run `shopware-cli project dump` on the source and
   place `db.sql.gz` for import (`fyrst-cli shopware db import --file` is the
   same module). `--skip-db` skips this leg.
5. Opt-in `fyrst:sales-channel:rewrite-urls` when rewrite env is set **and** a
   dump was imported. Skipped when `--skip-db` / no dump imported.
6. Volume leg: rsync `SYNC_REMOTE_DATA_ROOT/<item>/` → local
   `$DATA_ROOT/<item>/` (tar/docker fallback if rsync cannot write), then
   `chown 82:82`, matching `sync_bind_from_remote`. `--dry-run` prints rsync
   without copying.
7. Start previously stopped app services; non-fatal cache:clear /
   `SYNC_POST_RESTORE_CMD` hints.

`--from local` is documented as a capture+apply pipeline check (rsync
through `--snapshot-dir`), not the cron path.

SSH env (not clap): `SYNC_SSH_HOST` (default `--from` alias), `SYNC_SSH_USER`,
`SYNC_SSH_PORT`, `SYNC_SSH_KEY`, `SYNC_REMOTE_PATH`, `SYNC_REMOTE_DATA_ROOT`,
per-alias `SYNC_<ALIAS>_SSH_*` / `SYNC_<ALIAS>_DATA_ROOT`. Passwords are never
logged.

## Exact backup CLI

```text
fyrst-cli shopware backup create [--data LIST] [--dry-run]
```

Default `--data`: `db,media,files,thumbnail,theme,sitemap`.

Required env: `BACKUP_TARGET` (local path, second disk, or SSH
`user@host:/path` / `ssh://user@host:22/abs/path`). Also `BACKUP_KEEP_DAYS`
(default 14, `0` = keep forever), `BACKUP_SSH_KEY`, `BACKUP_SSH_PORT`.
Identity: `SHOPWARE_SHOP_ID`, `SHOPWARE_DEPLOY_ENV`. Optional
`BACKUP_DB_DUMP` = path to an already-made `db.sql.gz` (or `.sql`) to copy
into the artifact. Overlay `deploy/backup.env` is loaded; process-env
`BACKUP_*` values win when non-empty.

Layout on `BACKUP_TARGET`:

```text
$BACKUP_TARGET/$SHOPWARE_SHOP_ID/$SHOPWARE_DEPLOY_ENV/YYYYMMDDTHHMMSSZ/
  db.sql.gz                 # operator / shopware-cli, not fyrst-cli dump
  data/media/ …
  BACKUP_MANIFEST.txt
  SHA256SUMS                # when sha256sum exists
```

## `shopware backup create` (implemented)

Off-host backup of runtime artifacts. Sync is **not** a backup. Live
(`SHOPWARE_DEPLOY_ENV=live`) is allowed and expected (cron on live).

Divergence from overlay `backup-runtime.sh`: the overlay execs
`sync-runtime.sh snapshot` (which dumps). fyrst-cli **never** wraps
`shopware-cli project dump` and never shells out to shopware-cli.

1. Resolve shop root; load `.env`, `.env.prod`, `deploy/sync.env`,
   `deploy/backup.env`. Require `SHOPWARE_SHOP_ID`, `SHOPWARE_DEPLOY_ENV`,
   `BACKUP_TARGET`.
2. Lock overlapping runs (`var/backup-runtime.lock` / flock).
3. Copy selected bind-mount trees from
   `$SHOPWARE_DATA_ROOT/<item>/` (or derived
   `$SHOPWARE_DATA_BASE/$SHOPWARE_SHOP_ID/$SHOPWARE_DEPLOY_ENV`) into
   `data/<item>/`. Missing bind-mounts fall back to a named-volume tar
   (`${COMPOSE_PROJECT_NAME}_<item>`, `SYNC_ARCHIVE_IMAGE`, default
   `alpine:3.20`). Local target writes the timestamped dir in place; SSH
   target builds under `<shop>/var/backup-work/<stamp>` then rsyncs.
4. If `--data` includes `db`: **do not dump**. `--dry-run` prints
   “run shopware-cli project dump yourself”. Execute copies `BACKUP_DB_DUMP`
   into the artifact as `db.sql.gz` (or `db.sql`). Missing dump **fails**
   (exit 1) with that instruction — it does not wrap shopware-cli.
5. Write `BACKUP_MANIFEST.txt` and `SHA256SUMS` (when `sha256sum` exists).
6. After a successful artifact, run the in-process retention helper
   (same stamp cutoff as overlay `prune_artifacts`: delete
   `YYYYMMDDTHHMMSSZ` dirs older than `BACKUP_KEEP_DAYS` UTC days;
   `0` keeps forever; non-matching names are left alone). Standalone
   `fyrst-cli shopware backup prune` is the same retention path.

`--dry-run` prints the stamp path, volume copy plan, dump instruction (if
db selected), manifest/checksum plan, and prune actions. It does not copy,
dump, SSH, or delete.

## Exact prune CLI

```text
fyrst-cli shopware backup prune [--data LIST] [--dry-run]
```

`--data` is accepted for overlay flag parity. Prune deletes **artifact
directories by timestamp name**, not by `--data` subset.

## `shopware backup prune` (implemented)

Stamp-based retention under
`$BACKUP_TARGET/$SHOPWARE_SHOP_ID/$SHOPWARE_DEPLOY_ENV/`. Not a dump
command and does not wrap `shopware-cli`. Creating artifacts is
`shopware backup create`.

Resolves shop root (`COMPOSE_DIR` or walk from cwd for `.env` + `deploy/`),
loads `.env` then `.env.prod` then `deploy/sync.env` then `deploy/backup.env`.
Identity keys and `BACKUP_TARGET` / `BACKUP_KEEP_DAYS` / `BACKUP_SSH_KEY` /
`BACKUP_SSH_PORT` from the process environment win when non-empty.
Requires `SHOPWARE_SHOP_ID`, `SHOPWARE_DEPLOY_ENV`, and `BACKUP_TARGET`.

1. Parse `BACKUP_TARGET` as a local path, `user@host:/path` / `host:/path`,
   or `ssh://user@host:22/abs/path`. Missing `BACKUP_TARGET` fails with an
   operator error (exit 1), not “not implemented”.
2. `BACKUP_KEEP_DAYS` default 14. `0` logs that all artifacts are kept and
   returns without listing or deleting.
3. List names under the shop/env prefix. Delete those whose name matches
   `YYYYMMDDTHHMMSSZ` **and** is older than the UTC cutoff (`stamp < now
   minus keep days`, same string compare as overlay `date -u -d`). Names
   that are not that stamp form are left alone. A stamp equal to the cutoff
   is kept.
4. `--dry-run` prints `rm -rf` targets and does not delete.
5. SSH targets use `ssh -o BatchMode=yes -o ConnectTimeout=15` (`-p` from
   `BACKUP_SSH_PORT` or the `ssh://` URL; optional `-i BACKUP_SSH_KEY`) and
   remote `rm -rf`.

Live is allowed (this is retention, not sync apply).

## Environment (not clap flags)

Scripts and this CLI read shop identity and secrets from the environment /
shop-root `.env`. Names match the overlay:

| Area | Variables (non-exhaustive) |
| --- | --- |
| Shop identity | `COMPOSE_DIR`, `SHOPWARE_SHOP_ID`, `SHOPWARE_DEPLOY_ENV`, `SHOPWARE_DATA_BASE`, `SHOPWARE_DATA_ROOT`, `COMPOSE_PROJECT_NAME`, `SYNC_ENV` |
| Env init | `APP_SECRET` (optional `--generate-app-secret`; never logged), `IMAGE` |
| Import | `MYSQL_DATABASE`, `DATABASE_URL`, `SYNC_MYSQL_CLIENT_IMAGE`, `SYNC_SNAPSHOT_DIR`, `SYNC_ALLOW_LIVE_RESTORE` |
| Restore volumes | `SHOPWARE_DATA_ROOT`, `SYNC_DATA_ROOT`, `SHOPWARE_DATA_BASE`, `SYNC_ARCHIVE_IMAGE` |
| Opt-in rewrite / post-restore | `SYNC_REWRITE_APP_URL`, `SYNC_REWRITE_URL_MAP`, `SYNC_POST_RESTORE_CMD`, `IMAGE`, `APP_URL`, `SYNC_APP_URL` |
| Dump (shopware-cli / overlay only) | `MYSQL_USER`, `MYSQL_PASSWORD`, `MYSQL_ROOT_PASSWORD`, `SYNC_SHOPWARE_CLI_IMAGE`, `SYNC_DUMP_ENGINE`, `SYNC_DUMP_QUICK`, `SYNC_DUMP_CLEAN`, `SYNC_DUMP_ANONYMIZE` |
| Release | `IMAGE`, `IMAGE_TAG` (release / `.env`; **ignored on rollback**), `COMPOSE_PROFILES`, `SMOKE_URL`, `PULL_POLICY`, `SKIP_PULL`, `ROLLBACK_ON_SMOKE_FAIL` |
| Rollback | `.previous-tag` is the only `IMAGE_TAG`; same optional env as release except `ROLLBACK_ON_SMOKE_FAIL` |
| Sync capture volumes | `SYNC_DATA_ROOT`, `SYNC_REMOTE_DATA_ROOT`, `SYNC_SOURCE_ENV`, `SYNC_REMOTE_PATH`, `SYNC_SSH_HOST`, `SYNC_SSH_USER`, `SYNC_SSH_PORT`, `SYNC_SSH_KEY`, `SYNC_ARCHIVE_IMAGE`, per-alias `SYNC_<ALIAS>_*` |
| Sync local | `SYNC_SSH_HOST`, `SYNC_SSH_USER`, `SYNC_SSH_PORT`, `SYNC_SSH_KEY`, `SYNC_REMOTE_DATA_ROOT`, `SYNC_SOURCE_ENV`, `SYNC_<ALIAS>_SSH_*`, `SYNC_<ALIAS>_DATA_ROOT` |
| Sync pull | `SYNC_SSH_HOST`, `SYNC_SSH_USER`, `SYNC_SSH_PORT`, `SYNC_SSH_KEY`, `SYNC_REMOTE_PATH`, `SYNC_REMOTE_DATA_ROOT`, `SYNC_SOURCE_ENV`, `SYNC_DATA_ROOT`, `SYNC_ARCHIVE_IMAGE`, per-alias `SYNC_<ALIAS>_SSH_*` / `SYNC_<ALIAS>_REMOTE_PATH` / `SYNC_<ALIAS>_DATA_ROOT` |
| Sync rewrite | `SYNC_REWRITE_APP_URL`, `SYNC_REWRITE_URL_MAP`, `SYNC_APP_URL`, `SYNC_POST_RESTORE_CMD` |
| Backup | `BACKUP_TARGET`, `BACKUP_KEEP_DAYS` (default 14, `0` = forever), `BACKUP_SSH_KEY`, `BACKUP_SSH_PORT`, `BACKUP_DB_DUMP`, `BACKUP_ALLOW_LIVE_RESTORE`, `BACKUP_CONFIRM_RESTORE` |

## Ownership

The six overlay scripts are implemented in-process in this CLI. Dump remains
`shopware-cli project dump` forever — fyrst-cli does not dump, wrap dump, or
shell out to shopware-cli. Recipe and `shopware-cd` product code stay out of
this repo. Recipe bash is not switched to `fyrst-cli` in this change. Do not
reimplement rewrite in SQL; `sync apply` calls compose `web` console.
Rollback reuses the release compose/rollout helper.
