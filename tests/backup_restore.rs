//! Integration tests for `fyrst-cli shopware backup restore`.

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
            "fyrst-cli-it-bk-{prefix}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(p.join("deploy")).unwrap();
        Self(p)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write_shop(&self, deploy_env: &str) {
        fs::write(
            self.0.join(".env"),
            format!(
                "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV={deploy_env}
MYSQL_USER=shop
MYSQL_PASSWORD=super-secret-pass
MYSQL_DATABASE=shopware
SHOPWARE_DATA_ROOT={}/data-root
",
                self.0.display()
            ),
        )
        .unwrap();
        fs::write(
            self.0.join("deploy/compose.yaml"),
            "services:\n  mysql:\n    image: mysql:8.4\n",
        )
        .unwrap();
    }

    fn write_stamp_artifact(&self, stamp: &str) -> PathBuf {
        let art = self
            .0
            .join("backups")
            .join("acme")
            .join("staging")
            .join(stamp);
        fs::create_dir_all(&art).unwrap();
        fs::write(art.join("db.sql"), b"SELECT 1;\n").unwrap();
        art
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
    "MYSQL_USER",
    "MYSQL_PASSWORD",
    "MYSQL_DATABASE",
    "MYSQL_ROOT_PASSWORD",
    "DATABASE_URL",
    "SYNC_MYSQL_CLIENT_IMAGE",
    "SYNC_SNAPSHOT_DIR",
    "SYNC_ALLOW_LIVE_RESTORE",
    "SYNC_ENV",
    "BACKUP_TARGET",
    "BACKUP_KEEP_DAYS",
    "BACKUP_SSH_KEY",
    "BACKUP_SSH_PORT",
    "BACKUP_CONFIRM_RESTORE",
    "BACKUP_ALLOW_LIVE_RESTORE",
];

fn backup_restore(shop: &Path, extra: &[&str], extra_env: &[(&str, &str)]) -> Output {
    let mut cmd = bin();
    cmd.current_dir(shop);
    cmd.env("COMPOSE_DIR", shop);
    for k in LEAK_KEYS {
        if *k != "COMPOSE_DIR" {
            cmd.env_remove(k);
        }
    }
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.args(["shopware", "backup", "restore"]);
    cmd.args(extra);
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to run backup restore {extra:?}: {e}"))
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

#[test]
fn missing_artifact_flag_exits_1() {
    let shop = TempShop::new("nofrom");
    shop.write_shop("staging");
    let out = backup_restore(
        shop.path(),
        &["--dry-run", "--i-understand-this-restores-this-host"],
        &[],
    );
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("--artifact"), "{}", stderr(&out));
    assert!(stderr(&out).contains("--from"), "{}", stderr(&out));
}

