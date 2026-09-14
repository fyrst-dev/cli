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
fn version_matches_crate() {
    let out = run(&["--version"]);
    assert!(
        out.status.success(),
        "fyrst-cli --version failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let expected = env!("CARGO_PKG_VERSION");
    assert!(
        text.contains(expected),
        "--version missing crate version {expected}: {text}"
    );
    assert!(
        !text.contains("0.0.0"),
        "--version still reports placeholder 0.0.0: {text}"
    );
}

#[test]
fn shopware_help_prints_command_tree() {
    let help = stdout(&["shopware", "--help"]);
    for needle in [
        "env init",
        "deploy release",
        "deploy rollback",
        "fyrst-cli shopware db import",
        "fyrst-cli shopware sync capture",
        "fyrst-cli shopware sync apply",
        "fyrst-cli shopware sync pull",
        "fyrst-cli shopware sync local",
        "fyrst-cli shopware backup create",
        "fyrst-cli shopware backup recover",
        "shopware-cli",
        "Dump = shopware-cli",
        "Import = fyrst-cli",
        "Backup = fyrst-cli shopware backup create",
        "Live policy",
        "SYNC_ALLOW_LIVE_RESTORE",
        "BACKUP_ALLOW_LIVE_RESTORE",
        "sync = between environments / workdir",
        "backup = off-host disaster recovery",
    ] {
        assert!(
            help.contains(needle),
            "shopware --help missing `{needle}`:\n{help}",
        );
    }
    for gone in [
        "init-env",
        "sync-local",
        "sync snapshot",
        "sync restore",
        "backup restore",
        "sync sync",
        "backup backup",
    ] {
        assert!(
            !help.contains(gone),
            "shopware --help still lists old path `{gone}`:\n{help}",
        );
    }
    assert!(
        !help.contains("sync capture` dumps the local DB"),
        "help still presents capture as a dump wrap:\n{help}",
    );
}

#[test]
fn dump_is_absent_from_cli_surface() {
    let shopware = stdout(&["shopware", "--help"]);
    let capture = stdout(&["shopware", "sync", "capture", "--help"]);
    let db = stdout(&["shopware", "db", "--help"]);
    let import = stdout(&["shopware", "db", "import", "--help"]);

    assert!(
        shopware.contains("db import"),
        "shopware --help missing db import:\n{shopware}",
    );
    assert!(
        capture.contains("shopware-cli"),
        "capture --help should point at shopware-cli:\n{capture}",
    );
    assert!(
        !capture
            .to_ascii_lowercase()
            .contains("dump db via shopware-cli"),
        "capture --help still describes wrapping dump:\n{capture}",
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
fn sync_help_lists_capture_apply_pull_local() {
    let help = stdout(&["shopware", "sync", "--help"]);
    for needle in ["capture", "apply", "pull", "local"] {
        assert!(
            help.contains(needle),
            "shopware sync --help missing `{needle}`:\n{help}",
        );
    }
    assert!(
        help.lines()
            .any(|l| l.split_whitespace().next() == Some("pull")),
        "shopware sync --help missing nested `pull` verb:\n{help}",
    );
    for gone in ["snapshot", "restore"] {
        assert!(
            help.lines()
                .all(|l| l.split_whitespace().next() != Some(gone)),
            "shopware sync --help still lists old verb `{gone}`:\n{help}",
        );
    }
}

#[test]
fn env_and_deploy_help_list_nested_verbs() {
    let env = stdout(&["shopware", "env", "--help"]);
    assert!(
        env.lines()
            .any(|l| l.split_whitespace().next() == Some("init")),
        "shopware env --help missing `init`:\n{env}",
    );
    let deploy = stdout(&["shopware", "deploy", "--help"]);
    for needle in ["release", "rollback"] {
        assert!(
            deploy
                .lines()
                .any(|l| l.split_whitespace().next() == Some(needle)),
            "shopware deploy --help missing `{needle}`:\n{deploy}",
        );
    }
}

#[test]
fn backup_recover_help_lists_flags_and_does_not_wrap_dump() {
    let help = stdout(&["shopware", "backup", "recover", "--help"]);
    for needle in [
        "--artifact",
        "--stamp",
        "--from",
        "--data",
        "--dry-run",
        "--i-understand-this-restores-this-host",
    ] {
        assert!(
            help.contains(needle),
            "backup recover --help missing `{needle}`:\n{help}",
        );
    }
    let lower = help.to_ascii_lowercase();
    assert!(
        !lower.contains("shopware-cli project dump") || help.contains("does not dump"),
        "backup recover --help must not wrap dump:\n{help}",
    );
}

#[test]
fn backup_help_lists_create_prune_recover() {
    let help = stdout(&["shopware", "backup", "--help"]);
    for needle in ["create", "prune", "recover"] {
        assert!(
            help.contains(needle),
            "shopware backup --help missing `{needle}`:\n{help}",
        );
    }
    assert!(
        help.lines()
            .all(|l| l.split_whitespace().next() != Some("restore")),
        "shopware backup --help still lists old `restore` verb:\n{help}",
    );
}

#[test]
fn old_paths_are_rejected() {
    let cases: &[&[&str]] = &[
        &["shopware", "init-env"],
        &["shopware", "release"],
        &["shopware", "rollback"],
        &["shopware", "sync-local"],
        &["shopware", "sync", "snapshot"],
        &["shopware", "sync", "restore"],
        &["shopware", "sync", "sync"],
        &["shopware", "backup", "backup"],
        &["shopware", "backup", "restore"],
    ];
    for args in cases {
        let out = run(args);
        assert_ne!(
            out.status.code(),
            Some(0),
            "old path unexpectedly succeeded for {args:?}"
        );
    }
}

#[test]
fn stubs_exit_2_with_not_implemented() {
    // All shopware verbs are implemented; keep the loop for future stubs.
    let cases: &[&[&str]] = &[];
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
