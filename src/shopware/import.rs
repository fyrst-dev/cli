//! `fyrst-cli shopware db import` — MySQL/MariaDB client import.
//!
//! shopware-cli has no import. Bundled Compose `mysql` uses
//! `docker compose exec -T mysql`; otherwise a one-shot client image talks to
//! `DATABASE_URL`. Never logs passwords.

use super::env::{
    compose_mentions_mysql_service, existing_compose_files, require_shop_id, resolve_compose_dir,
    vps_project_name, ShopEnv,
};
use super::error::Error;
use super::live::{assert_not_live, LivePolicy, LiveSignals};
use super::mysql::{
    client_image_for_url, collect_env_secrets, compose_cli_log, compose_up_mysql,
    log_contains_secret, parse_database_url, require_docker, require_gzip, verify_gzip_magic,
    DatabaseUrl, EXTERNAL_RESTORE_SH, MYSQL_RESTORE_SH,
};
use crate::cli::DbImportArgs;
use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlDumpKind {
    Sql,
    Gzip,
}

#[derive(Clone, PartialEq, Eq)]
pub enum ImportTarget {
    Bundled {
        compose_dir: PathBuf,
        compose_files: Vec<String>,
    },
    External {
        host: String,
        port: String,
        user: String,
        password: Option<String>,
        database: String,
        image: String,
    },
}

impl fmt::Debug for ImportTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bundled {
                compose_dir,
                compose_files,
            } => f
                .debug_struct("Bundled")
                .field("compose_dir", compose_dir)
                .field("compose_files", compose_files)
                .finish(),
            Self::External {
                host,
                port,
                user,
                database,
                image,
                password,
            } => f
                .debug_struct("External")
                .field("host", host)
                .field("port", port)
                .field("user", user)
                .field("password", &password.as_ref().map(|_| "***"))
                .field("database", database)
                .field("image", image)
                .finish(),
        }
    }
}

pub struct ImportPlan {
    pub compose_dir: PathBuf,
    pub shop_id: String,
    pub deploy_env: Option<String>,
    pub file: PathBuf,
    pub kind: SqlDumpKind,
    pub target: ImportTarget,
    pub dry_run: bool,
    pub notes: Vec<String>,
    secrets: Vec<String>,
}

impl fmt::Debug for ImportPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImportPlan")
            .field("compose_dir", &self.compose_dir)
            .field("shop_id", &self.shop_id)
            .field("deploy_env", &self.deploy_env)
            .field("file", &self.file)
            .field("kind", &self.kind)
            .field("target", &self.target)
            .field("dry_run", &self.dry_run)
            .finish_non_exhaustive()
    }
}

pub fn classify_dump_file(path: &Path) -> Result<SqlDumpKind, Error> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.ends_with(".sql.gz") || name.ends_with(".gz") {
        Ok(SqlDumpKind::Gzip)
    } else if name.ends_with(".sql") {
        Ok(SqlDumpKind::Sql)
    } else {
        Err(Error::fail(format!(
            "Unsupported dump file '{}' (expected .sql or .sql.gz)",
            path.display()
        )))
    }
}

pub fn decode_log(kind: SqlDumpKind, file: &Path) -> String {
    match kind {
        SqlDumpKind::Gzip => format!("gzip -dc {}", file.display()),
        SqlDumpKind::Sql => format!("cat {}", file.display()),
    }
}

impl ImportPlan {
    pub fn log_line(&self) -> String {
        let decode = decode_log(self.kind, &self.file);
        match &self.target {
            ImportTarget::Bundled { compose_files, .. } => {
                format!(
                    "{decode} | {} exec -T mysql sh -c '<mysql|mariadb>'",
                    compose_cli_log(compose_files, self.compose_project().as_deref())
                )
            }
            ImportTarget::External {
                database, image, ..
            } => {
                format!("{decode} | docker run {image} mysql {database}")
            }
        }
    }

    pub fn log_contains_secret(&self, text: &str) -> bool {
        log_contains_secret(text, &self.secrets)
    }

    fn compose_project(&self) -> Option<String> {
        self.deploy_env
            .as_ref()
            .map(|e| vps_project_name(&self.shop_id, e))
    }
}

