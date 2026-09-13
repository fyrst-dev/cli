//! `fyrst-cli shopware sync snapshot` — local DB dump via shopware-cli.

use super::data::{normalize_data, DataSelection};
use super::dump::{
    execute_dump, require_docker, resolve_dump_connection, DumpEngine, DumpOptions, DumpPlan,
};
use super::env::{
    compose_mentions_mysql_service, derive_project_name, is_local_source, require_shop_id,
    resolve_compose_dir, ShopEnv,
};
use super::error::Error;
use crate::cli::SyncOpArgs;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct SnapshotPlan {
    pub compose_dir: PathBuf,
    pub snapshot_dir: PathBuf,
    pub project_name: String,
    pub shop_id: String,
    pub deploy_env: Option<String>,
    pub selection: DataSelection,
    pub dry_run: bool,
    pub dump: Option<DumpPlan>,
    pub volume_stub: bool,
    pub notes: Vec<String>,
}

pub fn run(args: SyncOpArgs) -> Result<(), Error> {
    let process_env: HashMap<String, String> = std::env::vars().collect();
    let cwd = std::env::current_dir().map_err(|e| Error::fail(format!("cannot read cwd: {e}")))?;
    let plan = plan_snapshot(&process_env, &cwd, &args)?;
    if !plan.dry_run {
        require_docker()?;
    }
    execute(&plan)
}

pub fn plan_snapshot(
    process_env: &HashMap<String, String>,
    cwd: &Path,
    args: &SyncOpArgs,
) -> Result<SnapshotPlan, Error> {
    let compose_dir = resolve_compose_dir(process_env, cwd)?;
    let compose_dir = fs::canonicalize(&compose_dir).unwrap_or(compose_dir);
    let env = ShopEnv::load(compose_dir.clone(), process_env)?;
    let shop_id = require_shop_id(&env)?;
    let selection = normalize_data(args.data.as_deref(), args.skip_db, args.skip_volumes)?;

    if !is_local_source(args.from.as_deref()) {
        let alias = args.from.as_deref().unwrap_or("").trim();
        return Err(Error::stub(format!(
            "shopware sync snapshot --from {alias} (remote SSH is not implemented yet; use --from local)"
        )));
    }

    let snapshot_dir = resolve_snapshot_dir(args, &env, &compose_dir);
    let (project_name, derived) = derive_project_name(&env)?;

    let mut notes = Vec::new();
    if derived {
        notes.push(format!(
            "COMPOSE_PROJECT_NAME unset; derived {project_name}"
        ));
    } else if let (Some(id), Some(de)) =
        (env.get("SHOPWARE_SHOP_ID"), env.get("SHOPWARE_DEPLOY_ENV"))
    {
        let expected = format!("{id}-{de}");
        if project_name != expected {
            notes.push(format!(
                "WARNING: COMPOSE_PROJECT_NAME={project_name} is set and overrides Compose name: ({expected}). shopware-cli project create writes COMPOSE_PROJECT_NAME=sw-shop-… into .env for local project-dev. On the VPS, remove or comment out that line."
            ));
        }
    }

    let volume_stub = !selection.volumes.is_empty();
    if volume_stub && !selection.want_db {
        return Err(Error::stub(format!(
            "bind-mount volume snapshot ({})",
            selection.volumes.join(", ")
        )));
    }

    let dump = if selection.want_db {
        Some(plan_dump(&env, &snapshot_dir, &project_name)?)
    } else {
        None
    };

    Ok(SnapshotPlan {
        compose_dir,
        snapshot_dir,
        project_name,
        shop_id,
        deploy_env: env.get("SHOPWARE_DEPLOY_ENV").map(str::to_string),
        selection,
        dry_run: args.dry_run,
        dump,
        volume_stub,
        notes,
    })
}

fn plan_dump(env: &ShopEnv, snapshot_dir: &Path, project_name: &str) -> Result<DumpPlan, Error> {
    let opts = DumpOptions::from_env(|k| env.get(k).map(str::to_string));
    match &opts.engine {
        DumpEngine::MysqlDump => {
            return Err(Error::stub(
                "SYNC_DUMP_ENGINE=mysqldump (escape hatch still stub; default shopware-cli path is implemented)",
            ));
        }
        DumpEngine::Unknown(e) => {
            return Err(Error::fail(format!(
                "Unknown SYNC_DUMP_ENGINE={e}. Use shopware-cli (default) or mysqldump."
            )));
        }
        DumpEngine::ShopwareCli => {}
    }
    let has_mysql = compose_mentions_mysql_service(&env.compose_dir);
    let conn = resolve_dump_connection(env, project_name, has_mysql)?;
    let output = snapshot_dir.join("db.sql.gz");
    Ok(DumpPlan {
        options: opts,
        compose_dir: env.compose_dir.clone(),
        snapshot_dir: snapshot_dir.to_path_buf(),
        output,
        conn,
    })
}

