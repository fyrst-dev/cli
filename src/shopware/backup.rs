//! `fyrst-cli shopware backup backup` — timestamped off-host backup.
//!
//! Allowed (and expected) on `SHOPWARE_DEPLOY_ENV=live`. Sync is not a backup.
//!
//! Database dumps stay with `shopware-cli project dump`. This command copies
//! bind-mount trees and, when `--data` includes `db`, copies an operator-provided
//! `db.sql.gz` (`BACKUP_DB_DUMP`) — it never wraps dump.

use super::data::{normalize_data, DataSelection};
use super::env::{
    derive_project_name, require_deploy_env, require_shop_id, resolve_backup_data_root,
    resolve_compose_dir, ShopEnv,
};
use super::error::Error;
use super::lock;
use super::mysql::verify_gzip_magic;
use super::prune::{self, utc_stamp, PruneOpts};
use super::target::{
    artifact_relpath, parse_backup_target, resolve_local_target_path, ssh_argv, BackupTarget,
};
use super::volumes::{command_exists, snapshot_bind_local, DEFAULT_ARCHIVE_IMAGE};
use crate::cli::BackupOpArgs;
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Overlay operator instruction (never executed as a wrap).
pub const DUMP_OPERATOR_HINT: &str = "run shopware-cli project dump yourself";

pub fn run(args: BackupOpArgs) -> Result<(), Error> {
    let process_env: HashMap<String, String> = std::env::vars().collect();
    let cwd = std::env::current_dir().map_err(|e| Error::fail(format!("cannot read cwd: {e}")))?;
    run_with_env(&args, &process_env, &cwd)
}

pub fn run_with_env(
    args: &BackupOpArgs,
    process_env: &HashMap<String, String>,
    cwd: &Path,
) -> Result<(), Error> {
    let compose_dir = resolve_compose_dir(process_env, cwd)?;
    let compose_dir = fs::canonicalize(&compose_dir).unwrap_or(compose_dir);
    let mut env = ShopEnv::load(compose_dir.clone(), process_env)?;
    env.load_backup_overlay(process_env)?;

    let shop_id = require_shop_id(&env)?;
    let deploy_env = require_deploy_env(&env)?;
    let selection = normalize_data(args.data.as_deref(), false, false)?;
    let (project_name, _derived) = derive_project_name(&env)?;
    let (data_root, data_root_derived) = resolve_backup_data_root(&env, &shop_id, &deploy_env);

    let raw_target = env.get("BACKUP_TARGET").unwrap_or("").to_string();
    let ssh_port = env.get("BACKUP_SSH_PORT").unwrap_or("22");
    let mut target = parse_backup_target(&raw_target, ssh_port)?;
    if let BackupTarget::Local { path } = &target {
        target = BackupTarget::Local {
            path: resolve_local_target_path(path, &compose_dir),
        };
    }

    let keep_days = prune::parse_keep_days(env.get("BACKUP_KEEP_DAYS"))?;
    let ssh_key = env.get("BACKUP_SSH_KEY").map(PathBuf::from);
    let db_dump = env.get("BACKUP_DB_DUMP").map(PathBuf::from);
    let archive_image = env
        .get("SYNC_ARCHIVE_IMAGE")
        .unwrap_or(DEFAULT_ARCHIVE_IMAGE)
        .to_string();
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stamp = utc_stamp(now_unix);

    let ctx = BackupCtx {
        compose_dir,
        shop_id,
        deploy_env,
        project_name,
        data_root,
        data_root_derived,
        target,
        backup_target_raw: raw_target,
        keep_days,
        ssh_key,
        db_dump,
        archive_image,
        selection,
        dry_run: args.dry_run,
        stamp,
        now_unix,
    };
    execute(&ctx)
}