#[test]
fn missing_confirmation_mentions_flag_and_env() {
    let shop = TempShop::new("noconfirm");
    shop.write_shop("staging");
    let out = backup_restore(
        shop.path(),
        &["--dry-run", "--from", "20260912T020000Z"],
        &[],
    );
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(
        stderr(&out).contains("--i-understand-this-restores-this-host"),
        "{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("BACKUP_CONFIRM_RESTORE"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn live_refuses_without_backup_allow() {
    let shop = TempShop::new("live");
    shop.write_shop("live");
    let art = shop.path().join("artifact");
    fs::create_dir_all(&art).unwrap();
    fs::write(art.join("db.sql"), b"SELECT 1;\n").unwrap();
    let out = backup_restore(
        shop.path(),
        &[
            "--dry-run",
            "--data",
            "db",
            "--from",
            art.to_str().unwrap(),
            "--i-understand-this-restores-this-host",
        ],
        &[("SYNC_ALLOW_LIVE_RESTORE", "1")],
    );
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(
        stderr(&out).contains("BACKUP_ALLOW_LIVE_RESTORE"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn live_allow_dry_run_warns_and_sets_inner_flag() {
    let shop = TempShop::new("liveok");
    shop.write_shop("live");
    let art = shop.path().join("artifact");
    fs::create_dir_all(&art).unwrap();
    fs::write(art.join("db.sql"), b"SELECT 1;\n").unwrap();
    let out = backup_restore(
        shop.path(),
        &[
            "--dry-run",
            "--data",
            "db",
            "--from",
            art.to_str().unwrap(),
            "--i-understand-this-restores-this-host",
        ],
        &[("BACKUP_ALLOW_LIVE_RESTORE", "1")],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    assert!(stderr(&out).contains("WARNING:"), "{}", stderr(&out));
    assert!(stdout(&out).contains("DRY-RUN"), "{}", stdout(&out));
    assert!(
        stdout(&out).contains("SYNC_ALLOW_LIVE_RESTORE"),
        "{}",
        stdout(&out)
    );
    let all = combined(&out);
    assert!(!all.contains("super-secret-pass"), "{all}");
}

#[test]
fn dry_run_local_stamp_fetch_plan() {
    let shop = TempShop::new("dry");
    shop.write_shop("staging");
    let stamp = "20260912T020000Z";
    shop.write_stamp_artifact(stamp);
    let target = shop.path().join("backups");
    let out = backup_restore(
        shop.path(),
        &[
            "--dry-run",
            "--data",
            "db",
            "--from",
            stamp,
            "--i-understand-this-restores-this-host",
        ],
        &[("BACKUP_TARGET", target.to_str().unwrap())],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let log = stdout(&out);
    assert!(log.contains("DRY-RUN"), "{log}");
    assert!(log.contains(stamp), "{log}");
    assert!(
        log.contains("exec -T mysql") || log.contains("inner restore"),
        "{log}"
    );
    let all = combined(&out);
    assert!(!all.contains("super-secret-pass"), "{all}");
    assert!(!all.contains("MYSQL_PWD"), "{all}");
}

#[test]
fn dry_run_ssh_fetch_plan() {
    let shop = TempShop::new("ssh");
    shop.write_shop("staging");
    let out = backup_restore(
        shop.path(),
        &[
            "--dry-run",
            "--data",
            "db",
            "--from",
            "20260912T020000Z",
            "--i-understand-this-restores-this-host",
        ],
        &[("BACKUP_TARGET", "backup@backup-host:/var/backups/shopware")],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let log = stdout(&out);
    assert!(log.contains("DRY-RUN rsync"), "{log}");
    assert!(log.contains("backup@backup-host"), "{log}");
    assert!(log.contains("restore-20260912T020000Z"), "{log}");
    assert!(log.contains("inner restore"), "{log}");
    assert!(
        !shop.path().join("var/backup-work").exists(),
        "dry-run must not create backup-work"
    );
}

#[test]
fn missing_artifact_exits_1() {
    let shop = TempShop::new("miss");
    shop.write_shop("staging");
    let target = shop.path().join("backups");
    fs::create_dir_all(&target).unwrap();
    let out = backup_restore(
        shop.path(),
        &[
            "--dry-run",
            "--from",
            "19990101T000000Z",
            "--i-understand-this-restores-this-host",
        ],
        &[("BACKUP_TARGET", target.to_str().unwrap())],
    );
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(
        stderr(&out).contains("Artifact not found"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn confirm_via_env_without_flag() {
    let shop = TempShop::new("envok");
    shop.write_shop("staging");
    let stamp = "20260912T020000Z";
    shop.write_stamp_artifact(stamp);
    let target = shop.path().join("backups");
    let out = backup_restore(
        shop.path(),
        &["--dry-run", "--data", "db", "--from", stamp],
        &[
            ("BACKUP_TARGET", target.to_str().unwrap()),
            ("BACKUP_CONFIRM_RESTORE", "1"),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    assert!(stdout(&out).contains("DRY-RUN"), "{}", stdout(&out));
}

#[test]
fn volume_dry_run_does_not_copy() {
    let shop = TempShop::new("vol");
    shop.write_shop("staging");
    let art = shop.path().join("artifact");
    fs::create_dir_all(art.join("data/media")).unwrap();
    fs::write(art.join("data/media/hello.txt"), b"hi").unwrap();
    let out = backup_restore(
        shop.path(),
        &[
            "--dry-run",
            "--data",
            "media",
            "--from",
            art.to_str().unwrap(),
            "--i-understand-this-restores-this-host",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    assert!(stdout(&out).contains("DRY-RUN rsync"), "{}", stdout(&out));
    assert!(!shop.path().join("data-root/media/hello.txt").exists());
}

#[test]
fn volume_execute_copies_from_artifact_layout() {
    let shop = TempShop::new("volex");
    shop.write_shop("staging");
    let art = shop.path().join("artifact");
    fs::create_dir_all(art.join("data/media")).unwrap();
    fs::write(art.join("data/media/hello.txt"), b"hi").unwrap();
    let out = backup_restore(
        shop.path(),
        &[
            "--data",
            "media",
            "--from",
            art.to_str().unwrap(),
            "--i-understand-this-restores-this-host",
        ],
        &[],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    assert_eq!(
        fs::read(shop.path().join("data-root/media/hello.txt")).unwrap(),
        b"hi"
    );
    assert!(
        stdout(&out).contains("Rewrite sales-channel URLs"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn missing_backup_target_for_stamp() {
    let shop = TempShop::new("notarget");
    shop.write_shop("staging");
    let out = backup_restore(
        shop.path(),
        &[
            "--dry-run",
            "--from",
            "20260912T020000Z",
            "--i-understand-this-restores-this-host",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("BACKUP_TARGET"), "{}", stderr(&out));
}

#[test]
fn artifact_and_stamp_flags_work() {
    let shop = TempShop::new("artifact");
    shop.write_shop("staging");
    let stamp = "20260912T020000Z";
    shop.write_stamp_artifact(stamp);
    let backups = shop.path().join("backups");
    for flag in ["--artifact", "--stamp"] {
        let out = backup_restore(
            shop.path(),
            &[
                "--dry-run",
                "--data",
                "db",
                flag,
                stamp,
                "--i-understand-this-restores-this-host",
            ],
            &[("BACKUP_TARGET", backups.to_str().unwrap())],
        );
        assert_eq!(
            out.status.code(),
            Some(0),
            "flag={flag} stderr={} stdout={}",
            stderr(&out),
            stdout(&out)
        );
        assert!(stdout(&out).contains("DRY-RUN"), "{}", stdout(&out));
    }
}
