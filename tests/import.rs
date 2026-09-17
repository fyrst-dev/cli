//! Integration tests for `fyrst-cli shopware db import` and sync apply DB.

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
            "fyrst-cli-it-imp-{prefix}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(p.join("deploy")).unwrap();
        fs::create_dir_all(p.join("var/runtime-sync")).unwrap();
        Self(p)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write_min_shop(&self) {
        fs::write(
            self.0.join(".env"),
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=staging
COMPOSE_PROJECT_NAME=shopware-acme
MYSQL_USER=shop
MYSQL_PASSWORD=super-secret-pass
MYSQL_DATABASE=shopware
",
        )
        .unwrap();
        fs::write(
            self.0.join("deploy/compose.yaml"),
            "services:\n  mysql:\n    image: mysql:8.4\n",
        )
        .unwrap();
    }

    fn dump_gz(&self) -> PathBuf {
        let p = self.0.join("db.sql.gz");
        fs::write(&p, b"placeholder-sql-gz").unwrap();
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
    "SHOPWARE_DATA_ROOT",
    "SHOPWARE_DATA_BASE",
    "MYSQL_USER",
    "MYSQL_PASSWORD",
    "MYSQL_DATABASE",
    "MYSQL_ROOT_PASSWORD",
    "DATABASE_URL",
    "SHOPWARE_ALLOW_LIVE_RESTORE",
    "SYNC_MYSQL_CLIENT_IMAGE",
    "SYNC_SNAPSHOT_DIR",
    "SYNC_ALLOW_LIVE_RESTORE",
    "SYNC_ENV",
    "SYNC_REWRITE_APP_URL",
    "SYNC_REWRITE_URL_MAP",
    "SYNC_POST_RESTORE_CMD",
    "SYNC_ARCHIVE_IMAGE",
    "SYNC_DATA_ROOT",
    "IMAGE",
    "APP_URL",
    "SYNC_APP_URL",
];

