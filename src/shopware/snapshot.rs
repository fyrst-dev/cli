//! `fyrst-cli shopware sync snapshot` — bind-mount / volume trees, not a dump.
//!
//! Database dumps are owned by `shopware-cli project dump`. This command does
//! not dump, wrap, or shell out to shopware-cli. When `--data` includes `db`,
//! operators are told to dump themselves; volume copy still runs if selected.

use super::data::{normalize_data, DataSelection};
use super::env::{
    derive_local_data_root, derive_project_name, is_local_source, require_shop_id,
    resolve_compose_dir, resolve_snapshot_dir, ShopEnv,
};
use super::error::Error;
use super::ssh::{
    probe_ssh, probe_ssh_dry_run_line, resolve_remote_data_root, resolve_ssh_source, SshSource,
};
use super::volumes::{execute_volume, plan_local_volumes, plan_remote_volumes, VolumeAction};
use crate::cli::SyncOpArgs;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Kept for `--data db` / `--skip-volumes` (exit 2). Not a silent success.
pub const DUMP_OPERATOR_MSG: &str = "database dump. Dumps are owned by shopware-cli (`shopware-cli project dump`). fyrst-cli does not dump databases and does not wrap or shell out to shopware-cli. Import a dump with: fyrst-cli shopware db import --file <path.sql|.sql.gz>";

#[derive(Debug)]
pub struct SnapshotPlan {
    pub from: String,
    pub source_is_local: bool,
    pub shop_id: String,
    pub deploy_env: Option<String>,
    pub sync_env: Option<String>,
    pub project: Option<String>,
    pub data_root: PathBuf,
    pub remote_data_root: Option<PathBuf>,
    pub compose_dir: PathBuf,
    pub snapshot_dir: PathBuf,
    pub selection: DataSelection,
    pub dry_run: bool,
    pub extra_logs: Vec<String>,
    pub volumes: Vec<VolumeAction>,
    pub ssh: Option<SshSource>,
    env: ShopEnv,
}

impl SnapshotPlan {
    #[cfg(test)]
    pub fn log_text(&self) -> String {
        let mut s = String::new();
        for l in &self.extra_logs {
            s.push_str(l);
            s.push('\n');
        }
        for v in &self.volumes {
            for l in &v.logs {
                s.push_str(l);
                s.push('\n');
            }
        }
        s
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
) -> Result<SnapshotPlan, Error> {
    let selection = normalize_data(args.data.as_deref(), args.skip_db, args.skip_volumes)?;
    if selection.want_db && selection.volumes.is_empty() {
        return Err(Error::stub(DUMP_OPERATOR_MSG));
    }

    let compose_dir = resolve_compose_dir(process_env, cwd)?;
    let compose_dir = fs::canonicalize(&compose_dir).unwrap_or(compose_dir);
    let env = ShopEnv::load(compose_dir.clone(), process_env)?;
    let shop_id = require_shop_id(&env)?;
    let snapshot_dir = resolve_snapshot_dir(args.snapshot_dir.as_deref(), &env, &compose_dir);
    let data_root = derive_local_data_root(&env)?;
    let project = derive_project_name(&env).ok().map(|(n, _)| n);
    let from = match args
        .from
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => "local".to_string(),
        Some(s) => s.to_string(),
    };
    let source_is_local = is_local_source(args.from.as_deref());
    let mut extra_logs = Vec::new();

    let (remote_data_root, ssh, volumes) = if source_is_local {
        let volumes = plan_local_volumes(
            &selection.volumes,
            &data_root,
            &snapshot_dir,
            &env,
            args.dry_run,
        )?;
        (None, None, volumes)
    } else {
        let ssh = resolve_ssh_source(&from, &env)?;
        extra_logs.push(format!("Probing SSH {}", ssh.target));
        if args.dry_run {
            extra_logs.push(probe_ssh_dry_run_line(&ssh));
        }
        let (remote_root, mut root_logs) =
            resolve_remote_data_root(&ssh, &env, &shop_id, args.dry_run)?;
        extra_logs.append(&mut root_logs);
        if args.dry_run {
            extra_logs.push(format!(
                "Would snapshot from {} (--data {}) bind-mounts under {}",
                ssh.target,
                selection.items_csv(),
                remote_root.display()
            ));
        }
        let volumes = plan_remote_volumes(
            &selection.volumes,
            &remote_root,
            &snapshot_dir,
            &ssh,
            &shop_id,
            &env,
            args.dry_run,
        );
        (Some(remote_root), Some(ssh), volumes)
    };

    Ok(SnapshotPlan {
        from,
        source_is_local,
        shop_id,
        deploy_env: env.get("SHOPWARE_DEPLOY_ENV").map(str::to_string),
        sync_env: env.get("SYNC_ENV").map(str::to_string),
        project,
        data_root,
        remote_data_root,
        compose_dir,
        snapshot_dir,
        selection,
        dry_run: args.dry_run,
        extra_logs,
        volumes,
        ssh,
        env,
    })
}

