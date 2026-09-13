//! Integration tests for `fyrst-cli shopware sync snapshot` (no Shopware stack).

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
            "fyrst-cli-it-{prefix}-{}-{nanos}",
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
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=staging
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
    "SYNC_DUMP_ENGINE",
    "SYNC_DUMP_QUICK",
    "SYNC_DUMP_CLEAN",
    "SYNC_DUMP_ANONYMIZE",
    "SYNC_SHOPWARE_CLI_IMAGE",
    "SYNC_SNAPSHOT_DIR",
    "SYNC_DATA_ROOT",
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
fn dry_run_prints_docker_line_without_password() {
    let shop = TempShop::new("dry-run");
    shop.write_min_shop();
    let out = snapshot(
        shop.path(),
        &["--dry-run", "--data", "db", "--snapshot-dir", "var/snap"],
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
        !combined.contains("--password"),
        "password flag in logs:\n{combined}"
    );
    let log = stdout(&out);
    assert!(
        log.contains("DRY-RUN docker run --rm --network acme-staging_default"),
        "{log}"
    );
    assert!(
        log.contains("ghcr.io/shopware/shopware-cli:0.18.4"),
        "{log}"
    );
    assert!(log.contains("--host mysql"), "{log}");
    assert!(log.contains("--username shop"), "{log}");
    assert!(log.contains("--database shopware"), "{log}");
    assert!(log.contains("--skip-lock-tables"), "{log}");
    assert!(log.contains("--compression=gzip"), "{log}");
    assert!(log.contains("--quick"), "{log}");
    assert!(log.contains("--clean"), "{log}");
    assert!(log.contains("--output="), "{log}");
    assert!(log.contains("db.sql.gz"), "{log}");
}

#[test]
fn dry_run_default_data_dumps_and_warns_volumes() {
    let shop = TempShop::new("dry-all");
    shop.write_min_shop();
    let out = snapshot(shop.path(), &["--dry-run"]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    assert!(
        stdout(&out).contains("DRY-RUN docker run"),
        "{}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("bind-mount volume snapshot"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn skip_quick_and_anonymize_via_env() {
    let shop = TempShop::new("flags-env");
    shop.write_min_shop();
    let mut cmd = bin();
    cmd.current_dir(shop.path());
    cmd.env("COMPOSE_DIR", shop.path());
    for k in LEAK_KEYS {
        if *k != "COMPOSE_DIR" {
            cmd.env_remove(k);
        }
    }
    cmd.env("SYNC_DUMP_QUICK", "0");
    cmd.env("SYNC_DUMP_ANONYMIZE", "1");
    cmd.env("SYNC_DUMP_CLEAN", "0");
    cmd.args(["shopware", "sync", "snapshot", "--dry-run", "--data", "db"]);
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let log = stdout(&out);
    assert!(!log.contains("--quick"), "{log}");
    assert!(!log.contains("--clean"), "{log}");
    assert!(log.contains("--anonymize"), "{log}");
}

#[test]
fn compose_project_name_override_in_network() {
    let shop = TempShop::new("proj");
    fs::write(
        shop.path().join(".env"),
        "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=staging
COMPOSE_PROJECT_NAME=customproj
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
    let out = snapshot(shop.path(), &["--dry-run", "--data", "db"]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    assert!(
        stdout(&out).contains("--network customproj_default"),
        "{}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("WARNING: COMPOSE_PROJECT_NAME=customproj"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn external_database_url_uses_host_network() {
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
    let out = snapshot(shop.path(), &["--dry-run", "--data", "db"]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let combined = format!("{}{}", stdout(&out), stderr(&out));
    assert!(!combined.contains("s3cret-value"), "{combined}");
    let log = stdout(&out);
    assert!(log.contains("--network host"), "{log}");
    assert!(log.contains("--host db.example.com"), "{log}");
    assert!(log.contains("--port 3307"), "{log}");
    assert!(log.contains("--username alice"), "{log}");
}

#[test]
fn remote_from_exits_2() {
    let shop = TempShop::new("remote");
    shop.write_min_shop();
    let out = snapshot(
        shop.path(),
        &["--dry-run", "--data", "db", "--from", "live"],
    );
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("not implemented"), "{}", stderr(&out));
    assert!(stderr(&out).contains("remote SSH"), "{}", stderr(&out));
}

#[test]
fn volumes_only_exits_2() {
    let shop = TempShop::new("vols");
    shop.write_min_shop();
    let out = snapshot(shop.path(), &["--dry-run", "--data", "media"]);
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr(&out));
    assert!(
        stderr(&out).contains("bind-mount volume snapshot"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn skip_db_and_skip_volumes_exits_1() {
    let shop = TempShop::new("empty");
    shop.write_min_shop();
    let out = snapshot(shop.path(), &["--dry-run", "--skip-db", "--skip-volumes"]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("Nothing to do"), "{}", stderr(&out));
}

#[test]
fn mysqldump_engine_exits_2() {
    let shop = TempShop::new("mysqldump");
    shop.write_min_shop();
    let mut cmd = bin();
    cmd.current_dir(shop.path());
    cmd.env("COMPOSE_DIR", shop.path());
    for k in LEAK_KEYS {
        if *k != "COMPOSE_DIR" {
            cmd.env_remove(k);
        }
    }
    cmd.env("SYNC_DUMP_ENGINE", "mysqldump");
    cmd.args(["shopware", "sync", "snapshot", "--dry-run", "--data", "db"]);
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("mysqldump"), "{}", stderr(&out));
}

#[test]
fn missing_shop_id_exits_1() {
    let shop = TempShop::new("noid");
    fs::write(
        shop.path().join(".env"),
        "MYSQL_USER=shop\nMYSQL_PASSWORD=x\nCOMPOSE_PROJECT_NAME=x-dev\n",
    )
    .unwrap();
    fs::write(
        shop.path().join("deploy/compose.yaml"),
        "services:\n  mysql:\n    image: mysql:8.4\n",
    )
    .unwrap();
    let out = snapshot(shop.path(), &["--dry-run", "--data", "db"]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(
        stderr(&out).contains("SHOPWARE_SHOP_ID"),
        "{}",
        stderr(&out)
    );
}
