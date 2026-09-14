//! `fyrst-cli shopware sync apply` — DB import plus bind-mount volumes and
//! post-apply orchestration (stop/start, opt-in rewrite, cache:clear hint,
//! `SYNC_POST_RESTORE_CMD`).
//!
//! Database import is the same module as `shopware db import`. Rewrite is
//! `bin/console fyrst:sales-channel:rewrite-urls` via compose `web`, not SQL.

use super::data::{normalize_data, DataSelection};
use super::env::{
    existing_compose_files, require_shop_id, resolve_compose_dir, resolve_data_root,
    resolve_snapshot_dir, ShopEnv,
};
use super::error::Error;
use super::import::{self, ImportPlan};
use super::live::{assert_not_live, LivePolicy, LiveSignals};
use super::mysql::{
    collect_env_secrets, compose_argv, find_snapshot_dump, log_contains_secret, parse_database_url,
};
use super::rewrite::{
    assert_not_live_rewrite, compose_rewrite_args, rewrite_log_line, rewrite_requested,
};
use super::volumes::{archive_image, execute_bind_restore, plan_bind_restore, VolumeRestore};
use crate::cli::SyncOpArgs;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RewriteAction {
    None,
    SkipNoDb,
    Run {
        docker_args: Vec<String>,
        log_line: String,
    },
}

pub struct RestorePlan {
    pub compose_dir: PathBuf,
    pub compose_files: Vec<String>,
    pub shop_id: String,
    pub deploy_env: Option<String>,
    pub sync_env: Option<String>,
    pub snapshot_dir: PathBuf,
    pub data_root: Option<PathBuf>,
    pub selection: DataSelection,
    pub dry_run: bool,
    pub live_warning: Option<String>,
    pub import: Option<ImportPlan>,
    pub rewrite: RewriteAction,
    pub rewrite_requested: bool,
    pub volumes: Vec<VolumeRestore>,
    pub archive_image: String,
    pub image: Option<String>,
    pub app_url: Option<String>,
    post_restore_cmd: Option<String>,
    secrets: Vec<String>,
}

impl fmt::Debug for RestorePlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RestorePlan")
            .field("compose_dir", &self.compose_dir)
            .field("shop_id", &self.shop_id)
            .field("deploy_env", &self.deploy_env)
            .field("snapshot_dir", &self.snapshot_dir)
            .field("data_root", &self.data_root)
            .field("selection", &self.selection)
            .field("dry_run", &self.dry_run)
            .field("rewrite", &self.rewrite)
            .field("volumes", &self.volumes)
            .field("has_post_restore_cmd", &self.post_restore_cmd.is_some())
            .finish_non_exhaustive()
    }
}

impl RestorePlan {
    pub fn log_contains_secret(&self, text: &str) -> bool {
        log_contains_secret(text, &self.secrets)
    }
}

pub fn run(args: SyncOpArgs) -> Result<(), Error> {
    let process_env: HashMap<String, String> = std::env::vars().collect();
    let cwd = std::env::current_dir().map_err(|e| Error::fail(format!("cannot read cwd: {e}")))?;
    let plan = plan(&args, &process_env, &cwd)?;
    execute(&plan)
}

pub fn plan(
    args: &SyncOpArgs,
    process_env: &HashMap<String, String>,
    cwd: &Path,
) -> Result<RestorePlan, Error> {
    let compose_dir = resolve_compose_dir(process_env, cwd)?;
    let compose_dir = fs::canonicalize(&compose_dir).unwrap_or(compose_dir);
    let env = ShopEnv::load(compose_dir.clone(), process_env)?;
    let selection = normalize_data(args.data.as_deref(), args.skip_db, args.skip_volumes)?;
    let snapshot_dir = resolve_snapshot_dir(args.snapshot_dir.as_deref(), &env, &compose_dir);
    plan_with_env(&env, cwd, &snapshot_dir, &selection, args.dry_run)
}

