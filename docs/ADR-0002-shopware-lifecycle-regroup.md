# ADR 0002: Shopware commands grouped by lifecycle

- Status: Accepted
- Date: 2026-09-14
- Supersedes: command *names* in [ADR-0001](ADR-0001-shopware-namespace.md)
  (namespace, dump/import split, live guards, and dump ownership stay)

## Context

ADR-0001 mapped Flex overlay script names onto `fyrst-cli shopware` verbs
(`init-env`, top-level `release` / `rollback`, `sync snapshot` / `restore`,
`backup restore`). [#25](https://github.com/fyrst-dev/cli/issues/25) then
preferred operator verbs (`sync pull`, `backup create`, nested `sync local`)
while keeping clap aliases for overlay/script parity.

Operators think in lifecycle (set up the shop, deploy an image, move data,
disaster recovery), not `deploy/*.sh` filenames. Doubled verbs (`sync sync`,
`backup backup`) and two `restore` verbs on different groups were leftover
overlay shape. The CLI is still under development; a hard cut is acceptable.

## Decision

1. **Regroup** `fyrst-cli shopware` by lifecycle role:

   ```
   fyrst-cli shopware env init
   fyrst-cli shopware deploy {release|rollback}
   fyrst-cli shopware db import
   fyrst-cli shopware sync {capture|apply|pull|local}
   fyrst-cli shopware backup {create|prune|recover}
   ```

   | Group | Meaning |
   | --- | --- |
   | `env` | Shop identity / `.env` setup |
   | `deploy` | Image pull + Compose recreate on the VPS |
   | `db` | SQL import (room for later non-dump DB helpers) |
   | `sync` | Between environments / workdir (not retention) |
   | `backup` | Off-host retention + disaster recovery |

2. **Hard cut.** Do not keep old primary names or clap aliases for
   `init-env`, top-level `release` / `rollback`, `sync snapshot` / `restore`,
   `sync-local`, `sync sync`, `backup backup`, or `backup restore`. Overlay
   *scripts* (`deploy/init-env.sh`, `vps-release.sh`, …) are unchanged.

3. **Old → new**

   | Old (post-#25) | New |
   | --- | --- |
   | `shopware init-env` | `shopware env init` |
   | `shopware release` | `shopware deploy release` |
   | `shopware rollback` | `shopware deploy rollback` |
   | `shopware db import` | unchanged |
   | `shopware sync snapshot` | `shopware sync capture` |
   | `shopware sync restore` | `shopware sync apply` |
   | `shopware sync pull` (alias `sync sync`) | `shopware sync pull` only |
   | `shopware sync local` / `sync-local` | `shopware sync local` only |
   | `shopware backup create` (alias `backup backup`) | `shopware backup create` only |
   | `shopware backup prune` | unchanged |
   | `shopware backup restore` | `shopware backup recover` |

4. **Flags and env stay.** Behaviour from #25 is unchanged except naming:
   `sync local` refuses `--data all`; `backup recover` prefers `--artifact`
   (aliases `--stamp`, `--from`); live-policy gates are the same under the
   new names. `--snapshot-dir` remains the sync workdir flag.

5. **One-liner:** **sync = between environments / workdir; backup = off-host DR.**

## Consequences

- `fyrst-cli shopware --help` lists only the regrouped tree.
- Smoke-fail auto-rollback prints
  `IMAGE_TAG=$(cat .previous-tag) fyrst-cli shopware deploy rollback`.
- Room under `env` / `deploy` / `db` for later verbs without more top-level
  clutter.
- Recipe bash wrappers / ClickUp are **not** switched in this change.
