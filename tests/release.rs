//! Integration tests for `fyrst-cli shopware deploy release`.

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
            "fyrst-cli-it-rel-{prefix}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(p.join("deploy")).unwrap();
        Self(p)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write_vps_compose(&self) {
        for f in [
            "deploy/compose.yaml",
            "deploy/compose.prod.yaml",
            "deploy/compose.vps.yaml",
        ] {
            fs::write(
                self.0.join(f),
                "services:\n  web:\n    image: ${IMAGE:-x}:${IMAGE_TAG:-latest}\n",
            )
            .unwrap();
        }
    }

    fn write_env(&self, extra: &str) {
        fs::write(
            self.0.join(".env"),
            format!(
                "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=staging
IMAGE=ghcr.io/fyrst-dev/acme
IMAGE_TAG=deadbeef
MYSQL_PASSWORD=super-secret-pass
DATABASE_URL=mysql://alice:s3cret-value@db.example.com:3307/shop
{extra}"
            ),
        )
        .unwrap();
        self.write_vps_compose();
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
    "IMAGE",
    "IMAGE_TAG",
    "COMPOSE_PROFILES",
    "SMOKE_URL",
    "PULL_POLICY",
    "SKIP_PULL",
    "ROLLBACK_ON_SMOKE_FAIL",
    "MYSQL_PASSWORD",
    "MYSQL_ROOT_PASSWORD",
    "DATABASE_URL",
    "APP_SECRET",
];

