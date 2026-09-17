//! App container pause/resume and post-restore hints (recipes `lib/sync-app.sh`).

use super::env::{vps_project_name_opt, ShopEnv};
use super::error::Error;
use super::mysql::{compose_argv, require_docker};
use std::path::Path;
use std::process::{Command, Stdio};

const APP_SERVICES: &[&str] = &["web", "worker", "scheduler"];

pub fn stop_app_containers(
    compose_dir: &Path,
    files: &[String],
    project: Option<&str>,
    dry_run: bool,
) -> Result<Vec<String>, Error> {
    if dry_run {
        println!("==> DRY-RUN would stop web/worker/scheduler if running");
        return Ok(Vec::new());
    }
    if files.is_empty() {
        return Ok(Vec::new());
    }
    require_docker()?;
    let mut args = compose_argv(files, project);
    args.extend([
        "--profile".into(),
        "worker".into(),
        "--profile".into(),
        "scheduler".into(),
        "ps".into(),
        "--status".into(),
        "running".into(),
        "--format".into(),
        "{{.Service}}".into(),
    ]);
    let out = Command::new("docker")
        .args(&args)
        .current_dir(compose_dir)
        .output();
    let running = match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    let mut stopped = Vec::new();
    for svc in APP_SERVICES {
        if !running.iter().any(|r| r == svc) {
            continue;
        }
        println!("==> Stopping {svc} for restore");
        if !compose_stop(compose_dir, files, project, svc) {
            let mut with_profile = compose_argv(files, project);
            with_profile.extend([
                "--profile".into(),
                (*svc).into(),
                "stop".into(),
                (*svc).into(),
            ]);
            let _ = Command::new("docker")
                .args(&with_profile)
                .current_dir(compose_dir)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        stopped.push((*svc).to_string());
    }
    Ok(stopped)
}

fn compose_stop(compose_dir: &Path, files: &[String], project: Option<&str>, svc: &str) -> bool {
    let mut args = compose_argv(files, project);
    args.extend(["stop".into(), svc.into()]);
    Command::new("docker")
        .args(&args)
        .current_dir(compose_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn start_stopped_app(
    compose_dir: &Path,
    files: &[String],
    project: Option<&str>,
    stopped: &[String],
    dry_run: bool,
) -> Result<(), Error> {
    if stopped.is_empty() {
        return Ok(());
    }
    for svc in stopped {
        if dry_run {
            println!("==> DRY-RUN would start {svc}");
            continue;
        }
        println!("==> Starting {svc}");
        let mut args = compose_argv(files, project);
        match svc.as_str() {
            "worker" => {
                args.extend([
                    "--profile".into(),
                    "worker".into(),
                    "up".into(),
                    "-d".into(),
                    "--no-build".into(),
                    "worker".into(),
                ]);
                let _ = Command::new("docker")
                    .args(&args)
                    .current_dir(compose_dir)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            "scheduler" => {
                args.extend([
                    "--profile".into(),
                    "scheduler".into(),
                    "up".into(),
                    "-d".into(),
                    "--no-build".into(),
                    "scheduler".into(),
                ]);
                let _ = Command::new("docker")
                    .args(&args)
                    .current_dir(compose_dir)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            _ => {
                args.extend(["up".into(), "-d".into(), "--no-build".into(), "web".into()]);
                let status = Command::new("docker")
                    .args(&args)
                    .current_dir(compose_dir)
                    .status()
                    .map_err(|e| Error::fail(format!("could not exec docker compose: {e}")))?;
                if !status.success() {
                    return Err(Error::fail("docker compose up web failed after sync"));
                }
            }
        }
    }
    Ok(())
}

pub fn post_restore_hints(
    env: &ShopEnv,
    rewrite_requested: bool,
    dry_run: bool,
    compose_dir: &Path,
    files: &[String],
) {
    if rewrite_requested {
        println!(
            "==> Sales-channel domains: rewrite used APP_URL. Payment/shipping webhooks may still need manual review."
        );
    } else {
        println!(
            "==> Sales-channel domains were not rewritten. Set APP_URL on a non-live consumer to rewrite sales_channel_domain after restore."
        );
        if let Some(target) = env.get("APP_URL") {
            println!(
                "==> This shop APP_URL={target} — destination storefront URL if you rewrite manually."
            );
        }
    }
    if dry_run {
        println!("==> DRY-RUN would try cache:clear (non-fatal)");
        return;
    }
    if env.get("IMAGE").is_some() && !files.is_empty() {
        println!("==> Trying cache:clear (non-fatal if the image/console is unavailable)");
        let project = vps_project_name_opt(env);
        let mut args = compose_argv(files, project.as_deref());
        args.extend([
            "run".into(),
            "--rm".into(),
            "--pull".into(),
            "never".into(),
            "--entrypoint".into(),
            "php".into(),
            "web".into(),
            "bin/console".into(),
            "cache:clear".into(),
        ]);
        if let Ok(status) = Command::new("docker")
            .args(&args)
            .current_dir(compose_dir)
            .status()
        {
            if !status.success() {
                println!("==> cache:clear skipped or failed — not fatal");
            }
        } else {
            println!("==> cache:clear skipped or failed — not fatal");
        }
    }
}
