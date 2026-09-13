//! `fyrst-cli shopware sync restore` — DB path uses the same import module as
//! `shopware db import`. Bind-mount volumes remain stub.

use super::data::normalize_data;
use super::env::{require_shop_id, resolve_compose_dir, resolve_snapshot_dir, ShopEnv};
use super::error::Error;
use super::import;
use super::live::LivePolicy;
use super::mysql::find_snapshot_dump;
use crate::cli::SyncOpArgs;
use std::collections::HashMap;
use std::fs;

pub fn run(args: SyncOpArgs) -> Result<(), Error> {
    let process_env: HashMap<String, String> = std::env::vars().collect();
    let cwd = std::env::current_dir().map_err(|e| Error::fail(format!("cannot read cwd: {e}")))?;
    let compose_dir = resolve_compose_dir(&process_env, &cwd)?;
    let compose_dir = fs::canonicalize(&compose_dir).unwrap_or(compose_dir);
    let env = ShopEnv::load(compose_dir.clone(), &process_env)?;
    let shop_id = require_shop_id(&env)?;
    let selection = normalize_data(args.data.as_deref(), args.skip_db, args.skip_volumes)?;
    let snapshot_dir = resolve_snapshot_dir(args.snapshot_dir.as_deref(), &env, &compose_dir);

    println!(
        "==> Runtime data restore  data={}  shop={}  deploy_env={}  snapshot_dir={}  compose_dir={}  dry-run={}",
        selection.items_csv(),
        shop_id,
        env.get("SHOPWARE_DEPLOY_ENV").unwrap_or("unset"),
        snapshot_dir.display(),
        compose_dir.display(),
        if args.dry_run { 1 } else { 0 },
    );

    if selection.want_db {
        let file = find_snapshot_dump(&snapshot_dir)?;
        let plan = import::plan_with_env(&env, &cwd, &file, args.dry_run, LivePolicy::SyncRestore)?;
        import::execute(&plan)?;
    }

    if !selection.volumes.is_empty() {
        eprintln!(
            "not implemented: bind-mount volume restore ({}). Import the database with --data db or use fyrst-cli shopware db import --file.",
            selection.volumes.join(", ")
        );
        if !selection.want_db {
            return Err(Error::stub(format!(
                "bind-mount volume restore ({})",
                selection.volumes.join(", ")
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
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
    }
    impl Drop for TempShop {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn restore_db_plans_import() {
        let shop = TempShop::new("ok");
        fs::write(
            shop.path().join(".env"),
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=staging\nMYSQL_USER=u\nMYSQL_PASSWORD=s3cret-value\n",
        )
        .unwrap();
        fs::write(
            shop.path().join("deploy/compose.yaml"),
            "services:\n  mysql:\n    image: mysql:8.4\n",
        )
        .unwrap();
        fs::write(shop.path().join("var/runtime-sync/db.sql"), b"SELECT 1;\n").unwrap();
        let mut process = HashMap::new();
        process.insert(
            "COMPOSE_DIR".into(),
            shop.path().to_string_lossy().into_owned(),
        );
        let compose_dir = shop.path().to_path_buf();
        let env = ShopEnv::load(compose_dir.clone(), &process).unwrap();
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
        fs::write(
            shop.path().join(".env"),
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=live\nMYSQL_USER=u\nMYSQL_PASSWORD=p\n",
        )
        .unwrap();
        fs::write(
            shop.path().join("deploy/compose.yaml"),
            "services:\n  mysql:\n    image: mysql:8.4\n",
        )
        .unwrap();
        fs::write(shop.path().join("var/runtime-sync/db.sql"), b"x").unwrap();
        let mut process = HashMap::new();
        process.insert(
            "COMPOSE_DIR".into(),
            shop.path().to_string_lossy().into_owned(),
        );
        let env = ShopEnv::load(shop.path().to_path_buf(), &process).unwrap();
        let file = find_snapshot_dump(&shop.path().join("var/runtime-sync")).unwrap();
        let err = import::plan_with_env(&env, shop.path(), &file, true, LivePolicy::SyncRestore)
            .unwrap_err();
        assert!(err.to_string().contains("live"), "{err}");
        assert!(err.to_string().contains("SYNC_ALLOW_LIVE_RESTORE"), "{err}");
    }
}
