//! `fyrst-cli shopware backup recover` — disaster recovery onto this host.
//!
//! Not `sync apply` (live→staging clone). `--artifact` (aliases `--stamp`,
//! `--from`) and confirmation are required. Live needs
//! `BACKUP_ALLOW_LIVE_RESTORE=1`; inner apply then sets
//! `SYNC_ALLOW_LIVE_RESTORE=1`. DB import and bind-mount apply reuse the sync
//! apply module. fyrst-cli does not dump.

use super::data::normalize_data;
use super::env::{
    env_truthy, require_deploy_env, require_shop_id, resolve_compose_dir, resolve_data_root,
    ShopEnv,
};
use super::error::Error;
use super::restore;
use super::target::{
    artifact_relpath, parse_backup_target, resolve_local_target_path, BackupTarget,
};
use super::volumes::command_exists;
use crate::cli::BackupRecoverArgs;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn run(args: BackupRecoverArgs) -> Result<(), Error> {
    let process_env: HashMap<String, String> = std::env::vars().collect();
    let cwd = std::env::current_dir().map_err(|e| Error::fail(format!("cannot read cwd: {e}")))?;
    let compose_dir = resolve_compose_dir(&process_env, &cwd)?;
    let compose_dir = fs::canonicalize(&compose_dir).unwrap_or(compose_dir);
    let mut env = ShopEnv::load_backup(compose_dir, &process_env)?;
    restore_from_args(&args, &mut env, &cwd)
}

fn deploy_env_is_live(env: &ShopEnv) -> bool {
    env.get("SHOPWARE_DEPLOY_ENV")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.eq_ignore_ascii_case("live"))
        .unwrap_or(false)
}

fn looks_like_path(spec: &str) -> bool {
    spec.contains('/') || spec.starts_with('/')
}

pub(crate) fn restore_from_args(
    args: &BackupRecoverArgs,
    env: &mut ShopEnv,
    cwd: &Path,
) -> Result<(), Error> {
    let shop_id = require_shop_id(env)?;
    let from = args.artifact_spec().ok_or_else(|| {
        Error::fail("restore requires --artifact <timestamp|directory> (aliases: --stamp, --from)")
    })?;

    if !args.confirm_restore && !env_truthy(env.get("BACKUP_CONFIRM_RESTORE")) {
        return Err(Error::fail(
            "Restore onto this host needs --i-understand-this-restores-this-host (or BACKUP_CONFIRM_RESTORE=1). This overwrites DB and bind mounts.",
        ));
    }

    if deploy_env_is_live(env) && !env_truthy(env.get("BACKUP_ALLOW_LIVE_RESTORE")) {
        return Err(Error::fail(
            "Refusing restore onto SHOPWARE_DEPLOY_ENV=live without BACKUP_ALLOW_LIVE_RESTORE=1 (disaster recovery only; quarterly drill on staging does not need this).",
        ));
    }

    // Overlay always sets this for inner `sync apply` so the existing live
    // guard allows DR (hostname/checkout named live on a staging drill too).
    env.set("SYNC_ALLOW_LIVE_RESTORE", "1");

    let dry_run = args.common.dry_run;
    let selection = normalize_data(args.common.data.as_deref(), false, false)?;
    let (artifact, fetch_notes, skip_inner) = fetch_artifact(env, cwd, from, dry_run)?;

    let data_root = resolve_data_root(env)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unset".into());

    println!(
        "==> Runtime backup restore shop={} deploy_env={} from={} artifact={} target={} data={} dry-run={}",
        shop_id,
        env.get("SHOPWARE_DEPLOY_ENV").unwrap_or("unset"),
        from,
        artifact.display(),
        env.get("BACKUP_TARGET").unwrap_or("unset"),
        selection.items_csv(),
        if dry_run { 1 } else { 0 },
    );
    for n in &fetch_notes {
        println!("==> {n}");
    }
    println!(
        "==> Restoring {} into {} (SYNC_ALLOW_LIVE_RESTORE for live DR)",
        artifact.display(),
        data_root,
    );

    if skip_inner {
        println!(
            "==> DRY-RUN inner restore --data {} --snapshot-dir {}",
            selection.items_csv(),
            artifact.display()
        );
        return Ok(());
    }

    restore::apply_from_env(env, cwd, &artifact, &selection, dry_run)?;

    if !dry_run {
        println!(
            "==> Restore finished from {from}. Rewrite sales-channel URLs if this is not a same-host drill (rewrite itself is still refused on live)."
        );
    }
    Ok(())
}

