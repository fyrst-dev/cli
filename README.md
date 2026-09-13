# fyrst-cli

This is the fyrst.dev global CLI home (`fyrst-cli`).

Shopware CD operations live under the `shopware` namespace. Other fyrst
tooling can be added as sibling namespaces later.

License: MIT.

## Status

This repository currently ships a **clap skeleton**. `fyrst-cli shopware …`
prints a full help tree. Invoking a subcommand (other than `--help`) exits **2**
and prints `not implemented`.

**Thin wrappers around the Flex overlay scripts are future work — they are not
implemented here.** When they land, they will call the scripts (or a shared
library extracted from them) and keep using:

- `shopware-cli project dump` for DB snapshots
- the MySQL/MariaDB client for import
- `docker compose` for VPS release / rollback
- `bin/console fyrst:sales-channel:rewrite-urls` after opt-in restore

Those tools will not be reimplemented in this CLI. Overlay scripts live in
[fyrst-dev/recipes](https://github.com/fyrst-dev/recipes) (`fyrst/shopware-cd`).
The console command lives in [fyrst-dev/shopware-cd](https://github.com/fyrst-dev/shopware-cd).

See [docs/ADR-0001-shopware-namespace.md](docs/ADR-0001-shopware-namespace.md)
and [docs/command-matrix.md](docs/command-matrix.md).

## Command tree

```
fyrst-cli shopware init-env
fyrst-cli shopware release
fyrst-cli shopware rollback
fyrst-cli shopware sync snapshot
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
```

## Build

```bash
cargo build
cargo run -- shopware --help
```

Requires a Rust toolchain (edition 2021). CI runs `cargo check` and
`cargo test`.
