//! Integration tests for `fyrst-cli shopware sync local` (never DB).

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
            "fyrst-cli-it-sl-{prefix}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(p.join("deploy")).unwrap();
        fs::create_dir_all(p.join("public")).unwrap();
        fs::write(p.join("composer.json"), "{}\n").unwrap();
        Self(p)
    }

    fn named(parent_prefix: &str, basename: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let parent = std::env::temp_dir().join(format!(
            "fyrst-cli-it-sl-{parent_prefix}-{}-{nanos}",
            std::process::id()
        ));
        let p = parent.join(basename);
        fs::create_dir_all(p.join("deploy")).unwrap();
        fs::create_dir_all(p.join("public")).unwrap();
        fs::write(p.join("composer.json"), "{}\n").unwrap();
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
SHOPWARE_DEPLOY_ENV=dev
SHOPWARE_DATA_ROOT=/var/lib/shopware/data/acme/dev
",
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
    "SHOPWARE_SSH_HOST",
    "SHOPWARE_SSH_USER",
    "SHOPWARE_SSH_KEY",
    "SHOPWARE_REMOTE_DATA_ROOT",
    "SYNC_ENV",
    "SYNC_SOURCE_ENV",
    "SYNC_REMOTE_DATA_ROOT",
    "SYNC_DATA_ROOT",
    "SYNC_SSH_HOST",
    "SYNC_SSH_USER",
    "SYNC_SSH_PORT",
    "SYNC_SSH_KEY",
    "SYNC_LIVE_SSH_HOST",
    "SYNC_LIVE_SSH_USER",
    "SYNC_LIVE_SSH_PORT",
    "SYNC_LIVE_SSH_KEY",
    "SYNC_LIVE_DATA_ROOT",
    "SYNC_STAGING_DATA_ROOT",
];

