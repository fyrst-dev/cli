# fyrst-cli

This is the fyrst.dev global CLI home (`fyrst-cli`).

Shopware CD operations live under the `shopware` namespace. Other fyrst
tooling can be added as sibling namespaces later.

License: MIT.

## Status

**Implemented:** `fyrst-cli shopware sync snapshot` for a **local database
dump**. It delegates to `shopware-cli project dump` in a one-shot container
(`ghcr.io/shopware/shopware-cli:0.18.4`, override with
`SYNC_SHOPWARE_CLI_IMAGE`), attached to the Compose network
`${COMPOSE_PROJECT_NAME}_default` (derived as
`${SHOPWARE_SHOP_ID}-${SHOPWARE_DEPLOY_ENV}` when `COMPOSE_PROJECT_NAME` is
unset). Output is `${SNAPSHOT_DIR}/db.sql.gz` (default
`<shop>/var/runtime-sync`).

**Still stub (exit 2):** bind-mount volume copy, remote SSH `--from <alias>`,
`SYNC_DUMP_ENGINE=mysqldump`, and every other `shopware` verb.

This CLI does not reimplement dump, MySQL import, Compose release, or
`fyrst:sales-channel:rewrite-urls`. Overlay scripts live in
[fyrst-dev/recipes](https://github.com/fyrst-dev/recipes) (`fyrst/shopware-cd`).
The console command lives in [fyrst-dev/shopware-cd](https://github.com/fyrst-dev/shopware-cd).

See [docs/ADR-0001-shopware-namespace.md](docs/ADR-0001-shopware-namespace.md),
[docs/command-matrix.md](docs/command-matrix.md), and
[docs/manual-snapshot-test.md](docs/manual-snapshot-test.md).

## Command tree

```
fyrst-cli shopware init-env
fyrst-cli shopware release
fyrst-cli shopware rollback
fyrst-cli shopware sync snapshot    # local DB dump (this is real)
fyrst-cli shopware sync restore
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
| `deploy/sync-runtime.sh` | `fyrst-cli shopware sync {snapshot\|restore\|sync}` |
| `deploy/sync-runtime-local.sh` | `fyrst-cli shopware sync-local` |
| `deploy/backup-runtime.sh` | `fyrst-cli shopware backup {backup\|prune\|restore}` |

```text
fyrst-cli shopware --help
fyrst-cli shopware sync snapshot --help
```

## Snapshot (local DB)

Run from the shop checkout (directory with `.env`), or set `COMPOSE_DIR`.

```bash
# Print the docker run line (no dump, never prints passwords)
fyrst-cli shopware sync snapshot --data db --dry-run

# Dump into <shop>/var/runtime-sync/db.sql.gz
fyrst-cli shopware sync snapshot --data db

# Custom work directory
fyrst-cli shopware sync snapshot --data db --snapshot-dir /tmp/sw-snap
```

Required for dump-only:

- `SHOPWARE_SHOP_ID` in `.env` (always)
- `SHOPWARE_DEPLOY_ENV` **or** `COMPOSE_PROJECT_NAME` (network name)
- Bundled Compose `mysql` service **or** `DATABASE_URL` pointing at a real host
- `MYSQL_USER`/`MYSQL_PASSWORD` (or `MYSQL_ROOT_PASSWORD`, or user/password in
  `DATABASE_URL`)

Dump flags match the Flex overlay (`deploy/lib/sync-dump.sh`):

| Env | Default | Effect |
| --- | --- | --- |
| `SYNC_DUMP_QUICK` | on (`1`) | `--quick`; set `0` to opt out |
| `SYNC_DUMP_CLEAN` | on (`1`) | `--clean`; set `0` to opt out |
| `SYNC_DUMP_ANONYMIZE` | off | `--anonymize` when `1` |
| `SYNC_SHOPWARE_CLI_IMAGE` | `ghcr.io/shopware/shopware-cli:0.18.4` | one-shot image |
| `SYNC_DUMP_ENGINE` | `shopware-cli` | `mysqldump` is still a stub |

Default `--data` is `db,media,files,thumbnail,theme,sitemap`. Volume trees are
not copied yet: the DB dump still runs, and a `not implemented` line is printed
on stderr. Use `--data db` or `--skip-volumes` to dump without that warning.
`--from local` (default) only; other aliases error until SSH lands.

`.env` is parsed as `KEY=VALUE` (quotes stripped, **no shell expansion**).

## Build

```bash
cargo build
cargo run -- shopware --help
cargo run -- shopware sync snapshot --help
cargo test
```

Requires a Rust toolchain (edition 2021). CI runs `cargo check` and
`cargo test`.