/// Inner apply used by `shopware sync apply` and `shopware backup recover`.
pub fn apply_from_env(
    env: &ShopEnv,
    cwd: &Path,
    snapshot_dir: &Path,
    selection: &DataSelection,
    dry_run: bool,
) -> Result<(), Error> {
    let plan = plan_with_env(env, cwd, snapshot_dir, selection, dry_run)?;
    execute(&plan)
}

pub fn plan_with_env(
    env: &ShopEnv,
    cwd: &Path,
    snapshot_dir: &Path,
    selection: &DataSelection,
    dry_run: bool,
) -> Result<RestorePlan, Error> {
    let compose_dir = env.compose_dir.clone();
    let shop_id = require_shop_id(env)?;
    let snapshot_dir = snapshot_dir.to_path_buf();
    let selection = selection.clone();

    let signals = LiveSignals::from_shop(env);
    let live_warning = assert_not_live(&signals, LivePolicy::SyncRestore)?;
    let rewrite_opt_in = rewrite_requested(env);
    if rewrite_opt_in {
        assert_not_live_rewrite(&signals)?;
    }

    if !dry_run && !snapshot_dir.is_dir() {
        return Err(Error::fail(format!(
            "Snapshot directory not found: {}",
            snapshot_dir.display()
        )));
    }

    let data_root = if !selection.volumes.is_empty() {
        Some(resolve_data_root(env)?)
    } else {
        resolve_data_root(env).ok()
    };

    let import = if selection.want_db {
        let file = find_snapshot_dump(&snapshot_dir)?;
        Some(import::plan_with_env(
            env,
            cwd,
            &file,
            dry_run,
            LivePolicy::SyncRestore,
        )?)
    } else {
        None
    };

    let rewrite = if !rewrite_opt_in {
        RewriteAction::None
    } else if !selection.want_db {
        RewriteAction::SkipNoDb
    } else {
        RewriteAction::Run {
            docker_args: compose_rewrite_args(env, &compose_dir, dry_run),
            log_line: rewrite_log_line(env, &compose_dir, dry_run),
        }
    };

    let mut volumes = Vec::new();
    if let Some(root) = data_root.as_ref() {
        for item in &selection.volumes {
            volumes.push(plan_bind_restore(&snapshot_dir, root, item)?);
        }
    }

    let parsed_url = env.get("DATABASE_URL").and_then(parse_database_url);
    let secrets = collect_env_secrets(env, parsed_url.as_ref());
    let compose_files = existing_compose_files(&compose_dir);

    Ok(RestorePlan {
        compose_dir,
        compose_files,
        shop_id,
        deploy_env: env.get("SHOPWARE_DEPLOY_ENV").map(str::to_string),
        sync_env: env.get("SYNC_ENV").map(str::to_string),
        snapshot_dir,
        data_root,
        selection,
        dry_run,
        live_warning: if import.is_none() { live_warning } else { None },
        import,
        rewrite,
        rewrite_requested: rewrite_opt_in,
        volumes,
        archive_image: archive_image(env),
        image: env.get("IMAGE").map(str::to_string),
        app_url: env
            .get("SYNC_APP_URL")
            .or_else(|| env.get("APP_URL"))
            .map(str::to_string),
        post_restore_cmd: env.get("SYNC_POST_RESTORE_CMD").map(str::to_string),
        secrets,
    })
}

