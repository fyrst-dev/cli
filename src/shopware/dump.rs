//! `shopware-cli project dump` flags and one-shot `docker run` (recipes `sync-dump.sh` + `sync-db.sh`).
//!
//! Passwords are passed to docker as `--password=…` and must never appear in
//! log lines.

use super::env::ShopEnv;
use super::envfile::urldecode;
use super::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const DEFAULT_SHOPWARE_CLI_IMAGE: &str = "ghcr.io/shopware/shopware-cli:0.18.4";

const MYSQL_WAIT_SH: &str = r#"set -eu
i=0
while [ "$i" -lt 40 ]; do
  i=$((i + 1))
  if command -v mysqladmin >/dev/null 2>&1 && mysqladmin ping -h 127.0.0.1 --silent; then
    exit 0
  fi
  if command -v mariadb-admin >/dev/null 2>&1 && mariadb-admin ping -h 127.0.0.1 --silent; then
    exit 0
  fi
  sleep 2
done
echo "mysql did not become ready" >&2
exit 1
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DumpEngine {
    ShopwareCli,
    MysqlDump,
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DumpOptions {
    pub engine: DumpEngine,
    pub image: String,
    pub quick: bool,
    pub clean: bool,
    pub anonymize: bool,
}

impl DumpOptions {
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        let engine_raw = get("SYNC_DUMP_ENGINE").unwrap_or_default();
        let engine = match engine_raw.trim().to_ascii_lowercase().as_str() {
            "" | "shopware-cli" => DumpEngine::ShopwareCli,
            "mysqldump" | "mariadb-dump" => DumpEngine::MysqlDump,
            other => DumpEngine::Unknown(other.to_string()),
        };
        let image = get("SYNC_SHOPWARE_CLI_IMAGE")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_SHOPWARE_CLI_IMAGE.to_string());
        Self {
            engine,
            image,
            quick: !is_falsy(&get("SYNC_DUMP_QUICK").unwrap_or_else(|| "1".into())),
            clean: !is_falsy(&get("SYNC_DUMP_CLEAN").unwrap_or_else(|| "1".into())),
            anonymize: is_truthy(&get("SYNC_DUMP_ANONYMIZE").unwrap_or_else(|| "0".into())),
        }
    }
}

pub fn is_truthy(s: &str) -> bool {
    matches!(s, "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
}

pub fn is_falsy(s: &str) -> bool {
    matches!(s, "0" | "false" | "FALSE" | "no" | "NO" | "off" | "OFF")
}

/// Core `shopware-cli` argv (no connection secrets, no `--output`).
pub fn dump_core_flags(opts: &DumpOptions) -> Vec<String> {
    let mut f = vec![
        "--no-update-hint".into(),
        "project".into(),
        "dump".into(),
        "--skip-lock-tables".into(),
        "--compression=gzip".into(),
    ];
    if opts.quick {
        f.push("--quick".into());
    }
    if opts.clean {
        f.push("--clean".into());
    }
    if opts.anonymize {
        f.push("--anonymize".into());
    }
    f
}

#[derive(Clone, PartialEq, Eq)]
pub struct DumpConnection {
    pub network: String,
    pub host: String,
    pub port: String,
    pub username: String,
    pub password: Option<String>,
    pub database: String,
}

impl fmt::Debug for DumpConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DumpConnection")
            .field("network", &self.network)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field("database", &self.database)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DatabaseUrl {
    pub scheme: String,
    pub user: String,
    pub password: String,
    pub host: String,
    pub port: String,
    pub name: String,
}

impl fmt::Debug for DatabaseUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatabaseUrl")
            .field("scheme", &self.scheme)
            .field("user", &self.user)
            .field(
                "password",
                &if self.password.is_empty() { "" } else { "***" },
            )
            .field("host", &self.host)
            .field("port", &self.port)
            .field("name", &self.name)
            .finish()
    }
}