fn sync_local(shop: &Path, extra: &[&str]) -> Output {
    let mut cmd = bin();
    cmd.current_dir(shop);
    cmd.env("COMPOSE_DIR", shop);
    for k in LEAK_KEYS {
        if *k != "COMPOSE_DIR" {
            cmd.env_remove(k);
        }
    }
    cmd.args(["shopware", "sync", "local"]);
    cmd.args(extra);
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to run sync local {extra:?}: {e}"))
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn help_lists_flags() {
    let out = bin()
        .args(["shopware", "sync", "local", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    for needle in [
        "--from",
        "--data",
        "--remote-data-root",
        "--delete",
        "--dry-run",
        "not db",
    ] {
        assert!(help.contains(needle), "missing {needle} in:\n{help}");
    }
    assert!(
        !help.to_ascii_lowercase().contains("dump db via"),
        "sync local --help must not wrap dump:\n{help}"
    );
}

#[test]
fn dry_run_prints_remap_and_does_not_copy() {
    let shop = TempShop::new("dry-run");
    shop.write_min_shop();
    let out = sync_local(shop.path(), &["--dry-run"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let log = stdout(&out);
    assert!(log.contains("from=live"), "{log}");
    assert!(log.contains("delete=0"), "{log}");
    assert!(log.contains("not SHOPWARE_DATA_ROOT"), "{log}");
    for mapping in [
        "live:/var/lib/shopware/data/acme/live/media/ → ./public/media/",
        "live:/var/lib/shopware/data/acme/live/files/ → ./files/",
        "live:/var/lib/shopware/data/acme/live/thumbnail/ → ./public/thumbnail/",
        "live:/var/lib/shopware/data/acme/live/theme/ → ./public/theme/",
        "live:/var/lib/shopware/data/acme/live/sitemap/ → ./public/sitemap/",
    ] {
        assert!(log.contains(mapping), "missing mapping `{mapping}`:\n{log}");
    }
    assert!(log.contains("DRY-RUN rsync -azH"), "{log}");
    assert!(!log.contains("--numeric-ids"), "{log}");
    assert!(
        !log.contains("DRY-RUN rsync -azH --delete"),
        "delete must be opt-in:\n{log}"
    );
    assert!(
        log.contains("Reminder: shopware-cli project console cache:clear"),
        "{log}"
    );
    assert!(log.contains("database not touched"), "{log}");
    assert!(
        !shop.path().join("public/media").is_dir(),
        "dry-run copied media"
    );
    assert!(!shop.path().join("files").exists(), "dry-run copied files");
    let combined = format!("{log}{}", stderr(&out));
    assert!(
        !combined.contains("/var/lib/shopware/data/acme/dev/"),
        "must not rsync into SHOPWARE_DATA_ROOT:\n{combined}"
    );
    assert!(
        !combined.contains("docker run"),
        "must not wrap dump:\n{combined}"
    );
    assert!(
        !combined.contains("ghcr.io/shopware/shopware-cli"),
        "{combined}"
    );
}

#[test]
fn data_db_exits_nonzero() {
    let shop = TempShop::new("db");
    shop.write_min_shop();
    for spec in ["db", "database", "mysql"] {
        let out = sync_local(shop.path(), &["--dry-run", "--data", spec]);
        assert_eq!(
            out.status.code(),
            Some(1),
            "spec={spec} stderr={}",
            stderr(&out)
        );
        let err = stderr(&out);
        assert!(err.contains("does not restore the database"), "{err}");
        assert!(err.contains("shopware-cli project dump"), "{err}");
        assert!(err.contains("db import"), "{err}");
        assert!(!stdout(&out).contains("DRY-RUN rsync"), "{}", stdout(&out));
    }
}

#[test]
fn data_mysql_and_redis_volumes_refused() {
    let shop = TempShop::new("vol");
    shop.write_min_shop();
    for spec in ["mysql_data", "redis_data"] {
        let out = sync_local(shop.path(), &["--dry-run", "--data", spec]);
        assert_eq!(
            out.status.code(),
            Some(1),
            "spec={spec} stderr={}",
            stderr(&out)
        );
        assert!(stderr(&out).contains(spec), "{}", stderr(&out));
    }
}

#[test]
fn missing_shop_id_without_remote_root() {
    let shop = TempShop::new("noid");
    fs::write(shop.path().join(".env"), "SHOPWARE_DEPLOY_ENV=dev\n").unwrap();
    let out = sync_local(shop.path(), &["--dry-run", "--data", "media"]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("SHOPWARE_SHOP_ID"), "{err}");
    assert!(err.contains("--remote-data-root"), "{err}");
}

#[test]
fn delete_opt_in_on_dry_run_rsync() {
    let shop = TempShop::new("delete");
    shop.write_min_shop();
    let out = sync_local(shop.path(), &["--dry-run", "--data", "media", "--delete"]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let log = stdout(&out);
    assert!(log.contains("delete=1"), "{log}");
    assert!(log.contains("DRY-RUN rsync -azH --delete"), "{log}");
    assert!(
        log.contains("live:/var/lib/shopware/data/acme/live/media/ → ./public/media/"),
        "{log}"
    );
}

#[test]
fn live_checkout_warns_on_stderr() {
    let shop = TempShop::named("ck", "live");
    shop.write_min_shop();
    let out = sync_local(shop.path(), &["--dry-run", "--data", "files"]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("WARNING:"), "{}", stderr(&out));
    assert!(stderr(&out).contains("named 'live'"), "{}", stderr(&out));
    assert!(stdout(&out).contains(" → ./files/"), "{}", stdout(&out));
}

#[test]
fn data_all_is_refused() {
    let shop = TempShop::new("all");
    shop.write_min_shop();
    let out = sync_local(shop.path(), &["--dry-run", "--data", "all"]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("Refusing --data all"), "{err}");
    assert!(err.contains("includes db"), "{err}");
    assert!(!stdout(&out).contains("DRY-RUN rsync"), "{}", stdout(&out));
}