pub fn execute(plan: &RestorePlan) -> Result<(), Error> {
    if let Some(warn) = &plan.live_warning {
        eprintln!("{warn}");
    }

    println!(
        "==> Runtime data restore  data={}  shop={}  deploy_env={}  snapshot_dir={}  compose_dir={}  data_root={}  dry-run={}",
        plan.selection.items_csv(),
        plan.shop_id,
        plan.deploy_env.as_deref().unwrap_or("unset"),
        plan.snapshot_dir.display(),
        plan.compose_dir.display(),
        plan.data_root
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "unset".into()),
        if plan.dry_run { 1 } else { 0 },
    );

    let stopped = stop_app_containers(&plan.compose_dir, &plan.compose_files, plan.dry_run)?;

    if let Some(imp) = &plan.import {
        import::execute(imp)?;
    }

    match &plan.rewrite {
        RewriteAction::None => {}
        RewriteAction::SkipNoDb => {
            println!(
                "==> SYNC_REWRITE_APP_URL / SYNC_REWRITE_URL_MAP set but db was skipped — not rewriting sales_channel_domain"
            );
        }
        RewriteAction::Run {
            docker_args,
            log_line,
        } => {
            println!(
                "==> Opt-in sales_channel_domain rewrite via fyrst:sales-channel:rewrite-urls (sales channel domains only; media CDN / plugin configs / payment webhooks are not updated)"
            );
            if plan.log_contains_secret(log_line) {
                return Err(Error::fail(
                    "internal error: rewrite log would have included a password",
                ));
            }
            if plan.dry_run {
                println!("==> DRY-RUN {log_line}");
            } else {
                run_rewrite(&plan.compose_dir, docker_args)?;
            }
        }
    }

    for vol in &plan.volumes {
        let line = vol.restore_log_line();
        if plan.log_contains_secret(&line) || plan.log_contains_secret(&vol.dry_run_line()) {
            return Err(Error::fail(
                "internal error: volume restore log would have included a password",
            ));
        }
        execute_bind_restore(vol, &plan.archive_image, plan.dry_run)?;
    }

    start_stopped_app(
        &plan.compose_dir,
        &plan.compose_files,
        &stopped,
        plan.dry_run,
    )?;
    post_restore_hints(plan);
    println!(
        "==> Restore finished into {} data_root={} (SYNC_ENV={})",
        plan.compose_dir.display(),
        plan.data_root
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "unset".into()),
        plan.sync_env.as_deref().unwrap_or("unset"),
    );
    Ok(())
}

fn run_rewrite(compose_dir: &Path, docker_args: &[String]) -> Result<(), Error> {
    let status = Command::new("docker")
        .args(docker_args)
        .current_dir(compose_dir)
        .status()
        .map_err(|e| Error::fail(format!("failed to exec docker: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::fail(
            "fyrst:sales-channel:rewrite-urls failed. composer update fyrst/shopware-cd so the command and FyrstShopwareCdBundle exist, then composer recipes:update fyrst/shopware-cd.",
        ))
    }
}

fn stop_app_containers(
    compose_dir: &Path,
    files: &[String],
    dry_run: bool,
) -> Result<Vec<String>, Error> {
    if dry_run {
        println!("==> DRY-RUN would stop web/worker/scheduler if running");
        return Ok(Vec::new());
    }
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let running = list_running_app_services(compose_dir, files);
    let mut stopped = Vec::new();
    for svc in ["web", "worker", "scheduler"] {
        if running.iter().any(|s| s == svc) {
            println!("==> Stopping {svc} for restore");
            compose_stop(compose_dir, files, svc);
            stopped.push(svc.to_string());
        }
    }
    Ok(stopped)
}