pub fn parse_database_url(url: &str) -> Option<DatabaseUrl> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let (scheme, rest) = url.split_once("://")?;
    let rest = rest.split_once('?').map(|(a, _)| a).unwrap_or(rest);
    let (creds, hostpart) = match rest.split_once('@') {
        Some((c, h)) => (Some(c), h),
        None => (None, rest),
    };
    let mut user = String::new();
    let mut password = String::new();
    if let Some(creds) = creds {
        if let Some((u, p)) = creds.split_once(':') {
            user = urldecode(u);
            password = urldecode(p);
        } else {
            user = urldecode(creds);
        }
    }
    let (hp, name_part) = match hostpart.split_once('/') {
        Some((h, n)) => (h, n),
        None => (hostpart, ""),
    };
    let name = name_part.split('/').next().unwrap_or("").to_string();
    let (host, port) = parse_host_port(hp);
    Some(DatabaseUrl {
        scheme: scheme.to_string(),
        user,
        password,
        host,
        port,
        name,
    })
}

fn parse_host_port(hp: &str) -> (String, String) {
    if let Some(rest) = hp.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            let host = rest[..end].to_string();
            let after = &rest[end + 1..];
            let port = after
                .strip_prefix(':')
                .filter(|s| !s.is_empty())
                .unwrap_or("3306");
            return (host, port.to_string());
        }
    }
    if hp.matches(':').count() == 1 {
        if let Some((h, p)) = hp.split_once(':') {
            let port = if p.is_empty() {
                "3306".to_string()
            } else {
                p.to_string()
            };
            return (h.to_string(), port);
        }
    }
    (hp.to_string(), "3306".into())
}

pub fn resolve_dump_connection(
    env: &ShopEnv,
    project_name: &str,
    has_mysql: bool,
) -> Result<DumpConnection, Error> {
    let fallback_db = env.get("MYSQL_DATABASE").unwrap_or("shopware");
    if has_mysql {
        return resolve_bundled_mysql(env, project_name, fallback_db);
    }
    let parsed = parse_database_url(env.get("DATABASE_URL").unwrap_or("")).ok_or_else(|| {
        Error::fail(
            "No bundled mysql service and DATABASE_URL is missing. Set DATABASE_URL or keep the mysql service in deploy/compose.yaml.",
        )
    })?;
    if parsed.host == "mysql" {
        return Err(Error::fail(
            "DATABASE_URL host is 'mysql' but the compose mysql service is not available on the source. Start the stack or point DATABASE_URL at the real database host.",
        ));
    }
    Ok(DumpConnection {
        network: "host".into(),
        host: parsed.host,
        port: parsed.port,
        username: parsed.user,
        password: if parsed.password.is_empty() {
            None
        } else {
            Some(parsed.password)
        },
        database: if parsed.name.is_empty() {
            fallback_db.to_string()
        } else {
            parsed.name
        },
    })
}

fn resolve_bundled_mysql(
    env: &ShopEnv,
    project_name: &str,
    fallback_db: &str,
) -> Result<DumpConnection, Error> {
    let network = format!("{project_name}_default");
    if let Some(user) = env.get("MYSQL_USER") {
        return Ok(DumpConnection {
            network,
            host: "mysql".into(),
            port: "3306".into(),
            username: user.to_string(),
            password: env.get("MYSQL_PASSWORD").map(str::to_string),
            database: env.get("MYSQL_DATABASE").unwrap_or(fallback_db).to_string(),
        });
    }
    if let Some(parsed) = env
        .get("DATABASE_URL")
        .and_then(parse_database_url)
        .filter(|p| !p.user.is_empty())
    {
        return Ok(DumpConnection {
            network,
            host: "mysql".into(),
            port: parsed.port,
            username: parsed.user,
            password: if parsed.password.is_empty() {
                None
            } else {
                Some(parsed.password)
            },
            database: if parsed.name.is_empty() {
                fallback_db.to_string()
            } else {
                parsed.name
            },
        });
    }
    if let Some(root) = env.get("MYSQL_ROOT_PASSWORD") {
        return Ok(DumpConnection {
            network,
            host: "mysql".into(),
            port: "3306".into(),
            username: "root".into(),
            password: Some(root.to_string()),
            database: fallback_db.to_string(),
        });
    }
    Err(Error::fail(
        "Cannot dump bundled mysql: set MYSQL_USER/MYSQL_PASSWORD (or MYSQL_ROOT_PASSWORD) or DATABASE_URL in .env.",
    ))
}

