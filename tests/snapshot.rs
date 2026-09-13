//! Integration tests for `fyrst-cli shopware sync snapshot` (no dump wrap).

use std::process::{Command, Output};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_fyrst-cli"))
}

fn snapshot(extra: &[&str]) -> Output {
    let mut cmd = bin();
    cmd.args(["shopware", "sync", "snapshot"]);
    cmd.args(extra);
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to run snapshot {extra:?}: {e}"))
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn snapshot_help_lists_flags() {
    let out = bin()
        .args(["shopware", "sync", "snapshot", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    for needle in [
        "--from",
        "--data",
        "--snapshot-dir",
        "--dry-run",
        "--skip-db",
        "--skip-volumes",
    ] {
        assert!(help.contains(needle), "missing {needle} in:\n{help}");
    }
}

#[test]
fn db_snapshot_exits_2_points_at_shopware_cli() {
    let out = snapshot(&["--dry-run", "--data", "db"]);
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("not implemented"), "{err}");
    assert!(err.contains("shopware-cli project dump"), "{err}");
    assert!(err.contains("db import"), "{err}");
    let combined = format!("{}{}", stdout(&out), err);
    assert!(
        !combined.contains("docker run"),
        "must not wrap dump via docker run:\n{combined}"
    );
    assert!(
        !combined.contains("ghcr.io/shopware/shopware-cli"),
        "must not pull/run shopware-cli image:\n{combined}"
    );
    assert!(
        !combined.contains("DRY-RUN docker"),
        "must not print a dump docker plan:\n{combined}"
    );
}

#[test]
fn default_data_exits_2() {
    let out = snapshot(&["--dry-run"]);
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("shopware-cli"), "{}", stderr(&out));
    assert!(stderr(&out).contains("volume"), "{}", stderr(&out));
}

#[test]
fn remote_from_exits_2() {
    let out = snapshot(&["--dry-run", "--data", "db", "--from", "live"]);
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("not implemented"), "{}", stderr(&out));
    assert!(stderr(&out).contains("remote SSH"), "{}", stderr(&out));
}

#[test]
fn volumes_only_exits_2() {
    let out = snapshot(&["--dry-run", "--data", "media"]);
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr(&out));
    assert!(
        stderr(&out).contains("bind-mount volume snapshot"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn skip_db_and_skip_volumes_exits_1() {
    let out = snapshot(&["--dry-run", "--skip-db", "--skip-volumes"]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("Nothing to do"), "{}", stderr(&out));
}
