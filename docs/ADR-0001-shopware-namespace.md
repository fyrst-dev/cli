# ADR 0001: Shopware CD lives under `fyrst-cli shopware`

- Status: Accepted (namespace, dump/import split, live guards).
  Command **names** superseded by [ADR-0002](ADR-0002-shopware-lifecycle-regroup.md).
- Date: 2026-09-13

## Context

`fyrst-cli` is the fyrst.dev global CLI home. Shopware CD today is a set of
bash scripts copied into each shop by the Flex overlay in
[fyrst-dev/recipes](https://github.com/fyrst-dev/recipes)
(`fyrst/shopware-cd/1.0/root/deploy/`):

| Script | Role |
| --- | --- |
| `init-env.sh` | Finish shop-root `.env` after create + Flex |
| `vps-release.sh` | Pull `IMAGE:IMAGE_TAG`, recreate the VPS Compose stack |
| `vps-rollback.sh` | Roll back to `.previous-tag` with the same compose order |
| `sync-runtime.sh` | `snapshot` / `restore` / `sync` of DB + bind-mount trees |
| `sync-runtime-local.sh` | Live VPS upload trees → local project-dev paths |
| `backup-runtime.sh` | `backup` / `prune` / `restore` into `BACKUP_TARGET` |

Those scripts already call other tools:

- **DB dump:** `shopware-cli project dump` (one-shot
  `ghcr.io/shopware/shopware-cli` on the Compose network). Escape hatch:
  `SYNC_DUMP_ENGINE=mysqldump`. **Dump is owned completely by upstream
  shopware-cli.** fyrst-cli must not provide a dump command and must not
  wrap or orchestrate dump as a first-class feature.
- **DB restore / import:** MySQL/MariaDB client import (`gzip -dc` piped into
  `mysql`/`mariadb`). **shopware-cli is dump-only; it has no import.** That
  gap is what fyrst-cli implements.
- **Release / rollback:** `docker compose` with
  `deploy/compose.yaml` + `compose.prod.yaml` + `compose.vps.yaml`. Never
  build images or compile themes in these paths.
- **URL rewrite (opt-in after restore):**
  `bin/console fyrst:sales-channel:rewrite-urls` via compose `web`.

`fyrst-dev/recipes` and `fyrst-dev/shopware-cd` stay the product sources for
those scripts and the console command. This CLI does not become a second
implementation of dump, compose rollout, or rewrite-urls. It *does* call
the MySQL client for import, because no upstream CLI command exists.

## Decision

1. **Namespace:** all Shopware CD operations hang off
   `fyrst-cli shopware <subcommand>` so the binary can later host other fyrst
   namespaces without colliding with Shopware verbs.
2. **Command map:** overlay *scripts* stay the product source; operator verbs
   are grouped by lifecycle (see [ADR-0002](ADR-0002-shopware-lifecycle-regroup.md)
   and [command-matrix.md](command-matrix.md)), plus a first-class import verb
   that the overlay only has as `restore_db_*` internals:

   ```
   fyrst-cli shopware env init
   fyrst-cli shopware deploy {release|rollback}
   fyrst-cli shopware db import
   fyrst-cli shopware sync {capture|apply|pull|local}
   fyrst-cli shopware backup {create|prune|recover}
   ```

   Overlay script *files* are unchanged (out of scope for the CLI regroup).
   Old clap paths are a hard cut (no aliases).

3. **Dump vs import split:**
   - **Dump = shopware-cli only.** Operators run `shopware-cli project dump`
     (or the Flex overlay). fyrst-cli `sync capture` does not dump, does not
     shell out to shopware-cli, and tells the operator to use shopware-cli.
   - **Import = fyrst-cli.** `fyrst-cli shopware db import --file
     <path.sql|.sql.gz>` is the implemented path. `shopware sync apply`
     with `--data db` calls the same import module (`db.sql.gz` / `db.sql`
     under `--snapshot-dir`).
4. **Import mechanics** (recipes `restore_db_local` / `restore_db_via_url`):
   resolve `COMPOSE_DIR` / `.env`; prefer bundled Compose `mysql` via
   `docker compose … exec -T mysql`; else `DATABASE_URL` through a one-shot
   mysql/mariadb client image. Support `.sql` and `.sql.gz`. `--dry-run`
   prints the plan. Passwords never appear on stdout/stderr.
5. **Live guards:** `sync apply` and `sync pull` hard-refuse a live
   consumer unless `SYNC_ALLOW_LIVE_RESTORE=1` (overlay
   `assert_not_live_restore`). Standalone `db import` uses the same live
   detection but requires `--allow-live` or `SYNC_ALLOW_LIVE_RESTORE=1` so
   staging imports stay unscary while live is never a silent default.
   `backup recover` uses a confirm flag plus `BACKUP_ALLOW_LIVE_RESTORE=1`
   on live. The short table lives in `shopware --help` and
   [command-matrix.md](command-matrix.md).
6. **Backup recover:** `shopware backup recover` is disaster recovery onto
   **this host**, not live→staging sync. `--artifact` (aliases `--stamp`,
   `--from`) and `--i-understand-this-restores-this-host` (or
   `BACKUP_CONFIRM_RESTORE=1`) are required. `SHOPWARE_DEPLOY_ENV=live`
   needs `BACKUP_ALLOW_LIVE_RESTORE=1`. Inner apply sets
   `SYNC_ALLOW_LIVE_RESTORE=1` and reuses the sync apply module (db import +
   bind-mount apply from artifact layout). fyrst-cli does not dump.
7. **Thin wrappers (future):** remaining verbs may invoke matching recipe
   scripts. **Rollback** is implemented in this CLI (same compose files and
   `vps_rollout` order as `deploy/vps-rollback.sh`; `IMAGE_TAG` only from
   `.previous-tag`). The Rust layer owns clap, exit codes, and `COMPOSE_DIR`
   discovery. It still does not wrap dump.

## Consequences

- Operators dump with shopware-cli and import with fyrst-cli.
- Recipe and `shopware-cd` product code stay unchanged. Wrappers, when
  written, are expected to live in this repo and *call* those artifacts.
- Operator verbs are lifecycle groups (`env`, `deploy`, `db`, `sync`,
  `backup`). Naming of those verbs is in
  [ADR-0002](ADR-0002-shopware-lifecycle-regroup.md): `sync apply` vs
  `backup recover` is the product split (workdir vs off-host DR); clap
  aliases for old overlay-shaped paths are not kept.
- Other fyrst products should add a sibling of `shopware`, not top-level
  Shopware verbs.
- `.env` is read as `KEY=VALUE` without bash expansion so passwords containing
  `$` stay intact.