struct BackupCtx {
    compose_dir: PathBuf,
    shop_id: String,
    deploy_env: String,
    project_name: String,
    data_root: PathBuf,
    data_root_derived: bool,
    target: BackupTarget,
    backup_target_raw: String,
    keep_days: u32,
    ssh_key: Option<PathBuf>,
    db_dump: Option<PathBuf>,
    archive_image: String,
    selection: DataSelection,
    dry_run: bool,
    stamp: String,
    now_unix: u64,
}

impl BackupCtx {
    fn artifact_rel(&self) -> String {
        artifact_relpath(&self.shop_id, &self.deploy_env, &self.stamp)
    }

    fn local_artifact(&self) -> PathBuf {
        if self.target.is_ssh() {
            self.compose_dir.join("var/backup-work").join(&self.stamp)
        } else {
            PathBuf::from(self.target.artifact_dir(&self.artifact_rel()))
        }
    }
}

fn execute(ctx: &BackupCtx) -> Result<(), Error> {
    #[cfg(unix)]
    {
        extern "C" {
            fn umask(mask: u32) -> u32;
        }
        unsafe {
            umask(0o077);
        }
    }

    let _lock = lock::acquire(&ctx.compose_dir)?;

    log(&format!(
        "Runtime backup backup shop={} deploy_env={} target={} keep_days={} dry-run={}",
        ctx.shop_id,
        ctx.deploy_env,
        ctx.backup_target_raw,
        ctx.keep_days,
        u8::from(ctx.dry_run),
    ));

    if ctx.target.is_ssh() && !ctx.dry_run {
        probe_ssh(ctx)?;
    }

    let rel = ctx.artifact_rel();
    let local_artifact = ctx.local_artifact();
    log(&format!(
        "Backup {}/{} data={} → {}/{}",
        ctx.shop_id,
        ctx.deploy_env,
        ctx.selection.items_csv(),
        ctx.backup_target_raw.trim_end_matches('/'),
        rel
    ));
    log(&format!(
        "SHOPWARE_DEPLOY_ENV={} (live is allowed; this is not sync)",
        ctx.deploy_env
    ));
    if ctx.data_root_derived {
        log(&format!(
            "SHOPWARE_DATA_ROOT unset; derived {}",
            ctx.data_root.display()
        ));
    }
    log(&format!(
        "artifact stamp path: {}",
        local_artifact.display()
    ));

    if ctx.selection.want_db {
        print_dump_instruction(&local_artifact, ctx.db_dump.as_deref());
    }

    if ctx.dry_run {
        dry_run_plan(ctx, &local_artifact)?;
        prune::prune_artifacts(&prune_opts(ctx))?;
        return Ok(());
    }

    if ctx.selection.want_db {
        resolve_operator_dump(ctx)?;
    }

    mkdir_mode_700(&local_artifact)?;
    fs::create_dir_all(local_artifact.join("data")).ok();

    if ctx.selection.want_db {
        copy_operator_dump(ctx, &local_artifact)?;
    }

    for vol in &ctx.selection.volumes {
        snapshot_bind_local(
            vol,
            &ctx.data_root,
            &local_artifact,
            &ctx.project_name,
            &ctx.archive_image,
            false,
        )?;
    }

    write_backup_manifest(ctx, &local_artifact)?;
    write_sha256sums(&local_artifact)?;

    copy_artifact_to_target(ctx, &local_artifact, &rel)?;

    if ctx.target.is_ssh() {
        let _ = fs::remove_dir_all(&local_artifact);
    }

    prune::prune_artifacts(&prune_opts(ctx))?;
    log(&format!(
        "Backup finished {}/{}",
        ctx.backup_target_raw.trim_end_matches('/'),
        rel
    ));
    Ok(())
}

