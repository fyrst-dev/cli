//! VPS Compose file list and argv (recipes `deploy/lib/compose.sh`).
//!
//! Always invoked from shop-root `COMPOSE_DIR`:
//! `docker compose --env-file .env -f deploy/compose.yaml -f deploy/compose.prod.yaml -f deploy/compose.vps.yaml -p <shop-id>-<env>`
//!
//! `-p` pins `{SHOPWARE_SHOP_ID}-{SHOPWARE_DEPLOY_ENV}` so leftover
//! `COMPOSE_PROJECT_NAME` in committed `.env` cannot collapse live/staging.
//!
//! Never builds images. `compose up` uses `--no-build`. `compose run` uses
//! `--pull never` (Compose v5 dropped `--no-build` on `run`).

use super::env::COMPOSE_FILES;
use super::error::Error;
use super::mysql::require_docker;
use std::path::Path;
use std::process::{Command, Stdio};

/// The three overlay compose files plus `-p <project>`; all files must exist
/// for VPS release/rollback. `project` is `{SHOPWARE_SHOP_ID}-{SHOPWARE_DEPLOY_ENV}`.
pub fn vps_compose_argv(project: &str) -> Vec<String> {
    let files: Vec<String> = COMPOSE_FILES.iter().map(|s| (*s).to_string()).collect();
    super::mysql::compose_argv(&files, Some(project))
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
    let mut args = vps_compose_argv(project);
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

    #[test]
    fn vps_argv_is_the_three_overlay_files() {
        let a = vps_compose_argv("acme-staging");
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
    }

    #[test]
    fn vps_argv_pins_identity_name() {
        let a = vps_compose_argv("acme-live");
        assert!(a.windows(2).any(|w| w == ["-p", "acme-live"]), "{a:?}");
    }
}