fn import(shop: &Path, extra: &[&str]) -> Output {
    let mut cmd = bin();
    cmd.current_dir(shop);
    cmd.env("COMPOSE_DIR", shop);
    for k in LEAK_KEYS {
        if *k != "COMPOSE_DIR" {
            cmd.env_remove(k);
        }
    }
    cmd.args(["shopware", "db", "import"]);
    cmd.args(extra);
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to run import {extra:?}: {e}"))
}

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
fn import_help_lists_flags() {
    let out = bin()
        .args(["shopware", "db", "import", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    for needle in ["--file", "--dry-run", "--allow-live"] {
        assert!(help.contains(needle), "missing {needle} in:\n{help}");
    }
}

#[test]
fn dry_run_bundled_mysql_without_password() {
    let shop = TempShop::new("dry-run");
    shop.write_min_shop();
    let dump = shop.dump_gz();
    let out = import(
        shop.path(),
        &["--dry-run", "--file", dump.to_str().unwrap()],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let combined = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        !combined.contains("super-secret-pass"),
        "password leaked:\n{combined}"
    );
    assert!(
        !combined.contains("MYSQL_PWD"),
        "MYSQL_PWD in logs:\n{combined}"
    );
    assert!(!combined.contains("--password"), "{combined}");
    let log = stdout(&out);
    assert!(log.contains("DRY-RUN gzip -dc"), "{log}");
    assert!(log.contains("exec -T mysql"), "{log}");
    assert!(log.contains("docker compose --env-file .env"), "{log}");
    assert!(log.contains("-f deploy/compose.yaml"), "{log}");
    assert!(log.contains("-p acme-staging"), "{log}");
    assert!(!log.contains("-p shopware-acme"), "{log}");
}

#[test]
fn dry_run_sql_uses_cat() {
    let shop = TempShop::new("sql");
    shop.write_min_shop();
    let dump = shop.path().join("plain.sql");
    fs::write(&dump, b"SELECT 1;\n").unwrap();
    let out = import(
        shop.path(),
        &["--dry-run", "--file", dump.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let log = stdout(&out);
    assert!(log.contains("DRY-RUN cat "), "{log}");
    assert!(!log.contains("gzip"), "{log}");
}

#[test]
fn external_database_url_uses_client_image() {
    let shop = TempShop::new("ext-db");
    fs::write(
        shop.path().join(".env"),
        "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=dev
DATABASE_URL=mysql://alice:s3cret-value@db.example.com:3307/shop
",
    )
    .unwrap();
    fs::write(
        shop.path().join("deploy/compose.yaml"),
        "services:\n  web:\n    image: example\n",
    )
    .unwrap();
    let dump = shop.dump_gz();
    let out = import(
        shop.path(),
        &["--dry-run", "--file", dump.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let combined = format!("{}{}", stdout(&out), stderr(&out));
    assert!(!combined.contains("s3cret-value"), "{combined}");
    assert!(!combined.contains("mysql://alice"), "{combined}");
    let log = stdout(&out);
    assert!(log.contains("docker run mysql:8.4 mysql shop"), "{log}");
    assert!(log.contains("db.example.com:3307/shop"), "{log}");
}

#[test]
fn missing_file_exits_1() {
    let shop = TempShop::new("nofile");
    shop.write_min_shop();
    let out = import(
        shop.path(),
        &[
            "--dry-run",
            "--file",
            shop.path().join("missing.sql.gz").to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("not found"), "{}", stderr(&out));
}

#[test]
fn live_refuses_without_allow() {
    let shop = TempShop::new("live");
    fs::write(
        shop.path().join(".env"),
        "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=live
MYSQL_USER=shop
MYSQL_PASSWORD=super-secret-pass
MYSQL_DATABASE=shopware
",
    )
    .unwrap();
    fs::write(
        shop.path().join("deploy/compose.yaml"),
        "services:\n  mysql:\n    image: mysql:8.4\n",
    )
    .unwrap();
    let dump = shop.dump_gz();
    let out = import(
        shop.path(),
        &["--dry-run", "--file", dump.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("live"), "{}", stderr(&out));
    assert!(stderr(&out).contains("--allow-live"), "{}", stderr(&out));
}

#[test]
fn live_allow_flag_dry_run_warns() {
    let shop = TempShop::new("liveok");
    fs::write(
        shop.path().join(".env"),
        "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=live
MYSQL_USER=shop
MYSQL_PASSWORD=super-secret-pass
MYSQL_DATABASE=shopware
",
    )
    .unwrap();
    fs::write(
        shop.path().join("deploy/compose.yaml"),
        "services:\n  mysql:\n    image: mysql:8.4\n",
    )
    .unwrap();
    let dump = shop.dump_gz();
    let out = import(
        shop.path(),
        &[
            "--dry-run",
            "--allow-live",
            "--file",
            dump.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("WARNING:"), "{}", stderr(&out));
    assert!(stdout(&out).contains("DRY-RUN"), "{}", stdout(&out));
}

#[test]
fn restore_db_dry_run() {
    let shop = TempShop::new("restore");
    shop.write_min_shop();
    fs::write(
        shop.path().join("var/runtime-sync/db.sql.gz"),
        b"placeholder",
    )
    .unwrap();
    let out = restore(shop.path(), &["--dry-run", "--data", "db"]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let combined = format!("{}{}", stdout(&out), stderr(&out));
    assert!(!combined.contains("super-secret-pass"), "{combined}");
    assert!(
        stdout(&out).contains("DRY-RUN gzip -dc"),
        "{}",
        stdout(&out)
    );
    assert!(stdout(&out).contains("exec -T mysql"), "{}", stdout(&out));
}

#[test]
fn restore_live_refuses() {
    let shop = TempShop::new("rst-live");
    fs::write(
        shop.path().join(".env"),
        "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=live
MYSQL_USER=shop
MYSQL_PASSWORD=super-secret-pass
MYSQL_DATABASE=shopware
",
    )
    .unwrap();
    fs::write(
        shop.path().join("deploy/compose.yaml"),
        "services:\n  mysql:\n    image: mysql:8.4\n",
    )
    .unwrap();
    fs::write(shop.path().join("var/runtime-sync/db.sql"), b"SELECT 1;\n").unwrap();
    let out = restore(shop.path(), &["--dry-run", "--data", "db"]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(
        stderr(&out).contains("SHOPWARE_ALLOW_LIVE_RESTORE"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn restore_live_allowed_via_env() {
    let shop = TempShop::new("rst-live-ok");
    fs::write(
        shop.path().join(".env"),
        "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=live
MYSQL_USER=shop
MYSQL_PASSWORD=super-secret-pass
MYSQL_DATABASE=shopware
SHOPWARE_ALLOW_LIVE_RESTORE=1
",
    )
    .unwrap();
    fs::write(
        shop.path().join("deploy/compose.yaml"),
        "services:\n  mysql:\n    image: mysql:8.4\n",
    )
    .unwrap();
    fs::write(shop.path().join("var/runtime-sync/db.sql"), b"SELECT 1;\n").unwrap();
    let out = restore(shop.path(), &["--dry-run", "--data", "db"]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("WARNING:"), "{}", stderr(&out));
}

#[test]
fn restore_missing_dump_exits_1() {
    let shop = TempShop::new("rst-missing");
    shop.write_min_shop();
    let out = restore(shop.path(), &["--dry-run", "--data", "db"]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("db.sql"), "{}", stderr(&out));
}