fn list_running_app_services(compose_dir: &Path, files: &[String]) -> Vec<String> {
    let mut args = compose_argv(files);
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
    let out = match out {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn compose_stop(compose_dir: &Path, files: &[String], svc: &str) {
    let mut args = compose_argv(files);
    args.extend(["stop".into(), svc.to_string()]);
    let ok = Command::new("docker")
        .args(&args)
        .current_dir(compose_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        return;
    }
    let mut args = compose_argv(files);
    args.extend([
        "--profile".into(),
        svc.to_string(),
        "stop".into(),
        svc.to_string(),
    ]);
    let _ = Command::new("docker")
        .args(&args)
        .current_dir(compose_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn start_stopped_app(
    compose_dir: &Path,
    files: &[String],
    stopped: &[String],
    dry_run: bool,
) -> Result<(), Error> {
    for svc in stopped {
        if dry_run {
            println!("==> DRY-RUN would start {svc}");
            continue;
        }
        println!("==> Starting {svc}");
        match svc.as_str() {
            "web" => compose_up(compose_dir, files, None, "web")?,
            "worker" => {
                let _ = compose_up(compose_dir, files, Some("worker"), "worker");
            }
            "scheduler" => {
                let _ = compose_up(compose_dir, files, Some("scheduler"), "scheduler");
            }
            _ => {}
        }
    }
    Ok(())
}

fn compose_up(
    compose_dir: &Path,
    files: &[String],
    profile: Option<&str>,
    svc: &str,
) -> Result<(), Error> {
    let mut args = compose_argv(files);
    if let Some(p) = profile {
        args.extend(["--profile".into(), p.to_string()]);
    }
    args.extend([
        "up".into(),
        "-d".into(),
        "--no-build".into(),
        svc.to_string(),
    ]);
    let status = Command::new("docker")
        .args(&args)
        .current_dir(compose_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| Error::fail(format!("could not exec docker compose: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::fail(format!("docker compose up {svc} failed")))
    }
}

fn post_restore_hints(plan: &RestorePlan) {
    if plan.rewrite_requested {
        println!(
            "==> Sales-channel domains: opt-in rewrite was requested (see SYNC_REWRITE_*). Payment/shipping webhooks may still need manual review."
        );
    } else {
        println!(
            "==> Sales-channel domains were not rewritten (default). Set SYNC_REWRITE_APP_URL=https://staging.example.com (or SYNC_REWRITE_URL_MAP) on a non-live consumer to rewrite sales_channel_domain after restore."
        );
        if let Some(target) = &plan.app_url {
            if !plan.log_contains_secret(target) {
                println!(
                    "==> This shop APP_URL / SYNC_APP_URL={target} — destination storefront URL if you rewrite manually."
                );
            }
        }
    }
    if plan.dry_run {
        println!(
            "==> DRY-RUN would try cache:clear (non-fatal) and optional SYNC_POST_RESTORE_CMD"
        );
        return;
    }
    if plan.image.is_some() {
        println!("==> Trying cache:clear (non-fatal if the image/console is unavailable)");
        if cache_clear(&plan.compose_dir, &plan.compose_files).is_err() {
            println!("==> cache:clear skipped or failed — not fatal");
        }
    }
    if let Some(cmd) = &plan.post_restore_cmd {
        println!("==> Running SYNC_POST_RESTORE_CMD (non-fatal)");
        if !run_post_restore_cmd(cmd) {
            println!("==> SYNC_POST_RESTORE_CMD failed — not fatal");
        }
    }
}

fn cache_clear(compose_dir: &Path, files: &[String]) -> Result<(), Error> {
    let mut args = compose_argv(files);
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
    let status = Command::new("docker")
        .args(&args)
        .current_dir(compose_dir)
        .status()
        .map_err(|e| Error::fail(format!("failed to exec docker: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::fail("cache:clear failed"))
    }
}

fn run_post_restore_cmd(cmd: &str) -> bool {
    Command::new("bash")
        .args(["-lc", cmd])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempShop(PathBuf);
    impl TempShop {
        fn new(prefix: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir().join(format!(
                "fyrst-cli-rst-{prefix}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(p.join("deploy")).unwrap();
            fs::create_dir_all(p.join("var/runtime-sync")).unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
        fn write_env(&self, body: &str) {
            fs::write(self.0.join(".env"), body).unwrap();
        }
        fn write_compose_mysql(&self) {
            fs::write(
                self.0.join("deploy/compose.yaml"),
                "services:\n  mysql:\n    image: mysql:8.4\n  web:\n    image: example\n",
            )
            .unwrap();
        }
        fn write_media_snapshot(&self) {
            let d = self.0.join("var/runtime-sync/data/media");
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("hello.txt"), b"hi").unwrap();
        }
        fn write_dump(&self) {
            fs::write(self.0.join("var/runtime-sync/db.sql"), b"SELECT 1;\n").unwrap();
        }
    }
    impl Drop for TempShop {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn process(shop: &Path) -> HashMap<String, String> {
        let mut p = HashMap::new();
        p.insert("COMPOSE_DIR".into(), shop.to_string_lossy().into_owned());
        p
    }

    fn args(data: &str, dry: bool, skip_db: bool) -> SyncOpArgs {
        SyncOpArgs {
            from: None,
            data: Some(data.into()),
            snapshot_dir: None,
            dry_run: dry,
            skip_db,
            skip_volumes: false,
        }
    }

    #[test]
    fn restore_db_plans_import() {
        let shop = TempShop::new("ok");
        shop.write_env(
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=staging\nMYSQL_USER=u\nMYSQL_PASSWORD=s3cret-value\n",
        );
        shop.write_compose_mysql();
        shop.write_dump();
        let env = ShopEnv::load(shop.path().to_path_buf(), &process(shop.path())).unwrap();
        let file = find_snapshot_dump(&shop.path().join("var/runtime-sync")).unwrap();
        let plan =
            import::plan_with_env(&env, shop.path(), &file, true, LivePolicy::SyncRestore).unwrap();
        let log = plan.log_line();
        assert!(!log.contains("s3cret-value"), "{log}");
        assert!(log.contains("exec -T mysql"), "{log}");
    }

    #[test]
    fn restore_live_refuses_without_env() {
        let shop = TempShop::new("live");
        shop.write_env(
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=live\nMYSQL_USER=u\nMYSQL_PASSWORD=p\n",
        );
        shop.write_compose_mysql();
        shop.write_dump();
        let env = ShopEnv::load(shop.path().to_path_buf(), &process(shop.path())).unwrap();
        let file = find_snapshot_dump(&shop.path().join("var/runtime-sync")).unwrap();
        let err = import::plan_with_env(&env, shop.path(), &file, true, LivePolicy::SyncRestore)
            .unwrap_err();
        assert!(err.to_string().contains("live"), "{err}");
        assert!(err.to_string().contains("SYNC_ALLOW_LIVE_RESTORE"), "{err}");
    }

    #[test]
    fn volume_dry_run_plans_rsync() {
        let shop = TempShop::new("vol");
        shop.write_env(
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=staging\nMYSQL_PASSWORD=s3cret-value\nSHOPWARE_DATA_ROOT=/tmp/fyrst-data-acme\n",
        );
        shop.write_compose_mysql();
        shop.write_media_snapshot();
        let plan = plan(
            &args("media", true, false),
            &process(shop.path()),
            shop.path(),
        )
        .unwrap();
        assert!(!plan.selection.want_db);
        assert_eq!(plan.volumes.len(), 1);
        assert!(plan.volumes[0].dry_run_line().contains("rsync"));
        assert!(plan.volumes[0].dry_run_line().contains("chown 82:82"));
        assert!(plan.volumes[0]
            .dry_run_line()
            .contains("/tmp/fyrst-data-acme/media"));
        assert!(matches!(plan.rewrite, RewriteAction::None));
        assert!(plan.import.is_none());
    }

    #[test]
    fn volume_only_live_refuses() {
        let shop = TempShop::new("vollive");
        shop.write_env(
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=live\nSHOPWARE_DATA_ROOT=/tmp/data\nMYSQL_PASSWORD=p\n",
        );
        shop.write_compose_mysql();
        shop.write_media_snapshot();
        let err = plan(
            &args("media", true, false),
            &process(shop.path()),
            shop.path(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("live"), "{err}");
        assert!(err.to_string().contains("SYNC_ALLOW_LIVE_RESTORE"), "{err}");
    }

    #[test]
    fn rewrite_skipped_when_skip_db() {
        let shop = TempShop::new("rewskip");
        shop.write_env(
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=staging\nSHOPWARE_DATA_ROOT=/tmp/data\nSYNC_REWRITE_APP_URL=https://staging.example.com\nMYSQL_PASSWORD=p\n",
        );
        shop.write_compose_mysql();
        shop.write_media_snapshot();
        let plan = plan(
            &args("media", true, false),
            &process(shop.path()),
            shop.path(),
        )
        .unwrap();
        assert!(matches!(plan.rewrite, RewriteAction::SkipNoDb));
        assert!(plan.rewrite_requested);
        assert!(plan.import.is_none());
    }

    #[test]
    fn rewrite_refused_on_live_even_with_allow() {
        let shop = TempShop::new("rewlive");
        shop.write_env(
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=live\nSHOPWARE_DATA_ROOT=/tmp/data\nSYNC_ALLOW_LIVE_RESTORE=1\nSYNC_REWRITE_APP_URL=https://staging.example.com\nMYSQL_PASSWORD=p\n",
        );
        shop.write_compose_mysql();
        shop.write_media_snapshot();
        shop.write_dump();
        let err = plan(&args("db", true, false), &process(shop.path()), shop.path()).unwrap_err();
        assert!(
            err.to_string()
                .contains("Refusing sales-channel domain rewrite on a live host"),
            "{err}"
        );
        assert!(
            err.to_string()
                .contains("SYNC_ALLOW_LIVE_RESTORE=1 does not bypass"),
            "{err}"
        );
    }

    #[test]
    fn rewrite_run_is_compose_console_when_db_restored() {
        let shop = TempShop::new("rewrun");
        shop.write_env(
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=staging\nSYNC_ENV=staging\nSYNC_REWRITE_APP_URL=https://staging.example.com\nMYSQL_USER=u\nMYSQL_PASSWORD=s3cret-value\n",
        );
        shop.write_compose_mysql();
        shop.write_dump();
        let plan = plan(&args("db", true, false), &process(shop.path()), shop.path()).unwrap();
        match &plan.rewrite {
            RewriteAction::Run {
                log_line,
                docker_args,
            } => {
                assert!(
                    log_line.contains("fyrst:sales-channel:rewrite-urls"),
                    "{log_line}"
                );
                assert!(
                    log_line.contains("run --rm --pull never --entrypoint php"),
                    "{log_line}"
                );
                assert!(log_line.contains("--dry-run"), "{log_line}");
                assert!(!log_line.contains("s3cret-value"), "{log_line}");
                assert!(docker_args.contains(&"fyrst:sales-channel:rewrite-urls".to_string()));
                assert!(!log_line
                    .to_ascii_lowercase()
                    .contains("update sales_channel"));
            }
            other => panic!("{other:?}"),
        }
        assert!(plan.import.is_some());
        assert!(plan.volumes.is_empty());
    }

    #[test]
    fn mixed_data_plans_import_and_volumes() {
        let shop = TempShop::new("mix");
        shop.write_env(
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=staging\nSHOPWARE_DATA_ROOT=/tmp/data\nMYSQL_USER=u\nMYSQL_PASSWORD=s3cret-value\n",
        );
        shop.write_compose_mysql();
        shop.write_dump();
        shop.write_media_snapshot();
        let plan = plan(
            &args("db,media", true, false),
            &process(shop.path()),
            shop.path(),
        )
        .unwrap();
        assert!(plan.import.is_some());
        assert_eq!(plan.volumes.len(), 1);
        assert_eq!(plan.volumes[0].item, "media");
        let log = plan.import.as_ref().unwrap().log_line();
        assert!(!log.contains("s3cret-value"), "{log}");
    }
}
