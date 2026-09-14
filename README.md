# fyrst-cli

This is the fyrst.dev global CLI home (`fyrst-cli`).

Shopware CD operations live under the `shopware` namespace. Other fyrst
tooling can be added as sibling namespaces later.

License: MIT.

## Status

**Implemented:** `fyrst-cli shopware init-env` (shop-root `.env` after create
+ Flex) and `fyrst-cli shopware db import` — load a `.sql` or `.sql.gz`
dump into this shop's database. shopware-cli has no import; that is the gap
this CLI fills. `fyrst-cli shopware sync restore` uses the same import module
for `--data db` (`--snapshot-dir/db.sql.gz` or `db.sql`), restores bind-mount
volumes from `--snapshot-dir/data/<item>/` (or `volumes/<item>.tar.gz`),
stops/starts `web`/`worker`/`scheduler`, and runs opt-in
`fyrst:sales-channel:rewrite-urls` via compose `web`.
`fyrst-cli shopware release` — pull `IMAGE:IMAGE_TAG` and recreate the VPS
Compose stack (never builds images).

**Implemented:** `fyrst-cli shopware rollback` — re-deploy `IMAGE` using
`IMAGE_TAG` from `.previous-tag` (process-env `IMAGE_TAG` is ignored). Same
compose files and order as `deploy/vps-release.sh`. Writes `.deployed-tag`
only after success.

`fyrst-cli shopware sync snapshot` copies bind-mount / volume trees into
`--snapshot-dir/data/<item>/` (local and remote SSH `--from`). It does **not**
dump.

**Also implemented:** `fyrst-cli shopware sync local` (alias `sync-local`) — rsync VPS upload
trees into a local `shopware-cli` project-dev checkout (`./public/media/`,
`./files/`, …). Never the database. Never local `SHOPWARE_DATA_ROOT`.

**Also implemented:** `fyrst-cli shopware sync pull` (alias `sync sync`) — cron pull path: rsync
remote bind-mounts onto this host and import an already-present dump (it does
**not** dump).

**Also implemented:** `fyrst-cli shopware backup create` (alias `backup backup`) — copies bind-mount
trees (and an operator-provided `db.sql.gz`) into `BACKUP_TARGET`. Live is
allowed. It does **not** dump.

**Also implemented:** `fyrst-cli shopware backup prune` — stamp-based
retention under `BACKUP_TARGET` (`BACKUP_KEEP_DAYS`, default 14, `0` = keep
forever). Prune does not dump.

**Also implemented:** `fyrst-cli shopware backup restore` — disaster recovery
onto this host from a backup artifact (`--artifact` / `--stamp`, alias `--from`;
confirmation required; live needs `BACKUP_ALLOW_LIVE_RESTORE=1`). Inner apply
reuses sync restore. This CLI does not dump.

**Dump is not in fyrst-cli.** Database dumps are owned completely by
`shopware-cli project dump`. This CLI does not provide a dump command and
does not wrap or shell out to shopware-cli for dump. `--data db` on snapshot
exits 2 with that instruction.

Snapshot `--data db` still exits 2 (points operators at shopware-cli; not a
dump wrap).