pub fn run(args: DbImportArgs) -> Result<(), Error> {
    let process_env: HashMap<String, String> = std::env::vars().collect();
    let cwd = std::env::current_dir().map_err(|e| Error::fail(format!("cannot read cwd: {e}")))?;
    let plan = plan_import(
        &process_env,
        &cwd,
        Path::new(&args.file),
        args.dry_run,
        LivePolicy::DbImport {
            allow_live_flag: args.allow_live,
        },
    )?;
    execute(&plan)
}

pub fn plan_import(
    process_env: &HashMap<String, String>,
    cwd: &Path,
    file: &Path,
    dry_run: bool,
    policy: LivePolicy,
) -> Result<ImportPlan, Error> {
    let compose_dir = resolve_compose_dir(process_env, cwd)?;
    let compose_dir = fs::canonicalize(&compose_dir).unwrap_or(compose_dir);
    let env = ShopEnv::load(compose_dir.clone(), process_env)?;
    plan_with_env(&env, cwd, file, dry_run, policy)
}

pub fn plan_with_env(
    env: &ShopEnv,
    cwd: &Path,
    file: &Path,
    dry_run: bool,
    policy: LivePolicy,
) -> Result<ImportPlan, Error> {
    let shop_id = require_shop_id(env)?;
    let signals = LiveSignals::from_shop(env);
    let mut notes = Vec::new();
    if let Some(warn) = assert_not_live(&signals, policy)? {
        notes.push(warn);
    }

    let file = resolve_dump_file(cwd, file)?;
    let kind = classify_dump_file(&file)?;

    let has_mysql = compose_mentions_mysql_service(&env.compose_dir);
    let (target, url) = resolve_target(env, has_mysql)?;
    let secrets = collect_env_secrets(env, url.as_ref());

    if has_mysql && dry_run {
        notes.push("Would start compose service mysql if needed".into());
    }

    Ok(ImportPlan {
        compose_dir: env.compose_dir.clone(),
        shop_id,
        deploy_env: env.get("SHOPWARE_DEPLOY_ENV").map(str::to_string),
        file,
        kind,
        target,
        dry_run,
        notes,
        secrets,
    })
}

fn resolve_dump_file(cwd: &Path, file: &Path) -> Result<PathBuf, Error> {
    if file.as_os_str().is_empty() {
        return Err(Error::fail("Dump --file is empty"));
    }
    let p = if file.is_absolute() {
        file.to_path_buf()
    } else {
        cwd.join(file)
    };
    if !p.is_file() {
        return Err(Error::fail(format!("Dump file not found: {}", p.display())));
    }
    Ok(fs::canonicalize(&p).unwrap_or(p))
}

fn resolve_target(
    env: &ShopEnv,
    has_mysql: bool,
) -> Result<(ImportTarget, Option<DatabaseUrl>), Error> {
    if has_mysql {
        let compose_files = existing_compose_files(&env.compose_dir);
        if compose_files.is_empty() {
            return Err(Error::fail(
                "Compose mysql service is present but no compose files were found under shop root.",
            ));
        }
        return Ok((
            ImportTarget::Bundled {
                compose_dir: env.compose_dir.clone(),
                compose_files,
            },
            None,
        ));
    }

    let parsed = parse_database_url(env.get("DATABASE_URL").unwrap_or("")).ok_or_else(|| {
        Error::fail(
            "No bundled mysql service and DATABASE_URL is missing; cannot restore db. Set DATABASE_URL or keep the mysql service in deploy/compose.yaml.",
        )
    })?;
    if parsed.host == "mysql" {
        return Err(Error::fail(
            "DATABASE_URL host is 'mysql' but the compose mysql service is not running here.",
        ));
    }
    if parsed.user.is_empty() {
        return Err(Error::fail(
            "DATABASE_URL has no username; cannot import into the external database.",
        ));
    }
    let database = if parsed.name.is_empty() {
        env.get("MYSQL_DATABASE").unwrap_or("shopware").to_string()
    } else {
        parsed.name.clone()
    };
    let image = client_image_for_url(&parsed.scheme, env);
    let password = if parsed.password.is_empty() {
        None
    } else {
        Some(parsed.password.clone())
    };
    Ok((
        ImportTarget::External {
            host: parsed.host.clone(),
            port: parsed.port.clone(),
            user: parsed.user.clone(),
            password,
            database,
            image,
        },
        Some(parsed),
    ))
}