pub fn execute(plan: &SnapshotPlan) -> Result<(), Error> {
    println!(
        "==> Runtime data snapshot  from={}  data={}  shop={}  deploy_env={}  project={}  env={}  data_root={}  snapshot_dir={}  compose_dir={}  dry-run={}",
        plan.from,
        plan.selection.items_csv(),
        plan.shop_id,
        plan.deploy_env.as_deref().unwrap_or("unset"),
        plan.project.as_deref().unwrap_or(""),
        plan.sync_env.as_deref().unwrap_or("unset"),
        plan.data_root.display(),
        plan.snapshot_dir.display(),
        plan.compose_dir.display(),
        if plan.dry_run { 1 } else { 0 },
    );
    if let Some(ssh) = &plan.ssh {
        println!(
            "==> SSH {} port {}  remote={}",
            ssh.target, ssh.port, ssh.remote_path
        );
    }
    for line in &plan.extra_logs {
        println!("==> {line}");
    }
    if let (Some(ssh), false) = (&plan.ssh, plan.dry_run) {
        probe_ssh(ssh)?;
    }

    if plan.selection.want_db {
        println!(
            "==> {DUMP_OPERATOR_MSG} Place the dump at {}/db.sql.gz (shopware-cli --output).",
            plan.snapshot_dir.display()
        );
    }

    ensure_snapshot_dir(&plan.snapshot_dir, plan.dry_run)?;

    for action in &plan.volumes {
        execute_volume(action, plan.dry_run, &plan.env)?;
    }

    write_manifest(plan)?;

    if plan.dry_run {
        println!(
            "==> Snapshot plan finished (no files copied) {}",
            plan.snapshot_dir.display()
        );
    } else {
        println!("==> Snapshot written to {}", plan.snapshot_dir.display());
    }
    Ok(())
}

