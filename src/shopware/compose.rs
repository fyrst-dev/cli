//! VPS Compose file list and argv (recipes `deploy/lib/compose.sh`).
//!
//! Always invoked from shop-root `COMPOSE_DIR`. Source of truth for the
//! project name is `deploy/compose.yaml`:
//! `name: "${SHOPWARE_SHOP_ID:?…}-${SHOPWARE_DEPLOY_ENV:?…}"`.
//! That interpolates only if host env is loaded, so argv passes `--env-file`
//! in identity-load order (always `.env`, then `.env.local` / `.env.prod` when
//! those files exist). `-p <shop-id>-<env>` is a matching pin (defense in
//! depth), not the only naming mechanism.
//!
//! Example when both host files exist:
//! `docker compose --env-file .env --env-file .env.local --env-file .env.prod -f deploy/compose.yaml -f deploy/compose.prod.yaml -f deploy/compose.vps.yaml -p <shop-id>-<env>`
//!
//! Never builds images. `compose up` uses `--no-build`. `compose run` uses
//! `--pull never` (Compose v5 dropped `--no-build` on `run`).

use super::env::{COMPOSE_FILES, ENV_FILES};
use super::error::Error;
use super::mysql::require_docker;
use std::path::Path;
use std::process::{Command, Stdio};

/// `--env-file` flags: always `.env`, then `.env.local` / `.env.prod` if present.
pub fn vps_env_file_flags(compose_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for rel in ENV_FILES {
        if *rel != ".env" && !compose_dir.join(rel).is_file() {
            continue;
        }
        out.push("--env-file".into());
        out.push((*rel).to_string());
    }
    out
}

/// The three overlay compose files, host env files, plus `-p <project>`.
///
/// `project` is `{SHOPWARE_SHOP_ID}-{SHOPWARE_DEPLOY_ENV}` and must match
/// compose `name:` after interpolation. All overlay files must exist for VPS
/// release/rollback.
pub fn vps_compose_argv(compose_dir: &Path, project: &str) -> Vec<String> {
    let mut a = vec!["compose".into()];
    a.extend(vps_env_file_flags(compose_dir));
    for f in COMPOSE_FILES {
        a.push("-f".into());
        a.push((*f).to_string());
    }
    a.push("-p".into());
    a.push(project.to_string());
    a
}

pub fn docker_log(compose_args: &[String]) -> String {
    let mut s = String::from("docker");
    for a in compose_args {
        s.push(' ');
        s.push_str(a);
    }
    s
}

pub fn require_vps_compose_files(compose_dir: &Path) -> Result<(), Error> {
    for rel in COMPOSE_FILES {
        if !compose_dir.join(rel).is_file() {
            return Err(Error::fail(format!(
                "Missing {rel} (expected under shop root COMPOSE_DIR={})",
                compose_dir.display()
            )));
        }
    }
    Ok(())
}

pub fn extend_profiles(args: &mut Vec<String>, profiles: &[String]) {
    for p in profiles {
        args.push("--profile".into());
        args.push(p.clone());
    }
}

pub fn args_contain_build(args: &[String]) -> bool {
    args.iter().any(|a| a == "--build")
}

pub fn run_compose(
    compose_dir: &Path,
    args: &[String],
    extra_env: &[(String, String)],
) -> Result<(), Error> {
    if args_contain_build(args) {
        return Err(Error::fail(
            "internal error: compose argv included --build (VPS paths never build images)",
        ));
    }
    require_docker()?;
    let mut cmd = Command::new("docker");
    cmd.args(args).current_dir(compose_dir);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let status = cmd
        .status()
        .map_err(|e| Error::fail(format!("failed to exec docker compose: {e}")))?;
    if !status.success() {
        return Err(Error::fail(format!(
            "docker compose failed: {}",
            docker_log(args)
        )));
    }
    Ok(())
}