fn dry_run_plan(ctx: &BackupCtx, local_artifact: &Path) -> Result<(), Error> {
    if ctx.selection.want_db {
        if let Some(src) = ctx.db_dump.as_ref() {
            let dest_name = dump_dest_name(src);
            log(&format!(
                "DRY-RUN copy operator dump {} → {}/{dest_name}",
                src.display(),
                local_artifact.display()
            ));
        }
    }
    for vol in &ctx.selection.volumes {
        snapshot_bind_local(
            vol,
            &ctx.data_root,
            local_artifact,
            &ctx.project_name,
            &ctx.archive_image,
            true,
        )?;
    }
    log(&format!(
        "DRY-RUN would write {}/BACKUP_MANIFEST.txt",
        local_artifact.display()
    ));
    if command_exists("sha256sum") {
        log(&format!(
            "DRY-RUN would write {}/SHA256SUMS",
            local_artifact.display()
        ));
    }
    if ctx.target.is_ssh() {
        let dest = ctx.target.artifact_dir(&ctx.artifact_rel());
        let ssh_target = ctx.target.ssh_destination().unwrap();
        log(&format!(
            "DRY-RUN rsync {}/ → {ssh_target}:{dest}/",
            local_artifact.display()
        ));
    }
    Ok(())
}

fn print_dump_instruction(artifact: &Path, db_dump: Option<&Path>) {
    log(&format!(
        "{DUMP_OPERATOR_HINT} (fyrst-cli does not dump databases and does not wrap shopware-cli). \
Place db.sql.gz in {} or set BACKUP_DB_DUMP to copy an already-made dump. \
Example: shopware-cli project dump --skip-lock-tables --compression=gzip --output {}/db.sql.gz \
then import later with: fyrst-cli shopware db import --file <path.sql|.sql.gz>",
        artifact.display(),
        artifact.display()
    ));
    if db_dump.is_none() {
        log("No BACKUP_DB_DUMP set — execute will fail if --data includes db and no dump file is provided (it will not wrap shopware-cli).");
    }
}

fn resolve_operator_dump(ctx: &BackupCtx) -> Result<PathBuf, Error> {
    if let Some(p) = ctx.db_dump.as_ref() {
        let p = if p.is_absolute() {
            p.clone()
        } else {
            ctx.compose_dir.join(p)
        };
        if !p.is_file() {
            return Err(Error::fail(format!(
                "BACKUP_DB_DUMP is not a file: {}. fyrst-cli does not dump databases. Run `{DUMP_OPERATOR_HINT}` into a .sql.gz and point BACKUP_DB_DUMP at it.",
                p.display()
            )));
        }
        return Ok(p);
    }
    Err(Error::fail(format!(
        "--data includes db but no dump file was provided. fyrst-cli does not dump databases and does not wrap or shell out to shopware-cli. {DUMP_OPERATOR_HINT} (`shopware-cli project dump --skip-lock-tables --compression=gzip --output db.sql.gz`) and copy it into the artifact, or set BACKUP_DB_DUMP to an already-made db.sql.gz."
    )))
}

fn dump_dest_name(src: &Path) -> &'static str {
    let name = src
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.ends_with(".sql") && !name.ends_with(".sql.gz") {
        "db.sql"
    } else {
        "db.sql.gz"
    }
}

fn copy_operator_dump(ctx: &BackupCtx, artifact: &Path) -> Result<(), Error> {
    let src = resolve_operator_dump(ctx)?;
    let dest_name = dump_dest_name(&src);
    let dest = artifact.join(dest_name);
    if dest_name.ends_with(".gz") {
        verify_gzip_magic(&src)?;
    }
    log(&format!(
        "Copy operator dump {} → {}",
        src.display(),
        dest.display()
    ));
    fs::copy(&src, &dest).map_err(|e| {
        Error::fail(format!(
            "cannot copy dump {} → {}: {e}",
            src.display(),
            dest.display()
        ))
    })?;
    Ok(())
}