fn ensure_snapshot_dir(dir: &Path, dry_run: bool) -> Result<(), Error> {
    println!("==> Snapshot directory: {}", dir.display());
    if dry_run {
        return Ok(());
    }
    fs::create_dir_all(dir.join("volumes")).map_err(|e| {
        Error::fail(format!(
            "cannot create {}: {e}",
            dir.join("volumes").display()
        ))
    })?;
    fs::create_dir_all(dir.join("data"))
        .map_err(|e| Error::fail(format!("cannot create {}: {e}", dir.join("data").display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
            .map_err(|e| Error::fail(format!("cannot chmod 700 {}: {e}", dir.display())))?;
    }
    Ok(())
}

fn write_manifest(plan: &SnapshotPlan) -> Result<(), Error> {
    let dest = plan.snapshot_dir.join("MANIFEST.txt");
    if plan.dry_run {
        println!("==> Would write {}", dest.display());
        return Ok(());
    }
    let created = utc_stamp();
    let source_host = plan
        .ssh
        .as_ref()
        .map(|s| s.host.as_str())
        .unwrap_or("localhost");
    let remote_root = plan
        .remote_data_root
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let body = format!(
        "shopware-runtime-snapshot 1\n\
created={created}\n\
source_alias={from}\n\
source_local={local}\n\
source_host={source_host}\n\
consumer_sync_env={sync_env}\n\
compose_dir={compose}\n\
project={project}\n\
data={data}\n\
data_root={data_root}\n\
remote_data_root={remote_root}\n\
transport=rsync-bind-mounts\n\
dump=operator-run shopware-cli project dump (fyrst-cli does not dump)\n\
object_storage=out-of-scope\n",
        from = plan.from,
        local = if plan.source_is_local { "1" } else { "0" },
        sync_env = plan.sync_env.as_deref().unwrap_or(""),
        compose = plan.compose_dir.display(),
        project = plan.project.as_deref().unwrap_or(""),
        data = plan.selection.items_csv(),
        data_root = plan.data_root.display(),
    );
    fs::write(&dest, body)
        .map_err(|e| Error::fail(format!("cannot write {}: {e}", dest.display())))?;
    Ok(())
}

fn utc_stamp() -> String {
    Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::SyncOpArgs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempShop(PathBuf);
    impl TempShop {
        fn new(prefix: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir().join(format!(
                "fyrst-cli-snap-{prefix}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(p.join("deploy")).unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
        fn write_min(&self) {
            let data = self.0.join("data");
            fs::create_dir_all(data.join("media")).unwrap();
            fs::write(data.join("media/hello.txt"), b"media-bytes").unwrap();
            fs::write(
                self.0.join(".env"),
                format!(
                    "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=staging\nSHOPWARE_DATA_ROOT={}\n",
                    data.display()
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

    fn args(data: Option<&str>, from: Option<&str>) -> SyncOpArgs {
        SyncOpArgs {
            from: from.map(str::to_string),
            data: data.map(str::to_string),
            snapshot_dir: None,
            dry_run: true,
            skip_db: false,
            skip_volumes: false,
        }
    }

    fn process(shop: &Path) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("COMPOSE_DIR".into(), shop.to_string_lossy().into_owned());
        m
    }

    #[test]
    fn db_points_at_shopware_cli() {
        let err = plan(
            &args(Some("db"), Some("local")),
            &HashMap::new(),
            Path::new("/"),
        )
        .unwrap_err();
        match err {
            Error::NotImplemented(m) => {
                assert!(m.contains("shopware-cli project dump"), "{m}");
                assert!(m.contains("db import"), "{m}");
                assert!(!m.contains("docker run"), "{m}");
            }
            Error::Fail(m) => panic!("expected stub: {m}"),
        }
    }

    #[test]
    fn skip_volumes_db_is_not_silent_success() {
        let mut a = args(Some("all"), None);
        a.skip_volumes = true;
        let err = plan(&a, &HashMap::new(), Path::new("/")).unwrap_err();
        match err {
            Error::NotImplemented(m) => {
                assert!(m.contains("shopware-cli project dump"), "{m}");
                assert!(m.contains("db import"), "{m}");
            }
            Error::Fail(m) => panic!("expected stub: {m}"),
        }
    }

    #[test]
    fn remote_db_only_still_not_dump() {
        let err = plan(
            &args(Some("db"), Some("live")),
            &HashMap::new(),
            Path::new("/"),
        )
        .unwrap_err();
        match err {
            Error::NotImplemented(m) => {
                assert!(m.contains("shopware-cli project dump"), "{m}");
                assert!(!m.contains("docker run"), "{m}");
            }
            Error::Fail(m) => panic!("expected stub: {m}"),
        }
    }

    #[test]
    fn volumes_dry_run_plans_rsync() {
        let shop = TempShop::new("vol");
        shop.write_min();
        let mut a = args(Some("media"), Some("local"));
        a.snapshot_dir = Some(shop.path().join("var/runtime-sync").display().to_string());
        let p = plan(&a, &process(shop.path()), shop.path()).unwrap();
        assert!(!p.selection.want_db);
        assert_eq!(p.volumes.len(), 1);
        let text = p.log_text();
        assert!(text.contains("DRY-RUN rsync"), "{text}");
        assert!(text.contains("data/media"), "{text}");
        assert!(!text.contains("ghcr.io/shopware/shopware-cli"), "{text}");
        assert!(!text.contains("project dump"), "{text}");
        assert!(!text.contains("docker run"), "{text}");
    }

    #[test]
    fn all_data_plans_volume_rsync() {
        let shop = TempShop::new("all");
        shop.write_min();
        for item in ["files", "thumbnail", "theme", "sitemap"] {
            fs::create_dir_all(shop.path().join("data").join(item)).unwrap();
        }
        let p = plan(&args(None, None), &process(shop.path()), shop.path()).unwrap();
        assert!(p.selection.want_db);
        assert_eq!(p.volumes.len(), 5);
        let text = p.log_text();
        assert!(text.contains("DRY-RUN rsync"), "{text}");
        assert!(!text.contains("ghcr.io/shopware/shopware-cli"), "{text}");
        assert!(!text.contains("docker run"), "{text}");
    }

    #[test]
    fn remote_from_dry_run_rsync_no_dump() {
        let shop = TempShop::new("ssh");
        shop.write_min();
        fs::write(
            shop.path().join("deploy/sync.env"),
            "SYNC_REMOTE_PATH=/opt/shopware/acme\nSYNC_REMOTE_DATA_ROOT=/var/lib/shopware/data/acme/live\n",
        )
        .unwrap();
        let p = plan(
            &args(Some("media"), Some("live")),
            &process(shop.path()),
            shop.path(),
        )
        .unwrap();
        assert!(!p.source_is_local);
        let text = p.log_text();
        assert!(text.contains("DRY-RUN rsync"), "{text}");
        assert!(text.contains("live:"), "{text}");
        assert!(text.contains("/media"), "{text}");
        assert!(!text.contains("ghcr.io/shopware/shopware-cli"), "{text}");
        assert!(!text.contains("deploy/sync-runtime.sh"), "{text}");
        assert!(!text.contains("project dump"), "{text}");
    }

    #[test]
    fn refuse_mysql_data() {
        let err = plan(
            &args(Some("mysql_data"), None),
            &HashMap::new(),
            Path::new("/"),
        )
        .unwrap_err();
        match err {
            Error::Fail(m) => assert!(m.contains("mysql_data"), "{m}"),
            Error::NotImplemented(m) => panic!("expected fail: {m}"),
        }
    }
}