#[derive(Clone, PartialEq, Eq)]
pub struct DumpPlan {
    pub options: DumpOptions,
    pub compose_dir: PathBuf,
    pub snapshot_dir: PathBuf,
    pub output: PathBuf,
    pub conn: DumpConnection,
}

impl fmt::Debug for DumpPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DumpPlan")
            .field("options", &self.options)
            .field("compose_dir", &self.compose_dir)
            .field("snapshot_dir", &self.snapshot_dir)
            .field("output", &self.output)
            .field("conn", &self.conn)
            .finish()
    }
}

impl DumpPlan {
    pub fn core_flags_with_output(&self) -> Vec<String> {
        let mut f = dump_core_flags(&self.options);
        f.push(format!("--output={}", self.output.display()));
        f
    }

    /// Full shopware-cli argv including connection (may contain `--password=`).
    pub fn shopware_cli_args(&self) -> Vec<String> {
        let mut f = self.core_flags_with_output();
        if !self.conn.host.is_empty() {
            f.push("--host".into());
            f.push(self.conn.host.clone());
        }
        if !self.conn.port.is_empty() {
            f.push("--port".into());
            f.push(self.conn.port.clone());
        }
        if !self.conn.username.is_empty() {
            f.push("--username".into());
            f.push(self.conn.username.clone());
        }
        if let Some(pw) = &self.conn.password {
            if !pw.is_empty() {
                f.push(format!("--password={pw}"));
            }
        }
        if !self.conn.database.is_empty() {
            f.push("--database".into());
            f.push(self.conn.database.clone());
        }
        f
    }

    /// `docker run …` arguments (program name not included).
    pub fn docker_run_args(&self) -> Vec<String> {
        let mut a = vec![
            "run".into(),
            "--rm".into(),
            "--network".into(),
            self.conn.network.clone(),
            "-v".into(),
            format!(
                "{}:{}:ro",
                self.compose_dir.display(),
                self.compose_dir.display()
            ),
            "-v".into(),
            format!(
                "{}:{}",
                self.snapshot_dir.display(),
                self.snapshot_dir.display()
            ),
            "-w".into(),
            self.compose_dir.display().to_string(),
            "-e".into(),
            "HOME=/tmp".into(),
            "-e".into(),
            "SHOPWARE_CLI_NO_UPDATE_NOTIFICATION=true".into(),
            self.options.image.clone(),
        ];
        a.extend(self.shopware_cli_args());
        a
    }

    /// Dry-run / log line. Never includes `--password` or the secret itself.
    pub fn log_line(&self) -> String {
        format!(
            "docker run --rm --network {} -v {}:{}:ro -v {}:{} -w {} {} {} --host {} --port {} --username {} --database {}",
            self.conn.network,
            self.compose_dir.display(),
            self.compose_dir.display(),
            self.snapshot_dir.display(),
            self.snapshot_dir.display(),
            self.compose_dir.display(),
            self.options.image,
            self.core_flags_with_output().join(" "),
            self.conn.host,
            self.conn.port,
            self.conn.username,
            self.conn.database,
        )
    }

    pub fn log_contains_secret(&self, text: &str) -> bool {
        match &self.conn.password {
            Some(p) if !p.is_empty() => text.contains(p),
            _ => false,
        }
    }
}

pub fn dump_fail_hint(image: &str) -> String {
    format!(
        "\
shopware-cli project dump could not run (image missing, pull failed, or docker run failed).
Pinned image: {image}
The Shopware app image (compose service web) does not ship shopware-cli. Dumps use a
one-shot container from the official CLI image, attached to the Compose network
(hostname mysql) with the shop root mounted for .env / .shopware-project.yml.

  docker pull {image}

Override the pin with SYNC_SHOPWARE_CLI_IMAGE=<registry/image:tag>.
Docs:
  https://developer.shopware.com/docs/products/tools/cli/installation.html
  https://developer.shopware.com/docs/products/tools/cli/project-commands/mysql-dump.html
This path does not silently fall back to mysqldump. Escape hatch: SYNC_DUMP_ENGINE=mysqldump
(not implemented in fyrst-cli yet)."
    )
}

