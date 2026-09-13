//! Integration tests for `fyrst-cli shopware rollback`.

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
            "fyrst-cli-it-rb-{prefix}-{}-{nanos}",
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
IMAGE=ghcr.io/example/acme
IMAGE_TAG=from-env-file
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=staging
MYSQL_PASSWORD=super-secret-pass
DATABASE_URL=mysql://alice:s3cret-value@db.example.com/shop
",
        )
        .unwrap();
        for name in [
            "deploy/compose.yaml",
            "deploy/compose.prod.yaml",
            "deploy/compose.vps.yaml",
        ] {
            fs::write(
                self.0.join(name),
                "services:\n  web:\n    image: ${IMAGE}:${IMAGE_TAG}\n",
            )
            .unwrap();
        }
    }

    fn write_previous(&self, tag: &str) {
        fs::write(self.0.join(".previous-tag"), tag).unwrap();
    }

    fn install_docker_trap(&self) -> PathBuf {
        let bin = self.0.join("trap-bin");
        fs::create_dir_all(&bin).unwrap();
        for name in ["docker", "curl"] {
            let p = bin.join(name);
            fs::write(
                &p,
                format!(
                    "#!/bin/sh\necho \"{} invoked: $*\" >> \"{}\"\nexit 99\n",
                    name,
                    self.0.join("trap-invoked").display()
                ),
            )
            .unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        bin
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
];

fn rollback(shop: &Path, extra: &[&str], extra_env: &[(&str, &str)]) -> Output {
    rollback_with_path(shop, extra, extra_env, None)
}

fn rollback_with_path(
    shop: &Path,
    extra: &[&str],
    extra_env: &[(&str, &str)],
    trap_bin: Option<&Path>,
) -> Output {
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
    if let Some(trap) = trap_bin {
        let mut path = trap.display().to_string();
        if let Ok(orig) = std::env::var("PATH") {
            path.push(':');
            path.push_str(&orig);
        }
        cmd.env("PATH", path);
    }
    cmd.args(["shopware", "rollback"]);
    cmd.args(extra);
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to run rollback {extra:?}: {e}"))
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn rollback_help_lists_flags() {
    let out = bin()
        .args(["shopware", "rollback", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    for needle in ["--dry-run", "--skip-pull"] {
        assert!(help.contains(needle), "missing {needle} in:\n{help}");
    }
}

#[test]
fn missing_previous_tag_exits_1() {
    let shop = TempShop::new("missing-tag");
    shop.write_min_shop();
    let out = rollback(shop.path(), &["--dry-run"], &[]);
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    let err = stderr(&out);
    assert!(!err.contains("not implemented"), "{err}");
    assert!(err.contains(".previous-tag"), "{err}");
    assert!(err.contains("IMAGE_TAG"), "{err}");
    assert!(!shop.path().join(".deployed-tag").is_file());
}

#[test]
fn empty_previous_tag_exits_1() {
    let shop = TempShop::new("empty-tag");
    shop.write_min_shop();
    shop.write_previous("  \n");
    let out = rollback(
        shop.path(),
        &["--dry-run"],
        &[("IMAGE_TAG", "from-process")],
    );
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("empty"), "{err}");
    assert!(err.contains("IMAGE_TAG"), "{err}");
    assert!(!err.contains("not implemented"), "{err}");
}

#[test]
fn process_env_image_tag_is_ignored() {
    let shop = TempShop::new("ignore-proc-tag");
    shop.write_min_shop();
    shop.write_previous("rolled-back-sha\n");
    let trap = shop.install_docker_trap();
    let out = rollback_with_path(
        shop.path(),
        &["--dry-run"],
        &[("IMAGE_TAG", "from-process-must-not-win")],
        Some(&trap),
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr(&out),
        stdout(&out)
    );
    let log = stdout(&out);
    assert!(log.contains("rolled-back-sha"), "{log}");
    assert!(!log.contains("from-process-must-not-win"), "{log}");
    assert!(!log.contains("from-env-file"), "{log}");
    assert!(
        log.contains("DRY-RUN would write .deployed-tag=rolled-back-sha"),
        "{log}"
    );
    let combined = format!("{}{}", log, stderr(&out));
    assert!(!combined.contains("super-secret-pass"), "{combined}");
    assert!(!combined.contains("s3cret-value"), "{combined}");
    assert!(!combined.contains("DATABASE_URL="), "{combined}");
    assert!(!shop.path().join(".deployed-tag").is_file());
    assert!(
        !shop.path().join("trap-invoked").is_file(),
        "dry-run must not invoke docker/curl"
    );
}

#[test]
fn dry_run_prints_same_compose_order_as_release() {
    let shop = TempShop::new("dry-order");
    shop.write_min_shop();
    shop.write_previous("abc123\n");
    fs::write(
        shop.path().join(".env.prod"),
        "COMPOSE_PROFILES=redis,worker\n",
    )
    .unwrap();
    let trap = shop.install_docker_trap();
    let out = rollback_with_path(shop.path(), &["--dry-run"], &[], Some(&trap));
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
            "docker compose --env-file .env -f deploy/compose.yaml -f deploy/compose.prod.yaml -f deploy/compose.vps.yaml"
        ),
        "{log}"
    );
    let setup_idx = log
        .find("--profile setup run --rm --pull never setup")
        .unwrap_or_else(|| panic!("setup missing:\n{log}"));
    let web_idx = log
        .find("up -d --no-build --remove-orphans web")
        .unwrap_or_else(|| panic!("web missing:\n{log}"));
    assert!(setup_idx < web_idx, "setup must precede web:\n{log}");
    assert!(
        log.contains("DRY-RUN extra profiles: redis,worker"),
        "{log}"
    );
    assert!(
        log.contains("--profile redis --profile worker pull"),
        "{log}"
    );
    assert!(!log.contains("run --rm --no-build"), "{log}");
    assert!(!log.contains("theme:compile"), "{log}");
    assert!(!log.contains("assets:install"), "{log}");
    assert!(
        log.contains("Rollback finished ghcr.io/example/acme:abc123"),
        "{log}"
    );
    assert!(!shop.path().join("trap-invoked").is_file());
    let prod = fs::read_to_string(shop.path().join(".env.prod")).unwrap();
    assert!(prod.contains("COMPOSE_PROFILES=redis,worker"), "{prod}");
}

#[test]
fn skip_pull_dry_run() {
    let shop = TempShop::new("skip-pull");
    shop.write_min_shop();
    shop.write_previous("old-sha\n");
    let trap = shop.install_docker_trap();
    let out = rollback_with_path(shop.path(), &["--dry-run", "--skip-pull"], &[], Some(&trap));
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let log = stdout(&out);
    assert!(log.contains("DRY-RUN skip compose pull"), "{log}");
    assert!(
        log.contains("up -d --no-build --pull never --remove-orphans web"),
        "{log}"
    );
    assert!(
        !log.lines()
            .any(|l| l.contains("DRY-RUN") && l.trim_end().ends_with(" pull")),
        "skip-pull must not print a compose pull command:\n{log}"
    );
    assert!(
        log.contains("--profile setup run --rm --pull never setup"),
        "{log}"
    );
    assert!(!shop.path().join("trap-invoked").is_file());
    assert!(!shop.path().join(".deployed-tag").is_file());
}

#[test]
fn skip_pull_via_env() {
    let shop = TempShop::new("skip-env");
    shop.write_min_shop();
    shop.write_previous("t1\n");
    let out = rollback(shop.path(), &["--dry-run"], &[("SKIP_PULL", "true")]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    assert!(
        stdout(&out).contains("DRY-RUN skip compose pull"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn smoke_url_dry_run_does_not_curl() {
    let shop = TempShop::new("smoke");
    shop.write_min_shop();
    shop.write_previous("t1\n");
    let trap = shop.install_docker_trap();
    let out = rollback_with_path(
        shop.path(),
        &["--dry-run"],
        &[("SMOKE_URL", "http://127.0.0.1:9/")],
        Some(&trap),
    );
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr(&out));
    let log = stdout(&out);
    assert!(
        log.contains("DRY-RUN would GET http://127.0.0.1:9/"),
        "{log}"
    );
    assert!(
        log.contains("DRY-RUN would write .deployed-tag=t1"),
        "{log}"
    );
    assert!(!shop.path().join("trap-invoked").is_file());
}

#[test]
fn setup_in_profiles_refused() {
    let shop = TempShop::new("setup-prof");
    shop.write_min_shop();
    shop.write_previous("t1\n");
    let out = rollback(
        shop.path(),
        &["--dry-run"],
        &[("COMPOSE_PROFILES", "redis,setup")],
    );
    assert_eq!(out.status.code(), Some(1), "stderr={}", stderr(&out));
    assert!(stderr(&out).contains("setup"), "{}", stderr(&out));
}
