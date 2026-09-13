//! Integration tests for `fyrst-cli shopware backup backup`.

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
"
            ),
        )
        .unwrap();
        fs::write(
            self.0.join("deploy/compose.yaml"),
            "services:\n  web:\n    image: example\n",
        )
        .unwrap();
    }

    fn data_root(&self) -> PathBuf {
        let d = self.0.join("sw-data");
        fs::create_dir_all(d.join("media")).unwrap();
        fs::write(d.join("media/logo.png"), b"png-bytes").unwrap();
        d
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
    "BACKUP_TARGET",
    "BACKUP_KEEP_DAYS",
    "BACKUP_SSH_KEY",
    "BACKUP_SSH_PORT",
    "BACKUP_DB_DUMP",
    "SYNC_ENV",
    "SYNC_ALLOW_LIVE_RESTORE",
];

fn backup(shop: &Path, extra_env: &[(&str, &str)], extra: &[&str]) -> Output {
    let mut cmd = bin();
    cmd.current_dir(shop);
    cmd.env("COMPOSE_DIR", shop);
    for k in LEAK_KEYS {
        if *k != "COMPOSE_DIR" {
            cmd.env_remove(k);
        }
    }
    for (k, v) in extra_env {
        cmd.env(*k, *v);
    }
    cmd.args(["shopware", "backup", "backup"]);
    cmd.args(extra);
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to run backup {extra:?}: {e}"))
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
fn backup_help_lists_flags() {
    let out = bin()
        .args(["shopware", "backup", "backup", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    for needle in ["--data", "--dry-run", "BACKUP_TARGET", "live"] {
        assert!(help.contains(needle), "missing {needle} in:\n{help}");
    }
}

#[test]
fn missing_backup_target_exits_1() {
    let shop = TempShop::new("no-tgt");
    shop.write_shop("live");
    let out = backup(shop.path(), &[], &["--dry-run", "--data", "media"]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("BACKUP_TARGET"), "{err}");
    assert!(!err.contains("not implemented"), "{err}");
}

#[test]
fn dry_run_db_prints_operator_dump_instruction_no_wrap() {
    let shop = TempShop::new("dry-db");
    shop.write_shop("live");
    let data = shop.data_root();
    let backups = shop.path().join("backups");
    let out = backup(
        shop.path(),
        &[
            ("BACKUP_TARGET", backups.to_str().unwrap()),
            ("SHOPWARE_DATA_ROOT", data.to_str().unwrap()),
        ],
        &["--dry-run", "--data", "db,media"],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let log = stdout(&out);
    assert!(log.contains("acme/live/"), "{log}");
    assert!(
        log.contains("run shopware-cli project dump yourself"),
        "{log}"
    );
    assert!(log.contains("DRY-RUN"), "{log}");
    assert!(
        log.contains("rsync") || log.contains("cp -a"),
        "volume copy plan:\n{log}"
    );
    let all = combined(&out);
    assert!(
        !all.contains("ghcr.io/shopware/shopware-cli"),
        "must not wrap dump via shopware-cli image:\n{all}"
    );
    assert!(
        !all.contains("DRY-RUN docker") || !all.contains("project dump"),
        "must not print a dump docker plan:\n{all}"
    );
    assert!(
        !all.to_ascii_lowercase()
            .contains("sync-runtime.sh snapshot"),
        "{all}"
    );
    assert!(!backups.exists() || backups.read_dir().map(|d| d.count()).unwrap_or(0) == 0);
}

#[test]
fn dry_run_ssh_target_prints_rsync_plan_without_ssh() {
    let shop = TempShop::new("dry-ssh");
    shop.write_shop("staging");
    let data = shop.data_root();
    let out = backup(
        shop.path(),
        &[
            ("BACKUP_TARGET", "backup@backup-host:/var/backups/shopware"),
            ("SHOPWARE_DATA_ROOT", data.to_str().unwrap()),
        ],
        &["--dry-run", "--data", "media"],
    );
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let log = stdout(&out);
    assert!(log.contains("backup@backup-host"), "{log}");
    assert!(log.contains("DRY-RUN rsync"), "{log}");
    assert!(log.contains("var/backup-work"), "{log}");
    assert!(!log.contains("Probing SSH"), "{log}");
}

#[test]
fn execute_copies_tree_manifest_checksums_live() {
    let shop = TempShop::new("exec");
    shop.write_shop("live");
    let data = shop.data_root();
    let backups = shop.path().join("backups");
    let dump = shop.path().join("provided.sql.gz");
    fs::write(&dump, [0x1f, 0x8b, 0x08, 0x00]).unwrap();
    let out = backup(
        shop.path(),
        &[
            ("BACKUP_TARGET", backups.to_str().unwrap()),
            ("SHOPWARE_DATA_ROOT", data.to_str().unwrap()),
            ("BACKUP_DB_DUMP", dump.to_str().unwrap()),
            ("BACKUP_KEEP_DAYS", "0"),
        ],
        &["--data", "db,media"],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let log = stdout(&out);
    assert!(log.contains("live is allowed"), "{log}");
    let env_dir = backups.join("acme").join("live");
    let stamps: Vec<_> = fs::read_dir(&env_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    assert_eq!(stamps.len(), 1);
    let art = &stamps[0];
    let name = art.file_name().unwrap().to_string_lossy();
    assert!(
        name.len() == 16 && name.as_bytes()[8] == b'T' && name.ends_with('Z'),
        "stamp {name}"
    );
    assert_eq!(
        fs::read(art.join("data/media/logo.png")).unwrap(),
        b"png-bytes"
    );
    assert!(art.join("db.sql.gz").is_file());
    let mf = fs::read_to_string(art.join("BACKUP_MANIFEST.txt")).unwrap();
    assert!(mf.contains("shopware-vps-backup 1"), "{mf}");
    assert!(mf.contains("shop_id=acme"), "{mf}");
    assert!(mf.contains("deploy_env=live"), "{mf}");
    let sums = fs::read_to_string(art.join("SHA256SUMS")).unwrap();
    assert!(
        sums.contains("logo.png") || sums.contains("db.sql.gz"),
        "{sums}"
    );
    assert!(!sums.contains("BACKUP_MANIFEST"), "{sums}");
    let all = combined(&out);
    assert!(!all.contains("ghcr.io/shopware/shopware-cli"), "{all}");
}

#[test]
fn execute_db_without_dump_fails_clearly() {
    let shop = TempShop::new("nodump");
    shop.write_shop("staging");
    let backups = shop.path().join("backups");
    let out = backup(
        shop.path(),
        &[("BACKUP_TARGET", backups.to_str().unwrap())],
        &["--data", "db"],
    );
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("shopware-cli project dump"), "{err}");
    assert!(!err.contains("not implemented"), "{err}");
    assert!(!err.contains("docker run"), "{err}");
}

#[test]
fn overlapping_lock_is_refused() {
    let shop = TempShop::new("lock");
    shop.write_shop("live");
    let backups = shop.path().join("backups");
    fs::create_dir_all(shop.path().join("var")).unwrap();
    let lock_path = shop.path().join("var/backup-runtime.lock");
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        const LOCK_EX: i32 = 2;
        const LOCK_NB: i32 = 4;
        extern "C" {
            fn flock(fd: i32, operation: i32) -> i32;
        }
        let rc = unsafe { flock(lock_file.as_raw_fd(), LOCK_EX | LOCK_NB) };
        assert_eq!(rc, 0, "test failed to acquire flock");
    }
    let data = shop.data_root();
    let out = backup(
        shop.path(),
        &[
            ("BACKUP_TARGET", backups.to_str().unwrap()),
            ("SHOPWARE_DATA_ROOT", data.to_str().unwrap()),
        ],
        &["--dry-run", "--data", "media"],
    );
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("holds"), "{err}");
    assert!(err.contains("backup-runtime.lock"), "{err}");
    drop(lock_file);
}

#[test]
fn execute_prunes_old_stamps() {
    let shop = TempShop::new("prune");
    shop.write_shop("live");
    let data = shop.data_root();
    let backups = shop.path().join("backups");
    let old = backups.join("acme").join("live").join("20200101T000000Z");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("stale.txt"), b"old").unwrap();
    let keep_name = "notes.txt";
    fs::create_dir_all(backups.join("acme").join("live").join(keep_name)).unwrap();
    let out = backup(
        shop.path(),
        &[
            ("BACKUP_TARGET", backups.to_str().unwrap()),
            ("SHOPWARE_DATA_ROOT", data.to_str().unwrap()),
            ("BACKUP_KEEP_DAYS", "14"),
        ],
        &["--data", "media"],
    );
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    assert!(!old.exists(), "old stamp should be pruned");
    assert!(
        backups.join("acme").join("live").join(keep_name).exists(),
        "non-stamp name kept"
    );
}