pub fn require_docker() -> Result<(), Error> {
    match Command::new("docker")
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => Err(Error::fail(
            "Cannot talk to the Docker daemon. Add this user to the docker group or run where the daemon is reachable.",
        )),
        Err(_) => Err(Error::fail(
            "Missing command 'docker'. Install Docker Engine + Compose v2.",
        )),
    }
}

pub fn execute_dump(plan: &DumpPlan, dry_run: bool) -> Result<(), Error> {
    if dry_run {
        let line = plan.log_line();
        if plan.log_contains_secret(&line) {
            return Err(Error::fail(
                "internal error: dump log would have included a password",
            ));
        }
        println!("==> DRY-RUN {line}");
        return Ok(());
    }
    fs::create_dir_all(&plan.snapshot_dir).map_err(|e| {
        Error::fail(format!(
            "cannot create snapshot dir {}: {e}",
            plan.snapshot_dir.display()
        ))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&plan.snapshot_dir, fs::Permissions::from_mode(0o700));
    }

    ensure_image(&plan.options.image)?;
    maybe_compose_up_mysql(&plan.compose_dir);

    let args = plan.docker_run_args();
    let status = Command::new("docker").args(&args).status().map_err(|e| {
        Error::fail(format!(
            "failed to exec docker: {e}\n{}",
            dump_fail_hint(&plan.options.image)
        ))
    })?;
    if !status.success() {
        return Err(Error::fail(dump_fail_hint(&plan.options.image)));
    }
    finish_dump_file(&plan.output)
}

fn ensure_image(img: &str) -> Result<(), Error> {
    let inspect = Command::new("docker")
        .args(["image", "inspect", img])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if matches!(inspect, Ok(s) if s.success()) {
        return Ok(());
    }
    println!("==> Pulling {img} (shopware-cli project dump; not present locally)");
    let pull = Command::new("docker")
        .args(["pull", img])
        .status()
        .map_err(|e| Error::fail(format!("docker pull failed: {e}\n{}", dump_fail_hint(img))))?;
    if !pull.success() {
        return Err(Error::fail(dump_fail_hint(img)));
    }
    Ok(())
}

fn maybe_compose_up_mysql(compose_dir: &Path) {
    let files = super::env::existing_compose_files(compose_dir);
    if files.is_empty() || !super::env::compose_mentions_mysql_service(compose_dir) {
        return;
    }
    let mut args = vec!["compose".into(), "--env-file".into(), ".env".into()];
    for f in &files {
        args.push("-f".into());
        args.push(f.clone());
    }
    args.extend([
        "up".into(),
        "-d".into(),
        "--no-build".into(),
        "mysql".into(),
    ]);
    match Command::new("docker")
        .args(&args)
        .current_dir(compose_dir)
        .status()
    {
        Ok(s) if s.success() => {
            wait_mysql(compose_dir, &files);
        }
        Ok(_) => {
            eprintln!(
                "WARNING: docker compose up mysql failed; dumping anyway (is the stack already running?)"
            );
        }
        Err(e) => {
            eprintln!("WARNING: could not exec docker compose ({e}); dumping anyway");
        }
    }
}

fn wait_mysql(compose_dir: &Path, files: &[String]) {
    let mut args = vec!["compose".into(), "--env-file".into(), ".env".into()];
    for f in files {
        args.push("-f".into());
        args.push(f.clone());
    }
    args.extend([
        "exec".into(),
        "-T".into(),
        "mysql".into(),
        "sh".into(),
        "-c".into(),
        MYSQL_WAIT_SH.to_string(),
    ]);
    let status = Command::new("docker")
        .args(&args)
        .current_dir(compose_dir)
        .status();
    if !matches!(status, Ok(s) if s.success()) {
        eprintln!("WARNING: mysql did not become ready via compose exec; dumping anyway");
    }
}

