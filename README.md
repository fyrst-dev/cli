# fyrst-cli

`fyrst-cli` is the fyrst.dev global CLI. Shopware CD operations live under
`shopware`. Other fyrst tooling can land as sibling namespaces later.

License: MIT.

**Dump = shopware-cli only. Import = fyrst-cli.** This CLI does not dump.

**sync** moves data between environments / a workdir. **backup** is off-host
disaster recovery. Do not use sync as a backup.

## Install

Linux amd64 (`x86_64-unknown-linux-gnu`) and arm64 (`aarch64-unknown-linux-gnu`)
binaries are on [GitHub Releases](https://github.com/fyrst-dev/cli/releases).
Tags look like `0.1.0` (no `v` prefix). macOS and other hosts: build from source.

### Script (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/fyrst-dev/cli/main/scripts/install.sh | bash
```

Installs to `/usr/local/bin/fyrst-cli` (`PREFIX`, default `/usr/local`). `sudo`
is used only when that prefix is not writable.

```bash
# user-writable prefix (no sudo)
curl -fsSL https://raw.githubusercontent.com/fyrst-dev/cli/main/scripts/install.sh | PREFIX="$HOME/.local" bash

# pin a release
curl -fsSL https://raw.githubusercontent.com/fyrst-dev/cli/main/scripts/install.sh | FYRST_CLI_VERSION=0.1.0 bash
```

### Manual download

From [Releases](https://github.com/fyrst-dev/cli/releases), download the tarball
for your arch and `SHA256SUMS`:

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

Docker is required at runtime for `db import`, `deploy release`, and
`deploy rollback`. `sync local`, `sync pull`, and SSH backup targets also need
`rsync` and an OpenSSH `ssh` client (BatchMode; no password prompts). Installing
the binary does not replace Docker.

## Commands

```
# env
fyrst-cli shopware env init              # finish shop-root .env after create + Flex

# deploy
fyrst-cli shopware deploy release         # pull IMAGE:IMAGE_TAG, recreate the VPS stack
fyrst-cli shopware deploy rollback        # re-deploy IMAGE from .previous-tag

# db
fyrst-cli shopware db import              # load a .sql / .sql.gz into this shop

# sync — between envs / workdir
fyrst-cli shopware sync capture           # copy bind-mount trees into a workdir
fyrst-cli shopware sync apply             # apply that workdir onto this host
fyrst-cli shopware sync pull              # rsync from --from + import an existing dump
fyrst-cli shopware sync local              # VPS → laptop project-dev (never DB)

# backup — off-host DR
fyrst-cli shopware backup create          # copy artifacts into BACKUP_TARGET
fyrst-cli shopware backup prune            # stamp-based retention
fyrst-cli shopware backup recover          # recover this host from an artifact
```

`fyrst-cli shopware --help` lists the same tree.

### Overlay wrappers

The Flex overlay still ships these `deploy/*.sh` names so CI and cron stay
unchanged. The files are stubs around a dispatcher; **filenames are
unchanged**. Implementation is this CLI. **Dump stays `shopware-cli project
dump`.** There is no `deploy/lib/sync-dump.sh`.

| Overlay stub | fyrst-cli |
| --- | --- |
| `deploy/init-env.sh` | `fyrst-cli shopware env init` |
| `deploy/vps-release.sh` | `fyrst-cli shopware deploy release` |
| `deploy/vps-rollback.sh` | `fyrst-cli shopware deploy rollback` |
| `deploy/sync-runtime.sh snapshot\|restore\|sync` | `fyrst-cli shopware sync capture\|apply\|pull` |
| `deploy/sync-runtime-local.sh` | `fyrst-cli shopware sync local` (`--data all` refused) |
| `deploy/backup-runtime.sh backup\|prune\|restore` | `fyrst-cli shopware backup create\|prune\|recover` |

Flags and env: [docs/command-matrix.md](docs/command-matrix.md).

### Dump (shopware-cli)

From a shop checkout with a running Compose `mysql` (or `DATABASE_URL`):

```bash
shopware-cli project dump --skip-lock-tables --compression=gzip --output db.sql.gz
```

See the [Shopware CLI dump docs](https://developer.shopware.com/docs/products/tools/cli/project-commands/mysql-dump.html).
`sync capture --data db` exits 2 with that same instruction — it does not dump.

### Live hosts

A live consumer (`SHOPWARE_DEPLOY_ENV=live`, also `SYNC_ENV=live`, a checkout
named `live`, or hostname `live`) is refused by default.

| Command | Gate |
| --- | --- |
| `db import` | `--allow-live` or `SYNC_ALLOW_LIVE_RESTORE=1` |
| `sync apply` / `sync pull` | `SYNC_ALLOW_LIVE_RESTORE=1` only |
| `backup recover` | `--i-understand-this-restores-this-host` (or `BACKUP_CONFIRM_RESTORE=1`), **and** `BACKUP_ALLOW_LIVE_RESTORE=1` when `SHOPWARE_DEPLOY_ENV=live` |

`backup create` / `backup prune` are allowed on live (that is the cron path).
Opt-in URL rewrite after apply is never allowed on live. Passwords are never
printed.

## Usage

Run from the shop checkout (directory with `.env`), or set `COMPOSE_DIR`.
`--dry-run` prints the plan and does not write, copy, import, or delete.

### env init

Does not overwrite the whole `.env`. Does not invent MYSQL passwords or
`APP_URL`. `--shop-id` is required unless `SHOPWARE_SHOP_ID` is already set.
`--env` defaults to `live`. `--vps` comments out `COMPOSE_PROJECT_NAME`.
`.env` may still be missing; missing keys are merged from `.env.example`.

```bash
fyrst-cli shopware env init --shop-id acme --vps --dry-run
fyrst-cli shopware env init --shop-id acme --env live --vps --image ghcr.io/example/acme
fyrst-cli shopware env init --shop-id acme --generate-app-secret
```

### db import

Needs `SHOPWARE_SHOP_ID` and a Compose `mysql` service (preferred) or
`DATABASE_URL` to a real host.

```bash
fyrst-cli shopware db import --file /tmp/db.sql.gz --dry-run
fyrst-cli shopware db import --file /tmp/db.sql.gz
fyrst-cli shopware db import --file ./dump.sql
```

### deploy release / rollback

Needs `IMAGE`, `IMAGE_TAG` (release), `SHOPWARE_SHOP_ID`, and
`SHOPWARE_DEPLOY_ENV`. Never builds images. Process-env `IMAGE` / `IMAGE_TAG`
win over `.env` on release. Rollback ignores process-env `IMAGE_TAG` and reads
`.previous-tag` only. `--skip-pull` is for same-host / air-gap.

```bash
fyrst-cli shopware deploy release --dry-run
fyrst-cli shopware deploy release --skip-pull
fyrst-cli shopware deploy release

fyrst-cli shopware deploy rollback --dry-run
fyrst-cli shopware deploy rollback --skip-pull
```

On live smoke failure, release prints
`IMAGE_TAG=$(cat .previous-tag) fyrst-cli shopware deploy rollback` and may
auto-rollback; it still exits 1. Live with empty `COMPOSE_PROFILES` warns
(recommended: `redis,worker,scheduler`).

### sync capture / apply / pull / local

`capture` copies bind-mount trees into `--snapshot-dir` (default
`<shop>/var/runtime-sync`). `apply` loads that workdir onto this host (DB
import + volumes). `pull` is the consumer cron path: rsync remote trees and
import a dump that is already present. `local` rsyncs VPS upload trees into
`./public/media/`, `./files/`, … — never the database, never local
`SHOPWARE_DATA_ROOT`. `--data all` is refused on `sync local` (on `pull` /
`backup create`, `all` includes db).

```bash
fyrst-cli shopware sync capture --from live --skip-db --dry-run
fyrst-cli shopware sync apply --data db --snapshot-dir /tmp/sw-snap --dry-run
fyrst-cli shopware sync apply --data media,files --snapshot-dir /tmp/sw-snap --dry-run
fyrst-cli shopware sync pull --from live --skip-db --data media,files --dry-run
fyrst-cli shopware sync local --from live
fyrst-cli shopware sync local --from live --data media,files --delete
```

After `sync local`: `shopware-cli project console cache:clear`.

### backup create / prune / recover

Needs `BACKUP_TARGET` (local path or SSH), `SHOPWARE_SHOP_ID`, and
`SHOPWARE_DEPLOY_ENV`. Create copies bind-mount trees and an operator-provided
dump (`BACKUP_DB_DUMP`) into a timestamped artifact — it does not dump. Prune
deletes stamp dirs older than `BACKUP_KEEP_DAYS` (default 14; `0` = keep
forever). Recover is disaster recovery onto **this host**, not live→staging
sync.

```bash
BACKUP_DB_DUMP=./db.sql.gz fyrst-cli shopware backup create --dry-run
BACKUP_DB_DUMP=./db.sql.gz fyrst-cli shopware backup create

fyrst-cli shopware backup prune --dry-run
BACKUP_KEEP_DAYS=0 fyrst-cli shopware backup prune

fyrst-cli shopware backup recover --artifact 20260912T020000Z \
  --i-understand-this-restores-this-host --dry-run
fyrst-cli shopware backup recover --artifact 20260912T020000Z \
  --i-understand-this-restores-this-host
BACKUP_ALLOW_LIVE_RESTORE=1 fyrst-cli shopware backup recover --artifact 20260912T020000Z \
  --i-understand-this-restores-this-host
```

## Contributing

```bash
cargo build
cargo test
```

Real-stack import checklist: [docs/manual-db-import-test.md](docs/manual-db-import-test.md).

## Docs

Flags and env: [docs/command-matrix.md](docs/command-matrix.md).
Architecture decisions: [ADR-0001](docs/ADR-0001-shopware-namespace.md),
[ADR-0002](docs/ADR-0002-shopware-lifecycle-regroup.md).