pub fn execute(plan: &ImportPlan) -> Result<(), Error> {
    for n in &plan.notes {
        if n.starts_with("WARNING:") {
            eprintln!("{n}");
        } else {
            println!("==> {n}");
        }
    }

    println!(
        "==> Database import  file={}  shop={}  deploy_env={}  compose_dir={}  dry-run={}",
        plan.file.display(),
        plan.shop_id,
        plan.deploy_env.as_deref().unwrap_or("unset"),
        plan.compose_dir.display(),
        if plan.dry_run { 1 } else { 0 },
    );

    match &plan.target {
        ImportTarget::Bundled { .. } => {
            println!(
                "==> Restoring database on this host from {}",
                plan.file.display()
            );
        }
        ImportTarget::External {
            host,
            port,
            database,
            ..
        } => {
            println!(
                "==> Restoring external database {host}:{port}/{database} (credentials from DATABASE_URL, not printed)"
            );
        }
    }

    let line = plan.log_line();
    if plan.log_contains_secret(&line) {
        return Err(Error::fail(
            "internal error: import log would have included a password",
        ));
    }

    if plan.dry_run {
        println!("==> DRY-RUN {line}");
        return Ok(());
    }

    require_docker()?;
    let meta = fs::metadata(&plan.file)
        .map_err(|e| Error::fail(format!("cannot stat {}: {e}", plan.file.display())))?;
    if meta.len() == 0 {
        return Err(Error::fail(format!(
            "Dump file is empty: {}",
            plan.file.display()
        )));
    }
    if plan.kind == SqlDumpKind::Gzip {
        require_gzip()?;
        verify_gzip_magic(&plan.file)?;
    }

    match &plan.target {
        ImportTarget::Bundled {
            compose_dir,
            compose_files,
        } => {
            let project = plan.compose_project();
            compose_up_mysql(compose_dir, compose_files, project.as_deref())?;
            run_bundled(plan, compose_dir, compose_files, project.as_deref())?;
        }
        ImportTarget::External { .. } => run_external(plan)?,
    }
    println!("==> Import finished");
    Ok(())
}

fn bundled_exec_args(files: &[String], project: Option<&str>) -> Vec<String> {
    let mut a = super::mysql::compose_argv(files, project);
    a.extend([
        "exec".into(),
        "-T".into(),
        "mysql".into(),
        "sh".into(),
        "-c".into(),
        MYSQL_RESTORE_SH.to_string(),
    ]);
    a
}

fn external_run_args(target: &ImportTarget) -> Result<Vec<String>, Error> {
    let ImportTarget::External {
        host,
        port,
        user,
        password,
        database,
        image,
    } = target
    else {
        return Err(Error::fail(
            "internal error: expected external import target",
        ));
    };
    let mut a = vec![
        "run".into(),
        "--rm".into(),
        "-i".into(),
        "--network".into(),
        "host".into(),
        "--entrypoint".into(),
        "sh".into(),
    ];
    if password.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
        a.extend(["-e".into(), "MYSQL_PWD".into()]);
    }
    a.extend([
        "-e".into(),
        format!("DUMP_HOST={host}"),
        "-e".into(),
        format!("DUMP_PORT={port}"),
        "-e".into(),
        format!("DUMP_USER={user}"),
        "-e".into(),
        format!("DUMP_DB={database}"),
        image.clone(),
        "-c".into(),
        EXTERNAL_RESTORE_SH.to_string(),
    ]);
    Ok(a)
}

fn run_bundled(
    plan: &ImportPlan,
    compose_dir: &Path,
    files: &[String],
    project: Option<&str>,
) -> Result<(), Error> {
    let args = bundled_exec_args(files, project);
    pipe_sql_into_docker(plan, &args, Some(compose_dir), None)
}