fn release(shop: &Path, extra: &[&str], extra_env: &[(&str, &str)]) -> Output {
    let mut cmd = bin();
    cmd.current_dir(shop);
    cmd.env("COMPOSE_DIR", shop);
    for k in LEAK_KEYS {
        if *k != "COMPOSE_DIR" {
            cmd.env_remove(k);
        }
    }
    for (k, v) in extra_env {
        cmd.env(*k, v);
    }
    cmd.args(["shopware", "deploy", "release"]);
    cmd.args(extra);
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to run release {extra:?}: {e}"))
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
fn release_help_documents_flags() {
    let out = bin()
        .args(["shopware", "deploy", "release", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    for needle in ["--dry-run", "--skip-pull"] {
        assert!(help.contains(needle), "missing {needle} in:\n{help}");
    }
}

#[test]
fn dry_run_prints_compose_sequence_without_passwords() {
    let shop = TempShop::new("dry");
    shop.write_env("SMOKE_URL=http://127.0.0.1:8000\nCOMPOSE_PROFILES=redis,worker,scheduler\nCOMPOSE_PROJECT_NAME=shopware-acme\n");
    fs::write(shop.path().join(".deployed-tag"), "oldtag\n").unwrap();
    let out = release(shop.path(), &["--dry-run"], &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let log = stdout(&out);
    let all = combined(&out);
    assert!(
        !all.contains("super-secret-pass"),
        "password leaked:\n{all}"
    );
    assert!(!all.contains("s3cret-value"), "DATABASE_URL leaked:\n{all}");
    assert!(!all.contains("mysql://alice"), "{all}");
    assert!(log.contains("Previous tag: oldtag"), "{log}");
    assert!(
        log.contains("DRY-RUN docker compose --env-file .env -f deploy/compose.yaml -f deploy/compose.prod.yaml -f deploy/compose.vps.yaml"),
        "{log}"
    );
    assert!(log.contains("-p acme-staging"), "{log}");
    assert!(!log.contains("-p shopware-acme"), "{log}");
    assert!(
        log.contains("--profile setup run --rm --pull never setup"),
        "{log}"
    );
    assert!(
        log.contains("up -d --no-build --remove-orphans web"),
        "{log}"
    );
    assert!(
        log.contains("DRY-RUN extra profiles: redis,worker,scheduler"),
        "{log}"
    );
    assert!(
        log.contains("DRY-RUN would GET http://127.0.0.1:8000"),
        "{log}"
    );
    assert!(
        log.contains("DRY-RUN would write .deployed-tag=deadbeef"),
        "{log}"
    );
    assert!(!log.contains("--build"), "{log}");
    assert!(!log.contains("--skip-theme-compile"), "{log}");
    assert!(!log.contains("--skip-assets-install"), "{log}");
    assert!(!shop.path().join(".previous-tag").is_file());
    assert_eq!(
        fs::read_to_string(shop.path().join(".deployed-tag"))
            .unwrap()
            .trim(),
        "oldtag"
    );
}

#[test]
fn dry_run_passes_env_local_and_pins_identity_name() {
    let shop = TempShop::new("env-local");
    shop.write_env("");
    fs::write(shop.path().join(".env.local"), "SHOPWARE_DEPLOY_ENV=dev\n").unwrap();
    let out = release(shop.path(), &["--dry-run"], &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let log = stdout(&out);
    assert!(
        log.contains(
            "docker compose --env-file .env --env-file .env.local -f deploy/compose.yaml -f deploy/compose.prod.yaml -f deploy/compose.vps.yaml"
        ),
        "{log}"
    );
    assert!(log.contains("-p acme-dev"), "{log}");
    assert!(!log.contains("-p acme-staging"), "{log}");
    assert!(!log.contains("-p shopware-"), "{log}");
}

#[test]
fn skip_pull_dry_run_sets_pull_policy_never() {
    let shop = TempShop::new("skip");
    shop.write_env("");
    let out = release(shop.path(), &["--dry-run", "--skip-pull"], &[]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let log = stdout(&out);
    assert!(
        log.contains("DRY-RUN skip compose pull (SKIP_PULL=1 / PULL_POLICY=never)"),
        "{log}"
    );
    assert!(
        log.contains("up -d --no-build --pull never --remove-orphans web"),
        "{log}"
    );
    assert!(
        !log.contains("DRY-RUN docker compose --env-file .env")
            || !log
                .lines()
                .any(|l| l.contains("DRY-RUN docker compose") && l.ends_with(" pull")),
        "skip-pull still planned a compose pull:\n{log}"
    );
}

#[test]
fn skip_pull_via_env_same_as_flag() {
    let shop = TempShop::new("skip-env");
    shop.write_env("PULL_POLICY=never\n");
    let out = release(shop.path(), &["--dry-run"], &[]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let log = stdout(&out);
    assert!(log.contains("PULL_POLICY=never"), "{log}");
    assert!(log.contains("skip compose pull"), "{log}");
}

#[test]
fn ci_image_tag_wins_over_env_latest() {
    let shop = TempShop::new("ci-tag");
    shop.write_env("IMAGE_TAG=latest\nIMAGE=ghcr.io/from-file/shop\n");
    let out = release(
        shop.path(),
        &["--dry-run"],
        &[
            ("IMAGE", "ghcr.io/from-ci/shop"),
            ("IMAGE_TAG", "abc123deadbeef"),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let log = stdout(&out);
    assert!(log.contains("ghcr.io/from-ci/shop:abc123deadbeef"), "{log}");
    assert!(!log.contains("ghcr.io/from-file/shop:latest"), "{log}");
    assert!(log.contains(".deployed-tag=abc123deadbeef"), "{log}");
}

#[test]
fn live_empty_profiles_warns() {
    let shop = TempShop::new("live");
    fs::write(
        shop.path().join(".env"),
        "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=live
IMAGE=ghcr.io/fyrst-dev/acme
IMAGE_TAG=deadbeef
",
    )
    .unwrap();
    shop.write_vps_compose();
    let out = release(shop.path(), &["--dry-run"], &[]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("WARNING:"), "{err}");
    assert!(
        err.contains("COMPOSE_PROFILES=redis,worker,scheduler"),
        "{err}"
    );
}

#[test]
fn setup_profile_refused() {
    let shop = TempShop::new("setup");
    shop.write_env("COMPOSE_PROFILES=redis,setup\n");
    let out = release(shop.path(), &["--dry-run"], &[]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("setup"), "{}", stderr(&out));
}

#[test]
fn missing_compose_vps_exits_1() {
    let shop = TempShop::new("novps");
    fs::write(
        shop.path().join(".env"),
        "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=dev\nIMAGE=x\nIMAGE_TAG=t\n",
    )
    .unwrap();
    fs::write(
        shop.path().join("deploy/compose.yaml"),
        "services:\n  web:\n    image: x\n",
    )
    .unwrap();
    fs::write(
        shop.path().join("deploy/compose.prod.yaml"),
        "services:\n  web:\n    image: x\n",
    )
    .unwrap();
    let out = release(shop.path(), &["--dry-run"], &[]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(
        stderr(&out).contains("compose.vps.yaml"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn missing_image_tag_exits_1() {
    let shop = TempShop::new("notag");
    fs::write(
        shop.path().join(".env"),
        "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=dev\nIMAGE=ghcr.io/fyrst-dev/acme\n",
    )
    .unwrap();
    shop.write_vps_compose();
    let out = release(shop.path(), &["--dry-run"], &[]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("IMAGE_TAG"), "{}", stderr(&out));
}
