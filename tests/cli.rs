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
        "fyrst-cli shopware sync snapshot",
        "fyrst-cli shopware backup restore",
        "not implemented",
        "shopware-cli",
    ] {
        assert!(
            help.contains(needle),
            "shopware --help missing `{needle}`:\n{help}",
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
        &["shopware", "init-env", "--shop-id", "acme"],
        &["shopware", "release"],
        &["shopware", "rollback"],
        &["shopware", "sync", "restore"],
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
