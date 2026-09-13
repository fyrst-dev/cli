# fyrst-cli

This is the fyrst.dev global CLI home (`fyrst-cli`).

Shopware CD operations live under the `shopware` namespace. Other fyrst
tooling can be added as sibling namespaces later.

License: MIT.

## Status

**Implemented:** `fyrst-cli shopware db import` — load a `.sql` or `.sql.gz`
dump into this shop's database. shopware-cli has no import; this is the gap
this CLI fills. `fyrst-cli shopware sync restore --data db` uses the same
import module (`--snapshot-dir/db.sql.gz` or `db.sql`).

**Dump is not in fyrst-cli.** Database dumps are owned completely by
`shopware-cli project dump`. This CLI does not provide a dump command and
does not wrap or shell out to shopware-cli for dump.

**Still stub (exit 2):** `shopware sync snapshot` (points operators at
shopware-cli for DB; bind-mount volumes remain stub), bind-mount volume
restore, remote SSH `--from`, and every other `shopware` verb.

This CLI does not reimplement dump, Compose release, or
`fyrst:sales-channel:rewrite-urls`. Overlay scripts live in
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
exec, or a one-shot mysql/mariadb client container). Installing this binary
does not replace Docker. Dump stays with `shopware-cli project dump`; fyrst-cli
does not dump.

## Command tree

```
fyrst-cli shopware init-env
fyrst-cli shopware release
fyrst-cli shopware rollback
fyrst-cli shopware db import          # SQL import (this is real)
fyrst-cli shopware sync snapshot       # not a dump; use shopware-cli
fyrst-cli shopware sync restore        # DB path = same import module
fyrst-cli shopware sync sync
fyrst-cli shopware sync-local
fyrst-cli shopware backup backup
fyrst-cli shopware backup prune
fyrst-cli shopware backup restore
```

| Overlay script | CLI |
| --- | --- |
| `deploy/init-env.sh` | `fyrst-cli shopware init-env` |
| `deploy/vps-release.sh` | `fyrst-cli shopware release` |
| `deploy/vps-rollback.sh` | `fyrst-cli shopware rollback` |
| — | `fyrst-cli shopware db import` |
| `deploy/sync-runtime.sh` | `fyrst-cli shopware sync {snapshot\|restore\|sync}` |
| `deploy/sync-runtime-local.sh` | `fyrst-cli shopware sync-local` |
| `deploy/backup-runtime.sh` | `fyrst-cli shopware backup {backup\|prune\|restore}` |

```text
fyrst-cli shopware --help
fyrst-cli shopware db import --help
```

## Dump (shopware-cli only)

Do **not** use fyrst-cli to dump. From a shop checkout with a running Compose
`mysql` (or `DATABASE_URL`):

```bash
shopware-cli project dump --skip-lock-tables --compression=gzip --output db.sql.gz
```

See the [Shopware CLI dump docs](https://developer.shopware.com/docs/products/tools/cli/project-commands/mysql-dump.html)
and Flex overlay `deploy/lib/sync-dump.sh`.

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
- **`sync restore`:** same live detection as the overlay; override only with
  `SYNC_ALLOW_LIVE_RESTORE=1` (not `--allow-live`). Staging/playground/dev
  do not need extra flags.

Passwords (`MYSQL_PASSWORD`, `DATABASE_URL`) are never printed.

`.env` is parsed as `KEY=VALUE` (quotes stripped, **no shell expansion**).

## Build

For local development (also see [Install](#install) for release binaries):

```bash
cargo build
cargo run -- shopware --help
cargo run -- shopware db import --help
cargo test
```

CI runs `cargo check` and `cargo test` on pull requests and `main`. Release
tags like `0.1.0` (no `v` prefix) build Linux tarballs via
`.github/workflows/release.yml`.
