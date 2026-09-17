//! Integration tests for `fyrst-cli shopware env init`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
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
            "fyrst-cli-it-initenv-{prefix}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(p.join("deploy")).unwrap();
        Self(p)
    }

    fn path(&self) -> &Path {
        &self.0
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
    "APP_SECRET",
    "IMAGE",
    "MYSQL_USER",
    "MYSQL_PASSWORD",
    "MYSQL_DATABASE",
    "MYSQL_ROOT_PASSWORD",
    "DATABASE_URL",
];

const MYSQL_PASS: &str = "super-secret-pass";
const EXISTING_SECRET: &str = "existing-app-secret-do-not-print-0123456789abcdef";

fn init_env(shop: &Path, extra: &[&str]) -> Output {
    let mut cmd = bin();
    cmd.current_dir(shop);
    cmd.env("COMPOSE_DIR", shop);
    for k in LEAK_KEYS {
        if *k != "COMPOSE_DIR" {
            cmd.env_remove(k);
        }
    }
    cmd.args(["shopware", "env", "init"]);
    cmd.args(extra);
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to run env init {extra:?}: {e}"))
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

fn assert_no_secrets(out: &Output, extras: &[&str]) {
    let text = combined(out);
    assert!(!text.contains(MYSQL_PASS), "MYSQL password leaked:\n{text}");
    assert!(
        !text.contains(EXISTING_SECRET),
        "APP_SECRET leaked:\n{text}"
    );
    for extra in extras {
        if extra.is_empty() {
            continue;
        }
        assert!(!text.contains(extra), "secret leaked ({extra}):\n{text}");
    }
}

#[test]
fn init_env_help_lists_flags() {
    let out = bin()
        .args(["shopware", "env", "init", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr={}", stderr(&out));
    let help = stdout(&out);
    for needle in [
        "--shop-id",
        "--env",
        "--image",
        "--dry-run",
        "COMPOSE_DIR",
        "COMPOSE_PROJECT_NAME",
    ] {
        assert!(help.contains(needle), "missing {needle} in:\n{help}");
    }
    assert!(
        !help.contains("--vps"),
        "env init --help must not list --vps:\n{help}"
    );
    assert!(
        !help.contains("generate-app-secret"),
        "env init --help must not list --generate-app-secret:\n{help}"
    );
    assert!(
        help.contains("APP_SECRET"),
        "env init --help should say APP_SECRET is not generated:\n{help}"
    );
    assert!(
        !help.to_ascii_lowercase().contains("wrap dump"),
        "env init --help must not wrap dump:\n{help}"
    );
}

#[test]
fn dry_run_prints_plan_and_does_not_write() {
    let shop = TempShop::new("dry");
    let original = format!(
        "\
SHOPWARE_SHOP_ID=
SHOPWARE_DEPLOY_ENV=
MYSQL_PASSWORD={MYSQL_PASS}
APP_SECRET={EXISTING_SECRET}
COMPOSE_PROJECT_NAME=sw-shop-acme
"
    );
    fs::write(shop.path().join(".env"), &original).unwrap();
    let out = init_env(
        shop.path(),
        &[
            "--shop-id",
            "acme",
            "--image",
            "ghcr.io/example/acme",
            "--dry-run",
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
    assert!(log.contains("DRY-RUN (no write)"), "{log}");
    assert!(log.contains("set SHOPWARE_SHOP_ID=acme"), "{log}");
    assert!(log.contains("set SHOPWARE_DEPLOY_ENV=live"), "{log}");
    assert!(log.contains("set IMAGE=ghcr.io/example/acme"), "{log}");
    assert!(log.contains("commented 1 COMPOSE_PROJECT_NAME"), "{log}");
    assert!(!log.contains("--vps"), "{log}");
    assert!(!log.contains("generate-app-secret"), "{log}");
    assert!(!log.contains("set APP_SECRET"), "{log}");
    assert!(
        log.contains("left unchanged: MYSQL passwords, APP_URL"),
        "{log}"
    );
    assert!(!log.contains("chmod 600"), "{log}");
    assert_no_secrets(&out, &[]);
    let after = fs::read_to_string(shop.path().join(".env")).unwrap();
    assert_eq!(after, original, "dry-run wrote .env");
}

#[test]
fn write_merges_example_comments_compose_project_name_and_chmod() {
    let shop = TempShop::new("write");
    fs::write(
        shop.path().join(".env"),
        format!(
            "\
SHOPWARE_SHOP_ID=
MYSQL_PASSWORD={MYSQL_PASS}
APP_URL=
COMPOSE_PROJECT_NAME=sw-shop-acme
"
        ),
    )
    .unwrap();
    fs::write(
        shop.path().join(".env.example"),
        "\
SHOPWARE_SHOP_ID=
MYSQL_PASSWORD=from-example
APP_URL=http://localhost
NEW_FROM_EXAMPLE=1
",
    )
    .unwrap();
    let out = init_env(shop.path(), &["--shop-id", "acme", "--env", "staging"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let log = stdout(&out);
    assert!(log.contains("Updated "), "{log}");
    assert!(log.contains("chmod 600 .env"), "{log}");
    assert!(
        log.contains("merge missing keys from .env.example: NEW_FROM_EXAMPLE"),
        "{log}"
    );
    assert!(log.contains("set SHOPWARE_SHOP_ID=acme"), "{log}");
    assert!(log.contains("set SHOPWARE_DEPLOY_ENV=staging"), "{log}");
    assert!(log.contains("commented 1 COMPOSE_PROJECT_NAME"), "{log}");
    assert!(!log.contains("--vps"), "{log}");
    assert_no_secrets(&out, &["from-example"]);

    let env = fs::read_to_string(shop.path().join(".env")).unwrap();
    assert!(env.contains("SHOPWARE_SHOP_ID=acme"));
    assert!(env.contains("SHOPWARE_DEPLOY_ENV=staging"));
    assert!(env.contains(&format!("MYSQL_PASSWORD={MYSQL_PASS}")));
    assert!(!env.contains("MYSQL_PASSWORD=from-example"));
    assert!(env.contains("NEW_FROM_EXAMPLE=1"));
    assert!(env.contains("# COMPOSE_PROJECT_NAME=sw-shop-acme # commented by deploy/init-env.sh"));
    assert!(!env.contains("--vps"));
    assert!(!env
        .lines()
        .any(|l| l.trim_start().starts_with("COMPOSE_PROJECT_NAME=")));
    let mode = fs::metadata(shop.path().join(".env"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "expected chmod 600, got {mode:o}");
}

#[test]
fn leaves_existing_app_secret_unchanged() {
    let shop = TempShop::new("keep-sec");
    fs::write(
        shop.path().join(".env"),
        format!(
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=live
APP_SECRET={EXISTING_SECRET}
MYSQL_PASSWORD={MYSQL_PASS}
"
        ),
    )
    .unwrap();
    let out = init_env(shop.path(), &[]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let log = stdout(&out);
    assert!(!log.contains("generate-app-secret"), "{log}");
    assert!(!log.contains("set APP_SECRET"), "{log}");
    assert_no_secrets(&out, &[]);
    let env = fs::read_to_string(shop.path().join(".env")).unwrap();
    assert!(env.contains(&format!("APP_SECRET={EXISTING_SECRET}")));
}

#[test]
fn does_not_generate_empty_app_secret() {
    let shop = TempShop::new("empty-sec");
    fs::write(
        shop.path().join(".env"),
        format!(
            "\
SHOPWARE_SHOP_ID=acme
APP_SECRET=
MYSQL_PASSWORD={MYSQL_PASS}
"
        ),
    )
    .unwrap();
    let out = init_env(shop.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let log = stdout(&out);
    assert!(!log.contains("generate-app-secret"), "{log}");
    assert!(!log.contains("set APP_SECRET"), "{log}");
    assert_no_secrets(&out, &[]);
    let env = fs::read_to_string(shop.path().join(".env")).unwrap();
    let secret = env
        .lines()
        .find_map(|l| l.strip_prefix("APP_SECRET="))
        .unwrap_or("missing");
    assert_eq!(
        secret, "",
        "env init must not write APP_SECRET, got {secret:?}"
    );
}

#[test]
fn vps_flag_is_rejected() {
    let shop = TempShop::new("gone-vps");
    fs::write(shop.path().join(".env"), "SHOPWARE_SHOP_ID=acme\n").unwrap();
    let out = init_env(shop.path(), &["--vps"]);
    assert_ne!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let text = combined(&out);
    assert!(
        text.contains("unexpected argument") || text.contains("unexpected"),
        "{text}"
    );
}

#[test]
fn already_commented_compose_project_name_reports_none() {
    let shop = TempShop::new("already-cpn");
    fs::write(
        shop.path().join(".env"),
        "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=live
# COMPOSE_PROJECT_NAME=sw-shop-acme
",
    )
    .unwrap();
    let out = init_env(shop.path(), &[]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let log = stdout(&out);
    assert!(log.contains("no uncommented COMPOSE_PROJECT_NAME"), "{log}");
    assert!(!log.contains("commented 1 COMPOSE_PROJECT_NAME"), "{log}");
    assert!(!log.contains("--vps"), "{log}");
    let env = fs::read_to_string(shop.path().join(".env")).unwrap();
    assert!(env.contains("# COMPOSE_PROJECT_NAME=sw-shop-acme"));
    assert!(!env
        .lines()
        .any(|l| l.trim_start().starts_with("COMPOSE_PROJECT_NAME=")));
}

#[test]
fn generate_app_secret_flag_is_rejected() {
    let shop = TempShop::new("gone-flag");
    fs::write(shop.path().join(".env"), "SHOPWARE_SHOP_ID=acme\n").unwrap();
    let out = init_env(shop.path(), &["--generate-app-secret"]);
    assert_ne!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let text = combined(&out);
    assert!(
        text.contains("unexpected argument") || text.contains("unexpected"),
        "{text}"
    );
}

#[test]
fn copy_example_dry_run_does_not_create_env() {
    let shop = TempShop::new("copy-dry");
    fs::write(
        shop.path().join(".env.example"),
        format!(
            "\
SHOPWARE_SHOP_ID=
MYSQL_PASSWORD={MYSQL_PASS}
"
        ),
    )
    .unwrap();
    let out = init_env(shop.path(), &["--shop-id", "acme", "--dry-run"]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    assert!(stdout(&out).contains("copy .env.example → .env"));
    assert_no_secrets(&out, &[]);
    assert!(!shop.path().join(".env").is_file());
}

#[test]
fn missing_shop_id_exits_1_not_stub() {
    let shop = TempShop::new("noid");
    fs::write(shop.path().join(".env"), "SHOPWARE_DEPLOY_ENV=live\n").unwrap();
    let out = init_env(shop.path(), &["--dry-run"]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("SHOPWARE_SHOP_ID is empty"), "{err}");
    assert!(err.contains("--shop-id"), "{err}");
    assert!(!err.contains("not implemented"), "{err}");
}

#[test]
fn invalid_slug_exits_1() {
    let shop = TempShop::new("slug");
    fs::write(shop.path().join(".env"), "SHOPWARE_SHOP_ID=\n").unwrap();
    let out = init_env(shop.path(), &["--shop-id", "ACME", "--dry-run"]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("Invalid --shop-id 'ACME'"), "{err}");
    assert!(!err.contains("not implemented"), "{err}");
}

#[test]
fn invalid_env_is_not_not_implemented() {
    let shop = TempShop::new("badenv");
    fs::write(shop.path().join(".env"), "SHOPWARE_SHOP_ID=acme\n").unwrap();
    let out = init_env(shop.path(), &["--env", "prod", "--dry-run"]);
    assert_ne!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let text = combined(&out);
    assert!(
        text.contains("invalid value") || text.contains("Invalid --env"),
        "{text}"
    );
    assert!(!text.contains("not implemented"), "{text}");
}

#[test]
fn xtrace_env_is_refused() {
    let shop = TempShop::new("xtrace");
    fs::write(shop.path().join(".env"), "SHOPWARE_SHOP_ID=acme\n").unwrap();
    let mut cmd = bin();
    cmd.current_dir(shop.path());
    cmd.env("COMPOSE_DIR", shop.path());
    cmd.env("BASH_XTRACEFD", "2");
    cmd.args(["shopware", "env", "init", "--dry-run"]);
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("xtrace"), "{err}");
    assert!(err.contains("credentials may be in .env"), "{err}");
    assert!(!err.contains("not implemented"), "{err}");
}
