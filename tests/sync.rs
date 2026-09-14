//! Integration tests for `fyrst-cli shopware sync pull` (pull, no dump wrap).

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
            "fyrst-cli-it-sync-{prefix}-{}-{nanos}",
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
        let data = self.0.join("data");
        fs::create_dir_all(&data).unwrap();
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
SHOPWARE_SSH_HOST=live.example.com
SHOPWARE_SSH_USER=deploy
SHOPWARE_REMOTE_DATA_ROOT=/var/lib/shopware/data/acme/live
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

    fn write_live(&self, allow: bool, rewrite: bool) {
        let data = self.0.join("data");
        fs::create_dir_all(&data).unwrap();
        let mut body = format!(
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=live
MYSQL_USER=shop
MYSQL_PASSWORD=super-secret-pass
MYSQL_DATABASE=shopware
SHOPWARE_DATA_ROOT={}
SHOPWARE_SSH_HOST=live.example.com
SHOPWARE_SSH_USER=deploy
SHOPWARE_REMOTE_DATA_ROOT=/var/lib/shopware/data/acme/live
",
            data.display()
        );
        if allow {
            body.push_str("SHOPWARE_ALLOW_LIVE_RESTORE=1\n");
        }
        if rewrite {
            body.push_str("APP_URL=https://staging.example.com\n");
        }
        fs::write(self.0.join(".env"), body).unwrap();
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
    "SHOPWARE_ALLOW_LIVE_RESTORE",
    "SHOPWARE_SSH_HOST",
    "SHOPWARE_SSH_USER",
    "SHOPWARE_SSH_KEY",
    "SHOPWARE_REMOTE_DATA_ROOT",
    "APP_URL",
    "SYNC_MYSQL_CLIENT_IMAGE",
    "SYNC_SNAPSHOT_DIR",
    "SYNC_ALLOW_LIVE_RESTORE",
    "SYNC_ENV",
    "SYNC_SSH_HOST",
    "SYNC_SSH_USER",
    "SYNC_SSH_PORT",
    "SYNC_SSH_KEY",
    "SYNC_REMOTE_PATH",
    "SYNC_REMOTE_DATA_ROOT",
    "SYNC_SOURCE_ENV",
    "SYNC_DATA_ROOT",
    "SYNC_REWRITE_APP_URL",
    "SYNC_REWRITE_URL_MAP",
    "SYNC_APP_URL",
    "SYNC_POST_RESTORE_CMD",
    "SYNC_ARCHIVE_IMAGE",
];