fn write_backup_manifest(ctx: &BackupCtx, dest: &Path) -> Result<(), Error> {
    let mf = dest.join("BACKUP_MANIFEST.txt");
    let created = iso_utc(ctx.now_unix);
    let host = hostname_full();
    let body = format!(
        "\
shopware-vps-backup 1
created={created}
stamp={}
shop_id={}
deploy_env={}
host={host}
compose_dir={}
project={}
data={}
data_root={}
backup_target={}
sync_is_not_backup=true
object_storage=out-of-scope
wal_pitr=out-of-scope
",
        ctx.stamp,
        ctx.shop_id,
        ctx.deploy_env,
        ctx.compose_dir.display(),
        ctx.project_name,
        ctx.selection.items_csv(),
        ctx.data_root.display(),
        ctx.backup_target_raw,
    );
    fs::write(&mf, body).map_err(|e| Error::fail(format!("cannot write {}: {e}", mf.display())))?;
    log(&format!("Wrote {}", mf.display()));
    Ok(())
}

fn write_sha256sums(dest: &Path) -> Result<(), Error> {
    if !command_exists("sha256sum") {
        return Ok(());
    }
    let mut files = list_files_relative(dest);
    files.retain(|p| {
        let name = Path::new(p)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        name != "SHA256SUMS" && name != "BACKUP_MANIFEST.txt"
    });
    files.sort();
    if files.is_empty() {
        return Ok(());
    }
    let mut cmd = Command::new("sha256sum");
    cmd.args(&files);
    cmd.current_dir(dest);
    let out = cmd
        .output()
        .map_err(|e| Error::fail(format!("could not exec sha256sum: {e}")))?;
    if !out.status.success() {
        return Err(Error::fail("sha256sum failed"));
    }
    let path = dest.join("SHA256SUMS");
    fs::write(&path, out.stdout)
        .map_err(|e| Error::fail(format!("cannot write {}: {e}", path.display())))?;
    log(&format!("Wrote {}", path.display()));
    Ok(())
}

fn list_files_relative(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        let rd = match fs::read_dir(dir) {
            Ok(r) => r,
            Err(_) => return,
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, root, out);
            } else if p.is_file() {
                if let Ok(rel) = p.strip_prefix(root) {
                    out.push(rel.to_string_lossy().into_owned());
                }
            }
        }
    }
    walk(root, root, &mut out);
    out
}

fn copy_artifact_to_target(ctx: &BackupCtx, src: &Path, rel: &str) -> Result<(), Error> {
    match &ctx.target {
        BackupTarget::Local { .. } => Ok(()),
        BackupTarget::Ssh { port, .. } => {
            if !command_exists("rsync") {
                return Err(Error::fail(
                    "rsync is required to copy backups to an SSH BACKUP_TARGET",
                ));
            }
            let dest = ctx.target.artifact_dir(rel);
            let ssh_target = ctx.target.ssh_destination().unwrap();
            let parent = dest.rsplit_once('/').map(|(p, _)| p).unwrap_or("/");
            ensure_remote_parent(ctx, parent)?;
            let e_flag = ssh_e_flag(port, ctx.ssh_key.as_deref());
            log(&format!("rsync {}/ → {ssh_target}:{dest}/", src.display()));
            let status = Command::new("rsync")
                .args(["-aH", "--delete", "--numeric-ids", "-e", &e_flag])
                .arg(format!("{}/", src.display()))
                .arg(format!("{ssh_target}:{dest}/"))
                .status()
                .map_err(|e| Error::fail(format!("could not exec rsync: {e}")))?;
            if !status.success() {
                return Err(Error::fail(format!("rsync to {ssh_target}:{dest} failed")));
            }
            Ok(())
        }
    }
}

fn ssh_e_flag(port: &str, key: Option<&Path>) -> String {
    let mut parts = vec!["ssh".to_string()];
    parts.extend(ssh_argv(port, key));
    parts.join(" ")
}

fn ensure_remote_parent(ctx: &BackupCtx, dest: &str) -> Result<(), Error> {
    let payload = format!("mkdir -p {}", sh_quote(dest));
    super::prune::remote_sh(&ctx.target, ctx.ssh_key.as_deref(), &payload)?;
    Ok(())
}