pub fn finish_dump_file(output: &Path) -> Result<(), Error> {
    let meta = fs::metadata(output).map_err(|_| {
        Error::fail(format!(
            "shopware-cli project dump produced an empty {}",
            output.display()
        ))
    })?;
    if meta.len() == 0 {
        return Err(Error::fail(format!(
            "shopware-cli project dump produced an empty {}",
            output.display()
        )));
    }
    let mut mag = [0u8; 2];
    File::open(output)
        .and_then(|mut f| f.read_exact(&mut mag))
        .map_err(|e| Error::fail(format!("cannot read {}: {e}", output.display())))?;
    if mag != [0x1f, 0x8b] {
        return Err(Error::fail(format!(
            "Dump at {} is not valid gzip (expected shopware-cli --compression=gzip --output).",
            output.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(output, fs::Permissions::from_mode(0o600));
        if let Some((uid, gid)) = current_ids() {
            let _ = std::os::unix::fs::chown(output, Some(uid), Some(gid));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn current_ids() -> Option<(u32, u32)> {
    fn id_flag(flag: &str) -> Option<u32> {
        let o = Command::new("id").arg(flag).output().ok()?;
        if !o.status.success() {
            return None;
        }
        String::from_utf8(o.stdout).ok()?.trim().parse().ok()
    }
    Some((id_flag("-u")?, id_flag("-g")?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn opts_from(pairs: &[(&str, &str)]) -> DumpOptions {
        DumpOptions::from_env(|k| {
            pairs
                .iter()
                .find(|(pk, _)| *pk == k)
                .map(|(_, v)| (*v).to_string())
        })
    }

    fn env_from(pairs: &[(&str, &str)]) -> ShopEnv {
        let mut vars = HashMap::new();
        for (k, v) in pairs {
            vars.insert((*k).to_string(), (*v).to_string());
        }
        ShopEnv::from_vars(PathBuf::from("/shop"), vars)
    }

    #[test]
    fn default_flags_quick_clean_no_anonymize() {
        let opts = opts_from(&[]);
        assert_eq!(opts.image, DEFAULT_SHOPWARE_CLI_IMAGE);
        assert!(opts.quick && opts.clean && !opts.anonymize);
        let flags = dump_core_flags(&opts);
        assert!(flags.contains(&"--skip-lock-tables".into()));
        assert!(flags.contains(&"--compression=gzip".into()));
        assert!(flags.contains(&"--quick".into()));
        assert!(flags.contains(&"--clean".into()));
        assert!(!flags.contains(&"--anonymize".into()));
        assert_eq!(flags[0], "--no-update-hint");
        assert_eq!(&flags[1..3], ["project", "dump"]);
    }

    #[test]
    fn env_opts_out_and_anonymize() {
        let opts = opts_from(&[
            ("SYNC_DUMP_QUICK", "0"),
            ("SYNC_DUMP_CLEAN", "off"),
            ("SYNC_DUMP_ANONYMIZE", "1"),
            ("SYNC_SHOPWARE_CLI_IMAGE", "ghcr.io/example/cli:dev"),
        ]);
        let flags = dump_core_flags(&opts);
        assert!(!flags.contains(&"--quick".into()));
        assert!(!flags.contains(&"--clean".into()));
        assert!(flags.contains(&"--anonymize".into()));
        assert_eq!(opts.image, "ghcr.io/example/cli:dev");
    }

    #[test]
    fn parse_url_percent_password() {
        let u = parse_database_url("mysql://user:p%40ss@mysql:3306/shopware").unwrap();
        assert_eq!(u.user, "user");
        assert_eq!(u.password, "p@ss");
        assert_eq!(u.host, "mysql");
        assert_eq!(u.port, "3306");
        assert_eq!(u.name, "shopware");
    }

    #[test]
    fn parse_url_ipv6() {
        let u = parse_database_url("mysql://u:p@[::1]:3307/db").unwrap();
        assert_eq!(u.host, "::1");
        assert_eq!(u.port, "3307");
        assert_eq!(u.name, "db");
    }

    #[test]
    fn bundled_mysql_user_password() {
        let env = env_from(&[
            ("MYSQL_USER", "shop"),
            ("MYSQL_PASSWORD", "s3cret-value"),
            ("MYSQL_DATABASE", "shopware"),
        ]);
        let c = resolve_dump_connection(&env, "acme-staging", true).unwrap();
        assert_eq!(c.network, "acme-staging_default");
        assert_eq!(c.host, "mysql");
        assert_eq!(c.username, "shop");
        assert_eq!(c.password.as_deref(), Some("s3cret-value"));
        assert_eq!(c.database, "shopware");
        assert_eq!(c.port, "3306");
    }

    #[test]
    fn bundled_falls_back_to_database_url() {
        let env = env_from(&[(
            "DATABASE_URL",
            "mysql://alice:hidden-pw@mysql:3306/shopware",
        )]);
        let c = resolve_dump_connection(&env, "acme-live", true).unwrap();
        assert_eq!(c.username, "alice");
        assert_eq!(c.password.as_deref(), Some("hidden-pw"));
        assert_eq!(c.host, "mysql");
    }

    #[test]
    fn bundled_root_password() {
        let env = env_from(&[("MYSQL_ROOT_PASSWORD", "root-secret")]);
        let c = resolve_dump_connection(&env, "p", true).unwrap();
        assert_eq!(c.username, "root");
        assert_eq!(c.password.as_deref(), Some("root-secret"));
    }

    #[test]
    fn external_url_uses_host_network() {
        let env = env_from(&[(
            "DATABASE_URL",
            "mysql://alice:s3cret-value@db.example.com:3307/shop",
        )]);
        let c = resolve_dump_connection(&env, "unused", false).unwrap();
        assert_eq!(c.network, "host");
        assert_eq!(c.host, "db.example.com");
        assert_eq!(c.port, "3307");
        assert_eq!(c.username, "alice");
        assert_eq!(c.database, "shop");
    }

    #[test]
    fn mysql_host_without_service_errors() {
        let env = env_from(&[("DATABASE_URL", "mysql://u:p@mysql/shopware")]);
        let err = resolve_dump_connection(&env, "p", false).unwrap_err();
        assert!(err.to_string().contains("mysql"), "{err}");
    }

    #[test]
    fn log_line_omits_password_argv_keeps_it() {
        let env = env_from(&[
            ("MYSQL_USER", "shop"),
            ("MYSQL_PASSWORD", "s3cret-value"),
            ("MYSQL_DATABASE", "shopware"),
        ]);
        let conn = resolve_dump_connection(&env, "acme-staging", true).unwrap();
        let plan = DumpPlan {
            options: opts_from(&[]),
            compose_dir: PathBuf::from("/shop"),
            snapshot_dir: PathBuf::from("/shop/var/runtime-sync"),
            output: PathBuf::from("/shop/var/runtime-sync/db.sql.gz"),
            conn,
        };
        let log = plan.log_line();
        assert!(!plan.log_contains_secret(&log), "{log}");
        assert!(!log.contains("--password"), "{log}");
        assert!(log.contains("acme-staging_default"));
        assert!(log.contains("--host mysql"));
        assert!(log.contains("--output=/shop/var/runtime-sync/db.sql.gz"));
        assert!(log.contains(DEFAULT_SHOPWARE_CLI_IMAGE));
        let argv = plan.docker_run_args();
        assert!(
            argv.iter().any(|a| a == "--password=s3cret-value"),
            "{argv:?}"
        );
        assert!(argv.contains(&"--rm".to_string()));
        assert_eq!(argv[0], "run");
    }

    #[test]
    fn finish_dump_rejects_non_gzip() {
        let dir = std::env::temp_dir().join(format!(
            "fyrst-cli-gzip-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("db.sql.gz");
        fs::write(&p, b"not gzip").unwrap();
        let err = finish_dump_file(&p).unwrap_err();
        assert!(err.to_string().contains("gzip"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn finish_dump_accepts_gzip_magic() {
        let dir = std::env::temp_dir().join(format!(
            "fyrst-cli-gzip-ok-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("db.sql.gz");
        fs::write(&p, [0x1f, 0x8b, 0x08, 0x00]).unwrap();
        finish_dump_file(&p).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}