fn fetch_artifact(
    env: &ShopEnv,
    cwd: &Path,
    spec: &str,
    dry_run: bool,
) -> Result<(PathBuf, Vec<String>, bool), Error> {
    let mut notes = Vec::new();
    if looks_like_path(spec) {
        let p = if Path::new(spec).is_absolute() {
            PathBuf::from(spec)
        } else {
            cwd.join(spec)
        };
        if p.is_dir() {
            let p = fs::canonicalize(&p).unwrap_or(p);
            if dry_run {
                notes.push(format!(
                    "DRY-RUN restore from {} (directory --from, no fetch copy)",
                    p.display()
                ));
            }
            return Ok((p, notes, false));
        }
        return Err(Error::fail(format!(
            "Artifact directory not found: {}",
            p.display()
        )));
    }

    let shop_id = require_shop_id(env)?;
    let deploy_env = require_deploy_env(env)?;
    let default_port = env.get("BACKUP_SSH_PORT").unwrap_or("22");
    let mut target = parse_backup_target(env.get("BACKUP_TARGET").unwrap_or(""), default_port)?;
    if let BackupTarget::Local { path } = &target {
        target = BackupTarget::Local {
            path: resolve_local_target_path(path, &env.compose_dir),
        };
    }
    let rel = artifact_relpath(&shop_id, &deploy_env, spec);

    if !target.is_ssh() {
        let artifact = PathBuf::from(target.artifact_dir(&rel));
        if !artifact.is_dir() {
            return Err(Error::fail(format!(
                "Artifact not found: {}",
                artifact.display()
            )));
        }
        if dry_run {
            notes.push(format!(
                "DRY-RUN local artifact {} (no fetch copy)",
                artifact.display()
            ));
        }
        return Ok((artifact, notes, false));
    }

    let local = env
        .compose_dir
        .join("var/backup-work")
        .join(format!("restore-{spec}"));
    let src = target.artifact_dir(&rel);
    let ssh_target = target
        .ssh_destination()
        .ok_or_else(|| Error::fail("internal error: ssh_destination on a local BACKUP_TARGET"))?;
    if dry_run {
        notes.push(format!(
            "DRY-RUN rsync {ssh_target}:{}/ → {}/",
            src.trim_end_matches('/'),
            local.display()
        ));
        return Ok((local, notes, true));
    }

    rsync_from_ssh(env, &target, &src, &local)?;
    Ok((local, notes, false))
}

fn ssh_e(target: &BackupTarget, identity: Option<&str>) -> Result<String, Error> {
    let port = target
        .ssh_port()
        .ok_or_else(|| Error::fail("internal error: ssh_e on a local BACKUP_TARGET"))?;
    let mut s = format!("ssh -o BatchMode=yes -o ConnectTimeout=15 -p {port}");
    if let Some(key) = identity.map(str::trim).filter(|s| !s.is_empty()) {
        s.push_str(" -o IdentitiesOnly=yes -i ");
        s.push_str(key);
    }
    Ok(s)
}