fn resolve_snapshot_dir(args: &SyncOpArgs, env: &ShopEnv, compose_dir: &Path) -> PathBuf {
    if let Some(d) = args
        .snapshot_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let p = PathBuf::from(d);
        if p.is_absolute() {
            p
        } else {
            compose_dir.join(p)
        }
    } else if let Some(d) = env.get("SYNC_SNAPSHOT_DIR") {
        let p = PathBuf::from(d);
        if p.is_absolute() {
            p
        } else {
            compose_dir.join(p)
        }
    } else {
        compose_dir.join("var/runtime-sync")
    }
}

pub fn execute(plan: &SnapshotPlan) -> Result<(), Error> {
    for n in &plan.notes {
        if n.starts_with("WARNING:") {
            eprintln!("{n}");
        } else {
            println!("==> {n}");
        }
    }

    println!(
        "==> Runtime data snapshot  from=local  data={}  shop={}  deploy_env={}  project={}  compose_dir={}  dry-run={}",
        plan.selection.items_csv(),
        plan.shop_id,
        plan.deploy_env.as_deref().unwrap_or("unset"),
        plan.project_name,
        plan.compose_dir.display(),
        if plan.dry_run { 1 } else { 0 },
    );

    if plan.volume_stub {
        eprintln!(
            "not implemented: bind-mount volume snapshot ({}). This CLI dumps the database only; skip volumes with --data db or --skip-volumes.",
            plan.selection.volumes.join(", ")
        );
    }

    if let Some(dump) = &plan.dump {
        println!("==> Dumping database on this host");
        println!(
            "==> Using shopware-cli project dump ({})",
            dump.options.image
        );
        if plan.dry_run {
            println!("==> Snapshot directory: {}", plan.snapshot_dir.display());
        }
        execute_dump(dump, plan.dry_run)?;
        if !plan.dry_run {
            println!("==> Snapshot written to {}", plan.snapshot_dir.display());
        }
    }
    Ok(())
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
            let p = std::env::temp_dir()
                .join(format!("fyrst-cli-{prefix}-{}-{nanos}", std::process::id()));
            fs::create_dir_all(p.join("deploy")).unwrap();
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

    fn args_db_dry() -> SyncOpArgs {
        SyncOpArgs {
            from: Some("local".into()),
            data: Some("db".into()),
            snapshot_dir: None,
            dry_run: true,
            skip_db: false,
            skip_volumes: false,
        }
    }

    #[test]
    fn plans_docker_run_for_bundled_mysql() {
        let shop = TempShop::new("snap-plan");
        shop.write_env(
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=staging
MYSQL_USER=shop
MYSQL_PASSWORD=super-secret-pass
MYSQL_DATABASE=shopware
",
        );
        shop.write_compose_mysql();
        let mut process = HashMap::new();
        process.insert(
            "COMPOSE_DIR".into(),
            shop.path().to_string_lossy().into_owned(),
        );
        let plan = plan_snapshot(&process, shop.path(), &args_db_dry()).unwrap();
        assert_eq!(plan.project_name, "acme-staging");
        assert!(plan.dry_run);
        let dump = plan.dump.as_ref().unwrap();
        assert_eq!(dump.conn.host, "mysql");
        assert_eq!(dump.conn.network, "acme-staging_default");
        assert!(dump.output.ends_with("db.sql.gz"));
        let log = dump.log_line();
        assert!(!dump.log_contains_secret(&log), "{log}");
        assert!(log.contains("--quick"));
        assert!(log.contains("--clean"));
        assert!(log.contains("--skip-lock-tables"));
        assert!(log.contains("--compression=gzip"));
        assert!(log.contains("ghcr.io/shopware/shopware-cli:0.18.4"));
    }

    #[test]
    fn remote_from_is_stub() {
        let shop = TempShop::new("snap-remote");
        shop.write_env(
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=live\nMYSQL_USER=shop\nMYSQL_PASSWORD=x\n",
        );
        shop.write_compose_mysql();
        let mut process = HashMap::new();
        process.insert(
            "COMPOSE_DIR".into(),
            shop.path().to_string_lossy().into_owned(),
        );
        let mut args = args_db_dry();
        args.from = Some("live".into());
        let err = plan_snapshot(&process, shop.path(), &args).unwrap_err();
        match err {
            Error::NotImplemented(m) => {
                assert!(m.contains("remote SSH"), "{m}");
                assert!(m.contains("live"), "{m}");
            }
            Error::Fail(m) => panic!("expected stub, got fail: {m}"),
        }
    }

    #[test]
    fn mysqldump_engine_is_stub() {
        let shop = TempShop::new("snap-engine");
        shop.write_env(
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=dev
MYSQL_USER=shop
MYSQL_PASSWORD=x
SYNC_DUMP_ENGINE=mysqldump
",
        );
        shop.write_compose_mysql();
        let mut process = HashMap::new();
        process.insert(
            "COMPOSE_DIR".into(),
            shop.path().to_string_lossy().into_owned(),
        );
        let err = plan_snapshot(&process, shop.path(), &args_db_dry()).unwrap_err();
        match err {
            Error::NotImplemented(m) => assert!(m.contains("mysqldump"), "{m}"),
            Error::Fail(m) => panic!("expected stub, got fail: {m}"),
        }
    }

    #[test]
    fn missing_shop_id_fails() {
        let shop = TempShop::new("snap-noid");
        shop.write_env("MYSQL_USER=shop\nMYSQL_PASSWORD=x\nCOMPOSE_PROJECT_NAME=x-dev\n");
        shop.write_compose_mysql();
        let mut process = HashMap::new();
        process.insert(
            "COMPOSE_DIR".into(),
            shop.path().to_string_lossy().into_owned(),
        );
        let err = plan_snapshot(&process, shop.path(), &args_db_dry()).unwrap_err();
        assert!(err.to_string().contains("SHOPWARE_SHOP_ID"), "{err}");
    }

    #[test]
    fn dump_only_needs_compose_project_or_deploy_env() {
        let shop = TempShop::new("snap-nodenve");
        shop.write_env("SHOPWARE_SHOP_ID=acme\nMYSQL_USER=shop\nMYSQL_PASSWORD=x\n");
        shop.write_compose_mysql();
        let mut process = HashMap::new();
        process.insert(
            "COMPOSE_DIR".into(),
            shop.path().to_string_lossy().into_owned(),
        );
        let err = plan_snapshot(&process, shop.path(), &args_db_dry()).unwrap_err();
        assert!(
            err.to_string().contains("COMPOSE_PROJECT_NAME")
                || err.to_string().contains("SHOPWARE_DEPLOY_ENV"),
            "{err}"
        );
    }

    #[test]
    fn snapshot_dir_flag_and_skip_volumes() {
        let shop = TempShop::new("snap-dir");
        shop.write_env(
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=dev\nMYSQL_USER=u\nMYSQL_PASSWORD=p\n",
        );
        shop.write_compose_mysql();
        let mut process = HashMap::new();
        process.insert(
            "COMPOSE_DIR".into(),
            shop.path().to_string_lossy().into_owned(),
        );
        let args = SyncOpArgs {
            from: None,
            data: Some("all".into()),
            snapshot_dir: Some("tmp/snap".into()),
            dry_run: true,
            skip_db: false,
            skip_volumes: true,
        };
        let plan = plan_snapshot(&process, shop.path(), &args).unwrap();
        assert!(!plan.volume_stub);
        assert!(plan.snapshot_dir.ends_with("tmp/snap"));
        assert!(plan.dump.unwrap().output.ends_with("tmp/snap/db.sql.gz"));
    }

    #[test]
    fn volumes_only_is_stub() {
        let shop = TempShop::new("snap-vol");
        shop.write_env(
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=dev\nMYSQL_USER=u\nMYSQL_PASSWORD=p\n",
        );
        shop.write_compose_mysql();
        let mut process = HashMap::new();
        process.insert(
            "COMPOSE_DIR".into(),
            shop.path().to_string_lossy().into_owned(),
        );
        let args = SyncOpArgs {
            from: None,
            data: Some("media".into()),
            snapshot_dir: None,
            dry_run: true,
            skip_db: false,
            skip_volumes: false,
        };
        let err = plan_snapshot(&process, shop.path(), &args).unwrap_err();
        match err {
            Error::NotImplemented(m) => assert!(m.contains("volume"), "{m}"),
            Error::Fail(m) => panic!("expected stub: {m}"),
        }
    }
}