This CLI does not reimplement dump or the
`fyrst:sales-channel:rewrite-urls` command itself (restore *calls* it via
compose `web`). Overlay scripts live in
[fyrst-dev/recipes](https://github.com/fyrst-dev/recipes) (`fyrst/shopware-cd`).
The console command lives in [fyrst-dev/shopware-cd](https://github.com/fyrst-dev/shopware-cd).

See [docs/ADR-0001-shopware-namespace.md](docs/ADR-0001-shopware-namespace.md),
[docs/command-matrix.md](docs/command-matrix.md), and
[docs/manual-db-import-test.md](docs/manual-db-import-test.md).

## Install

Linux amd64 (`x86_64-unknown-linux-gnu`) and arm64 (`aarch64-unknown-linux-gnu`)
binaries are published on [GitHub Releases](https://github.com/fyrst-dev/cli/releases).
Release tags look like `0.1.0` (no `v` prefix). macOS and other hosts: build from
source.

### Script (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/fyrst-dev/cli/main/scripts/install.sh | bash
```

Installs to `/usr/local/bin/fyrst-cli` (`PREFIX`, default `/usr/local`). `sudo`
is used only when that prefix is not writable. The script does not print tokens
or passwords.

```bash
# user-writable prefix (no sudo)
PREFIX="$HOME/.local" curl -fsSL https://raw.githubusercontent.com/fyrst-dev/cli/main/scripts/install.sh | bash

# pin a release
FYRST_CLI_VERSION=0.1.0 curl -fsSL https://raw.githubusercontent.com/fyrst-dev/cli/main/scripts/install.sh | bash
```

### Manual download

From [Releases](https://github.com/fyrst-dev/cli/releases), download the tarball
for your arch (`fyrst-cli-x86_64-unknown-linux-gnu.tar.gz` or
`fyrst-cli-aarch64-unknown-linux-gnu.tar.gz`) and `SHA256SUMS`:

```bash
curl -fsSL -O https://github.com/fyrst-dev/cli/releases/download/0.1.0/fyrst-cli-x86_64-unknown-linux-gnu.tar.gz
curl -fsSL -O https://github.com/fyrst-dev/cli/releases/download/0.1.0/SHA256SUMS
sha256sum -c SHA256SUMS --ignore-missing
tar -xzf fyrst-cli-x86_64-unknown-linux-gnu.tar.gz
sudo install -m 0755 fyrst-cli /usr/local/bin/fyrst-cli
```

### From source

```bash
cargo build --release
install -m 0755 target/release/fyrst-cli /usr/local/bin/fyrst-cli
```

Requires a Rust toolchain (edition 2021).

### Runtime

Docker is still required at runtime for `shopware db import` (Compose `mysql`
exec, or a one-shot mysql/mariadb client container) and for `shopware release`
/ `shopware rollback` (Compose pull + recreate). `shopware sync local` needs
`rsync` and an OpenSSH `ssh` client (BatchMode; no password prompts).
Installing this binary does not replace Docker.
Dump stays with `shopware-cli project dump`; fyrst-cli does not dump.

## Command tree

```
fyrst-cli shopware init-env            # shop-root .env finisher (this is real)
fyrst-cli shopware release            # VPS compose pull + recreate
fyrst-cli shopware rollback           # VPS rollback to .previous-tag
fyrst-cli shopware db import          # SQL import (this is real)
fyrst-cli shopware sync snapshot       # volumes; dump = shopware-cli
fyrst-cli shopware sync restore        # DB + volumes + opt-in rewrite
fyrst-cli shopware sync pull           # rsync + import (not dump); alias: sync sync
fyrst-cli shopware sync local          # VPS → project-dev rsync (never DB); alias: sync-local
fyrst-cli shopware backup create       # alias: backup backup
fyrst-cli shopware backup prune         # stamp-based retention
fyrst-cli shopware backup restore      # DR onto this host (confirmation)
```

| Overlay script | CLI |
| --- | --- |
| `deploy/init-env.sh` | `fyrst-cli shopware init-env` |
| `deploy/vps-release.sh` | `fyrst-cli shopware release` |
| `deploy/vps-rollback.sh` | `fyrst-cli shopware rollback` |
| — | `fyrst-cli shopware db import` |
| `deploy/sync-runtime.sh` | `fyrst-cli shopware sync {snapshot\|restore\|pull}` (`pull` alias: `sync`) |
| `deploy/sync-runtime-local.sh` | `fyrst-cli shopware sync local` (alias `sync-local`) |
| `deploy/backup-runtime.sh` | `fyrst-cli shopware backup {create\|prune\|restore}` (`create` alias: `backup`) |

```text
fyrst-cli shopware --help
fyrst-cli shopware init-env --help
fyrst-cli shopware db import --help
fyrst-cli shopware release --help
fyrst-cli shopware rollback --help
fyrst-cli shopware sync local --help
fyrst-cli shopware backup prune --help
```

## Dump (shopware-cli only)

Do **not** use fyrst-cli to dump. From a shop checkout with a running Compose
`mysql` (or `DATABASE_URL`):

```bash
shopware-cli project dump --skip-lock-tables --compression=gzip --output db.sql.gz
```

See the [Shopware CLI dump docs](https://developer.shopware.com/docs/products/tools/cli/project-commands/mysql-dump.html)
and Flex overlay `deploy/lib/sync-dump.sh`.

## Init env

Run from the shop checkout (directory with `.env` or `.env.example`), or set
`COMPOSE_DIR`. Finishes shop-root `.env` after create + Flex. Does not
overwrite the whole file. Does not invent MYSQL passwords or `APP_URL`.

```bash
# Print the plan (does not write .env; never prints APP_SECRET / MYSQL passwords)
fyrst-cli shopware init-env --shop-id acme --vps --dry-run

fyrst-cli shopware init-env --shop-id acme --env live --vps --image ghcr.io/example/acme
fyrst-cli shopware init-env --shop-id acme --generate-app-secret
```

`--shop-id` is required unless `SHOPWARE_SHOP_ID` is already set in `.env`.
`--env` defaults to `live` when empty and keeps an existing non-empty value.
`--vps` comments `COMPOSE_PROJECT_NAME=` (does not leave an empty assignment).
`--dry-run` does not write. A real write `chmod 600`s `.env`.

## Import

Run from the shop checkout (directory with `.env`), or set `COMPOSE_DIR`.

```bash
# Print the plan (never prints passwords)
fyrst-cli shopware db import --file /tmp/db.sql.gz --dry-run

# Import into bundled Compose mysql (preferred)
fyrst-cli shopware db import --file /tmp/db.sql.gz

# Uncompressed SQL
fyrst-cli shopware db import --file ./dump.sql

# Same import via sync restore (looks for <snapshot-dir>/db.sql.gz)
fyrst-cli shopware sync restore --data db --snapshot-dir /tmp/sw-snap --dry-run

# Bind-mount volumes from snapshot data/<item>/ (or volumes/<item>.tar.gz)
fyrst-cli shopware sync restore --data media --snapshot-dir /tmp/sw-snap --dry-run

# Cron pull on staging (volumes from live; DB only if db.sql.gz is already in --snapshot-dir)
fyrst-cli shopware sync pull --from live --skip-db --data media,files --dry-run
```

Behaviour (aligned with recipes `restore_db_local` / `restore_db_via_url`):

1. Resolve shop root (`COMPOSE_DIR` or walk from cwd for `.env` + `deploy/`).
2. If Compose has a `mysql` service: optional `docker compose … up -d --no-build mysql`,
   then `gzip -dc` (or the `.sql` file) piped into
   `docker compose … exec -T mysql` with the mysql/mariadb client inside the
   container.
3. Else `DATABASE_URL` to a real host: pipe into a one-shot
   `mysql:8.4` / `mariadb:11.4` client container (`SYNC_MYSQL_CLIENT_IMAGE`
   override), `--network host`. `DATABASE_URL` host `mysql` without a compose
   mysql service is refused.
4. `--dry-run` prints that plan and does not call Docker.

Required:

- `SHOPWARE_SHOP_ID` in `.env`
- Bundled Compose `mysql` **or** `DATABASE_URL` pointing at a real host

### Live hosts

`SHOPWARE_DEPLOY_ENV=live` (also `SYNC_ENV=live`, checkout directory named
`live`, or hostname `live`) is refused by default so a casual import cannot
silently trash production.

- **`db import`:** pass `--allow-live` or set `SYNC_ALLOW_LIVE_RESTORE=1`.
- **`sync restore` / `sync pull`:** same live detection as the overlay; override
  only with `SYNC_ALLOW_LIVE_RESTORE=1` (not `--allow-live`). Staging/playground/dev
  do not need extra flags. Opt-in URL rewrite after restore/sync is **never**
  allowed on live, even with `SYNC_ALLOW_LIVE_RESTORE=1`.
- **`backup restore`:** `SHOPWARE_DEPLOY_ENV=live` needs
  `BACKUP_ALLOW_LIVE_RESTORE=1`. Confirmation is always required
  (`--i-understand-this-restores-this-host` or `BACKUP_CONFIRM_RESTORE=1`).
  Inner apply then sets `SYNC_ALLOW_LIVE_RESTORE=1`.

Passwords (`MYSQL_PASSWORD`, `DATABASE_URL`) are never printed.

`.env` is parsed as `KEY=VALUE` (quotes stripped, **no shell expansion**).

## Release

Run from the shop checkout (directory with `.env`), or set `COMPOSE_DIR`.
Requires `IMAGE`, `IMAGE_TAG`, `SHOPWARE_SHOP_ID`, `SHOPWARE_DEPLOY_ENV`, and
`deploy/compose.yaml` + `compose.prod.yaml` + `compose.vps.yaml`.

```bash
# Print the compose sequence (no pull / recreate)
fyrst-cli shopware release --dry-run

# Same-host / air-gap: skip registry pull (PULL_POLICY=never)
fyrst-cli shopware release --skip-pull --dry-run

fyrst-cli shopware release
```

Process-env `IMAGE` / `IMAGE_TAG` win over `.env` (CI tags beat
`IMAGE_TAG=latest`). `--skip-pull` or `SKIP_PULL=1` / `PULL_POLICY=never`
skips `docker compose pull` and passes `--pull never` to `up`.

Never builds images. Setup is `compose --profile setup run --rm --pull never setup`
(Compose v5 dropped `--no-build` on `run`). `up` uses `--no-build`.

`.deployed-tag` is written only after setup/web succeed and optional `SMOKE_URL`
passes. On smoke failure the command prints
`IMAGE_TAG=$(cat .previous-tag) fyrst-cli shopware rollback`. Auto-rollback
runs for live by default (`ROLLBACK_ON_SMOKE_FAIL` unset → on when
`SHOPWARE_DEPLOY_ENV=live`); the command still exits 1 so CI does not treat the
new tag as live. `shopware rollback` is implemented and reads `IMAGE_TAG`
from `.previous-tag` (process-env `IMAGE_TAG` is ignored).

Live with empty `COMPOSE_PROFILES` prints a warning (does not auto-enable
`redis,worker,scheduler`). `setup` in `COMPOSE_PROFILES` is refused.

## Rollback

Run from the shop checkout (directory with `.env`), or set `COMPOSE_DIR`.
Requires a prior release that wrote `.previous-tag`.

```bash
# Print the compose sequence (does not pull or recreate)
fyrst-cli shopware rollback --dry-run

# Same-host / air-gap
fyrst-cli shopware rollback --skip-pull

fyrst-cli shopware rollback
```

`IMAGE` comes from env / `.env`. `IMAGE_TAG` is **only** `.previous-tag`
(process-env `IMAGE_TAG` is ignored). Missing or empty `.previous-tag` exits
non-zero. Optional env matches release: `COMPOSE_PROFILES`, `SMOKE_URL`,
`PULL_POLICY`, `SKIP_PULL`. `.deployed-tag` is written only after a successful
rollout (and smoke, if `SMOKE_URL` is set). Never builds images or compiles
themes/assets.

## sync local (VPS → laptop project-dev)

Run from the shop checkout (directory with `.env` + `public/` or
`composer.json`), or set `COMPOSE_DIR`. Pulls remote bind-mount trees into
**project-tree** paths. Does not restore the database. Does not write into
`SHOPWARE_DATA_ROOT`. Alias: `fyrst-cli shopware sync-local`.

```bash
# Print mappings; do not copy
fyrst-cli shopware sync local --dry-run

# Default --from live; default --data media,files,thumbnail,theme,sitemap
# (--data all is refused: on sync pull / backup create, all includes db)
fyrst-cli shopware sync local --from live

fyrst-cli shopware sync local --from live --data media,files
fyrst-cli shopware sync local --from live --delete
```

`--data db` is refused. After a successful pull:

```bash
shopware-cli project console cache:clear
```

See [docs/command-matrix.md](docs/command-matrix.md).

## Backup prune

Stamp-based retention. Not a dump. Requires `BACKUP_TARGET` (local path or
SSH) plus `SHOPWARE_SHOP_ID` and `SHOPWARE_DEPLOY_ENV`.

```bash
# Preview deletions (does not dump, copy, or delete)
fyrst-cli shopware backup prune --dry-run

# Delete stamp dirs older than BACKUP_KEEP_DAYS (default 14)
fyrst-cli shopware backup prune

# Keep forever
BACKUP_KEEP_DAYS=0 fyrst-cli shopware backup prune
```

Deletes `$BACKUP_TARGET/$SHOPWARE_SHOP_ID/$SHOPWARE_DEPLOY_ENV/YYYYMMDDTHHMMSSZ/`
when the stamp is older than the UTC cutoff. Other names are left alone.
`--data` is ignored (retention is stamp-based). `--dry-run` prints `rm -rf`
targets only. SSH targets use `ssh -o BatchMode=yes`.

## Backup restore

Disaster recovery onto **this host** (not live→staging sync). `--artifact` is a
stamp under `$BACKUP_TARGET/$SHOPWARE_SHOP_ID/$SHOPWARE_DEPLOY_ENV/` or a
directory path. `--stamp` and `--from` are aliases.

```bash
# Staging drill (print fetch + import/volume plan)
fyrst-cli shopware backup restore --artifact 20260912T020000Z \
  --i-understand-this-restores-this-host --dry-run

# Apply (overwrites DB and bind mounts on this host)
fyrst-cli shopware backup restore --artifact 20260912T020000Z \
  --i-understand-this-restores-this-host

# Live DR only
BACKUP_ALLOW_LIVE_RESTORE=1 fyrst-cli shopware backup restore --artifact 20260912T020000Z \
  --i-understand-this-restores-this-host
```

`--data db` uses the existing import module (`db.sql.gz` / `db.sql` in the
artifact). fyrst-cli does not dump. SSH `BACKUP_TARGET` rsyncs into
`var/backup-work/restore-<stamp>` then restores.

## Build

For local development (also see [Install](#install) for release binaries):

```bash
cargo build
cargo run -- shopware --help
cargo run -- shopware init-env --help
cargo run -- shopware db import --help
cargo run -- shopware release --help
cargo run -- shopware rollback --help
cargo run -- shopware sync local --help
cargo run -- shopware backup prune --help
cargo run -- shopware backup restore --help
cargo test
```

CI runs `cargo check` and `cargo test` on pull requests and `main`. Release
tags like `0.1.0` (no `v` prefix) build Linux tarballs via
`.github/workflows/release.yml`.