/// Overlay `vps_has_service`: `compose config --services`, stderr ignored.
pub fn compose_service_names(
    compose_dir: &Path,
    profile_args: &[String],
    extra_env: &[(String, String)],
    project: &str,
) -> Vec<String> {
    let mut args = vps_compose_argv(compose_dir, project);
    args.extend(profile_args.iter().cloned());
    args.extend(["config".into(), "--services".into()]);
    let mut cmd = Command::new("docker");
    cmd.args(&args)
        .current_dir(compose_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    match cmd.output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempShop(PathBuf);

    impl TempShop {
        fn new(prefix: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir().join(format!(
                "fyrst-cli-compose-{prefix}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }

    impl Drop for TempShop {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn env_files(argv: &[String]) -> Vec<&str> {
        argv.windows(2)
            .filter(|w| w[0] == "--env-file")
            .map(|w| w[1].as_str())
            .collect()
    }

    #[test]
    fn vps_argv_is_the_three_overlay_files() {
        let shop = TempShop::new("base");
        let a = vps_compose_argv(&shop.0, "acme-staging");
        assert_eq!(
            a,
            vec![
                "compose",
                "--env-file",
                ".env",
                "-f",
                "deploy/compose.yaml",
                "-f",
                "deploy/compose.prod.yaml",
                "-f",
                "deploy/compose.vps.yaml",
                "-p",
                "acme-staging",
            ]
        );
        let log = docker_log(&a);
        assert_eq!(
            log,
            "docker compose --env-file .env -f deploy/compose.yaml -f deploy/compose.prod.yaml -f deploy/compose.vps.yaml -p acme-staging"
        );
        assert!(!args_contain_build(&a));
        assert_eq!(env_files(&a), [".env"]);
    }

    #[test]
    fn vps_argv_pins_identity_name() {
        let shop = TempShop::new("pin");
        let a = vps_compose_argv(&shop.0, "acme-live");
        assert!(a.windows(2).any(|w| w == ["-p", "acme-live"]), "{a:?}");
    }

    #[test]
    fn vps_argv_includes_host_env_files_when_present() {
        let shop = TempShop::new("host-env");
        fs::write(shop.0.join(".env.local"), "SHOPWARE_DEPLOY_ENV=dev\n").unwrap();
        fs::write(shop.0.join(".env.prod"), "SHOPWARE_DEPLOY_ENV=live\n").unwrap();
        let a = vps_compose_argv(&shop.0, "acme-dev");
        assert_eq!(env_files(&a), [".env", ".env.local", ".env.prod"]);
        assert!(
            a.windows(2).any(|w| w == ["-p", "acme-dev"]),
            "matching pin missing: {a:?}"
        );
        let log = docker_log(&a);
        assert!(
            log.contains(
                "docker compose --env-file .env --env-file .env.local --env-file .env.prod -f deploy/compose.yaml"
            ),
            "{log}"
        );
        assert!(log.contains("-p acme-dev"), "{log}");
        let env_idx: Vec<usize> = a
            .windows(2)
            .enumerate()
            .filter(|(_, w)| w[0] == "--env-file")
            .map(|(i, _)| i)
            .collect();
        let dash_f = a.iter().position(|s| s == "-f").unwrap();
        let dash_p = a.iter().position(|s| s == "-p").unwrap();
        assert!(env_idx.iter().all(|i| *i < dash_f), "{a:?}");
        assert!(dash_f < dash_p, "{a:?}");
    }

    #[test]
    fn vps_argv_omits_missing_host_env_files() {
        let shop = TempShop::new("prod-only");
        fs::write(shop.0.join(".env.prod"), "").unwrap();
        let a = vps_compose_argv(&shop.0, "acme-staging");
        assert_eq!(env_files(&a), [".env", ".env.prod"]);
        assert!(!a.iter().any(|s| s == ".env.local"), "{a:?}");
        assert!(a.windows(2).any(|w| w == ["-p", "acme-staging"]), "{a:?}");
    }
}