fn run_external(plan: &ImportPlan) -> Result<(), Error> {
    let args = external_run_args(&plan.target)?;
    let mut mysql_pwd = None;
    if let ImportTarget::External {
        password: Some(pw), ..
    } = &plan.target
    {
        if !pw.is_empty() {
            mysql_pwd = Some(pw.clone());
        }
    }
    pipe_sql_into_docker(plan, &args, None, mysql_pwd.as_deref())
}

fn pipe_sql_into_docker(
    plan: &ImportPlan,
    docker_args: &[String],
    cwd: Option<&Path>,
    mysql_pwd: Option<&str>,
) -> Result<(), Error> {
    let mut docker = Command::new("docker");
    docker.args(docker_args);
    if let Some(dir) = cwd {
        docker.current_dir(dir);
    }
    if let Some(pw) = mysql_pwd {
        docker.env("MYSQL_PWD", pw);
    }

    let status = match plan.kind {
        SqlDumpKind::Gzip => {
            let mut gz = Command::new("gzip")
                .args(["-dc"])
                .arg(&plan.file)
                .stdout(Stdio::piped())
                .spawn()
                .map_err(|e| Error::fail(format!("failed to exec gzip: {e}")))?;
            let stdout = gz
                .stdout
                .take()
                .ok_or_else(|| Error::fail("internal error: gzip stdout pipe missing"))?;
            docker.stdin(Stdio::from(stdout));
            let docker_status = docker
                .status()
                .map_err(|e| Error::fail(format!("failed to exec docker: {e}")))?;
            let gz_status = gz
                .wait()
                .map_err(|e| Error::fail(format!("gzip wait failed: {e}")))?;
            if !gz_status.success() {
                return Err(Error::fail(format!(
                    "gzip -dc failed for {}",
                    plan.file.display()
                )));
            }
            docker_status
        }
        SqlDumpKind::Sql => {
            let file = File::open(&plan.file)
                .map_err(|e| Error::fail(format!("cannot open {}: {e}", plan.file.display())))?;
            docker.stdin(Stdio::from(file));
            docker
                .status()
                .map_err(|e| Error::fail(format!("failed to exec docker: {e}")))?
        }
    };

    if !status.success() {
        return Err(Error::fail(
            "MySQL/MariaDB import failed (docker compose exec or client container). Passwords are not printed; check that the dump is valid SQL and the target is reachable.",
        ));
    }
    Ok(())
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
                "fyrst-cli-imp-{prefix}-{}-{nanos}",
                std::process::id()
            ));
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
        fn dump(&self, name: &str, body: &[u8]) -> PathBuf {
            let p = self.0.join(name);
            fs::write(&p, body).unwrap();
            p
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

    #[test]
    fn classify_extensions() {
        assert_eq!(
            classify_dump_file(Path::new("db.sql.gz")).unwrap(),
            SqlDumpKind::Gzip
        );
        assert_eq!(
            classify_dump_file(Path::new("/tmp/x.SQL")).unwrap(),
            SqlDumpKind::Sql
        );
        assert!(classify_dump_file(Path::new("db.dump")).is_err());
    }

    #[test]
    fn bundled_dry_run_omits_password() {
        let shop = TempShop::new("bundled");
        shop.write_env(
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=staging
COMPOSE_PROJECT_NAME=shopware-acme
MYSQL_USER=shop
MYSQL_PASSWORD=super-secret-pass
MYSQL_DATABASE=shopware
",
        );
        shop.write_compose_mysql();
        let dump = shop.dump("db.sql.gz", b"not-a-real-gzip-but-exists");
        let plan = plan_import(
            &process(shop.path()),
            shop.path(),
            &dump,
            true,
            LivePolicy::DbImport {
                allow_live_flag: false,
            },
        )
        .unwrap();
        let log = plan.log_line();
        assert!(!plan.log_contains_secret(&log), "{log}");
        assert!(!log.contains("super-secret-pass"), "{log}");
        assert!(log.contains("gzip -dc"), "{log}");
        assert!(log.contains("exec -T mysql"), "{log}");
        assert!(log.contains("-f deploy/compose.yaml"), "{log}");
        assert!(log.contains("-p acme-staging"), "{log}");
        assert!(!log.contains("shopware-acme"), "{log}");
        assert!(matches!(plan.target, ImportTarget::Bundled { .. }));
    }

    #[test]
    fn sql_file_uses_cat_in_plan() {
        let shop = TempShop::new("sql");
        shop.write_env(
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=dev\nMYSQL_USER=u\nMYSQL_PASSWORD=p\n",
        );
        shop.write_compose_mysql();
        let dump = shop.dump("db.sql", b"SELECT 1;\n");
        let plan = plan_import(
            &process(shop.path()),
            shop.path(),
            &dump,
            true,
            LivePolicy::DbImport {
                allow_live_flag: false,
            },
        )
        .unwrap();
        let log = plan.log_line();
        assert!(log.starts_with("cat "), "{log}");
        assert!(!log.contains("gzip"), "{log}");
    }

    #[test]
    fn external_url_plan() {
        let shop = TempShop::new("ext");
        shop.write_env(
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=dev
DATABASE_URL=mysql://alice:s3cret-value@db.example.com:3307/shop
",
        );
        fs::write(
            shop.path().join("deploy/compose.yaml"),
            "services:\n  web:\n    image: example\n",
        )
        .unwrap();
        let dump = shop.dump("db.sql.gz", &[0x1f, 0x8b, 0x08, 0x00]);
        let plan = plan_import(
            &process(shop.path()),
            shop.path(),
            &dump,
            true,
            LivePolicy::DbImport {
                allow_live_flag: false,
            },
        )
        .unwrap();
        let log = plan.log_line();
        assert!(!plan.log_contains_secret(&log), "{log}");
        assert!(!log.contains("s3cret-value"), "{log}");
        assert!(log.contains("docker run mysql:8.4 mysql shop"), "{log}");
        match &plan.target {
            ImportTarget::External {
                host,
                port,
                user,
                database,
                image,
                password,
            } => {
                assert_eq!(host, "db.example.com");
                assert_eq!(port, "3307");
                assert_eq!(user, "alice");
                assert_eq!(database, "shop");
                assert_eq!(image, "mysql:8.4");
                assert_eq!(password.as_deref(), Some("s3cret-value"));
            }
            other => panic!("{other:?}"),
        }
        let args = external_run_args(&plan.target).unwrap();
        assert!(args.contains(&"--rm".to_string()));
        assert!(args.windows(2).any(|w| w == ["-e", "MYSQL_PWD"]));
        assert!(!args.iter().any(|a| a.contains("s3cret-value")));
        assert!(!args.iter().any(|a| a.contains("MYSQL_PWD=")));
    }

    #[test]
    fn live_without_flag_fails() {
        let shop = TempShop::new("live");
        shop.write_env(
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=live\nMYSQL_USER=u\nMYSQL_PASSWORD=p\n",
        );
        shop.write_compose_mysql();
        let dump = shop.dump("db.sql", b"x");
        let err = plan_import(
            &process(shop.path()),
            shop.path(),
            &dump,
            true,
            LivePolicy::DbImport {
                allow_live_flag: false,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("live"), "{err}");
        assert!(err.to_string().contains("--allow-live"), "{err}");
    }

    #[test]
    fn live_with_flag_plans() {
        let shop = TempShop::new("liveok");
        shop.write_env(
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=live\nMYSQL_USER=u\nMYSQL_PASSWORD=p\n",
        );
        shop.write_compose_mysql();
        let dump = shop.dump("db.sql", b"x");
        let plan = plan_import(
            &process(shop.path()),
            shop.path(),
            &dump,
            true,
            LivePolicy::DbImport {
                allow_live_flag: true,
            },
        )
        .unwrap();
        assert!(plan.notes.iter().any(|n| n.starts_with("WARNING:")));
    }

    #[test]
    fn mysql_host_without_service_errors() {
        let shop = TempShop::new("mysqlhost");
        shop.write_env(
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=dev\nDATABASE_URL=mysql://u:p@mysql/shopware\n",
        );
        fs::write(
            shop.path().join("deploy/compose.yaml"),
            "services:\n  web:\n    image: example\n",
        )
        .unwrap();
        let dump = shop.dump("db.sql", b"x");
        let err = plan_import(
            &process(shop.path()),
            shop.path(),
            &dump,
            true,
            LivePolicy::DbImport {
                allow_live_flag: false,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("mysql"), "{err}");
    }
}
