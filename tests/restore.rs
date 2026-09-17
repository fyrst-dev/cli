//! Integration tests for `fyrst-cli shopware sync apply` volumes and orchestration.

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
            "fyrst-cli-it-rst-{prefix}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(p.join("deploy")).unwrap();
        fs::create_dir_all(p.join("var/runtime-sync")).unwrap();
        Self(p)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write_staging(&self) {
        let data_root = self.0.join("data-root");
        fs::create_dir_all(&data_root).unwrap();
        fs::write(
            self.0.join(".env"),
            format!(
                "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=staging
MYSQL_USER=shop
MYSQL_PASSWORD=super-secret-pass
MYSQL_DATABASE=shopware
SHOPWARE_DATA_ROOT={}
",
                data_root.display()
            ),
        )
        .unwrap();
        fs::write(
            self.0.join("deploy/compose.yaml"),
            "services:\n  mysql:\n    image: mysql:8.4\n  web:\n    image: example\n",
        )
        .unwrap();
    }

    fn write_media_tree(&self) {
        let d = self.0.join("var/runtime-sync/data/media");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("hello.txt"), b"hi").unwrap();
    }

    fn write_dump(&self) {
        fs::write(self.0.join("var/runtime-sync/db.sql"), b"SELECT 1;\n").unwrap();
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
    "SHOPWARE_ALLOW_LIVE_RESTORE",
    "MYSQL_USER",
    "MYSQL_PASSWORD",
    "MYSQL_DATABASE",
    "MYSQL_ROOT_PASSWORD",
    "DATABASE_URL",
    "IMAGE",
    "APP_URL",
    "SYNC_MYSQL_CLIENT_IMAGE",
    "SYNC_SNAPSHOT_DIR",
    "SYNC_ALLOW_LIVE_RESTORE",
    "SYNC_ENV",
    "SYNC_REWRITE_APP_URL",
    "SYNC_REWRITE_URL_MAP",
    "SYNC_POST_RESTORE_CMD",
    "SYNC_ARCHIVE_IMAGE",
    "SYNC_DATA_ROOT",
    "SYNC_APP_URL",
];

