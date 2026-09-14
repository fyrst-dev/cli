# Manual test: database import

Automated tests cover flag parsing and `--dry-run` plans (no Docker). This
checklist is for a real stack.

Dump a database with **shopware-cli**, not fyrst-cli:

```bash
shopware-cli project dump --skip-lock-tables --compression=gzip --output /tmp/db.sql.gz
```

## Prerequisites

- Docker Engine + Compose v2, daemon reachable
- A shop checkout with shop-root `.env` and `deploy/compose.yaml` that includes
  service `mysql` (or `DATABASE_URL` to a reachable host)
- `SHOPWARE_SHOP_ID`
- A `.sql` or `.sql.gz` dump file (from shopware-cli, not from fyrst-cli)
- Compose stack up enough that service `mysql` can start (the CLI will try
  `docker compose up -d --no-build mysql`)

Build the CLI:

```bash
cargo build --release
export PATH="$PWD/target/release:$PATH"
```

## Dry-run (no import)

From the shop root, with `SHOPWARE_DEPLOY_ENV` **not** `live`:

```bash
fyrst-cli shopware db import --file /tmp/db.sql.gz --dry-run
```

Expect stdout `==> DRY-RUN gzip -dc … | docker compose --env-file .env -f … exec -T mysql sh -c '<mysql|mariadb>'`
(or `docker run mysql:8.4 mysql <db>` for an external `DATABASE_URL`).
Stdout/stderr must **not** contain `MYSQL_PASSWORD`, `MYSQL_PWD`, or the
password from `DATABASE_URL`.

## Import

```bash
fyrst-cli shopware db import --file /tmp/db.sql.gz
```

Same file via sync apply:

```bash
mkdir -p /tmp/sw-snap
cp /tmp/db.sql.gz /tmp/sw-snap/db.sql.gz
fyrst-cli shopware sync apply --data db --snapshot-dir /tmp/sw-snap --dry-run
```

## Live refuse

With `SHOPWARE_DEPLOY_ENV=live` in `.env` (or a checkout/hostname named `live`):

```bash
fyrst-cli shopware db import --file /tmp/db.sql.gz --dry-run
# expect exit 1, message about --allow-live / SYNC_ALLOW_LIVE_RESTORE

fyrst-cli shopware db import --file /tmp/db.sql.gz --dry-run --allow-live
# expect WARNING and a DRY-RUN plan

fyrst-cli shopware sync apply --data db --dry-run
# expect exit 1 unless SYNC_ALLOW_LIVE_RESTORE=1
```

## Confirm dump is gone from fyrst-cli

```bash
fyrst-cli shopware --help
fyrst-cli shopware sync capture --help
fyrst-cli shopware sync capture --data db --dry-run
```

`--help` must not present capture as a dump implementation. `--data db`
must exit 2 and mention `shopware-cli project dump` (no `docker run … project dump`).
