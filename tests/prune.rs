//! Integration tests for `fyrst-cli shopware backup prune`.

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
            "fyrst-cli-it-prune-{prefix}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(p.join("deploy")).unwrap();
        Self(p)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write_min_shop(&self) {
        fs::write(
            self.0.join(".env"),
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=staging\n",
        )
        .unwrap();
        fs::write(
            self.0.join("deploy/compose.yaml"),
            "services:\n  web:\n    image: example\n",
        )
        .unwrap();
    }

    fn backups(&self) -> PathBuf {
        self.0.join("backups")
    }

    fn artifact(&self, stamp: &str) -> PathBuf {
        let p = self.backups().join("acme").join("staging").join(stamp);
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("KEEP"), b"x").unwrap();
        p
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
    "BACKUP_TARGET",
    "BACKUP_KEEP_DAYS",
    "SHOPWARE_SSH_KEY",
    "BACKUP_SSH_KEY",
    "BACKUP_SSH_PORT",
    "BACKUP_ALLOW_LIVE_RESTORE",
    "BACKUP_CONFIRM_RESTORE",
    "SYNC_ENV",
];

fn prune(shop: &Path, extra: &[&str], target: Option<&Path>, keep: Option<&str>) -> Output {
    let mut cmd = bin();
    cmd.current_dir(shop);
    cmd.env("COMPOSE_DIR", shop);
    for k in LEAK_KEYS {
        if *k != "COMPOSE_DIR" {
            cmd.env_remove(k);
        }
    }
    if let Some(t) = target {
        cmd.env("BACKUP_TARGET", t);
    }
    if let Some(k) = keep {
        cmd.env("BACKUP_KEEP_DAYS", k);
    }
    cmd.args(["shopware", "backup", "prune"]);
    cmd.args(extra);
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to run prune {extra:?}: {e}"))
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
fn prune_help_lists_flags() {
    let out = bin()
        .args(["shopware", "backup", "prune", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    for needle in ["--data", "--dry-run"] {
        assert!(help.contains(needle), "missing {needle} in:\n{help}");
    }
}

#[test]
fn default_backup_target_is_local() {
    let shop = TempShop::new("missing");
    shop.write_min_shop();
    let out = prune(shop.path(), &[], None, None);
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    let text = combined(&out);
    assert!(text.contains("target=local"), "{text}");
    assert!(!text.contains("not implemented"), "{text}");
}

#[test]
fn dry_run_lists_old_only_and_does_not_delete() {
    let shop = TempShop::new("dry-run");
    shop.write_min_shop();
    let old = shop.artifact("20000101T000000Z");
    let kept = shop.artifact("20990101T000000Z");
    let junk = shop.artifact("latest");
    let out = prune(
        shop.path(),
        &["--dry-run", "--data", "db"],
        Some(&shop.backups()),
        Some("14"),
    );
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    let text = combined(&out);
    assert!(text.contains("DRY-RUN rm -rf"), "{text}");
    assert!(text.contains("20000101T000000Z"), "{text}");
    assert!(!text.contains("20990101T000000Z"), "{text}");
    assert!(!text.contains("latest"), "{text}");
    assert!(text.contains("stamp-based"), "{text}");
    assert!(
        !text.to_ascii_lowercase().contains("project dump"),
        "prune must not dump:\n{text}"
    );
    assert!(
        !text.contains("shopware-cli"),
        "prune must not wrap dump:\n{text}"
    );
    assert!(old.is_dir());
    assert!(kept.is_dir());
    assert!(junk.is_dir());
}

#[test]
fn execute_deletes_old_keeps_new_and_non_stamps() {
    let shop = TempShop::new("exec");
    shop.write_min_shop();
    let old = shop.artifact("20000101T000000Z");
    let kept = shop.artifact("20990101T000000Z");
    let junk = shop.artifact("latest");
    let out = prune(shop.path(), &[], Some(&shop.backups()), Some("14"));
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    assert!(!old.exists(), "old stamp should be removed");
    assert!(kept.is_dir());
    assert!(junk.is_dir());
    let text = combined(&out);
    assert!(text.contains("Prune"), "{text}");
    assert!(!text.contains("DRY-RUN"), "{text}");
}

#[test]
fn keep_zero_keeps_all() {
    let shop = TempShop::new("forever");
    shop.write_min_shop();
    let old = shop.artifact("20000101T000000Z");
    let out = prune(shop.path(), &[], Some(&shop.backups()), Some("0"));
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    let text = stdout(&out);
    assert!(text.contains("BACKUP_KEEP_DAYS=0"), "{text}");
    assert!(text.contains("keeping all artifacts"), "{text}");
    assert!(!text.contains("DRY-RUN rm -rf"), "{text}");
    assert!(old.is_dir());
}

#[test]
fn invalid_keep_days_fails() {
    let shop = TempShop::new("bad-keep");
    shop.write_min_shop();
    let out = prune(
        shop.path(),
        &["--dry-run"],
        Some(&shop.backups()),
        Some("nope"),
    );
    assert_eq!(out.status.code(), Some(1), "{}", combined(&out));
    let err = stderr(&out);
    assert!(err.contains("BACKUP_KEEP_DAYS"), "{err}");
    assert!(!err.contains("not implemented"), "{err}");
}
