//! `fyrst-cli shopware sync pull` — pull orchestration, no dump wrap.
//!
//! Matches overlay `do_sync` minus `dump_db_remote` / `dump_db_local`. For `--data`
//! including `db`, import an already-present dump via the existing import module
//! or tell the operator to run `shopware-cli project dump` on the source.

use super::app;
use super::data::normalize_data;
use super::env::{
    existing_compose_files, is_local_source, local_data_root, remote_data_root, require_shop_id,
    resolve_compose_dir, resolve_snapshot_dir, vps_project_name_opt, ShopEnv,
};
use super::error::Error;
use super::import;
use super::live::{assert_not_live, LivePolicy, LiveSignals};
use super::mysql::find_snapshot_dump;
use super::rewrite;
use super::ssh::{probe_ssh, probe_ssh_dry_run_line, resolve_ssh_source, SshSource};
use super::volumes;
use crate::cli::SyncOpArgs;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn dump_operator_instructions(snapshot_dir: &Path) -> String {
    format!(
        "No db.sql.gz (or db.sql) in {}. fyrst-cli does not dump databases and does not wrap or shell out to shopware-cli (no `docker run … project dump`). On the source, run `shopware-cli project dump` (example: shopware-cli project dump --skip-lock-tables --compression=gzip --output db.sql.gz) and place db.sql.gz in that directory. Then re-run `fyrst-cli shopware sync pull`, or import with: fyrst-cli shopware db import --file <path.sql|.sql.gz> (same module as `sync apply --data db`).",
        snapshot_dir.display()
    )
}

pub fn run(args: SyncOpArgs) -> Result<(), Error> {
    let process_env: HashMap<String, String> = std::env::vars().collect();
    let cwd = std::env::current_dir().map_err(|e| Error::fail(format!("cannot read cwd: {e}")))?;
    run_with(&args, &process_env, &cwd)
}

fn run_with(
    args: &SyncOpArgs,
    process_env: &HashMap<String, String>,
    cwd: &Path,
) -> Result<(), Error> {
    let compose_dir = resolve_compose_dir(process_env, cwd)?;
    let compose_dir = fs::canonicalize(&compose_dir).unwrap_or(compose_dir);
    let env = ShopEnv::load(compose_dir.clone(), process_env)?;
    let shop_id = require_shop_id(&env)?;
    let selection = normalize_data(args.data.as_deref(), args.skip_db, args.skip_volumes)?;
    let snapshot_dir = resolve_snapshot_dir(args.snapshot_dir.as_deref(), &env, &compose_dir);
    let from = args
        .from
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("local")
        .to_string();
    let local = is_local_source(Some(&from));
    let dry_run = args.dry_run;

    let signals = LiveSignals::from_shop(&env);
    if let Some(warn) = assert_not_live(&signals, LivePolicy::SyncRestore)? {
        eprintln!("{warn}");
    }
    if rewrite::requested(&env) {
        rewrite::assert_not_live(&signals)?;
    }

    let (data_root, derived) = local_data_root(&env)?;
    if derived {
        println!(
            "==> SHOPWARE_DATA_ROOT unset; derived {}",
            data_root.display()
        );
    }
    let project = vps_project_name_opt(&env).unwrap_or_default();
    let compose_files = existing_compose_files(&compose_dir);

    println!(
        "==> Runtime data sync  from={from}  data={}  shop={shop_id}  deploy_env={}  project={}  data_root={}  snapshot_dir={}  compose_dir={}  dry-run={}",
        selection.items_csv(),
        env.get("SHOPWARE_DEPLOY_ENV").unwrap_or("unset"),
        if project.is_empty() { "unset" } else { project.as_str() },
        data_root.display(),
        snapshot_dir.display(),
        compose_dir.display(),
        if dry_run { 1 } else { 0 },
    );
    if dry_run {
        println!("==> Snapshot directory: {}", snapshot_dir.display());
    }

    if local {
        println!(
            "==> sync --from local captures this host then applies the same files (pipeline check). Prefer --from <live-alias> on staging."
        );
        return run_local(
            &env,
            &signals,
            cwd,
            &compose_dir,
            &compose_files,
            &selection,
            &snapshot_dir,
            &data_root,
            dry_run,
        );
    }

    let ssh = resolve_ssh_source(&from, &env)?;
    println!("==> SSH {} port {}", ssh.target, ssh.port);
    run_remote(
        &env,
        &signals,
        cwd,
        &compose_dir,
        &compose_files,
        &selection,
        &snapshot_dir,
        &data_root,
        &shop_id,
        &from,
        &ssh,
        dry_run,
    )
}