fn probe_ssh(ctx: &BackupCtx) -> Result<(), Error> {
    let dest = ctx.target.ssh_destination().unwrap();
    let port = ctx.target.ssh_port().unwrap_or("22");
    log(&format!("Probing SSH {dest}"));
    if !command_exists("ssh") {
        return Err(Error::fail("ssh is required to reach an SSH BACKUP_TARGET"));
    }
    let args = ssh_argv(port, ctx.ssh_key.as_deref());
    let status = Command::new("ssh")
        .args(&args)
        .arg(&dest)
        .arg("true")
        .status()
        .map_err(|e| Error::fail(format!("could not exec ssh: {e}")))?;
    if !status.success() {
        return Err(Error::fail(format!(
            "SSH to {dest} failed (BatchMode). Check BACKUP_TARGET / BACKUP_SSH_KEY."
        )));
    }
    Ok(())
}

fn mkdir_mode_700(path: &Path) -> Result<(), Error> {
    fs::create_dir_all(path)
        .map_err(|e| Error::fail(format!("cannot mkdir {}: {e}", path.display())))?;
    let mut perms = fs::metadata(path)
        .map_err(|e| Error::fail(format!("cannot stat {}: {e}", path.display())))?
        .permissions();
    perms.set_mode(0o700);
    let _ = fs::set_permissions(path, perms);
    Ok(())
}

fn prune_opts<'a>(ctx: &'a BackupCtx) -> PruneOpts<'a> {
    PruneOpts {
        target: &ctx.target,
        shop_id: &ctx.shop_id,
        deploy_env: &ctx.deploy_env,
        keep_days: ctx.keep_days,
        dry_run: ctx.dry_run,
        now_unix: ctx.now_unix,
        ssh_key: ctx.ssh_key.as_deref(),
    }
}

fn iso_utc(unix_secs: u64) -> String {
    let stamp = utc_stamp(unix_secs);
    // YYYYMMDDTHHMMSSZ → YYYY-MM-DDTHH:MM:SSZ
    format!(
        "{}-{}-{}T{}:{}:{}Z",
        &stamp[0..4],
        &stamp[4..6],
        &stamp[6..8],
        &stamp[9..11],
        &stamp[11..13],
        &stamp[13..15],
    )
}

