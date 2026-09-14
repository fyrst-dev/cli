//! Integration tests for `fyrst-cli shopware sync capture` (volumes; no dump wrap).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_fyrst-cli"))
}

struct TempShop(PathBuf);

impl TempShop {
    fn new(prefix: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!(
            "fyrst-cli-it-snap-{prefix}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(p.join("deploy")).unwrap();
        Self(p)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write_min_shop(&self) {
        let data = self.0.join("data");
        fs::create_dir_all(data.join("media")).unwrap();
        fs::write(data.join("media/hello.txt"), b"media-bytes").unwrap();
        fs::write(
            self.0.join(".env"),
            format!(
                "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=staging
SHOPWARE_DATA_ROOT={}
",
                data.display()
            ),
        )
        .unwrap();
        fs::write(
            self.0.join("deploy/compose.yaml"),
            "services:\n  mysql:\n    image: mysql:8.4\n",
        )
        .unwrap();
    }

    fn write_all_volume_dirs(&self) {
        for item in ["files", "thumbnail", "theme", "sitemap"] {
            fs::create_dir_all(self.0.join("data").join(item)).unwrap();
            fs::write(self.0.join("data").join(item).join("keep.txt"), item).unwrap();
        }
    }
}

impl Drop for TempShop {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const LEAK_KEYS: &[&str] = &[
    "COMPOSE_DIR",
    "COMPOSE_PROJECT_NAME",
    "SHOPWARE_SHOP_ID",
    "SHOPWARE_DEPLOY_ENV",
    "SHOPWARE_DATA_ROOT",
    "SHOPWARE_DATA_BASE",
    "SYNC_DATA_ROOT",
    "SYNC_SNAPSHOT_DIR",
    "SYNC_ENV",
    "SYNC_SOURCE_ENV",
    "SYNC_REMOTE_DATA_ROOT",
    "SYNC_REMOTE_PATH",
    "SYNC_SSH_HOST",
    "SYNC_SSH_USER",
    "SYNC_SSH_PORT",
    "SYNC_SSH_KEY",
    "SYNC_ARCHIVE_IMAGE",
    "SYNC_LIVE_SSH_HOST",
    "SYNC_LIVE_SSH_USER",
    "SYNC_LIVE_REMOTE_PATH",
    "SYNC_LIVE_DATA_ROOT",
];

fn snapshot(shop: &Path, extra: &[&str]) -> Output {
    let mut cmd = bin();
    cmd.current_dir(shop);
    cmd.env("COMPOSE_DIR", shop);
    for k in LEAK_KEYS {
        if *k != "COMPOSE_DIR" {
            cmd.env_remove(k);
        }
    }
    cmd.args(["shopware", "sync", "capture"]);
    cmd.args(extra);
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to run snapshot {extra:?}: {e}"))
}

fn snapshot_cwd(extra: &[&str]) -> Output {
    let mut cmd = bin();
    for k in LEAK_KEYS {
        cmd.env_remove(k);
    }
    cmd.args(["shopware", "sync", "capture"]);
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

fn combined(out: &Output) -> String {
    format!("{}{}", stdout(out), stderr(out))
}

fn assert_no_dump_wrap(out: &Output) {
    let combined = combined(out);
    assert!(
        !combined.contains("ghcr.io/shopware/shopware-cli"),
        "must not pull/run shopware-cli image:\n{combined}"
    );
    assert!(
        !combined.contains("SYNC_DUMP_ENGINE"),
        "must not wrap SYNC_DUMP_ENGINE:\n{combined}"
    );
    let lower = combined.to_ascii_lowercase();
    if lower.contains("docker run") {
        assert!(
            !combined.contains("project dump"),
            "must not docker-run shopware-cli dump:\n{combined}"
        );
        assert!(
            !combined.contains("shopware-cli"),
            "docker run must not mention shopware-cli:\n{combined}"
        );
    }
}

#[test]
fn snapshot_help_lists_flags() {
    let out = bin()
        .args(["shopware", "sync", "capture", "--help"])
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
    assert!(
        help.contains("shopware-cli"),
        "help should keep dump = shopware-cli:\n{help}"
    );
}

#[test]
fn db_snapshot_exits_2_points_at_shopware_cli() {
    let out = snapshot_cwd(&["--dry-run", "--data", "db"]);
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("not implemented"), "{err}");
    assert!(err.contains("shopware-cli project dump"), "{err}");
    assert!(err.contains("db import"), "{err}");
    let combined = combined(&out);
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
fn skip_volumes_db_exits_2_not_silent_success() {
    let out = snapshot_cwd(&["--dry-run", "--skip-volumes"]);
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr(&out));
    assert!(
        stderr(&out).contains("shopware-cli project dump"),
        "{}",
        stderr(&out)
    );
    assert!(stderr(&out).contains("db import"), "{}", stderr(&out));
    assert_no_dump_wrap(&out);
}

#[test]
fn remote_db_only_still_not_dump() {
    let out = snapshot_cwd(&["--dry-run", "--data", "db", "--from", "live"]);
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr(&out));
    assert!(
        stderr(&out).contains("shopware-cli project dump"),
        "{}",
        stderr(&out)
    );
    assert_no_dump_wrap(&out);
}

#[test]
fn skip_db_and_skip_volumes_exits_1() {
    let out = snapshot_cwd(&["--dry-run", "--skip-db", "--skip-volumes"]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("Nothing to do"), "{}", stderr(&out));
}

#[test]
fn refuse_mysql_data() {
    let out = snapshot_cwd(&["--dry-run", "--data", "mysql_data"]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("mysql_data"), "{}", stderr(&out));
    assert_no_dump_wrap(&out);
}

#[test]
fn volume_dry_run_prints_rsync_plan_and_does_not_copy() {
    let shop = TempShop::new("dry-run");
    shop.write_min_shop();
    let snap = shop.path().join("var/runtime-sync");
    let out = snapshot(
        shop.path(),
        &[
            "--dry-run",
            "--data",
            "media",
            "--snapshot-dir",
            snap.to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let combined = combined(&out);
    assert!(combined.contains("DRY-RUN rsync"), "{combined}");
    assert!(combined.contains("data/media"), "{combined}");
    assert!(
        !snap.join("data/media/hello.txt").is_file(),
        "dry-run must not copy files"
    );
    assert!(
        !combined.contains("ghcr.io/shopware/shopware-cli"),
        "{combined}"
    );
    assert!(!combined.contains("project dump"), "{combined}");
    assert_no_dump_wrap(&out);
}

#[test]
fn volume_copy_writes_data_item_tree() {
    let shop = TempShop::new("copy");
    shop.write_min_shop();
    let snap = shop.path().join("var/runtime-sync");
    let out = snapshot(
        shop.path(),
        &["--data", "media", "--snapshot-dir", snap.to_str().unwrap()],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let copied = snap.join("data/media/hello.txt");
    assert!(
        copied.is_file(),
        "expected {} after snapshot; stdout={} stderr={}",
        copied.display(),
        stdout(&out),
        stderr(&out)
    );
    assert_eq!(fs::read(copied).unwrap(), b"media-bytes");
    let manifest = fs::read_to_string(snap.join("MANIFEST.txt")).unwrap();
    assert!(manifest.contains("shopware-runtime-snapshot"), "{manifest}");
    assert!(manifest.contains("does not dump"), "{manifest}");
    assert_no_dump_wrap(&out);
}

#[test]
fn all_data_copies_volumes_and_reminds_dump_without_wrapping() {
    let shop = TempShop::new("all");
    shop.write_min_shop();
    shop.write_all_volume_dirs();
    let snap = shop.path().join("snap-all");
    let out = snapshot(
        shop.path(),
        &["--dry-run", "--snapshot-dir", snap.to_str().unwrap()],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let combined = combined(&out);
    assert!(combined.contains("shopware-cli project dump"), "{combined}");
    assert!(combined.contains("db import"), "{combined}");
    assert!(combined.contains("DRY-RUN rsync"), "{combined}");
    assert!(combined.contains("data/media"), "{combined}");
    assert!(combined.contains("db.sql.gz"), "{combined}");
    assert_no_dump_wrap(&out);
}

#[test]
fn remote_from_dry_run_volumes_over_ssh() {
    let shop = TempShop::new("ssh");
    shop.write_min_shop();
    fs::write(
        shop.path().join("deploy/sync.env"),
        "SYNC_REMOTE_PATH=/opt/shopware/acme\nSYNC_REMOTE_DATA_ROOT=/var/lib/shopware/data/acme/live\n",
    )
    .unwrap();
    let snap = shop.path().join("var/runtime-sync");
    let out = snapshot(
        shop.path(),
        &[
            "--dry-run",
            "--from",
            "live",
            "--data",
            "media",
            "--snapshot-dir",
            snap.to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let combined = combined(&out);
    assert!(combined.contains("DRY-RUN rsync"), "{combined}");
    assert!(combined.contains("live:"), "{combined}");
    assert!(combined.contains("/media"), "{combined}");
    assert!(!combined.contains("deploy/sync-runtime.sh"), "{combined}");
    assert!(!combined.contains("project dump"), "{combined}");
    assert_no_dump_wrap(&out);
}

#[test]
fn remote_all_dry_run_volumes_plus_dump_instruction() {
    let shop = TempShop::new("ssh-all");
    shop.write_min_shop();
    fs::write(
        shop.path().join("deploy/sync.env"),
        "SYNC_REMOTE_PATH=/opt/shopware/acme\nSYNC_LIVE_DATA_ROOT=/data/live\n",
    )
    .unwrap();
    let out = snapshot(
        shop.path(),
        &["--dry-run", "--from", "live", "--data", "all"],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let combined = combined(&out);
    assert!(combined.contains("shopware-cli project dump"), "{combined}");
    assert!(combined.contains("DRY-RUN rsync"), "{combined}");
    assert!(!combined.contains("deploy/sync-runtime.sh"), "{combined}");
    assert_no_dump_wrap(&out);
}