fn rsync_from_ssh(
    env: &ShopEnv,
    target: &BackupTarget,
    remote_path: &str,
    dest: &Path,
) -> Result<(), Error> {
    if !command_exists("rsync") {
        return Err(Error::fail(
            "rsync is required to fetch backups from an SSH BACKUP_TARGET",
        ));
    }
    if let Some(key) = env.get("BACKUP_SSH_KEY") {
        if !Path::new(key).is_file() {
            return Err(Error::fail(format!("BACKUP_SSH_KEY not found: {key}")));
        }
    }
    fs::create_dir_all(dest)
        .map_err(|e| Error::fail(format!("cannot create {}: {e}", dest.display())))?;
    let ssh_e_opt = ssh_e(target, env.get("BACKUP_SSH_KEY"))?;
    let ssh_target = target
        .ssh_destination()
        .ok_or_else(|| Error::fail("internal error: ssh_target on a local BACKUP_TARGET"))?;
    let src = format!("{ssh_target}:{}/", remote_path.trim_end_matches('/'));
    let dest_a = format!("{}/", dest.display());
    println!("==> rsync {src} → {dest_a}");
    let status = Command::new("rsync")
        .args(["-aH", "--delete", "--numeric-ids", "-e", &ssh_e_opt])
        .arg(&src)
        .arg(&dest_a)
        .status()
        .map_err(|e| Error::fail(format!("failed to exec rsync: {e}")))?;
    if !status.success() {
        return Err(Error::fail(format!(
            "rsync from {ssh_target} failed (BatchMode). Check BACKUP_TARGET / BACKUP_SSH_KEY."
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::BackupOpArgs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempShop(PathBuf);
    impl TempShop {
        fn new(prefix: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir().join(format!(
                "fyrst-cli-bk-{prefix}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(p.join("deploy")).unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
        fn write_min(&self, deploy_env: &str) {
            fs::write(
                self.0.join(".env"),
                format!(
                    "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV={deploy_env}\nMYSQL_USER=u\nMYSQL_PASSWORD=s3cret-value\nSHOPWARE_DATA_ROOT={}/data-root\n",
                    self.0.display()
                ),
            )
            .unwrap();
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

    fn args(
        from: Option<&str>,
        confirm: bool,
        dry_run: bool,
        data: Option<&str>,
    ) -> BackupRecoverArgs {
        BackupRecoverArgs {
            common: BackupOpArgs {
                data: data.map(str::to_string),
                dry_run,
            },
            artifact: from.map(str::to_string),
            confirm_restore: confirm,
        }
    }

    fn env_for(shop: &TempShop) -> ShopEnv {
        let mut process = HashMap::new();
        process.insert(
            "COMPOSE_DIR".into(),
            shop.path().to_string_lossy().into_owned(),
        );
        ShopEnv::load_backup(shop.path().to_path_buf(), &process).unwrap()
    }

    #[test]
    fn missing_from_fails() {
        let shop = TempShop::new("nofrom");
        shop.write_min("staging");
        let mut env = env_for(&shop);
        let err = restore_from_args(&args(None, true, true, Some("db")), &mut env, shop.path())
            .unwrap_err();
        assert!(err.to_string().contains("--artifact"), "{err}");
        assert!(err.to_string().contains("--from"), "{err}");
    }

    #[test]
    fn missing_confirm_mentions_flag_and_env() {
        let shop = TempShop::new("noconfirm");
        shop.write_min("staging");
        let mut env = env_for(&shop);
        let err = restore_from_args(
            &args(Some("20260912T020000Z"), false, true, Some("db")),
            &mut env,
            shop.path(),
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("--i-understand-this-restores-this-host"),
            "{err}"
        );
        assert!(err.to_string().contains("BACKUP_CONFIRM_RESTORE"), "{err}");
    }

    #[test]
    fn live_refuses_without_backup_allow() {
        let shop = TempShop::new("live");
        shop.write_min("live");
        let mut env = env_for(&shop);
        env.set("SYNC_ALLOW_LIVE_RESTORE", "1");
        let err = restore_from_args(
            &args(Some("20260912T020000Z"), true, true, Some("db")),
            &mut env,
            shop.path(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("BACKUP_ALLOW_LIVE_RESTORE"),
            "{err}"
        );
        assert!(err.to_string().contains("live"), "{err}");
    }

    #[test]
    fn missing_local_stamp_fails() {
        let shop = TempShop::new("miss");
        shop.write_min("staging");
        let backups = shop.path().join("backups");
        fs::create_dir_all(&backups).unwrap();
        let mut env = env_for(&shop);
        env.set("BACKUP_TARGET", backups.to_string_lossy());
        let err = restore_from_args(
            &args(Some("19990101T000000Z"), true, true, Some("db")),
            &mut env,
            shop.path(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("Artifact not found"), "{err}");
    }

    #[test]
    fn confirm_via_env_dry_run_local() {
        let shop = TempShop::new("envconfirm");
        shop.write_min("staging");
        let stamp = "20260912T020000Z";
        let art = shop.path().join("backups/acme/staging").join(stamp);
        fs::create_dir_all(&art).unwrap();
        fs::write(art.join("db.sql"), b"SELECT 1;\n").unwrap();
        let mut env = env_for(&shop);
        env.set(
            "BACKUP_TARGET",
            shop.path().join("backups").to_string_lossy(),
        );
        env.set("BACKUP_CONFIRM_RESTORE", "1");
        restore_from_args(
            &args(Some(stamp), false, true, Some("db")),
            &mut env,
            shop.path(),
        )
        .unwrap();
    }
}