fn hostname_full() -> String {
    for args in [&["-f"][..], &[][..]] {
        let out = match Command::new("hostname").args(args).output() {
            Ok(o) if o.status.success() => o,
            _ => continue,
        };
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    "unknown".into()
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn log(msg: &str) {
    println!("==> {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
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
        fn write_shop(&self, deploy_env: &str) {
            fs::write(
                self.0.join(".env"),
                format!("SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV={deploy_env}\n"),
            )
            .unwrap();
            fs::write(
                self.0.join("deploy/compose.yaml"),
                "services:\n  web:\n    image: example\n",
            )
            .unwrap();
        }
    }
    impl Drop for TempShop {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn process(shop: &Path, extra: &[(&str, &str)]) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("COMPOSE_DIR".into(), shop.to_string_lossy().into_owned());
        for (k, v) in extra {
            m.insert((*k).into(), (*v).into());
        }
        m
    }

    fn args(data: Option<&str>, dry_run: bool) -> BackupOpArgs {
        BackupOpArgs {
            data: data.map(str::to_string),
            dry_run,
        }
    }

    #[test]
    fn missing_backup_target_fails_clearly() {
        let shop = TempShop::new("no-tgt");
        shop.write_shop("live");
        let err = run_with_env(
            &args(Some("media"), true),
            &process(shop.path(), &[]),
            shop.path(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("BACKUP_TARGET"), "{err}");
        assert!(!err.to_string().contains("not implemented"), "{err}");
    }

    #[test]
    fn dry_run_db_instructs_shopware_cli_no_wrap() {
        let shop = TempShop::new("dry-db");
        shop.write_shop("live");
        let backups = shop.path().join("backups");
        let proc = process(shop.path(), &[("BACKUP_TARGET", backups.to_str().unwrap())]);
        // Capture via executing is integration-level; here we only check Result + that
        // resolve_operator_dump fails without wrapping.
        run_with_env(&args(Some("db"), true), &proc, shop.path()).unwrap();
        let err = resolve_operator_dump(&BackupCtx {
            compose_dir: shop.path().to_path_buf(),
            shop_id: "acme".into(),
            deploy_env: "live".into(),
            project_name: "acme-live".into(),
            data_root: shop.path().join("data"),
            data_root_derived: false,
            target: BackupTarget::Local { path: backups },
            backup_target_raw: String::new(),
            keep_days: 14,
            ssh_key: None,
            db_dump: None,
            archive_image: DEFAULT_ARCHIVE_IMAGE.into(),
            selection: normalize_data(Some("db"), false, false).unwrap(),
            dry_run: false,
            stamp: "20260913T000000Z".into(),
            now_unix: 0,
        })
        .unwrap_err();
        assert!(
            err.to_string().contains("shopware-cli project dump"),
            "{err}"
        );
        assert!(err.to_string().contains(DUMP_OPERATOR_HINT), "{err}");
        assert!(!err.to_string().contains("docker run"), "{err}");
    }

    #[test]
    fn execute_copies_volumes_and_manifest_on_live() {
        let shop = TempShop::new("exec");
        shop.write_shop("live");
        let data = shop.path().join("sw-data");
        fs::create_dir_all(data.join("media")).unwrap();
        fs::write(data.join("media/logo.png"), b"png").unwrap();
        let backups = shop.path().join("backups");
        let dump = shop.path().join("provided.sql.gz");
        {
            let mut f = fs::File::create(&dump).unwrap();
            f.write_all(&[0x1f, 0x8b, 0x08, 0x00]).unwrap();
        }
        let proc = process(
            shop.path(),
            &[
                ("BACKUP_TARGET", backups.to_str().unwrap()),
                ("SHOPWARE_DATA_ROOT", data.to_str().unwrap()),
                ("BACKUP_DB_DUMP", dump.to_str().unwrap()),
                ("BACKUP_KEEP_DAYS", "0"),
            ],
        );
        run_with_env(&args(Some("db,media"), false), &proc, shop.path()).unwrap();

        let env_dir = backups.join("acme").join("live");
        let mut stamps: Vec<_> = fs::read_dir(&env_dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_dir())
            .collect();
        assert_eq!(stamps.len(), 1, "one timestamped artifact");
        let art = stamps.remove(0);
        assert!(art.join("data/media/logo.png").is_file());
        assert!(art.join("db.sql.gz").is_file());
        let mf = fs::read_to_string(art.join("BACKUP_MANIFEST.txt")).unwrap();
        assert!(mf.contains("shopware-vps-backup 1"), "{mf}");
        assert!(mf.contains("shop_id=acme"), "{mf}");
        assert!(mf.contains("deploy_env=live"), "{mf}");
        assert!(mf.contains("sync_is_not_backup=true"), "{mf}");
        if command_exists("sha256sum") {
            let sums = fs::read_to_string(art.join("SHA256SUMS")).unwrap();
            assert!(
                sums.contains("logo.png") || sums.contains("db.sql.gz"),
                "{sums}"
            );
            assert!(!sums.contains("BACKUP_MANIFEST.txt"), "{sums}");
        }
    }

    #[test]
    fn execute_db_without_dump_fails_not_wrap() {
        let shop = TempShop::new("nodump");
        shop.write_shop("staging");
        let backups = shop.path().join("backups");
        let proc = process(shop.path(), &[("BACKUP_TARGET", backups.to_str().unwrap())]);
        let err = run_with_env(&args(Some("db"), false), &proc, shop.path()).unwrap_err();
        assert!(err.to_string().contains("shopware-cli"), "{err}");
        assert!(!err.to_string().contains("docker run"), "{err}");
        assert!(!backups.join("acme").exists());
    }
}