fn run_local(
    env: &ShopEnv,
    signals: &LiveSignals,
    cwd: &Path,
    compose_dir: &Path,
    compose_files: &[String],
    selection: &super::data::DataSelection,
    snapshot_dir: &Path,
    data_root: &Path,
    dry_run: bool,
) -> Result<(), Error> {
    let missing_dump = db_missing(selection, snapshot_dir);
    if !dry_run && missing_dump {
        return Err(Error::fail(dump_operator_instructions(snapshot_dir)));
    }

    let stopped = app::stop_app_containers(
        compose_dir,
        compose_files,
        vps_project_name_opt(env).as_deref(),
        dry_run,
    )?;
    let imported = import_db_if_present(env, cwd, selection, snapshot_dir, dry_run)?;
    rewrite::maybe_rewrite(env, signals, compose_dir, compose_files, imported, dry_run)?;
    for logical in &selection.volumes {
        volumes::sync_bind_local_roundtrip(env, data_root, snapshot_dir, logical, dry_run)?;
    }
    app::start_stopped_app(
        compose_dir,
        compose_files,
        vps_project_name_opt(env).as_deref(),
        &stopped,
        dry_run,
    )?;
    app::post_restore_hints(
        env,
        rewrite::requested(env),
        dry_run,
        compose_dir,
        compose_files,
    );
    println!(
        "==> Sync finished from local → {} (SHOPWARE_DEPLOY_ENV={})",
        data_root.display(),
        env.get("SHOPWARE_DEPLOY_ENV").unwrap_or("unset")
    );
    if missing_dump {
        return Err(Error::fail(dump_operator_instructions(snapshot_dir)));
    }
    Ok(())
}

fn run_remote(
    env: &ShopEnv,
    signals: &LiveSignals,
    cwd: &Path,
    compose_dir: &Path,
    compose_files: &[String],
    selection: &super::data::DataSelection,
    snapshot_dir: &Path,
    data_root: &Path,
    shop_id: &str,
    from: &str,
    ssh: &SshSource,
    dry_run: bool,
) -> Result<(), Error> {
    println!("==> Probing SSH {}", ssh.target);
    if dry_run {
        println!("==> {}", probe_ssh_dry_run_line(ssh));
    } else {
        probe_ssh(ssh)?;
    }

    let remote_root = resolve_remote_data_root(ssh, env, shop_id, dry_run)?;
    let missing_dump = db_missing(selection, snapshot_dir);
    if !dry_run && missing_dump {
        return Err(Error::fail(dump_operator_instructions(snapshot_dir)));
    }

    let stopped = app::stop_app_containers(
        compose_dir,
        compose_files,
        vps_project_name_opt(env).as_deref(),
        dry_run,
    )?;
    let imported = import_db_if_present(env, cwd, selection, snapshot_dir, dry_run)?;
    rewrite::maybe_rewrite(env, signals, compose_dir, compose_files, imported, dry_run)?;
    for logical in &selection.volumes {
        volumes::sync_bind_from_remote(
            ssh,
            env,
            &remote_root,
            data_root,
            snapshot_dir,
            logical,
            dry_run,
        )?;
    }
    app::start_stopped_app(
        compose_dir,
        compose_files,
        vps_project_name_opt(env).as_deref(),
        &stopped,
        dry_run,
    )?;
    app::post_restore_hints(
        env,
        rewrite::requested(env),
        dry_run,
        compose_dir,
        compose_files,
    );
    println!(
        "==> Sync finished from {from} → {} (SHOPWARE_DEPLOY_ENV={})",
        data_root.display(),
        env.get("SHOPWARE_DEPLOY_ENV").unwrap_or("unset")
    );
    if missing_dump {
        return Err(Error::fail(dump_operator_instructions(snapshot_dir)));
    }
    Ok(())
}

fn db_missing(selection: &super::data::DataSelection, snapshot_dir: &Path) -> bool {
    selection.want_db && find_snapshot_dump(snapshot_dir).is_err()
}

fn import_db_if_present(
    env: &ShopEnv,
    cwd: &Path,
    selection: &super::data::DataSelection,
    snapshot_dir: &Path,
    dry_run: bool,
) -> Result<bool, Error> {
    if !selection.want_db {
        return Ok(false);
    }
    let Ok(file) = find_snapshot_dump(snapshot_dir) else {
        return Ok(false);
    };
    let plan = import::plan_with_env(env, cwd, &file, dry_run, LivePolicy::SyncRestore)?;
    import::execute(&plan)?;
    Ok(true)
}

fn resolve_remote_data_root(
    ssh: &SshSource,
    env: &ShopEnv,
    shop_id: &str,
    dry_run: bool,
) -> Result<PathBuf, Error> {
    if let Some(p) = &ssh.remote_data_root {
        println!("==> Remote bind-mount root: {}", p.display());
        return Ok(p.clone());
    }
    let p = remote_data_root(env, shop_id);
    if dry_run {
        println!(
            "==> DRY-RUN remote data root derived {} (probe skipped)",
            p.display()
        );
    } else {
        println!(
            "==> Remote bind-mount root derived: {} (shop={shop_id} env=live)",
            p.display()
        );
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_instructions_point_at_shopware_cli_not_wrap() {
        let msg = dump_operator_instructions(Path::new("/shop/var/runtime-sync"));
        assert!(msg.contains("shopware-cli project dump"), "{msg}");
        assert!(msg.contains("db import"), "{msg}");
        assert!(msg.contains("db.sql.gz"), "{msg}");
        assert!(msg.contains("does not dump"), "{msg}");
        assert!(msg.contains("docker run"), "{msg}");
        assert!(msg.contains("project dump"), "{msg}");
        assert!(
            !msg.contains("ghcr.io/shopware/shopware-cli"),
            "must not instruct running the dump image:\n{msg}"
        );
        assert!(!msg.contains("SYNC_DUMP_ENGINE"), "{msg}");
    }
}