fn restore(shop: &Path, extra: &[&str]) -> Output {
    let mut cmd = bin();
    cmd.current_dir(shop);
    cmd.env("COMPOSE_DIR", shop);
    for k in LEAK_KEYS {
        if *k != "COMPOSE_DIR" {
            cmd.env_remove(k);
        }
    }
    cmd.args(["shopware", "sync", "apply"]);
    cmd.args(extra);
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to run restore {extra:?}: {e}"))
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn volume_dry_run_prints_rsync_plan() {
    let shop = TempShop::new("vol-dry");
    shop.write_staging();
    shop.write_media_tree();
    let out = restore(shop.path(), &["--dry-run", "--data", "media"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let log = stdout(&out);
    let combined = format!("{log}{}", stderr(&out));
    assert!(!combined.contains("super-secret-pass"), "{combined}");
    assert!(log.contains("DRY-RUN rsync"), "{log}");
    assert!(log.contains("chown 82:82"), "{log}");
    assert!(log.contains("data/media"), "{log}");
    assert!(log.contains("would stop web/worker/scheduler"), "{log}");
    assert!(log.contains("cache:clear"), "{log}");
    assert!(
        log.contains("Sales-channel domains were not rewritten"),
        "{log}"
    );
    assert!(!log.contains("fyrst:sales-channel:rewrite-urls"), "{log}");
}

#[test]
fn volume_only_live_refuses() {
    let shop = TempShop::new("vol-live");
    shop.write_staging();
    fs::write(
        shop.path().join(".env"),
        format!(
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=live
MYSQL_PASSWORD=super-secret-pass
SHOPWARE_DATA_ROOT={}
",
            shop.path().join("data-root").display()
        ),
    )
    .unwrap();
    shop.write_media_tree();
    let out = restore(shop.path(), &["--dry-run", "--data", "media"]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("live"), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("SHOPWARE_ALLOW_LIVE_RESTORE"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn volume_only_live_allowed_via_env() {
    let shop = TempShop::new("vol-live-ok");
    fs::write(
        shop.path().join(".env"),
        format!(
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=live
MYSQL_PASSWORD=super-secret-pass
SHOPWARE_ALLOW_LIVE_RESTORE=1
SHOPWARE_DATA_ROOT={}
",
            shop.path().join("data-root").display()
        ),
    )
    .unwrap();
    fs::write(
        shop.path().join("deploy/compose.yaml"),
        "services:\n  mysql:\n    image: mysql:8.4\n",
    )
    .unwrap();
    shop.write_media_tree();
    let out = restore(shop.path(), &["--dry-run", "--data", "media"]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("WARNING:"), "{}", stderr(&out));
    assert!(stdout(&out).contains("DRY-RUN rsync"), "{}", stdout(&out));
}

#[test]
fn rewrite_skipped_when_skip_db() {
    let shop = TempShop::new("rew-skip");
    shop.write_staging();
    let mut env = fs::read_to_string(shop.path().join(".env")).unwrap();
    env.push_str("APP_URL=https://staging.example.com\n");
    fs::write(shop.path().join(".env"), env).unwrap();
    shop.write_media_tree();
    let out = restore(shop.path(), &["--dry-run", "--data", "media"]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let log = stdout(&out);
    assert!(
        log.contains("db was skipped") && log.contains("not rewriting"),
        "{log}"
    );
    assert!(
        !log.contains("fyrst:sales-channel:rewrite-urls"),
        "must not run rewrite when db skipped:\n{log}"
    );
}

#[test]
fn rewrite_skipped_on_live_even_with_allow() {
    let shop = TempShop::new("rew-live");
    fs::write(
        shop.path().join(".env"),
        format!(
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=live
MYSQL_USER=shop
MYSQL_PASSWORD=super-secret-pass
SHOPWARE_ALLOW_LIVE_RESTORE=1
APP_URL=https://staging.example.com
SHOPWARE_DATA_ROOT={}
",
            shop.path().join("data-root").display()
        ),
    )
    .unwrap();
    fs::write(
        shop.path().join("deploy/compose.yaml"),
        "services:\n  mysql:\n    image: mysql:8.4\n",
    )
    .unwrap();
    shop.write_dump();
    shop.write_media_tree();
    let out = restore(shop.path(), &["--dry-run", "--data", "db"]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let log = stdout(&out);
    assert!(
        log.contains("Skipping sales-channel domain rewrite on a live host"),
        "{log}"
    );
    assert!(
        !log.contains("fyrst:sales-channel:rewrite-urls"),
        "must not run rewrite on live:\n{log}"
    );
    let combined = format!("{log}{}", stderr(&out));
    assert!(!combined.contains("super-secret-pass"), "{combined}");
}

#[test]
fn rewrite_dry_run_is_compose_console() {
    let shop = TempShop::new("rew-run");
    shop.write_staging();
    let mut env = fs::read_to_string(shop.path().join(".env")).unwrap();
    env.push_str("APP_URL=https://staging.example.com\n");
    fs::write(shop.path().join(".env"), env).unwrap();
    shop.write_dump();
    let out = restore(shop.path(), &["--dry-run", "--data", "db"]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let log = stdout(&out);
    let combined = format!("{log}{}", stderr(&out));
    assert!(!combined.contains("super-secret-pass"), "{combined}");
    assert!(log.contains("fyrst:sales-channel:rewrite-urls"), "{log}");
    assert!(
        log.contains("run --rm --pull never --entrypoint php"),
        "{log}"
    );
    assert!(
        log.contains("--app-url=https://staging.example.com"),
        "{log}"
    );
    assert!(
        !log.to_ascii_lowercase().contains("update sales_channel"),
        "{log}"
    );
    assert!(log.contains("--env-file .env"), "{log}");
    assert!(
        !log.split_whitespace()
            .any(|t| t == "-p" || t == "--project-name"),
        "{log}"
    );
}

#[test]
fn mixed_db_and_media_dry_run() {
    let shop = TempShop::new("mix");
    shop.write_staging();
    shop.write_dump();
    shop.write_media_tree();
    let out = restore(shop.path(), &["--dry-run", "--data", "db,media"]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let log = stdout(&out);
    let combined = format!("{log}{}", stderr(&out));
    assert!(!combined.contains("super-secret-pass"), "{combined}");
    assert!(log.contains("DRY-RUN cat "), "{log}");
    assert!(log.contains("exec -T mysql"), "{log}");
    assert!(log.contains("DRY-RUN rsync"), "{log}");
}

#[test]
fn tar_fallback_dry_run() {
    let shop = TempShop::new("tar");
    shop.write_staging();
    fs::create_dir_all(shop.path().join("var/runtime-sync/volumes")).unwrap();
    fs::write(
        shop.path().join("var/runtime-sync/volumes/theme.tar.gz"),
        b"not-gzip-but-exists",
    )
    .unwrap();
    let out = restore(shop.path(), &["--dry-run", "--data", "theme"]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let log = stdout(&out);
    assert!(log.contains("DRY-RUN extract"), "{log}");
    assert!(log.contains("volumes/theme.tar.gz"), "{log}");
}
