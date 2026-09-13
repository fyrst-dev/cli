# Manual test: local Shopware DB snapshot

Automated tests cover flag parsing and `--dry-run` argv (no Docker). This
checklist is for a real stack.

## Prerequisites

- Docker Engine + Compose v2, daemon reachable
- A shop checkout with shop-root `.env` and `deploy/compose.yaml` that includes
  service `mysql` (or `DATABASE_URL` to a reachable host)
- `SHOPWARE_SHOP_ID` and either `SHOPWARE_DEPLOY_ENV` or `COMPOSE_PROJECT_NAME`
- `MYSQL_USER` / `MYSQL_PASSWORD` (or `MYSQL_ROOT_PASSWORD`)
- Compose stack up enough that hostname `mysql` answers on
  `${COMPOSE_PROJECT_NAME}_default` (the CLI will try
  `docker compose up -d --no-build mysql`)

Build the CLI:

```bash
cargo build --release
export PATH="$PWD/target/release:$PATH"
```

## Dry-run (no dump)

From the shop root:

```bash
fyrst-cli shopware sync snapshot --data db --dry-run
```

Expect stdout `==> DRY-RUN docker run --rm --network <project>_default …`
with `--host mysql`, `--output=…/db.sql.gz`, image
`ghcr.io/shopware/shopware-cli:0.18.4`. Stdout/stderr must **not** contain
`MYSQL_PASSWORD` or `--password`.

## Dump

```bash
fyrst-cli shopware sync snapshot --data db --snapshot-dir /tmp/fyrst-snap
gzip -t /tmp/fyrst-snap/db.sql.gz
ls -l /tmp/fyrst-snap/db.sql.gz
```

Expect a non-empty gzip file. First run may `docker pull`
`ghcr.io/shopware/shopware-cli:0.18.4`.

Optional env (same as recipes):

```bash
SYNC_DUMP_ANONYMIZE=1 fyrst-cli shopware sync snapshot --data db --dry-run
SYNC_DUMP_QUICK=0 SYNC_DUMP_CLEAN=0 fyrst-cli shopware sync snapshot --data db --dry-run
SYNC_SHOPWARE_CLI_IMAGE=ghcr.io/shopware/shopware-cli:0.18.4 \
  fyrst-cli shopware sync snapshot --data db --dry-run
```

Default `--data` (includes media/files/…) dumps the DB and prints
`not implemented: bind-mount volume snapshot` on stderr. `--from live` should
exit 2 until SSH snapshot exists.

`SYNC_DUMP_ENGINE=mysqldump` is still stub (exit 2).
