//! Integration tests for the `fyrst-cli` help tree and stub exits.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_fyrst-cli"))
}

fn stdout(args: &[&str]) -> String {
    let out = bin()
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run fyrst-cli {args:?}: {e}"));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn run(args: &[&str]) -> std::process::Output {
    bin()
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run fyrst-cli {args:?}: {e}"))
}

#[test]
fn shopware_help_prints_command_tree() {
    let help = stdout(&["shopware", "--help"]);
    for needle in [
        "init-env",
        "release",
        "rollback",
        "sync",
        "sync-local",
        "backup",
        "fyrst-cli shopware db import",
        "fyrst-cli shopware sync snapshot",
        "fyrst-cli shopware backup restore",
        "not implemented",
        "shopware-cli",
        "Dump = shopware-cli",
        "Import = fyrst-cli",
    ] {
        assert!(
            help.contains(needle),
            "shopware --help missing `{needle}`:\n{help}",
        );
    }
    assert!(
        !help.contains("sync snapshot` dumps the local DB"),
        "help still presents snapshot as a dump wrap:\n{help}",
    );
}

#[test]
fn dump_is_absent_from_cli_surface() {
    let shopware = stdout(&["shopware", "--help"]);
    let snapshot = stdout(&["shopware", "sync", "snapshot", "--help"]);
    let db = stdout(&["shopware", "db", "--help"]);
    let import = stdout(&["shopware", "db", "import", "--help"]);

    assert!(
        shopware.contains("db import"),
        "shopware --help missing db import:\n{shopware}",
    );
    assert!(
        snapshot.contains("shopware-cli"),
        "snapshot --help should point at shopware-cli:\n{snapshot}",
    );
    assert!(
        !snapshot
            .to_ascii_lowercase()
            .contains("dump db via shopware-cli"),
        "snapshot --help still describes wrapping dump:\n{snapshot}",
    );
    assert!(db.contains("import"), "shopware db --help:\n{db}");
    for needle in ["--file", "--dry-run", "--allow-live"] {
        assert!(
            import.contains(needle),
            "import --help missing {needle}:\n{import}"
        );
    }

    let dump_verbs = [
        &["shopware", "dump"][..],
        &["shopware", "db", "dump"][..],
        &["shopware", "sync", "dump"][..],
    ];
    for args in dump_verbs {
        let out = run(args);
        assert_ne!(
            out.status.code(),
            Some(0),
            "unexpected dump verb success for {args:?}"
        );
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !combined.contains("project dump") || out.status.code() != Some(0),
            "dump wrap surfaced for {args:?}: {combined}"
        );
    }
}

#[test]
fn sync_help_lists_snapshot_restore_sync() {
    let help = stdout(&["shopware", "sync", "--help"]);
    for needle in ["snapshot", "restore"] {
        assert!(
            help.contains(needle),
            "shopware sync --help missing `{needle}`:\n{help}",
        );
    }
    assert!(
        help.lines()
            .any(|l| l.split_whitespace().next() == Some("sync")),
        "shopware sync --help missing nested `sync` verb:\n{help}",
    );
}

#[test]
fn backup_help_lists_backup_prune_restore() {
    let help = stdout(&["shopware", "backup", "--help"]);
    for needle in ["backup", "prune", "restore"] {
        assert!(
            help.contains(needle),
            "shopware backup --help missing `{needle}`:\n{help}",
        );
    }
}

#[test]
fn stubs_exit_2_with_not_implemented() {
    let cases: &[&[&str]] = &[
        &["shopware", "rollback"],
        &["shopware", "sync", "sync"],
        &["shopware", "sync-local"],
        &["shopware", "backup", "backup"],
        &["shopware", "backup", "prune"],
        &["shopware", "backup", "restore"],
    ];
    for args in cases {
        let out = run(args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "expected exit 2 for {args:?}, got {:?}\nstderr={}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr),
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("not implemented"),
            "stderr for {args:?} missing \"not implemented\": {stderr}",
        );
    }
}