fn sync_cmd(shop: &Path, extra: &[&str]) -> Output {
    let mut cmd = bin();
    cmd.current_dir(shop);
    cmd.env("COMPOSE_DIR", shop);
    for k in LEAK_KEYS {
        if *k != "COMPOSE_DIR" {
            cmd.env_remove(k);
        }
    }
    cmd.args(["shopware", "sync", "pull"]);
    cmd.args(extra);
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to run sync pull {extra:?}: {e}"))
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

fn assert_no_dump_wrap(text: &str) {
    assert!(
        !text.contains("ghcr.io/shopware/shopware-cli"),
        "must not pull/run shopware-cli image:\n{text}"
    );
    assert!(
        !text.contains("shopware-cli:0.18"),
        "must not pin/run the overlay dump image:\n{text}"
    );
    assert!(
        !text.contains("DRY-RUN docker run --rm --network"),
        "must not print an overlay dump docker plan:\n{text}"
    );
    assert!(
        !text.contains("SYNC_DUMP_ENGINE"),
        "must not mention dump engine wrap:\n{text}"
    );
}

#[test]
fn sync_help_lists_flags() {
    let out = bin()
        .args(["shopware", "sync", "pull", "--help"])
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
fn volume_pull_dry_run_prints_rsync_without_copying() {
    let shop = TempShop::new("vol-dry");
    shop.write_staging();
    let media = shop.path().join("data/media");
    let files = shop.path().join("data/files");
    assert!(!media.exists());
    assert!(!files.exists());

    let out = sync_cmd(
        shop.path(),
        &[
            "--dry-run",
            "--from",
            "live",
            "--skip-db",
            "--data",
            "media,files",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let log = stdout(&out);
    assert!(log.contains("DRY-RUN rsync -az --delete"), "{log}");
    assert!(
        log.contains("deploy@live.example.com:/var/lib/shopware/data/acme/live/media/"),
        "{log}"
    );
    assert!(
        log.contains("deploy@live.example.com:/var/lib/shopware/data/acme/live/files/"),
        "{log}"
    );
    assert!(log.contains("chown 82:82"), "{log}");
    assert!(log.contains("DRY-RUN ssh -o BatchMode=yes"), "{log}");
    assert!(log.contains("true"), "{log}");
    assert!(
        !media.exists() && !files.exists(),
        "dry-run must not copy bind-mount trees"
    );
    assert_no_dump_wrap(&combined(&out));
    assert!(!combined(&out).contains("super-secret-pass"));
}

#[test]
fn db_not_wrapped_instructs_shopware_cli() {
    let shop = TempShop::new("db-nodump");
    shop.write_staging();
    let out = sync_cmd(
        shop.path(),
        &[
            "--dry-run",
            "--from",
            "live",
            "--data",
            "db",
            "--skip-volumes",
        ],
    );
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("shopware-cli project dump"), "{err}");
    assert!(err.contains("db import"), "{err}");
    assert!(err.contains("db.sql.gz"), "{err}");
    assert_no_dump_wrap(&combined(&out));
    let all = combined(&out);
    assert!(!all.contains("DRY-RUN docker run"), "{all}");
}

#[test]
fn db_existing_dump_uses_import_module() {
    let shop = TempShop::new("db-imp");
    shop.write_staging();
    fs::write(
        shop.path().join("var/runtime-sync/db.sql.gz"),
        b"placeholder",
    )
    .unwrap();
    let out = sync_cmd(
        shop.path(),
        &[
            "--dry-run",
            "--from",
            "live",
            "--data",
            "db",
            "--skip-volumes",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let log = stdout(&out);
    assert!(log.contains("DRY-RUN gzip -dc"), "{log}");
    assert!(log.contains("exec -T mysql"), "{log}");
    assert_no_dump_wrap(&combined(&out));
    assert!(!combined(&out).contains("super-secret-pass"));
}

#[test]
fn mixed_data_imports_and_prints_rsync() {
    let shop = TempShop::new("mixed");
    shop.write_staging();
    fs::write(shop.path().join("var/runtime-sync/db.sql"), b"SELECT 1;\n").unwrap();
    let out = sync_cmd(
        shop.path(),
        &["--dry-run", "--from", "live", "--data", "db,media"],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let log = stdout(&out);
    assert!(log.contains("DRY-RUN cat "), "{log}");
    assert!(log.contains("DRY-RUN rsync -az --delete"), "{log}");
    assert!(
        log.contains("/var/lib/shopware/data/acme/live/media/"),
        "{log}"
    );
    assert_no_dump_wrap(&combined(&out));
}

#[test]
fn live_volume_only_refuses_without_env() {
    let shop = TempShop::new("live-vol");
    shop.write_live(false, false);
    let out = sync_cmd(
        shop.path(),
        &[
            "--dry-run",
            "--from",
            "live",
            "--skip-db",
            "--data",
            "media",
        ],
    );
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(
        stderr(&out).contains("SHOPWARE_ALLOW_LIVE_RESTORE"),
        "{}",
        stderr(&out)
    );
    assert!(stderr(&out).contains("live"), "{}", stderr(&out));
}

#[test]
fn live_allow_env_volume_dry_run_warns() {
    let shop = TempShop::new("live-ok");
    shop.write_live(true, false);
    let out = sync_cmd(
        shop.path(),
        &[
            "--dry-run",
            "--from",
            "live",
            "--skip-db",
            "--data",
            "media",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    assert!(stderr(&out).contains("WARNING:"), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("DRY-RUN rsync -az --delete"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn rewrite_skipped_on_live_even_with_allow() {
    let shop = TempShop::new("live-rw");
    shop.write_live(true, true);
    let out = sync_cmd(
        shop.path(),
        &[
            "--dry-run",
            "--from",
            "live",
            "--skip-db",
            "--data",
            "media",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let all = combined(&out);
    assert!(all.contains("live"), "{all}");
    assert!(
        stdout(&out).contains("Skipping sales-channel domain rewrite")
            || stdout(&out).contains("DRY-RUN rsync"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn rewrite_skipped_when_skip_db() {
    let shop = TempShop::new("rw-skip");
    shop.write_staging();
    fs::write(
        shop.path().join(".env"),
        format!(
            "{}\nAPP_URL=https://staging.example.com\n",
            fs::read_to_string(shop.path().join(".env")).unwrap().trim()
        ),
    )
    .unwrap();
    let out = sync_cmd(
        shop.path(),
        &[
            "--dry-run",
            "--from",
            "live",
            "--skip-db",
            "--data",
            "media",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let log = stdout(&out);
    assert!(log.contains("db was skipped"), "{log}");
    assert!(log.contains("DRY-RUN rsync -az --delete"), "{log}");
    assert!(!log.contains("fyrst:sales-channel:rewrite-urls"), "{log}");
}

#[test]
fn from_local_is_pipeline_check() {
    let shop = TempShop::new("local");
    shop.write_staging();
    fs::create_dir_all(shop.path().join("data/media")).unwrap();
    let out = sync_cmd(
        shop.path(),
        &[
            "--dry-run",
            "--from",
            "local",
            "--skip-db",
            "--data",
            "media",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let log = stdout(&out);
    assert!(log.contains("pipeline check"), "{log}");
    assert!(
        log.contains("Prefer --from <live-alias> on staging"),
        "{log}"
    );
    assert!(log.contains("DRY-RUN rsync"), "{log}");
    assert_no_dump_wrap(&combined(&out));
}

#[test]
fn refuses_mysql_data_volume() {
    let shop = TempShop::new("mysqlvol");
    shop.write_staging();
    let out = sync_cmd(
        shop.path(),
        &[
            "--dry-run",
            "--from",
            "live",
            "--skip-db",
            "--data",
            "mysql_data",
        ],
    );
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("mysql_data"), "{}", stderr(&out));
}

#[test]
fn all_without_dump_still_prints_volume_plan() {
    let shop = TempShop::new("all-nodump");
    shop.write_staging();
    let out = sync_cmd(
        shop.path(),
        &["--dry-run", "--from", "live", "--data", "all"],
    );
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(
        stderr(&out).contains("shopware-cli project dump"),
        "{}",
        stderr(&out)
    );
    let log = stdout(&out);
    assert!(log.contains("DRY-RUN rsync -az --delete"), "{log}");
    assert_no_dump_wrap(&combined(&out));
}
