# ADR 0001: Shopware CD lives under `fyrst-cli shopware`

- Status: Accepted
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

Those scripts already call other tools and must keep doing so:

- **DB dump:** `shopware-cli project dump` (one-shot
  `ghcr.io/shopware/shopware-cli:0.18.4` on the Compose network). Escape hatch:
  `SYNC_DUMP_ENGINE=mysqldump`.
- **DB restore:** MySQL/MariaDB client import (`gzip -dc` piped into
  `mysql`/`mariadb`). shopware-cli is dump-only.
- **Release / rollback:** `docker compose` with
  `deploy/compose.yaml` + `compose.prod.yaml` + `compose.vps.yaml`. Never
  build images or compile themes in these paths.
- **URL rewrite (opt-in after restore):**
  `bin/console fyrst:sales-channel:rewrite-urls` via compose `web`.

The CLI must not become a second implementation of dump, import, compose
rollout, or rewrite-urls. `fyrst-dev/recipes` and `fyrst-dev/shopware-cd`
stay the product sources for those scripts and the console command.

## Decision

1. **Namespace:** all Shopware CD operations hang off
   `fyrst-cli shopware <subcommand>` so the binary can later host other fyrst
   namespaces without colliding with Shopware verbs.
2. **Command map:** overlay script names become CLI verbs (see
   [command-matrix.md](command-matrix.md)):

   ```
   fyrst-cli shopware init-env
   fyrst-cli shopware release
   fyrst-cli shopware rollback
   fyrst-cli shopware sync {snapshot|restore|sync}
   fyrst-cli shopware sync-local
   fyrst-cli shopware backup {backup|prune|restore}
   ```

3. **Thin wrappers (future, not this change):** each verb will invoke the
   matching recipe script (or a shared library extracted from it) with the
   same flags and environment. The Rust layer owns clap, exit codes, and
   discovery of `COMPOSE_DIR`; it does not reimplement:

   - `shopware-cli project dump`
   - MySQL/MariaDB client import
   - Compose pull / setup / web / extra-profile order
   - `fyrst:sales-channel:rewrite-urls`

4. **Skeleton first, then real dump:** the clap tree landed as stubs (`not
   implemented`, exit 2). The first real path is **`shopware sync snapshot`**
   for a **local DB dump**: Rust resolves `COMPOSE_DIR` / `.env` / project name
   and execs the same `docker run` + `shopware-cli project dump` the overlay
   uses (`deploy/lib/sync-dump.sh` + `sync-db.sh`). It does **not** wrap
   `deploy/sync-runtime.sh` yet (recipe wrappers stay future work). Bind-mount
   volume copy, remote SSH `--from`, and `SYNC_DUMP_ENGINE=mysqldump` remain
   stub (exit 2). `--help` is still the flag contract.

## Consequences

- Operators can learn one command tree (`fyrst-cli shopware …`) while shops
  still run `bash deploy/….sh` until wrappers land.
- Recipe and `shopware-cd` product code stay unchanged. Wrappers, when
  written, are expected to live in this repo and *call* those artifacts.
- Nested `sync sync` and `backup backup` look odd in help; they match the
  overlay script subcommands so a later 1:1 wrap stays obvious.
- Other fyrst products should add a sibling of `shopware`, not top-level
  Shopware verbs.
- Operators can dump a running Compose MySQL with
  `fyrst-cli shopware sync snapshot --data db` without invoking the bash
  overlay. Passwords never appear on stdout/stderr (dry-run omits
  `--password`). `.env` is read as `KEY=VALUE` without bash expansion.
